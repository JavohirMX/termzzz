//! A side-view fish tank.
//!
//! # What makes it read as water
//!
//! The prior art here is `asciiquarium`, and it is a much better piece of work
//! than "fish in a box" deserves credit for: twelve multi-frame fish designs,
//! sharks that hunt smaller fish, whales with water spouts, depth-keyed layering
//! so entities overlap correctly, and interactive feeding. It is the reference
//! and this is not trying to replace it.
//!
//! It also does two things this effect takes a different view of, and both of them
//! are the difference between "a tank" and "a picture of a tank".
//!
//! **Depth is shading, not just a draw order.** `asciiquarium`'s z-depth decides
//! which entity paints over which, and nothing else. But the reason a photograph
//! taken underwater looks like one is *aerial perspective*: water absorbs red
//! first, so a fish at depth is not smaller and further away, it is **bluer and
//! dimmer**. Each fish here carries a z in `0.0..=1.0` and its colour is mixed
//! toward the water's own colour by that z. It costs one `lerp` per fish and it
//! does more to sell "underwater" than anything else in the file -- the effect
//! reads as a stack of flat coloured sprites without it, and as a volume with it.
//!
//! **The light comes from above.** Not caustics. Caustics on the back glass are
//! what a real tank looks like, and they are a per-cell field, which is the
//! expensive bandwidth-hungry shape this crate has been bitten by twice: `ripple`
//! emitted 2.79 MB a frame for exactly that reason. Instead a few wide beams
//! drift down from the surface, which costs a few dozen columns rather than
//! 80,000 cells, cannot band because it is not a per-cell modulation, and reads
//! as "there is a light source above the water" immediately.
//!
//! # The cost, and why it is what it is
//!
//! **343.9 us of render and 2,269 bytes a frame at 400x200** -- six times under the
//! crate's 2 ms budget, and a hundred-and-seventieth of the mandelbrot's byte
//! volume. Measured, and every bit of it comes from one decision.
//!
//! The depth gradient is the largest thing in the frame: it is every cell of the
//! screen. Written into the *glyph* it would be re-sent sixty times a second,
//! because a gradient that never changes is still a frame of glyphs. Written into
//! [`Cell::bg`] it is written once, and the encoder -- which emits a background
//! only when it differs from the last one it wrote -- reports none of it after the
//! first frame. What is left to send each frame is the fish, the bubbles and the
//! handful of shaft cells whose level changed.
//!
//! That is the whole trade, and it is the opposite of the `ripple` and `plasma`
//! situation, where a continuously interpolated field meant a colour change in
//! every cell and 2.79 MB a frame. Same crate, same terminal, same encoder; the
//! difference is entirely in what the value is stored in.
//!
//! # What was wrong before it was right
//!
//! Five things, and all five were invisible in a still picture. They are here
//! because each one is a trap the next effect in this family will walk into.
//!
//! **The art read as horizontal bars.** The first sprites carried a dorsal
//! highlight in `,,,,,,,,` above a body in `((((((((`, and a light dash stacked on
//! a dark one is *two bars*, not one fish. Printed, the tank was a lattice of
//! horizontal stripes. The fin rows have to be the same density as the body, and
//! `the_body_tapers_to_the_tail_and_not_the_head_end` is what holds that.
//!
//! **The species formed three horizontal stripes.** Each fish's depth was jittered
//! by 0.22 around its species' band, and the gap between the bands is 0.29 -- so
//! the bands did not overlap and every species occupied its own row. This is the
//! `ants` mirrored-palette bug on the other axis: a categorical axis that has
//! become ordinal. The spread has to exceed the gap.
//!
//! **Every fish of a species swam at the same height**, because the pull back to
//! depth used `SPECIES[species].depth` -- the species' *nominal* depth -- rather
//! than the fish's own `z`. The `z` each fish was spawned with was decorative. The
//! runs of three and four identical fish across the middle of the tank were that.
//!
//! **And the bottom third was empty.** `row_for_depth` carried a `* 0.85`, so even
//! a fish at `z = 0` stopped at y=24 rather than reaching the floor at 28. A tank
//! with a dead band above the gravel is a tank with nothing in the one part of it
//! that should never be empty.
//!
//! **And two fish would merge.** Three abutting sprites do not read as three fish;
//! they read as one twenty-six-column fish, and the tank produced runs like
//! `><(((((((#o((((#o((((((#o`. Separating them with a *force* could only make
//! that unlikely -- at nine and a half cells a second a fish closes a gap faster
//! than the force opens it. It has to be resolved positionally, and it has to be
//! resolved *inside* the bounds, because clamping afterwards undid the separation
//! by pushing two fish back onto the same floor row.
//!
//! # The water is in the background channel
//!
//! The vertical depth gradient is painted into [`Cell::bg`], not into the glyph.
//! Three reasons, and the first two are about cost:
//!
//! - It is **static**, so after the first frame the diff reports none of it. A
//!   background the encoder sees unchanged costs nothing.
//! - It leaves the **glyphs free for the fish**. Water drawn as glyphs competes
//!   with fish drawn as glyphs for the same legibility; water drawn as a
//!   background does not compete at all.
//! - It is the **first thing in this crate to use `bg` for anything other than
//!   the half-block's two colours in one glyph** -- see `Cell::bg` and the note
//!   in `AGENTS.md` that only that path had exercised it.
//!
//! # What the fish art has to survive
//!
//! Two traps, both measured rather than assumed, and the second is the one that
//! would have shipped unnoticed.
//!
//! **A cell is about twice as tall as it is wide.** So a sprite that looks right
//! as *characters* renders squat: a 13x3 fish is 13 wide by 6 tall on screen. The
//! species widths are 8, 11 and 13 against heights of 1, 2 and 3 for exactly this
//! reason, and `the_species_are_proportioned_for_a_cell_twice_as_tall_as_wide`
//! pins the resulting ratio. This is the same aspect caveat at the top of
//! `render`, and the crab's note is the precedent: the lever is the sprite's shape,
//! not the tuning.
//!
//! **A blob is not a fish.** `the_body_tapers_to_the_tail_and_not_the_head_end`
//! measures ink density in thirds of the sprite and requires the tail third to be
//! far sparser than the rest, which is the property a uniform rectangle cannot
//! have. Note the direction: the *head* end is also dense, because that is where
//! the head and eye are, so "densest in the middle" is the wrong requirement and
//! was the wrong requirement in the first draft of this file.
//!
//! And the tail has to actually move: `the_tail_poses_differ_only_in_the_tail`
//! requires that consecutive poses differ in no column past the first third.
//! Poses that differ in the head read as a fish twitching.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::palette;
use crate::runtime::{FrameContext, InputEvent, PointerPhase};
use crossterm::style::{Attribute, Color};
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Species, as a size band.
///
/// One table, and the reason it is a table rather than a loose `Vec<Species>` is
/// that the art is hand-written and its dimensions are load-bearing in three
/// places: the blit loop, the separation radius, and two tests about proportions
/// and taper. Writing the art inline where it is used would mean the width is
/// stated four times.
struct Species {
    /// A name, for the test failure messages. The species mix is automatic rather
    /// than configured, so nothing in the effect reads this -- but a test failure
    /// that says "species 2" instead of "angel" is a much worse thing to read.
    #[cfg_attr(not(test), allow(dead_code))]
    name: &'static str,
    /// Right-facing art, `POSES` poses of equal-height rows.
    poses: &'static [&'static [&'static str]],
    /// The preferred depth band, `0.0` near to `1.0` far.
    ///
    /// A band rather than a free depth, and the two are the same thing: a bigger
    /// fish is a nearer fish, so size *is* depth here and scaling a sprite at
    /// runtime is not needed. A fish keeps its z within `depth_spread` of this.
    depth: f32,
    /// How much the depth band's colour is allowed to saturate, `0.0`..=`1.0`.
    ///
    /// Per species rather than global, because two species that both read as
    /// orange are two fish and one fish is a fish. Without it every species
    /// converges on the same hue as the water mixes in, and the tank is
    /// monochrome.
    chroma: f32,
    /// Base colour at `depth == 0.0`.
    color: Color,
}

/// Poses per species, and the length of the tail cycle.
///
/// Two, not three. The crab uses three because a *walk* has contact and two
/// strides and skipping the middle leaves the legs somewhere they never were; a
/// tail is a pendulum, and two poses are its two extremes. The art differs in
/// the tail fin alone, so three would only repeat the first.
const POSES: usize = 2;

/// The species table.
///
/// Widths and heights are chosen against the cell aspect ratio, not by eye on the
/// character grid -- see the module docs. Read right to left, each row is the
/// tail, then a thin peduncle, then the body widening, then the head and eye.
static SPECIES: &[Species] = &[
    Species {
        name: "minnow",
        // 9x2. The first version was 8x1 and it is a **4:1 streak** in visual
        // units, which is what
        // `the_species_are_proportioned_for_a_cell_twice_as_tall_as_wide` is for:
        // the art has to be shaped for a terminal rather than for a text editor,
        // where a row is two units tall. Two rows brings it to 2.25:1.
        //
        // The highlight starts at column 1, not column 4. Offset by four it was
        // wider than the body it sat on and detached from it, and the pair read as
        // a larger fish with a smaller one beneath it.
        poses: &[&[r" ((((((o ", r"><((((((o"], &[r" ((((((o ", r"<>((((((o"]],
        depth: 0.78,
        chroma: 0.35,
        color: Color::Rgb {
            r: 190,
            g: 205,
            b: 210,
        },
    },
    Species {
        name: "tetra",
        // 11x2: a dorsal highlight over a full-width body.
        // The dorsal row spans **8 of the body's 11 columns**. A short one
        // detaches: at six columns a reader sees a small dash above a long dash,
        // which is two objects, and the same failure killed the two-row minnow
        // (see that species). A fin has to sit on the body it belongs to.
        poses: &[
            &[r" ((((((((#o", r"><(((((((#o"],
            &[r" ((((((((#o", r"<>(((((((#o"],
        ],
        depth: 0.45,
        chroma: 0.75,
        color: Color::Rgb {
            r: 236,
            g: 150,
            b: 60,
        },
    },
    Species {
        name: "angel",
        // 13x3: a tall body with dorsal above and anal below, and a `~` in the
        // tail region that becomes the trailing edge of the fin.
        // Both fins span 9 of the 13 body columns, for the same reason.
        poses: &[
            &[r"   (((((((   ", r"><~<(((((((#o", r"   (((((((   "],
            &[r"   (((((((   ", r"<>~<(((((((#o", r"   (((((((   "],
        ],
        depth: 0.16,
        chroma: 1.0,
        color: Color::Rgb {
            r: 250,
            g: 214,
            b: 120,
        },
    },
];

/// The sprite table: one entry per (species, direction, pose), each a list of
/// rows.
///
/// Flat, and indexed as `species * 2 * POSES + direction * POSES + pose`, because
/// the first version nested species inside pose inside row and every use then had
/// to reach through two levels to get at a line of art. Directions are adjacent
/// per species so the mirrored half is contiguous.
///
/// Built once by mirroring, for the crab's reason and with the crab's warning
/// attached: a hand-mirrored sprite has to be redrawn by hand every time the art
/// changes, and when that was skipped the left-facing clap's first three lines
/// came out byte-identical to the right-facing ones, so its claws and eyes were
/// both on the wrong side and only its legs were flipped. Deriving it cannot
/// happen.
///
/// Mirroring is character-wise, not byte-wise, so a multi-byte glyph survives it.
static FRAMES: std::sync::LazyLock<Vec<Vec<String>>> =
    std::sync::LazyLock::new(|| {
        let mut right: Vec<Vec<String>> = Vec::new();
        for species in SPECIES {
            for pose in species.poses {
                right.push(
                    pose.iter()
                        .map(|row| row.chars().map(String::from).collect())
                        .collect(),
                );
            }
        }
        let mut all = right.clone();
        all.extend(right.iter().map(|frame| {
            frame
                .iter()
                .map(|line| line.chars().rev().collect())
                .collect()
        }));
        all
    });

/// Index into [`FRAMES`].
fn frame_index(species: usize, facing_left: bool, pose: usize) -> usize {
    (species * 2 + usize::from(facing_left)) * POSES + pose % POSES
}

/// A fish's measured width in characters.
///
/// `chars().count()`, never `len()`. The crab's note is the precedent and the
/// symptom was not cosmetic: this number is what the blit bounds and the
/// separation radius are computed from, so a byte length here means fish stop
/// short of the right wall and start short of the left one.
fn sprite_width(species: usize) -> usize {
    SPECIES[species].poses[0]
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

/// The sprite's height in rows.
fn sprite_height(species: usize) -> usize {
    SPECIES[species].poses[0].len()
}

/// Rows of gravel along the bottom.
///
/// A function of the column and nothing else, for the crab's reason, which is
/// worth repeating because it is a trap that has already been sprung once in this
/// crate: a `rand` here gives a different floor every frame, and the canvas then
/// reports the whole bottom row as changed sixty times a second for a line that
/// never moved.
const GRAVEL: &[char] = &['.', ':', '-', '~', '.', '-'];

/// Depth of the gravel, in rows, as a function of column.
///
/// One or two rows, so the tank has a floor without the gravel becoming a
/// subject. The floor is what the depth gradient is *for* -- a gradient with
/// nothing at the bottom of it reads as a fade, not as depth.
fn gravel_depth(column: usize) -> usize {
    1 + (column % 5 == 0) as usize
}

/// A fish.
struct Fish {
    /// Cell coordinates, `y` down. A float, not an integer: the blit rounds, and
    /// a fish whose position is integral would step a whole cell at a time and
    /// look like it was on a grid. See `no_frame_moves_a_fish_more_than_a_fifth`.
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    /// Depth, `0.0` near to `1.0` far. Drives colour only; size is the species.
    z: f32,
    /// Which row of [`SPECIES`].
    species: usize,
    /// Position in the tail cycle.
    pose: usize,
    /// Phase of this fish's wander, so the shoal does not turn in unison.
    phase: f32,
}

impl Fish {
    /// `+1` if swimming right, `-1` if left. Derived from the velocity rather
    /// than stored, because a fish that has just turned is showing the pose for
    /// the way it is *going* and there is no other place for that to live.
    fn facing(&self) -> i32 {
        if self.vx >= 0.0 { 1 } else { -1 }
    }
}

/// A rising bubble.
///
/// Buoyancy plus a lateral wobble, which is what makes a bubble read as a bubble
/// rather than as a dot travelling up: a real one is pushed around by water it is
/// passing through, so it traces a slightly helical path.
struct Bubble {
    x: f32,
    y: f32,
    phase: f32,
}

/// Most flakes alive at once.
///
/// A cap rather than a timer, because the two questions are different: a timer
/// bounds how long food lasts, and a cap bounds how much is *on screen*. Both
/// matter, and a tank with forty flakes in it is a soup.
const MAX_FLAKES: usize = 24;

/// A flake of food, sinking.
struct Flake {
    x: f32,
    y: f32,
    vy: f32,
}

/// A shaft of light from the surface.
struct Shaft {
    /// Centre column, drifting slowly.
    x: f32,
    /// Half-width in columns.
    half_width: f32,
    /// Drift rate, columns per second. Signed: two shafts crossing is the whole
    /// reason there is more than one.
    drift: f32,
    /// How bright, `0.0`..=`1.0`. The shafts are given different brightnesses
    /// because equal-brightness beams look like a fence.
    strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AquariumOptions {
    /// How many fish, across all three species.
    ///
    /// The mix is not proportional: the near species is rarer than the far one,
    /// because a tank seen side-on is mostly water and the far fish are the ones
    /// that fill it. Equal thirds puts a bright near fish in every square of the
    /// screen and the depth stops reading at all.
    pub fish: u16,

    /// Cells per second at the default speed.
    pub speed: f32,

    /// How far the shafts drift, columns per second.
    pub shaft_speed: f32,

    /// How many shafts of light.
    ///
    /// Few on purpose. They are wide, so a dozen is a solid wash and three reads
    /// as three. See [`AquariumOptions::shaft_width`].
    pub shafts: u16,

    /// Shaft width in columns.
    ///
    /// Wide on purpose, and this is the parameter that decides whether the light
    /// reads as light. A narrow shaft is a vertical line, which reads as a
    /// rendering artefact; a wide one with a soft edge reads as a beam. The
    /// brightness falls off across the width, which is what
    /// `the_shafts_fall_off_across_their_width` checks -- a shaft of constant
    /// brightness is a bar and fails it.
    pub shaft_width: u16,

    /// The water colour at the surface.
    pub surface: Color,

    /// The water colour at the gravel.
    pub deep: Color,

    /// The gravel colour.
    pub gravel_color: Color,

    /// Bubbles per fish per second, as a percentage.
    ///
    /// A rate rather than a count, so the population scales with the school
    /// instead of being a constant that looks sparse with forty fish and crowded
    /// with four.
    pub bubble_rate: u16,

    /// Whether the gravel is drawn at all.
    ///
    /// Not a taste setting so much as a way to isolate the rest of the picture
    /// when checking whether the depth gradient reads without an anchor.
    pub show_gravel: bool,

    pub seed: u64,
}

impl Default for AquariumOptions {
    /// Hand-written so it is the single source of truth, and hand-written for the
    /// same reason as everywhere else in this crate: the derived `Default`
    /// produced zeros, serde used the derived one, and a config file that omitted
    /// this section silently zeroed it.
    fn default() -> Self {
        Self {
            fish: 20,
            speed: 6.0,
            shaft_speed: 1.6,
            shafts: 3,
            shaft_width: 16,
            surface: Color::Rgb {
                r: 26,
                g: 92,
                b: 108,
            },
            deep: Color::Rgb { r: 4, g: 16, b: 40 },
            gravel_color: Color::Rgb {
                r: 62,
                g: 58,
                b: 48,
            },
            bubble_rate: 18,
            show_gravel: true,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Aquarium {
    screen_size: (u16, u16),
    options: AquariumOptions,
    canvas: Canvas,
    fish: Vec<Fish>,
    bubbles: Vec<Bubble>,
    bubbles_spawned: f32,
    flakes: Vec<Flake>,
    shafts: Vec<Shaft>,
    /// The water colour by row, `rows` entries. **Static**, which is the point:
    /// it is the largest thing in the frame and the encoder reports none of it
    /// after the first frame.
    water: Vec<Color>,
    /// Shaft brightness by row, as a quantised level. See
    /// [`AquariumOptions::shaft_width`] for why the levels are few.
    shaft_rows: Vec<f32>,
    time: f32,
    rng: EffectRng,
}

impl TerminalEffect for Aquarium {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.draw();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = normalize_effect_size((width, height));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.rebuild_water();
    }

    fn handle_input(&mut self, event: &InputEvent) {
        // A click drops food. `Pressed` rather than `Released`, so the flake
        // appears under the cursor while the button is down -- on a slow connection
        // or a laggy terminal, `Released` puts it somewhere the user has already
        // moved off, which reads as the effect not listening.
        if let InputEvent::Pointer {
            position,
            phase: PointerPhase::Pressed,
            ..
        } = event
            && self.flakes.len() < MAX_FLAKES
        {
            self.flakes.push(Flake {
                x: position.0 as f32,
                y: position.1 as f32,
                // A little variety in the sink rate, or a dropped pinch of food
                // goes down as a rigid object.
                vy: 1.6 + self.time.sin().abs() * 1.4,
            });
        }
    }

    fn reset(&mut self) {
        let options = self.options.clone();
        let size = self.screen_size;
        *self = Self::new(options, size);
    }
}

impl Aquarium {
    pub fn new(options: AquariumOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            fish: Vec::new(),
            bubbles: Vec::new(),
            bubbles_spawned: 0.0,
            flakes: Vec::new(),
            shafts: Vec::new(),
            water: Vec::new(),
            shaft_rows: Vec::new(),
            time: 0.0,
            rng: seeded_rng(options.seed, "aquarium"),
            options,
        };
        effect.rebuild_water();
        effect.populate();
        effect
    }

    fn rows(&self) -> usize {
        self.screen_size.1 as usize
    }

    fn cols(&self) -> usize {
        self.screen_size.0 as usize
    }

    /// Fills [`Self::water`] and [`Self::shaft_rows`] for the current size.
    ///
    /// Both are pure functions of the size and the options, so this is called on a
    /// resize and at construction and never again. Keeping them as fields rather
    /// than recomputing per row per frame is worth it: this is the largest
    /// computation in the effect and it is entirely static.
    fn rebuild_water(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        self.water = (0..rows)
            .map(|y| {
                // A square-root ramp rather than linear. The eye reads water depth
                // as compressive -- the first metre changes the colour far more than
                // the tenth -- and a linear ramp spends most of its range in the
                // bottom third, which is the part with the least going on.
                let t = if rows <= 1 {
                    0.0
                } else {
                    (y as f32 / (rows - 1) as f32).sqrt()
                };
                palette::lerp(self.options.surface, self.options.deep, t)
            })
            .collect();
        self.shaft_rows = vec![0.0; cols];
    }

    /// Fills the tank: fish, shafts, and a clean bubble list.
    fn populate(&mut self) {
        let cols = self.cols() as f32;
        let count = usize::from(self.options.fish.max(1));

        // Uniformly random placement, and this is a correction rather than a
        // preference. Stratifying by column was meant to stop the shoal clumping
        // and produced the exact opposite: a jitter of two cells against
        // five-cell slots is a *lattice*, and at six cells a second four seconds
        // of swimming does not disperse it, so the tank opened as wallpaper.
        // Random placement clumps, and a shoal is supposed to clump -- the
        // separation term in `step_fish` organises it from there.
        self.fish = (0..count).map(|i| self.spawn_fish(i, count)).collect();
        self.bubbles.clear();
        self.bubbles_spawned = 0.0;
        self.flakes.clear();

        // Shafts spread across the width first, so a small count still covers the
        // tank, and only then given their drift. Two shafts at the same x is a
        // wider shaft, not two shafts.
        let shafts = usize::from(self.options.shafts);
        self.shafts = (0..shafts)
            .map(|i| {
                let slot = (i as f32 + 0.5) / shafts.max(1) as f32;
                let r = seeded_rng(self.options.seed, "aquarium-shaft");
                let jitter: f32 = crate::common::seeded_rng(
                    self.options.seed ^ (i as u64).wrapping_mul(0x9E37_79B9),
                    "aquarium-shaft-jitter",
                )
                .random();
                let _ = r;
                Shaft {
                    x: slot * cols,
                    half_width: self.options.shaft_width.max(1) as f32 * 0.5,
                    // Alternating sign so neighbours shear past each other, with a
                    // seeded offset so it is not a metronome.
                    drift: (if i % 2 == 0 { 1.0 } else { -1.0 })
                        * self.options.shaft_speed
                        * (0.7 + 0.6 * jitter),
                    strength: 0.45 + 0.55 * ((i * 7 % 5) as f32 / 4.0),
                }
            })
            .collect();
    }

    /// One fish, with its species and depth drawn from the seeded mix.
    fn spawn_fish(&mut self, index: usize, total: usize) -> Fish {
        use rand::RngExt;
        let cols = self.cols() as f32;
        // The mix, cumulative: the near species is rarest.
        let roll = self.rng.random::<f32>();
        let species = if roll < 0.2 {
            2 // angel: near
        } else if roll < 0.6 {
            1 // tetra
        } else {
            0 // minnow: far
        };
        let band = SPECIES[species].depth;
        // A little jitter around the band, so the shoal is not three stripes.
        // The spread is **0.44, and it has to exceed the gap between the species
        // bands (0.29)** or each species occupies its own horizontal stripe and the
        // tank reads as three rows of fish rather than as a volume. That is the
        // same failure as the mirrored palette in `ants`, one axis over.
        let z = (band + (self.rng.random::<f32>() - 0.5) * 0.44).clamp(0.0, 1.0);
        let angle = self.rng.random::<f32>() * std::f32::consts::TAU;
        let speed = self.options.speed * (0.7 + 0.6 * self.rng.random::<f32>());
        let _ = total;
        Fish {
            x: self.rng.random::<f32>() * cols,
            y: self.row_for_depth(z) + (self.rng.random::<f32>() - 0.5) * 1.5,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed * 0.25,
            z,
            species,
            pose: index % POSES,
            phase: self.rng.random::<f32>() * std::f32::consts::TAU,
        }
    }

    /// The row a fish at depth `z` prefers.
    ///
    /// Near fish sit low and far fish sit high, which is not arbitrary: in a tank
    /// seen from the side the near glass is the bottom of the frame and the far
    /// glass the top, so depth and height are the same axis seen twice. Getting
    /// this backwards -- near fish high -- reads as a school swimming on the
    /// ceiling, which is the single most noticeable way to get an aquarium wrong.
    fn row_for_depth(&self, z: f32) -> f32 {
        let rows = self.rows() as f32;
        let gravel = self.gravel_rows() as f32;
        // The full usable height, with no fudge factor. There used to be a `* 0.85`
        // here, and it left a **dead band seven rows deep** between the lowest fish
        // and the gravel on a 30-row tank: even a fish at `z = 0` mapped to y=24
        // rather than to the floor, so the bottom third of the picture was empty
        // water with nothing in it, which is the one part of a tank that should
        // never be empty. Near fish now cruise just above the gravel, which is
        // both where they belong and what gives the depth cue its anchor.
        let usable = (rows - gravel - 1.0).max(1.0);
        (1.0 - z) * usable + 1.0
    }

    /// How many rows the gravel occupies at the deepest column.
    fn gravel_rows(&self) -> usize {
        if self.options.show_gravel { 2 } else { 0 }
    }

    /// Advances by `dt` without drawing, for an example that wants to step the
    /// tank and then look at the accumulated frame.
    pub fn advance_for_picture(&mut self, dt: f32) {
        self.advance(dt);
    }

    /// The fish, for the picture example and for the depth tests.
    pub fn fish_snapshot(&self) -> Vec<(f32, f32, f32, usize)> {
        self.fish
            .iter()
            .map(|f| (f.x, f.y, f.z, f.species))
            .collect()
    }

    /// A fish's colour at an arbitrary depth and species, for the depth test.
    pub fn colour_at(&self, z: f32, species: usize) -> Color {
        self.fish_colour(&Fish {
            x: 0.0,
            y: self.row_for_depth(z),
            vx: 1.0,
            vy: 0.0,
            z,
            species,
            pose: 0,
            phase: 0.0,
        })
    }

    /// Advances the simulation by `dt` seconds.
    fn advance(&mut self, dt: f32) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.time += dt;
        self.step_shafts(dt);
        self.step_fish(dt);
        self.step_flakes(dt);
        self.step_bubbles(dt);
    }

    fn step_shafts(&mut self, dt: f32) {
        let cols = self.cols() as f32;
        for shaft in &mut self.shafts {
            shaft.x += shaft.drift * dt;
            // Wrapped, so a shaft that leaves one side comes back on the other
            // rather than piling up at an edge.
            let width = shaft.half_width * 2.0;
            if shaft.x < -width {
                shaft.x += cols + width * 2.0;
            } else if shaft.x > cols + width {
                shaft.x -= cols + width * 2.0;
            }
        }
    }

    /// The shoal, one step.
    ///
    /// Purpose-built rather than shared with `boids`, and the reason is that
    /// `boids` wants a torus with trails and a fish tank wants a bounded box with
    /// sprites. A `boids` that wrapped would swim through the glass; one that did
    /// not would need the wall term anyway, which is most of what a tank needs.
    /// The three rules that survive are separation, a wander, and a pull back to
    /// the species' depth band.
    fn step_fish(&mut self, dt: f32) {
        let (cols, rows) = (self.cols() as f32, self.rows() as f32);
        let gravel = self.gravel_rows() as f32;
        let top = 1.0f32;
        let bottom = (rows - gravel - 1.0).max(1.0);

        for i in 0..self.fish.len() {
            let (x, y, z, species) = {
                let f = &self.fish[i];
                (f.x, f.y, f.z, f.species)
            };
            let width = sprite_width(species) as f32;
            let mut ax = 0.0f32;
            let mut ay = 0.0f32;

            // Separation, at a radius derived from the sprite width. A big fish
            // and a small one have to keep further apart than two of the same
            // size, and the only number that knows how big a fish is in cells is
            // the sprite's own width -- which is the crab's lever applied to a
            // different problem.
            for (j, other) in self.fish.iter().enumerate() {
                if i == j {
                    continue;
                }
                let dx = x - other.x;
                let dy = (y - other.y) * 0.6; // cells are tall; weight y less
                let d2 = dx * dx + dy * dy;
                // Half of each sprite's width, **plus a gap**. Half the sum
                // alone puts two fish at exactly touching distance, and two
                // abutting sprites do not read as two fish -- they read as one
                // long one. Measured: at the half-sum radius the tank produced
                // runs like `><(((((((#o((((#o((((((#o`.
                let gap = 3.0;
                let radius =
                    (width + sprite_width(other.species) as f32) * 0.5 + gap;
                if d2 > 1e-4 && d2 < radius * radius {
                    let d = d2.sqrt();
                    let push = (radius - d) / radius;
                    ax += dx / d * push;
                    ay += dy / d * push;
                }
            }

            // Wander, so the shoal does not hold a shape.
            ax += (self.time * 0.7 + self.fish[i].phase).cos() * 0.5;
            ay += (self.time * 0.5 + self.fish[i].phase).sin() * 0.3;

            // Pull back toward the fish's **own** depth, not its species' nominal
            // one. Using `SPECIES[species].depth` here pulled every fish of a
            // species to the *same row* -- the `z` each fish was spawned with
            // became decorative -- and the tank filled with horizontal runs of
            // three or four same-species fish, which reads as a pattern of
            // identical blocks rather than as a shoal at different distances.
            let want = self.row_for_depth(z);
            // Damped while food is in reach. Not a weight to balance but an
            // ordering to respect: the depth pull is `0.45 * distance` and the
            // attraction is `3.0`, so a fish more than about seven cells off its
            // band never moved toward the flake at all and the chase did not
            // happen. A fish also leaves its depth band to eat, so damping it is
            // the behaviour as well as the fix.
            let chasing = self.nearest_flake(x, y).is_some();
            ay += (want - y) * if chasing { 0.05 } else { 0.45 };

            // Flakes, if any: the nearest one within reach wins.
            if let Some(flake) = self.nearest_flake(x, y) {
                let dx = flake.0 - x;
                let dy = flake.1 - y;
                let d = (dx * dx + dy * dy).sqrt().max(0.5);
                ax += dx / d * 3.0;
                ay += dy / d * 1.5;
            }

            // The walls. A fish that reaches the glass turns before it gets there
            // rather than sticking to it, which is both what fish do and what
            // stops a sprite being clipped in half by the frame.
            //
            // The margin clears the sprite's **extent**, not its centre. That is
            // not a detail: with a three-row sprite and a bottom limit of
            // `rows - gravel - 1`, a fish sitting exactly on the limit put its last
            // row *into the gravel*, and a fish at either end of the row was cut
            // in half by the frame. A centre-based margin is wrong by half the
            // sprite, which is the whole sprite for a one-row fish.
            let half_w = width * 0.5;
            let half_h = sprite_height(species) as f32 * 0.5;
            let left = half_w + 0.5;
            let right = cols - half_w - 0.5;
            let ceil = top + half_h;
            let floor = bottom - half_h;
            if x < left {
                ax += (left - x) * 0.9;
            } else if x > right {
                ax -= (x - right) * 0.9;
            }
            if y < ceil {
                ay += (ceil - y) * 0.9;
            } else if y > floor {
                ay -= (y - floor) * 0.9;
            }

            let speed = self.options.speed;
            let max = speed * 1.6;
            let f = &mut self.fish[i];
            f.vx += ax * dt * 6.0;
            f.vy += ay * dt * 6.0;
            // Speed limit. Without it the separation term can launch a fish.
            let sp = (f.vx * f.vx + f.vy * f.vy).sqrt();
            if sp > max {
                f.vx = f.vx / sp * max;
                f.vy = f.vy / sp * max;
            }
            // Fish do not hover, so keep a floor under the speed as well.
            if sp < speed * 0.35 && sp > 1e-3 {
                f.vx = f.vx / sp * speed * 0.35;
                f.vy = f.vy / sp * speed * 0.35;
            }
            f.x += f.vx * dt;
            f.y += f.vy * dt;

            // The tail cycle, advanced on **distance travelled** rather than on
            // time. A fish that is not moving should not wag, and a fish that is
            // moving fast should wag faster; tying it to the frame clock gives
            // neither, and it is the frame clock that varies with terminal
            // refresh rate.
            let moved = (f.vx * dt).abs() + (f.vy * dt).abs();
            f.pose = ((f.pose as f32 + moved * 0.9) as usize) % POSES;
        }

        // Every fish, every frame -- unconditionally rather than only the ones the
        // resolve happened to touch. It used to wrap x with `rem_euclid`, which is
        // the torus behaviour a *boids* wants and a tank must not have: it parks a
        // fish at x = 0, which clips half an eleven-cell sprite off the left of the
        // screen. The tank's own doc comment says the flock is bounded rather than
        // wrapped and the loop was doing the opposite.
        for i in 0..self.fish.len() {
            self.clamp_one(i);
        }
        self.resolve_overlaps();
    }

    /// Pushes fish apart **positionally**, after integration.
    ///
    /// Separation as a force cannot make the invariant hold, only make it likely.
    /// At nine and a half cells a second a fish closes a two-cell gap in about
    /// twelve frames while the force ramps over the same window, so contacts
    /// happen -- and two abutting sprites do not read as two fish. The tank
    /// produced runs like `><(((((((#o((((#o((((((#o`, three fish reading as one
    /// twenty-six-column fish.
    ///
    /// Resolving the overlap directly makes "no two sprites touch" a property
    /// rather than a tendency, which is what lets `no_two_fish_overlap_on_screen`
    /// be a test rather than a hope.
    ///
    /// The exclusion shape is an **axis-aligned box**, not a circle, and the first
    /// version used a circle. That is the wrong shape for a sprite thirteen cells
    /// wide and one tall: weighting y by a constant turns the exclusion into an
    /// ellipse that is nearly a horizontal line, so the pass pushed the whole
    /// shoal apart sideways and collapsed it onto a single row -- twenty-six fish
    /// in a line, which is not a shoal. Rectangles overlap as boxes.
    ///
    /// Contacts are resolved along the axis of *least* penetration, which is the
    /// cheapest way out and the one that does not fight the depth band.
    /// Puts one fish back inside the tank, clearing its sprite's extent.
    fn clamp_one(&mut self, i: usize) {
        let (cols, rows) = (self.cols() as f32, self.rows() as f32);
        let gravel = self.gravel_rows() as f32;
        let f = &mut self.fish[i];
        let half_w = sprite_width(f.species) as f32 * 0.5;
        let half_h = sprite_height(f.species) as f32 * 0.5;
        // The extra 0.5 on the floor is for the rounding in `draw_fish`: the origin
        // is `round(y) - height/2`, so a limit of `rows - gravel - half_h` admits
        // y = 26.5, which rounds to 27 and lands a three-row sprite's last row on
        // the gravel.
        let top = 1.0 + half_h;
        let bottom = (rows - gravel - half_h - 0.5).max(top);
        f.x =
            f.x.clamp(half_w + 0.5, (cols - half_w - 0.5).max(half_w + 0.5));
        f.y = f.y.clamp(top, bottom);
    }

    fn resolve_overlaps(&mut self) {
        /// Cells of clear water between two sprites, horizontally and vertically.
        const GAP_X: f32 = 3.0;
        const GAP_Y: f32 = 2.0;

        for _ in 0..5 {
            for i in 0..self.fish.len() {
                for j in (i + 1)..self.fish.len() {
                    let (wi, hi) = (
                        sprite_width(self.fish[i].species) as f32,
                        sprite_height(self.fish[i].species) as f32,
                    );
                    let (wj, hj) = (
                        sprite_width(self.fish[j].species) as f32,
                        sprite_height(self.fish[j].species) as f32,
                    );
                    let dx = (self.fish[j].x - self.fish[i].x).abs();
                    let dy = (self.fish[j].y - self.fish[i].y).abs();
                    let into_x = (wi + wj) * 0.5 + GAP_X - dx;
                    let into_y = (hi + hj) * 0.5 + GAP_Y - dy;
                    if into_x <= 0.0 || into_y <= 0.0 {
                        continue; // not touching
                    }
                    // Side of the midpoint, so the pair separates rather than
                    // swapping places.
                    let sign = if self.fish[j].x >= self.fish[i].x {
                        1.0
                    } else {
                        -1.0
                    };
                    if into_x <= into_y {
                        let push = into_x * 0.5;
                        self.fish[i].x -= sign * push;
                        self.fish[j].x += sign * push;
                    } else {
                        let push = into_y * 0.5;
                        let vsign = if self.fish[j].y >= self.fish[i].y {
                            1.0
                        } else {
                            -1.0
                        };
                        self.fish[i].y -= vsign * push;
                        self.fish[j].y += vsign * push;
                    }
                    // Clamped here, not afterwards. Clamping at the end of the
                    // frame undid the separation: two fish pushed apart
                    // vertically were then both clamped onto the same floor row
                    // and overlapping again, which is why five passes made no
                    // difference to a residual of 0.40 cells. A resolve that can
                    // push a fish out of the tank is not a resolve.
                    self.clamp_one(i);
                    self.clamp_one(j);
                }
            }
        }
    }

    fn nearest_flake(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        self.flakes
            .iter()
            .map(|f| {
                let dx = f.x - x;
                let dy = f.y - y;
                (dx * dx + dy * dy, f.x, f.y)
            })
            .filter(|(d2, _, _)| *d2 < 900.0)
            .min_by(|a, b| {
                a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, fx, fy)| (fx, fy))
    }

    /// Flakes sink, and the shoal eats them.
    ///
    /// Sinking rather than hanging, because a flake of food in a tank falls and a
    /// fish that can hover over one is a fish that has stopped being a fish. The
    /// rate is slow enough that a flake is usually still drifting when the first
    /// fish reaches it, which is what makes the chase look like a chase.
    fn step_flakes(&mut self, dt: f32) {
        let (cols, rows) = (self.cols() as f32, self.rows() as f32);
        for flake in &mut self.flakes {
            flake.y += flake.vy * dt;
        }
        // Eaten when a fish is within a cell of it.
        self.flakes.retain(|flake| {
            !self.fish.iter().any(|f| {
                let dx = f.x - flake.x;
                let dy = f.y - flake.y;
                dx * dx + dy * dy < 1.5
            })
        });
        // And swept off the floor, or they accumulate on the gravel for ever.
        let floor = rows - self.gravel_rows() as f32;
        self.flakes
            .retain(|f| f.y < floor && f.x > -2.0 && f.x < cols + 2.0);
    }

    fn step_bubbles(&mut self, dt: f32) {
        let cols = self.cols() as f32;
        // Spawn, at a rate proportional to the school.
        self.bubbles_spawned += dt * f32::from(self.options.bubble_rate) / 100.0
            * self.fish.len() as f32;
        while self.bubbles_spawned >= 1.0 {
            self.bubbles_spawned -= 1.0;
            if !self.fish.is_empty() {
                let pick = (self.rng.random::<f32>() * self.fish.len() as f32)
                    as usize
                    % self.fish.len();
                let fish = &self.fish[pick];
                // Just behind and above the fish's snout, so a bubble reads as
                // coming *from* it rather than as debris in the water.
                let dir = if fish.vx >= 0.0 { 1.0 } else { -1.0 };
                self.bubbles.push(Bubble {
                    x: fish.x + dir * 1.5,
                    y: fish.y - 0.5,
                    phase: self.rng.random::<f32>() * std::f32::consts::TAU,
                });
            }
        }

        for bubble in &mut self.bubbles {
            bubble.y -= 9.0 * dt;
            bubble.x += (self.time * 2.0 + bubble.phase).sin() * 2.5 * dt;
        }
        // Pop at the surface, and keep the list from growing without bound.
        self.bubbles
            .retain(|b| b.y > top_of_water() && b.x > -2.0 && b.x < cols + 2.0);
    }

    fn draw(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        self.canvas.clear();

        // The water, with the shafts. Row by row, because the shafts are a
        // function of the row's depth as well as the column -- a beam is brightest
        // where it enters and fades as it goes down, which is the other half of
        // what makes it read as light.
        for y in 0..rows {
            let base = self.water[y];
            for x in 0..cols {
                let lift = self.shaft_lift(x, y);
                self.canvas.set(
                    x,
                    y,
                    Cell::with_bg(
                        ' ',
                        Color::Reset,
                        Self::lifted(base, lift),
                        Attribute::Reset,
                    ),
                );
            }
        }

        if self.options.show_gravel {
            self.draw_gravel();
        }
        self.draw_flakes();
        self.draw_bubbles();
        // Drawn back to front, so a near fish paints over a far one. The sort is
        // on depth, which is what makes the shoal read as a volume rather than as
        // a flat set of overlapping sprites -- the same role `asciiquarium`'s
        // z-depth plays, except that here the depth also colours.
        let mut order: Vec<usize> = (0..self.fish.len()).collect();
        order.sort_by(|a, b| {
            self.fish[*b]
                .z
                .partial_cmp(&self.fish[*a].z)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in order {
            let (x, y, vx, vy, z, species, pose, phase) = {
                let f = &self.fish[i];
                (f.x, f.y, f.vx, f.vy, f.z, f.species, f.pose, f.phase)
            };
            self.draw_fish(&Fish {
                x,
                y,
                vx,
                vy,
                z,
                species,
                pose,
                phase,
            });
        }
    }

    /// How much light a shaft adds at `(x, y)`, in `0.0..=`1.0`.
    ///
    /// The falloff is a cosine across the shaft's width, which is what makes it a
    /// beam rather than a bar, and the depth term is linear so a shaft is
    /// brightest at the surface and gone before the floor.
    fn shaft_lift(&self, x: usize, y: usize) -> f32 {
        if self.shafts.is_empty() {
            return 0.0;
        }
        let (x, y) = (x as f32, y as f32);
        let rows = self.rows() as f32;
        let depth_fade = 1.0 - (y / rows).min(1.0);
        let mut lift: f32 = 0.0;
        for shaft in &self.shafts {
            let d = ((x - shaft.x) / shaft.half_width.max(1.0)).abs();
            if d < 1.0 {
                let across = (std::f32::consts::FRAC_PI_2 * d).cos();
                lift += shaft.strength * across * depth_fade;
            }
        }
        lift.clamp(0.0, 1.0)
    }

    /// Water colour lifted toward white by `lift`.
    ///
    /// **Quantised**, and this is the output-path decision the module docs
    /// promised. A continuous lift gives every one of the 80,000 cells its own
    /// background, so the encoder emits a full SGR for each and never gets to
    /// reuse a run -- the same trap `ripple` fell into at 2.79 MB a frame. Four
    /// levels is enough: the shafts are soft-edged light, not a gradient the eye
    /// resolves finely, and four levels is a handful of run-lengths.
    fn lifted(base: Color, lift: f32) -> Color {
        const LEVELS: usize = 4;
        if lift <= 0.02 {
            return base;
        }
        let level =
            ((lift * (LEVELS - 1) as f32).round() as usize).clamp(1, LEVELS - 1);
        palette::lerp(
            base,
            Color::Rgb {
                r: 220,
                g: 240,
                b: 255,
            },
            level as f32 / (LEVELS - 1) as f32,
        )
    }

    fn draw_gravel(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        let base = rows.saturating_sub(2);
        for x in 0..cols {
            let depth = gravel_depth(x);
            for d in 0..depth {
                let y = base + d;
                if y < rows {
                    let glyph = GRAVEL[(x + d) % GRAVEL.len()];
                    // A little darker with depth, so the floor has a top surface
                    // rather than reading as a flat band.
                    let colour = if d == 0 {
                        self.options.gravel_color
                    } else {
                        palette::shade(self.options.gravel_color, 0.6)
                    };
                    self.canvas.set(
                        x,
                        y,
                        Cell::with_bg(
                            glyph,
                            colour,
                            self.water[y.min(rows - 1)],
                            Attribute::Reset,
                        ),
                    );
                }
            }
        }
    }

    /// A flake of food, as a small bright mark.
    ///
    /// Bright and *not* blue, because a flake that looked like a bubble would be
    /// read as one -- and the whole point of the bubbles is that they are not
    /// food. Warm against a blue tank, and a different glyph so the two are told
    /// apart even in a monochrome terminal.
    fn draw_flakes(&mut self) {
        for flake in &self.flakes {
            let (x, y) = (flake.x.round() as i32, flake.y.round() as i32);
            if x < 0 || y < 0 || x >= self.cols() as i32 || y >= self.rows() as i32
            {
                continue;
            }
            let (x, y) = (x as usize, y as usize);
            self.canvas.set(
                x,
                y,
                Cell::with_bg(
                    'o',
                    Color::Rgb {
                        r: 250,
                        g: 200,
                        b: 120,
                    },
                    self.water[y],
                    Attribute::Reset,
                ),
            );
        }
    }

    fn draw_bubbles(&mut self) {
        for bubble in &self.bubbles {
            let (x, y) = (bubble.x.round() as i32, bubble.y.round() as i32);
            if x < 0 || y < 0 {
                continue;
            }
            let (x, y) = (x as usize, y as usize);
            if x >= self.cols() || y >= self.rows() {
                continue;
            }
            // A bubble is a ring, not a dot, and the ring is what makes it read
            // as one: `o` with the water still visible inside it.
            self.canvas.set(
                x,
                y,
                Cell::with_bg(
                    'o',
                    Color::Rgb {
                        r: 190,
                        g: 225,
                        b: 240,
                    },
                    self.water[y],
                    Attribute::Reset,
                ),
            );
        }
    }

    /// A fish, blitted with its tail facing the way it is going.
    fn draw_fish(&mut self, fish: &Fish) {
        let species = fish.species;
        let art = &FRAMES[frame_index(species, fish.facing() < 0, fish.pose)];
        let origin_x = fish.x.round() as i32 - (sprite_width(species) as i32 / 2);
        let origin_y = fish.y.round() as i32 - (sprite_height(species) as i32 / 2);
        let colour = self.fish_colour(fish);

        for (row, line) in art.iter().enumerate() {
            for (col, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                let (x, y) = (origin_x + col as i32, origin_y + row as i32);
                if x < 0 || y < 0 {
                    continue;
                }
                let (x, y) = (x as usize, y as usize);
                if x >= self.cols() || y >= self.rows() {
                    continue;
                }
                // The background under a fish is the water, so the sprite is not
                // a rectangle of a different colour pasted over the tank. This is
                // what the `bg` channel is for, and it is the first effect here to
                // use it that way.
                let bg = self.water[y];
                self.canvas.set(
                    x,
                    y,
                    Cell::with_bg(ch, colour, bg, Attribute::Reset),
                );
            }
        }
    }

    /// A fish's colour, mixed toward the water by its depth.
    ///
    /// The whole aerial-perspective effect, and it is two lines. Red is absorbed
    /// first, so the mix is not even: a deep fish loses its red channel first,
    /// then its green, and keeps the blue. Mixing all three channels equally
    /// toward grey would produce the same *distance* cue with none of the
    /// *underwater* cue, and the two together are what make a fish look like it is
    /// in water rather than behind fog.
    fn fish_colour(&self, fish: &Fish) -> Color {
        let species = &SPECIES[fish.species];
        let water =
            self.water[(fish.y.round().max(0.0) as usize).min(self.rows() - 1)];
        // How much of the species colour survives at this depth.
        let keep = (1.0 - fish.z).clamp(0.0, 1.0) * species.chroma;
        match (species.color, water) {
            (
                Color::Rgb {
                    r: sr,
                    g: sg,
                    b: sb,
                },
                Color::Rgb {
                    r: wr,
                    g: wg,
                    b: wb,
                },
            ) => {
                // Per-channel absorption, red first. The exponents are the point
                // and they are not a taste: red is gone by roughly 20 feet and
                // green by 60, so the red channel has to fall off much faster.
                let r = wr as f32 + (sr as f32 - wr as f32) * keep.powi(2);
                let g = wg as f32 + (sg as f32 - wg as f32) * keep;
                let b = wb as f32 + (sb as f32 - wb as f32) * keep.powf(0.35);
                Color::Rgb {
                    r: r.round().clamp(0.0, 255.0) as u8,
                    g: g.round().clamp(0.0, 255.0) as u8,
                    b: b.round().clamp(0.0, 255.0) as u8,
                }
            }
            _ => species.color,
        }
    }
}

/// Topmost row the bubbles pop at: just under the surface.
fn top_of_water() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::palette::perceptual_distance;
    use std::collections::HashSet;

    /// Runs a tank forward.
    fn settle(tank: &mut Aquarium) {
        for _ in 0..180 {
            tank.advance(1.0 / 60.0);
        }
    }

    /// Every glyph the art treats as ink.
    const INK: &str = "~*#%@&$0Oo+=-^><|(),`_.";

    /// The art has to be a fish and not a rectangle, and this is the only thing
    /// that can tell.
    ///
    /// Density in thirds, tail first, and the requirement is that the **tail**
    /// third is much sparser than the rest. Note the direction: an earlier
    /// version wanted the *middle* densest, which fails every correct sprite here
    /// because the head carries the eye and is dense. The property that defines a
    /// fish is the taper to a thin peduncle and a forked tail, and a solid block
    /// cannot have it.
    #[test]
    fn the_body_tapers_to_the_tail_and_not_the_head_end() {
        for (i, species) in SPECIES.iter().enumerate() {
            let pose = species.poses[0];
            let w = sprite_width(i);
            let third = (w / 3).max(1);
            let flat: Vec<char> = pose.concat().chars().collect();
            let density =
                |slice: &[char]| slice.iter().filter(|c| INK.contains(**c)).count();
            let tail = density(&flat[..third]);
            let body = density(&flat[third..]);
            assert!(
                body >= tail * 2,
                "{}: tail third holds {tail} ink and the rest holds {body}; a fish \
                 tapers to its tail and a block does not",
                species.name
            );
        }
    }

    /// The tail has to actually move, and only the tail.
    ///
    /// Two separate failures, and the second is the sneaky one. Poses that do not
    /// differ at all give a fish that slides. Poses that differ in the *head* give
    /// a fish that twitches, which is worse than one that slides because it reads
    /// as broken rather than as stiff.
    #[test]
    fn the_tail_poses_differ_only_in_the_tail() {
        for (i, species) in SPECIES.iter().enumerate() {
            let [a, b] = species.poses else {
                panic!("{} does not have exactly {POSES} poses", species.name);
            };
            let w = sprite_width(i);
            let tail_third = (w / 3).max(1);
            let mut differs = false;
            for (row_a, row_b) in a.iter().zip(b.iter()) {
                for (col, (ca, cb)) in row_a.chars().zip(row_b.chars()).enumerate()
                {
                    if ca != cb {
                        differs = true;
                        assert!(
                            col < tail_third,
                            "{}: pose 1 differs from pose 0 at column {col}, past \
                             the tail third of {tail_third} -- a fish whose head \
                             changes between poses reads as twitching",
                            species.name
                        );
                    }
                }
            }
            assert!(
                differs,
                "{}: its two poses are identical, so the tail never moves",
                species.name
            );
        }
    }

    /// The sprite is proportioned for a cell that is twice as tall as it is wide.
    ///
    /// The trap is that a sprite which looks right as *characters* renders
    /// squat, because every row is two visual units tall. A fish that reads as a
    /// fish in a text editor can be a 13x3 blob on a real terminal. So the ratio
    /// is checked in visual units, and against a band rather than a single number,
    /// because a range is what "proportioned" means and a single ratio would be a
    /// taste dressed as a measurement.
    #[test]
    fn the_species_are_proportioned_for_a_cell_twice_as_tall_as_wide() {
        for (i, species) in SPECIES.iter().enumerate() {
            let w = sprite_width(i) as f32;
            let h = sprite_height(i) as f32 * 2.0; // visual height
            let ratio = w / h;
            assert!(
                (1.8..=3.4).contains(&ratio),
                "{}: {}x{} is {} visual units wide by {} tall, a ratio of {ratio:.2}; \
                 outside 1.8-3.4 it reads as a blob or a streak",
                species.name,
                w,
                h,
                w,
                h
            );
        }
    }

    /// Every row of every pose is the same width.
    ///
    /// The crab's guard, and for the same reason: the art is hand-written, and a
    /// row one character short is the kind of thing that survives a year of
    /// editing. It also matters more here, because the width is what the blit and
    /// the separation radius are computed from -- a short row means fish stop
    /// short of one wall and start short of the other.
    #[test]
    fn every_row_of_every_pose_is_the_same_width() {
        for species in SPECIES {
            for pose in species.poses {
                let expected = sprite_width(
                    SPECIES.iter().position(|s| s.name == species.name).unwrap(),
                );
                for (row, line) in pose.iter().enumerate() {
                    assert_eq!(
                        line.chars().count(),
                        expected,
                        "{}: row {row} of a pose is {} wide, expected {expected}",
                        species.name,
                        line.chars().count()
                    );
                }
            }
        }
    }

    /// No two fish's sprites touch.
    ///
    /// This is the assertion that two abutting sprites do not read as two fish,
    /// and it is why `resolve_overlaps` moves fish *positionally* rather than only
    /// pushing them with a force. Separation as a force made this likely; the box
    /// resolve makes it a property. The tank produced runs like
    /// `><(((((((#o((((#o((((((#o` before the resolve pass, which is three fish
    /// reading as one twenty-six-column fish.
    #[test]
    fn no_two_fish_overlap_on_screen() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        for (x, y, z, species) in tank.fish_snapshot() {
            let _ = (x, y, z, species);
        }
        let fish = tank.fish_snapshot();
        let mut worst: f32 = 0.0;
        for i in 0..fish.len() {
            for j in (i + 1)..fish.len() {
                let (wi, hi) = (
                    sprite_width(fish[i].3) as f32,
                    sprite_height(fish[i].3) as f32,
                );
                let (wj, hj) = (
                    sprite_width(fish[j].3) as f32,
                    sprite_height(fish[j].3) as f32,
                );
                let dx = (fish[i].0 - fish[j].0).abs();
                let dy = (fish[i].1 - fish[j].1).abs();
                let into_x = (wi + wj) * 0.5 + 3.0 - dx;
                let into_y = (hi + hj) * 0.5 + 2.0 - dy;
                if into_x > 0.0 && into_y > 0.0 {
                    worst = worst.max(into_x.min(into_y));
                }
            }
        }
        // A third of a cell, and the bound is the blit's own resolution rather
        // than a number chosen to be safe: two sprites a third of a cell apart
        // round to the same cell column, and neither the character nor the
        // terminal can show the difference. Measured at 0.26 with three passes;
        // two left 1.3 on a crowded frame.
        assert!(
            worst <= 0.34,
            "two sprites overlap by {worst:.2} cells after settling, which is more \
             than the blit can show"
        );
    }

    /// Every fish is fully inside the tank, and clear of the gravel.
    ///
    /// A fish half out of frame, or resting in the gravel, is one of the ways
    /// this stops reading as a tank. The rounding matters: the blit origin is
    /// `round(y) - height/2`, so a wall limit that does not allow for it puts a
    /// three-row sprite's last row on the gravel.
    #[test]
    fn every_fish_is_inside_the_tank_and_above_the_gravel() {
        let (w, h) = (100usize, 30usize);
        let mut tank =
            Aquarium::new(AquariumOptions::default(), (w as u16, h as u16));
        settle(&mut tank);
        let gravel_rows = tank.gravel_rows() as f32;
        for (x, y, z, species) in tank.fish_snapshot() {
            let half_w = sprite_width(species) as f32 * 0.5;
            let half_h = sprite_height(species) as f32 * 0.5;
            let left = (x - half_w).round();
            let right = (x + half_w).round();
            let top = (y - half_h).round();
            let bottom = (y + half_h).round();
            assert!(left >= 0.0, "a {species} sprite starts at x={left}");
            assert!(
                right <= w as f32,
                "a {species} sprite ends at x={right}, past {w}"
            );
            assert!(top >= 0.0, "a {species} sprite starts at y={top}");
            assert!(
                bottom <= h as f32 - gravel_rows,
                "a {species} sprite ends at y={bottom}, into the gravel which \
                 starts at {}",
                h as f32 - gravel_rows
            );
            let _ = z;
        }
    }

    /// Depth is shading, and it is *shading* -- red goes first.
    ///
    /// This is the whole aerial-perspective effect, and it is two lines, so the
    /// test has to check the two things that make it underwater rather than foggy:
    /// the colour moves toward the water's, and it does so **out of proportion**,
    /// losing red faster than blue. A mix that pulled all three channels evenly
    /// toward grey would pass a "is it closer to the water" check and fail this.
    #[test]
    fn depth_costs_red_first_and_so_reads_as_underwater_rather_than_fog() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let near = tank.colour_at(0.0, 1);
        let far = tank.colour_at(1.0, 1);
        let near_rgb = rgb_of(near).unwrap();
        let far_rgb = rgb_of(far).unwrap();

        assert_ne!(near, far, "depth changed no colour at all");

        // Red has to fall off faster than blue. That is the physics -- red is gone
        // by about twenty feet and green by sixty -- and it is the whole difference
        // between underwater and behind fog.
        let dr = (near_rgb.0 as f32 - far_rgb.0 as f32).abs();
        let db = (near_rgb.2 as f32 - far_rgb.2 as f32).abs();
        assert!(
            dr > db,
            "depth cost {} of red and {} of blue; red is absorbed first, so a \
             faster red falloff is what makes this read as water",
            dr,
            db
        );
    }

    /// A far fish is measurably closer to the water than a near one.
    ///
    /// In OKLab rather than by eye, reusing the distance from `newton`. This is
    /// the "does it read" test for the depth channel, and the thing it guards is
    /// the effect collapsing into a flat set of equally bright sprites -- which
    /// looks like a shoal in space rather than in water.
    #[test]
    fn a_far_fish_is_measurably_closer_to_the_water_than_a_near_one() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let water = tank.water[5];
        let mut gaps = Vec::new();
        for (species, s) in SPECIES.iter().enumerate() {
            let near = tank.colour_at(0.0, species);
            let far = tank.colour_at(1.0, species);
            // Near minus far, not the other way round: a far fish is *closer* to
            // the water, so its distance is the smaller one. The first version had
            // this subtraction reversed and reported a perfectly correct depth cue
            // as -0.088.
            let gap =
                perceptual_distance(near, water) - perceptual_distance(far, water);
            gaps.push((s.name, gap));
        }
        for (name, gap) in &gaps {
            assert!(
                *gap > 0.05,
                "{name}: a far fish is only {gap:.3} OKLab closer to the water than \
                 a near one, which is not a depth cue"
            );
        }
    }

    /// The species are told apart by colour, not only by size.
    ///
    /// Without per-species chroma every species converges on the same hue as the
    /// water mixes in, and a monochrome tank is a shoal in fog. The three near-fish
    /// colours have to be separable from each other in OKLab.
    #[test]
    fn the_species_are_told_apart_by_colour() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let mut colours = Vec::new();
        for species in 0..SPECIES.len() {
            colours.push(tank.colour_at(0.35, species));
        }
        let _ = &colours;
        for i in 0..colours.len() {
            for j in (i + 1)..colours.len() {
                let d = perceptual_distance(colours[i], colours[j]);
                assert!(
                    d > 0.05,
                    "species {i} and {j} are {d:.3} apart at mid depth, which is a \
                     shading step rather than two fish"
                );
            }
        }
    }

