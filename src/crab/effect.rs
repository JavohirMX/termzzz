use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::terrain::noise::PerlinNoise;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Direction the crab is facing
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Left,
    Right,
}

/// Walk poses per direction. Three, not two: a two-frame toggle alternates
/// between a planted pose and a lifted one with nothing between them, and at
/// five pose changes a second that reads as a shimmer rather than a scuttle.
/// Three poses give the cycle a middle -- contact, one stride, the other stride
/// -- so the legs are somewhere new every time rather than in one of two places.
const WALK_POSES: usize = 3;

/// Poses per direction including the clap, and so the stride between the
/// right-facing and left-facing halves of the table.
const POSES: usize = WALK_POSES + 1;

/// The pose index of the clap.
const CLAP_POSE: usize = WALK_POSES;

/// Rows in a sprite.
const SPRITE_ROWS: usize = 4;

/// The right-facing art: `POSES` poses of `SPRITE_ROWS` rows.
///
/// Authored once, at a fixed width, with every row's content centred so that
/// reversing the line mirrors the crab *inside its own bounding box* rather
/// than sliding it a column. A hand-mirrored sprite has to be redrawn by hand
/// every time the art changes, which is exactly what went wrong here: the
/// left-facing clap's first three lines were byte-identical to the
/// right-facing one, so its claws and its eyes were both on the wrong side and
/// only its legs were flipped.
///
/// `\/` doubles as "open claw" in the clap and as "raised claw" in the two
/// stride poses. That is deliberate rather than a shortage of characters: the
/// two readings are the same shape, a pincer that is not closed, which is what
/// the glyph already meant before this file was given a third use for it.
const RIGHT_POSES: [[&str; SPRITE_ROWS]; POSES] = [
    // Contact: both claws planted, legs splayed.
    [
        r"   _~^~^~^~_   ",
        r"\) /  o o  \ (/",
        r"  '_   ¬   _'  ",
        r"  \ '-----' /  ",
    ],
    // First stride: trailing claw raised, legs drawn in under the body.
    [
        r"   _~^~^~^~_   ",
        r"\/ /  o o  \ (/",
        r"  '_   ¬   _'  ",
        r"  | '-----' |  ",
    ],
    // Second stride: forward claw raised, legs crossed the other way.
    [
        r"   _~^~^~^~_   ",
        r"\) /  o o  \ \/",
        r"  '_   ¬   _'  ",
        r"  / '-----' \  ",
    ],
    // Clap: both claws open, mouth open, legs planted.
    [
        r"   _~^~^~^~_   ",
        r"\/ /  o o  \ \/",
        r"  '_   u   _'  ",
        r"  \ '-----' /  ",
    ],
];

/// Reverses one line of art.
///
/// Character-wise, not byte-wise, and that is the whole point of the function
/// existing. `str::reverse` does not exist and `chars().rev()` is the only
/// reversal that survives a multi-byte glyph: reversing UTF-8 *bytes* turns
/// `¬` into two replacement characters, so a mirrored sprite with an
/// accent in it comes out as mojibake rather than as a crab.
fn mirror(line: &str) -> String {
    line.chars().rev().collect()
}

/// The sprite table: `POSES` right-facing poses, then the same poses reversed.
///
/// Built once, by running [`mirror`] over [`RIGHT_POSES`]. See there for why
/// the left-facing art is derived rather than drawn.
static CRAB_FRAMES: LazyLock<Vec<Vec<String>>> = LazyLock::new(|| {
    let mut frames: Vec<Vec<String>> = RIGHT_POSES
        .iter()
        .map(|pose| pose.iter().map(|line| line.to_string()).collect())
        .collect();
    frames.extend(
        RIGHT_POSES
            .iter()
            .map(|pose| pose.iter().map(|line| mirror(line)).collect()),
    );
    frames
});

/// The sprite's width in characters, and the function that measures it.
///
/// The old measurement was `line.len()`, a *byte* count, against a renderer
/// that walks `line.chars().enumerate()`. It happened to be harmless only
/// because `¬` is two bytes and sat on a 13-character line whose byte length
/// was 14, exactly matching the other 14-byte lines. Any other multi-byte
/// glyph makes the two disagree, and the symptom is not a slightly wrong
/// bounce at the edge -- `frame_width` is what the edge clamp and the initial
/// colony spread are computed from, so the crabs stop short of one wall and
/// start short of the other.
fn sprite_width(rows: &[&str]) -> usize {
    rows.iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

/// The sprite's measured shape.
///
/// All four rows are the same width by construction, so this is the one number
/// the whole renderer agrees on. `no_cell_is_beyond_the_sprite` is the guard on
/// that: the art is hand-written, and a row that is one character short is the
/// kind of thing that survives a year of editing.
fn sprite() -> (usize, usize) {
    let width = sprite_width(&RIGHT_POSES[0]);
    (width, SPRITE_ROWS)
}

/// The seabed, by column.
///
/// A function of the column and nothing else, deliberately: a `rand` here would
/// give a different sand line every frame, and the canvas would report the
/// whole bottom row as changed sixty times a second for a static line.
const SAND: &[char] = &['-', '~', '.', '~', '-', '.'];

/// The sand's own colour: a dim, desaturated brown.
///
/// Not black and not `Color::Reset`. A seabed drawn in the terminal's default
/// foreground competes with the crabs for attention, and one drawn in black is a
/// black block on a light profile. A third colour that is neither is the only
/// version of "the ground" that is quieter than the thing standing on it in
/// both profiles.
const SAND_COLOUR: style::Color = style::Color::Rgb {
    r: 104,
    g: 88,
    b: 62,
};

/// A crab's shadow on the sand.
///
/// The sand's colour, two-thirds of the way to black, so it reads as a shadow
/// on either profile. Deriving it from [`SAND_COLOUR`] rather than picking a
/// grey is what keeps it the same *hue* as the ground it falls on.
const SHADOW_COLOUR: style::Color = style::Color::Rgb {
    r: 44,
    g: 37,
    b: 26,
};

/// The shadow glyph.
///
/// An underscore rather than another character from the sprite: the sprite is
/// drawn in the crab's colour and the shadow in the ground's, and if the two
/// shared a glyph then the only thing distinguishing a shadow from a second row
/// of crab would be a colour that a monochrome terminal throws away.
const SHADOW_GLYPH: char = '_';

/// Cells per second, per unit of horizontal velocity, per unit of
/// `movement_speed`.
///
/// 1.0, which makes the gain a no-op and puts the walk back where it started.
/// The constant exists because it was not always 1.0, and both values it has
/// held were chosen to answer a complaint about the same number.
///
/// It was 5.0, raised to fix a real one. `get_diff` rounds to whole cells, so a
/// colony moving 1.5 to 4.5 cells a second changed its drawn column once every
/// 13 to 40 frames -- measured at 80x24, once every 21. A sprite that jumps a
/// column a third of a second at a time reads as teleporting, and that was
/// worth fixing. There is no way to interpolate on a character grid, so the fix
/// had to be a faster crab: 5.0 put the default colony at 10.5 to 19.5 cells a
/// second, a cell every three to six frames, which is continuous to the eye.
///
/// It overshot, and the person who asked for the change then reported the
/// crabs "change too fast". Nineteen and a half cells a second is a crab
/// crossing a third of its own body length every pose -- the pose interval is
/// 0.2 seconds and the body does 3.9 cells in one -- and one drawn column every
/// three frames, so the whole sprite is somewhere else before the eye has
/// finished following it. 1.0 is back in the original band: 2.1 to 3.9 cells a
/// second after [`WALK_SPEED_RANGE`] narrowed the velocity range, which is one
/// drawn column every 15 to 29 frames.
///
/// The two complaints are the same knob, which is the part worth recording so
/// this is not "fixed" a third time. A crab is drawn one cell at a time, so its
/// drawn speed and its drawn smoothness are the same number: any gain high
/// enough to change the column every few frames also moves the crab several
/// cells between poses. There is no setting that buys both, and 5.0 bought
/// smoothness with speed and lost the other end.
///
/// Note what does *not* follow from lowering it. The pose cycle is not derived
/// from the walk -- `animation_speed` is a plain interval in seconds and the
/// legs cycle at `1 / animation_speed` whatever this is -- so the legs do not
/// slow down. What halves is the cells covered per leg position, which is the
/// other end of the same trade: at 0.6 cells a pose the crab is still moving
/// between leg positions rather than planting and then sliding.
const WALK_GAIN: f32 = 1.0;

/// The horizontal speed a crab is drawn from, as a fraction of a cell per
/// second.
///
/// Narrower than the old `-1.5..1.5`. A crab twice as fast as its neighbours
/// looks like it is trying to escape, and even at `WALK_GAIN` a fast end much
/// past 1.3 is a sprint rather than a personality; the width is a range of
/// characters, not of speeds.
const WALK_SPEED_RANGE: (f32, f32) = (0.7, 1.3);

/// Upward speed at the start of a hop, in cells per second.
///
/// With [`GRAVITY`] this puts the apex at about 1.8 cells -- just under half the
/// sprite's own height, which is a hop rather than a leap. A taller one stops
/// reading as a crab and starts reading as a projectile.
const HOP_SPEED: f32 = 9.0;

/// Downward acceleration while airborne, in cells per second squared.
const GRAVITY: f32 = 22.0;

/// Seconds a crab waits between hops, as a range.
const HOP_INTERVAL: (f32, f32) = (1.4, 5.0);

/// How close two crabs have to be before they turn around, in cells.
///
/// The sprite's own width, not a fixed six. At six the two crabs are already a
/// sprite-width apart by the time they react, which means they are drawn on top
/// of each other -- and a colony where a third of all pairs overlap is a heap,
/// not a shoal. The old constant also had a worse consequence: two adjacent
/// crabs re-triggered each other's clap every frame, resetting its timer before
/// it could run out, so a pair that met once stayed open-clawed for as long as
/// they were neighbours and the clap became a state rather than an event.
///
/// A sprite's width plus a gap, because two boxes that exactly abut still read
/// as one shape: at fifteen columns with no gap the claws of the left-hand crab
/// are drawn against the shell of the right-hand one.
///
/// Not much more than that. A distance well past the sprite reads as personal
/// space, and the crabs bounce off each other across a visible gap, which is a
/// different effect and a colder one.
fn touch_distance() -> f32 {
    sprite().0 as f32 + 2.0
}

/// The closest two crabs may stand to each other, in cells.
///
/// A floor, not a whole sprite's width, and the screen is the reason. The
/// sprite is fifteen columns across, so a fifteen-column gap needs a terminal
/// of about a hundred and forty before even three crabs fit on it, and this
/// effect is required to run at six by six. Six keeps the two shells clear of
/// one another -- at three the claws of the left-hand crab are inside the body
/// of the right-hand one -- without asking the seabed for a sprite per crab,
/// which turns a crossing colony into a queue.
///
/// Anything above one cell carries a second property, and it is the one the
/// report was actually about. `get_diff` rounds to whole columns, so two crabs
/// a tenth of a cell apart are *drawn* in the same column, and two sprites that
/// overlap on fifteen of their fifteen columns are one crab, not two. A gap of
/// one is the smallest that can guarantee two crabs are never painted on top of
/// each other, and a screen narrower than the colony cannot give even that --
/// see [`separation`].
const MIN_SEPARATION: f32 = 6.0;

/// The gap to actually enforce, given a screen and a colony size.
///
/// [`MIN_SEPARATION`] where the seabed can afford it, and otherwise a share of
/// the seabed per crab.
///
/// The second branch is not a nicety. `right_edge` is zero on any terminal
/// narrower than the sprite -- which includes the six by six the contract suite
/// drives -- so five crabs on a small screen all integrate to column zero and
/// there is no gap between anything. Demanding six cells there would be a
/// demand the screen cannot pay, and the two ways of trying to pay it are both
/// worse than the problem: refuse to move, which is a frozen seabed, or push a
/// crab off the edge, which is a crab on the moon.
///
/// Divided by `count` rather than by `count - 1`, and the difference of one is
/// the whole of it. There are only `count - 1` gaps between `count` crabs, so a
/// gap of `span / (count - 1)` uses the seabed exactly to the last column and
/// the colony is then *rigid*: at twenty by nine, three crabs are pinned to
/// columns 0, 2.5 and 5 forever and the drawn column changes seven times in a
/// hundred seconds. One gap's worth of slack lets them shuffle along it, which
/// is the difference between a crowded seabed and a frozen one.
///
/// Capping at that share is also what makes the sweep in
/// [`Crab::separate_crabs`] provably stay on the screen. It guarantees
/// `(count - 1) * separation < span`, which is the inequality the whole of that
/// function rests on, and
/// `the_gap_asked_for_never_exceeds_what_the_seabed_can_hold` is its test.
fn separation(screen_width: u16, count: usize) -> f32 {
    if count < 2 {
        return 0.0;
    }
    let span = right_edge(screen_width, sprite().0);
    MIN_SEPARATION.min(span / count as f32)
}

// Individual crab entity
#[derive(Clone)]
struct CrabEntity {
    position: (f32, f32), // Floating point for smooth movement
    velocity: (f32, f32), // Direction and speed
    direction: Direction, // Facing left or right
    current_frame: usize, // Current animation frame
    animation_timer: f32, // Timer for animation
    special_timer: f32,   // Timer for special animations
    is_special: bool,     // Whether doing special animation
    hop_timer: f32,       // Seconds until the next hop
    color: style::Color,  // Crab color
    /// Whether this crab is in the air because it was startled.
    ///
    /// An explicit flag rather than the obvious `position.1 < ground` test,
    /// because that test is a float comparison against a *sampled* value and
    /// the sample moves. On a flat seabed the ground never changed under a
    /// crab, so `position.1` always equalled it exactly and the comparison was
    /// free. With a slope the ground rises and falls under a walking crab, and
    /// a crab that happens to be a hundredth of a row above its ground reads as
    /// airborne -- so it could not turn away from a neighbour, and the collision
    /// response silently stopped running for anything walking downhill. That is
    /// the same class of bug as the one this flag exists to avoid: inferring a
    /// state from a measurement when the state is already known.
    airborne: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CrabOptions {
    #[serde(skip)]
    pub crab_count: u16,

    /// Seconds between walk poses.
    ///
    /// The walk rate and nothing else. It used to be both this and a multiplier
    /// on the clap duration -- `animation_speed * 5.0` -- so a user who wanted a
    /// livelier walk also shortened the crab's clap, which is not a thing anyone
    /// would ask for and is invisible in a config file because both are the
    /// same number. [`clap_duration`](Self::clap_duration) is the second half.
    ///
    /// This is *not* derived from the walk speed, which was the assumption when
    /// the walk was slowed for being too fast. It is a plain timer, so a crab
    /// whose body has been slowed from 15 cells a second to 3 still snaps
    /// between leg poses five times a second -- and at that rate the legs read as
    /// a flicker rather than as a scuttle, which is the complaint the walk speed
    /// was slowed for in the first place. 0.4 is two and a half poses a second.
    pub animation_speed: f32,

    /// How long a clap lasts, in seconds.
    ///
    /// Independent of [`animation_speed`](Self::animation_speed) so the two can
    /// be tuned separately. 0.6 is a quarter of a second, long enough to see the
    /// claws open and short enough that a colony of clapping crabs does not look
    /// like a colony of statues.
    pub clap_duration: f32,

    pub clap_chance: f32, // Random chance for special animation

    pub movement_speed: f32,

    pub crab_coeff: f32,

    /// Seed for the initial colony and every movement, turn and clap after it.
    pub seed: u64,

    /// How far the seabed rises above its lowest point, in rows.
    ///
    /// The reported behaviour was "they are only walking at the very bottom;
    /// they are not going up", and the cause was not a bug: the ground was a
    /// single constant row, `(height - 1) - SPRITE_ROWS`, and a crab's `y` was
    /// clamped to exactly it. The only way off it was a hop. Every piece of
    /// writing in this file called that a *seabed*, which is what made it look
    /// like something was broken rather than something absent.
    ///
    /// 7.0, which is about 3 rows of actual relief on a 24-row terminal.
    ///
    /// The two numbers are not the same and the reason is worth stating, because
    /// 7.0 sounds like far too much slope and 3.0 measured as one row. The
    /// profile is octave noise, and octave noise does not reach +/-1: measured
    /// over four thousand samples at three different periods, this generator
    /// spans 0.79, so a nominal amplitude of 3 moves the ground 1.2 rows, which
    /// rounds to one row on a flat stretch and is invisible. Scaled, 7.0 gives
    /// the intended relief.
    ///
    /// Three rows of relief is small on purpose: a crab is four rows tall, so a
    /// profile with more relief than the sprite is tall starts to read as a
    /// cliff face, and a crab partway up one looks like it is falling.
    pub seabed_amplitude: f32,

    /// Cells between one rise of the seabed and the next.
    ///
    /// Long, so the slope is a couple of gentle banks across a screen rather
    /// than a rippled texture the crabs appear to rattle along.
    pub seabed_period: f64,
}

impl Default for CrabOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            crab_count: 5,
            animation_speed: 0.4,
            clap_duration: 0.6,
            clap_chance: 0.05,
            movement_speed: 3.0,
            crab_coeff: 1.0,
            seed: DEFAULT_SEED,
            seabed_amplitude: 7.0,
            seabed_period: 34.0,
        }
    }
}

