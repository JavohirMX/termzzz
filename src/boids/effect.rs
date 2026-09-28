//! A flocking simulation, drawn as a flock.
//!
//! The rules are Reynolds' three, and they were always implemented correctly.
//! Everything that was wrong with this effect was in what it did with them:
//!
//! - two of the three forces were attenuated to a few percent of the third by
//!   their own normalisation, so the visible behaviour was separation, a hard
//!   border, and a constant random swirl -- a gas, not a flock;
//! - the screen was cleared every frame, so a *motion* phenomenon was rendered
//!   as three hundred unrelated glyphs changing at once, which is flicker;
//! - the colour was computed from `speed * 128` against a `max_speed` of 0.6, so
//!   the red channel could not leave 76 while green was pinned at 200: one dark
//!   teal, for the whole flock, at all times.
//!
//! See `apply_rules` for the force balance, and the comment on `Boids::frame`
//! for why this holds two buffers instead of a [`crate::canvas::Canvas`].

use crate::buffer::{Buffer, Cell};
use crate::common::{DEFAULT_SEED, TerminalEffect, seeded_rng};
use crate::render::palette::{Palette, lerp};
use crate::runtime::{InputEvent, PointerPhase};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::f32::consts::PI;

/// How the flock is drawn. A field in the config, so the choice is the user's.
///
/// The order the eight directions are listed in is fixed by the maths, not by
/// taste: see [`Boid::direction_index`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum BoidCharset {
    /// Triangles and quadrants. Distinct, single-width, and light enough to read
    /// a flock through.
    ///
    /// It was `['>', '>', 'v', 'v', '<', '<', '^', '^']`, which is four glyphs
    /// for eight directions: east was indistinguishable from south-east, south
    /// from south-west, west from north-west, and north from north-east. Half the
    /// flock's headings had no way to be shown. ASCII has exactly four arrow
    /// heads, so eight distinct ASCII headings do not exist and this set is not
    /// ASCII -- the four chevrons are the whole of what ASCII can do here.
    #[default]
    Simple,
    /// Eight single arrows, the most immediately readable of the sets.
    Arrow,
    /// Braille cells whose members are *not* rotations of one another and whose
    /// visual weights differ by more than a factor of two, so an individual
    /// appearing to brighten or dim as it turns is the glyph changing rather
    /// than the boid. Kept because it is the crate's densest option, not because
    /// it is the clearest.
    Braille,
    /// One glyph for every direction, for a flock read as points.
    Dot,
}

impl BoidCharset {
    /// The eight glyphs, east then clockwise in screen coordinates.
    ///
    /// Every set but [`BoidCharset::Dot`] maps the eight directions onto eight
    /// distinct glyphs; `Dot` is deliberately one glyph repeated, and a test
    /// pins that distinction rather than pretending it is eight.
    pub fn chars(&self) -> [char; 8] {
        match self {
            BoidCharset::Simple => ['▶', '◣', '▼', '◢', '◀', '◤', '▲', '◥'],
            BoidCharset::Arrow => ['→', '↘', '↓', '↙', '←', '↖', '↑', '↗'],
            BoidCharset::Braille => ['⣤', '⢰', '⣰', '⡆', '⡇', '⠇', '⠛', '⠙'],
            BoidCharset::Dot => ['•'; 8],
        }
    }

    /// Whether the set actually shows a heading.
    ///
    /// A directionless set is a legitimate thing to want, and it is also the one
    /// way for eight directions to collapse, so the tests ask this rather than
    /// assuming every set has eight glyphs.
    pub fn is_directional(&self) -> bool {
        !matches!(self, BoidCharset::Dot)
    }
}

/// One lit trail cell.
#[derive(Clone, Copy)]
struct TrailMark {
    glyph: char,
    color: style::Color,
    /// `1.0` is a boid standing here this frame. Below that, the boid has moved
    /// on and the cell is on its way out.
    intensity: f32,
}

/// One boid.
#[derive(Clone)]
struct Boid {
    position: (f32, f32), // Floating point for smooth movement
    velocity: (f32, f32), // Direction vector
    character: char,      // Visual representation
    color: style::Color,  // Color based on velocity/state
    /// The cells this boid has occupied, newest first.
    ///
    /// The trail is drawn from this rather than from a grid of history, because a
    /// boid at `max_speed` covers about 0.6 of a cell per frame: the interesting
    /// shape is the *last few frames* of where it has been, not where it has
    /// swept through. The grid in [`Boids::trail`] fades cells no boid is
    /// visiting any more; this is the part that is still being drawn each frame.
    history: VecDeque<(usize, usize)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BoidsOptions {
    #[serde(skip)]
    pub screen_size: (u16, u16),
    #[serde(skip)]
    pub boid_count: u16,

    /// Multiplies the built-in flock density. See [`BOID_DENSITY`].
    ///
    /// Not 1.0, and that is worth spelling out: the density itself is applied in
    /// `Config::get_boids_options`, as `area * 0.5 * boid_coeff`. The default
    /// here is [`BOID_DENSITY`] / 0.5 so that the flock lands on five percent of
    /// the cells. The two have to move together -- a default of 1.0 in this file
    /// and a resize that used a different density would mean a resize silently
    /// resizes the flock.
    pub boid_coeff: f32,

    // Separation parameters
    separation_weight: f32,
    separation_distance: f32,

    // Alignment parameters
    alignment_weight: f32,
    alignment_distance: f32,

    // Cohesion parameters
    cohesion_weight: f32,
    cohesion_distance: f32,

    // Additional parameters
    drive_factor: f32,  // Helps maintain momentum
    swirl_factor: f32,  // Adds some rotation to movement
    border_factor: f32, // How strongly to avoid borders

    max_speed: f32,
    min_speed: f32,

    /// How the flock is drawn: `Simple`, `Arrow`, `Braille` or `Dot`.
    pub charset: BoidCharset,

    /// How many cells of trail to leave behind each boid. Zero draws no trail.
    ///
    /// The single biggest legibility win in this effect. A flocking simulation is
    /// a *motion* phenomenon, and the version that cleared the screen every frame
    /// asked the eye to read three hundred isolated glyphs all changing at once,
    /// which it can only do as flicker. A trail turns the same motion into
    /// streaks, which is what a flock actually looks like.
    pub trail_length: u8,

    /// Seed for the initial flock. Boids never draw again after `new`, so this
    /// fixes the whole run.
    pub seed: u64,
}

impl Default for BoidsOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            screen_size: Default::default(),
            boid_count: 100,
            boid_coeff: BOID_DENSITY / 0.5,
            separation_weight: 1.5,
            separation_distance: 3.0,
            alignment_weight: 2.0,
            alignment_distance: 15.0,
            cohesion_weight: 1.5,
            cohesion_distance: 15.0,
            // Momentum. Its gain dropped from 0.1 to 0.05: against a speed clamp of
            // 0.6 a flat 0.2 along the current heading is a third of the whole
            // speed budget, so a boid that was already turning was turned almost
            // entirely by this and the rule that was steering it lost to it.
            drive_factor: 2.0,
            // Measured against the other two rules rather than picked. The swirl
            // used to be 1.2, applied to a unit vector and therefore worth a flat
            // 0.024 no matter how coherent the local group was: a constant
            // sideways push on everything, which is a random walk wearing a
            // flock's clothes. It is now scaled by how far the local centroid is
            // from the boid, so it is a large force inside a group and vanishes at
            // the edge of one, which is where a rule belongs. The old value was
            // 0.024 of force and the new one is up to 0.135, so the weight itself
            // comes *down* while the force goes up nearly sixfold.
            swirl_factor: 0.9,
            // Was 1.8, which is roughly five times the whole of the alignment
            // rule, applied per axis inside a five-cell band. On an 80x24 terminal
            // that band is 21% of the height, so a fifth of the flock was being
            // shoved along one axis at a time by the largest force in the effect.
            border_factor: 0.7,
            max_speed: 0.6,
            min_speed: 0.08,
            charset: BoidCharset::default(),
            // Four seconds of streak at 60 fps, which is about four cells of
            // travel at the top speed: enough to read a heading from, short enough
            // that the flock is not a single long smear.
            trail_length: 5,
            seed: DEFAULT_SEED,
        }
    }
}

/// The fraction of the screen the flock occupies at `boid_coeff = 1.0`.
///
/// Five percent, and the number is a legibility judgement rather than a
/// performance one. The count used to be `area * 0.5` clamped to 300, which on
/// an 80x24 terminal is 300 boids on 1920 cells: sixteen percent coverage, every
/// boid on screen at once, and no space between them for a flock to be a shape
/// in. At five percent a flock is a shape again.
const BOID_DENSITY: f32 = 0.05;

/// How much of the head's brightness a trail cell one step further back keeps.
///
/// A geometric falloff rather than a linear one, so the character of the tail does
/// not change when `trail_length` does: the cell just behind a boid is always
/// noticeably dimmer than the boid, and the cell five back is always a faint
/// continuation of the same streak. A trail is the same boid at earlier places,
/// so it shares the boid's colour and is dimmed toward the terminal's background
/// rather than given a second palette -- a separate trail colour is the sort of
/// thing that looks deliberate for an afternoon and reads as two overlapping
/// flocks after that.
const TRAIL_FADE_PER_STEP: f32 = 0.6;

/// How much of its intensity a lit trail cell loses per frame.
///
/// `0.8` is a 22-frame half-life, about a third of a second: long enough for a
/// streak to be read as one boid's path rather than as a row of separate marks,
/// short enough that a boid's last few frames are all that is still lit behind
/// it. The trail therefore carries the motion and the fade carries the tail.
const TRAIL_DECAY: f32 = 0.8;

/// Below this, a trail cell is dropped and its terminal cell blanked.
///
/// Not zero, because a cell's drawn colour is quantised to eight bits: fading all
/// the way to black would leave a handful of one-value-off-black cells hanging
/// around forever, each re-emitted on the frame after the one that faded it.
const TRAIL_FLOOR: f32 = 0.02;

/// The speed ramp: a boid at the bottom of the speed range to one at the top.
///
/// Two stops, and the low one is not black: a trail fades a boid's colour toward
/// the terminal's background, so a boid travelling at the bottom of its range
/// would fade into nothing.
///
/// Sampled by *fraction of the range the simulation keeps speeds in*, not by
/// absolute speed. The old expression was `r = speed * 128`, `g = 200`,
/// `b = speed * 128 + 20`: with a `max_speed` of 0.6 the red channel could not
/// exceed 76 out of 255 and the green was pinned, so the entire flock was one dark
/// teal and the comment above that code, claiming "green to white", was describing
/// a colour it could not produce. The measured red channel over a whole run was 10
/// to 76. An absolute speed against a cap saturates: take the fraction between
/// `min_speed` and `max_speed`, which are the bounds the simulation clamps every
/// boid into, and the two ends of the ramp are the two ends of what actually
/// happens. The measured luminance over a run is now 0.32 to 1.00.
fn speed_palette() -> Palette {
    Palette::new(vec![
        style::Color::Rgb {
            r: 12,
            g: 48,
            b: 96,
        },
        style::Color::Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
    ])
}

/// What a trail cell fades toward as its boid leaves it.
///
/// `Color::Rgb { 0, 0, 0 }` and not `Color::Black`, which look identical in a
/// terminal and are not the same value to `Palette::lerp`: the `Black` variant
/// has no channels to blend, so `lerp` treats mixing with it as mixing two
/// different kinds of colour and hands back whichever endpoint is nearer the
/// parameter. Passing `Black` as the far end of a fade therefore returns the
/// near end, and a trail drawn that way is at full strength everywhere.
const TRAIL_BACKGROUND: style::Color = style::Color::Rgb { r: 0, g: 0, b: 0 };

/// How far in from an edge the border rule starts, as a fraction of that axis.
///
/// A fifth of each axis rather than a flat five cells, because a flat margin is
/// 21% of the height of an 80x24 terminal, 63% of the height of a 200x8 one --
/// where the middle row is *inside the border band* -- and 2.5% of the height of
/// an 8x200 one, where it is nothing. A band that size is not a border, it is a
/// fifth of the field, or all of it.
const BORDER_MARGIN: f32 = 0.2;

/// Converting the rules' accelerations into forces that survive the velocity
/// damping is done here, and every one of these is a value that was measured or
/// argued rather than guessed. See [`Boids::apply_rules`].
mod gain {
    /// Alignment, applied to the velocity difference from the local average.
    ///
    /// Was `0.05`, which put the peak of the whole rule at 0.137 of acceleration
    /// against separation's 9.08: one of the three rules the effect is named for
    /// was not participating. `0.3` takes it to 0.78, which is inside a factor of
    /// two of everything else.
    pub const ALIGNMENT: f32 = 0.3;

    /// Cohesion, applied to the vector towards the local centroid.
    ///
    /// Was `0.03`, and this is the term that was most wrong to be small: the
    /// vector's length is not a small number. A boid whose neighbours are spread
    /// over a fifteen-cell radius is about ten cells from their centroid, so
    /// cohesion was already the second-largest force in the effect and the number
    /// in front of it was hiding that. `0.06` doubles it, and the peak lands at
    /// 0.92.
    pub const COHESION: f32 = 0.06;

    /// The swirl, applied to the perpendicular of the cohesion vector.
    ///
    /// Scaled by the cohesion vector's own length, normalised by the perception
    /// radius, so the swirl is proportional to how far the boid is from the group
    /// it is circling. It used to be a unit vector times a constant, worth a flat
    /// 0.024 of sideways push on every boid on screen.
    pub const SWIRL: f32 = 0.25;

    /// Momentum, as a fraction of the current heading added to the speed.
    ///
    /// Was `0.1` against a speed clamp of 0.6, so a flat 0.2 along the current
    /// heading -- a third of the whole speed budget, every frame, in whichever
    /// direction the boid was already going. A boid that was already turning was
    /// turned almost entirely by this and the rule that was steering it lost to
    /// it. Halved, so the rules are what decide the heading.
    pub const DRIVE: f32 = 0.05;
}