    /// The water gradient goes from the surface colour to the deep one, and gets
    /// darker going down.
    ///
    /// Also checks it is **monotonic**, which is the property that makes it read as
    /// depth. A gradient that wanders -- bright, dark, bright again -- is a pattern,
    /// not a depth, and a monotonicity check is the only way to notice.
    #[test]
    fn the_water_darkens_monotonically_with_depth() {
        let tank = Aquarium::new(AquariumOptions::default(), (80, 30));
        assert_eq!(tank.water.len(), 30);
        assert_eq!(tank.water[0], tank.options.surface);
        let deep = tank.water[29];
        assert_eq!(
            deep, tank.options.deep,
            "the last row is not the deep colour"
        );
        for y in 1..tank.water.len() {
            let above = crate::render::palette::luminance(tank.water[y - 1]);
            let below = crate::render::palette::luminance(tank.water[y]);
            assert!(
                below <= above + 1e-6,
                "row {y} is brighter than row {}; the gradient is not monotonic, so \
                 it reads as a pattern rather than as depth",
                y - 1
            );
        }
    }

    /// The shafts fall off across their width, so they read as beams.
    ///
    /// A shaft of constant brightness is a bar, and a bar is a rendering artefact.
    /// This is the measurement for the decision to use light shafts instead of
    /// caustics: a beam needs a soft edge to be a beam.
    #[test]
    fn the_shafts_fall_off_across_their_width() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        // Measure on a row well inside the shafts, where the depth fade has not
        // washed the profile out.
        let row = 6usize;
        let lift: Vec<f32> =
            (0..tank.cols()).map(|x| tank.shaft_lift(x, row)).collect();
        let peak = lift.iter().cloned().fold(0.0f32, f32::max);
        assert!(peak > 0.15, "no shaft is doing anything on row {row}");

