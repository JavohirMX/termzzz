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
        self.update_positions(scale);
        self.fade_trail(scale);
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

        let cells = self.shown.diff(&self.frame);
        for (x, y, cell) in &cells {
            self.shown.set(*x, *y, *cell);
        }
        cells
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