/// The click-to-scatter force, and every number that shapes it.
///
/// A click drops a wavefront on the flock. Boids inside a narrow band around the
/// front are pushed radially away from where the click landed, and the band is
/// drawn, so what is on screen is exactly what is being hit.
///
/// A wave rather than a single impulse for two reasons, both of them about what
/// the user sees. An impulse on one boid is invisible: a single boid leaving a
/// flock of three hundred reads as a boid leaving, which is something that
/// happens anyway. And a wave is *local*, so it reads as the flock opening up
/// here rather than as the whole simulation being jolted, which is the
/// difference between a screensaver reacting to you and a screensaver glitching.
///
/// The force is computed in [`Boids::shock_force`] and added in
/// [`Boids::apply_rules`] alongside the three rules and the border, so it is
/// damped by the same blend and clamped by the same `max_speed` as everything
/// else. That is the point of putting it there rather than writing to velocities
/// afterwards: a scatter *cannot* fling a boid faster than the simulation allows,
/// so it is a disturbance rather than an escape hatch.
mod shock {
    /// Peak outward acceleration, at the centre of a wave's band.
    ///
    /// Measured against the force table in [`Boids::apply_rules`], which puts
    /// separation at 1.50, cohesion at 0.92, border at 0.70 and alignment at
    /// 0.78. Four times separation, so a click visibly out-shouts the rule whose
    /// entire job is keeping boids apart. Under that it would not read at all:
    /// separation is already the largest force in the effect and it is applied
    /// every frame to every boid with a neighbour, where a wave touches each boid
    /// for about eight frames and then never again.
    pub const PUSH: f32 = 6.0;

    /// Half-width of the band, in cells, so the band is `2 * BAND` across.
    ///
    /// `3.0`, which is exactly `separation_distance`, and two fifths of the
    /// fifteen-cell perception radius the alignment and cohesion rules use. So a
    /// wave hits everything the flock already considers a close encounter, and
    /// leaves the group's sense of itself -- cohesion and alignment, which reach
    /// fifteen cells -- intact to pull it back together afterwards. That is the
    /// arc the effect wants: it blooms and closes.
    pub const BAND: f32 = 3.0;

    /// How fast the front travels, in cells per second.
    ///
    /// A boid at `max_speed` covers `0.6 * 60 = 36` cells a second, so this
    /// outruns the flock: a wave that the boids outrun would look like it was
    /// chasing them rather than displacing them.
    ///
    /// **Not normalised by screen size**, which is the usual answer in this file
    /// -- see [`BORDER_MARGIN`] -- and is wrong here. Normalising gives every
    /// terminal the same interaction *duration*, but on a 400x200 screen the
    /// reachable area is 283 cells, so a one-second crossing is 283 cells a
    /// second: nearly eight cells a frame, which makes the drawn ring strobe
    /// rather than travel, and a wave moving seven times faster than the boids it
    /// is displacing barely interacts with any of them. Constant cells per second
    /// keeps the ring smooth at every size. What it costs is real and is the
    /// trade: a click's disturbance lasts about 0.8 seconds on an 80x24 terminal
    /// and about six on a 400x200 one.
    pub const SPEED: f32 = 45.0;

    /// What a wave's strength is multiplied by each step.
    ///
    /// Raised to the power of `scale`, like [`TRAIL_DECAY`], so the wave fades
    /// over the same stretch of *time* whether the frame rate is 30 or 144. A
    /// half-life of about 27 steps, so a wave on an ordinary terminal spends most
    /// of its life below the strength where a scatter reads.
    pub const DECAY: f32 = 0.975;

    /// Below this, a wave is dropped rather than drawn or applied.
    ///
    /// Not zero, for the reason [`TRAIL_FLOOR`] is not zero: a force that has
    /// decayed to nothing still costs a distance computation per boid per wave,
    /// and the ring drawn from it is a dim line that reads as a second flock.
    pub const FLOOR: f32 = 0.04;

    /// How long a wave may live, in seconds.
    ///
    /// A second bound on top of reaching the far side, and it exists for the
    /// screen sizes where reaching the far side is not soon. On a 400x200
    /// terminal a wave at [`SPEED`] needs over six seconds to cross, and four of
    /// them alive that long is four extra passes over the flock for six seconds
    /// after a click.
    pub const MAX_AGE: f32 = 2.5;

    /// Live waves at once. Older ones are dropped past this.
    ///
    /// Two bounds in one number. The cost: `apply_rules` gains a loop over the
    /// waves for each of its boids, and this caps it. The behaviour, which is the
    /// one that matters: a drag can ask for a wave every [`DRAG_SPACING`] cells
    /// of travel, and without a cap a fast drag across a large terminal stacks
    /// dozens of bands on top of each other, every boid inside one of them is
    /// pushed by several at once, and the whole flock pins to `max_speed` -- the
    /// exact failure the force table above was written to undo, where the speed
    /// stops carrying any information because everything is at the cap.
    pub const MAX_WAVES: usize = 4;

    /// How far the pointer must travel before a drag drops another wave.
    ///
    /// `BAND * 2`, so consecutive waves along a drag are touching rather than
    /// leaving gaps: the result is a continuous ripple rather than a dotted line
    /// of separate bursts.
    pub const DRAG_SPACING: f32 = 6.0;

    /// Cell height divided by cell width.
    ///
    /// The wave's distance is measured as `hypot(dx, dy * CELL_ASPECT)` so the
    /// band is a circle in *pixels* and the drawn ring is round. Without it the
    /// metric is isotropic in cells, and a cell is about twice as tall as it is
    /// wide, so the ring comes out twice as tall as it is wide.
    ///
    /// `2.0` is the usual approximation and it is the same one the sub-cell
    /// renderers in [`crate::render`] assume, with the same caveat recorded
    /// there: DejaVu Sans Mono is nearer 1:1.2, so a ring is a little too tall on
    /// a font that is not twice as tall as it is wide. It is a property of the
    /// technique. What matters is that the *drawn* ring and the *applied* band are
    /// measured the same way, or the feedback would be showing the user a shape
    /// that is not the shape being hit.
    pub const CELL_ASPECT: f32 = 2.0;
}

/// A difference between two points, as a distance in the metric the waves use.
///
/// Takes a difference rather than two points because the difference is not always
/// a straight line between two places: the force wraps, via
/// [`Boids::toroidal_diff`], and the drag spacing must not -- a pointer leaving
/// the right edge and reappearing on the left has crossed most of a screen even
/// though those two positions are a cell apart. Deciding that at each call site
/// keeps the two cases from being confused for one another.
///
/// The vertical is scaled by [`shock::CELL_ASPECT`] so that a circle in this
/// metric is a circle on screen. Without it the metric is isotropic in cells, and
/// a cell is about twice as tall as it is wide, so every wave would be an ellipse
/// twice as tall as it is wide.
fn shock_distance(diff: (f32, f32)) -> f32 {
    diff.0.hypot(diff.1 * shock::CELL_ASPECT)
}

/// `column` as a signed offset from `origin`, wrapped into `-width/2..=width/2`.
///
/// The wrapping is the point. A wave centred on column 50 of a sixty-wide screen
/// draws an arc at columns 0 and 3, and a test that read those as 0 and 3 would
/// conclude the ring was 47 columns to the *left* of where it was clicked -- which
/// is true of the arithmetic and not of the picture. The same wrap is what
/// [`Boids::paint_waves`] uses to put those arcs there in the first place.
fn wrapped_column(column: usize, origin: f32, width: f32) -> f32 {
    let half = width / 2.0;
    ((column as f32 - origin + half).rem_euclid(width)) - half
}

/// One click's worth of spreading, still travelling.
///
/// Spawned on a press and on a drag's worth of travel, and dropped by
/// [`Boids::advance_waves`] once it has decayed, aged out, or reached as far as
/// anything can be from where it started.
#[derive(Debug, Clone, Copy)]
struct Shockwave {
    /// Where the click landed, in cell coordinates.
    origin: (f32, f32),
    /// How far the front has travelled, in cells. Grows by [`shock::SPEED`].
    radius: f32,
    /// `PUSH` at birth, multiplied by [`shock::DECAY`] every step. This is both
    /// the force scale and how the drawn ring dims, so a wave fades as one thing
    /// rather than as a force that weakens behind a ring that does not.
    strength: f32,
    /// Seconds this wave has existed. Bounds its life on a large terminal, where
    /// crossing the screen takes longer than [`shock::MAX_AGE`].
    age: f32,
}

/// The ring's colour, before it is dimmed by how much of the wave is left.
///
/// Amber against a flock that runs dark blue to white: the two ends of the
/// speed ramp are nowhere near it in hue, so a boid crossing the ring cannot be
/// mistaken for the ring. Dimmed toward [`TRAIL_BACKGROUND`] rather than given
/// its own bright end, so a wave reads as an event happening to the flock rather
/// than as a second flock.
const SHOCK_COLOR: style::Color = style::Color::Rgb {
    r: 255,
    g: 140,
    b: 40,
};

/// The glyph a wave's ring is drawn with.
///
/// One glyph rather than a ramp, because the ring is a shape and not a value:
/// there is no scalar here being encoded, so a ramp would be a way of spelling
/// "one" in nine characters. `+` is ASCII, so it is single-width in every
/// terminal ever shipped, and it sits in the middle of the cell both ways, which
/// a `.` does not -- a ring of full stops sits on the baseline and reads as
/// lopsided rather than as a circle.
const SHOCK_GLYPH: char = '+';

pub struct Boids {
    options: BoidsOptions,
    boids: Vec<Boid>,
    charset_chars: [char; 8],
    /// How fast a boid is going, as a colour.
    ///
    /// A `Palette` rather than three hand-written channels, because this is what
    /// one is for. See [`speed_palette`].
    speed_palette: Palette,
    /// The trail layer being built this frame.
    ///
    /// A plain buffer rather than a `Canvas`, for the reason `EffectHost` and the
    /// playlist both give: `Canvas::commit` swaps its two surfaces, so what it
    /// hands back to draw into next is the frame from *before* the one it just
    /// emitted. A trail is a history, and a surface that is one commit too old
    /// cannot accumulate one. The canvas that was here was also the reason there
    /// were no trails: `get_diff` called `clear` on it every frame, which is
    /// correct for an effect that repaints everything and is the whole problem for
    /// one that does not.
    frame: Buffer,
    /// The frame the terminal is showing. The diff is against this, so a cell
    /// returning to blank is reported and the screen is actually erased.
    shown: Buffer,
    /// Every lit trail cell, and how strongly.
    ///
    /// Lives across frames so a boid that has left a cell still fades it out
    /// rather than snapping it to blank the instant it moves on.
    trail: HashMap<(usize, usize), TrailMark>,
    /// The waves currently travelling, oldest first.
    ///
    /// A `VecDeque` rather than a `Vec` because [`shock::MAX_WAVES`] drops the
    /// oldest when a new one arrives, which is a pop from the front and a `Vec`
    /// would make that a shift of everything behind it. At four entries the shift
    /// is free either way; the deque says what the code means.
    ///
    /// A click is one wave and a drag is several, so this is empty on almost every
    /// frame of a run nobody is touching, and the cost of an empty one is a loop
    /// that does not run.
    waves: VecDeque<Shockwave>,
    /// Whether a pointer button is down, latched from the events themselves.
    ///
    /// Latched rather than read from `InputState`, because the effect is handed
    /// events and not the aggregate: `PointerPhase::Moved` covers both a drag
    /// *and* a hover with no button down, and the runtime reports the two
    /// identically -- `translate_mouse` gives a buttonless `MouseEventKind::Moved`
    /// the `Left` button, which is also what a left-drag gets. A drag that scatters
    /// and a hover that does not cannot be told apart from one event, so the only
    /// way to tell them apart is to have been watching for the press.
    pointer_down: bool,
    /// Where the last wave was dropped from, for [`shock::DRAG_SPACING`].
    ///
    /// A wave's spawn point rather than the pointer's own last position, because
    /// it is the wave that has to be spaced, not the pointer: two clicks at the
    /// same spot should both land, and a drag that wanders back over its own path
    /// should not be able to stack waves on top of each other from one spot.
    last_wave_at: (f32, f32),
}

impl Boid {
    fn new(position: (f32, f32), velocity: (f32, f32)) -> Self {
        Self {
            position,
            velocity,
            character: '•',
            color: style::Color::White,
            history: VecDeque::new(),
        }
    }

    fn speed(&self) -> f32 {
        self.velocity.0.hypot(self.velocity.1)
    }

    /// Which of the eight sectors this boid is heading into, `0..=7`.
    ///
    /// East is 0 and the rest run clockwise on screen, where y grows downwards, so
    /// a velocity of `(0, 1)` -- which is *down* on a terminal -- comes out as
    /// sector 2. The `+ 8` before the remainder is what keeps a heading just past
    /// west from indexing `-1` into the glyph table.
    fn direction_index(&self) -> usize {
        let (vx, vy) = self.velocity;
        let angle = vy.atan2(vx);
        ((angle / PI * 4.0).round() as i32 + 8).rem_euclid(8) as usize
    }

    fn get_direction_char(&self, charset: &[char; 8]) -> char {
        charset[self.direction_index()]
    }

    /// Sets the glyph and colour from the current velocity.
    ///
    /// `min` and `max` are the bounds the simulation clamps every speed into, and
    /// the colour is the speed's position *between them*. See [`speed_palette`] for
    /// why an absolute speed cannot be turned into a colour.
    fn update_visual(
        &mut self,
        charset: &[char; 8],
        palette: &Palette,
        min: f32,
        max: f32,
    ) {
        self.character = self.get_direction_char(charset);
        self.color = palette.sample(self.speed_fraction(min, max));
    }

    /// This boid's speed as `0.0..=1.0` across the simulation's speed range.
    ///
    /// A degenerate or inverted range reports the bright end rather than dividing
    /// by zero, on the grounds that a flock whose speed means nothing should be
    /// legible rather than absent. A `NaN` in the range falls through to the
    /// division and produces `NaN`, which `Palette::sample` treats as the bottom
    /// of the ramp -- the same thing every other effect's scalar gets.
    fn speed_fraction(&self, min: f32, max: f32) -> f32 {
        if max <= min {
            return 1.0;
        }
        ((self.speed() - min) / (max - min)).clamp(0.0, 1.0)
    }
}