/// The seabed: a seeded height profile the crabs walk along.
///
/// A separate type rather than a function of `(x, size)` because it is state.
/// The profile scrolls, so the ground under a given column changes as the
/// colony walks, and answering "where is the ground" from a stateless function
/// would mean threading the accumulated offset through the renderer, the
/// integrator and the collision sweep separately -- three chances to disagree,
/// and a disagreement here is a crab drawn inside the sand.
struct Seabed {
    noise: PerlinNoise,
    /// Terminal rows. The profile is clamped into this.
    height: usize,
    /// Rows of relief above the lowest point.
    amplitude: f32,
    /// Cells per period, guarded.
    period: f64,
    /// Columns scrolled past, accumulated from the frame delta.
    offset: f64,
}

impl Seabed {
    fn new(options: &CrabOptions, height: usize) -> Self {
        Self {
            noise: PerlinNoise::new(options.seed ^ SEABED_SEED_SALT),
            height,
            amplitude: if options.seabed_amplitude.is_finite() {
                options.seabed_amplitude.max(0.0)
            } else {
                0.0
            },
            period: if options.seabed_period.is_finite()
                && options.seabed_period > 0.0
            {
                options.seabed_period
            } else {
                34.0
            },
            offset: 0.0,
        }
    }

    /// The lowest the ground ever goes: the row a crab's sprite top sits on when
    /// the profile is at its bottom, which is also what keeps the sprite inside
    /// the screen.
    fn base_row(&self) -> f32 {
        self.height as f32 - 1.0 - SPRITE_ROWS as f32
    }

    /// The ground at a column, as the sprite's top row.
    ///
    /// Smaller is higher on the screen, so a positive noise value lifts the
    /// ground. Clamped so the sprite cannot leave the bottom of the screen --
    /// `saturating_sub` in spirit, and the floor is row 0 rather than row 1
    /// because a terminal shorter than the sprite plus a row of sky has no
    /// room for the sky, and clipping the sprite is better than pushing it off
    /// the bottom. At any usable size the floor is nowhere near reachable: the
    /// profile lives within `amplitude` rows of `base_row`.
    fn row_at(&self, x: f32) -> f32 {
        let base = self.base_row();
        if self.amplitude <= 0.0 {
            return base.max(0.0);
        }
        // The `y = 0` line of the noise, which is what makes this a one
        // dimensional profile: `noise_2d` interpolates on y first and y is
        // zero, so only the bottom row of gradients contributes. Same trick
        // `terrain` uses for its height field.
        let height = self.noise.octave_noise_2d(
            x as f64 + self.offset,
            0.0,
            SEABED_OCTAVES,
            SEABED_PERSISTENCE,
            1.0 / self.period,
        ) as f32;
        (base - height * self.amplitude).clamp(0.0, base.max(0.0))
    }

    /// The row of sand directly under a crab standing at `x`.
    fn sand_at(&self, x: f32) -> usize {
        let foot = self.row_at(x) + SPRITE_ROWS as f32 - 1.0;
        (foot.round() as isize).clamp(0, self.height as isize - 1) as usize
    }

    /// Advances the scroll, in the same units as the walk so a crab stays put
    /// relative to the ground under it.
    fn advance(&mut self, delta: f64, walk_speed: f64) {
        self.offset += walk_speed * delta;
    }
}

/// Octaves and persistence for the seabed profile.
///
/// Two, not four. The seabed is a couple of gentle banks, and a finely
/// detailed floor would put high-frequency ripple under a crab that walks in
/// straight lines -- the crab would appear to jitter against ground that is
/// itself moving, which is a much worse artefact than a plain slope.
const SEABED_OCTAVES: i32 = 2;
const SEABED_PERSISTENCE: f64 = 0.5;

/// Mixed into the seed so the seabed is not the same shape as anything else
/// derived from the same seed in the crate.
const SEABED_SEED_SALT: u64 = 0x5EA8_ED00_D15E_ABED;

pub struct Crab {
    pub screen_size: (u16, u16),
    options: CrabOptions,
    canvas: Canvas,
    crabs: Vec<CrabEntity>,
    rng: EffectRng,
    frame_timer: f32,
    /// The ground the crabs are walking on. Held rather than recomputed so the
    /// renderer, the integrator and the collision sweep cannot disagree about
    /// where it is -- see [`Seabed`].
    seabed: Seabed,
}

impl CrabEntity {
    fn new(
        position: (f32, f32),
        velocity: (f32, f32),
        hop_timer: f32,
        rng: &mut EffectRng,
    ) -> Self {
        // Determine initial direction based on velocity
        let direction = if velocity.0 >= 0.0 {
            Direction::Right
        } else {
            Direction::Left
        };

        // Random color with predominantly red tint for crabs
        let color = style::Color::Rgb {
            r: rng.random_range(200..=255),
            g: rng.random_range(50..=150),
            b: rng.random_range(50..=100),
        };

        Self {
            position,
            velocity,
            direction,
            current_frame: 0,
            animation_timer: 0.0,
            special_timer: 0.0,
            is_special: false,
            airborne: false,
            hop_timer,
            color,
        }
    }

    /// The pose to draw.
    fn frame_index(&self) -> usize {
        let pose = if self.is_special {
            CLAP_POSE
        } else {
            self.current_frame % WALK_POSES
        };
        pose + match self.direction {
            Direction::Right => 0,
            Direction::Left => POSES,
        }
    }

    /// The lines of the current pose, borrowed.
    fn frame_lines(&self) -> &[String] {
        &CRAB_FRAMES[self.frame_index()]
    }

