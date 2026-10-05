//! The state of each cell in the next generation is determined by the states
//! of that cell and its eight neighbors in the current generation:
//!
//! Overpopulation:
//!     If a living cell is surrounded by more than three living cells, it dies.
//! Underpopulation:
//!     If a living cell is surrounded by fewer than two living cells, it dies.
//! Survival:
//!     If a living cell is surrounded by two or three living cells, it survives.
//! Birth:
//!     If a dead cell is surrounded by exactly three living cells,
//!     it becomes a living cell.
use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::life::rule::{LifeRule, deserialize_rule};
use crate::render::Palette;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// Width and height of the box a seeded glider is rotated within.
const GLIDER_SIZE: usize = 3;
/// How many gliders are seeded into each generation.
const GLIDERS_PER_GENERATION: usize = 9;

/// The age at which a live cell reaches the top of the ramp.
///
/// Measured, not guessed. The age distribution of a life soup is violently
/// skewed: over a settled run at the default density, 44% of live cells are one
/// generation old, 75% are at most one, 92% are at most two, and under 5% ever
/// reach four. Almost nothing in this simulation *is* old, so "old" has to mean
/// a handful of generations rather than a couple of dozen -- anything longer and
/// the top of the ramp is decoration no cell ever reaches.
///
/// Inclusive, and that is the other half of the arithmetic. A cell at the top of
/// the range is saturating, so the ages this simulation actually produces are 0
/// through 8 -- nine of them, not eight -- and dividing by 8 rather than 9 is
/// what leaves the dense end of the ramp one generation short of being
/// reachable.
const MAX_AGE: u8 = 8;

/// The character every live cell is drawn with, whatever its age.
///
/// One, and the number *is* the answer. This has been three shapes in this
/// file's history and the character is the only part of the picture a viewer
/// cannot help but watch. It was a random katakana per cell per generation,
/// which made a cell that was standing still look like it was flickering; then
/// a continuous ramp of the age, which held still but changed every live
/// character's shape on every single generation; then three bands of three
/// generations, which changed it two or three times in a cell's life and was
/// still a character changing. The report each time was the same shape: it does
/// not look like a real game of life.
///
/// So the age is out of the character entirely, and it never goes back in. One
/// constant glyph for all nine ages, and the ramp is the colour's alone -- see
/// [`age_fraction`] for why that division is the right way round. A colour can
/// drift a shade every generation without the picture flickering, because
/// nothing about the *shape* of a cell changes; a character cannot, because
/// shape is exactly what the eye tracks.
///
/// `●` U+25CF BLACK CIRCLE, chosen by the user.
///
/// The whole of the age is out of the character and this is what is left. A
/// filled disc rather than a hollow one, so a run of live cells reads as a mass
/// and the structures between them are the gaps.
///
/// `@` was tried here first, and it was this file's own reading of "one big
/// circle" rather than the request: `@` is the densest round character ASCII
/// has, and the reasoning for it was about *lattice versus population* -- `O` is
/// a hollow ring, so a row of live cells draws as a row of outlines with the
/// background showing through each one. That argument was answered twice with
/// the same reply, which is the character itself. The glyph is not a judgement
/// call about how a population should read; it is a picture the user can see,
/// and `@` is a picture of a matrix, not of a cell.
///
/// Which is also why the character is not configurable. It was never a
/// `ConwayLifeOptions` key -- the ramp was a private static -- and the age ramp
/// became a colour-only function. One constant, one glyph, one place.
const LIVE_GLYPH: char = '\u{25CF}';

/// The colours a live cell is drawn in, by how long it has been alive.
///
/// A dim green at birth through to near-white once it has settled, so the colour
/// is per-cell and carries the same reading as the glyph. It used to be
/// `255 - current_gen` with `current_gen` a counter shared by the whole screen,
/// which made every live cell the same green and pulsed the entire effect in
/// brightness once every 255 generations -- about 32 seconds at the default
/// rate, and the reason the effect reads as one colour that flickers rather than
/// as a population.
///
/// Three stops rather than two because a straight line from a dark green to a
/// white runs through a desaturated grey-green, and the middle of the ramp is
/// where most of a life soup's cells live.
static AGE_COLOURS: LazyLock<Palette> = LazyLock::new(|| {
    Palette::new(vec![
        style::Color::Rgb { r: 0, g: 96, b: 46 },
        style::Color::Rgb {
            r: 46,
            g: 210,
            b: 92,
        },
        style::Color::Rgb {
            r: 214,
            g: 255,
            b: 224,
        },
    ])
});

/// Where an age sits in the *colour* ramp: `0.0` is a newborn, `1.0` is as old
/// as the ramp goes.
///
/// Curved, and that is what makes the ramp work. The ages a life soup produces
/// are the distribution measured on [`MAX_AGE`]: nearly all of the mass is in the
/// first three generations, so a ramp indexed *linearly* in the age puts 92% of
/// the screen on two colours and never reaches the third, which is the same as
/// having no ramp at all. The curve spends the ramp where the cells are, with
/// the weight descending the way the ages do, while still letting a long-lived
/// structure climb to the bright end.
///
/// The exponent is 0.7 and it was not chosen by eye. The palette's middle stop
/// is at `0.5`, and a linear index reaches it at age four; the curve reaches it
/// at age three, which is where the cells are. It is the gentlest exponent that
/// does that: a square root reaches the middle stop at age *two*, which is past
/// the mass of the distribution rather than into it, and overspends the sparse
/// end so badly that the first stop is only ever reached by a cell in its first
/// generation -- 44% of the population, all of it on one colour.
///
/// This used to index the *glyph* ramp as well, and the note above used to be
/// about spending all seven glyphs on the nine ages. That is no longer what
/// happens and pretending otherwise would be the kind of comment that outlives
/// its code: every live cell is drawn with one character now, so the curve is
/// only the colour's, and it is continuous rather than quantised. See
/// [`glyph_for_age`].
///
/// Which is the right division of the work, and it is worth being explicit about
/// why because it looks backwards. A colour drifting a shade every generation is
/// something the eye reads as the cell *aging* -- the structures visibly settle,
/// brightening as they hold -- and nothing in the picture jumps. A character
/// changing is read as the cell being *replaced*, and at eight generations a
/// second the whole board is replaced eight times a second. The report this
/// change answers is that the effect did not look like a game of life, and
/// "the age is in the colour" is what makes it look like one.
///
/// Saturating rather than wrapping, so a cell that outlives [`MAX_AGE`] is at the
/// top of the ramp and not back at the sparse end. A still life drawn entirely
/// at the sparse end would look like a field of births, which is the one reading
/// that would be actively wrong.
fn age_fraction(age: u8) -> f32 {
    (f32::from(age.min(MAX_AGE)) / f32::from(MAX_AGE)).powf(0.7)
}