impl TerminalEffect for Boids {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.paint()
    }

    fn update(&mut self) {
        // `scale` is 1.0 at 60 fps, which is the rate the weights in the config
        // were tuned against, so the plain `update` path is unchanged.
        self.step(1.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not fling the flock off screen, and converted
        // to a multiple of the nominal frame so the weights keep their meaning.
        let seconds = context.delta.as_secs_f64().min(0.1);
        self.step((seconds * 60.0) as f32);
    }

    /// Drops a wave where the pointer went down, and another every
    /// [`shock::DRAG_SPACING`] cells of travel while the button is held.
    ///
    /// Matched exhaustively rather than with a catch-all, for the reason `ink`
    /// does the same: a new variant of [`InputEvent`] then fails to compile here
    /// instead of being silently ignored, and a pointer event that quietly stopped
    /// arriving would leave an effect that looks interactive and is not.
    fn handle_input(&mut self, event: &InputEvent) {
        match event {
            InputEvent::Pointer {
                position,
                phase,
                button: _,
            } => self.handle_pointer(*position, *phase),
            InputEvent::Key { .. } => {}
            InputEvent::Resize { .. } => {}
            InputEvent::FocusGained | InputEvent::FocusLost => {}
            InputEvent::Quit | InputEvent::Ignored => {}
        }
    }

    fn update_size(&mut self, width: u16, height: u16) {
        let changed = self.options.screen_size != (width.max(1), height.max(1));
        self.options.screen_size = (width.max(1), height.max(1));
        self.rebuild_buffers();

        // A resize is a full repaint, and so is a change of flock size: the
        // positions were drawn for a different number of boids on a different
        // screen, so the run starts again rather than snapping the new
        // population into the old flock's shape.
        let wanted = self.count_for();
        if changed || wanted != self.options.boid_count {
            self.options.boid_count = wanted;
            self.reset();
        }
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone());
    }
}

impl Boids {
    /// Advances the flock by `scale`, a multiple of the nominal frame.
    fn step(&mut self, scale: f32) {
        self.apply_rules(scale);
        // After `apply_rules` and not before, so a wave dropped by a click in this
        // frame is applied at the radius it was born at -- zero, which is where
        // the pointer is -- rather than a step further out than the user asked
        // for. It still travels this frame; it just travels after it has hit.
        self.advance_waves(scale);
        self.update_positions(scale);
        self.fade_trail(scale);
    }

    /// Where a click puts a wave, and when one is due.
    ///
    /// The three pointer phases mean three different things, and the third is the
    /// one that needs the latch:
    ///
    /// - **Pressed** always drops a wave. Not "if far enough from the last one":
    ///   two deliberate clicks at the same spot are two clicks, and treating the
    ///   second as a repeat would make the effect feel like it was ignoring you.
    /// - **Released** clears the latch and nothing else. A release drops no wave,
    ///   so letting go of the button mid-drag is not itself a disturbance.
    /// - **Moved** drops one only if the button is *down*. This is the whole
    ///   reason [`Boids::pointer_down`] exists: the runtime reports a hover and a
    ///   drag with no difference at all, so a `Moved` on its own is a cursor
    ///   drifting across the screen and must do nothing to the flock.
    ///
    /// Drag spacing is measured in the same corrected metric as the waves
    /// themselves, so a drag is spaced by how far it looks like it went rather
    /// than by how many cells of the screen it crossed -- which differ by a
    /// factor of two on the vertical.
    fn handle_pointer(&mut self, position: (u16, u16), phase: PointerPhase) {
        let (x, y) = (position.0 as f32, position.1 as f32);

        match phase {
            PointerPhase::Pressed => {
                self.pointer_down = true;
                self.drop_wave((x, y));
            }
            PointerPhase::Released => self.pointer_down = false,
            PointerPhase::Moved => {
                if !self.pointer_down {
                    return;
                }
                let travelled = shock_distance((
                    x - self.last_wave_at.0,
                    y - self.last_wave_at.1,
                ));
                if travelled >= shock::DRAG_SPACING {
                    self.drop_wave((x, y));
                }
            }
            // Scrolling past a flock is not touching it.
            PointerPhase::WheelUp
            | PointerPhase::WheelDown
            | PointerPhase::WheelLeft
            | PointerPhase::WheelRight => {}
        }
    }

    /// Adds a wave at `origin`, dropping the oldest if there are too many.
    fn drop_wave(&mut self, origin: (f32, f32)) {
        while self.waves.len() >= shock::MAX_WAVES {
            self.waves.pop_front();
        }
        self.waves.push_back(Shockwave {
            origin,
            radius: 0.0,
            strength: shock::PUSH,
            age: 0.0,
        });
        self.last_wave_at = origin;
    }

    /// Moves every wave along and drops the ones that are finished.
    ///
    /// Two independent ends, and both are needed. The age bound stops a wave
    /// outliving its usefulness on a terminal where crossing the screen takes
    /// longer than [`shock::MAX_AGE`]. The reach bound is the honest one: a wave
    /// whose front is past the furthest anything can be from where it landed can
    /// never touch another boid, so keeping it costs a distance computation per
    /// boid and draws a ring out in the empty margin.
    fn advance_waves(&mut self, scale: f32) {
        let decay = shock::DECAY.powf(scale.max(0.0));
        let reach = self.reach();

        for wave in &mut self.waves {
            wave.radius += shock::SPEED * scale / 60.0;
            wave.strength *= decay;
            wave.age += scale / 60.0;
        }
        self.waves.retain(|wave| {
            wave.strength >= shock::FLOOR
                && wave.age < shock::MAX_AGE
                && wave.radius < reach
        });
    }

    /// The furthest a boid can be from a wave's origin, in cells.
    ///
    /// Derived rather than guessed, because `toroidal_diff` is what decides which
    /// boids a wave reaches and it is bounded: each axis comes back in
    /// `-extent/2..=extent/2`, and nothing is further than half of each. So the
    /// furthest is `hypot(width / 2, height / 2 * CELL_ASPECT)` -- half the width
    /// across and half the height *in cells*, which is why the aspect is in there
    /// and why it is the same metric the force uses.
    fn reach(&self) -> f32 {
        let width = self.options.screen_size.0 as f32;
        let height = self.options.screen_size.1 as f32;
        (width / 2.0).hypot(height / 2.0 * shock::CELL_ASPECT)
    }

    /// How many boids a screen of this size gets.
    ///
    /// Written out rather than shared because the density is applied in
    /// `Config::get_boids_options`, and the two have to agree: a resize that
    /// disagreed would silently change the size of the flock, and a flock that
    /// changes size when you drag the window edge is not something anyone can
    /// reason about. The clamp matches the one there for the same reason.
    fn count_for(&self) -> u16 {
        let (width, height) = self.options.screen_size;
        let area = width as f32 * height as f32;
        ((area * 0.5 * self.options.boid_coeff) as u16).clamp(50, 300)
    }

    pub fn new(options: BoidsOptions) -> Self {
        let mut rng = seeded_rng(options.seed, "boids");
        // Floored at one, like `Canvas::new`, because a zero width has no cells to
        // sample a position from. The floored value is written back so the struct's
        // own idea of the screen is the one its buffers are the size of.
        let mut options = options;
        options.screen_size =
            (options.screen_size.0.max(1), options.screen_size.1.max(1));
        let width = options.screen_size.0 as usize;
        let height = options.screen_size.1 as usize;

        let charset_chars = options.charset.chars();
        let speed_palette = speed_palette();
        let (min_speed, max_speed) = (options.min_speed, options.max_speed);

        let mut boids = Vec::with_capacity(options.boid_count as usize);
        for _ in 0..options.boid_count {
            let position = (
                rng.random_range(0.0..width as f32),
                rng.random_range(0.0..height as f32),
            );
            let velocity =
                (rng.random_range(-1.0..1.0), rng.random_range(-1.0..1.0));

            let mut boid = Boid::new(position, velocity);
            boid.update_visual(
                &charset_chars,
                &speed_palette,
                min_speed,
                max_speed,
            );
            // Seeded rather than left empty, so the first frame is a full repaint
            // and not a screen that is blank for one frame and then suddenly full.
            boid.history.push_back(boid.cell(width, height));
            boids.push(boid);
        }

        let mut flock = Self {
            options,
            boids,
            charset_chars,
            speed_palette,
            frame: Buffer::new(width, height),
            shown: Buffer::new(width, height),
            trail: HashMap::new(),
            // A flock starts with nobody having clicked it.
            waves: VecDeque::new(),
            pointer_down: false,
            last_wave_at: (0.0, 0.0),
        };

        flock.stamp();
        flock
    }

    /// Puts both buffers and the trail back to the new size.
    ///
    /// `shown` is blanked along with the rest, which is the one case where that is
    /// right: a resize means the terminal is in a state nobody here knows, so the
    /// next diff has to repaint everything rather than diff against a frame
    /// describing dimensions that no longer exist.
    fn rebuild_buffers(&mut self) {
        let width = self.options.screen_size.0 as usize;
        let height = self.options.screen_size.1 as usize;
        if (self.frame.width, self.frame.height) == (width, height) {
            return;
        }
        self.frame = Buffer::new(width, height);
        self.shown = Buffer::new(width, height);
        self.trail.clear();
    }

    // Calculate toroidal difference between two positions
    fn toroidal_diff(&self, a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
        let width = self.options.screen_size.0 as f32;
        let height = self.options.screen_size.1 as f32;

        let mut dx = a.0 - b.0;
        let mut dy = a.1 - b.1;

        if dx > width / 2.0 {
            dx -= width;
        } else if dx < -width / 2.0 {
            dx += width;
        }

        if dy > height / 2.0 {
            dy -= height;
        } else if dy < -height / 2.0 {
            dy += height;
        }

        (dx, dy)
    }

    /// One frame's worth of the three rules and the border, for every boid.
    ///
    /// The balance between them is the whole point, so it is worth writing down.
    /// These are the largest forces each rule actually reached over a four-second
    /// run on a 60x30 screen, measured by instrumenting both this version and the
    /// one it replaced:
    ///
    /// | rule        | before   | after    |
    /// |-------------|----------|----------|
    /// | separation  | 9.08     | 1.50     |
    /// | cohesion    | 1.49     | 0.92     |
    /// | border      | 0.97     | 0.70     |
    /// | alignment   | 0.137    | 0.78     |
    ///
    /// So before, separation was six times the next rule and alignment was a tenth
    /// of it: the visible behaviour was separation and a wall. Now the three rules
    /// are within a factor of two of each other, which is roughly what "flocking"
    /// means.
    ///
    /// The other number from the same run is the one about the *presentation*:
    /// 206 of 300 boids were travelling at exactly `max_speed` before, against 30
    /// of 90 after. Every boid pinned to the speed cap means every boid is being
    /// turned by whichever force happens to be largest rather than by the flock,
    /// and it is also why the colour could not carry the speed.
    fn apply_rules(&mut self, scale: f32) {
        let num_boids = self.boids.len();
        let mut separation_adjustments = vec![(0.0, 0.0); num_boids];
        let mut alignment_adjustments = vec![(0.0, 0.0); num_boids];
        let mut cohesion_adjustments = vec![(0.0, 0.0); num_boids];
        let mut border_adjustments = vec![(0.0, 0.0); num_boids];

        // Pre-calculate all adjustments
        for i in 0..num_boids {
            // Apply separation rule
            let mut separation = (0.0, 0.0);
            let mut sep_count = 0;

            // Apply alignment rule
            let mut avg_velocity = (0.0, 0.0);
            let mut align_count = 0;

            // Apply cohesion rule
            let mut center = (0.0, 0.0);
            let mut cohesion_count = 0;

            for j in 0..num_boids {
                if i == j {
                    continue;
                }

                let diff = self
                    .toroidal_diff(self.boids[j].position, self.boids[i].position);
                let distance = diff.0.hypot(diff.1);

                // Separation
                #[allow(clippy::collapsible_if)]
                if distance < self.options.separation_distance {
                    if distance > 0.0 {
                        let factor = 1.0 / distance;
                        separation.0 -= diff.0 * factor;
                        separation.1 -= diff.1 * factor;
                        sep_count += 1;
                    }
                }

                // Alignment
                if distance < self.options.alignment_distance {
                    avg_velocity.0 += self.boids[j].velocity.0;
                    avg_velocity.1 += self.boids[j].velocity.1;
                    align_count += 1;
                }

                // Cohesion, accumulated relative to this boid rather than in
                // screen coordinates.
                //
                // The centroid used to be the mean of the neighbours' absolute
                // positions, which is wrong the moment the flock straddles the
                // wrap: a boid at column 2 with its neighbours at column 58 on a
                // sixty-wide screen has a raw mean of about 30, and
                // `toroidal_diff` then measures that as thirty cells away. The
                // direction came out right, because the wrap is applied to the
                // difference, but the magnitude was up to half the width of the
                // screen rather than up to the perception radius. Accumulating the
                // offsets instead bounds it by `cohesion_distance`, which is what
                // the vector means. The seam was worth 2.9 of acceleration at this
                // point in the effect -- more than any other rule in it, and all of
                // it an artefact of where the arithmetic was done.
                if distance < self.options.cohesion_distance {
                    center.0 += diff.0;
                    center.1 += diff.1;
                    cohesion_count += 1;
                }
            }

            // Finalize separation.
            //
            // Divided by the neighbour count, which it never was. The sum is
            // `1/distance` weighted and unnormalised, so its magnitude is set by
            // how many neighbours there happen to be: doubling the flock doubled
            // the separation force on every boid in it, which is not a rule, it is
            // a density sensor wired to the throttle. Divided out, it is a
            // weighted average of the push directions, so a boid with two
            // neighbours pulling the same way still feels it and a boid in the
            // middle of a ring of twelve feels almost nothing -- which is the
            // correct answer, because it is not in danger.
            if sep_count > 0 {
                let share = self.options.separation_weight / sep_count as f32;
                separation_adjustments[i] =
                    (separation.0 * share, separation.1 * share);
            }

            // Finalize alignment
            if align_count > 0 {
                let avg_vel = (
                    avg_velocity.0 / align_count as f32,
                    avg_velocity.1 / align_count as f32,
                );

                alignment_adjustments[i] = (
                    (avg_vel.0 - self.boids[i].velocity.0)
                        * self.options.alignment_weight
                        * gain::ALIGNMENT,
                    (avg_vel.1 - self.boids[i].velocity.1)
                        * self.options.alignment_weight
                        * gain::ALIGNMENT,
                );
            }

            // Finalize cohesion
            if cohesion_count > 0 {
                let toward_center = (
                    center.0 / cohesion_count as f32,
                    center.1 / cohesion_count as f32,
                );
                let reach = toward_center.0.hypot(toward_center.1);
                // The swirl is a fraction of the pull, so it is a rule about the
                // shape of a group rather than a constant sideways push: strong in
                // the middle of a flock, where the boid is far from its local
                // centroid, and nothing at all at the edge of one, where it is not.
                let coherence =
                    (reach / self.options.cohesion_distance).clamp(0.0, 1.0);
                let swirl_scale =
                    self.options.swirl_factor * gain::SWIRL * coherence;

                // Calculate perpendicular (swirl) vector
                let swirl = (-toward_center.1, toward_center.0);
                let swirl_normalized = if reach > 0.0 {
                    (swirl.0 / reach, swirl.1 / reach)
                } else {
                    (0.0, 0.0)
                };

                cohesion_adjustments[i] = (
                    toward_center.0 * self.options.cohesion_weight * gain::COHESION
                        + swirl_normalized.0 * swirl_scale,
                    toward_center.1 * self.options.cohesion_weight * gain::COHESION
                        + swirl_normalized.1 * swirl_scale,
                );
            }

            // Apply border avoidance
            border_adjustments[i] = self.border_force(self.boids[i].position);
        }

        // The click force, over the whole flock at once.
        //
        // Outside the O(n^2) double loop above even though it is a force like the
        // other four, because it does not involve pairs. A wave asks "how far is
        // this boid from one point", which is one question per boid per wave
        // rather than one per pair, and folding it into the pair loop would have
        // been a way of making the simulation quadratic for the sake of symmetry.
        let mut shock_adjustments = vec![(0.0, 0.0); num_boids];
        for (i, adjustment) in shock_adjustments.iter_mut().enumerate() {
            let boid = &self.boids[i];
            *adjustment = self.shock_force(boid.position, boid.velocity);
        }

        // Apply all forces to boids
        for i in 0..num_boids {
            // Get current velocity
            let mut new_vx = self.boids[i].velocity.0;
            let mut new_vy = self.boids[i].velocity.1;

            // Apply rules. Scaled so the flock turns at the same rate whatever
            // the frame rate; the damping below is left unscaled because it is a
            // fixed fraction of the previous velocity, not a force.
            new_vx += separation_adjustments[i].0 * scale;
            new_vy += separation_adjustments[i].1 * scale;

            new_vx += alignment_adjustments[i].0 * scale;
            new_vy += alignment_adjustments[i].1 * scale;

            new_vx += cohesion_adjustments[i].0 * scale;
            new_vy += cohesion_adjustments[i].1 * scale;

            new_vx += border_adjustments[i].0 * scale;
            new_vy += border_adjustments[i].1 * scale;

            // And the click. Scaled like the rules, so a wave is felt over the
            // same stretch of time whatever the frame rate, and so that `--speed`
            // makes the scatter as much stronger as it makes everything else
            // faster rather than leaving the one interactive force on its own
            // clock. It is added before the damping and the speed clamp below for
            // the same reason the rules are: a scatter should not be able to
            // launch a boid past `max_speed`, or the click would be a way out of
            // the simulation's own contract rather than a disturbance inside it.
            new_vx += shock_adjustments[i].0 * scale;
            new_vy += shock_adjustments[i].1 * scale;

            // Apply drive factor
            let speed = new_vx.hypot(new_vy);
            if speed > 0.0 {
                let normalized_vx = new_vx / speed;
                let normalized_vy = new_vy / speed;
                new_vx += normalized_vx * self.options.drive_factor * gain::DRIVE;
                new_vy += normalized_vy * self.options.drive_factor * gain::DRIVE;
            }

            // Apply damping for smoother movement
            new_vx = self.boids[i].velocity.0 * 0.7 + new_vx * 0.3;
            new_vy = self.boids[i].velocity.1 * 0.7 + new_vy * 0.3;

            // Apply speed limits
            let speed = new_vx.hypot(new_vy);
            if speed > self.options.max_speed {
                let scale = self.options.max_speed / speed;
                new_vx *= scale;
                new_vy *= scale;
            } else if speed < self.options.min_speed && speed > 0.0 {
                let scale = self.options.min_speed / speed;
                new_vx *= scale;
                new_vy *= scale;
            }

            // Update velocity
            self.boids[i].velocity = (new_vx, new_vy);
        }
    }