    /// Update the crab's position and animation state
    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        dt: f32,
        screen_size: (u16, u16),
        seabed: &Seabed,
        animation_speed: f32,
        clap_duration: f32,
        movement_speed: f32,
        clap_chance: f32,
        rng: &mut EffectRng,
    ) {
        let (width, _height) = sprite();
        let _ = screen_size;

        // Horizontal first, then the ground under the column it arrived at.
        //
        // The order is load-bearing and was wrong at first. Sampling the ground
        // before the move leaves `position.1` pinned to the profile at the
        // crab's *old* column while `position.0` has already advanced, so
        // anything comparing the two -- the collision response's "is this crab
        // airborne" test -- reads a crab walking downhill as permanently off the
        // ground, and a crab on a slope never turns away from anything.
        self.position.0 += self.velocity.0 * movement_speed * WALK_GAIN * dt;
        // The ground under *this* crab's own column, not a single row for the
        // screen. That is the whole of the reported bug: with one shared row
        // every crab stood on the same line however far along the seabed it was.
        let ground = seabed.row_at(self.position.0);

        // Gravity first, so a hop that ends this frame still lands.
        if self.position.1 < ground {
            self.velocity.1 += GRAVITY * dt;
        }

        self.position.1 += self.velocity.1 * dt;

        // Screen boundary collision detection.
        //
        // Both ends clamped rather than one end reflected. The old code
        // reflected the position to the edge and re-rolled the velocity, which
        // on a terminal narrower than the sprite set the position to a
        // *negative* column; `position.round() as usize` saturates that to
        // zero, so the crab survived, but only because the cast saturates and
        // for the wrong reason.
        let max_x = right_edge(screen_size.0, width);
        if self.position.0 < 0.0 {
            self.position.0 = 0.0;
            self.velocity.0 =
                rng.random_range(WALK_SPEED_RANGE.0..WALK_SPEED_RANGE.1);
        } else if self.position.0 > max_x {
            self.position.0 = max_x;
            self.velocity.0 =
                -rng.random_range(WALK_SPEED_RANGE.0..WALK_SPEED_RANGE.1);
        }

        // Vertical. A crab lives on the sand and leaves it only to hop, so the
        // old random `velocity.1` is gone: it was drawn from `-0.5..0.5` and
        // never damped toward anything, which is why the crabs read as floating
        // rather than as scuttling. Now the only way off the ground is a hop,
        // gravity brings them back, and the shadow shrinks as they rise.
        if self.position.1 >= ground {
            self.position.1 = ground;
            self.velocity.1 = 0.0;
            self.airborne = false;
        }
        if self.position.1 < 0.0 {
            self.position.1 = 0.0;
            self.velocity.1 = 0.0;
        }

        // Update direction based on velocity
        if self.velocity.0 > 0.0 {
            self.direction = Direction::Right;
        } else if self.velocity.0 < 0.0 {
            self.direction = Direction::Left;
        }

        // Update animation timer
        let is_moving = self.velocity.0.abs() > 0.1 || self.position.1 < ground;
        if is_moving {
            self.animation_timer += dt;
            let interval = if animation_speed > 0.0 {
                animation_speed
            } else {
                CrabOptions::default().animation_speed
            };
            if self.animation_timer >= interval {
                self.animation_timer = 0.0;
                self.current_frame = (self.current_frame + 1) % WALK_POSES;
            }
        } else {
            // Use standing frame when not moving
            self.animation_timer = 0.0;
            self.current_frame = 0;
        }

        // Update special animation
        if self.is_special {
            self.special_timer -= dt;
            if self.special_timer <= 0.0 {
                self.is_special = false;
            }
        } else if rng.random::<f32>() < clap_chance * dt {
            // Random chance to trigger special animation
            self.is_special = true;
            self.special_timer = clap_duration;
        }

        // The next hop. Counted down whether the crab is on the ground or in the
        // air, so a long hop does not come due twice in one landing.
        self.hop_timer -= dt;
        if self.hop_timer <= 0.0 {
            self.hop_timer = rng.random_range(HOP_INTERVAL.0..HOP_INTERVAL.1);
            // Only from the ground: a second hop mid-air would stack two
            // impulses and launch the crab off the top of the screen.
            if self.position.1 >= ground {
                self.velocity.1 = -HOP_SPEED;
            }
        }

        // Occasionally add slight randomness to movement
        if rng.random::<f32>() < 0.02 {
            self.velocity.0 += rng.random_range(-0.2..0.2);

            // Keep velocity in reasonable bounds
            if self.velocity.0.abs() > WALK_SPEED_RANGE.1 {
                self.velocity.0 = self.velocity.0.signum() * WALK_SPEED_RANGE.1;
            }
        }
    }

    /// A startled hop, from two crabs meeting.
    fn startle(&mut self, rng: &mut EffectRng) {
        self.velocity.1 = -HOP_SPEED * rng.random_range(0.5..0.8);
        self.hop_timer = rng.random_range(HOP_INTERVAL.0..HOP_INTERVAL.1);
        self.airborne = true;
    }
}

/// The rightmost column a crab's sprite may start at.
fn right_edge(screen_width: u16, sprite_width: usize) -> f32 {
    (screen_width as f32 - sprite_width as f32).max(0.0)
}

/// One function used to answer "where is the ground" for the whole screen, and
/// three places called it: integration, the renderer, and the collision
/// response's question of whether a crab was on the ground. As three separate
/// expressions they could drift, and a drift there is not a wrong pixel -- it is
/// a crab that believes it is airborne and so can never be startled, forever.
///
/// It is [`Seabed::row_at`] now, and it takes the column, because on a sloping
/// seabed there is no single answer to give.
///
/// The width of a crab's shadow, in cells, at a given height above the sand.
///
/// Narrowing with altitude is the whole of the shadow's second job. A shadow
/// that keeps its size at every height is a mark on the ground rather than a
/// shadow, and it is the one cue that says a hop is a hop and not a crab
/// painted in two places.
///
/// The base is 0.62 of the sprite, not all of it. A shadow as wide as the
/// animal is as wide as the sand, and the two run together into one band across
/// the bottom of the screen -- measured on a dumped frame, the sand's own
/// pattern was invisible between the shadows. A shade narrower than its caster
/// reads as a pool of shadow underneath it, and leaves the sand visible at both
/// ends, which is what tells the eye there is ground under there at all.
fn shadow_span(sprite_width: usize, altitude: f32) -> f32 {
    let shrink = 1.0 - (altitude / 3.0).clamp(0.0, 0.75);
    (((sprite_width as f32) * 0.62) * shrink).max(1.0)
}

impl TerminalEffect for Crab {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear();

        let (width, _height) = sprite();
        let seabed = &self.seabed;
        let screen_height = self.canvas.height();

        // The seabed: filled from the profile down to the bottom of the screen,
        // per column. It used to be a single row of sand along the bottom edge,
        // which is what made the crabs look like they were walking on the frame
        // rather than on anything. Filling it is also what makes the slope
        // visible -- a profile with nothing under it is a line, not a bank.
        for x in 0..self.canvas.width() {
            let top = (seabed.row_at(x as f32).round() as isize)
                .clamp(0, screen_height as isize - 1)
                as usize;
            for y in top..screen_height {
                self.canvas.set(
                    x,
                    y,
                    Cell::new(
                        SAND[(x + y) % SAND.len()],
                        SAND_COLOUR,
                        style::Attribute::Reset,
                    ),
                );
            }
        }

        for crab in &self.crabs {
            let base_x = crab.position.0.round().max(0.0) as usize;
            let base_y = crab.position.1.round().max(0.0) as usize;
            // The sand under this crab, which is not the bottom row any more.
            let sand_row = seabed.sand_at(crab.position.0);

            // The shadow: a run of underscores on the sand, narrowing as the
            // crab rises. A shadow that stays the same size at every height is
            // a mark on the ground rather than a shadow, and it is the one cue
            // that says the crab is *above* the sand rather than printed on it.
            let altitude =
                (seabed.row_at(crab.position.0) - crab.position.1).max(0.0);
            let span = shadow_span(width, altitude);
            let centre = base_x as f32 + (width as f32) * 0.5;
            let from = (centre - span * 0.5).round().max(0.0) as usize;
            let to = (centre + span * 0.5).round().max(0.0) as usize;
            for x in from..to.min(self.canvas.width()) {
                self.canvas.set(
                    x,
                    sand_row,
                    Cell::new(SHADOW_GLYPH, SHADOW_COLOUR, style::Attribute::Reset),
                );
            }

            for (y_offset, line) in crab.frame_lines().iter().enumerate() {
                let y = base_y + y_offset;
                if y >= self.canvas.height() {
                    continue;
                }

                for (x_offset, ch) in line.chars().enumerate() {
                    let x = base_x + x_offset;
                    if x >= self.canvas.width() || ch == ' ' {
                        continue;
                    }

                    // `Attribute::Reset`, not `Attribute::Bold`. Every crab cell
                    // was bold, and bold on a truecolor foreground is a
                    // rendering hint that many terminals act on by brightening
                    // the colour -- so two crabs of the same drawn colour came
                    // out visibly different, which is not something a random hue
                    // should be able to do. Nothing here is a brightness ramp,
                    // so there is nothing for it to spoil either.
                    self.canvas.set(
                        x,
                        y,
                        Cell::new(ch, crab.color, style::Attribute::Reset),
                    );
                }
            }
        }

