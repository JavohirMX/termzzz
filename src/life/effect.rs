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
///
/// Inclusive, and that is the other half of the arithmetic. A cell at the top of
/// the range is saturating, so the ages this simulation actually produces are 0
/// through 8 -- nine of them, not eight -- and dividing by 8 rather than 9 is
/// what leaves the dense end of the ramp one generation short of being
/// reachable.
const MAX_AGE: u8 = 8;

/// How many glyphs a live cell is drawn with, whatever its age.
///
/// Three, and the number *is* the fix rather than a setting. The glyph used to
/// be a continuous function of the age, which meant every live cell's character
/// changed on every single generation -- at the old rate of eight generations a
/// second, that is every cell on the screen changing character eight times a
/// second, and the report was that the effect "doesn't look like a real game of
/// life because every frame the characters change".
///
/// Three bands of three generations each, so every glyph is held for three
/// generations whatever the cell's age, rather than one age per glyph for the
/// ages that are common and a shared glyph for the rest. That uniformity is the
/// part that matters: an age-quantised ramp whose bands are sized by how many
/// cells fall in them would give the newborn band -- which is where 92% of a
/// life soup lives -- the shortest hold of all.
///
/// See [`band_width`] for why the bands divide the range evenly rather than
/// following [`age_fraction`].
const AGE_BANDS: u8 = 3;

/// Generations that share one glyph.
///
/// `(MAX_AGE + 1) / AGE_BANDS`: three here, and the division is the point. The
/// ages that exist are 0 through [`MAX_AGE`] *inclusive*, so there are nine of
/// them and nine divides into three bands of three with nothing left over --
/// which is what keeps the dense end of the ramp reachable, since a still life
/// has to land in the last band and the last band has to be non-empty.
///
/// Deliberately not derived from [`age_fraction`]. Sizing the bands by where the
/// cells are would be the obvious thing, and it is wrong for this complaint: the
/// curve spends almost all of its range on the first three ages, so those bands
/// come out two, two and four generations wide, and a cell at the sparse end --
/// which is 92% of them, and the only band most viewers will ever look at --
/// changes its character every second generation.
fn band_width() -> u8 {
    (MAX_AGE + 1) / AGE_BANDS
}

