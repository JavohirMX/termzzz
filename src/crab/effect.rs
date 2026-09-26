use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
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
/// The old effect moved a crab 1.5 to 4.5 cells a second -- `movement_speed`
/// times a velocity drawn from `-1.5..1.5` -- and `get_diff` rounds to whole
/// cells, so the drawn position changed once every 14 to 40 frames. Measured at
/// 80x24 with the default colony, it was once every 21. That is a sprite
/// visibly teleporting one column at a time, and it is the single largest thing
/// wrong with the effect.
///
/// There is no way to interpolate on a cell grid, so the fix is a scuttle made
/// of more, smaller steps rather than fewer big ones. 5.0 puts the default
/// colony at 10.5 to 19.5 cells a second -- a cell every three to six frames --
/// which is continuous to the eye. Past about 25 the sprite outruns its own
/// animation and the legs appear to slide rather than step, so the top of that
/// range is where this stops.
const WALK_GAIN: f32 = 5.0;

/// The horizontal speed a crab is drawn from, as a fraction of a cell per
/// second.
///
/// Narrower than the old `-1.5..1.5`. A crab twice as fast as its neighbours
/// looks like it is trying to escape, and at `WALK_GAIN` the fast end of a wide
/// range is a sprint; the width is a personality range instead.
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
    pub animation_speed: f32,

    /// How long a clap lasts, in seconds.
    ///
    /// Independent of [`animation_speed`](Self::animation_speed) so the two can
    /// be tuned separately. 0.6 is a quarter of a second at a 0.2 second walk
    /// interval, which is long enough to see the claws open and short enough
    /// that a colony of clapping crabs does not look like a colony of statues.
    pub clap_duration: f32,

    pub clap_chance: f32, // Random chance for special animation

    pub movement_speed: f32,

    pub crab_coeff: f32,

    /// Seed for the initial colony and every movement, turn and clap after it.
    pub seed: u64,
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
            animation_speed: 0.2,
            clap_duration: 0.6,
            clap_chance: 0.05,
            movement_speed: 3.0,
            crab_coeff: 1.0,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Crab {
    pub screen_size: (u16, u16),
    options: CrabOptions,
    canvas: Canvas,
    crabs: Vec<CrabEntity>,
    rng: EffectRng,
    frame_timer: f32,
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
        animation_speed: f32,
        clap_duration: f32,
        movement_speed: f32,
        clap_chance: f32,
        rng: &mut EffectRng,
    ) {
        let (width, height) = sprite();
        let sand_row = screen_size.1.saturating_sub(1);
        // The seabed line itself takes the bottom row, so a grounded crab's
        // feet are on the row above it. That is what leaves room for the
        // shadow: a crab whose feet are *on* the sand has nowhere to put one.
        let ground = (sand_row.saturating_sub(height as u16)) as f32;

        // Gravity first, so a hop that ends this frame still lands.
        if self.position.1 < ground {
            self.velocity.1 += GRAVITY * dt;
        }

        self.position.0 += self.velocity.0 * movement_speed * WALK_GAIN * dt;
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
    }
}