        self.canvas.commit()
    }

    fn update(&mut self) {
        // The rate the defaults were tuned against. Driven through the plain
        // `update` path, the colony still walks at the speed it was designed
        // for; the frame loop supplies the real delta below.
        self.step(1.0 / 30.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not teleport the colony across the screen.
        self.step(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        let area = self.screen_size.0 as f32 * self.screen_size.1 as f32;
        self.options.crab_count =
            (area / 800.0 * self.options.crab_coeff).clamp(3.0, 15.0) as u16;
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Crab {
    fn step(&mut self, dt: f64) {
        self.frame_timer += dt as f32;
        // The ground scrolls at exactly the rate the crabs walk, so a crab holds
        // one contour of the seabed instead of walking across a slope that is
        // itself moving underneath it.
        //
        // Negative, and the direction is worth deriving rather than guessing
        // because the first version of this comment had it backwards. The profile
        // is sampled at `x + offset`. A feature sitting at column `x1` when the
        // offset is `o1` is the same feature at column `x2` when it is `o2` when
        // `x2 + o2 == x1 + o1`. So as the offset *falls*, every feature moves to
        // **higher** columns: the ground travels right, which is the direction a
        // crab with positive velocity walks. The two rates match, so a crab is
        // stationary in profile coordinates.
        //
        // `the_ground_scrolls_with_the_colony_rather_than_against_it` measures
        // that rather than trusting the algebra, because reversed is invisible in
        // a still frame: the crab is still exactly on the ground under it, it
        // just descends slopes it never climbed, at twice the colony's speed.
        self.seabed.advance(
            dt,
            -(f64::from(self.options.movement_speed) * f64::from(WALK_GAIN)),
        );

        // Borrowed rather than cloned: `crab.update` needs to ask the seabed
        // where the ground is under its own column, and `self.crabs` and
        // `self.seabed` are two fields of the same struct.
        let seabed = &self.seabed;
        for crab in &mut self.crabs {
            crab.update(
                dt as f32,
                self.screen_size,
                seabed,
                self.options.animation_speed,
                self.options.clap_duration,
                self.options.movement_speed,
                self.options.clap_chance,
                &mut self.rng,
            );
        }

        // Separation before the collision response rather than after it, and the
        // order is the point. The response fires at a sprite's width -- three
        // gaps -- so a pair that is genuinely on a collision course has already
        // turned around by the time a correction could reach it, and the
        // correction has nothing left to do. What it *does* have to fix is the
        // pair the response never sees at all: `check_crab_collisions` acts only
        // on a closing pair, and two crabs in the same column travelling the
        // same way are not closing on each other, so they were skipped forever
        // and stayed stacked for the rest of the run. That was the whole of the
        // report, and there was no code here that could have answered it.
        self.separate_crabs();
        self.check_crab_collisions();
    }

    pub fn new(options: CrabOptions, screen_size: (u16, u16)) -> Self {
        // One generator for the whole colony, drawn from sequentially, so the
        // crabs diverge from each other the way separate draws would.
        let mut rng = seeded_rng(options.seed, "crab");
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        let (sprite_width, _sprite_height) = sprite();

        // Every crab starts on the sand. The old code scattered them anywhere
        // in the frame, which is what made the effect read as a shoal of
        // sprites drifting in space rather than as animals on a seabed.
        let seabed = Seabed::new(&options, screen_size.1 as usize);

        // Spread the colony along the sand, then let it walk. Each crab starts
        // on the profile at its own column, so a colony dropped onto a slope
        // begins on the slope rather than hovering above it and dropping.
        let max_x = right_edge(screen_size.0, sprite_width);
        let columns = Self::spread(&mut rng, max_x, options.crab_count as usize);

        let mut crabs = Vec::with_capacity(options.crab_count as usize);
        for column in columns {
            let ground = seabed.row_at(column);
            let velocity = (
                rng.random_range(WALK_SPEED_RANGE.0..WALK_SPEED_RANGE.1)
                    * if rng.random::<f32>() < 0.5 { -1.0 } else { 1.0 },
                0.0,
            );
            let hop_timer = rng.random_range(HOP_INTERVAL.0..HOP_INTERVAL.1);
            crabs.push(CrabEntity::new(
                (column, ground),
                velocity,
                hop_timer,
                &mut rng,
            ));
        }

        let seabed = Seabed::new(&options, screen_size.1 as usize);
        Self {
            screen_size,
            options,
            canvas,
            crabs,
            rng,
            frame_timer: 0.0,
            seabed,
        }
    }

    /// Lays the colony out along the sand without stacking.
    ///
    /// Two requirements, tried in that order: a whole sprite's width between
    /// neighbours, and then only enough to keep two crabs out of the same
    /// column. The first is what "a colony" means; the second is all a
    /// six-column terminal can manage, and a crab is fifteen columns wide, so
    /// overlap there is unavoidable -- spreading to the ideal and then refusing
    /// to place the remainder would leave the effect with fewer crabs than its
    /// `crab_count` says, which is a worse answer than a crowded seabed.
    ///
    /// The minimum gap is one whole cell because the renderer rounds: two
    /// positions a cell apart always round to different columns, so one cell is
    /// the smallest gap that can guarantee it.
    fn spread(rng: &mut EffectRng, max_x: f32, count: usize) -> Vec<f32> {
        const ATTEMPTS: usize = 24;
        let (sprite_width, _) = sprite();
        let mut columns = vec![0.0f32; count];

        for index in 0..count {
            let mut placed = false;
            for gap in [sprite_width as f32 + 2.0, 1.0] {
                for _ in 0..ATTEMPTS {
                    let candidate = rng.random_range(0.0..=max_x);
                    if columns[..index]
                        .iter()
                        .all(|other| (other - candidate).abs() >= gap)
                    {
                        columns[index] = candidate;
                        placed = true;
                        break;
                    }
                }
                if placed {
                    break;
                }
            }
            if !placed {
                columns[index] = rng.random_range(0.0..=max_x);
            }
        }
        columns
    }

    /// Keeps the colony from standing on top of itself, every frame.
    ///
    /// Along the seabed axis only, and that is a decision rather than a
    /// shortcut. The general answer -- push two things apart along the line
    /// between them, the way a boids flock separates -- is wrong for this
    /// scene: crabs share one row of ground and hop a cell or two off it, so the
    /// line between a grounded crab and one mid-hop is nearly vertical, and
    /// separating along it lifts them. A colony that swims is not a colony on a
    /// seabed. The horizontal component is also the component that decides
    /// whether they overlap at all, since every crab is drawn within
    /// [`SPRITE_ROWS`] rows of the same sand row, so correcting along it is the
    /// whole correction and nothing is being left out.
    ///
    /// It moves positions rather than velocities, which is the thing to be
    /// careful about, because two crabs pushing each other along a line can push
    /// themselves back and forth forever and the usual answer is to damp the
    /// correction. This one does not need damping. A sorted sweep is a
    /// *projection*, not a force: each crab is moved by exactly the overlap and
    /// no more, the movement is always in the same direction for a given
    /// arrangement, and once the gaps are wide enough the sweep does nothing at
    /// all. A second pass over an already-separated colony is therefore a no-op,
    /// which is what settles it -- and that is a property worth a test rather
    /// than a claim, so `separation_settles_rather_than_jittering` is the
    /// assertion.
    ///
    /// Sorted rather than a pairwise loop over indices, for the same reason. A
    /// pairwise loop corrects in index order, so crab 0 and crab 1 can each be
    /// shoved by a *different* neighbour in the same frame and end the frame
    /// exactly where they started -- and with three crabs in a line that is not
    /// a corner case, it is the only case. The order is by column with the
    /// index as the tie-break, so the sweep is a function of the positions
    /// alone: nothing here draws from the generator and a run replays.
    fn separate_crabs(&mut self) {
        let count = self.crabs.len();
        if count < 2 {
            return;
        }
        let min_gap = separation(self.screen_size.0, count);
        if min_gap <= 0.0 {
            return;
        }
        let max_x = right_edge(self.screen_size.0, sprite().0);

        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|&a, &b| {
            self.crabs[a]
                .position
                .0
                .total_cmp(&self.crabs[b].position.0)
                .then(a.cmp(&b))
        });

        // Left to right, each crab lifted to at least a gap behind the one in
        // front of it, and held inside the right-hand wall. One pass, because a
        // crab moved here is the `behind` the next crab in the order is measured
        // against -- which is also why the order has to be by column and not by
        // index. A gap between adjacent crabs of a sorted order is a gap between
        // all of them, since everything between has to be between them too.
        for window in order.windows(2) {
            let (behind, ahead) = (window[0], window[1]);
            let floor = self.crabs[behind].position.0 + min_gap;
            let lifted = self.crabs[ahead].position.0.max(floor);
            // The clamp is load-bearing, not defensive. Without it a colony
            // that is wider than the seabed walks the back of itself off the
            // right-hand side, and the whole two-pass argument below is about
            // keeping every column inside `[0, max_x]`.
            self.crabs[ahead].position.0 = lifted.min(max_x);
        }

        // Right to left, each crab drawn back to at most a gap ahead of the one
        // in front of it. This is what the first pass cannot do, and the reason
        // is the clamp: five crabs bunched at the right-hand wall all clamp to
        // the same column there, and only a pass running the other way can tell
        // them apart.
        //
        // Two passes and no third, and that is the whole correctness argument
        // rather than a guess. Writing `a` for the columns the first pass left
        // and `f` for the result, `f[k] = min(a[k], f[k+1] - gap)`, which
        // unfolds to `f[k] = min over j >= k of (a[j] - (j - k) * gap)`. The
        // gap between neighbours is then
        // `f[k] - f[k-1] = max(P - a[k-1], gap)` for `P = f[k]`, which is never
        // below `gap` however the two passes came out. And the bounds: `f` never
        // rises above `a`, so nothing goes past the right-hand wall, and where
        // the first pass's clamp bit, `f[k-1] = max_x - gap` and the chain
        // continues leftwards as `max_x - m * gap`, which stays non-negative for
        // every `m < count` precisely because `separation` guarantees
        // `(count - 1) * gap <= max_x`. That inequality is the one thing this
        // function leans on, and
        // `the_gap_asked_for_never_exceeds_what_the_seabed_can_hold` is its
        // test.
        for window in order.windows(2).rev() {
            // Same reading as the first pass, not the mirrored one: `windows(2)`
            // slides forward either way, so `window[0]` is still the crab
            // further left. Reversing the iteration is not reversing the pair.
            let (behind, ahead) = (window[0], window[1]);
            let ceiling = self.crabs[ahead].position.0 - min_gap;
            if self.crabs[behind].position.0 > ceiling {
                self.crabs[behind].position.0 = ceiling;
            }
        }
    }

    // Check for collisions between crabs and handle them
    fn check_crab_collisions(&mut self) {
        let crab_count = self.crabs.len();
        if crab_count < 2 {
            return;
        }

        // The turn-away and the clap are gated separately, and that split is the
        // fix. Gating the whole response on "neither crab is already clapping"
        // stopped the re-clapping -- without it, two crabs re-triggered each
        // other every frame, which resets `special_timer` before it can run out,
        // so a pair that met once stayed open-clawed for as long as they were
        // neighbours and the clap became a state rather than an event. But it
        // also stopped the *turn-away*, and that is the part which is not
        // cosmetic: a crab whose neighbour happened to be clapping walked
        // straight through it.
        //
        // So: always reverse, always hop, and clap only if neither is already
        // clapping. The clap's own duration is the cooldown.
        //
        let reach = touch_distance();
        let reach = reach * reach;
        for i in 0..crab_count {
            for j in (i + 1)..crab_count {
                let dx = self.crabs[i].position.0 - self.crabs[j].position.0;
                let dy = self.crabs[i].position.1 - self.crabs[j].position.1;
                let distance_squared = dx * dx + dy * dy;

                if distance_squared >= reach {
                    continue;
                }
                // Only for a pair that is closing *and* head-on. The two halves
                // are different questions and either one alone leaves a lock.
                //
                // `closing` is the rate of change of the gap, so it is true for
                // two crabs walking the same way when the one behind is the
                // faster -- an overtake. Reversing both of those does not turn
                // them around, it turns them into a head-on pair that is
                // *more* closing than they were, and the pair then oscillates:
                // measured at the current walk speed, two crabs sixteen columns
                // apart reversed on 571 of 600 consecutive frames, so the whole
                // colony was vibrating in place and its drawn column changed
                // half a time a second instead of three. The path length looked
                // right -- each crab covered three cells a second -- which is
                // exactly why this took a trace to find rather than a number.
                //
                // Unconditional reversal has the same failure from the other
                // end: two crabs that are merely *near* each other swap
                // directions every frame they are near, which sends them into
                // each other again immediately, and a pair that met once spent
                // the rest of the run oscillating across the screen.
                //
                // A genuine overtake is not a collision, and it does not get
                // one: nothing here fires, and the pair is left to
                // [`separate_crabs`](Self::separate_crabs), which is the
                // question that actually concerns a same-direction pair --
                // whether the faster one is about to be inside the slower one.
                let relative = self.crabs[i].position.0 - self.crabs[j].position.0;
                let closing = (self.crabs[i].velocity.0 - self.crabs[j].velocity.0)
                    * relative
                    < 0.0;
                let head_on =
                    self.crabs[i].velocity.0 * self.crabs[j].velocity.0 < 0.0;
                if !closing || !head_on {
                    continue;
                }
                let clap = !self.crabs[i].is_special && !self.crabs[j].is_special;

                for crab in [i, j] {
                    if clap {
                        self.crabs[crab].is_special = true;
                        self.crabs[crab].special_timer = self.options.clap_duration;
                    }

                    // The turn-away is a hop, and a crab cannot hop while it is
                    // already in the air. Gating on that is the fix for a crowd:
                    // a crab with a head-on neighbour on each side is reversed
                    // again the frame after it turned, and on a crowded colony it
                    // was doing that on every frame -- 47 reversals a second,
                    // the colony going nowhere. A startled crab is off the
                    // ground for about half a second, so it cannot be startled
                    // again until it lands, and the rate falls to a couple a
                    // second.
                    //
                    // Gated per crab rather than per pair, so the neighbour on
                    // the ground still turns away from the one flying over it,
                    // which reads correctly. Measured: 47 reversals a second
                    // without this, at most 4 with it, across every size and
                    // colony this effect is given.
                    if self.crabs[crab].airborne {
                        continue;
                    }
                    self.crabs[crab].velocity.0 = -self.crabs[crab].velocity.0;
                    self.crabs[crab].direction =
                        if self.crabs[crab].velocity.0 >= 0.0 {
                            Direction::Right
                        } else {
                            Direction::Left
                        };
                    // Both hop. The old code added a random amount to
                    // `velocity.1` and then never damped it, so a collision
                    // launched a crab and left it drifting; a hop has a start and
                    // an end, and gravity is what ends it.
                    self.crabs[crab].startle(&mut self.rng);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SIZE: (u16, u16) = (80, 24);

    fn colony() -> Crab {
        Crab::new(CrabOptions::default(), SIZE)
    }

    #[test]
    fn resize_recomputes_crab_count() {
        let options = CrabOptions {
            crab_count: 10,
            crab_coeff: 1.0,
            ..Default::default()
        };
        let mut crab = Crab::new(options, (80, 40));

        crab.update_size(10, 10);

        assert_eq!(crab.screen_size, (10, 10));
        assert_eq!(crab.options.crab_count, 3);
    }

    #[test]
    fn new_terminates_at_minimum_size() {
        let options = CrabOptions {
            crab_count: 3,
            ..Default::default()
        };

        let _ = Crab::new(options, (6, 6));
    }

    /// The default walk is a scuttle, and the band it sits in is the point.
    ///
    /// 2.1 to 3.9 cells a second at the defaults: `movement_speed` of 3.0, a
    /// gain of 1.0, and a velocity drawn from `0.7..1.3`. That is inside the
    /// 1.5 to 4.5 the effect walked at before any gain existed, and it is
    /// deliberately back inside.
    ///
    /// The gain was 5.0 for one session, which put the colony at 10.5 to 19.5
    /// cells a second, and the person who had asked for *that* then reported the
    /// crabs "change too fast". The two reports are about the same number from
    /// opposite ends, which is why this asserts the whole band rather than a
    /// single ceiling: the fast end of `WALK_SPEED_RANGE` and the slow end both
    /// have to be inside it, so neither end of the personality range can quietly
    /// become a sprint.
    #[test]
    fn the_default_walk_is_a_scuttle_rather_than_a_sprint() {
        let cells_per_second = |velocity: f32| {
            velocity * CrabOptions::default().movement_speed * WALK_GAIN
        };
        let slow = cells_per_second(WALK_SPEED_RANGE.0);
        let fast = cells_per_second(WALK_SPEED_RANGE.1);

        assert!(
            (1.5..=4.5).contains(&slow) && (1.5..=4.5).contains(&fast),
            "the default colony walks at {slow:.1} to {fast:.1} cells a second. \
             It wants the 1.5 to 4.5 band: below it a crab jumps a whole column \
             every third of a second, and above it the body crosses a cell \
             between every two leg positions"
        );
    }

    /// And a running colony really covers that much ground.
    ///
    /// The test above reads constants, which is a proxy; this one reads the
    /// simulation, and it is the one that would notice the constants being
    /// right and the *walk* being wrong -- a crab that was turned around on every
    /// frame would have a plausible-looking velocity and cover no ground at all.
    ///
    /// Measured as total path length rather than displacement, because a crab
    /// that meets a neighbour and turns away is still walking. That is the whole
    /// subtlety: a colony locked into a vibration has a large path length and a
    /// displacement of nothing, and the pair of measurements is what tells those
    /// apart. The bounds are the same 1.5 to 4.5 as above, measured at 2.2 to
    /// 3.7 on the default colony.
    #[test]
    fn a_running_colony_covers_the_ground_its_speed_says() {
        let mut crab = colony();
        for _ in 0..30 {
            crab.step(1.0 / 60.0);
        }

        let frames = 600u16;
        let mut path = vec![0.0f32; crab.crabs.len()];
        for _ in 0..frames {
            let before: Vec<f32> =
                crab.crabs.iter().map(|c| c.position.0).collect();
            crab.step(1.0 / 60.0);
            for (index, crab) in crab.crabs.iter().enumerate() {
                path[index] += (crab.position.0 - before[index]).abs();
            }
        }

        let seconds = f32::from(frames) / 60.0;
        for (index, walked) in path.iter().enumerate() {
            let per_second = walked / seconds;
            assert!(
                (1.5..=4.5).contains(&per_second),
                "crab {index} covered {walked:.1} cells in ten seconds, which is \
                 {per_second:.1} cells a second rather than a scuttle"
            );
        }
    }

    /// The drawn column keeps up with the walk -- and does not run ahead of it.
    ///
    /// Two bounds, because the drawn column and the walk are the same number and
    /// both readings of it have been reported as bugs.
    ///
    /// The lower one: `get_diff` rounds to whole columns, so a crab is a sprite
    /// that jumps a column at a time, and the rate it does that at is its speed
    /// divided by one. A colony whose crabs each change column about three times
    /// a second is walking. A colony whose crabs change column half a time a
    /// second is *vibrating* -- measured, in a version of this effect whose
    /// collision response turned two crabs walking the same way around on every
    /// frame, so the two of them were permanently "closing", permanently
    /// reversing, and never got a column apart. Its velocities looked correct
    /// throughout, which is why this is asserted on the drawn column and not on
    /// the speed.
    ///
    /// The upper one: a crab cannot change column faster than it walks, so this
    /// is the check that fails if the walk is ever turned up again. At the old
    /// gain of 5.0 it measured 12.6 changes a second per crab.
    #[test]
    fn the_drawn_column_keeps_up_with_the_walk() {
        let mut crab = colony();
        for _ in 0..30 {
            crab.step(1.0 / 60.0);
        }

        let mut changes = 0usize;
        let mut previous: Vec<i64> = crab
            .crabs
            .iter()
            .map(|c| c.position.0.round() as i64)
            .collect();
        let frames = 600;
        for _ in 0..frames {
            crab.step(1.0 / 60.0);
            let now: Vec<i64> = crab
                .crabs
                .iter()
                .map(|c| c.position.0.round() as i64)
                .collect();
            // Per crab, not per frame. "Any crab moved" is a different and much
            // easier question, and dividing it by the colony size -- which is
            // what a frame-level count would need -- measures the colony and not
            // the walk.
            for (index, column) in now.iter().enumerate() {
                if *column != previous[index] {
                    changes += 1;
                }
            }
            previous = now;
        }

        let per_crab_second = changes as f64
            / (frames as f64 / 60.0)
            / crab.crabs.len().max(1) as f64;
        assert!(
            per_crab_second > 1.0,
            "a crab changed drawn column {per_crab_second:.1} times a second, so \
             it is standing still and twitching rather than walking; a vibrating \
             colony measured 0.5"
        );
        assert!(
            per_crab_second < 4.5,
            "a crab changed drawn column {per_crab_second:.1} times a second, \
             which is faster than the fastest crab in the colony is meant to walk; \
             the gain of 5.0 measured 12.6"
        );
    }

    /// A pose change and a step of the body are the same event, and neither runs
    /// away from the other.
    ///
    /// `cells_per_pose` is the number of cells the body covers between two leg
    /// positions, and it is the relationship the eye actually judges a scuttle
    /// by: too many and the body slides through the picture while the legs trail
    /// behind it, too few and the legs move in slow motion on a crab that has
    /// stopped. At the defaults it is 0.6 cells.
    ///
    /// Both bounds are one-sided arguments about which failure is worse. The
    /// upper one is the sprite's own half-width: a real crab's stride is about
    /// that, so more than a couple of cells per pose is faster than life rather
    /// than slower. The lower one is a quarter of a cell, which is the point at
    /// which a pose change is no longer accompanied by the body having gone
    /// anywhere at all.
    ///
    /// The pose *rate* is checked here too because it is the other half of the
    /// same knob and it is not derived from the walk: the legs cycle at
    /// `1 / animation_speed` whatever `WALK_GAIN` is. Three poses per second is
    /// a scuttle; below two it is a shuffle, and much above eight the three
    /// distinct pictures blur into a shimmer -- which is what a two-pose cycle at
    /// five hertz used to be.
    #[test]
    fn the_legs_and_the_body_move_at_the_same_time() {
        let options = CrabOptions::default();
        let cells_per_second = (WALK_SPEED_RANGE.0 + WALK_SPEED_RANGE.1)
            * 0.5
            * options.movement_speed
            * WALK_GAIN;
        let cells_per_pose = cells_per_second * options.animation_speed;
        let poses_per_second = 1.0 / options.animation_speed;

        assert!(
            (0.25..3.0).contains(&cells_per_pose),
            "a crab covers {cells_per_pose:.1} cells between poses, so the legs \
             are either trailing a sliding body or pedalling on a crab that is \
             not moving"
        );
        // Capped at 4, not at 8. The walk speed and the leg rate are separate
        // timers, so slowing the body left the legs snapping five times a second
        // and the complaint that came back was "they change too fast" -- which is
        // the legs, not the body. Above about 4 the pose change stops reading as
        // a step and starts reading as a flicker.
        assert!(
            (1.5..=4.0).contains(&poses_per_second),
            "the legs cycle {poses_per_second:.1} times a second, which is not a \
             scuttle"
        );
        assert!(
            cells_per_second < 25.0,
            "a crab covers {cells_per_second:.1} cells a second, which is a \
             sprint rather than a scuttle"
        );
    }

    /// Every frame is the same width, measured in characters, and every row of
    /// every frame is padded to it.
    ///
    /// Two separate defects behind one assertion, and they need different fixes.
    /// The width was measured with `line.len()`, a *byte* count, against a
    /// renderer that walks `line.chars().enumerate()`; that was harmless only
    /// because the one multi-byte glyph, `¬`, sat on a 13-character line whose
    /// byte length was 14 and so matched the other 14-byte lines. And the rows
    /// were ragged -- 11 to 15 characters -- so the sprite had no bounding box
    /// at all, which is what made the edge clamp and the initial spread
    /// disagree with the art.
    #[test]
    fn every_frame_is_a_rectangle_of_one_width_in_characters() {
        let mut widths: HashSet<usize> = HashSet::new();
        for frame in CRAB_FRAMES.iter() {
            assert_eq!(frame.len(), SPRITE_ROWS, "a frame has the wrong height");
            for line in frame {
                widths.insert(line.chars().count());
            }
        }
        assert_eq!(
            widths.len(),
            1,
            "the sprite's rows are not all one width: {widths:?}"
        );
        assert_eq!(
            sprite_width(&RIGHT_POSES[0]),
            widths.iter().next().copied().unwrap(),
            "sprite_width disagrees with the table it measures"
        );
    }

    /// The width is measured in characters, not bytes -- checked on a glyph the
    /// current art does not contain.
    ///
    /// The test above cannot fail on its own: the art happens to be arranged so
    /// that the byte count and the character count agree at the widest row. It
    /// would start failing the moment somebody drew a box-drawing character, at
    /// which point `frame_width` would be four columns too wide and every crab
    /// would bounce a sprite's width short of the left wall while stopping short
    /// of neither wall on the right.
    ///
    /// So this asserts the *function* on art that is deliberately wrong for a
    /// byte count: `¬` is two bytes, `▀` is three, and a row of both is five
    /// characters and eight bytes.
    #[test]
    fn the_sprite_width_is_measured_in_characters_not_bytes() {
        let rows = ["¬▀¬▀", "   ¬"];
        assert_eq!(sprite_width(&rows), 4, "a byte count would be 10");
        assert_eq!(rows[0].len(), 10, "the fixture is not the byte count");
    }

    /// Every left-facing frame is the exact character reversal of its
    /// right-facing counterpart.
    ///
    /// Asserted as a *full-line reversal* rather than as a per-line glyph
    /// multiset, and the difference is the whole point. A multiset cannot see
    /// ordering, so it would have passed on the art that is being fixed here:
    /// the left-facing clap had the same characters as the right-facing one in
    /// the same rows -- its claws and its eyes on the wrong sides, but the same
    /// glyphs on the same lines -- and only its legs were flipped. A multiset
    /// check is exactly the test that would have called that correct.
    ///
    /// Left-facing frames are derived by [`mirror`], so this is really a guard
    /// on the derivation: it fails if someone re-adds hand-drawn art, or if the
    /// art is edited so that a row's content is no longer centred and the
    /// reversal slides the sprite a column sideways.
    #[test]
    fn every_left_facing_frame_is_the_exact_reversal_of_its_counterpart() {
        assert_eq!(CRAB_FRAMES.len(), POSES * 2);
        for (right, left) in (0..POSES).map(|pose| (pose, pose + POSES)) {
            for (row, (a, b)) in CRAB_FRAMES[right]
                .iter()
                .zip(CRAB_FRAMES[left].iter())
                .enumerate()
            {
                let reversed: String = a.chars().rev().collect();
                assert_eq!(
                    reversed, *b,
                    "right-facing pose {right} row {row} reverses to {reversed:?}, \
                     and the left-facing pose is {b:?}"
                );
            }
        }
    }

    /// The walk cycle has more than two poses, and all of them are reachable.
    ///
    /// A two-frame toggle alternated between a planted pose and a lifted one
    /// with nothing between them. At the old 5 Hz that is ten pose changes a
    /// second drawn from two pictures, which reads as a shimmer: the eye sees the
    /// sprite flickering between two shapes rather than a leg moving through
    /// three.
    #[test]
    fn the_walk_cycle_has_more_than_two_poses() {
        let mut crab = colony();
        let mut seen: HashSet<usize> = HashSet::new();
        for entity in &mut crab.crabs {
            entity.is_special = false;
            entity.direction = Direction::Right;
            for frame in 0..WALK_POSES * 3 {
                entity.current_frame = frame;
                seen.insert(entity.frame_index());
            }
        }
        assert!(
            seen.len() > 2,
            "the walk cycle only reaches {} poses: {seen:?}",
            seen.len()
        );
    }

    /// And a running colony actually gets through all of them.
    ///
    /// `the_walk_cycle_has_more_than_two_poses` counts what the frame selector
    /// *can* return; this counts what a real run produces, which is the half
    /// that could be broken by a timer that never fires.
    #[test]
    fn a_running_colony_uses_the_whole_walk_cycle() {
        let mut crab = colony();
        let mut seen: HashSet<String> = HashSet::new();
        for _ in 0..200 {
            crab.step(1.0 / 60.0);
            for entity in &crab.crabs {
                if !entity.is_special {
                    seen.insert(entity.frame_lines().join("\n"));
                }
            }
        }
        assert!(
            seen.len() >= 3,
            "a colony that has run for three seconds only shows {} walk poses, \
             so the extra poses are not reachable",
            seen.len()
        );
    }

    /// The walk poses are actually different pictures.
    ///
    /// A count of distinct indices is a count of distinct *addresses*: it would
    /// pass on three poses that are all the same glyphs with a claw moved, and
    /// the whole point of the extra poses is that the leg is somewhere new.
    /// Compared as whole sprites, this catches a "third pose" that is the second
    /// with a trailing space.
    #[test]
    fn the_walk_poses_differ_by_more_than_one_glyph() {
        let walk: Vec<String> = (0..WALK_POSES)
            .map(|pose| CRAB_FRAMES[pose].join("\n"))
            .collect();
        for (index, a) in walk.iter().enumerate() {
            for b in walk.iter().skip(index + 1) {
                let differing =
                    a.chars().zip(b.chars()).filter(|(x, y)| x != y).count();
                assert!(
                    differing >= 2,
                    "walk poses {index} and one other differ in only {differing} \
                     characters, so the third pose is the second one again"
                );
            }
        }
    }

    /// The clap and the walk rate are separately tunable.
    ///
    /// `animation_speed` used to be the walk interval *and* a multiplier on the
    /// clap duration, so `animation_speed * 5.0` -- a user who wanted a livelier
    /// walk also got a crab that could barely keep its claws open. There is no
    /// way to see that in a config file, because both readings were the same
    /// number.
    #[test]
    fn the_clap_duration_does_not_move_with_the_walk_rate() {
        let slow_walk = CrabOptions {
            animation_speed: 0.05,
            ..Default::default()
        };
        let fast_walk = CrabOptions {
            animation_speed: 0.6,
            ..Default::default()
        };
        assert_eq!(slow_walk.clap_duration, fast_walk.clap_duration);
        assert_eq!(slow_walk.clap_duration, 0.6);
        assert_eq!(
            slow_walk.clap_duration,
            CrabOptions::default().clap_duration,
            "the clap duration should not depend on the walk rate"
        );
    }

    /// Crabs stay on screen, at every size the contract suite drives.
    ///
    /// Two things this pins. A crab's drawn box has to be inside the canvas --
    /// `Canvas::set` drops an out-of-range write rather than failing, so an
    /// off-screen crab is silently half-drawn rather than an error. And the
    /// right-hand clamp has to saturate: the sprite is fifteen columns wide, so
    /// on a six-column terminal `width - frame_width` is negative, and the old
    /// code assigned that to the position, where `-9.0f32.round() as usize`
    /// only survived because the cast saturates.
    #[test]
    fn crabs_stay_inside_the_canvas() {
        for (width, height) in [
            (1u16, 1u16),
            (4, 4),
            (6, 6),
            (15, 5),
            (20, 9),
            (80, 24),
            (200, 50),
        ] {
            let mut crab = Crab::new(CrabOptions::default(), (width, height));
            let (sprite_width, sprite_height) = sprite();
            for _ in 0..240 {
                crab.step(1.0 / 60.0);
                let diff = crab.get_diff();
                for (x, y, _) in diff {
                    assert!(
                        x < width as usize && y < height as usize,
                        "{width}x{height}: a cell at ({x}, {y}) is off screen"
                    );
                }
                for entity in &crab.crabs {
                    let x = entity.position.0.round();
                    let y = entity.position.1.round();
                    assert!(
                        x >= 0.0 && y >= 0.0,
                        "{width}x{height}: a crab is at ({x}, {y})"
                    );
                    assert!(
                        x <= width as f32 && y <= height as f32,
                        "{width}x{height}: a crab is at ({x}, {y}), past the edge"
                    );
                    assert!(
                        x + sprite_width as f32 <= width as f32 + 1.0
                            || width < sprite_width as u16,
                        "{width}x{height}: a crab at {x} overhangs the right edge \
                         by {} columns",
                        x + sprite_width as f32 - width as f32
                    );
                    // Only meaningful when the sprite fits. A four-row crab on a
                    // one-row terminal is clipped, not misplaced, and the
                    // clipping is `Canvas::set` dropping the write.
                    if height >= sprite_height as u16 {
                        assert!(
                            y + sprite_height as f32 <= height as f32,
                            "{width}x{height}: a crab at y={y} is below the screen"
                        );
                    }
                }
            }
        }
    }

    /// No two crabs are ever drawn in the same column.
    ///
    /// The bug, measured rather than argued. There was no runtime separation at
    /// all -- only the spawn-time placement, which is a suggestion rather than a
    /// rule -- and `check_crab_collisions` could not cover for it, because it
    /// acts only on a *closing* pair. Two crabs travelling the same way in the
    /// same column are not closing on each other, so nothing ever separated them
    /// and they stayed stacked for the rest of the run. At 200x50 with the
    /// colony size that terminal actually builds, the closest approach over ten
    /// seconds was 0.03 cells and a pair shared a drawn column on 28 of 39,600
    /// pair-frames; at 80x24 with nine crabs it was 0.0004 and 835 of 21,600.
    ///
    /// Asserted as *zero* now rather than as a rate, because it is a rate no
    /// longer: [`separate_crabs`](Crab::separate_crabs) runs every frame, so the
    /// guarantee is a floor and not a tendency. The old bound allowed one frame
    /// in fifty on the reasoning that "two crabs meeting *is* the collision
    /// response", which was true of the turn-away and false of the pair that had
    /// already met and could not separate.
    ///
    /// The same drawn column rather than a cell, because that is what is on
    /// screen: the sprite is drawn from the crab's rounded column, so two crabs a
    /// tenth of a cell apart are one crab painted twice. The vertical component
    /// is deliberately not compared, since a hop is a real difference on screen.
    ///
    /// The sizes are the ones a terminal gives a real colony, and the one that
    /// is left out is the one where the claim is not true. Below sixteen columns
    /// the sprite is wider than the screen, so `right_edge` is zero and there is
    /// exactly one column a crab may stand in: three crabs on a six-column
    /// terminal are one crab, and no arrangement of them is any other. That is
    /// asserted separately at the end of this test rather than left implicit, so
    /// the exclusion in the loop is a consequence and not a preference.
    #[test]
    fn no_two_crabs_are_ever_drawn_in_the_same_column() {
        for (width, height) in [
            (20u16, 9u16),
            (40, 12),
            (80, 24),
            (120, 40),
            (200, 50),
            (400, 200),
        ] {
            // The colony the runtime would build here, which for a large screen
            // is the densest one this effect is ever given.
            let count =
                ((width as f32 * height as f32) / 800.0).clamp(3.0, 15.0) as u16;
            let mut crab = Crab::new(
                CrabOptions {
                    crab_count: count,
                    ..Default::default()
                },
                (width, height),
            );
            assert!(
                separation(width, crab.crabs.len()) >= 1.0,
                "{width}x{height} is in this list but cannot hold {} crabs a \
                 column apart, so the assertion below would fail for a reason \
                 that has nothing to do with the separation",
                crab.crabs.len()
            );

            for frame in 0..600 {
                crab.step(1.0 / 60.0);
                let columns: Vec<i64> = crab
                    .crabs
                    .iter()
                    .map(|c| c.position.0.round() as i64)
                    .collect();
                let mut unique = HashSet::new();
                for column in &columns {
                    assert!(
                        unique.insert(column),
                        "{width}x{height}, frame {frame}: two crabs are both \
                         drawn in column {column}, which is two crabs painted as \
                         one: {columns:?}"
                    );
                }
            }
        }

        // And the screen that cannot: the sprite is fifteen columns wide, so on
        // anything narrower than that `right_edge` is zero and every crab is
        // clamped to column zero. Fifteen is the last such width and sixteen the
        // first with a seabed at all, one column of it. The claim is unmeetable
        // rather than unmet, and the only honest thing is to say so in the test
        // that states it.
        for (width, height) in [(1u16, 1u16), (6, 6), (15, 5)] {
            let mut crab = Crab::new(CrabOptions::default(), (width, height));
            assert_eq!(
                right_edge(width, sprite().0),
                0.0,
                "{width}x{height} now has a seabed, so the degenerate case has \
                 to move out of this comment and into the loop above"
            );
            for _ in 0..60 {
                crab.step(1.0 / 60.0);
            }
            for entity in &crab.crabs {
                assert_eq!(
                    entity.position.0, 0.0,
                    "{width}x{height}: a crab is at column {} where the only \
                     column is zero",
                    entity.position.0
                );
            }
        }
    }

    /// And no two crabs are ever closer than the separation, which is a stronger
    /// claim than the one above.
    ///
    /// A gap of one cell is the smallest that guarantees distinct drawn columns,
    /// so this is the property that says the two are not an accident of
    /// rounding: on a crowded colony the sweep holds the whole row, and the
    /// measured closest approach comes out at exactly the gap rather than near
    /// it. The gap it is measured against is [`separation`]'s, not
    /// [`MIN_SEPARATION`]'s, because a screen narrower than the colony cannot
    /// give six cells to every pair and asking for six anyway is not a stricter
    /// test, it is an unmeetable one.
    ///
    /// Driven across the sizes and colony sizes where the two differ, because the
    /// small ones are the interesting ones: at 20x9 a crab is fifteen columns of
    /// a twenty-column screen and there is a cell and two thirds of slack between
    /// three of them.
    #[test]
    fn no_two_crabs_are_ever_within_the_separation() {
        for (width, height) in [
            (20u16, 9u16),
            (40, 12),
            (80, 24),
            (120, 40),
            (200, 50),
            (400, 200),
        ] {
            for count in [3u16, 5, 12, 15] {
                let mut crab = Crab::new(
                    CrabOptions {
                        crab_count: count,
                        ..Default::default()
                    },
                    (width, height),
                );
                let wanted = separation(width, crab.crabs.len());

                for frame in 0..300 {
                    crab.step(1.0 / 60.0);
                    for (index, a) in crab.crabs.iter().enumerate() {
                        for (other, b) in
                            crab.crabs.iter().enumerate().skip(index + 1)
                        {
                            let gap = (a.position.0 - b.position.0).abs();
                            // A thousandth of a cell of slack for the arithmetic
                            // in the sweep, which subtracts and adds gaps of
                            // this size across up to fifteen of them.
                            assert!(
                                gap >= wanted - 1.0e-3,
                                "{width}x{height} with {count} crabs, frame \
                                 {frame}: crabs {index} and {other} are {gap:.3} \
                                 cells apart, inside the {wanted:.3} the seabed \
                                 can hold"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The gap the effect asks for is one the seabed can actually pay.
    ///
    /// The single inequality
    /// [`Crab::separate_crabs`] rests on, and the reason it is a test rather
    /// than a comment: it is not checked anywhere in the code, because the sweep
    /// *cannot* check it -- a violation does not fail, it quietly walks a crab
    /// off the left-hand wall, and `crabs_stay_inside_the_canvas` would then be
    /// the thing reporting a separation bug as a rendering one.
    #[test]
    fn the_gap_asked_for_never_exceeds_what_the_seabed_can_hold() {
        for width in [1u16, 6, 15, 20, 40, 80, 120, 200, 400] {
            let span = right_edge(width, sprite().0);
            for count in [2usize, 3, 5, 9, 12, 15, 16] {
                let wanted = separation(width, count);
                assert!(
                    wanted as f64 * (count - 1) as f64 <= span as f64 + 1.0e-3,
                    "{count} crabs on a {width}-column screen ask for {wanted:.3} \
                     cells each, which needs {:.1} of the {span:.1} columns of \
                     seabed they have",
                    wanted as f64 * (count - 1) as f64
                );
                assert!(
                    wanted <= MIN_SEPARATION,
                    "the gap is {wanted:.3} on a {width}-column screen, so a \
                     screen wide enough is not getting the full \
                     {MIN_SEPARATION}"
                );
            }
        }
        // A colony of one has nothing to be separated from, and the old formula
        // divided by `count - 1` -- which is zero there.
        assert_eq!(separation(200, 1), 0.0);
        assert_eq!(separation(200, 0), 0.0);
    }

    /// Catching up to a slower crab is not a collision.
    ///
    /// The head-on half of the collision gate, asserted on the two shapes of
    /// encounter it separates rather than on a rate, because a rate cannot tell
    /// them apart: both of them reverse crabs, and the difference is whether the
    /// pair was ever going to meet.
    ///
    /// `closing` is the rate of change of the *gap*, so a faster crab behind a
    /// slower one is closing on it for as long as it takes. Reversing both of
    /// those does not turn them around -- it points them at each other, which is
    /// more closing than the overtake was, and the pair then oscillates without
    /// ever passing. Measured: two crabs sixteen columns apart, both walking
    /// right, the one behind a fifth faster, reversed on 571 of 600
    /// consecutive frames. Neither of them went anywhere, and every velocity
    /// reading in the effect was correct throughout.
    ///
    /// The other half of the same test is the head-on pair, which *must* turn
    /// around: without that the gate would be satisfied by never reacting at all,
    /// and the turn-away is the whole reason two crabs meeting looks like
    /// something.
    #[test]
    fn a_crab_following_a_slower_one_is_not_a_collision() {
        /// Two crabs, at the given columns with the given horizontal velocities,
        /// and nothing else in play. Both on the sand, both facing with their
        /// velocity, no hop pending, so the assertions are about the turn-away
        /// and not about whether a crab happened to be airborne.
        fn facing(width: u16, first: (f32, f32), second: (f32, f32)) -> Crab {
            let mut colony = Crab::new(CrabOptions::default(), (width, 24));
            colony.crabs.truncate(2);
            for (entity, (x, velocity)) in
                colony.crabs.iter_mut().zip([first, second])
            {
                // The profile under this crab's own column, not one shared row.
                entity.position = (x, colony.seabed.row_at(x));
                entity.velocity = (velocity, 0.0);
                entity.hop_timer = HOP_INTERVAL.1;
                entity.is_special = false;
                entity.special_timer = 0.0;
                entity.direction = if velocity >= 0.0 {
                    Direction::Right
                } else {
                    Direction::Left
                };
            }
            colony
        }

        // Ten columns apart, which is inside the sprite-width reach, and the
        // left crab is walking away from the right one and the right crab is
        // walking left towards it.
        let head_on = facing(80, (30.0, 1.0), (40.0, -1.0));
        let following = facing(80, (30.0, 1.2), (40.0, 1.0));

        for (label, mut crab) in [("head-on", head_on), ("following", following)] {
            for _ in 0..5 {
                crab.step(1.0 / 60.0);
            }
            let heading: Vec<f32> =
                crab.crabs.iter().map(|c| c.velocity.0.signum()).collect();
            if label == "head-on" {
                assert_eq!(
                    heading,
                    vec![-1.0, 1.0],
                    "two crabs walking into each other ten columns apart did not \
                     turn around, so the collision response is not running"
                );
            } else {
                assert_eq!(
                    heading,
                    vec![1.0, 1.0],
                    "a crab that was overtaking a slower one turned around \
                     instead: {heading:?}"
                );
            }
        }
    }

    /// A separation that settles: a second pass over a separated colony moves
    /// nothing.
    ///
    /// The reason a positional correction is worth being careful about. Two
    /// crabs pushing each other along a line can push themselves back and forth
    /// forever, and the standard answer is to damp the correction or give it a
    /// deadzone. A sorted two-pass sweep needs neither, and this is the assertion
    /// for that: the sweep is a projection rather than a force, so it moves each
    /// crab by exactly the overlap and no more, and over a colony that is already
    /// a gap apart it is the identity. If a future change makes it a force
    /// instead -- a relaxation factor, a "nudge rather than a shove" multiplier,
    /// a deadzone that is subtracted rather than avoided -- this is the test that
    /// says it no longer settles.
    ///
    /// Asserted on a colony that is deliberately *not* separated first, so the
    /// test covers the transition as well as the resting state: the first call
    /// has real work to do and the second must do none of it.
    #[test]
    fn separation_settles_rather_than_jittering() {
        let mut crab = Crab::new(
            CrabOptions {
                crab_count: 8,
                ..Default::default()
            },
            (120, 24),
        );
        // All eight in one column, which is the state the report was about.
        for entity in &mut crab.crabs {
            entity.position.0 = 10.0;
        }

        crab.separate_crabs();
        let after_first: Vec<f32> =
            crab.crabs.iter().map(|c| c.position.0).collect();
        let mut columns = after_first.clone();
        columns.sort_by(f32::total_cmp);
        for pair in columns.windows(2) {
            assert!(
                pair[1] - pair[0] >= MIN_SEPARATION - 1.0e-3,
                "the first pass left a gap of {:.3}: {columns:?}",
                pair[1] - pair[0]
            );
        }

        crab.separate_crabs();
        let after_second: Vec<f32> =
            crab.crabs.iter().map(|c| c.position.0).collect();
        for (index, (a, b)) in after_first.iter().zip(&after_second).enumerate() {
            assert_eq!(
                a, b,
                "crab {index} moved again on the second pass, from {a} to {b}, so \
                 the correction is a force and will oscillate"
            );
        }
    }

    /// A crab walks, and does not reverse itself several times a second.
    ///
    /// The symptom of the collision response being wrong, and the reason it
    /// needed a trace to find rather than a number: a crab whose velocity sign
    /// flips every frame has a *plausible* speed, covers three cells a second of
    /// path, and does not go anywhere. Every speed measurement in this file was
    /// happy with it.
    ///
    /// Two of the ways to get here, both real, both measured at the time:
    ///
    /// - A same-direction pair. `closing` is the rate of change of the gap, so
    ///   it is true when the crab behind is the faster one, and reversing *both*
    ///   turns an overtake into a head-on pair that is more closing than the
    ///   overtake was. Measured: 571 reversals in 600 consecutive frames, a
    ///   drawn column changing half a time a second, at five crabs on 80x24.
    /// - A crab in the middle of a chain of three, reversed once by the pair on
    ///   its left and again by the pair on its right in the same frame, so it
    ///   ended the frame going the way it started. Measured: 47 reversals a
    ///   second on a crowded colony, after the first fix.
    ///
    /// The bound is loose on purpose -- a crab legitimately meets a neighbour
    /// every second or so, and a hop it cannot be startled out of is half a
    /// second long -- because the property is that the response is an *event*
    /// rather than a state, and 47 and 571 are not on the same side of that line
    /// as anything measured here.
    #[test]
    fn a_crab_walks_rather_than_vibrating_in_place() {
        for (width, height) in [(80u16, 24u16), (120, 40), (200, 50), (400, 200)] {
            for count in [3u16, 5, 12, 15] {
                let mut crab = Crab::new(
                    CrabOptions {
                        crab_count: count,
                        ..Default::default()
                    },
                    (width, height),
                );
                let mut reversals = vec![0usize; crab.crabs.len()];
                let mut previous: Vec<f32> =
                    crab.crabs.iter().map(|c| c.velocity.0).collect();

                let frames = 600;
                for _ in 0..frames {
                    crab.step(1.0 / 60.0);
                    for (index, entity) in crab.crabs.iter().enumerate() {
                        if entity.velocity.0 * previous[index] < 0.0 {
                            reversals[index] += 1;
                        }
                        previous[index] = entity.velocity.0;
                    }
                }

                let seconds = frames as f64 / 60.0;
                for (index, turns) in reversals.iter().enumerate() {
                    let per_second = *turns as f64 / seconds;
                    assert!(
                        per_second < 10.0,
                        "{width}x{height} with {count} crabs: crab {index} turned \
                         itself around {per_second:.0} times a second, so it is \
                         vibrating rather than walking"
                    );
                }
            }
        }
    }

    /// Every crab is on the sand, and the sand is on the bottom row.
    ///
    /// The staging, and the reason the effect stopped reading as sprites in
    /// space. The old `velocity.1` was drawn from `-0.5..0.5` and never damped
    /// toward anything, so a crab's height was a random walk with no floor and
    /// no reference: nothing said which way was down.
    #[test]
    fn the_colony_walks_on_a_seabed() {
        let mut crab = colony();
        let (_, sprite_height) = sprite();
        let sand_row = SIZE.1 as usize - 1;
        let ground = (sand_row - sprite_height) as f32;
        let _ = ground;

        for _ in 0..300 {
            crab.step(1.0 / 60.0);
        }
        crab.get_diff();

        // The committed frame, not the diff. The sand does not move, so after
        // the first frame the diff never mentions the bottom row again -- which
        // is the point of the seabed and also why asking the diff whether there
        // is a seabed answers "no" on every frame but the first.
        let frame = crab.canvas.on_screen();
        let bottom: HashSet<style::Color> = (0..frame.width)
            .filter(|x| frame.get(*x, sand_row).symbol != ' ')
            .map(|x| frame.get(x, sand_row).color)
            .collect();
        let covered = (0..frame.width)
            .filter(|x| frame.get(*x, sand_row).symbol != ' ')
            .count();
        assert!(
            covered > 8,
            "the bottom row has only {covered} cells on it, so there is no seabed"
        );
        assert!(
            bottom
                .iter()
                .all(|colour| *colour == SAND_COLOUR || *colour == SHADOW_COLOUR),
            "the bottom row is not all sand and shadow: {bottom:?}"
        );

        // And the crabs are on it: airborne for a moment at a time, and never
        // below the ground.
        for entity in &crab.crabs {
            let y = entity.position.1;
            assert!(
                (0.0..=24.0).contains(&y),
                "a crab at y={y} is not on the seabed, whose surface is row {ground}"
            );
        }
    }

    /// A crab's shadow is under it, on the sand, and it is not the crab's own
    /// colour.
    ///
    /// Without the shadow the crabs are sprites on a background; with it they
    /// are standing on something. And a shadow that shrinks as the crab rises
    /// is the only cue that says a hop is a hop -- a shadow of fixed size is a
    /// mark on the ground.
    #[test]
    fn a_crabs_shadow_is_under_it_and_shrinks_as_it_rises() {
        let mut crab = colony();
        for _ in 0..30 {
            crab.step(1.0 / 60.0);
        }

        let diff = crab.get_diff();
        // The sand under each crab, rather than one row for the screen. The
        // seabed slopes now, so "the bottom row" is not where the sand is, and
        // asking for it is how this test came to be checking nothing at all.
        let sand_rows: Vec<usize> = crab
            .crabs
            .iter()
            .map(|c| crab.seabed.sand_at(c.position.0))
            .collect();
        let shadows: Vec<(usize, usize)> = diff
            .iter()
            .filter(|(_, y, cell)| {
                sand_rows.contains(y)
                    && cell.symbol == SHADOW_GLYPH
                    && cell.color == SHADOW_COLOUR
            })
            .map(|(x, y, _)| (*x, *y))
            .collect();
        assert!(
            !shadows.is_empty(),
            "no shadow is on the sand under any of the {} crabs, so they are \
             floating",
            crab.crabs.len()
        );

        // Every shadow sits under a crab, and every crab has one. Compared by
        // column rather than by row because the shadow and the crab are on
        // different rows by construction.
        let crab_columns: Vec<f32> =
            crab.crabs.iter().map(|c| c.position.0.round()).collect();
        for (shadow, _) in &shadows {
            assert!(
                crab_columns
                    .iter()
                    .any(|c| (*shadow as f32 - c).abs() < 16.0),
                "a shadow at column {shadow} is not under any crab: {crab_columns:?}"
            );
        }

        // And the hop shrinks it. A crab held two cells up draws a narrower
        // shadow than the same crab on the ground.
        let (width, _) = sprite();
        let on_ground = shadow_span(width, 0.0);
        let airborne = shadow_span(width, 2.0);
        assert!(
            airborne < on_ground,
            "a shadow is {on_ground} cells wide on the ground and {airborne:.0} \
             two cells up, so the crab's height is not reaching the ground"
        );
    }

    /// The reported bug: "they are only walking at the very bottom; they are not
    /// going up".
    ///
    /// There was no bug. `ground_row` returned `(height - 1) - SPRITE_ROWS` --
    /// one row, for the whole screen -- and every crab's `y` was clamped to
    /// exactly it, so a crab could only leave the sand by hopping. Every piece
    /// of prose in this file called that a *seabed*, which is what made it look
    /// like something was broken rather than something missing.
    ///
    /// This measures the spread of the ground itself rather than the crabs, so
    /// it fails against a flat profile whatever the colony happens to be doing.
    #[test]
    fn the_seabed_goes_up_and_down() {
        let seabed = Seabed::new(&CrabOptions::default(), 24);
        let rows: Vec<usize> = (0..80)
            .map(|x| seabed.row_at(x as f32).round() as usize)
            .collect();
        let (low, high) =
            (*rows.iter().min().unwrap(), *rows.iter().max().unwrap());
        assert!(
            high > low,
            "the seabed is one flat row ({low}) across all 80 columns, so there \
             is nowhere for a crab to go up to"
        );
    }

    /// The crabs walk on the profile rather than beside it.
    ///
    /// The spread test above says the ground varies; this says the animals are
    /// on it. A profile that varies while every crab ignores it is the same
    /// failure wearing a different hat.
    #[test]
    fn the_crabs_walk_on_the_profile_rather_than_beside_it() {
        let mut crab = colony();
        // Spread the colony out first, so the crabs are sampling different parts
        // of the profile rather than standing in one place on it.
        for entity in &mut crab.crabs {
            entity.velocity = (2.5, 0.0);
        }
        for _ in 0..200 {
            crab.step(1.0 / 60.0);
        }

        let seated: Vec<f32> = crab
            .crabs
            .iter()
            .filter(|c| !c.airborne)
            .map(|c| (c.position.1 - crab.seabed.row_at(c.position.0)).abs())
            .collect();
        assert!(
            !seated.is_empty(),
            "every crab was airborne, so there was nothing to measure"
        );
        let worst = seated.iter().cloned().fold(0.0f32, f32::max);
        assert!(
            worst < 0.001,
            "a seated crab is up to {worst:.4} of a row off the ground under it, \
             so the colony is walking beside the seabed rather than on it"
        );
    }

    /// The ground scrolls *with* the colony, so a crab holds its contour.
    ///
    /// The sign of the scroll is load-bearing and was worth measuring rather than
    /// deriving, because backwards is invisible in a still frame and obvious in
    /// motion: the ground slides left while the crabs walk right, so every crab
    /// descends slopes it never climbed, at twice the colony's speed.
    ///
    /// The consequence is measurable, and sharply so. The profile is sampled at
    /// `x + offset`, so a crab walking right at `w` while the offset falls at `w`
    /// is *stationary in profile coordinates*: it holds one contour, and its row
    /// is not merely close to constant, it is exactly constant. With the sign
    /// reversed it travels through the profile at `2w` and its row wanders over
    /// the full relief. A crab is always exactly on the ground under it either
    /// way, so a test asserting *that* cannot tell them apart -- only the drift
    /// can.
    ///
    /// Measured over a window the crab cannot walk out of. A crab reflected off a
    /// wall reverses while the ground keeps scrolling the other way, so it stops
    /// holding a contour for reasons that have nothing to do with the sign; the
    /// first version of this ran for ten seconds on an 80 column screen, hit the
    /// right edge twice, and reported 1.09 rows of drift for that reason alone.
    #[test]
    fn the_ground_scrolls_with_the_colony_rather_than_against_it() {
        let size = (200u16, 24);
        let mut crab = Crab::new(CrabOptions::default(), size);
        crab.crabs.truncate(1);
        // Centred, so the walk has `width/2` of room on either side.
        let centre = f32::from(size.0) * 0.5;
        for entity in &mut crab.crabs {
            entity.position = (centre, crab.seabed.row_at(centre));
            entity.velocity = (1.0, 0.0);
            entity.hop_timer = f32::MAX;
        }

        // Five cells of travel, which is a sixth of the profile's 34 cell period
        // and well short of either wall at 3 cells a second.
        let mut rows = Vec::new();
        for _ in 0..100 {
            crab.step(1.0 / 60.0);
            assert!(
                !crab.crabs[0].airborne,
                "the crab left the ground, so there is no contour to hold"
            );
            rows.push(crab.crabs[0].position.1);
        }
        let swing = rows.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - rows.iter().copied().fold(f32::INFINITY, f32::min);

        assert!(
            swing < 0.01,
            "a crab walking in a straight line changed row by {swing:.4} over five \
             cells of travel, so the seabed is sliding past it rather than \
             travelling with it -- the scroll sign is reversed"
        );
    }

    /// The profile is a slope, not a cliff, and not a ripple.
    ///
    /// Three failures, one test. Relief larger than the sprite is tall reads as a
    /// wall and a crab partway up one looks like it is falling; relief so small
    /// it quantises away is the flat row this replaced. And the sprite is four
    /// rows, so on a screen that cannot hold four rows plus a row of sky the
    /// clamp has to give rather than push the animal off the bottom.
    #[test]
    fn the_profile_is_a_bank_and_not_a_cliff_or_a_ripple() {
        let options = CrabOptions::default();
        let seabed = Seabed::new(&options, 24);
        let rows: Vec<usize> = (0..200)
            .map(|x| seabed.row_at(x as f32).round() as usize)
            .collect();
        let relief = *rows.iter().max().unwrap() - *rows.iter().min().unwrap();
        assert!(
            (2..=SPRITE_ROWS + 1).contains(&relief),
            "the seabed has {relief} rows of relief on a 24 row screen, against \
             2 to {} for something that reads as ground to walk on",
            SPRITE_ROWS + 1
        );

        // A degenerate config falls back rather than producing a wall.
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let options = CrabOptions {
                seabed_amplitude: bad as f32,
                ..Default::default()
            };
            let seabed = Seabed::new(&options, 24);
            let row = seabed.row_at(12.0);
            assert!(
                row.is_finite() && (0.0..=24.0).contains(&row),
                "an amplitude of {bad} put the ground at row {row}"
            );
        }
    }

    /// The sprite's rows are the width `sprite_width` reports.
    ///
    /// The art is hand-written, so a row that is one character short is the kind
    /// of thing that survives a year of editing -- and it is invisible in a
    /// dump, because the missing character is a space at the end of a line.
    #[test]
    fn no_row_is_shorter_than_the_sprite() {
        let (width, height) = sprite();
        for (pose, frame) in CRAB_FRAMES.iter().enumerate() {
            assert_eq!(frame.len(), height, "pose {pose} has the wrong height");
            for (row, line) in frame.iter().enumerate() {
                assert_eq!(
                    line.chars().count(),
                    width,
                    "pose {pose} row {row} is {} characters, not {width}",
                    line.chars().count()
                );
            }
        }
    }

    /// The seabed is a function of the column, so it does not churn.
    ///
    /// A `rand` in the sand line would give a different bottom row every frame,
    /// and the canvas would report all eighty cells of it as changed sixty times
    /// a second -- for a line that is not supposed to move. This is that claim
    /// as a test, and the measurement is the diff.
    #[test]
    fn the_seabed_does_not_churn() {
        let mut crab = colony();
        for _ in 0..60 {
            crab.step(1.0 / 60.0);
        }
        crab.get_diff();

        // Hold the crabs still so only the sand could be repainting, and check
        // that a frame in which nothing moved changes nothing on the bottom row.
        for entity in &mut crab.crabs {
            entity.velocity = (0.0, 0.0);
        }
        let before = crab.get_diff();
        let after = crab.get_diff();
        let sand_row = SIZE.1 as usize - 1;
        let churn: Vec<_> = after
            .iter()
            .filter(|(x, y, cell)| {
                *y == sand_row && !before.contains(&(*x, *y, *cell))
            })
            .collect();
        assert!(
            churn.is_empty(),
            "the sand changed {} cells in a frame where nothing moved",
            churn.len()
        );
    }

    /// Nothing is bold, and every glyph the effect can draw is one cell wide.
    ///
    /// Bold on a truecolor foreground is a brightening hint that many terminals
    /// act on, so two crabs of the same drawn colour came out visibly different.
    /// The width half is the one that shears: a double-width character in a
    /// cell-indexed grid shifts every column after it, and a crab is fifteen
    /// columns wide.
    #[test]
    fn no_cell_is_bold_and_every_glyph_is_one_cell_wide() {
        use crate::render::glyph_ramp;

        let mut crab = colony();
        let diff = crab.get_diff();
        assert!(!diff.is_empty(), "the fixture drew nothing");
        for (x, y, cell) in &diff {
            assert_eq!(
                cell.attr,
                style::Attribute::Reset,
                "cell ({x}, {y}) is bold, which brightens the crab's colour"
            );
        }

        let mut glyphs: HashSet<char> = HashSet::new();
        for frame in CRAB_FRAMES.iter() {
            for line in frame {
                glyphs.extend(line.chars().filter(|c| *c != ' '));
            }
        }
        glyphs.extend(SAND.iter().copied());
        glyphs.insert(SHADOW_GLYPH);
        for glyph in glyphs {
            // Not "is ASCII". `¬` is the sprite's mandible tick and it is
            // deliberately two bytes, because that is what keeps the
            // character-count bug live in the art rather than only in a test
            // fixture. What has to hold is that it occupies one *cell*, and
            // `is_ambiguous_or_narrow` is the predicate for that: a genuinely
            // double-width character in a cell-indexed grid shears every column
            // after it, and a crab is fifteen columns wide.
            assert!(
                glyph_ramp::is_ambiguous_or_narrow(glyph),
                "{glyph:?} (U+{:04X}) could shear a cell-indexed grid",
                glyph as u32
            );
        }
    }

    /// `--print-config` writes every key to disk, so a generated config is
    /// pinned to whatever the defaults were the day it was generated.
    #[test]
    fn the_new_keys_round_trip_through_toml() {
        let options: CrabOptions =
            toml::from_str("clap_duration = 1.25\nanimation_speed = 0.3\n")
                .expect("two keys parse");
        assert_eq!(options.clap_duration, 1.25);
        assert_eq!(options.animation_speed, 0.3);
        assert_eq!(
            options.movement_speed,
            CrabOptions::default().movement_speed,
            "two keys in the section silently reset the others"
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        for key in [
            "animation_speed",
            "clap_duration",
            "clap_chance",
            "movement_speed",
            "crab_coeff",
            "seed",
        ] {
            assert!(
                serialised.contains(key),
                "{key} is missing from the serialised form, so --print-config \
                 would not write it: {serialised}"
            );
        }
    }

    /// `movement_speed` is unchanged, because a test outside this module pins
    /// it and the gain is a separate constant.
    ///
    /// The walk's *rate* has been changed twice and both times it went here
    /// rather than into this default, so a user's config keeps the meaning it
    /// was written with: `movement_speed` still multiplies how fast a crab
    /// walks, and `WALK_GAIN` is a fixed factor on top of it that no config
    /// file can see.
    ///
    /// That is the argument for keeping the pinned 3.0, and it is why the
    /// revert did not go here. At 3.0 with a 1.0 gain the colony walks 2.1 to
    /// 3.9 cells a second, which is the band the effect had before any gain
    /// existed; getting there by dividing `movement_speed` by five would have
    /// moved the number a test in `tests/runtime_and_ascii.rs` deliberately
    /// pins, and would have silently redefined the scale of a key that a user's
    /// config file already contains.
    #[test]
    fn movement_speed_is_unchanged_and_the_gain_carries_the_speed() {
        assert_eq!(CrabOptions::default().movement_speed, 3.0);
        // Not an `assert!`: `WALK_GAIN` is a constant, so the comparison is
        // resolved at compile time and an `assert!` over it is a `true` the
        // optimiser removes, which is the warning clippy is right to raise. A
        // constant that is supposed to be *at most* 1.5 says so by being at
        // most 1.5, and this one is caught as a build failure rather than a
        // test failure, which is the right severity for a value that was
        // wrong once and had to be put back.
        //
        // 1.5 rather than 1.0 because the width of `WALK_SPEED_RANGE` is
        // deliberately a personality range and a gain has to leave room for
        // it: 1.5 * 3.0 * 1.3 is 5.85 cells a second at the fast end, which is
        // already a blur. See `the_default_walk_is_a_scuttle_rather_than_a_
        // sprint` for the band this exists to keep.
        const _: () = assert!(WALK_GAIN <= 1.5);
    }
}
