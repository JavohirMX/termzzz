use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::render::glyph_ramp::{GlyphRamp, presets as glyph_presets};
use crate::render::palette::{Palette, presets as palette_presets};
use crate::runtime::FrameContext;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Flip count at which a cell is drawn at the top of the ramp.
///
/// Not a taste. The grid stores how many times a cell has been flipped, and an
/// ant revisits a cell about once every few hundred steps on its chaotic walk, so
/// over a session a cell can be flipped dozens of times. Without a cap the
/// histogram is dominated by cells flipped many times and the picture saturates
/// to the brightest colour within a minute, after which a new ant's trail is
/// invisible against the old ones -- which is the one thing this effect exists to
/// show. Saturating at eight keeps the newest trail legible against the busiest
/// region of the board.
const SATURATION: f32 = 8.0;

/// Direction indices, in the order a right turn advances them.
const NORTH: usize = 0;
const EAST: usize = 1;
const SOUTH: usize = 2;
const WEST: usize = 3;

/// One Langton's ant.
///
/// The rule is two lines and the whole effect: look at the cell you are on, turn
/// right if it is light and left if it is dark, flip it, step forward. A single
/// ant wanders apparently at random for about ten thousand steps and then falls
/// into a repeating diagonal highway, which it builds out of a mess it has no
/// way of seeing.
struct Ant {
    x: usize,
    y: usize,
    /// 0 = north, increasing clockwise. Kept as an index rather than a vector so
    /// "turn" is a modulo rather than four cases at every use.
    heading: usize,
}

impl Ant {
    fn turn_right(&mut self) {
        self.heading = (self.heading + 1) % 4;
    }

    fn turn_left(&mut self) {
        self.heading = (self.heading + 3) % 4;
    }

    /// One cell in the current heading, wrapping. The grid is a torus, so an ant
    /// leaving one edge reappears on the other rather than dying against a wall.
    fn step(&mut self, width: usize, height: usize) {
        match self.heading {
            NORTH => self.y = (self.y + height - 1) % height,
            EAST => self.x = (self.x + 1) % width,
            SOUTH => self.y = (self.y + 1) % height,
            WEST => self.x = (self.x + width - 1) % width,
            // Unreachable: a heading is only ever built from a `random_range(0..4)`
            // and then moved by `turn_right` or `turn_left`, both of which reduce
            // mod 4. Exhaustive rather than a `_` arm on purpose -- a new direction
            // would fail to compile here instead of silently walking the wrong way.
            _ => unreachable!("heading {} is not a direction", self.heading),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AntsOptions {
    /// How many ants share the board. Derived from the terminal size, so it is
    /// not read from the config file.
    #[serde(skip)]
    pub ants: u16,

    /// Multiplies the built-in ant count. See [`Ants::new`] for why this is not
    /// simply the count.
    pub ant_coeff: f32,

    /// Ant steps per second.
    ///
    /// One step is one cell, and the interesting part of this effect is the
    /// roughly ten thousand steps an ant spends wandering before its highway
    /// locks in. At one step per frame that is nearly three minutes of nothing
    /// happening on a 60 Hz terminal, so the default is several steps per frame
    /// and a highway forms in well under a minute.
    pub steps_per_second: f32,

    /// Ceiling on steps spent in one frame.
    ///
    /// A long stall hands this effect a large delta, and without a ceiling it
    /// would try to spend all of it at once. The clamp is what turns a hitch
    /// into a brief pause rather than a jump.
    pub max_steps_per_frame: u16,

    pub seed: u64,
}

impl Default for AntsOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file that
    /// omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            ants: DEFAULT_ANT_COUNT,
            ant_coeff: 1.0,
            steps_per_second: 240.0,
            max_steps_per_frame: 8,
            seed: DEFAULT_SEED,
        }
    }
}

/// Ants per square of screen area before the coefficient is applied.
///
/// The count is derived rather than configured because a fixed number is wrong
/// at both ends of the range: eight ants on a 6x6 terminal is a solid block of
/// overlapping trails, and one on a 400x200 is a single lonely line in a sea of
/// untouched cells. Divided by this and clamped in [`Config::get_ants_options`],
/// which is where every other effect's density arithmetic lives.
pub const ANT_DENSITY: f32 = 1.0 / 6000.0;

/// The floor and ceiling on [`AntsOptions::ants`].
///
/// The upper bound is the one that matters. Ants that share a cell flip it
/// within a step of each other and destroy each other's turn decisions, so past
/// about a dozen the board is neither chaotic nor a set of highways but a field
/// of noise, which is the same picture a lower density produces for free.
pub const ANTS_MIN_COUNT: u16 = 1;
pub const ANTS_MAX_COUNT: u16 = 12;

