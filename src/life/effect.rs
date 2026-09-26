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
use crate::render::{GlyphRamp, Palette};
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
const MAX_AGE: u8 = 8;

/// How a live cell is drawn, sparse to dense, by how long it has been alive.
///
/// ASCII on purpose. This used to be thirty-two halfwidth katakana, U+FF8A and
/// its neighbours: one column wide in a Latin-configured terminal and two in a
/// CJK-configured one, so the same run of live cells came out one glyph wide in
/// one terminal and two in the next. Deciding a character's width properly wants
/// a width table, which this crate does not carry; ASCII is the one range that
/// is single-width in every terminal ever shipped.
///
/// The ordering is by eye, and it is the ordering this crate's own glyph-ramp
/// notes are careful about: `-` and `=` are *lighter* than `+` and `*`, so the
/// classic ten cannot be used as a value carrier. They are left out, and what is
/// left only ever adds strokes as it rises.
///
/// No space either. The classic ramp starts with one, which is right for a
/// field being faded out and exactly wrong here, where a space is an invisible
/// cell in the middle of a live structure.
static AGE_GLYPHS: LazyLock<GlyphRamp> =
    LazyLock::new(|| GlyphRamp::from_text(".:+*#%@"));

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

/// Where an age sits in the two ramps above: `0.0` is a newborn, `1.0` is as old
/// as the ramp goes.
///
/// Curved, and that is what makes the ramps work. The ages a life soup produces
/// are the distribution measured on [`MAX_AGE`]: nearly all of the mass is in the
/// first three generations, so a ramp indexed *linearly* in the age puts 92% of
/// the screen on two glyphs and never reaches the third, which is the same as
/// having no ramp at all. The curve spends the ramp where the cells are, with
/// the weight descending the way the ages do, while still letting a long-lived
/// structure climb to the dense end.
///
/// The exponent is 0.7 and it was not chosen by eye. It is the largest step that
/// still spends every entry of a seven-glyph ramp on the nine ages this
/// simulation produces: it maps ages 0 through 8 onto glyphs 0 through 6 with no
/// gaps. A square root is more aggressive and overspends the sparse end so badly
/// that the second glyph is never drawn at all -- 31% of the cells land on the
/// third and the second is decoration.
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
/// A pure function of the age, and that is the whole change. It used to be
/// `rng.random_range(0..32)` into a character table, redrawn for every surviving
/// cell on every generation and depending on nothing: not the cell's age, not
/// its neighbours, not where it is. A cell standing still was redrawn in a
/// different character sixty times a second, so the only thing the eye could
/// follow was characters disappearing, and the effect read as noise moving
/// around rather than as a population with a history.
fn glyph_for_age(age: u8) -> char {
    AGE_GLYPHS.sample(age_fraction(age))
}