/// The rightmost column a crab's sprite may start at.
fn right_edge(screen_width: u16, sprite_width: usize) -> f32 {
    (screen_width as f32 - sprite_width as f32).max(0.0)
}

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

        let (width, height) = sprite();
        let sand_row = self.canvas.height().saturating_sub(1);
        let ground = (sand_row.saturating_sub(height)) as f32;

        // The seabed, then the shadows, then the crabs. A crab's feet are on the
        // row above the sand, so the shadow has a row of its own and the two
        // never contend for a cell.
        for x in 0..self.canvas.width() {
            self.canvas.set(
                x,
                sand_row,
                Cell::new(
                    SAND[x % SAND.len()],
                    SAND_COLOUR,
                    style::Attribute::Reset,
                ),
            );
        }

        for crab in &self.crabs {
            let base_x = crab.position.0.round().max(0.0) as usize;
            let base_y = crab.position.1.round().max(0.0) as usize;

            // The shadow: a run of underscores on the sand, narrowing as the
            // crab rises. A shadow that stays the same size at every height is
            // a mark on the ground rather than a shadow, and it is the one cue
            // that says the crab is *above* the sand rather than printed on it.
            let altitude = (ground - crab.position.1).max(0.0);
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

        for crab in &mut self.crabs {
            crab.update(
                dt as f32,
                self.screen_size,
                self.options.animation_speed,
                self.options.clap_duration,
                self.options.movement_speed,
                self.options.clap_chance,
                &mut self.rng,
            );
        }

        self.check_crab_collisions();
    }

    pub fn new(options: CrabOptions, screen_size: (u16, u16)) -> Self {
        // One generator for the whole colony, drawn from sequentially, so the
        // crabs diverge from each other the way separate draws would.
        let mut rng = seeded_rng(options.seed, "crab");
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        let (sprite_width, sprite_height) = sprite();

        // Every crab starts on the sand. The old code scattered them anywhere
        // in the frame, which is what made the effect read as a shoal of
        // sprites drifting in space rather than as animals on a seabed.
        let sand_row = screen_size.1.saturating_sub(1);
        let ground = (sand_row.saturating_sub(sprite_height as u16)) as f32;

        // Spread the colony along the sand, then let it walk.
        let max_x = right_edge(screen_size.0, sprite_width);
        let columns = Self::spread(&mut rng, max_x, options.crab_count as usize);

        let mut crabs = Vec::with_capacity(options.crab_count as usize);
        for column in columns {
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

        Self {
            screen_size,
            options,
            canvas,
            crabs,
            rng,
            frame_timer: 0.0,
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

    // Check for collisions between crabs and handle them
    fn check_crab_collisions(&mut self) {
        let crab_count = self.crabs.len();
        if crab_count < 2 {
            return;
        }

        // Simple collision detection based on proximity.
        //
        // The turn-away and the clap are gated separately, and that split is the
        // fix. Gating the whole response on "neither crab is already clapping"
        // stopped the re-clapping -- without it, two crabs re-triggered each
        // other every frame, which resets `special_timer` before it can run out,
        // so a pair that met once stayed open-clawed for as long as they were
        // neighbours and the clap became a state rather than an event. But it
        // also stopped the *turn-away*, and that is the part which is not
        // cosmetic: a crab whose neighbour happened to be clapping walked
        // straight through it, and the colony measured a third of all pairs
        // occupying the same cell.
        //
        // So: always reverse, always hop, and clap only if neither is already
        // clapping. The clap's own duration is the cooldown.
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
                // Only if they are closing. Unconditional reversal makes two
                // crabs that are merely *near* each other swap directions every
                // frame they are near, which sends them into each other again
                // immediately -- a pair that met once spent the rest of the run
                // oscillating across the screen and 27% of all pairs were inside
                // a sprite's width. Reversing only a closing pair separates them
                // once and lets them travel.
                let relative = self.crabs[i].position.0 - self.crabs[j].position.0;
                let closing = (self.crabs[i].velocity.0 - self.crabs[j].velocity.0)
                    * relative
                    < 0.0;
                if !closing {
                    continue;
                }
                let clap = !self.crabs[i].is_special && !self.crabs[j].is_special;

                for crab in [i, j] {
                    if clap {
                        self.crabs[crab].is_special = true;
                        self.crabs[crab].special_timer = self.options.clap_duration;
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

    /// The drawn column must change far more often than once every fourteen
    /// frames.
    ///
    /// The bug, measured rather than argued. The old effect moved a crab 1.5 to
    /// 4.5 cells a second and `get_diff` rounds to whole cells, so the drawn
    /// position changed once every 14 to 40 frames -- and at 80x24 with the
    /// default colony it measured once every 21. A sprite that moves one column
    /// every third of a second is not walking, it is teleporting, and no amount
    /// of extra animation frames hides that: the *body* has to move too.
    ///
    /// Asserted as a rate over a run rather than as a minimum step size, because
    /// a rate is the thing that was wrong. A crab can be momentarily still --
    /// bouncing off a wall, or mid-hop with its column unchanged -- so the
    /// assertion is about the average, and the threshold is a quarter of frames,
    /// i.e. one column per four.
    #[test]
    fn the_drawn_column_moves_far_often_often() {
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

        let per_crab_frame =
            changes as f64 / (frames * crab.crabs.len().max(1)) as f64;
        // A fifth of frames, i.e. a cell every five. Measured at 19.3% on the
        // default colony, against 4.7% for the old effect -- so this is a 4.1x
        // improvement and not a marginal one. The threshold sits below the
        // measurement so the test is about the order of magnitude rather than
        // about a particular crab's luck at a wall.
        assert!(
            per_crab_frame > 0.15,
            "a crab changed column on only {:.1}% of frames, so it moves one \
             cell every {:.0} frames; the old effect managed one every 21",
            per_crab_frame * 100.0,
            1.0 / per_crab_frame.max(f64::MIN_POSITIVE)
        );
    }

    /// And not so fast that the sprite outruns its own legs.
    ///
    /// The other end of the same knob, and it is a real one: a crab covering
    /// three cells between two pose changes reads as sliding rather than
    /// stepping, and the fix for the teleporting bug would have introduced it.
    #[test]
    fn the_crab_does_not_outrun_its_own_walk_cycle() {
        let options = CrabOptions::default();
        let cells_per_second = (WALK_SPEED_RANGE.0 + WALK_SPEED_RANGE.1)
            * 0.5
            * options.movement_speed
            * WALK_GAIN;
        let cells_per_pose = cells_per_second * options.animation_speed;

        // Four cells per pose, against a sprite fifteen columns wide. The
        // natural bound is the sprite's own half-width: a real crab's stride is
        // about that, so anything under about seven is slower than life rather
        // than faster. Below two the legs would be moving faster than the body,
        // which is the other way to look wrong.
        assert!(
            (2.0..4.0).contains(&cells_per_pose),
            "a crab covers {cells_per_pose:.1} cells between poses, so the legs \
             are either faster than the body or too slow to read as steps"
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

    /// No two crabs share a drawn cell, and none of them walks off the seabed.
    ///
    /// "Pathologically" needs a number. The colony is placed with a whole
    /// sprite's width between neighbours where the terminal allows it, which is
    /// 17 columns; the crabs then walk freely and the collision response turns
    /// them around, so the honest bound is the sprite's own width: two crabs
    /// closer than that are on top of each other, and two crabs at the same
    /// drawn cell are certainly on top of each other.
    #[test]
    fn crabs_do_not_stack_on_each_other() {
        let mut crab = colony();
        let (sprite_width, _) = sprite();

        let mut stacked = 0usize;
        let mut overlapping = 0usize;
        let frames = 600;
        for _ in 0..frames {
            crab.step(1.0 / 60.0);
            let cells: Vec<(i64, i64)> = crab
                .crabs
                .iter()
                .map(|c| (c.position.0.round() as i64, c.position.1.round() as i64))
                .collect();
            let mut unique = HashSet::new();
            for cell in &cells {
                if !unique.insert(*cell) {
                    stacked += 1;
                }
            }
            for (index, a) in cells.iter().enumerate() {
                for b in cells.iter().skip(index + 1) {
                    if (a.0 - b.0).unsigned_abs() < sprite_width as u64 {
                        overlapping += 1;
                    }
                }
            }
        }

        // Not zero. Two crabs meeting *is* the collision response, and they are
        // level for the frame before they turn around; asserting they are never
        // level would be asserting that the collision code never runs. Under one
        // frame in fifty is a collision rather than a pile-up.
        assert!(
            stacked * 50 < frames,
            "two crabs shared a cell on {stacked} of {frames} frames, which is \
             a pile-up rather than a collision"
        );
        // And not usually overlapping either. The turn-away is at a sprite's
        // width, so a pair is inside that only for the frame or two it takes to
        // react: at 0.25 cells a frame that is a quarter of a cell of overshoot.
        // A tenth of frames is the bound, against the old constant's third --
        // which is what "a colony where a third of all pairs overlap" measured.
        let pairs = frames * crab.crabs.len() * (crab.crabs.len() - 1) / 2;
        assert!(
            overlapping * 10 < pairs,
            "two crabs were within a sprite's width on {overlapping} of {pairs} \
             frames, so the colony is a heap rather than a shoal"
        );
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
        let ground = sand_row - sprite_height;

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
                y >= 0.0 && y <= ground as f32,
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
        let sand_row = SIZE.1 as usize - 1;
        let shadows: Vec<usize> = diff
            .iter()
            .filter(|(_, y, cell)| {
                *y == sand_row
                    && cell.symbol == SHADOW_GLYPH
                    && cell.color == SHADOW_COLOUR
            })
            .map(|(x, _, _)| *x)
            .collect();
        assert!(
            !shadows.is_empty(),
            "no shadow is on the sand, so the crabs are floating"
        );

        // Every shadow sits under a crab, and every crab has one. Compared by
        // column rather than by row because the shadow and the crab are on
        // different rows by construction.
        let crab_columns: Vec<f32> =
            crab.crabs.iter().map(|c| c.position.0.round()).collect();
        for shadow in &shadows {
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
    /// The walk is faster than it was, and it had to be: at 3.0 the old effect
    /// moved a crab 1.5 to 4.5 cells a second, which is one drawn column every
    /// 14 to 40 frames. `WALK_GAIN` is where the change went instead of into
    /// this default, so `movement_speed` keeps the meaning a user's config gave
    /// it -- it still scales how fast a crab walks -- and the pinned value still
    /// holds.
    #[test]
    fn movement_speed_is_unchanged_and_the_gain_carries_the_speed() {
        assert_eq!(CrabOptions::default().movement_speed, 3.0);
        // Not an `assert!`: `WALK_GAIN` is a constant, so the comparison is
        // resolved at compile time and an `assert!` over it is a `true` the
        // optimiser removes, which is the warning clippy is right to raise. A
        // constant that is supposed to be above 1.0 says so by being above 1.0.
        const _: () = assert!(WALK_GAIN > 1.0);
    }
}