/// Which of [`AGE_BANDS`] glyphs a cell of this age is drawn with.
///
/// Saturating rather than wrapping, like [`age_fraction`]: a cell that outlives
/// [`MAX_AGE`] is in the last band and not back at the sparse end.
fn age_band(age: u8) -> usize {
    usize::from(age.min(MAX_AGE) / band_width())
}

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
/// happens, and pretending otherwise would be the kind of comment that outlives
/// its code: the glyph is banded by [`age_band`] now, because a continuous ramp
/// meant every live cell changed character on every generation. See
/// [`glyph_for_age`]. The curve stayed, and it is now only the colour's -- which
/// is the right division of the work, because a colour can change every
/// generation without the picture flickering, and a character cannot.
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
/// A pure function of the age, banded rather than continuous. It used to be
/// `rng.random_range(0..32)` into a character table, redrawn for every surviving
/// cell on every generation and depending on nothing: not the cell's age, not
/// its neighbours, not where it is. A cell standing still was redrawn in a
/// different character sixty times a second, so the only thing the eye could
/// follow was characters disappearing, and the effect read as noise moving
/// around rather than as a population with a history.
///
/// Fixing that was not the end of it. Deriving the glyph from the age made it
/// *stable*, which was the point, and it also made it change on every single
/// generation -- the age of a surviving cell is its age plus one, and with a
/// continuous ramp that is a different character, so every live cell on the
/// screen was redrawn eight times a second and the report came back that the
/// effect still "doesn't look like a real game of life because every frame the
/// characters change". A second fix, not the same one twice.
///
/// So the age is quantised into [`AGE_BANDS`] bands first, and the bands are
/// spread across the *whole* ramp rather than taken from its front: a settled
/// cell is the densest glyph there is, and a still life drawn at the sparse end
/// would be a field of births, which is the one reading that is definitely
/// wrong. The cost is that four of the seven glyphs are now unreachable, which
/// `a_live_cells_glyph_depends_on_how_long_it_has_been_alive` now asserts as
/// deliberate rather than as a bug -- and the trade is worth it, because a glyph
/// that changes is noticed and a glyph that does not is not.
fn glyph_for_age(age: u8) -> char {
    let band = age_band(age);
    AGE_GLYPHS.at(band * (AGE_GLYPHS.len() - 1) / (AGE_BANDS as usize - 1))
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
    /// Generations a second, and how fast the population evolves.
    ///
    /// 3.0, down from 8.0, and the other half of that is the glyph: at eight a
    /// second every live cell's character changed on every generation, and a
    /// generation is a third of a second now rather than an eighth. Neither half
    /// works on its own -- banding the glyph at eight generations a second holds a
    /// character for three and a half frames, and slowing the rate while the glyph
    /// is continuous still redraws the whole board every third of a second. See
    /// `glyph_for_age`.
    ///
    /// The rate is a reading of Conway's rules, not just a clock. At eight a
    /// second a glider crosses a cell every one and a half frames, a block is over
    /// in less than a twentieth of a second, and the board is a different pattern
    /// before any of it can be followed. A third of a second per generation is
    /// about the rate at which a person can watch a single cell and say what
    /// happened to it, which is the only thing a screensaver of Conway's rules has
    /// to offer.
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

    /// A glyph that says how long a cell has been alive, in bands wide enough to
    /// be read.
    ///
    /// The glyph used to be `rng.random_range(0..32)` into a table of halfwidth
    /// katakana, redrawn for every surviving cell on every generation and
    /// depending on nothing at all -- not the cell's age, not its neighbours,
    /// not where it is. Nothing about a cell's appearance changed while the cell
    /// itself was standing still, so the only thing the eye could follow was the
    /// glyphs vanishing, which is what "just some characters moving around"
    /// describes.
    ///
    /// Deriving it from the age fixed that and created the second half of the
    /// problem: the age of a surviving cell is its age plus one, so with a ramp
    /// indexed *continuously* in the age, every live cell on the screen changed
    /// character on every generation. Eight times a second, and the report came
    /// back that it still did not look like a game of life.
    ///
    /// So this now asserts the banded shape, and the first half of it is the part
    /// that was asserted before and is now deliberately inverted. It used to
    /// require the nine ages to resolve to all seven glyphs, with the reasoning
    /// that "some of the ramp is unreachable" is a bug. It is the *number* of
    /// glyphs that was the bug: spending seven glyphs on nine ages means one age
    /// per glyph, so a still life changes character every generation and four of
    /// the seven are decoration anyway. Fewer, wider bands is the fix, and four
    /// unreachable entries of the ramp is what the fix looks like. The
    /// contiguity and the ordering are the properties that still matter, and they
    /// are the ones a constant mapping, a per-generation redraw, and a shuffled
    /// ramp all fail.
    ///
    /// The lower bound on a band's width is the load-bearing half. Two
    /// generations is not a band a viewer can hold in their head at a rate where
    /// a generation is a third of a second, and `AGE_BANDS` is small enough that
    /// every band is three generations or more rather than only the common ones.
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

        // Fewer bands than the ramp has entries, and never more: that inversion
        // is the fix, and a ramp that is fully spent again is the bug returning.
        assert!(
            bands.len() <= AGE_GLYPHS.len(),
            "the ages resolve to {} distinct glyphs out of a {}-entry ramp, so the \
             glyph is continuous again and every live cell changes character on \
             every generation",
            bands.len(),
            AGE_GLYPHS.len()
        );
        assert_eq!(
            bands.len(),
            usize::from(AGE_BANDS),
            "{AGE_BANDS} bands are declared and the ages resolve to {}, so a band \
             is unreachable and the top of the ramp is not being drawn",
            bands.len()
        );

        let mut expected = 0;
        let mut previous_index = None;
        for (index, (glyph, youngest, oldest)) in bands.iter().enumerate() {
            assert_eq!(
                *youngest, expected,
                "the glyph {glyph:?} starts at age {youngest}, not {expected}, so \
                 the ages it covers are not contiguous"
            );
            // At least three generations per glyph, in *every* band. 92% of a
            // life soup is at most two generations old, so a band width that
            // held for the long-lived cells but not for the newborns would fix
            // the still lifes and leave the whole rest of the screen flickering.
            assert!(
                oldest - youngest + 1 >= 3,
                "band {index} ({glyph:?}) covers ages {youngest} to {oldest}, which \
                 is {} generations -- too few to read at a rate where a \
                 generation is a fraction of a second",
                oldest - youngest + 1
            );
            // Monotonic in the ramp, and the whole ramp's worth of it: first
            // band at the sparsest entry, last band at the densest. A still life
            // drawn at the sparse end is a field of births, which is the reading
            // that is definitely wrong.
            let ramp_index = AGE_GLYPHS
                .glyphs()
                .iter()
                .position(|candidate| candidate == glyph)
                .unwrap_or_else(|| panic!("{glyph:?} is not in the ramp"));
            if let Some(previous) = previous_index {
                assert!(
                    ramp_index > previous,
                    "band {index} is {glyph:?} at ramp index {ramp_index}, which \
                     is not above the previous band's {previous}, so the glyph is \
                     not rising with the age"
                );
            }
            previous_index = Some(ramp_index);
            expected = oldest + 1;
        }
        assert_eq!(expected, MAX_AGE + 1, "the bands do not cover every age");
        assert_eq!(bands.first().map(|b| b.0), Some(AGE_GLYPHS.at(0)));
        assert_eq!(
            bands.last().map(|b| b.0),
            Some(AGE_GLYPHS.at(AGE_GLYPHS.len() - 1)),
            "the oldest band is not the densest glyph in the ramp, so a still life \
             would be drawn as a newborn"
        );

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

    /// The bands divide the age range with nothing left over.
    ///
    /// Two things break if they do not, and neither of them is visible in the
    /// table above. A `(MAX_AGE + 1) / AGE_BANDS` that does not divide evenly
    /// leaves the last band shorter than the rest, or -- worse -- leaves it
    /// empty, and an empty last band means a still life is drawn one step short
    /// of the densest glyph in the ramp. And a band count above the ramp's length
    /// makes `glyph_for_age` ask the ramp for an index it does not have, which it
    /// would silently clamp, so two adjacent bands would come out the same
    /// character and the band count would be a lie.
    #[test]
    fn the_bands_divide_the_age_range_and_fit_in_the_ramp() {
        // The first two are `const _: () = assert!(..)` rather than `assert!` for
        // the reason the crab gain has the same shape: a comparison of two
        // constants is resolved at compile time, so an `assert!` over one is a
        // `true` the optimiser removes, which is a warning clippy is right to
        // raise and which cannot be read off the source. The third cannot be,
        // because `AGE_GLYPHS` is a lazily built ramp and its length is not a
        // constant expression.
        const _: () = assert!((MAX_AGE + 1) % AGE_BANDS == 0);
        const _: () = assert!(AGE_BANDS >= 2);

        let bands = usize::from(AGE_BANDS);
        assert!(
            bands <= AGE_GLYPHS.len(),
            "{AGE_BANDS} bands over a {}-entry ramp means at least two of them \
             land on the same character, so the band count is a lie",
            AGE_GLYPHS.len()
        );
        assert_eq!(band_width(), 3, "the bands are three generations wide");

        // Every band is non-empty, and the last one is the top of the ramp.
        for band in 0..bands {
            let ages: Vec<u8> =
                (0..=MAX_AGE).filter(|age| age_band(*age) == band).collect();
            assert_eq!(
                ages.len(),
                usize::from(band_width()),
                "band {band} covers ages {ages:?}, so the {} bands do not divide \
                 the {} ages evenly -- the last band would come out short, or \
                 empty, and a still life would be drawn short of the densest \
                 glyph in the ramp",
                AGE_BANDS,
                MAX_AGE + 1
            );
        }
    }

    /// A cell's character is held for about a second, which is the whole point
    /// of both halves of this change.
    ///
    /// The two are one fix and neither works alone. Banding the glyph while the
    /// simulation runs at eight generations a second holds a character for three
    /// frames and a half, and slowing the simulation to three generations a second
    /// while the glyph is continuous still changes every live cell's character
    /// every third of a second. The invariant worth pinning is the product: how
    /// long a character stays on screen.
    ///
    /// Stated as a duration rather than as either number on its own so that
    /// raising the rate and narrowing the bands together cannot pass, which is
    /// the change that would look like an improvement to a reader of either
    /// constant alone. A third of a second is a generation; three generations is
    /// a second.
    #[test]
    fn a_character_stays_on_screen_long_enough_to_read() {
        let seconds_per_glyph = f32::from(band_width())
            / ConwayLifeOptions::default().generations_per_second;
        assert!(
            seconds_per_glyph >= 0.75,
            "a character is held for {seconds_per_glyph:.2} seconds, so a cell \
             that is not moving still flickers"
        );
        assert!(
            ConwayLifeOptions::default().generations_per_second <= 4.0,
            "the population evolves {} times a second, so a glider crosses a cell \
             every {} frames and no single cell can be followed",
            ConwayLifeOptions::default().generations_per_second,
            60.0 / ConwayLifeOptions::default().generations_per_second
        );
    }

    /// A live cell that survives a generation usually keeps its character.
    ///
    /// The complaint, as a number rather than as an impression: "every frame the
    /// characters change". Measured over a settled soup, the fraction of cells
    /// that are live both before and after a generation *and* whose character
    /// changed across it. Before the glyph was banded, that was every one of
    /// them, every generation -- which is not a ramp reading a cell's age, it is
    /// noise, and no rate of generation makes it watchable because the character
    /// changes on each one.
    ///
    /// With the age banded, a character only changes when a cell's age crosses a
    /// band boundary, and at the age distribution [`MAX_AGE`] documents that is
    /// the 14% of survivors arriving at age three and the few per cent arriving
    /// at six. The threshold is a third rather than the measurement, so this is
    /// about the order of magnitude and not about which cell happened to be
    /// where.
    ///
    /// Only *survivors*, and that is the harder half of the question rather than
    /// the easier one: a cell that dies is not a cell whose character changed, it
    /// is a cell that is gone, and counting those in would let the number be
    /// dominated by the churn at the edge of a soup.
    #[test]
    fn most_cells_keep_their_character_from_one_generation_to_the_next() {
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
        let mut changed = 0usize;
        let mut survivors = 0usize;
        for _ in 0..100 {
            life.step_generation();
            for (cell, data) in &life.cells {
                let glyph = glyph_for_age(data.age);
                if let Some(before) = previous.get(cell) {
                    survivors += 1;
                    if before != &glyph {
                        changed += 1;
                    }
                }
            }
            previous = life
                .cells
                .iter()
                .map(|(cell, data)| (*cell, glyph_for_age(data.age)))
                .collect();
        }

        let fraction = changed as f64 / survivors.max(1) as f64;
        assert!(
            fraction < 1.0 / 3.0,
            "{:.0}% of the cells that survived a generation had their character \
             changed by it, which is the effect this was meant to stop",
            fraction * 100.0
        );
    }

    /// A cell's character is held for several generations *before* the ramp runs
    /// out -- which is the only version of that claim banding actually buys.
    ///
    /// The subtlety, and the reason this test exists in this shape. A ramp that
    /// saturates at the top already holds a character indefinitely for anything
    /// older than [`MAX_AGE`], so "some cell holds its character for a long time"
    /// is true of the *unbanded* code and proves nothing: measured against the
    /// old continuous ramp, the longest run in a settled soup is over thirty
    /// generations, entirely made of cells sitting at the top of the ramp doing
    /// nothing. A test written that way passes against the bug it is meant to
    /// catch.
    ///
    /// So the claim here is about the *young* end, where nearly all of the
    /// population is. 92% of a life soup is at most two generations old, so the
    /// first band is the band almost every cell is ever drawn from, and if it is
    /// not a band then the picture is still a flicker. The assertion is that
    /// some cell holds one character across all three of its first generations --
    /// which is precisely ages 0, 1 and 2, the first band, and a claim the
    /// continuous ramp cannot satisfy at any age below [`MAX_AGE`], because
    /// there each of those three ages is a different character.
    ///
    /// The cell is identified after the fact rather than placed in advance, and
    /// that is deliberate. A block is the obvious fixture -- the one structure
    /// Conway's rules guarantees -- and it does not survive this simulation:
    /// `step_generation` seeds nine gliders a generation at random positions, and
    /// a glider landing in the five-by-five box around a block is not rejected
    /// (only the cells it *overlaps* are), so it changes the block's neighbour
    /// counts and the block dies. A hand-placed cell in a soup is a coin flip as
    /// well, 44% of them being one generation old. So this runs the simulation,
    /// follows every cell, and reports the youngest age at which any cell held a
    /// character for three generations -- which is the number the banding is
    /// supposed to have brought down from eight.
    #[test]
    fn a_surviving_cell_keeps_its_character_for_several_generations() {
        const HELD: usize = 3;
        // The first band is ages 0 through this, and a cell that has reached the
        // top of it has been drawn with one character for `HELD` generations.
        let first_band_top = band_width() - 1;

        let options = ConwayLifeOptions {
            initial_cells: 220,
            cells_coeff: 1.0,
            ..Default::default()
        };
        let mut life = ConwayLife::new(options, (80, 40));
        for _ in 0..40 {
            life.step_generation();
        }

        // How many consecutive generations each cell has been live with its
        // character unchanged. A cell that dies drops out and its run ends with
        // it, which is the only honest way to count a run: a run across a death
        // is a run across two different cells.
        let mut run: HashMap<(usize, usize), usize> =
            life.cells.keys().map(|cell| (*cell, 1)).collect();
        let mut previous: HashMap<(usize, usize), char> = life
            .cells
            .iter()
            .map(|(cell, data)| (*cell, glyph_for_age(data.age)))
            .collect();
        // The youngest age at which a cell anywhere has completed such a run,
        // and the longest run seen at all -- the second is reported because a
        // run that only ever happens at the top of the ramp is the failure mode
        // this test exists to rule out, and it is worth seeing both numbers.
        let mut youngest = u8::MAX;
        let mut longest = 0usize;
        for _ in 0..120 {
            life.step_generation();
            for (cell, data) in &life.cells {
                let glyph = glyph_for_age(data.age);
                let held = match previous.get(cell) {
                    Some(before) if before == &glyph => {
                        run.get(cell).copied().unwrap_or(1) + 1
                    }
                    _ => 1,
                };
                run.insert(*cell, held);
                if held >= HELD {
                    youngest = youngest.min(data.age);
                }
                longest = longest.max(held);
            }
            run.retain(|cell, _| life.cells.contains_key(cell));
            previous = life
                .cells
                .iter()
                .map(|(cell, data)| (*cell, glyph_for_age(data.age)))
                .collect();
        }

        assert!(
            youngest <= first_band_top,
            "the youngest age at which any cell held its character for {HELD} \
             generations was {youngest}, so no cell holds a character before the \
             ramp saturates at {MAX_AGE} -- 92% of a life soup never gets that far \
             and is still being redrawn every generation. The longest run \
             anywhere was {longest}, which is the ramp sitting still at the top"
        );
    }

    /// The colour is per-cell, not one global counter for the whole screen.
    ///
    /// It used to be `255 - current_gen` with `current_gen` a global counter, so
    /// every live cell was the same green and the entire screen pulsed together
    /// in brightness once every 255 generations -- about 32 seconds at the rate
    /// this effect used to run at, and 85 at the one it runs at now. Two cells
    /// of clearly different ages have to come out clearly different, or the
    /// colour is still a clock.
    ///
    /// The colour is also the *continuous* half of the age ramp now, and that is
    /// deliberate rather than an oversight. The glyph is banded, because a
    /// character that changes is noticed, and a colour that changes every
    /// generation is not: a green that creeps a shade brighter as a structure
    /// settles reads as the structure settling, and the banding would have taken
    /// that away for the 92% of cells that live in the first band.
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