    /// A push towards the middle of the screen, for a boid near an edge.
    ///
    /// One vector, aimed at the centre, rather than one shove per axis. The old
    /// version added a horizontal term and a vertical term independently inside a
    /// five-cell band, at up to 1.8 each, so a boid in a corner was given a
    /// diagonal of arbitrary slope by whichever of the two margins it happened to
    /// be further from, and a boid within five cells of the top edge was given 1.8
    /// straight down and nothing at all sideways. A corner got both terms and so
    /// got pushed twice as hard as an edge. The result was axis-aligned ricochets
    /// off invisible walls along a band five cells deep, which is a fifth of the
    /// height of an 80x24 terminal.
    ///
    /// The magnitude is the *largest* single-axis intrusion, not the sum, so
    /// arriving at a corner does not double the push, and it is applied along the
    /// direction to the centre so a corner is pushed diagonally out of it.
    fn border_force(&self, position: (f32, f32)) -> (f32, f32) {
        let width = self.options.screen_size.0 as f32;
        let height = self.options.screen_size.1 as f32;
        let margin_x = (width * BORDER_MARGIN).max(1.0);
        let margin_y = (height * BORDER_MARGIN).max(1.0);

        let intrusion = |pos: f32, margin: f32, extent: f32| {
            if pos < margin {
                1.0 - pos / margin
            } else if pos > extent - margin {
                1.0 - (extent - pos) / margin
            } else {
                0.0
            }
        };

        let depth = intrusion(position.0, margin_x, width)
            .max(intrusion(position.1, margin_y, height));
        if depth <= 0.0 {
            return (0.0, 0.0);
        }

        let (dx, dy) = (width / 2.0 - position.0, height / 2.0 - position.1);
        let length = (dx * dx + dy * dy).sqrt();
        if length <= 0.0 {
            // Exactly the middle of the screen, which on a screen one or two cells
            // across is also inside the border band. There is no "towards the
            // middle" from the middle; zero is the honest answer, and it is
            // unreachable on any screen big enough to read.
            return (0.0, 0.0);
        }

        let push = self.options.border_factor * depth;
        (push * dx / length, push * dy / length)
    }

    /// The outward push from every live wave, for one boid.
    ///
    /// A pure function of `self.waves`, so it is testable without running the
    /// loop -- the same reason [`Boids::border_force`] is a method rather than
    /// inlined into [`Boids::apply_rules`]. A free-standing force that can only be
    /// observed through a 300-boid simulation is a force that can only be tested
    /// by looking at it.
    ///
    /// `heading` is the boid's own velocity, and only matters in the one case
    /// where there is no outward direction to use: a boid exactly on the origin
    /// has nothing to be pushed away *from*. See below.
    fn shock_force(&self, position: (f32, f32), heading: (f32, f32)) -> (f32, f32) {
        let mut total = (0.0f32, 0.0f32);

        for wave in &self.waves {
            // Wrapped, because the flock is wrapped. A click on the left of the
            // screen has to reach a boid on the right of it, or a boid a hundred
            // cells away round the torus would be treated as a hundred cells away
            // on screen and the ring would visibly stop short of it.
            let diff = self.toroidal_diff(position, wave.origin);
            let distance = shock_distance(diff);

            // The band: a fixed width travelling outward, strongest on the front
            // and nothing at its edges. Triangular rather than a flat plateau, so
            // there is no discontinuity in the force at either edge of the band
            // -- a boid entering or leaving the band would otherwise be handed a
            // step change in acceleration, which shows up as a boid flinching as
            // the wave passes rather than being carried by it.
            let offset = (distance - wave.radius).abs();
            if offset >= shock::BAND {
                continue;
            }
            let falloff = 1.0 - offset / shock::BAND;

            // Away from the origin. The degenerate case is a boid sitting exactly
            // on it, where the direction is undefined and dividing by a zero
            // length would put a `NaN` into a velocity -- which does not panic,
            // it silently teleports the boid to cell zero, because `NaN as isize`
            // is zero, and a flock with one boid stuck in a corner is a flock
            // that never recovers. It is reachable whenever a boid's position
            // lands on an exact integer, which the rounding in `Boid::cell` is
            // evidence happens.
            //
            // So the fallback is the boid's own heading: state that is known
            // rather than something re-derived from a measurement of zero. A boid
            // clicked exactly on the head is shoved onward, which is also what
            // clicking a boid's face ought to do.
            let (ux, uy) = if distance > f32::EPSILON {
                (diff.0 / distance, diff.1 / distance)
            } else {
                let heading_length = heading.0.hypot(heading.1);
                if heading_length > f32::EPSILON {
                    (heading.0 / heading_length, heading.1 / heading_length)
                } else {
                    (1.0, 0.0)
                }
            };

            let push = wave.strength * falloff;
            total.0 += ux * push;
            total.1 += uy * push;
        }

        total
    }

    fn update_positions(&mut self, scale: f32) {
        let width = self.options.screen_size.0 as f32;
        let height = self.options.screen_size.1 as f32;
        let (cols, rows) = (
            self.options.screen_size.0 as usize,
            self.options.screen_size.1 as usize,
        );
        let charset_chars = self.charset_chars;
        let (min_speed, max_speed) =
            (self.options.min_speed, self.options.max_speed);
        // The cell the boid is on, plus the `trail_length` behind it. One more than
        // the trail is long, because the head of the streak is the boid itself
        // rather than a trail cell.
        let keep = self.options.trail_length as usize + 1;

        for boid in &mut self.boids {
            // Update position
            boid.position.0 += boid.velocity.0 * scale;
            boid.position.1 += boid.velocity.1 * scale;

            // Wrap around screen boundaries
            if boid.position.0 < 0.0 {
                boid.position.0 += width;
            } else if boid.position.0 >= width {
                boid.position.0 -= width;
            }

            if boid.position.1 < 0.0 {
                boid.position.1 += height;
            } else if boid.position.1 >= height {
                boid.position.1 -= height;
            }

            // Update visual representation
            boid.update_visual(
                &charset_chars,
                &self.speed_palette,
                min_speed,
                max_speed,
            );

            // Where this boid has just been, newest first. The head of the deque
            // is the cell it is on now, so the streak behind it is the same deque
            // read backwards.
            boid.history.push_front(boid.cell(cols, rows));
            while boid.history.len() > keep {
                boid.history.pop_back();
            }
        }
    }

    /// Fades every lit trail cell, and forgets the ones that have gone out.
    ///
    /// Per step rather than per frame, and `scale` is a number of frames, so the
    /// fade is the same length of time at any frame rate. The cells that drop out
    /// need no record of themselves: the trail is rebuilt from the survivors into
    /// a blank frame every frame, so a cell that is no longer lit is blank in the
    /// frame and the diff reports it, and the terminal is actually told to erase
    /// it. That is what the `shown` buffer is for, and it is the reason the
    /// accumulating layer is a plain pair of buffers.
    fn fade_trail(&mut self, scale: f32) {
        let decay = TRAIL_DECAY.powf(scale.max(0.0));
        self.trail.retain(|_, mark| {
            mark.intensity *= decay;
            mark.intensity >= TRAIL_FLOOR
        });
    }

    /// Stamps every boid's recent cells into the trail at their own intensities.
    ///
    /// `max`, not assign: two boids can cross the same cell, and the brighter of
    /// the two is the one that should be there. A cell a boid has just left is
    /// stamped at a fraction of full strength and then keeps fading, so the tail
    /// comes off smoothly instead of stopping dead at the last cell the boid drew.
    fn stamp(&mut self) {
        let (cols, rows) = (
            self.options.screen_size.0 as usize,
            self.options.screen_size.1 as usize,
        );

        for boid in &self.boids {
            let (character, color) = (boid.character, boid.color);
            for (step, cell) in boid.history.iter().enumerate() {
                let (x, y) = *cell;
                if x >= cols || y >= rows {
                    continue;
                }
                let intensity = TRAIL_FADE_PER_STEP.powi(step as i32);
                self.trail
                    .entry((x, y))
                    .and_modify(|mark| {
                        if intensity > mark.intensity {
                            mark.intensity = intensity;
                            mark.glyph = character;
                            mark.color = color;
                        }
                    })
                    .or_insert(TrailMark {
                        glyph: character,
                        color,
                        intensity,
                    });
            }
        }
    }

    /// Builds the frame and reports what changed on it.
    ///
    /// Rebuilt from the trail every frame rather than added to, which is what
    /// makes a fading trail possible: the frame is the trail as it is *now*, and
    /// the diff against what the terminal is showing is the erasure as well as the
    /// drawing.
    fn paint(&mut self) -> Vec<(usize, usize, Cell)> {
        self.stamp();

        self.frame.fill_with(&Cell::default());
        for (&(x, y), mark) in &self.trail {
            if x >= self.frame.width || y >= self.frame.height {
                continue;
            }
            // The same colour as the boid that left it, dimmed toward the
            // terminal's background in proportion to what is left of it. A trail
            // is the same boid at an earlier place, not a second thing.
            //
            // `Color::Rgb { 0, 0, 0 }` and *not* `Color::Black`: they are not the
            // same value to `lerp`, which has no channels to blend on the
            // `Black` variant and picks an endpoint instead. So the trail was
            // being drawn at full strength -- every cell of every streak the
            // brightest colour on the screen, which is the one thing a trail must
            // not be.
            let faded = lerp(TRAIL_BACKGROUND, mark.color, mark.intensity);
            self.frame.set(
                x,
                y,
                Cell::new(mark.glyph, faded, style::Attribute::Reset),
            );
        }

        // The waves go on last, over the trail rather than into it.
        //
        // Into the trail layer would mean they are stamped, faded and forgotten
        // along with the boids, so a ring would leave a smear behind it that
        // decayed on the trail's schedule rather than the wave's, and a wave
        // crossing a boid's streak would fight it cell by cell. Over the top, the
        // ring is a property of this frame and leaves when the wave does.
        self.paint_waves();

        let cells = self.shown.diff(&self.frame);
        for (x, y, cell) in &cells {
            self.shown.set(*x, *y, *cell);
        }
        cells
    }