/// Used as the `ants` default so that a struct built without a screen size still
/// has a sensible number rather than a panic or a silent zero.
const DEFAULT_ANT_COUNT: u16 = 4;

pub struct Ants {
    screen_size: (u16, u16),
    options: AntsOptions,
    canvas: Canvas,
    /// How many times each cell has been flipped, saturating at 255.
    ///
    /// A `u8` count rather than a bit, because the age of a trail is what makes
    /// one readable against another: with a bit the board is a binary image of
    /// where ants have been, and a new ant walking through an old highway is
    /// invisible because both regions read the same.
    grid: Vec<u8>,
    colony: Vec<Ant>,
    /// Elapsed time not yet spent on whole ant steps.
    step_accumulator: f32,
    rng: EffectRng,
    /// Cached so the per-cell draw does not rebuild them.
    ramp: GlyphRamp,
    palette: Palette,
}

impl TerminalEffect for Ants {
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
        self.screen_size = (width, height);
        self.reset();
    }

    fn reset(&mut self) {
        let (width, height) = self.size();
        // A resize invalidates the frame on screen, so the canvas blanks its
        // baseline and the next commit repaints in full.
        self.canvas.resize(self.screen_size.0, self.screen_size.1);

        self.grid = vec![0u8; width * height];
        self.step_accumulator = 0.0;
        // Reseeded with the rest of the state this rebuilds, so a resize places
        // the colony afresh rather than continuing the old board.
        self.rng = seeded_rng(self.options.seed, "ants");
        self.spawn_colony();
    }
}