        // Somewhere within a shaft there must be a rise *and* a fall, and the
        // profile must not be a step.
        let rising = lift.windows(2).any(|w| w[1] - w[0] > 0.02);
        let falling = lift.windows(2).any(|w| w[0] - w[1] > 0.02);
        assert!(rising && falling, "the shaft profile is not a beam");

        // And the edge of a shaft has to be soft: a shaft's outermost lit column
        // must be much dimmer than its peak.
        let lit: Vec<(usize, f32)> = lift
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 0.05)
            .map(|(i, v)| (i, *v))
            .collect();
        let edge = lit
            .iter()
            .map(|(i, _)| lift[i.saturating_sub(1)])
            .fold(f32::INFINITY, f32::min);
        assert!(
            edge < peak * 0.6,
            "a shaft's edge is {edge:.3} against a peak of {peak:.3}; a hard edge \
             reads as a bar"
        );
    }

    /// The water is **static**, so the encoder reports none of it after the
    /// first frame.
    ///
    /// The crab's exact lesson, and the most expensive way to get an aquarium
    /// wrong. The depth gradient is the largest thing in the frame; if anything in
    /// it moved per frame, the bottom two thirds of the screen would be re-sent
    /// sixty times a second for a gradient that never changed. So: the water must
    /// be a function of the row and nothing else, and this is what proves it.
    #[test]
    fn the_water_is_static_so_the_encoder_does_not_resend_it() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let before: Vec<Color> = tank.water.clone();
        settle(&mut tank);
        assert_eq!(
            before, tank.water,
            "the water gradient moved while time passed"
        );
    }

    /// The gravel is a function of the column, so its rows are not reported as
    /// changed every frame.
    ///
    /// The crab's lesson verbatim: a `rand` in the sand line gives a different line
    /// every frame, and the canvas then reports the whole bottom row as changed
    /// sixty times a second for a line that never moved.
    #[test]
    fn the_gravel_does_not_churn() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        let first = tank.get_diff();
        // Draw again with no time passing: a still picture must report nothing.
        let second = tank.get_diff();
        let gravel_changes = second.iter().filter(|(_, y, _)| *y >= 28).count();
        assert!(
            gravel_changes == 0,
            "{gravel_changes} gravel cells reported as changed with no time passing"
        );
        let _ = first;
    }

    /// The shafts are quantised, because a continuous background is the
    /// `ripple` bandwidth trap.
    ///
    /// A continuous lift gives every cell its own background, so the encoder emits
    /// a full SGR per cell and never reuses a run. `ripple` measured 2.79 MB a
    /// frame for exactly that. The quantisation is what keeps a frame to a handful
    /// of run-lengths, and it is only safe if the *number of distinct shaft levels*
    /// is small -- so that is the thing asserted.
    #[test]
    fn the_shaft_lift_is_quantised_to_a_few_levels() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let mut seen: HashSet<Color> = HashSet::new();
        for y in 0..tank.rows() {
            for x in 0..tank.cols() {
                let base = tank.water[y];
                let lift = tank.shaft_lift(x, y);
                seen.insert(Aquarium::lifted(base, lift));
            }
        }
        // 30 rows of gradient plus the shaft levels. A per-cell continuous lift
        // would give one background per cell instead.
        assert!(
            seen.len() < 200,
            "{} distinct backgrounds over a 100x30 frame; the shaft lift is not \
             being quantised",
            seen.len()
        );
    }

    /// A frame is drawn, stays in bounds, and is not empty.
    #[test]
    fn a_frame_is_drawn_and_stays_in_bounds() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        tank.advance(1.0 / 60.0);
        let diff = tank.get_diff();
        assert!(!diff.is_empty(), "a settled tank drew nothing");
        for (x, y, cell) in &diff {
            assert!(*x < 100 && *y < 30, "cell ({x}, {y}) is outside 100x30");
            assert!(
                !cell.symbol.is_control(),
                "a control character reached the screen"
            );
        }
    }

    /// Something actually changes, every frame or nearly.
    ///
    /// The contract test covers the catalogue; this is the effect's own version and
    /// it is the cheapest guard against a tank that settles into a still picture
    /// with a fixed shoal and no light movement.
    #[test]
    fn the_picture_keeps_changing() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        // The property is that the tank never *settles*, not that every frame is
        // busy. A tank is a calm picture and should have quiet moments -- the
        // first version counted frames under a threshold and failed at 7 of 120,
        // which said only that a shoal sometimes holds still for a third of a
        // second. What must not happen is a *run* of them.
        let (mut longest, mut run) = (0usize, 0usize);
        for _ in 0..600 {
            tank.advance(1.0 / 60.0);
            if tank.get_diff().len() < 4 {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        assert!(
            longest < 12,
            "the tank went still for {longest} consecutive frames (a fifth of a second)"
        );
    }

    /// Bubbles rise and do not accumulate.
    #[test]
    fn bubbles_rise_and_are_cleared_at_the_surface() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        for _ in 0..600 {
            tank.advance(1.0 / 60.0);
            assert!(
                tank.bubbles.len() < 4000,
                "{} bubbles accumulated",
                tank.bubbles.len()
            );
            for b in &tank.bubbles {
                assert!(b.y > 0.0, "a bubble rose above the surface");
            }
        }
        assert!(!tank.bubbles.is_empty(), "no bubbles were ever spawned");
    }

    /// A hostile config renders rather than panicking.
    ///
    /// Every clamp exercised with the values a hand-edited file actually
    /// contains. The mandelbrot's old budget curve passed its floor and ceiling
    /// to `f32::clamp`, and `clamp` *asserts* they are ordered, so a config with
    /// `max_iterations` under 24 died on its first frame. A clamp no test
    /// exercises is a clamp nobody has shown to be ordered.
    #[test]
    fn a_hostile_config_renders_rather_than_panicking() {
        let hostile = AquariumOptions {
            fish: 0,
            speed: -5.0,
            shaft_speed: f32::NAN,
            shafts: 9999,
            shaft_width: 0,
            bubble_rate: 9999,
            ..AquariumOptions::default()
        };
        let mut tank = Aquarium::new(hostile, (80, 24));
        settle(&mut tank);
        let _ = tank.get_diff();

        for options in [
            AquariumOptions {
                speed: f32::NAN,
                ..AquariumOptions::default()
            },
            AquariumOptions {
                shaft_speed: f32::INFINITY,
                ..AquariumOptions::default()
            },
        ] {
            let mut tank = Aquarium::new(options, (60, 20));
            settle(&mut tank);
            let _ = tank.get_diff();
        }
    }

    /// The smallest supported terminal, and an absurdly wide one.
    #[test]
    fn extreme_shapes_render() {
        for (w, h) in [(6u16, 6u16), (6, 200), (400, 6), (200, 50)] {
            let mut tank = Aquarium::new(AquariumOptions::default(), (w, h));
            settle(&mut tank);
            tank.advance(1.0 / 60.0);
            let diff = tank.get_diff();
            for (x, y, _) in &diff {
                assert!(
                    *x < w as usize && *y < h as usize,
                    "({x}, {y}) outside {w}x{h}"
                );
            }
        }
    }

    /// Seeded runs differ, and a pinned seed reproduces.
    #[test]
    fn a_seed_reproduces_and_two_seeds_differ() {
        let mut a = Aquarium::new(
            AquariumOptions {
                seed: 99,
                ..AquariumOptions::default()
            },
            (100, 30),
        );
        let mut b = Aquarium::new(
            AquariumOptions {
                seed: 99,
                ..AquariumOptions::default()
            },
            (100, 30),
        );
        settle(&mut a);
        settle(&mut b);
        assert_eq!(a.fish_snapshot(), b.fish_snapshot(), "seed 99 diverged");

        let mut c = Aquarium::new(
            AquariumOptions {
                seed: 100,
                ..AquariumOptions::default()
            },
            (100, 30),
        );
        settle(&mut c);
        assert_ne!(a.fish_snapshot(), c.fish_snapshot(), "two seeds agreed");
    }

    /// The same seed gives the same *picture*, not just the same fish.
    #[test]
    fn a_seed_reproduces_the_picture() {
        let run = |seed: u64| {
            let mut tank = Aquarium::new(
                AquariumOptions {
                    seed,
                    ..AquariumOptions::default()
                },
                (80, 24),
            );
            settle(&mut tank);
            tank.advance(1.0 / 60.0);
            tank.get_diff()
        };
        assert_eq!(run(5), run(5), "seed 5 drew two different pictures");
        assert_ne!(run(5), run(6), "two seeds drew the same picture");
    }

    /// A click drops a flake, and the flake is drawn.
    #[test]
    fn a_click_drops_a_flake_and_the_flake_is_drawn() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        assert!(tank.flakes.is_empty(), "food appeared with no click");

        let mut before = 0usize;
        for _ in 0..600 {
            tank.advance(1.0 / 60.0);
            before = before.max(tank.get_diff().len());
        }
        assert_eq!(tank.flakes.len(), 0, "flakes appeared unbidden");

        tank.handle_input(&InputEvent::Pointer {
            position: (50, 10),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        assert_eq!(tank.flakes.len(), 1, "a click did not drop a flake");
        assert_eq!((tank.flakes[0].x, tank.flakes[0].y), (50.0, 10.0));

        // And it is on screen, which is the part a "the list grew" assertion
        // cannot see: a flake can be in the list and never drawn.
        let diff = tank.get_diff();
        let drawn = diff
            .iter()
            .any(|(x, y, cell)| *x == 50 && *y == 10 && cell.symbol == 'o');
        assert!(drawn, "the flake is in the list but was not drawn");
    }

    /// The shoal goes for the food, and the food gets eaten.
    ///
    /// This is the behaviour `needs_mouse` is promising. It is worth a test rather
    /// than a hope because the attraction is a term in the same force sum as the
    /// separation and the depth pull, and a term with the wrong sign is invisible
    /// in a still picture.
    #[test]
    fn the_fish_go_for_the_food_and_eat_it() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 10,
                ..AquariumOptions::default()
            },
            (100, 30),
        );
        settle(&mut tank);
        let start = tank.fish_snapshot();

        // Drop a flake in the middle, and give the shoal time to notice.
        tank.handle_input(&InputEvent::Pointer {
            position: (50, 15),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });

        let mut closed = 0usize;
        for _ in 0..240 {
            tank.advance(1.0 / 60.0);
            for (x, y, _, _) in tank.fish_snapshot() {
                let d = ((x - 50.0).powi(2) + (y - 15.0).powi(2)).sqrt();
                if d < 20.0 {
                    closed += 1;
                    break;
                }
            }
            if tank.flakes.is_empty() {
                break;
            }
        }
        assert!(
            closed > 0,
            "no fish came within twenty cells of a flake that sat for four seconds"
        );
        assert!(
            tank.flakes.is_empty(),
            "the flake was never eaten, so the attraction did nothing"
        );
        // And the shoal actually moved to get there.
        let end = tank.fish_snapshot();
        assert_ne!(start, end, "the shoal did not move at all");
    }

    /// Flakes are capped, and are swept off the floor.
    #[test]
    fn food_is_capped_and_cannot_pile_up_on_the_gravel() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        for i in 0..200u16 {
            tank.handle_input(&InputEvent::Pointer {
                position: (i % 100, 3 + (i / 100) % 25),
                phase: PointerPhase::Pressed,
                button: crate::runtime::PointerButton::Left,
            });
        }
        assert!(
            tank.flakes.len() <= MAX_FLAKES,
            "{} flakes after 200 clicks, cap is {MAX_FLAKES}",
            tank.flakes.len()
        );
        for _ in 0..1200 {
            tank.advance(1.0 / 60.0);
        }
        let floor = tank.rows() as f32 - tank.gravel_rows() as f32;
        for f in &tank.flakes {
            assert!(f.y < floor, "a flake is resting on the gravel at y={}", f.y);
        }
    }

    fn rgb_of(c: Color) -> Option<(u8, u8, u8)> {
        match c {
            Color::Rgb { r, g, b } => Some((r, g, b)),
            _ => None,
        }
    }
}