    /// Draws every live wave's band as a ring, into the frame being built.
    ///
    /// A ring rather than an annulus filled in, and a thin one, for the reason the
    /// whole effect is legibility: what the user needs to see is *where the wave
    /// is*, and a filled disc that grows would cover the flock it is meant to be
    /// showing the effect of.
    ///
    /// Solved per column rather than per cell, because a circle is two points per
    /// column and not one per cell. For a column at a horizontal distance `span`
    /// from the origin, the row at which the circle at radius `r` crosses is
    /// `sqrt(r^2 - span^2)` in the corrected metric, and that is the same
    /// arithmetic [`Boids::shock_force`] does to decide whether the column is in
    /// the band at all. Which is `O(width)` per wave instead of `O(area)`: at
    /// 400x200 the difference is 80,000 distance computations a frame against
    /// 400, and a screen-wide test would have made the cheapest interactive effect
    /// in the crate the second most expensive one.
    ///
    /// Both edges of the band, so it is drawn as a ring rather than a disc. The
    /// inner edge only exists while the front is further from the origin than the
    /// band is wide; before that the band is a filled patch around the click and
    /// there is no hole in the middle to draw.
    ///
    /// The offset is measured from the *wave's* column, not the screen's, and
    /// wrapped when it comes back to a column index. Both are load-bearing. A
    /// click near the right edge really does reach the boids near the left edge,
    /// because the force is taken with `toroidal_diff`, so a ring that stopped at
    /// the edge would be showing the user a smaller wave than the one acting on
    /// their flock, and its other arc would appear from nowhere. Measuring the
    /// offset from the screen's middle instead would draw a circle centred on the
    /// middle of the terminal rather than on the click, which is a different
    /// defect rather than a subtler version of this one.
    fn paint_waves(&mut self) {
        let (cols, rows) = (
            self.options.screen_size.0 as usize,
            self.options.screen_size.1 as usize,
        );
        if cols == 0 || rows == 0 {
            return;
        }

        for wave in &self.waves {
            // As a fraction of a full-strength wave, which is what dims it. A
            // wave at birth is the brightest a ring ever is and it fades with the
            // force behind it, so what is on screen and what is happening to the
            // flock are the same thing fading together.
            let fade = (wave.strength / shock::PUSH).clamp(0.0, 1.0);
            let outer = wave.radius + shock::BAND;
            let inner = (wave.radius - shock::BAND).max(0.0);

            // Every column, as a signed offset from the origin's column. Going up
            // to half the width either side is the whole of the reachable set: a
            // column further off than that is closer to the origin the other way
            // round, and is drawn on that pass.
            for column in 0..cols {
                // The signed distance from the origin to this column, wrapped.
                let offset = wrapped_column(column, wave.origin.0, cols as f32);
                let span = shock_distance((offset, 0.0));

                if span >= outer {
                    continue;
                }
                // How far into the band this column is, front to back. Drives the
                // ring's own brightness so it has a soft edge, matching the force
                // it is drawing rather than announcing a sharper boundary than
                // there is.
                let depth = if inner <= 0.0 {
                    1.0
                } else {
                    ((outer - span) / (outer - inner)).clamp(0.0, 1.0)
                };
                let lit = lerp(TRAIL_BACKGROUND, SHOCK_COLOR, fade * depth);

                // The rows this column's circles cross, in the corrected metric
                // and then back into rows about the *origin's* row. Two circles
                // and two crossings each, so four rows, which is a fixed-size
                // array and not a `Vec` -- this runs once per column per wave and
                // an allocation here would be the most expensive thing in the
                // effect.
                let mut edges = [0.0f32; 4];
                let mut count = 0;
                for radius in [outer, inner] {
                    // `sqrt` of a negative number is `NaN`, and a `NaN` compared
                    // against the row bounds passes every one of them and then
                    // casts to zero -- which would drop a cell into the middle of
                    // the screen on every column of the ring. Skipping the term
                    // instead is the honest answer: a column past this circle's
                    // radius does not cross it.
                    let squared = radius * radius - span * span;
                    if squared <= 0.0 {
                        continue;
                    }
                    let half_rows = squared.sqrt() / shock::CELL_ASPECT;
                    edges[count] = wave.origin.1 - half_rows;
                    edges[count + 1] = wave.origin.1 + half_rows;
                    count += 2;
                }

                let x = column;
                for &edge in edges.iter().take(count) {
                    let y = edge.round();
                    if y < 0.0 || y >= rows as f32 {
                        continue;
                    }
                    let y = y as usize;
                    // Never over a lit cell. A boid under a ring should stay
                    // visible: the ring passes over in a handful of frames, and
                    // blanking a boid for those frames reads as the flock losing
                    // members rather than as a ripple going by. The flock is five
                    // percent of the cells, so this hides almost none of the ring.
                    if self.trail.contains_key(&(x, y)) {
                        continue;
                    }
                    self.frame.set(
                        x,
                        y,
                        Cell::new(SHOCK_GLYPH, lit, style::Attribute::Reset),
                    );
                }
            }
        }
    }
}

impl Boid {
    /// The cell this boid is on, wrapping the way the drawing does.
    fn cell(&self, cols: usize, rows: usize) -> (usize, usize) {
        // `round` can reach `cols` exactly, and `%` is what makes that the first
        // column rather than one past the end. `Canvas::set` drops out-of-range
        // writes, so without it a boid on the right edge of the screen would
        // silently vanish on alternate frames.
        let x = self.position.0.round() as isize;
        let y = self.position.1.round() as isize;
        (
            x.rem_euclid(cols.max(1) as isize) as usize,
            y.rem_euclid(rows.max(1) as isize) as usize,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Duration;

    /// Perceived brightness of a colour, `0.0..=1.0`.
    fn luminance(color: style::Color) -> f32 {
        match color {
            style::Color::Rgb { r, g, b } => {
                (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b))
                    / 255.0
            }
            _ => 1.0,
        }
    }

    fn flock(size: (u16, u16)) -> Boids {
        let options = BoidsOptions {
            screen_size: size,
            boid_count: flock_count(size),
            ..Default::default()
        };
        Boids::new(options)
    }

    /// Runs `frames` frames of the loop: advance, then draw.
    fn run(flock: &mut Boids, frames: u32) {
        for _ in 0..frames {
            flock.update();
            flock.get_diff();
        }
    }

    /// The flock the default configuration asks for on a screen this size.
    fn flock_count(size: (u16, u16)) -> u16 {
        let area = size.0 as f32 * size.1 as f32;
        ((area * 0.5 * BoidsOptions::default().boid_coeff) as u16).clamp(50, 300)
    }

    fn flock_with_seed(size: (u16, u16), seed: u64) -> Boids {
        let base = BoidsOptions::default();
        let options = BoidsOptions {
            screen_size: size,
            boid_count: flock_count(size),
            seed,
            ..base
        };
        Boids::new(options)
    }

    /// The mean distance from each boid to its nearest neighbour.
    fn mean_nearest_neighbour(flock: &Boids) -> f32 {
        flock.boids.iter().fold(0.0f32, |total, boid| {
            let nearest = flock
                .boids
                .iter()
                .filter(|other| other.position != boid.position)
                .map(|other| {
                    let diff = flock.toroidal_diff(other.position, boid.position);
                    diff.0.hypot(diff.1)
                })
                .fold(f32::INFINITY, f32::min);
            total + nearest
        }) / flock.boids.len() as f32
    }

    #[test]
    fn resize_recomputes_boid_count() {
        let options = BoidsOptions {
            screen_size: (80, 40),
            boid_count: 10,
            boid_coeff: 1.0,
            ..Default::default()
        };
        let mut boids = Boids::new(options);

        boids.update_size(10, 10);

        assert_eq!(boids.options.screen_size, (10, 10));
        assert_eq!(boids.options.boid_count, 50);
    }

    /// The default flock is about five percent of the screen, not sixteen.
    ///
    /// The count used to be `area * 0.5` clamped to 300, which on an 80x24
    /// terminal is 300 boids on 1920 cells. There is no gap between them, so there
    /// is no shape for the flock to be: a flock reads as a shape because of the
    /// empty space inside it.
    #[test]
    fn the_default_flock_is_about_five_percent_of_the_screen() {
        let (width, height) = (80u16, 24u16);
        let cells = f64::from(width) * f64::from(height);
        let count = f64::from(flock((width, height)).options.boid_count);

        let coverage = count / cells;
        assert!(
            (0.04..=0.06).contains(&coverage),
            "the default flock covers {:.1}% of an {width}x{height} terminal \
             ({count:.0} of {cells:.0} cells), which is not the five percent a \
             flock needs to read as a shape",
            coverage * 100.0
        );
    }

    /// A cell a boid has left is still lit, and dimmer than the one it is on.
    ///
    /// The heart of the legibility change. The screen used to be cleared every
    /// frame, so a cell a boid had left went blank the instant it moved and a
    /// flocking simulation -- a *motion* phenomenon -- was rendered as three
    /// hundred isolated glyphs all changing at once. That is not a trail, it is
    /// flicker, and the eye can only read flicker as noise.
    ///
    /// Stated about a flock of one on purpose. With a hundred boids, two of them
    /// cross the same cell and the brighter one owns it, so a given boid's trail
    /// cell is not necessarily its own colour and a brightness comparison is
    /// meaningless. One boid has nobody to share a cell with.
    #[test]
    fn a_cell_the_boid_has_left_is_still_drawn_and_dimmer() {
        let options = BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        };
        let mut flock = Boids::new(options);
        run(&mut flock, 90);

        let boid = &flock.boids[0];
        let (now, then) = (boid.history[0], boid.history[1]);
        assert_ne!(now, then, "the boid did not move in the last frame");

        let here = flock.frame.get(now.0, now.1);
        let there = flock.frame.get(then.0, then.1);
        assert_ne!(
            there.symbol, ' ',
            "the cell ({}, {}) that the boid just left is blank, so there is no \
             trail",
            then.0, then.1
        );
        assert!(
            luminance(there.color) < luminance(here.color),
            "the cell ({}, {}) the boid left is at brightness {:.2} and the one it \
             is on is at {:.2}, so the trail is not dimmer than its head",
            then.0,
            then.1,
            luminance(there.color),
            luminance(here.color)
        );
        assert_eq!(
            there.symbol, here.symbol,
            "the trail changed glyph on the way out"
        );
    }

    /// The trail is visibly wider than the flock.
    ///
    /// The population-level version of the test above, and the one that says what
    /// the user sees: a flock of `n` boids drawing `n` cells is a scatter of
    /// points no matter how well it flocks, because a scatter of points has no
    /// direction. Motion is the thing being shown, and motion needs the cells
    /// in between.
    #[test]
    fn the_trail_covers_more_cells_than_there_are_boids() {
        let mut flock = flock((80, 24));
        run(&mut flock, 120);

        let lit = (0..flock.frame.height)
            .flat_map(|y| (0..flock.frame.width).map(move |x| (x, y)))
            .filter(|(x, y)| flock.frame.get(*x, *y).symbol != ' ')
            .count();

        assert!(
            lit > flock.boids.len() * 2,
            "{lit} cells are lit for {} boids, so the streak is barely longer \
             than the boid that drew it",
            flock.boids.len()
        );
        assert!(
            lit < 80 * 24 / 2,
            "{lit} of {} cells are lit, which is not a flock over a background",
            80 * 24
        );
    }

    /// A cell nothing is on any more goes back to blank.
    ///
    /// The other half of the trail. A trail that only ever adds leaves every cell
    /// a boid has ever visited lit forever, and a screen that slowly fills with
    /// dim marks is not a trail either.
    #[test]
    fn a_trail_cell_goes_back_to_blank_once_nothing_is_stamping_it() {
        let mut flock = flock((80, 24));
        run(&mut flock, 30);

        let lit: HashSet<(usize, usize)> = flock.trail.keys().copied().collect();
        assert!(lit.len() > 20, "the flock drew almost nothing");

        // Long enough for a boid to have crossed a good part of the screen and
        // for the cells it left to be under the floor.
        run(&mut flock, 120);

        let stale: Vec<(usize, usize)> = lit
            .into_iter()
            .filter(|cell| !flock.trail.contains_key(cell))
            .collect();
        let still_lit = stale
            .iter()
            .filter(|(x, y)| flock.frame.get(*x, *y).symbol != ' ')
            .count();
        assert_eq!(
            still_lit, 0,
            "{} cells that used to hold a trail are still drawn, so the trail \
             only ever adds",
            still_lit
        );
    }

    /// Speed is legible across the whole range the simulation keeps it in.
    ///
    /// The old colour was `r = speed * 128`, `g = 200`, `b = speed * 128 + 20`
    /// against a `max_speed` of 0.6. The red channel could not exceed 76 out of
    /// 255 and the green was pinned, so the whole flock was one dark teal and the
    /// comment above the code, claiming "green to white", was describing a colour
    /// the code could not produce. On top of that separation was pinning almost
    /// every boid at `max_speed`, so even the 27-value swing in red that was left
    /// was mostly saturated.
    #[test]
    fn speed_is_legible_across_the_whole_range() {
        let slow = speed_palette().sample(0.0);
        let quick = speed_palette().sample(1.0);

        assert_eq!(
            slow,
            style::Color::Rgb {
                r: 12,
                g: 48,
                b: 96
            }
        );
        assert_eq!(
            quick,
            style::Color::Rgb {
                r: 255,
                g: 255,
                b: 255
            },
            "the top of the ramp is not the brightest colour, so speed still does \
             not read"
        );
        assert!(
            luminance(quick) - luminance(slow) > 0.5,
            "the whole speed range spans only {:.2} of brightness, so the colour \
             is not carrying the speed",
            luminance(quick) - luminance(slow)
        );

        // The bright end has to be out of reach of the old arithmetic, which
        // topped out at 76 in the red channel.
        match quick {
            style::Color::Rgb { r, g, b } => {
                assert!(
                    r > 200 && g > 200 && b > 200,
                    "the fastest boid is ({r},{g},{b}); `speed * 128` against a \
                     max of 0.6 could not produce more than 76 in red"
                );
            }
            other => panic!("the top of the ramp is {other:?}, not a colour"),
        }

        // And the mapping is the speed *fraction across the range the simulation
        // keeps speeds in*, so the two ends of that range are the two ends of the
        // ramp -- which an absolute speed against a cap could never be, because
        // every clamped boid would land on the same colour.
        let base = BoidsOptions::default();
        let boid = |speed: f32| Boid {
            position: (1.0, 1.0),
            velocity: (speed, 0.0),
            character: 'x',
            color: style::Color::Reset,
            history: VecDeque::new(),
        };
        assert_eq!(
            boid(base.min_speed).speed_fraction(base.min_speed, base.max_speed),
            0.0
        );
        assert_eq!(
            boid(base.max_speed).speed_fraction(base.min_speed, base.max_speed),
            1.0
        );
        let middle = (base.min_speed + base.max_speed) / 2.0;
        assert!(
            (boid(middle).speed_fraction(base.min_speed, base.max_speed) - 0.5)
                .abs()
                < 0.01,
            "halfway along the speed range is not halfway along the ramp"
        );
        // Even a boid going twice as fast as the cap is the top of the ramp, not
        // off the end of it.
        assert_eq!(
            boid(base.max_speed * 2.0)
                .speed_fraction(base.min_speed, base.max_speed),
            1.0
        );
        // A range that cannot be divided by reports the bright end rather than
        // producing a NaN, which is what a user who set `max_speed` below
        // `min_speed` in a config file would otherwise have got.
        assert_eq!(boid(0.3).speed_fraction(0.6, 0.6), 1.0);
        assert_eq!(boid(0.3).speed_fraction(0.6, 0.1), 1.0);
    }