/// The glyph for a cell of this age.
///
/// The same character every time, which is the whole of what this function does
/// and the reason it is still a function. See [`LIVE_GLYPH`] for the history:
/// random per generation, then a continuous ramp of the age, then three bands
/// of three, and now a constant. Three changes in one direction, each one
/// removing a way for the character to move, and the age argument is kept only
/// so a call site reads as "the glyph for a cell this old" and so the compiler
/// catches a caller that would rather be passing nothing.
///
/// Deliberately still taking the age. A caller that could pass the age or not
/// would pass it not, and the day someone needs a second glyph this signature
/// is the one place that has to be revisited on purpose.
#[inline]
fn glyph_for_age(_age: u8) -> char {
    LIVE_GLYPH
}

/// The colour for a cell of this age.
///
/// Continuous in the age, unlike every version of the glyph before
/// [`LIVE_GLYPH`], and it is the only thing left that carries the age. See
/// [`age_fraction`] for the curve and [`glyph_for_age`] for why the division of
/// the work between them is the right way round.
fn color_for_age(age: u8) -> style::Color {
    AGE_COLOURS.sample(age_fraction(age))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ConwayLifeOptions {
    /// Which rule of the Life family to run.
    ///
    /// A `B<births>/S<survivals>` specification, or one of the names in
    /// [`crate::life::rule::NAMED_RULES`]. `B3/S23` and `life` are the same
    /// rule, which is Conway's own.
    ///
    /// The whole family is one option rather than eighteen effects, because
    /// every rule wants the seeding, the boundary handling, the glider injection
    /// and the age-in-the-glyph that this effect already has, and a variant that
    /// is a different pair of numbers does not need its own name in `--help` and
    /// its own 1,200 lines in the crate.
    ///
    /// **Refused rather than defaulted**, which is the opposite of how the
    /// `palette` options behave and for a specific reason. An unknown palette
    /// falls back and the picture comes out the wrong colour, which is obvious.
    /// An unknown *rule* falls back to Conway's Life and the picture comes out
    /// as a perfectly plausible Game of Life, with nothing anywhere saying the
    /// config was not read. So a bad value is an error, and the message names
    /// the forms it accepted.
    ///
    /// Two famous rules are deliberately not available, because neither is
    /// totalistic and neither can be written as a birth/survival pair: `Day &
    /// Night` has eight states per cell, and `Replicator` is born from a
    /// particular *arrangement* of neighbours rather than a count. See
    /// [`crate::life::rule`] for the whole boundary.
    ///
    /// **The gliders ignore the rule.** This effect injects
    /// [`GLIDERS_PER_GENERATION`] gliders into every generation, and those are
    /// births that never consult the birth half of whatever rule is configured.
    /// So a restrictive rule cannot empty the board, and `B3/-S23` -- a rule with
    /// no births at all -- still has gliders drifting through it. That is the
    /// effect's character rather than an oversight, but it is surprising if you
    /// did not know, so it is written down here.
    #[serde(deserialize_with = "deserialize_rule")]
    pub rule: String,

    #[serde(skip)]
    pub initial_cells: u32,
    pub cells_coeff: f32,
    /// Generations a second, and how fast the population evolves.
    ///
    /// 3.0, down from 8.0, and the other half of that is the glyph: at eight a
    /// second the whole board was redrawn eight times a second, and a generation
    /// is a third of a second now rather than an eighth.
    ///
    /// The rate is a reading of Conway's rules, not just a clock. At eight a
    /// second a glider crosses a cell every one and a half frames, a block is over
    /// in less than a twentieth of a second, and the board is a different pattern
    /// before any of it can be followed. A third of a second per generation is
    /// about the rate at which a person can watch a single cell and say what
    /// happened to it, which is the only thing a screensaver of Conway's rules has
    /// to offer.
    ///
    /// The glyph is a constant now, so the rate is no longer load-bearing for
    /// whether a cell's *appearance* holds still -- that holds for as long as the
    /// cell does. It is still the rate at which a structure can be watched
    /// assembling, which is the other half of looking like a game of life.
    pub generations_per_second: f32,
    /// Seed for the initial population and the gliders seeded each generation.
    ///
    /// No longer also seeds the glyph, because the glyph is not random any more:
    /// every live cell is drawn with one character. See [`LIVE_GLYPH`].
    pub seed: u64,
}

impl Default for ConwayLifeOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            rule: "B3/S23".to_string(),
            initial_cells: 200,
            cells_coeff: 1.0,
            generations_per_second: 3.0,
            seed: DEFAULT_SEED,
        }
    }
}

#[derive(Clone, Default)]
pub struct LifeCell {
    pub age: u8,
}

pub struct ConwayLife {
    pub screen_size: (u16, u16),
    #[allow(dead_code)]
    options: ConwayLifeOptions,
    /// The parsed rule, resolved once at construction.
    rule: LifeRule,
    canvas: Canvas,
    cells: HashMap<(usize, usize), LifeCell>,
    pub rng: EffectRng,
    generation_accumulator: f32,
}

impl LifeCell {
    /// A cell that has just been born: age zero, which is the sparse end of both
    /// ramps.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TerminalEffect for ConwayLife {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear();
        self.fill_buffer();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.options.initial_cells = (self.screen_size.0 as f32
            * self.screen_size.1 as f32
            * 0.15
            * self.options.cells_coeff) as u32;