/// The colour for a cell of this age. See [`glyph_for_age`] for why this is a
/// function of the cell rather than of the generation.
fn color_for_age(age: u8) -> style::Color {
    AGE_COLOURS.sample(age_fraction(age))
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct ConwayLifeOptions {
    #[serde(skip)]
    pub initial_cells: u32,
    pub cells_coeff: f32,
    pub generations_per_second: f32,
    /// Seed for the initial population and the gliders seeded each generation.
    ///
    /// No longer also seeds the glyph, because the glyph is not random any more:
    /// it is a function of a cell's age, so there is nothing for a seed to choose.
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
            initial_cells: 200,
            cells_coeff: 1.0,
            generations_per_second: 8.0,
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
        for _ in 0..GLIDERS_PER_GENERATION {
            if width <= GLIDER_SIZE || height <= GLIDER_SIZE {
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

                let survives = if alive {
                    live_neighbors == 2 || live_neighbors == 3
                } else {
                    live_neighbors == 3
                };

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

        let mut cells = HashMap::new();
        for _ in 0..options.initial_cells {
            let x = rng.random_range(0..screen_size.0) as usize;
            let y = rng.random_range(0..screen_size.1) as usize;

            cells.insert((x, y), LifeCell::new());
        }

        Self {
            screen_size,
            options,
            canvas,
            cells,
            rng,
            generation_accumulator: 0.0,
        }
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

    /// A glyph that says how long a cell has been alive.
    ///
    /// The glyph used to be `rng.random_range(0..32)` into a table of halfwidth
    /// katakana, redrawn for every surviving cell on every generation and
    /// depending on nothing at all -- not the cell's age, not its neighbours,
    /// not where it is. Nothing about a cell's appearance changed while the cell
    /// itself was standing still, so the only thing the eye could follow was the
    /// glyphs vanishing, which is what "just some characters moving around"
    /// describes.
    ///
    /// Stated as *bands* rather than as "two different ages get two different
    /// glyphs". A seven-step ramp over thirty-three ages has to give several ages
    /// the same glyph, and the property worth pinning is that each glyph owns one
    /// contiguous run of ages and that the runs tile the range in order -- which a
    /// constant mapping, a per-generation redraw, and a shuffled ramp all fail,
    /// and a ramp indexed by anything monotonic in the age passes.
    #[test]
    fn a_live_cells_glyph_depends_on_how_long_it_has_been_alive() {
        let mut bands: Vec<(char, u8, u8)> = Vec::new();
        for age in 0..=MAX_AGE {
            let glyph = glyph_for_age(age);
            match bands.last_mut() {
                Some((last, _, hi)) if *last == glyph => *hi = age,
                _ => bands.push((glyph, age, age)),
            }
        }

        assert_eq!(
            bands.len(),
            AGE_GLYPHS.len(),
            "the ramp has {} entries but the ages resolve to {} distinct \
             glyphs, so some of the ramp is unreachable",
            AGE_GLYPHS.len(),
            bands.len()
        );
        let mut expected = 0;
        for (index, (glyph, youngest, oldest)) in bands.iter().enumerate() {
            assert_eq!(
                *youngest, expected,
                "the glyph {glyph:?} starts at age {youngest}, not {expected}, so \
                 the ages it covers are not contiguous"
            );
            assert_eq!(
                glyph,
                &AGE_GLYPHS.at(index),
                "the {index}th band is {glyph:?} but the ramp's {index}th entry is \
                 {:?}, so the glyph is not rising with the age",
                AGE_GLYPHS.at(index)
            );
            expected = oldest + 1;
        }
        assert_eq!(expected, MAX_AGE + 1, "the bands do not cover every age");

        // And the two ends really do come out different on a live population, so
        // the ramp is reaching the screen rather than only the table.
        let life = draw_a_settled_population();
        assert_ne!(
            glyph_for_age(0),
            glyph_for_age(MAX_AGE),
            "a newborn and a settled cell are drawn with the same glyph"
        );
        let drawn: HashSet<char> = (0..=MAX_AGE).map(glyph_for_age).collect();
        assert!(
            life.cells
                .values()
                .all(|cell| drawn.contains(&glyph_for_age(cell.age))),
            "a live cell was drawn with a glyph that is not in the ramp"
        );
    }

    /// The colour is per-cell, not one global counter for the whole screen.
    ///
    /// It used to be `255 - current_gen` with `current_gen` a global counter, so
    /// every live cell was the same green and the entire screen pulsed together
    /// in brightness once every 255 generations -- about 32 seconds at the
    /// default eight generations a second. Two cells of clearly different ages
    /// have to come out clearly different, or the colour is still a clock.
    #[test]
    fn a_live_cells_colour_depends_on_its_age_rather_than_a_global_clock() {
        let life = draw_a_settled_population();

        let colours: HashSet<style::Color> = life
            .cells
            .values()
            .map(|cell| color_for_age(cell.age))
            .collect();
        assert!(
            colours.len() > 1,
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

    /// The ramp is ASCII, so it cannot shear a cell-indexed grid.
    ///
    /// The old set was thirty-two halfwidth katakana, U+FF8A and neighbours.
    /// Halfwidth katakana occupy one column in a Latin-configured terminal and
    /// two in a CJK-configured one, so the same run of live cells came out one
    /// column wide in one terminal and two in the next. Whether a glyph is
    /// double-width is not something this crate can decide without a width
    /// table, which is the other reason the replacement is ASCII: it is the one
    /// range guaranteed single-width everywhere.
    #[test]
    fn the_glyph_ramp_is_ascii_and_free_of_control_codes() {
        for age in 0..=MAX_AGE {
            let glyph = glyph_for_age(age);
            assert!(
                glyph.is_ascii(),
                "age {age} draws {glyph:?} (U+{:04X}), which is not ASCII",
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
        for glyph in AGE_GLYPHS.glyphs() {
            assert!(
                glyph.is_ascii() && !glyph.is_control() && *glyph != ' ',
                "the ramp itself contains {glyph:?} (U+{:04X})",
                *glyph as u32
            );
        }
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