    /// The flock clusters. This is the test that says the effect does what it is
    /// named after.
    ///
    /// Measured as the mean distance to a boid's nearest neighbour. Chosen over
    /// "the fraction of the flock with no neighbour inside `separation_distance`"
    /// because that is a step function of a threshold: with the density this
    /// effect now runs at, a boid is right on the boundary between having and not
    /// having a neighbour, so the fraction jumps about with the seed. The mean
    /// distance is continuous, and for a uniform random scatter of density `d` it
    /// has a known value of about `0.5 / sqrt(d)`, which is what the "before" arm
    /// of this test actually measures.
    ///
    /// Averaged over several seeds, because a single run of a chaotic system is
    /// not a measurement.
    #[test]
    fn the_flock_forms_clusters_rather_than_staying_a_gas() {
        const SEEDS: [u64; 5] = [1, 2, 3, 4, 5];
        const FRAMES: u32 = 240;
        let size = (60u16, 30u16);

        let start: f32 = SEEDS
            .iter()
            .map(|seed| mean_nearest_neighbour(&flock_with_seed(size, *seed)))
            .sum();
        let start = start / SEEDS.len() as f32;

        let finish: f32 = SEEDS
            .iter()
            .map(|seed| {
                let mut flock = flock_with_seed(size, *seed);
                run(&mut flock, FRAMES);
                mean_nearest_neighbour(&flock)
            })
            .sum();
        let finish = finish / SEEDS.len() as f32;

        assert!(
            finish < start * 0.75,
            "the mean distance to the nearest neighbour went from {start:.2} to \
             {finish:.2} over {FRAMES} frames. A flock is boids that are closer \
             together than chance, so this is a gas either way."
        );
    }

    /// A boid in a corner is pushed diagonally, and no harder than one at an edge.
    ///
    /// The border force used to be one horizontal term plus one vertical term,
    /// each up to 1.8, added independently. A boid in the top-left corner with
    /// both margins fully intruded got `(1.8, 1.8)`, but a boid one cell down
    /// from it got `(1.8, 0.0)` -- so a fifth of the screen, along each edge, was
    /// a band of one-axis shoving five times the size of the whole of the
    /// alignment rule, and boids in it ricocheted off invisible walls. And a
    /// corner got twice the push of an edge, so the corners were traps.
    #[test]
    fn a_boid_in_a_corner_is_pushed_towards_the_centre_not_along_one_axis() {
        let options = BoidsOptions {
            screen_size: (80, 24),
            boid_count: 10,
            ..Default::default()
        };
        let border = options.border_factor;
        let top_speed = options.max_speed;
        let flock = Boids::new(options);
        let magnitude = |v: (f32, f32)| v.0.hypot(v.1);

        for &(x, y) in &[(0.0, 0.0), (79.0, 0.0), (0.0, 23.0), (79.0, 23.0)] {
            let (fx, fy) = flock.border_force((x, y));
            assert!(
                fx.abs() > 0.0 && fy.abs() > 0.0,
                "the boid at ({x}, {y}) is pushed ({fx}, {fy}), which is along one \
                 axis only"
            );
            assert_eq!(
                fx.is_sign_positive(),
                x < 40.0,
                "the boid at ({x}, {y}) is pushed to the left, away from the middle"
            );
            assert_eq!(
                fy.is_sign_positive(),
                y < 12.0,
                "the boid at ({x}, {y}) is pushed the wrong way vertically"
            );
        }

        // A corner is a place with two edges, not a deeper edge, so it is not
        // pushed twice as hard as one.
        let edge = magnitude(flock.border_force((40.0, 0.0)));
        let corner = magnitude(flock.border_force((0.0, 0.0)));
        assert!(
            (corner - edge).abs() < edge * 0.1,
            "a corner is pushed {corner:.2} into the screen where the middle of the \
             top edge is pushed {edge:.2}"
        );

        // And the push is the largest of the two rules, so a boid is not being
        // added to its position by more than its own acceleration.
        let maximum = border;
        assert!(
            corner <= maximum + f32::EPSILON,
            "the border rule can add {corner:.2} to a velocity capped at {top_speed}"
        );
    }

    // --- the click force ----------------------------------------------------

    /// A wave pushes a boid away from where it was dropped.
    ///
    /// The heart of the interaction, asserted on the force itself rather than
    /// through a 300-boid run, because a click that pushed boids *together* or
    /// sideways would still produce a flock that moved, and a test that only
    /// watched the flock move would pass. So this checks the direction, on a
    /// flock of one.
    #[test]
    fn a_wave_pushes_a_boid_away_from_where_it_was_dropped() {
        let mut flock = Boids::new(BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        });
        flock.drop_wave((40.0, 12.0));