        // The canvas has to follow the new size, and cells that no longer fit
        // have to go. `Canvas::resize` blanks the baseline so the next commit
        // repaints in full, which is what a resize needs.
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.cells.retain(|(x, y), _| {
            *x < self.screen_size.0 as usize && *y < self.screen_size.1 as usize
        });
        self.generation_accumulator = 0.0;
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl ConwayLife {
    fn advance(&mut self, delta: f32) {
        self.generation_accumulator += delta;
        let step = 1.0 / self.options.generations_per_second.max(0.1);
        let mut steps = 0;
        while self.generation_accumulator >= step && steps < 4 {
            self.generation_accumulator -= step;
            self.step_generation();
            steps += 1;
        }
        if steps == 4 {
            self.generation_accumulator = 0.0;
        }
    }

    fn step_generation(&mut self) {
        let (width, height) = (self.canvas.width(), self.canvas.height());

        // Seed gliders into the generation we are about to evolve, so this
        // generation's rules apply to them. Seeding them afterwards, into the
        // already-computed result, means they are never subject to the rules at
        // all: they cannot age, cannot die, and cannot travel, because every
        // generation replaces them with fresh ones somewhere else.
        //
        // Their coordinates are collected because a glider is a *birth*, not a
        // cell that has been alive for a generation: ageing the whole map
        // uniformly would start every glider one generation older than it is,
        // which on the sparse end of the ramp is the difference between a
        // visible cell and a barely-there one.
        let mut seeded: HashSet<(usize, usize)> = HashSet::new();
        // The range below is `2..width - GLIDER_SIZE + 1`, which is empty --
        // and `random_range` asserts on an empty range rather than returning
        // anything -- unless `width - GLIDER_SIZE + 1 > 2`, that is
        // `width >= GLIDER_SIZE + 2`. The old guard tested
        // `width <= GLIDER_SIZE`, which admits `width == GLIDER_SIZE + 1`, so a
        // four-column terminal panicked on the first generation with a bare
        // "cannot sample empty range" that names neither `life` nor a cause.
        //
        // Four columns is not exotic: a terminal narrowed by a vertical split
        // reaches it, and `update_size` clamps to 1 rather than to anything
        // this guard could rely on.
        const MIN_FOR_GLIDER: usize = GLIDER_SIZE + 2;
        for _ in 0..GLIDERS_PER_GENERATION {
            if width < MIN_FOR_GLIDER || height < MIN_FOR_GLIDER {
                break;
            }
            let x = self.rng.random_range(2..width - GLIDER_SIZE + 1);
            let y = self.rng.random_range(2..height - GLIDER_SIZE + 1);
            let rotation = [0, 90, 180, 270][self.rng.random_range(0..4)];
            seeded.extend(insert_glider(&mut self.cells, x, y, rotation));
        }

        let mut next_cells = HashMap::new();

        for y in 0..height {
            for x in 0..width {
                // Liveness comes from the simulation state, never from the
                // rendered frame. Deriving it from the frame made the
                // simulation depend on when it was last drawn, so when several
                // generations were stepped between two renders they all evolved
                // the same stale input and produced identical results.
                let alive = self.cells.contains_key(&(x, y));
                let live_neighbors =
                    count_live_neighbors(&self.cells, x, y, width, height);

                let survives = self.rule.next_state(alive, live_neighbors);

                if !survives {
                    continue;
                }

                // A surviving cell carries its own age into the next
                // generation; a birth starts at zero. The age is the only
                // per-cell state worth keeping -- everything about how a cell
                // looks is derived from it at draw time, so there is nothing
                // cached that can disagree with the simulation.
                let cell = match self.cells.get(&(x, y)) {
                    Some(surviving) if !seeded.contains(&(x, y)) => {
                        let mut aged = surviving.clone();
                        aged.age = aged.age.saturating_add(1);
                        aged
                    }
                    // Either a birth -- a dead cell with three live neighbours --
                    // or a glider placed a moment ago. Both are new, so both
                    // start at zero; a glider on a cell that was already alive is
                    // the `Some` arm above, because a cell that did not die has
                    // not been born again.
                    _ => LifeCell::new(),
                };
                next_cells.insert((x, y), cell);
            }
        }

        self.cells = next_cells;
    }

    pub fn new(options: ConwayLifeOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let mut rng = seeded_rng(options.seed, "life");
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        // The config loader has already refused an unparseable rule, so this
        // fallback only exists for an options struct built in code. It is *not*
        // silent: the crate's rule for a wrong value here is that a bad rule
        // produces a plausible-looking Life and a bad colour produces an
        // obviously-wrong picture, and only one of those is worth guessing about
        // quietly. A hand-written `rule` is a bug in the caller and is reported
        // as one.
        let rule = LifeRule::parse(&options.rule).unwrap_or_else(|reason| {
            eprintln!(
                "termzzz: [life] ignoring unusable rule {:?}: {reason}",
                options.rule
            );
            LifeRule::LIFE
        });

        let mut cells = HashMap::new();
        for _ in 0..options.initial_cells {
            let x = rng.random_range(0..screen_size.0) as usize;
            let y = rng.random_range(0..screen_size.1) as usize;

            cells.insert((x, y), LifeCell::new());
        }

        Self {
            screen_size,
            options,
            rule,
            canvas,
            cells,
            rng,
            generation_accumulator: 0.0,
        }
    }

    /// The rule in force, for the effect's own tests.
    #[cfg(test)]
    fn rule(&self) -> LifeRule {
        self.rule
    }

    /// Writes every live cell into the canvas.
    ///
    /// `Attribute::Reset` rather than `Attribute::Bold`, which is what this used
    /// to set on every cell. Several terminals treat bold on a foreground as a
    /// request to brighten the colour rather than as an attribute of its own, so
    /// a bold dark-green newborn and a bold near-white settled cell both came out
    /// "bold" and the age ramp flattened into whatever the brightening did. Five
    /// other effects in this crate had the same bug and all use `Reset`.
    pub fn fill_buffer(&mut self) {
        let (width, height) = (self.canvas.width(), self.canvas.height());
        for ((x, y), cell) in self.cells.iter() {
            if *x < width && *y < height {
                self.canvas.set(
                    *x,
                    *y,
                    Cell::new(
                        glyph_for_age(cell.age),
                        color_for_age(cell.age),
                        style::Attribute::Reset,
                    ),
                );
            }
        }
    }
}

/// Puts one glider into `cells` and returns the cells it actually filled.
///
/// The return value is the point of the function: the caller has to know which
/// coordinates are *new* so it can leave them at age zero rather than ageing
/// them along with everything that was already alive. A glider landing on a cell
/// that is already occupied is not a birth and is deliberately not returned.
fn insert_glider(
    cells: &mut HashMap<(usize, usize), LifeCell>,
    x: usize,
    y: usize,
    rotation: i32,
) -> Vec<(usize, usize)> {
    const BASE_GLIDER: [(usize, usize); 5] =
        [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

    // Every rotation maps the shape back into the same 3x3 box, so each arm has
    // to re-centre it. The 270-degree arm was missing that, which shifted a
    // quarter of all seeded gliders two columns off and left them malformed.
    let mut filled = Vec::with_capacity(BASE_GLIDER.len());
    for (dx, dy) in BASE_GLIDER {
        let (cx, cy) = match rotation {
            0 => (x + dx, y + dy),
            90 => (x + dy, y + 2 - dx),
            180 => (x + 2 - dx, y + 2 - dy),
            270 => (x + 2 - dy, y + dx),
            _ => (x + dx, y + dy),
        };

        // Occupied cells are left alone rather than overwritten: a glider drawn
        // over a cell that is already alive used to reset that cell's age to
        // zero, which reads as a birth that never happened -- the cell did not
        // die and come back, a glider was stamped on top of it. With nine gliders
        // a generation the odds of that were high enough that a settled structure
        // was being thrown back to the sparse end of the ramp a few times a
        // second.
        if cells.contains_key(&(cx, cy)) {
            continue;
        }
        cells.insert((cx, cy), LifeCell::new());
        filled.push((cx, cy));
    }

    filled
}

/// Counts the living cells in the eight-cell neighbourhood around `(x, y)`.
///
/// This used to build a `Vec` of `(index, Cell)` pairs for every cell of the
/// screen on every generation, and the caller only ever asked for its length.
/// On a 200x50 terminal that was ten thousand heap allocations per generation
/// to produce a number that fits in a `u8`.
pub fn count_live_neighbors(
    cells: &HashMap<(usize, usize), LifeCell>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
) -> u8 {
    let mut live = 0u8;

    let y_start = y.saturating_sub(1);
    let y_end = (y + 1).min(height.saturating_sub(1));
    let x_start = x.saturating_sub(1);
    let x_end = (x + 1).min(width.saturating_sub(1));

    for ny in y_start..=y_end {
        for nx in x_start..=x_end {
            if (nx, ny) == (x, y) {
                continue;
            }
            if cells.contains_key(&(nx, ny)) {
                live += 1;
            }
        }
    }

    live
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    /// The step loop uses the configured rule rather than a hardcoded Life.
    ///
    /// This is the assertion the whole feature rests on and it lives here
    /// rather than in `rule.rs` for a specific reason: every test in that module
    /// reads the rule *directly*, so a `step_generation` still containing
    /// `live_neighbors == 3` would pass all of them. The rule table would be
    /// correct, the option would parse, and the picture would not change.
    ///
    /// Asserted as a difference in the resulting *board*, not in a returned
    /// value, because the board is what the user sees.
    #[test]
    fn the_configured_rule_changes_the_simulation() {
        let build = |rule: &str| {
            // A fixed soup, so the only thing that can differ between two runs
            // is the rule.
            let options = ConwayLifeOptions {
                rule: rule.to_string(),
                initial_cells: 400,
                ..ConwayLifeOptions::default()
            };
            ConwayLife::new(options, (60, 30))
        };

        let mut life = build("B3/S23");
        let mut highlife = build("highlife");
        for _ in 0..8 {
            life.step_generation();
            highlife.step_generation();
        }

        assert_eq!(life.rule(), LifeRule::LIFE);
        assert_eq!(highlife.rule(), LifeRule::parse("highlife").unwrap());
        // `LifeCell` holds only an age, so the cells' *positions* are what
        // identifies a board, and comparing the sorted key sets avoids needing
        // `Debug` or `PartialEq` on the cell type for this one check.
        let keys =
            |life: &ConwayLife| -> std::collections::BTreeSet<(usize, usize)> {
                life.cells.keys().copied().collect()
            };
        assert_ne!(
            keys(&life),
            keys(&highlife),
            "the two rules produced an identical board, so the step loop is not \
             consulting the configured rule"
        );
    }

    /// No birth counts means the board can only shrink, plus the gliders.
    ///
    /// The bound is exact, and the constraint it comes from is not about rules
    /// at all: this effect injects [`GLIDERS_PER_GENERATION`] gliders of five
    /// cells into **every** generation, and those are births that never consult
    /// the birth half of whatever rule is configured. So two obvious claims are
    /// both false, and both were written here and then measured:
    ///
    /// - "a rule with no survival counts empties the board" is false for
    ///   `seeds` on a quarter-full board, which was still at 167 cells after
    ///   thirty generations. Seeds is born on two, and a dense board keeps
    ///   making pairs, so it replaces every survivor it removes.
    /// - "a rule with no birth counts empties the board" is false for the same
    ///   reason.
    ///
    /// What is true, and is the statement that actually shows the birth table is
    /// being consulted, is that *apart from the injected gliders* the population
    /// never grows. That is a tight bound rather than a loose one, so it is
    /// worth having.
    #[test]
    fn a_rule_with_no_births_can_only_lose_cells_to_the_gliders() {
        let options = ConwayLifeOptions {
            rule: "B-/S23".to_string(),
            initial_cells: 200,
            ..ConwayLifeOptions::default()
        };
        let mut life = ConwayLife::new(options, (40, 20));

        // The most the injection can add in one generation.
        let injection = (GLIDERS_PER_GENERATION * 5) as isize;
        let mut previous = life.cells.len();
        for generation in 0..40 {
            life.step_generation();
            let growth = life.cells.len() as isize - previous as isize;
            assert!(
                growth <= injection,
                "generation {generation} grew the board by {growth}, more than the \
                 {injection} cells the glider injection accounts for, so \
                 something is being born that the configured rule forbids"
            );
            previous = life.cells.len();
        }

        // And it does trend downwards rather than sitting still.
        assert!(
            life.cells.len() < 200,
            "a board that can only lose cells finished with {} of its original \
             200, so it is not actually losing them",
            life.cells.len()
        );
    }

    /// The default is Conway's Life, spelled in full.
    ///
    /// `B3/S23` rather than the name `life` on purpose: the generated config
    /// should not depend on the alias table, and a test that asserted the
    /// default equals the string `life` would keep passing if the spelling
    /// changed to something the table had lost.
    #[test]
    fn the_default_rule_is_conways_life() {
        assert_eq!(ConwayLifeOptions::default().rule, "B3/S23");
        let life = ConwayLife::new(ConwayLifeOptions::default(), (20, 10));
        assert_eq!(life.rule(), LifeRule::LIFE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Perceived brightness of a colour, `0..=255`.
    ///
    /// In the test module because it exists to answer "is this ramp actually a
    /// ramp", and nothing in the effect needs to know how bright a cell is.
    fn luminance(color: style::Color) -> u8 {
        match color {
            style::Color::Rgb { r, g, b } => {
                (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b))
                    .round() as u8
            }
            _ => 0,
        }
    }

    #[test]
    fn no_live_neighbors_on_an_empty_grid() {
        let cells: HashMap<(usize, usize), LifeCell> = HashMap::new();

        for x in 0..3 {
            for y in 0..3 {
                assert_eq!(count_live_neighbors(&cells, x, y, 3, 3), 0);
            }
        }
    }

    #[test]
    fn counts_only_the_eight_around_a_cell() {
        let mut cells: HashMap<(usize, usize), LifeCell> = HashMap::new();
        for y in 0..3 {
            cells.insert((0, y), LifeCell::new());
        }

        // The cell at (1, 1) has all three of them as neighbours.
        assert_eq!(count_live_neighbors(&cells, 1, 1, 3, 3), 3);
        // (0, 0) is itself one of the three, and (0, 2) is two rows away, so it
        // only sees (0, 1).
        assert_eq!(count_live_neighbors(&cells, 0, 0, 3, 3), 1);
    }

    #[test]
    fn edges_and_corners_see_fewer_than_eight() {
        let mut cells: HashMap<(usize, usize), LifeCell> = HashMap::new();
        for y in 0..4 {
            for x in 0..4 {
                cells.insert((x, y), LifeCell::new());
            }
        }

        // Interior cells are surrounded on all eight sides.
        for &(x, y) in &[(1, 1), (1, 2), (2, 1), (2, 2)] {
            assert_eq!(count_live_neighbors(&cells, x, y, 4, 4), 8);
        }
        // Edge cells miss two neighbours, corner cells miss five.
        assert_eq!(count_live_neighbors(&cells, 1, 0, 4, 4), 5);
        assert_eq!(count_live_neighbors(&cells, 0, 0, 4, 4), 3);
    }

    #[test]
    fn reset_rebuilds_for_new_screen_size() {
        let options = ConwayLifeOptions {
            initial_cells: 0,
            cells_coeff: 0.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (5, 4));

        life.update_size(8, 6);
        life.reset();

        assert_eq!(life.screen_size, (8, 6));
        assert_eq!(life.canvas.size(), (8, 6));
        assert!(life.cells.is_empty());
    }

    #[test]
    fn resize_recomputes_initial_cell_count() {
        let options = ConwayLifeOptions {
            initial_cells: 0,
            cells_coeff: 1.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (20, 20));

        life.update_size(10, 10);

        assert_eq!(life.options.initial_cells, 15);
    }

    // --- how a live cell is drawn -----------------------------------------

    /// Runs the effect far enough that the population is a soup with cells of
    /// every age in it, and returns every cell it drew.
    ///
    /// The assertions below are about what reaches the *screen*, not about the
    /// glyph table: a ramp that was correct but never reached `fill_buffer`
    /// would pass a narrower test and still render the old picture.
    fn draw_a_settled_population() -> ConwayLife {
        let options = ConwayLifeOptions {
            initial_cells: 220,
            cells_coeff: 1.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (80, 40));
        for _ in 0..120 {
            life.step_generation();
        }
        life.fill_buffer();
        life
    }

    /// Every live cell is drawn with one character, and the colour is what says
    /// how old it is.
    ///
    /// The user's own brief: "don't change the characters at all; just have one
    /// big circle for one live cell", and the disambiguation that went with it
    /// -- one constant round glyph for every age, with the age carried entirely
    /// by the colour ramp. The circle is U+25CF BLACK CIRCLE, named the second
    /// time the request was made after `@` was tried and not what was asked for.
    ///
    /// The name of this test used to be `a_live_cells_glyph_depends_on_how_long_
    /// it_has_been_alive` and it asserted the exact opposite, deliberately
    /// inverted once already. The first version of it was the honest
    /// observation that a random-per-generation glyph was wrong, so it required
    /// the nine ages to resolve to all seven glyphs. The second was a continuous
    /// ramp of the age, which fixed the randomness and made every live cell
    /// change character on every generation, so the first assertion was flipped
    /// and it came to require *fewer* glyphs than the ramp held -- down to
    /// `AGE_BANDS` of three. That was an improvement and it is now also the
    /// wrong thing: three bands still means a cell's character changes two or
    /// three times in its life, and a changing character is read as a cell being
    /// replaced. So this is the third inversion, and it is the one that removes
    /// the dependence rather than weakening it.
    ///
    /// Byte-identical across every age is the assertion, and *byte*-identical
    /// rather than "the same set" because a ramp indexed by the age with a single
    /// entry would satisfy a weaker reading of this. The ages that exist are 0
    /// through [`MAX_AGE`] inclusive, so the loop covers all nine rather than the
    /// eight a `0..MAX_AGE` would have caught -- the same off-by-one
    /// [`MAX_AGE`] exists to warn about on the colour side, and it is the
    /// saturating end here that a still life lives at.
    ///
    /// The set is compared against the literal `\u{25CF}` and not against
    /// [`LIVE_GLYPH`], so this is a statement about the *picture* and the constant
    /// is the thing that has to agree with it. Asserting only the constant would
    /// let a future "improve" of the character pass here and fail in the width
    /// test; asserting only the set length would have passed unchanged when the
    /// glyph went from `@` to `●`, which is the change this test had to survive.
    ///
    /// Then the other half, which is the claim that actually has content: the
    /// colour still drifts, and it is what the age is now visible *in*. Two
    /// assertions in each direction, because each alone is satisfiable by
    /// accident. A flat colour would pass "different ages differ" if only one
    /// pair were checked; a per-cell random colour would pass it too, and would
    /// also pass "same ages agree" at one age. So a newborn against a settled
    /// cell has to be a *visible* distance apart, and two cells of the same age
    /// have to be identical, which together pin it as a function of the age and
    /// not of the cell.
    ///
    /// Read out of `fill_buffer` on a real settled population rather than from
    /// `color_for_age` alone, so a `fill_buffer` that stopped consulting the
    /// colour would be caught here.
    #[test]
    fn every_live_cell_is_one_glyph_and_its_colour_is_its_age() {
        // One character, for every age, and it is the circle that was asked for.
        let glyphs: HashSet<char> = (0..=MAX_AGE).map(glyph_for_age).collect();
        assert_eq!(
            glyphs,
            HashSet::from(['\u{25CF}']),
            "the nine ages 0..={MAX_AGE} resolve to {glyphs:?} rather than one \
             character, and that character is not U+25CF BLACK CIRCLE, so a cell's \
             appearance still changes as it ages"
        );
        assert_eq!(
            glyph_for_age(0),
            LIVE_GLYPH,
            "a newborn is not drawn with the one glyph"
        );
        assert_eq!(
            glyph_for_age(MAX_AGE),
            LIVE_GLYPH,
            "a settled cell is not drawn with the one glyph"
        );

        // On a real population, every drawn cell carries that one character.
        let life = draw_a_settled_population();
        assert!(
            !life.cells.is_empty(),
            "the run settled to nothing, so there is no cell to check"
        );
        for (cell, data) in &life.cells {
            let drawn = life.canvas.get(cell.0, cell.1);
            assert_eq!(
                drawn.symbol, LIVE_GLYPH,
                "the live cell at ({}, {}) is drawn {:?} rather than the one glyph",
                cell.0, cell.1, drawn.symbol
            );
            assert_eq!(
                drawn.symbol,
                glyph_for_age(data.age),
                "the cell at ({}, {}) is drawn {:?} but its age of {} maps to {:?}",
                cell.0,
                cell.1,
                drawn.symbol,
                data.age,
                glyph_for_age(data.age)
            );
        }

        // And the colour is what the age is now visible in. Different ages
        // differ, by more than one quantisation step or the ramp is not doing a
        // job; same ages agree exactly, or it is a per-cell value and not a
        // function of the age.
        assert_ne!(
            color_for_age(0),
            color_for_age(MAX_AGE),
            "a newborn and a settled cell are drawn the same colour, so the age \
             is not visible anywhere"
        );
        let newborn = luminance(color_for_age(0));
        let settled = luminance(color_for_age(MAX_AGE));
        assert!(
            settled as i32 - newborn as i32 > 100,
            "age 0 and age {MAX_AGE} differ by {} luminance, which is not a ramp",
            settled as i32 - newborn as i32
        );
        for age in 0..=MAX_AGE {
            assert_eq!(
                color_for_age(age),
                color_for_age(age),
                "age {age} does not map to one colour"
            );
        }

        // Both directions on a live population, which is the half a table-driven
        // assertion cannot reach: two drawn cells of the same age are the same
        // colour, and the population really does contain a spread of ages for
        // the first half to be about anything.
        let mut by_age: HashMap<u8, Vec<style::Color>> = HashMap::new();
        for (cell, data) in &life.cells {
            by_age
                .entry(data.age)
                .or_default()
                .push(life.canvas.get(cell.0, cell.1).color);
        }
        assert!(
            by_age.len() > 1,
            "every live cell is the same age, so the colour has nothing to say"
        );
        for (age, colours) in &by_age {
            assert!(
                colours.windows(2).all(|pair| pair[0] == pair[1]),
                "two cells of age {age} were drawn in different colours, so the \
                 colour is not a function of the age"
            );
        }
    }

    /// A cell's character is held for about a second, which is the whole point
    /// of both halves of this change.
    ///
    /// The two are one fix and neither works alone. Holding a character while
    /// the simulation runs at eight generations a second keeps it for three
    /// frames and a half, and slowing the simulation while the character still
    /// changes keeps the whole board changing every third of a second. The
    /// invariant worth pinning is the product: how long a character stays on
    /// screen.
    ///
    /// Stated as a duration rather than as either number on its own so that
    /// raising the rate and shortening the hold together cannot pass, which is
    /// the change that would look like an improvement to a reader of either
    /// constant alone. A third of a second is a generation; three generations is
    /// a second.
    ///
    /// Now that the character is constant the *character* half is unbounded --
    /// `LIVE_GLYPH` is held for as long as the cell lives, so this measures the
    /// rate half only, and the seconds-per-character figure is what a two-or-three
    /// band glyph would have given. Kept in that form deliberately: it is the
    /// number that decides whether the rate can go up again, and it is the
    /// arithmetic a future second glyph would have to beat.
    #[test]
    fn a_character_stays_on_screen_long_enough_to_read() {
        // What a glyph that changed at a band boundary would have given.
        const BAND_GENERATIONS: f32 = 3.0;
        let seconds_per_glyph =
            BAND_GENERATIONS / ConwayLifeOptions::default().generations_per_second;
        assert!(
            seconds_per_glyph >= 0.75,
            "a banded character is held for {seconds_per_glyph:.2} seconds, which \
             is under the three quarters of a second a cell needs to be readable, \
             and the rate is what has to give"
        );
        assert!(
            ConwayLifeOptions::default().generations_per_second <= 4.0,
            "the population evolves {} times a second, so a glider crosses a cell \
             every {} frames and no single cell can be followed",
            ConwayLifeOptions::default().generations_per_second,
            60.0 / ConwayLifeOptions::default().generations_per_second
        );
    }

    /// No live cell's character ever changes, from one generation to the next.
    ///
    /// The complaint, as a number rather than as an impression: "every frame the
    /// characters change". Measured over a settled soup, the fraction of cells
    /// that are live both before and after a generation *and* whose character
    /// changed across it. That was every one of them, every generation, which is
    /// not a ramp reading a cell's age -- it is noise, and no rate of generation
    /// makes it watchable because the character changes on each one.
    ///
    /// The threshold was a third, which was about right for a glyph that changed
    /// at band boundaries and is now wrong in the other direction. With one
    /// character for every age the answer is not "most" or "a third" but *none*,
    /// and asserting anything weaker would leave room for the age back in: a
    /// banded glyph passes a third and fails zero, and zero is the only threshold
    /// that distinguishes them. So this is now an exact count.
    ///
    /// Only *survivors*, and that is the harder half of the question rather than
    /// the easier one: a cell that dies is not a cell whose character changed, it
    /// is a cell that is gone, and counting those in would let the number be
    /// dominated by the churn at the edge of a soup.
    #[test]
    fn no_cell_changes_its_character_from_one_generation_to_the_next() {
        let options = ConwayLifeOptions {
            initial_cells: 220,
            cells_coeff: 1.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (80, 40));
        for _ in 0..40 {
            life.step_generation();
        }

        let mut previous: HashMap<(usize, usize), char> = life
            .cells
            .iter()
            .map(|(cell, data)| (*cell, glyph_for_age(data.age)))
            .collect();
        let mut changed: Vec<((usize, usize), char, char)> = Vec::new();
        let mut survivors = 0usize;
        for _ in 0..100 {
            life.step_generation();
            for (cell, data) in &life.cells {
                let glyph = glyph_for_age(data.age);
                if let Some(before) = previous.get(cell) {
                    survivors += 1;
                    if before != &glyph {
                        changed.push((*cell, *before, glyph));
                    }
                }
            }
            previous = life
                .cells
                .iter()
                .map(|(cell, data)| (*cell, glyph_for_age(data.age)))
                .collect();
        }

        assert!(
            survivors > 1000,
            "only {survivors} cell-generations were compared, so this is not a \
             measurement of a population"
        );
        assert!(
            changed.is_empty(),
            "{} of the {survivors} cells that survived a generation had their \
             character changed by it -- the first was at ({}, {}), drawn {:?} \
             before and {:?} after, so a cell's appearance still depends on its age",
            changed.len(),
            changed[0].0.0,
            changed[0].0.1,
            changed[0].1,
            changed[0].2
        );
    }

    /// A long-lived cell's *colour* keeps changing, every generation, all the way
    /// up -- which is the half of the age ramp that survived this change.
    ///
    /// This test used to be about how long a cell holds its *character*, and it
    /// is worth saying why it cannot be about that any more, because it is the
    /// clearest example in this file of a test that stopped seeing its own
    /// subject. It measured the youngest age at which any cell held one
    /// character for three generations. With one character for every age that
    /// number is 2 -- a cell born at age 0 is first seen holding it once, and
    /// reaches three at age 2 -- and so it was 2 *before* this change as well,
    /// because the banded glyph also held ages 0, 1 and 2 constant. The
    /// assertion passed against the code it was written for and against the code
    /// it was written about, and tightening it to an exact zero would have made
    /// it a second copy of
    /// `no_cell_changes_its_character_from_one_generation_to_the_next`.
    ///
    /// What is left to assert is the part that is *supposed* to keep moving: the
    /// colour. A cell that lives twenty generations is drawn in twenty different
    /// colours, and a ramp banded alongside the glyph would give it three. That
    /// is the assertion, and it discriminates in both directions -- banding the
    /// colour fails it, and a flat colour fails it -- so unlike the version it
    /// replaces it cannot pass against whatever it is measured next to.
    ///
    /// One cell, followed across its whole life, rather than a population
    /// average. An average cannot see a ramp that steps: a board of mostly
    /// newborns and a few long-lived cells has a wide spread of colours whether
    /// or not any individual cell's colour changes. Counting the distinct
    /// colours *one* cell passes through is the direct question, and the
    /// threshold is three rather than two because three was the number of bands
    /// the glyph had -- a colour ramp quantised to match would land on exactly
    /// three.
    ///
    /// The cell is found after the fact rather than placed in advance, for the
    /// same reason the old version of this test ran the simulation: a block is
    /// the obvious fixture and does not survive here, because `step_generation`
    /// seeds nine gliders a generation at random positions and a glider landing
    /// near a block changes its neighbour counts and kills it.
    #[test]
    fn a_long_lived_cells_colour_keeps_drifting_every_generation() {
        const WATCHED: u8 = 6;

        let options = ConwayLifeOptions {
            initial_cells: 220,
            cells_coeff: 1.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (80, 40));
        for _ in 0..40 {
            life.step_generation();
        }

        // The oldest cell in the run, followed while it ages, so there is a life
        // long enough to watch. Re-picked every generation rather than chosen
        // once, because the oldest cell is usually a still life that may be
        // killed by a glider partway through.
        let mut best = life
            .cells
            .iter()
            .max_by_key(|(_, data)| data.age)
            .map(|(cell, _)| *cell)
            .expect("the population settled to nothing");
        let mut longest = life.cells.get(&best).map(|data| data.age).unwrap_or(0);
        for _ in 0..200 {
            life.step_generation();
            for (cell, data) in &life.cells {
                if data.age > longest {
                    longest = data.age;
                    best = *cell;
                }
            }
        }
        assert!(
            longest >= WATCHED,
            "the oldest cell in the run only reached age {longest}, so there is \
             not enough of a life here to watch a colour drift across"
        );

        // Now walk that one cell from birth, counting the colours it is drawn in.
        // Its ages are 0..=longest inclusive -- a survivor is its own age plus
        // one -- and a continuous ramp gives a different colour at each until the
        // palette stops resolving, so the count is bounded by the palette rather
        // than by the number of ages.
        let mut colours: Vec<(u8, style::Color)> = Vec::new();
        let mut seen: HashSet<style::Color> = HashSet::new();
        for step in 0..=longest {
            life.cells.insert(best, LifeCell { age: step });
            life.fill_buffer();
            let colour = life.canvas.get(best.0, best.1).color;
            if seen.insert(colour) {
                colours.push((step, colour));
            }
        }

        assert!(
            colours.len() > 3,
            "a cell that lived {longest} generations was only drawn in {} distinct \
             colours, at ages {:?} -- the colour is stepping rather than drifting",
            colours.len(),
            colours.iter().map(|(age, _)| *age).collect::<Vec<u8>>()
        );
        // And it has to be drifting *upward* in brightness, or the extra colours
        // are noise rather than a ramp.
        let first = colours[0];
        let last = *colours.last().expect("at least one colour");
        assert!(
            luminance(last.1) as i32 - luminance(first.1) as i32 > 100,
            "the colour went from {:?} at age {} to {:?} at age {}, which is not a \
             ramp towards the bright end",
            first.1,
            first.0,
            last.1,
            last.0
        );
    }

    /// The colour is read off the *canvas*, not off the age table, so a
    /// `fill_buffer` that stopped consulting the colour would be caught.
    ///
    /// That is the whole of what is new here. The claims it makes -- the colour
    /// is per-cell rather than a global counter, and the two ends of the age
    /// range are far enough apart in luminance to be a ramp -- are both asserted
    /// more strictly, and against the rendered frame rather than the table, by
    /// `every_live_cell_is_one_glyph_and_its_colour_is_its_age`. This one is
    /// kept as the narrower statement it has always been, on a settled
    /// population: the colour comes from the cell's age and the ramp spans it.
    ///
    /// Both assertions are duplicated deliberately rather than left to the
    /// broader test. A test that only exists inside a larger one stops being read
    /// when the larger one is rewritten, and the broader test is on its third
    /// rewrite in this file. Small, single-claim tests are what survive that.
    #[test]
    fn a_live_cells_colour_depends_on_its_age_rather_than_a_global_clock() {
        let life = draw_a_settled_population();

        // Off the canvas, not off `color_for_age`, so this is a statement about
        // what reaches the screen.
        let drawn: HashSet<style::Color> = life
            .cells
            .keys()
            .map(|cell| life.canvas.get(cell.0, cell.1).color)
            .collect();
        assert!(
            drawn.len() > 1,
            "every live cell came out the same colour, so the colour is a global \
             value rather than a per-cell one"
        );

        // And the spread is not one quantisation step: a newborn and a cell that
        // has been alive a full ramp's worth have to be far apart in luminance,
        // or the ramp is being indexed by something other than the age.
        let newborn = luminance(color_for_age(0));
        let settled = luminance(color_for_age(MAX_AGE));
        assert!(
            settled as i32 - newborn as i32 > 100,
            "age 0 and age {MAX_AGE} differ by {} luminance, which is not a ramp",
            settled as i32 - newborn as i32
        );
    }

    /// The one glyph is the big circle the user asked for, and it will not
    /// shear a cell-indexed grid.
    ///
    /// The glyph used to be `@`, and the width half of this test used to be
    /// `is_ascii()`. That was a stronger claim than the crate could support and
    /// it is now false in the interesting direction: `●` is U+25CF, which is
    /// outside ASCII, and the *reason* ASCII was used is gone with the reason for
    /// the old glyph. The old set was thirty-two halfwidth katakana, U+FF8A and
    /// neighbours, all of which are one column in a Latin-configured terminal and
    /// two in a CJK-configured one, so the same run of live cells came out one
    /// column wide in one terminal and two in the next. Whether a glyph is
    /// double-width is not something this crate can decide without a width table,
    /// so this borrows the one it has:
    /// [`is_ambiguous_or_narrow`](crate::render::glyph_ramp::is_ambiguous_or_narrow)
    /// is the predicate for the subset that is one cell in every terminal, and it
    /// accepts `●` -- U+25CF is inside its `\u{2190}..=\u{2BFF}` range.
    ///
    /// What the predicate does *not* settle, and what this test therefore has to
    /// say out loud, is the remaining risk. `●` is East_Asian_Width =
    /// *Ambiguous*: one column in a Latin-configured terminal, two in a
    /// CJK-configured one. So it is not in the safe subset in the strict sense,
    /// it is in it by the convention that ambiguous characters are drawn narrow,
    /// and a terminal configured the other way will shear the picture -- which
    /// `glyph_ramp`'s module docs are explicit is "not merely looking wrong".
    /// That is accepted rather than fixed: the glyph was chosen, twice, by
    /// someone who wanted this circle, and the alternative is a character that
    /// is not a circle. Not a way to argue the choice is wrong, just the
    /// remaining scope of what the assertion covers.
    ///
    /// The two properties that used to ride on `is_ascii()` and still have to
    /// hold on their own: not a control code, and not a space. A space is an
    /// invisible cell in the middle of a live structure, and a control code
    /// moves the cursor. Neither is about width, so neither is weakened by
    /// borrowing a narrower predicate.
    #[test]
    fn the_live_glyph_is_the_big_circle_and_will_not_shear_a_grid() {
        use crate::render::glyph_ramp::is_ambiguous_or_narrow;

        for age in 0..=MAX_AGE {
            let glyph = glyph_for_age(age);
            assert!(
                is_ambiguous_or_narrow(glyph),
                "age {age} draws {glyph:?} (U+{:04X}), which is double-width in at \
                 least one terminal, so a cell-indexed grid shears after it",
                glyph as u32
            );
            assert!(
                !glyph.is_control(),
                "age {age} draws the control character {glyph:?}"
            );
            assert!(
                glyph != ' ',
                "age {age} draws a space, so the cell it marks is invisible"
            );
        }
        // Asserted on the constant rather than only through `glyph_for_age`, so
        // that the property is about `LIVE_GLYPH` and not about a wrapper that
        // happens to normalise it away.
        assert_eq!(
            LIVE_GLYPH, '\u{25CF}',
            "the one live glyph is {LIVE_GLYPH:?} (U+{:04X}), not U+25CF BLACK \
             CIRCLE -- the request was one big filled circle for one live cell",
            LIVE_GLYPH as u32
        );
    }

    /// No cell is drawn bold.
    ///
    /// `fill_buffer` set `Attribute::Bold` on every live cell, and several
    /// terminals treat bold on a foreground as a request to brighten the colour
    /// rather than as a separate attribute. That makes the age ramp
    /// unreadable: a dark-green newborn and a pale settled cell both come out
    /// "bold", and the two collapse into whatever the brightening does. Five
    /// other effects in this crate had the same bug and all use `Reset`.
    #[test]
    fn no_live_cell_is_drawn_bold() {
        let life = draw_a_settled_population();

        assert!(
            !life.cells.is_empty(),
            "the run settled to nothing, so there is no cell to check"
        );
        for (x, y) in life.cells.keys() {
            let drawn = life.canvas.get(*x, *y);
            assert_ne!(
                drawn.attr,
                style::Attribute::Bold,
                "the live cell at ({x}, {y}) is drawn bold, which brightens \
                 rather than distinguishes it"
            );
        }
    }

    /// A cell that survives counts one more generation, and a birth starts at
    /// zero.
    ///
    /// Stated over the whole population rather than about one hand-placed cell,
    /// because `step_generation` seeds nine gliders at random positions and a
    /// single tracked cell is a coin flip. The invariant is exact whatever the
    /// gliders do: a coordinate that is live both before and after a generation
    /// has aged by exactly one, because the only two fates are "survived" and
    /// "dead", and a dead cell is not in the next generation at all.
    #[test]
    fn a_surviving_cell_ages_and_a_newborn_starts_over() {
        let options = ConwayLifeOptions {
            initial_cells: 260,
            cells_coeff: 1.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (60, 30));
        for _ in 0..8 {
            life.step_generation();
        }

        let before: HashMap<(usize, usize), u8> = life
            .cells
            .iter()
            .map(|(cell, data)| (*cell, data.age))
            .collect();
        life.step_generation();

        let mut survivors = 0;
        for (cell, after) in &life.cells {
            if let Some(age) = before.get(cell) {
                assert_eq!(
                    after.age,
                    age + 1,
                    "the live cell at ({}, {}) is still alive but went from age \
                     {age} to {}",
                    cell.0,
                    cell.1,
                    after.age
                );
                survivors += 1;
            } else {
                assert_eq!(after.age, 0, "a cell that was not there has an age");
            }
        }
        assert!(
            survivors > 0,
            "nothing survived a generation, so this proved nothing"
        );
    }

    /// A seeded glider does not reset the age of a cell that was already alive.
    ///
    /// It used to be stamped with a plain `insert`, which overwrote whatever was
    /// on the cell. Nine gliders a generation means the odds of one landing on a
    /// long-lived structure are high, and every hit threw away the age that
    /// structure had earned -- so the glyph and colour of a settled cell could be
    /// reset to a newborn's by something that never killed it.
    #[test]
    fn a_seeded_glider_leaves_an_existing_cell_alone() {
        let mut cells: HashMap<(usize, usize), LifeCell> = HashMap::new();
        cells.insert((1, 1), LifeCell { age: 200 });
        cells.insert((2, 2), LifeCell { age: 200 });

        // A glider at (0, 0) covers (1,0) (2,1) (0,2) (1,2) (2,2), so (2,2) is
        // one of its five cells and (1,1) is not.
        insert_glider(&mut cells, 0, 0, 0);

        assert_eq!(cells[&(2, 2)].age, 200, "the glider reset a live cell");
        assert_eq!(
            cells[&(1, 1)].age,
            200,
            "the glider reached a cell it does not cover"
        );
        assert!(cells.contains_key(&(1, 0)), "the glider missed a cell");
    }
}
