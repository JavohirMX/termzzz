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
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Width and height of the box a seeded glider is rotated within.
const GLIDER_SIZE: usize = 3;
/// How many gliders are seeded into each generation.
const GLIDERS_PER_GENERATION: usize = 9;

static DEAD_CELLS_CHARS: LazyLock<Vec<char>> = LazyLock::new(|| {
    let characters = "ﾊﾐﾋｰｳｼﾅﾓﾆｻﾜﾂｵﾘｱﾎﾃﾏｹﾒｴｶｷﾑﾕﾗｾﾈｽﾀﾇﾍ";
    let char_vec: Vec<char> = characters.chars().collect();
    char_vec
});

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConwayLifeOptions {
    #[serde(skip)]
    pub initial_cells: u32,
    pub cells_coeff: f32,
    pub generations_per_second: f32,
    /// Seed for the initial population, the gliders seeded each generation, and
    /// the glyph each cell is drawn with.
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

#[derive(Clone)]
pub struct LifeCell {
    pub character: char,
    pub color: style::Color,
}

pub struct ConwayLife {
    pub screen_size: (u16, u16),
    #[allow(dead_code)]
    options: ConwayLifeOptions,
    canvas: Canvas,
    cells: HashMap<(usize, usize), LifeCell>,
    pub rng: EffectRng,
    pub current_gen: u8,
    generation_accumulator: f32,
}

impl LifeCell {
    pub fn new(character: char) -> Self {
        Self {
            character,
            color: style::Color::Rgb { r: 0, g: 255, b: 0 },
        }
    }

    pub fn update_color_and_char(&mut self, rng: &mut EffectRng, current_gen: u8) {
        let green_color = 255_u8.wrapping_sub(current_gen);
        match current_gen {
            0..=230 => {
                self.color = style::Color::Rgb {
                    r: 0,
                    g: green_color,
                    b: 0,
                }; // Green
                let random_index = rng.random_range(0..DEAD_CELLS_CHARS.len());
                self.character = *DEAD_CELLS_CHARS.get(random_index).unwrap();
            }
            _ => {
                self.color = style::Color::Rgb {
                    r: 0,
                    g: green_color,
                    b: 0,
                };
                let random_index = rng.random_range(0..DEAD_CELLS_CHARS.len());
                self.character = *DEAD_CELLS_CHARS.get(random_index).unwrap();
            }
        }
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
        self.current_gen = (self.current_gen + 1) % 255;

        let (width, height) = (self.canvas.width(), self.canvas.height());

        // Seed gliders into the generation we are about to evolve, so this
        // generation's rules apply to them. Seeding them afterwards, into the
        // already-computed result, means they are never subject to the rules at
        // all: they cannot age, cannot die, and cannot travel, because every
        // generation replaces them with fresh ones somewhere else.
        for _ in 0..GLIDERS_PER_GENERATION {
            if width <= GLIDER_SIZE || height <= GLIDER_SIZE {
                break;
            }
            let x = self.rng.random_range(2..width - GLIDER_SIZE + 1);
            let y = self.rng.random_range(2..height - GLIDER_SIZE + 1);
            let rotation = [0, 90, 180, 270][self.rng.random_range(0..4)];
            insert_glider(&mut self.cells, x, y, rotation, self.current_gen);
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

                let mut cell = match self.cells.get(&(x, y)) {
                    Some(existing) => existing.clone(),
                    None => LifeCell::new('*'),
                };
                cell.update_color_and_char(&mut self.rng, self.current_gen);
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
            let lc = LifeCell::new('*');
            let x = rng.random_range(0..screen_size.0) as usize;
            let y = rng.random_range(0..screen_size.1) as usize;

            cells.insert((x, y), lc);
        }

        Self {
            screen_size,
            options,
            canvas,
            cells,
            rng,
            current_gen: 0,
            generation_accumulator: 0.0,
        }
    }

    /// Writes every live cell into the canvas.
    pub fn fill_buffer(&mut self) {
        let (width, height) = (self.canvas.width(), self.canvas.height());
        for ((x, y), cell) in self.cells.iter() {
            if *x < width && *y < height {
                self.canvas.set(
                    *x,
                    *y,
                    Cell::new(cell.character, cell.color, style::Attribute::Bold),
                );
            }
        }
    }
}

fn insert_glider(
    cells: &mut HashMap<(usize, usize), LifeCell>,
    x: usize,
    y: usize,
    rotation: i32,
    current_gen: u8,
) {
    const BASE_GLIDER: [(usize, usize); 5] =
        [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

    // Every rotation maps the shape back into the same 3x3 box, so each arm has
    // to re-centre it. The 270-degree arm was missing that, which shifted a
    // quarter of all seeded gliders two columns off and left them malformed.
    let rotated_glider = BASE_GLIDER.iter().map(|&(dx, dy)| match rotation {
        0 => (x + dx, y + dy),
        90 => (x + dy, y + 2 - dx),
        180 => (x + 2 - dx, y + 2 - dy),
        270 => (x + 2 - dy, y + dx),
        _ => (x + dx, y + dy),
    });

    let green_color = 255_u8.wrapping_sub(current_gen);

    for (x, y) in rotated_glider {
        cells.insert(
            (x, y),
            LifeCell {
                character: '0',
                color: style::Color::Rgb {
                    r: 0,
                    g: green_color,
                    b: 0,
                },
            },
        );
    }
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
            cells.insert((0, y), LifeCell::new('*'));
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
                cells.insert((x, y), LifeCell::new('*'));
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
}