impl Ants {
    pub fn new(options: AntsOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = crate::common::normalize_effect_size(screen_size);
        let (width, height) = (screen_size.0 as usize, screen_size.1 as usize);

        let mut ants = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            options,
            grid: vec![0u8; width * height],
            colony: Vec::new(),
            step_accumulator: 0.0,
            rng: seeded_rng(DEFAULT_SEED, "ants"),
            ramp: GlyphRamp::new_ascii(glyph_presets::SHADE.chars().collect()),
            // `Magma` rather than `ember`: the trails are the *hot* thing in this
            // effect and the untouched board is the cold part, and only this ramp
            // puts a dark, near-black end at zero and a pale end at the top. A
            // ramp that ends in white would make the oldest trails the brightest
            // thing on screen, which is the ordering mistake the glyph ramp's own
            // docs warn about.
            palette: Palette::new(palette_presets::MAGMA.to_vec()),
        };
        ants.rng = seeded_rng(ants.options.seed, "ants");
        ants.spawn_colony();
        ants
    }

    /// The board's dimensions in cells, as `(width, height)`.
    pub fn grid_dimensions(&self) -> (usize, usize) {
        self.size()
    }

    /// How many times a cell has been flipped. Zero means untouched.
    pub fn flipped(&self, x: usize, y: usize) -> u8 {
        let (width, height) = self.size();
        if x >= width || y >= height {
            return 0;
        }
        self.grid[y * width + x]
    }

    /// Moves one step of the simulation, for a caller driving the model directly
    /// rather than through [`TerminalEffect`].
    ///
    /// The frame path reaches this through `advance`, which spends elapsed time in
    /// whole steps; the tests and the measurement example want to drive it a step
    /// at a time and count them.
    fn size(&self) -> (usize, usize) {
        (
            self.screen_size.0.max(1) as usize,
            self.screen_size.1.max(1) as usize,
        )
    }

    /// Places the colony at seeded positions and headings.
    ///
    /// Seeded rather than fixed, which is what keeps this effect out of the
    /// `SEED_INSENSITIVE` list in `tests/effect_contracts.rs`. A single ant on a
    /// fixed starting square is perfectly deterministic and would pass the
    /// reproducibility half of that test while failing the seed-sensitivity one --
    /// and that list `continue`s, so the two defects are easy to confuse.
    fn spawn_colony(&mut self) {
        let (width, height) = self.size();
        self.colony = (0..self.options.ants.max(1))
            .map(|_| Ant {
                x: self.rng.random_range(0..width),
                y: self.rng.random_range(0..height),
                heading: self.rng.random_range(0..4),
            })
            .collect();
    }

    /// Spends elapsed time on whole ant steps.
    fn advance(&mut self, delta: f32) {
        let rate = self.options.steps_per_second.max(0.0);
        if rate <= 0.0 {
            return;
        }
        self.step_accumulator += delta * rate;
        let cap = self.options.max_steps_per_frame.max(1) as usize;
        let mut steps = 0;
        while self.step_accumulator >= 1.0 && steps < cap {
            self.step_accumulator -= 1.0;
            self.step_once();
            steps += 1;
        }
        if steps == cap {
            self.step_accumulator = 0.0;
        }
    }

    /// One step for every ant, in the order they are listed.
    ///
    /// Sequential rather than simultaneous, so the ants genuinely share one
    /// board: an ant that walks onto a square a later ant is about to flip sees
    /// the old value. That coupling is the effect -- two ants building highways
    /// into each other's is the whole reason this is not a single ant.
    pub fn step_once(&mut self) {
        let (width, height) = self.size();
        for ant in &mut self.colony {
            let index = ant.y * width + ant.x;
            // **Parity, not "has this cell ever been touched".**
            //
            // Langton's ant *toggles* the cell it is standing on, so a cell
            // visited twice is white again and has to turn the ant the other way.
            // Testing the raw count as though "non-zero means black" is the
            // obvious shortcut and it is wrong from the ant's second lap of any
            // cell: measured, it pins the ant in a 4-by-4 block after sixteen
            // cells and leaves it there for ever, so the ant never wanders, never
            // builds its highway, and the effect is a static smudge.
            //
            // The count itself is still kept, because the renderer wants the *age*
            // of a trail rather than its colour: a cell flipped once and a cell
            // flipped nine times are both black, and the picture wants them to
            // look different so a fresh trail is legible against a busy one.
            if self.grid[index] % 2 == 0 {
                ant.turn_right();
            } else {
                ant.turn_left();
            }
            // Saturating add. An `+= 1` here would panic in debug on a cell
            // flipped 256 times, which Langton's ant reaches routinely once its
            // highway is retraced.
            self.grid[index] = self.grid[index].saturating_add(1);
            ant.step(width, height);
        }
    }

    fn draw(&mut self) {
        self.canvas.clear();
        let (width, height) = self.size();

        for y in 0..height {
            for x in 0..width {
                let flips = self.grid[y * width + x];
                if flips == 0 {
                    // Untouched, and drawn as the crate's blank rather than as a
                    // dimmed cell. A space in a ramp colour is not the same
                    // thing: it is a space the encoder has to emit a colour for,
                    // and the majority of the board is untouched for most of a
                    // run.
                    continue;
                }
                let t = (flips as f32 / SATURATION).min(1.0);
                let cell = Cell::new(
                    self.ramp.sample(t),
                    self.palette.sample(t),
                    style::Attribute::Reset,
                );
                self.canvas.set(x, y, cell);
            }
        }

        // The ants on top of their own trails, so the colony is visible as
        // agents rather than inferrable from where the bright cells are.
        for (i, ant) in self.colony.iter().enumerate() {
            let glyph = if i == 0 { '@' } else { 'o' };
            self.canvas.set(
                ant.x,
                ant.y,
                Cell::new(glyph, style::Color::White, style::Attribute::Bold),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small(seed: u64) -> Ants {
        // One ant: the multi-ant behaviour is tested separately, and a single ant
        // is the only version whose highway is a known, measurable object.
        Ants::new(
            AntsOptions {
                seed,
                ants: 1,
                ..AntsOptions::default()
            },
            (60, 20),
        )
    }

    /// The longest unbroken run of flipped cells along any of the four 45-degree
    /// diagonals.
    ///
    /// All four, because which one the highway runs along depends on the ant's
    /// starting heading: the highway advances one cell on each axis per step with
    /// signs set by the turn direction, and a seed that starts facing north
    /// produces a different diagonal from one that starts facing west. Searching
    /// one direction is a test that passes or fails on the seed rather than on the
    /// model.
    ///
    /// A *diagonal* specifically, not a row or a column, because that is the shape
    /// of the thing: a row of flipped cells is what an ant retracing its own path
    /// leaves, and the run would be short and stable. The highway's run is what
    /// keeps growing, and that is what the test below checks.
    fn longest_diagonal(ants: &Ants) -> usize {
        let (width, height) = ants.size();
        let mut best = 0usize;
        for (dx, dy) in [(1isize, 1isize), (-1, 1), (1, -1), (-1, -1)] {
            for x0 in 0..width as isize {
                for y0 in 0..height as isize {
                    let (mut x, mut y) = (x0, y0);
                    let mut run = 0usize;
                    while x >= 0
                        && y >= 0
                        && (x as usize) < width
                        && (y as usize) < height
                        && ants.grid[y as usize * width + x as usize] != 0
                    {
                        run += 1;
                        x += dx;
                        y += dy;
                    }
                    best = best.max(run);
                }
            }
        }
        best
    }

    #[test]
    fn a_highway_forms_and_then_keeps_extending() {
        // The direct assertion of the effect, and the one that caught the rule
        // being wrong. Two halves:
        //
        // 1. Growth. Langton's ant wanders for about ten thousand steps and then
        //    falls into a diagonal highway that it builds out of a mess it cannot
        //    see. So the run of flipped cells along its diagonal has to *keep
        //    growing* after that point. A length threshold alone would have passed
        //    against the broken rule, which pins the ant in a four-by-four block
        //    and leaves a diagonal run of four on the board for ever.
        // 2. It stays a highway rather than dissolving. Distinct cells plateau
        //    once the highway is retracing its own cells, so the run grows while
        //    the total does not; both are checked because either one alone is
        //    satisfiable by the wrong thing.
        let mut ants = Ants::new(
            AntsOptions {
                seed: 1,
                ants: 1,
                ..AntsOptions::default()
            },
            (200, 60),
        );

        for _ in 0..10_000 {
            ants.step_once();
        }
        let run_at_10k = longest_diagonal(&ants);
        let cells_at_10k = ants.grid.iter().filter(|v| **v != 0).count();

        for _ in 0..10_000 {
            ants.step_once();
        }
        let run_at_20k = longest_diagonal(&ants);
        let cells_at_20k = ants.grid.iter().filter(|v| **v != 0).count();

        assert!(
            run_at_10k >= 20,
            "the longest diagonal run was {run_at_10k} cells at ten thousand steps; \
             the chaotic phase ends at about ten thousand, so no highway had formed"
        );
        assert!(
            run_at_20k >= run_at_10k * 2,
            "the run went from {run_at_10k} to {run_at_20k} over the second ten \
             thousand steps: a highway keeps extending, so this is not one"
        );
        assert!(
            cells_at_20k < cells_at_10k * 3,
            "distinct flipped cells went from {cells_at_10k} to {cells_at_20k}; a \
             highway retraces the cells it has already made rather than spreading"
        );
    }

    #[test]
    fn a_cell_visited_twice_turns_the_ant_the_other_way() {
        // The regression test for the rule itself, and the narrowest possible
        // statement of it. Langton's ant *toggles* the cell, so parity is what
        // decides the turn. Reading the turn off "has this cell ever been touched"
        // instead is the obvious shortcut, it looks right, and it pins the ant in
        // place after sixteen cells -- so the test that catches it has to reach
        // past the first flip.
        let mut ants = small(3);
        let (width, _) = ants.size();
        let ant = &ants.colony[0];
        ants.grid[ant.y * width + ant.x] = 2;
        let start_heading = ant.heading;

        ants.step_once();

        assert_eq!(
            ants.colony[0].heading,
            (start_heading + 1) % 4,
            "a cell flipped twice is white again, so the ant must turn right"
        );
    }

    #[test]
    fn a_single_step_turns_flips_and_moves() {
        let mut ants = small(2);
        let (width, height) = ants.size();
        let ant = &ants.colony[0];
        let (start_x, start_y, start_heading) = (ant.x, ant.y, ant.heading);

        ants.step_once();

        let ant = &ants.colony[0];
        // The cell it left is flipped. Read at the *starting* coordinates, not
        // the ant's: it has already moved, so its own position is the cell it is
        // standing on now, which is a different cell and would assert nothing.
        assert_eq!(
            ants.grid[start_y * width + start_x],
            1,
            "the cell the ant was standing on was not flipped"
        );
        // It turned right, which is a quarter turn clockwise, and in this heading
        // order clockwise is +1.
        let expected = (start_heading + 1) % 4;
        assert_eq!(
            ant.heading, expected,
            "the ant did not turn right off a light cell"
        );
        // And it moved in the direction it is now facing.
        let (expected_x, expected_y) = match expected {
            NORTH => (start_x, (start_y + height - 1) % height),
            EAST => ((start_x + 1) % width, start_y),
            SOUTH => (start_x, (start_y + 1) % height),
            _ => ((start_x + width - 1) % width, start_y),
        };
        assert_eq!((ant.x, ant.y), (expected_x, expected_y));
    }

    #[test]
    fn a_dark_cell_makes_the_ant_turn_left() {
        let mut ants = small(4);
        let (width, _) = ants.size();
        let ant = &ants.colony[0];
        ants.grid[ant.y * width + ant.x] = 1;
        let start_heading = ant.heading;

        ants.step_once();

        assert_eq!(
            ants.colony[0].heading,
            (start_heading + 3) % 4,
            "the ant did not turn left off a dark cell"
        );
    }

    #[test]
    fn the_grid_wraps_rather_than_killing_the_ant_at_an_edge() {
        // On the move itself rather than through a whole step, because a step turns
        // the ant first: setting a heading and expecting the ant to leave in that
        // direction tests the turn, not the wrap, and the two failures look alike.
        let (width, height) = (20usize, 10usize);
        for (from, heading, expected) in [
            // Off the bottom edge going south, and off the top going north.
            ((width - 1, height - 1), SOUTH, (width - 1, 0)),
            ((width - 1, 0), NORTH, (width - 1, height - 1)),
            // Off the left edge going west, and off the right going east.
            ((0, 5), WEST, (width - 1, 5)),
            ((width - 1, 5), EAST, (0, 5)),
            // And a corner, where both axes wrap at once.
            ((0, 0), NORTH, (0, height - 1)),
        ] {
            let mut ant = Ant {
                x: from.0,
                y: from.1,
                heading,
            };
            ant.step(width, height);
            assert_eq!(
                (ant.x, ant.y),
                expected,
                "heading {heading} from {from:?} left the grid"
            );
        }
    }

    #[test]
    fn an_ant_survives_a_long_run_on_the_smallest_board() {
        // The wrap test above says the arithmetic is right; this says the effect
        // does not walk its colony off a 6x6 terminal over a full playlist length.
        let mut ants = small(6);
        for _ in 0..20_000 {
            ants.step_once();
        }
        let (width, height) = ants.size();
        for ant in &ants.colony {
            assert!(ant.x < width && ant.y < height);
        }
    }

    #[test]
    fn flip_counts_saturate_instead_of_overflowing() {
        // An `+= 1` panics in debug at 255, and a cell flipped 256 times is
        // routine once an ant is retracing its own highway.
        let mut ants = small(5);
        let (width, _) = ants.size();
        let ant = &ants.colony[0];
        ants.grid[ant.y * width + ant.x] = 255;
        let index = ant.y * width + ant.x;

        ants.step_once();

        assert_eq!(
            ants.grid[index], 255,
            "a saturated cell overflowed rather than clamping"
        );
    }

    #[test]
    fn more_than_one_ant_changes_the_picture() {
        // The reason this effect is not a single ant: ants that share a board
        // read each other's flips, so the same seed with two ants must not look
        // like one ant plus a ghost.
        let mut one = Ants::new(
            AntsOptions {
                seed: 11,
                ants: 1,
                ..AntsOptions::default()
            },
            (60, 20),
        );
        let mut many = Ants::new(
            AntsOptions {
                seed: 11,
                ants: 6,
                ..AntsOptions::default()
            },
            (60, 20),
        );
        for _ in 0..2_000 {
            one.step_once();
            many.step_once();
        }
        assert_ne!(
            one.grid, many.grid,
            "adding ants made no difference, so they are not sharing the board"
        );
    }

    #[test]
    fn the_seeded_colony_places_its_ants_differently() {
        let a = small(100);
        let b = small(101);
        let positions = |ants: &Ants| {
            ants.colony
                .iter()
                .map(|a| (a.x, a.y, a.heading))
                .collect::<Vec<_>>()
        };
        assert_ne!(positions(&a), positions(&b));
    }

    #[test]
    fn a_zero_step_rate_freezes_the_colony_rather_than_spinning_it() {
        let mut ants = Ants::new(
            AntsOptions {
                ants: 1,
                steps_per_second: 0.0,
                ..AntsOptions::default()
            },
            (40, 12),
        );
        let before: Vec<u8> = ants.grid.clone();

        ants.advance(1.0);
        ants.advance(1.0);

        assert_eq!(ants.grid, before, "the board changed with the rate at zero");
    }

    #[test]
    fn zero_requested_ants_still_puts_one_on_the_board() {
        // The effect's own floor. The derivation from the screen area and its
        // ceiling are `Config`'s job and are tested there; what belongs here is
        // that an options struct saying zero produces a colony rather than an
        // empty vector and a screen that never changes.
        let ants = Ants::new(
            AntsOptions {
                ants: 0,
                ..AntsOptions::default()
            },
            (40, 12),
        );
        assert_eq!(ants.colony.len(), 1);
    }

    #[test]
    fn every_emitted_cell_is_inside_the_canvas() {
        let mut ants = small(9);
        for _ in 0..300 {
            ants.step_once();
            ants.draw();
        }
        for (x, y, _) in ants.canvas.commit() {
            assert!(x < ants.screen_size.0 as usize);
            assert!(y < ants.screen_size.1 as usize);
        }
    }
}