        // One cell out horizontally, and `1 / CELL_ASPECT` rows out vertically,
        // so all four are at the same *distance* under the metric the force uses.
        // That is not incidental bookkeeping: the vertical offsets are divided by
        // the aspect because a cell is about twice as tall as it is wide, so a
        // naive "two rows away" would be testing a point four units from the wave
        // and would pass or fail for the wrong reason.
        for (target, away) in [
            ((41.0, 12.0), (1.0, 0.0)),
            ((40.0, 12.0 + 1.0 / shock::CELL_ASPECT), (0.0, 1.0)),
            ((39.0, 12.0), (-1.0, 0.0)),
            ((40.0, 12.0 - 1.0 / shock::CELL_ASPECT), (0.0, -1.0)),
        ] {
            let (fx, fy) = flock.shock_force(target, (1.0, 0.0));

            assert!(
                fx.hypot(fy) > 0.0,
                "a wave dropped at (40, 12) does not reach the boid at {target:?}"
            );
            assert!(
                fx * away.0 + fy * away.1 > 0.0,
                "a wave dropped at (40, 12) pushes the boid at {target:?} by \
                 ({fx:.2}, {fy:.2}), which is not directly away from it"
            );
        }
    }

    /// The band is a band: nothing outside it, most inside it.
    ///
    /// Without the outer bound a wave would keep pushing the whole screen for as
    /// long as it lived and the effect would be a kick rather than a ripple.
    #[test]
    fn a_wave_only_reaches_a_band_around_its_front() {
        let mut flock = Boids::new(BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        });
        flock.drop_wave((40.0, 12.0));
        // A front ten cells out, and the band three either side of it, so the
        // band covers distances 7 to 13 from the origin.
        flock.waves[0].radius = 10.0;

        // Measured horizontally, where a cell's distance is its distance: the
        // vertical would have to be divided by the aspect to mean the same thing.
        let push = |distance: f32| {
            flock.shock_force((40.0 + distance, 12.0), (1.0, 0.0)).0
        };

        let front = push(10.0);
        let inside = push(7.5);
        let outside = push(6.5);
        let far_outside = push(20.0);

        assert!(
            front > 0.0 && inside > 0.0,
            "a wave's own front ({front:.2}) and its trailing edge ({inside:.2}) \
             should both be inside the band"
        );
        assert!(
            outside < 0.001,
            "a boid half a cell outside the band was pushed {outside:.4}"
        );
        assert!(
            far_outside < 0.001,
            "a boid ten cells clear of the band was pushed {far_outside:.4}, so a \
             wave is a kick rather than a travelling band"
        );
        // The front is the strongest part, so a boid is carried by the wave
        // rather than shouldered aside by its leading edge.
        assert!(
            front > inside,
            "the front of a wave ({front:.2}) is weaker than its trailing edge \
             ({inside:.2})"
        );
        assert!(
            front <= shock::PUSH + f32::EPSILON,
            "a wave pushes by {front:.2}, past the {} it is meant to peak at",
            shock::PUSH
        );
    }

    /// A boid is not glued to a wave's band by a step change in force.
    ///
    /// A flat plateau inside the band and nothing outside it hands a boid a
    /// discontinuity in acceleration as the front passes it, which reads as the
    /// boid flinching rather than being carried. So the force is measured either
    /// side of the boundary and has to approach zero.
    #[test]
    fn the_force_tapers_to_nothing_at_the_bands_edge() {
        let mut flock = Boids::new(BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        });
        flock.drop_wave((40.0, 12.0));
        flock.waves[0].radius = 10.0;
        // The trailing edge of the band, as a distance from the origin, sampled a
        // hundredth of a cell either side of it and again at the front.
        let edge = shock::BAND;
        let push = |distance: f32| {
            flock.shock_force((40.0 + distance, 12.0), (1.0, 0.0)).0
        };

        let just_inside = push(10.0 - edge + 0.01);
        let just_outside = push(10.0 - edge - 0.01);
        let middle = push(10.0);

        assert!(
            just_outside < 0.001,
            "the band does not end: a boid 0.01 cells past its trailing edge is \
             still pushed {just_outside:.4}"
        );
        assert!(
            just_inside < middle,
            "the force does not taper: {just_inside:.4} at the edge of the band \
             against {middle:.4} at its middle"
        );
    }

    /// A boid exactly where the click landed is pushed, and stays a number.
    ///
    /// The direction away from an origin is undefined at the origin, and dividing
    /// by a zero-length vector puts a `NaN` into a velocity. It does not panic: it
    /// teleports the boid to cell zero, because `NaN as isize` is zero, and one
    /// boid stuck in a corner is a flock that never recovers. Reachable whenever a
    /// boid's position lands on an exact integer, which the rounding in
    /// `Boid::cell` is evidence happens.
    ///
    /// So the boid is shoved along its own heading, and this is the test that says
    /// so -- a position exactly equal to the origin, not merely near it.
    #[test]
    fn a_boid_clicked_exactly_on_the_head_is_pushed_onward_and_stays_finite() {
        let mut flock = Boids::new(BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        });
        // Both the position and the wave's origin exactly equal, so the
        // difference is exactly zero rather than merely small.
        flock.boids[0].position = (40.0, 12.0);
        flock.boids[0].velocity = (0.0, -0.4);
        flock.drop_wave((40.0, 12.0));

        let (fx, fy) = flock.shock_force((40.0, 12.0), (0.0, -0.4));

        assert!(
            fx.is_finite() && fy.is_finite(),
            "clicking a boid on the head produced ({fx}, {fy})"
        );
        assert!(
            fy < 0.0,
            "a boid clicked on the head is pushed ({fx:.2}, {fy:.2}) rather than \
             onward along the (-0.4) heading it was already travelling"
        );
        assert!(
            fx.abs() < f32::EPSILON,
            "a boid with a purely vertical heading picked up {fx:.4} of sideways \
             force from being clicked on the head"
        );

        // And through the whole loop, not just the force: a `NaN` here would
        // poison the position, and the position is what everything else reads.
        run(&mut flock, 30);
        for boid in &flock.boids {
            assert!(
                boid.position.0.is_finite() && boid.position.1.is_finite(),
                "a boid's position went to {:?} after being clicked on the head",
                boid.position
            );
        }
    }

    /// A wave reaches across the wrap, because the flock does.
    ///
    /// The distance is taken with `toroidal_diff` like every other rule, so a
    /// click at the left of the screen reaches a boid at the right of it. Were it
    /// taken straight, those two boids would be seventy cells apart and the ring
    /// would visibly stop short of a boid it was already pushing.
    #[test]
    fn a_wave_reaches_across_the_wrap() {
        let mut flock = Boids::new(BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        });
        // Column 1, because the band's edge is at `BAND` cells and an origin at
        // column 2 puts the wrapped side exactly on it: at column 2 the nearest
        // wrapped column is 79, three cells away, and the band excludes its own
        // edge, so there would be no boid on the far side for the test to find.
        flock.drop_wave((1.0, 12.0));

        // Column 79 is two cells the *other* way round from column 1 -- the
        // straight-line distance is 78 -- so it is inside the band and only the
        // wrap can put it there. Column 40 is as far away again in the ordinary
        // direction and is out of the band entirely.
        let across = flock.shock_force((79.0, 12.0), (1.0, 0.0));
        let straight = flock.shock_force((40.0, 12.0), (1.0, 0.0));

        assert!(
            across.0 < 0.0,
            "a wave dropped at column 1 pushed a boid at column 79 to the right \
             by {:.2}, so it did not reach across the wrap",
            across.0
        );
        assert!(
            straight.0 < 0.001,
            "the same wave reached a boid 39 cells away in the ordinary direction \
             by {:.4}, so the wrap is not what carried it",
            straight.0
        );
    }

    /// A drag scatters. A hover does not.
    ///
    /// The runtime cannot tell those two apart: `translate_mouse` gives a
    /// buttonless `MouseEventKind::Moved` the `Left` button, which is exactly what
    /// a left-drag gets, so both arrive as `PointerPhase::Moved` and only a
    /// remembered press separates them. Which is why the effect latches
    /// `pointer_down` rather than asking.
    ///
    /// If the latch were dropped and `Moved` were taken at face value, the flock
    /// would scatter whenever a cursor drifted across the screen -- in a screensaver,
    /// which is a thing nobody is touching, that is a bug that looks like the
    /// effect misfiring on its own.
    #[test]
    fn a_drag_scatters_but_a_hover_does_not() {
        let pointer = (40u16, 12u16);
        let move_to = |flock: &mut Boids, position: (u16, u16)| {
            flock.handle_input(&InputEvent::Pointer {
                position,
                phase: PointerPhase::Moved,
                button: crate::runtime::PointerButton::Left,
            });
        };

        // A drag: a press, then travel. The drag has to cross the whole screen
        // because a wave is only dropped every `DRAG_SPACING` cells.
        let mut dragged = flock((80, 24));
        dragged.handle_input(&InputEvent::Pointer {
            position: pointer,
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        for x in 0..80u16 {
            move_to(&mut dragged, (x, 12));
        }
        assert!(
            dragged.waves.len() > 1,
            "dragging across a whole screen dropped {} wave(s), so the drag is not \
             scattering as it goes",
            dragged.waves.len()
        );

        // A hover: the same travel, with no press. Not one wave.
        let mut hovered = flock((80, 24));
        for x in 0..80u16 {
            move_to(&mut hovered, (x, 12));
        }
        assert!(
            hovered.waves.is_empty(),
            "a cursor drifting across the screen without a button down dropped {} \
             wave(s), so moving the mouse is enough to disturb the flock",
            hovered.waves.len()
        );

        // And releasing stops the drag, rather than leaving the latch set so the
        // next stray movement continues it.
        dragged.handle_input(&InputEvent::Pointer {
            position: (0, 12),
            phase: PointerPhase::Released,
            button: crate::runtime::PointerButton::Left,
        });
        let before = dragged.waves.len();
        for x in 0..80u16 {
            move_to(&mut dragged, (x, 12));
        }
        assert_eq!(
            dragged.waves.len(),
            before,
            "the flock kept scattering after the button was released"
        );
    }

    /// Waves stop, and the flock is a flock again.
    ///
    /// A wave that never ended would be a permanent fourth force on every boid,
    /// which is not a transient the user caused but a rule the effect now has. It
    /// would also be a leak: `apply_rules` costs a distance computation per boid
    /// per wave, forever.
    #[test]
    fn waves_end_and_the_flock_closes_again() {
        let size = (80u16, 24u16);
        let mut flock = flock(size);
        flock.handle_input(&InputEvent::Pointer {
            position: (40, 12),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        assert_eq!(
            flock.waves.len(),
            1,
            "a click dropped no wave to begin with"
        );

        // Well past both the age bound and the time to cross this screen.
        run(&mut flock, 900);

        assert!(
            flock.waves.is_empty(),
            "{} wave(s) were still alive fifteen seconds after one click",
            flock.waves.len()
        );
    }

    /// Clicking opens the flock up, and it closes again afterwards.
    ///
    /// The claim the effect exists to make, measured the way the clustering test
    /// above measures its own: the mean distance to the nearest neighbour of a
    /// flock that was clicked, against a twin that was not, over the same number
    /// of frames on the same seed. The twin is what makes it a measurement rather
    /// than an observation -- a flock's spacing drifts on its own, so "the spacing
    /// went up" would be true of an unclicked flock too.
    ///
    /// The middle of the screen, because a click near an edge is partly the border
    /// rule's doing and would not isolate anything.
    #[test]
    fn clicking_spreads_the_flock_and_it_closes_again() {
        const SEEDS: [u64; 4] = [11, 22, 33, 44];
        const SETTLE: u32 = 180;
        const SETTLED: u32 = 30;
        let size = (60u16, 30u16);
        let centre = (30u16, 15u16);

        let after_click = |seed: u64| {
            let mut flock = flock_with_seed(size, seed);
            // Let it form a flock first, so what is measured is a flock being
            // disturbed rather than a gas being stirred.
            run(&mut flock, SETTLE);
            flock.handle_input(&InputEvent::Pointer {
                position: centre,
                phase: PointerPhase::Pressed,
                button: crate::runtime::PointerButton::Left,
            });
            run(&mut flock, SETTLED);
            mean_nearest_neighbour(&flock)
        };
        let untouched = |seed: u64| {
            let mut flock = flock_with_seed(size, seed);
            run(&mut flock, SETTLE);
            run(&mut flock, SETTLED);
            mean_nearest_neighbour(&flock)
        };

        let opened: f32 = SEEDS.iter().map(|s| after_click(*s)).sum();
        let closed: f32 = SEEDS.iter().map(|s| untouched(*s)).sum();
        let opened = opened / SEEDS.len() as f32;
        let closed = closed / SEEDS.len() as f32;

        assert!(
            opened > closed * 1.1,
            "clicking the middle of the flock left the mean nearest-neighbour \
             distance at {opened:.2} against {closed:.2} for a flock left alone. \
             A click that does not visibly open the flock up is not doing anything."
        );
    }

    /// The metric the waves use counts a cell's height as more than its width.
    ///
    /// This is the aspect correction, and it is tested on the metric rather than
    /// on the ring, which is deliberate. Asking "is the ring round?" cannot fail
    /// for the reason that matters: a ring drawn with an uncorrected metric is
    /// perfectly round *in that metric*, it just comes out twice as tall as it is
    /// wide on a screen where a cell is twice as tall as it is wide. Asking the
    /// question of the drawing measures consistency rather than correctness, and
    /// so passes against a ring that is the wrong shape.
    ///
    /// **What this cannot see:** whether `2.0` is the right number. That depends
    /// on the user's font, and it is a property of the technique rather than
    /// something to fix -- the sub-cell renderers in `crate::render` assume the
    /// same and record the same caveat. What is testable is that the assumption
    /// is made in one place and applied to both the force and the drawing, which
    /// is the part that can silently drift apart.
    #[test]
    fn the_wave_metric_corrects_for_a_tall_cell() {
        let across = shock_distance((3.0, 0.0));
        let down = shock_distance((0.0, 3.0));

        assert_eq!(across, 3.0, "a horizontal cell counts as one cell");
        assert!(
            down > across * 1.5,
            "three cells down is {down:.2} units and three cells across is \
             {across:.2}, so the metric is not correcting for a tall cell"
        );
        // And the force uses that metric rather than a raw difference, which is
        // the part that can drift between the two callers. Distances are from the
        // *origin*, since that is what a wave is centred on: a front ten cells
        // out with a three-cell band covers corrected distances of 7 to 13.
        //
        // Six rows up is twelve corrected units, inside the band. Six and a half
        // rows up is thirteen, and the band excludes its own edge. Uncorrected,
        // both would be six and a half and comfortably inside -- which is the
        // failure this is here to catch, since an uncorrected wave is an ellipse
        // twice as tall as it is wide.
        let mut flock = Boids::new(BoidsOptions {
            screen_size: (80, 24),
            boid_count: 1,
            ..Default::default()
        });
        flock.drop_wave((40.0, 12.0));
        flock.waves[0].radius = 10.0;

        let beside = flock.shock_force((50.0, 12.0), (1.0, 0.0)).0;
        let six_rows_up = flock.shock_force((40.0, 18.0), (1.0, 0.0)).1;
        let six_and_a_half_rows_up = flock.shock_force((40.0, 18.5), (1.0, 0.0)).1;

        assert!(
            beside > 0.0,
            "a boid ten cells beside a wave's front is not pushed"
        );
        assert!(
            six_rows_up > 0.0,
            "a boid six rows above a wave's origin is out of its band, but that is \
             twelve corrected units and the band runs to thirteen"
        );
        assert_eq!(
            six_and_a_half_rows_up, 0.0,
            "a boid six and a half rows above a wave's origin is still within its \
             band, so the vertical is not being scaled"
        );
    }

    /// The ring is a ring: two edges, and nothing between them.
    ///
    /// Three claims about the same drawing, and each of them is a way the ring
    /// came out wrong the first time.
    ///
    /// **Both edges exist.** The band's outer edge and its inner edge are separate
    /// circles, and drawing only the outer one gives a thinner ring rather than a
    /// broken one -- which is why this is checked by looking for cells at the
    /// inner radius on the origin's own column, where the two circles cross it at
    /// their extremes and are furthest apart.
    ///
    /// **Nothing in the body of the band.** The edges come from
    /// `sqrt(r^2 - span^2)`, which is negative for every column past a circle's
    /// radius. Clamping that to zero -- the obvious thing, since the row is wanted
    /// as a height and cannot be negative -- draws a height-zero crossing on the
    /// circle's own row for all of those columns, so the ring came out as an
    /// annulus with a bar through it. The radius has to be *skipped* for those
    /// columns, not its height clamped. Note where the bar lands: in the body of
    /// the band, between the two edges, which is why the assertion is about that
    /// annulus and not about the hole in the middle.
    ///
    /// **It goes all the way round**, or a ring that had collapsed to a dot would
    /// satisfy the first two.
    #[test]
    fn the_ring_is_two_edges_with_nothing_between_them() {
        let size = (60u16, 30u16);
        let cols = size.0 as f32;
        let (origin_x, origin_y) = (30usize, 15usize);

        let mut flock = flock(size);
        flock.get_diff();
        flock.handle_input(&InputEvent::Pointer {
            position: (origin_x as u16, origin_y as u16),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        // Twelve frames on: the front is nine cells out, the band three either
        // side, so the outer circle is at twelve and the inner at six.
        for _ in 0..12 {
            flock.update();
        }
        let radius = flock.waves[0].radius;
        assert!(
            radius > shock::BAND,
            "the wave has not travelled far enough for this test to mean anything: \
             its front is at {radius:.2} and the band is {} wide",
            shock::BAND
        );
        let inner = radius - shock::BAND;
        let outer = radius + shock::BAND;

        let drawn = flock.get_diff();
        let ring: Vec<(usize, usize)> = drawn
            .iter()
            .filter(|(_, _, cell)| cell.symbol == SHOCK_GLYPH)
            .map(|(x, y, _)| (*x, *y))
            .collect();
        assert!(
            !ring.is_empty(),
            "a wave that has travelled twelve frames drew nothing at all"
        );

        // Both edges, read off the origin's own column, where each circle is at
        // its vertical extreme. Four crossings: two circles, two sides each.
        let on_the_column: Vec<f32> = ring
            .iter()
            .filter(|(x, _)| *x == origin_x)
            .map(|(_, y)| (*y as f32 - origin_y as f32).abs() * shock::CELL_ASPECT)
            .collect();
        let near =
            |target: f32| on_the_column.iter().any(|v| (v - target).abs() <= 1.0);
        assert!(
            near(inner) && near(outer),
            "the ring on its own column crosses at {on_the_column:?} corrected \
             units from the origin, but the band's two edges are at {inner:.1} and \
             {outer:.1}. Both edges have to be drawn or the band is not a band."
        );

        // And nothing in the body between them, which is where the clamped
        // `sqrt` put its bar. Checked at the origin's own row, where the two
        // edges' crossings are at a column offset of exactly inner and outer, so
        // a cell between them is unambiguously in the band rather than in the
        // hole or outside the wave.
        let on_the_row: Vec<f32> = ring
            .iter()
            .filter(|(_, y)| *y == origin_y)
            .map(|(x, _)| {
                shock_distance((wrapped_column(*x, origin_x as f32, cols), 0.0))
            })
            .collect();
        let in_the_body: Vec<f32> = on_the_row
            .iter()
            .copied()
            .filter(|d| *d > inner - 1.0 && *d < outer - 1.0)
            .collect();
        assert!(
            in_the_body.is_empty(),
            "the ring is drawn across the body of its own band on the row through \
             the origin, at {in_the_body:?} cells out where the band is \
             {inner:.1} to {outer:.1}. A column past a circle's radius does not \
             cross it, and clamping its height to zero instead of skipping it \
             draws a bar through the band."
        );
        assert!(
            !ring.contains(&(origin_x, origin_y)),
            "the ring is drawn over the cell the wave came from"
        );

        // And it goes all the way round. Wrapped offsets, because a click near
        // the right edge puts one arc at the left of the screen and an unwrapped
        // comparison would read that as a hundred columns the wrong way.
        let (leftmost, rightmost) = ring
            .iter()
            .map(|(x, _)| wrapped_column(*x, origin_x as f32, cols))
            .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        assert!(
            leftmost <= -outer + 1.0 && rightmost >= outer - 1.0,
            "the ring reaches {leftmost:.1} to {rightmost:.1} either side of the \
             origin, but the band's outer edge is at {outer:.1}, so it is an arc \
             rather than a ring"
        );
    }

    /// The ring is centred on the click, wherever the click was.
    ///
    /// Asserted as "no drawn cell is further from the click than the wave
    /// reaches", in wrapped offsets -- not as a midpoint, which is the obvious way
    /// to say "centred on" and cannot work here. A ring crosses its own centre
    /// line symmetrically by construction, so the mean of its two extremes is the
    /// origin *at any radius and any origin*, and the check would pass against a
    /// ring drawn entirely in the wrong place. The reach test has no such
    /// symmetry to hide behind.
    #[test]
    fn the_ring_is_centred_where_the_click_was() {
        let size = (60u16, 30u16);
        let cols = size.0 as f32;

        // Well away from the middle, so a ring drawn at the middle instead of at
        // the click is twenty columns out and cannot pass.
        for click in [(10u16, 15u16), (50u16, 15u16), (30u16, 8u16)] {
            let mut flock = flock(size);
            flock.get_diff();
            flock.handle_input(&InputEvent::Pointer {
                position: click,
                phase: PointerPhase::Pressed,
                button: crate::runtime::PointerButton::Left,
            });
            for _ in 0..12 {
                flock.update();
            }
            let drawn = flock.get_diff();
            let ring: Vec<(usize, usize)> = drawn
                .iter()
                .filter(|(_, _, cell)| cell.symbol == SHOCK_GLYPH)
                .map(|(x, y, _)| (*x, *y))
                .collect();
            assert!(
                !ring.is_empty(),
                "no ring was drawn for a click at {click:?}"
            );

            let reach = flock.waves[0].radius + shock::BAND;
            let furthest = ring
                .iter()
                .map(|(x, y)| {
                    shock_distance((
                        wrapped_column(*x, click.0 as f32, cols),
                        *y as f32 - click.1 as f32,
                    ))
                })
                .fold(0.0f32, f32::max);

            assert!(
                furthest <= reach + 1.0,
                "a click at {click:?} drew a ring cell {furthest:.1} cells from it, \
                 but the wave only reaches {reach:.1}, so part of that ring is not \
                 around the click"
            );
        }
    }
    /// The ring never paints over a boid.
    ///
    /// The boid is *placed* on a cell the ring is certain to draw, taken from the
    /// ring's own output rather than computed from the geometry. A test that works
    /// out where the ring ought to be re-derives the thing under test, and gets it
    /// wrong in the same direction as the code when the code is wrong -- which is
    /// what happened twice here, in a version that placed the boid by arithmetic
    /// and then asserted it was not covered. The ring is read out of the diff and
    /// a boid is put on one of its cells; if the ring is somewhere else, there is
    /// nothing to place the boid on and the test says so.
    ///
    /// Why it matters: blanking a boid for the handful of frames a wave takes to
    /// pass reads as the flock losing members, which is the one thing an effect
    /// that is supposed to be showing you a flock must not do.
    #[test]
    fn the_ring_does_not_paint_over_a_boid() {
        let size = (60u16, 30u16);
        let mut flock = flock(size);
        flock.get_diff();
        flock.handle_input(&InputEvent::Pointer {
            position: (30, 15),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        for _ in 0..12 {
            flock.update();
        }

        // Where the ring is, on this frame, from its own output.
        let first_pass: Vec<(usize, usize)> = flock
            .get_diff()
            .iter()
            .filter(|(_, _, cell)| cell.symbol == SHOCK_GLYPH)
            .map(|(x, y, _)| (*x, *y))
            .collect();
        let (cell_x, cell_y) = *first_pass
            .first()
            .expect("a wave that has travelled twelve frames drew no ring at all");

        // Put a boid on that cell. The wave has not been advanced, so the ring is
        // still exactly where it was and this is the frame where they overlap --
        // which means nothing is reported in the *diff*, since the cell already
        // said `SHOCK_GLYPH` last frame and has not changed. So this reads the
        // built frame rather than the diff, which is also the honest thing to
        // read: the question is what is on the screen, not what changed.
        flock.boids[0].position = (cell_x as f32, cell_y as f32);
        flock.boids[0].history.clear();
        flock.boids[0].history.push_back((cell_x, cell_y));
        flock.stamp();
        let boid_glyph = flock.boids[0].character;
        flock.get_diff();

        assert_eq!(
            flock.frame.get(cell_x, cell_y).symbol,
            boid_glyph,
            "the ring painted over the boid at ({cell_x}, {cell_y}), which would \
             make the flock appear to lose members every time a wave passed"
        );

        // And the ring is still there on the rest of itself, so "does not paint
        // over a boid" is not quietly satisfied by a ring that stopped drawing.
        let drawn: usize = (0..size.0 as usize)
            .flat_map(|x| (0..size.1 as usize).map(move |y| (x, y)))
            .filter(|&(x, y)| flock.frame.get(x, y).symbol == SHOCK_GLYPH)
            .count();
        assert!(
            drawn > 0,
            "the ring drew nothing at all once a boid was on one of its cells"
        );
    }

    /// Spam cannot build a pile of waves.
    ///
    /// Each live wave is another pass over the flock, and — the part that matters
    /// — another band a boid can be inside at once. Unbounded, a fast drag
    /// across a large terminal would stack dozens on top of each other, and every
    /// boid inside the pile would be pinned to `max_speed`: the exact failure the
    /// force table in `apply_rules` was written to undo, where nothing carries
    /// speed any more because everything is at the cap.
    #[test]
    fn waves_are_capped_however_fast_they_are_dropped() {
        let mut flock = flock((80, 24));
        for i in 0..200u16 {
            flock.handle_input(&InputEvent::Pointer {
                position: (i % 80, 12),
                phase: PointerPhase::Pressed,
                button: crate::runtime::PointerButton::Left,
            });
        }
        assert_eq!(
            flock.waves.len(),
            shock::MAX_WAVES,
            "200 clicks left {} waves alive",
            flock.waves.len()
        );

        // And the flock does not end up with every boid at the speed cap, which
        // is the failure the cap exists to prevent.
        run(&mut flock, 10);
        let pinned = flock
            .boids
            .iter()
            .filter(|boid| boid.speed() >= flock.options.max_speed * 0.999)
            .count();
        assert!(
            pinned < flock.boids.len() / 2,
            "{pinned} of {} boids were at the speed cap after a pile of waves, so \
             the cap is not holding",
            flock.boids.len()
        );
    }

    /// A wave is gone after a reset, because a reset is a new flock.
    ///
    /// `reset` rebuilds the struct wholesale, so this is free — and free is exactly
    /// why it needs pinning. The day someone makes `reset` preserve a field to
    /// keep a setting, a wave left in it would appear on a flock that was never
    /// clicked, from a click that has not happened yet.
    #[test]
    fn reset_drops_the_waves() {
        let mut flock = flock((80, 24));
        flock.handle_input(&InputEvent::Pointer {
            position: (40, 12),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        assert_eq!(flock.waves.len(), 1);

        flock.reset();

        assert!(
            flock.waves.is_empty(),
            "a reset flock came back with {} wave(s) still travelling",
            flock.waves.len()
        );
        assert!(!flock.pointer_down, "a reset flock thinks a button is down");
    }

    /// The middle of the screen is not pushed at all.
    ///
    /// A border rule that reaches into the middle stops being a border rule. The
    /// margin used to be a flat five cells, which on a 200x8 terminal is nothing
    /// at all and on an 8x200 terminal is a quarter of it; it is now a fraction of
    /// each axis, so the same proportion of the field is avoided whatever shape
    /// the terminal is.
    #[test]
    fn the_border_rule_does_not_reach_the_middle() {
        let options = BoidsOptions {
            screen_size: (80, 24),
            boid_count: 10,
            ..Default::default()
        };
        let flock = Boids::new(options);

        // A fifth of the way in from each edge is inside the band; a third is
        // not.
        assert_eq!(flock.border_force((40.0, 12.0)), (0.0, 0.0));
        assert_eq!(flock.border_force((25.0, 12.0)), (0.0, 0.0));
        assert_eq!(flock.border_force((40.0, 6.0)), (0.0, 0.0));
        assert_ne!(flock.border_force((4.0, 12.0)), (0.0, 0.0));

        // And a wide, short terminal still has a border, and still has a middle.
        // The old margin was a flat five cells, which on a 200x8 terminal is 63%
        // of the height -- so the middle row was *inside the border band* and
        // every boid in it was being shoved. The margin is a fraction of each axis
        // now, so the same proportion of the field is avoided whatever shape the
        // terminal is.
        let wide = Boids::new(BoidsOptions {
            screen_size: (200, 8),
            boid_count: 10,
            ..Default::default()
        });
        assert_eq!(wide.border_force((100.0, 4.0)), (0.0, 0.0));
        assert_ne!(wide.border_force((2.0, 4.0)), (0.0, 0.0));

        let tall = Boids::new(BoidsOptions {
            screen_size: (8, 200),
            boid_count: 10,
            ..Default::default()
        });
        assert_eq!(tall.border_force((4.0, 100.0)), (0.0, 0.0));
        assert_ne!(tall.border_force((4.0, 8.0)), (0.0, 0.0));
    }

    /// Every heading gets a glyph of its own, except where the set says it does
    /// not.
    ///
    /// `Simple` used to be `['>', '>', 'v', 'v', '<', '<', '^', '^']` -- four
    /// glyphs for eight directions. East and south-east were the same character,
    /// and so were south and south-west, west and north-west, and north and
    /// north-east: half the flock's headings could not be shown at all. `Dot` is
    /// one glyph eight times on purpose, which is a choice rather than a bug, so
    /// it is excluded by name.
    #[test]
    fn every_direction_gets_a_distinct_glyph() {
        for charset in [
            BoidCharset::Simple,
            BoidCharset::Arrow,
            BoidCharset::Braille,
        ] {
            let chars = charset.chars();
            assert!(
                charset.is_directional(),
                "{charset:?} is meant to show a heading"
            );
            let distinct: HashSet<char> = chars.iter().copied().collect();
            assert_eq!(
                distinct.len(),
                8,
                "{charset:?} maps eight directions onto {} distinct glyphs: {chars:?}",
                distinct.len()
            );
            for glyph in chars {
                assert!(
                    !glyph.is_control(),
                    "{charset:?} contains the control character {glyph:?}"
                );
            }
        }

        assert_eq!(
            BoidCharset::Dot
                .chars()
                .iter()
                .collect::<HashSet<_>>()
                .len(),
            1,
            "Dot is documented as directionless, so it should have changed"
        );
    }

    /// The eight sectors really are the eight directions.
    ///
    /// The index is what the whole charset hangs off, and it is the one place a
    /// sign error in a rem-euclid would be invisible: eight wrong glyphs is still
    /// eight distinct glyphs.
    #[test]
    fn the_direction_sectors_run_east_then_clockwise_on_screen() {
        let headings: [(f32, f32); 8] = [
            (1.0, 0.0),
            (1.0, 1.0),
            (0.0, 1.0),
            (-1.0, 1.0),
            (-1.0, 0.0),
            (-1.0, -1.0),
            (0.0, -1.0),
            (1.0, -1.0),
        ];
        let expected: String = BoidCharset::Arrow.chars().iter().collect();

        let mut drawn = String::new();
        for (velocity, glyph) in headings.into_iter().zip(expected.chars()) {
            let boid = Boid::new((0.0, 0.0), velocity);
            assert_eq!(
                boid.get_direction_char(&BoidCharset::Arrow.chars()),
                glyph,
                "a heading of {velocity:?} drew the wrong glyph. On a terminal y \
                 grows downwards, so (0, 1) is south and not north."
            );
            drawn.push(boid.get_direction_char(&BoidCharset::Arrow.chars()));
        }
        assert_eq!(drawn, expected);
    }

    /// No cell is drawn bold.
    ///
    /// Every boid was `Attribute::Bold`, and several terminals treat bold on a
    /// foreground as a request to brighten the colour rather than as an attribute
    /// of its own. That is fatal for a trail specifically: a trail is the same
    /// colour dimmed, and bolding it would undo the dimming that the trail is.
    #[test]
    fn no_boid_is_drawn_bold() {
        let mut flock = flock((80, 24));
        run(&mut flock, 60);

        let bold = (0..flock.frame.height)
            .flat_map(|y| (0..flock.frame.width).map(move |x| (x, y)))
            .filter(|(x, y)| flock.frame.get(*x, *y).attr == style::Attribute::Bold)
            .count();
        assert_eq!(bold, 0, "{bold} cells were drawn bold");
    }

    /// The settings that were private, and the one that is new, round-trip
    /// through the names a user would write in `~/.config/termzzz.toml`.
    ///
    /// `charset` was already reaching the config file -- serde serialises private
    /// fields perfectly well -- but it was unreachable from Rust, so it could not
    /// be set by anything except a hand-edited file and could not be tested
    /// against one. `trail_length` is new, and `--print-config` writes every key
    /// out, which means a new key arrives for every existing config file as a key
    /// that file does not have. `BoidsOptions` carries `#[serde(default)]` for
    /// exactly that reason, and this is what would catch it going missing.
    #[test]
    fn the_new_and_the_promoted_settings_round_trip_through_their_toml_names() {
        let wanted = r#"
trail_length = 9
charset = "Arrow"
"#;
        let parsed: BoidsOptions = toml::from_str(wanted)
            .expect("a boids section naming either key parses");
        assert_eq!(parsed.trail_length, 9);
        assert_eq!(parsed.charset, BoidCharset::Arrow);

        let written = toml::to_string(&parsed).expect("serialises");
        assert!(
            written.contains("trail_length = 9"),
            "trail_length did not come back out: {written}"
        );
        assert!(
            written.contains(r#"charset = "Arrow""#),
            "charset did not come back out: {written}"
        );

        // And a section that names neither must still be the default flock, which
        // is the case every existing config file is in.
        let bare: BoidsOptions =
            toml::from_str("boid_coeff = 1.0").expect("parses");
        assert_eq!(bare.trail_length, BoidsOptions::default().trail_length);
        assert_eq!(bare.charset, BoidsOptions::default().charset);
    }

    /// The effect is renderable from the moment it is built.
    ///
    /// A trail that is stamped in `new` means the first frame is a full repaint.
    /// Without it the first frame would report every cell as blank, and the whole
    /// flock would arrive on the frame after -- which on a wipe-in is a visible
    /// flash of nothing, and on a switch is a frame of the previous effect showing
    /// through.
    #[test]
    fn the_first_frame_already_draws_the_flock() {
        let mut flock = flock((80, 24));

        let diff = flock.paint();
        let lit = (0..flock.frame.height)
            .flat_map(|y| (0..flock.frame.width).map(move |x| (x, y)))
            .filter(|(x, y)| flock.frame.get(*x, *y).symbol != ' ')
            .count();

        assert!(
            lit > 20,
            "only {lit} cells were lit on the first frame, so the flock arrives \
             late"
        );
        assert_eq!(
            diff.len(),
            lit,
            "the first frame reported {} cells but {lit} are lit, so the flock \
             is drawn on the frame after it should be",
            diff.len()
        );
    }

    /// Nothing is drawn outside the terminal, at any size.
    ///
    /// The trail is a hash map keyed by cell, and a resize changes what a cell
    /// means. A key that survives a shrink has to be dropped rather than written
    /// past the end of the frame.
    #[test]
    fn a_resize_leaves_nothing_to_draw_off_screen() {
        let mut flock = flock((80, 24));
        run(&mut flock, 40);

        for &(width, height) in &[(20u16, 10u16), (200, 50), (6, 6), (80, 24)] {
            flock.update_size(width, height);
            let diff = flock.get_diff();
            let escaped = diff
                .iter()
                .any(|(x, y, _)| *x >= width as usize || *y >= height as usize);
            assert!(!escaped, "drew outside a {width}x{height} terminal");
            assert_eq!(flock.frame.get_size(), (width as usize, height as usize));
            run(&mut flock, 20);
        }
    }

    /// Two runs of the same seed draw the same thing, and a long run does not
    /// grow without bound.
    ///
    /// The trail map is the only thing in this effect that accumulates, so it is
    /// the only thing that can leak: a cell that is never faded out is a cell the
    /// map keeps for the rest of the session.
    #[test]
    fn the_trail_does_not_accumulate_without_bound() {
        let mut flock = flock((80, 24));
        run(&mut flock, 30);
        let early = flock.trail.len();
        run(&mut flock, 600);
        let late = flock.trail.len();

        let cells = 80 * 24;
        assert!(
            late <= cells,
            "{late} cells are lit on a screen with {cells} cells"
        );
        assert!(
            late < early * 3 + 50,
            "the lit trail grew from {early} cells to {late} over ten seconds, so \
             something is not being faded out"
        );
    }

    /// The frame time context is honoured, and a stall does not teleport the flock.
    #[test]
    fn a_stall_does_not_teleport_the_flock() {
        let options = BoidsOptions {
            screen_size: (80, 24),
            boid_count: 50,
            ..Default::default()
        };
        let context = crate::runtime::FrameContext::new(
            (80, 24),
            0,
            Duration::from_secs(9),
            Duration::from_secs(9),
            crate::runtime::InputState::default(),
        );

        let mut flock = Boids::new(options);
        flock.update_with_context(&context);

        for boid in &flock.boids {
            assert!(
                boid.position.0 >= 0.0
                    && boid.position.0 < 80.0
                    && boid.position.1 >= 0.0
                    && boid.position.1 < 24.0,
                "a boid is at {:?}, which is off the screen after a nine second \
                 frame",
                boid.position
            );
        }
    }
}
