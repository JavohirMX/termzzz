use crate::buffer::{Buffer, Cell};
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static LINE_CHARS: LazyLock<Vec<Vec<char>>> = LazyLock::new(|| {
    vec![
        // Line type 0 - Regular box drawing
        vec![' ', '│', '│', '─', '─', '┌', '┐', '└', '┘'],
        // Line type 1 - Bold box drawing
        vec![' ', '┃', '┃', '━', '━', '┏', '┓', '┗', '┛'],
        // Line type 2 - Curved corners
        vec![' ', '│', '│', '─', '─', '╭', '╮', '╰', '╯'],
        // Line type 3 - Double characters
        vec![' ', '║', '║', '═', '═', '╔', '╗', '╚', '╝'],
        // Line type 4 - Block characters
        vec![' ', '█', '█', '▄', '▄', '▛', '▜', '▙', '▟'],
        // Line type 5 - Heavy rounded
        vec![' ', '┇', '┇', '┅', '┅', '┏', '┓', '┗', '┛'],
        // Line type 6 - Braille patterns
        vec![' ', '⠿', '⠿', '⠉', '⠉', '⠏', '⠕', '⠧', '⠽'],
    ]
});

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipesOptions {
    pub turn_probability: f64,
    pub line_type: usize,
    pub num_lines: usize,
    pub pipe_type_change: f64,
    pub cleanup_factor: f64,
    /// Seed for pipe placement, turning, colour and line style.
    pub seed: u64,
}

impl Default for PipesOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            turn_probability: 0.2,
            line_type: 2,
            num_lines: 3,
            pipe_type_change: 0.3,
            cleanup_factor: 0.9,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Pipe {
    line_type: usize,
    turn_probability: f64,
    prev_location: (usize, usize),
    prev_node_type: usize,
    next_location: (usize, usize),
    curr_color: style::Color,
    pub colors: Vec<style::Color>,
    pub rng: EffectRng,
}

pub struct Pipes {
    pub screen_size: (u16, u16),
    options: PipesOptions,
    /// Last frame handed to the terminal.
    buffer: Buffer,
    /// The frame being built. Pipes are drawn here during `update` and only
    /// diffed against `buffer` during `get_diff`.
    frame: Buffer,
    pipes_made: bool,
    pipes: Vec<Pipe>,
    /// Fraction of a pipe step not yet spent.
    step_accumulator: f32,
}

impl TerminalEffect for Pipes {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // Pure rendering. The pipes were advanced during `update`, so all that
        // happens here is to decide whether the frame is full enough to keep and
        // then hand over the cells that changed. Advancing from here meant the
        // pipes grew one cell per *rendered frame*, so they crawled on a slow
        // terminal and raced on a fast one.
        let curr_buffer = self.frame.clone();

        // Check if cleanup threshold has been reached by counting empty cells
        let total_cells = self.screen_size.0 as usize * self.screen_size.1 as usize;
        let mut empty_cells = 0;

        // Count empty cells in current buffer
        for y in 0..self.screen_size.1 as usize {
            for x in 0..self.screen_size.0 as usize {
                if curr_buffer.get(x, y).symbol == ' ' {
                    empty_cells += 1;
                }
            }
        }

        // Calculate empty space percentage
        let empty_percentage = empty_cells as f64 / total_cells as f64;

        // If empty space is less than (1 - cleanup_factor), reset
        if empty_percentage < (1.0 - self.options.cleanup_factor) {
            let diff = self.buffer.diff(&Buffer::new(
                self.screen_size.0 as usize,
                self.screen_size.1 as usize,
            ));
            self.reset();
            return diff;
        }

        let diff = self.buffer.diff(&curr_buffer);
        self.buffer = curr_buffer;

        diff
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall cannot draw a hundred cells in one frame.
        self.advance(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.reset();
    }

    fn reset(&mut self) {
        self.buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.frame =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.pipes_made = false;
        self.step_accumulator = 0.0;

        // This effect does not rebuild itself through `new`, so the generators
        // are refreshed here. Without it a resize would leave every pipe
        // carrying on from wherever its sequence had reached.
        for (index, pipe) in self.pipes.iter_mut().enumerate() {
            pipe.rng = seeded_rng(self.options.seed ^ index as u64, "pipes");
        }
    }
}

impl Pipe {
    fn start_new_pipe(
        &mut self,
        buffer: &mut Buffer,
        width: usize,
        height: usize,
        pipe_type_change: f64,
    ) {
        if self.rng.random_bool(pipe_type_change) {
            self.line_type = self.rng.random_range(0..LINE_CHARS.len());
        }

        let edge = self.rng.random_range(0..4);

        let (pos, direction) = match edge {
            0 => {
                // Top edge
                let x = self.rng.random_range(0..width);
                ((x, 0), (0, 1))
            }
            1 => {
                // Right edge
                let y = self.rng.random_range(0..height);
                ((width - 1, y), (-1, 0))
            }
            2 => {
                // Bottom edge
                let x = self.rng.random_range(0..width);
                ((x, height - 1), (0, -1))
            }
            3 => {
                // Left edge
                let y = self.rng.random_range(0..height);
                ((0, y), (1, 0))
            }
            _ => unreachable!(),
        };

        // Choose a random color
        self.curr_color = self.colors[self.rng.random_range(0..self.colors.len())];

        let node_type = match direction {
            (0, 1) | (0, -1) => 1, // Vertical
            (1, 0) | (-1, 0) => 3, // Horizontal
            _ => unreachable!(),
        };

        // Set initial node
        buffer.set(
            pos.0,
            pos.1,
            Cell::new(
                self.get_line_char(node_type),
                self.curr_color,
                style::Attribute::Bold,
            ),
        );

        self.prev_location = pos;
        self.prev_node_type = node_type;

        self.next_location = (
            (pos.0 as i32 + direction.0) as usize,
            (pos.1 as i32 + direction.1) as usize,
        );
    }

    fn continue_pipe(
        &mut self,
        buffer: &mut Buffer,
        width: usize,
        height: usize,
        pipe_type_change: f64,
    ) -> bool {
        // Check if reaches edge
        if self.next_location.0 >= width || self.next_location.1 >= height {
            self.start_new_pipe(buffer, width, height, pipe_type_change);
            return true;
        }

        let current_dir = self.get_direction();

        let turn = self.rng.random_range(0.0..1.0) < self.turn_probability;

        let (next_dir, node_type) = if turn {
            self.get_turn_direction_and_node(current_dir)
        } else {
            (
                current_dir,
                match current_dir {
                    (0, 1) | (0, -1) => 1, // Vertical
                    (1, 0) | (-1, 0) => 3, // Horizontal
                    _ => unreachable!(),
                },
            )
        };

        buffer.set(
            self.next_location.0,
            self.next_location.1,
            Cell::new(
                self.get_line_char(node_type),
                self.curr_color,
                style::Attribute::Bold,
            ),
        );

        // Update state for next iteration
        self.prev_location = self.next_location;
        self.prev_node_type = node_type;

        self.next_location = (
            (self.next_location.0 as i32 + next_dir.0) as usize,
            (self.next_location.1 as i32 + next_dir.1) as usize,
        );
        false
    }

    fn get_line_char(&self, node_type: usize) -> char {
        // default line_type to 0
        let line_type = if self.line_type < LINE_CHARS.len() {
            self.line_type
        } else {
            0
        };

        LINE_CHARS[line_type].get(node_type).copied().unwrap_or('?')
    }

    // Get the current direction based on previous node and location
    fn get_direction(&self) -> (i32, i32) {
        match self.prev_node_type {
            1 | 2 => {
                // Vertical pipe
                if self.next_location.1 > self.prev_location.1 {
                    (0, 1) // Down
                } else {
                    (0, -1) // Up
                }
            }
            3 | 4 => {
                // Horizontal pipe
                if self.next_location.0 > self.prev_location.0 {
                    (1, 0) // Right
                } else {
                    (-1, 0) // Left
                }
            }
            5 => {
                // ┏ Top-left corner
                if self.next_location.0 > self.prev_location.0 {
                    (1, 0) // Right
                } else {
                    (0, 1) // Down
                }
            }
            6 => {
                // ┓ Top-right corner
                if self.next_location.0 < self.prev_location.0 {
                    (-1, 0) // Left
                } else {
                    (0, 1) // Down
                }
            }
            7 => {
                // ┗ Bottom-left corner
                if self.next_location.0 > self.prev_location.0 {
                    (1, 0) // Right
                } else {
                    (0, -1) // Up
                }
            }
            8 => {
                // ┛ Bottom-right corner
                if self.next_location.0 < self.prev_location.0 {
                    (-1, 0) // Left
                } else {
                    (0, -1) // Up
                }
            }
            _ => (0, 0), // Nope
        }
    }

    // Get a new direction and node type when turning
    fn get_turn_direction_and_node(
        &mut self,
        current_dir: (i32, i32),
    ) -> ((i32, i32), usize) {
        match current_dir {
            (0, 1) => {
                // Moving down
                if self.rng.random_bool(0.5) {
                    ((1, 0), 7) // Turn right -> ┗
                } else {
                    ((-1, 0), 8) // Turn left -> ┛
                }
            }
            (0, -1) => {
                // Moving up
                if self.rng.random_bool(0.5) {
                    ((1, 0), 5) // Turn right -> ┏
                } else {
                    ((-1, 0), 6) // Turn left -> ┓
                }
            }
            (1, 0) => {
                // Moving right
                if self.rng.random_bool(0.5) {
                    ((0, 1), 6) // Turn down -> ┓
                } else {
                    ((0, -1), 8) // Turn up -> ┛
                }
            }
            (-1, 0) => {
                // Moving left
                if self.rng.random_bool(0.5) {
                    ((0, 1), 5) // Turn down -> ┏
                } else {
                    ((0, -1), 7) // Turn up -> ┗
                }
            }
            _ => ((0, 0), 1), // Nope
        }
    }
}

/// Seconds of progress per pipe cell, i.e. how fast the pipes grow.
const PIPE_STEP: f64 = 1.0 / 60.0;
/// Upper bound on cells grown in one frame.
const MAX_STEPS_PER_FRAME: usize = 8;

impl Pipes {
    /// Grows the pipes by `seconds` worth of progress, in whole cells.
    fn advance(&mut self, seconds: f64) {
        self.step_accumulator += (seconds / PIPE_STEP) as f32;
        let budget = self.step_accumulator.floor();
        self.step_accumulator -= budget;
        let mut steps = (budget as usize).min(MAX_STEPS_PER_FRAME);
        if steps == 0 {
            return;
        }
        // If the budget was clipped, drop the remainder rather than carrying it
        // forward, so a long stall cannot leave a burst of catch-up steps queued
        // for the following frames.
        if budget as usize > MAX_STEPS_PER_FRAME {
            self.step_accumulator = 0.0;
        }
        while steps > 0 {
            if self.pipes_made {
                Self::continue_pipes(
                    &mut self.pipes,
                    &mut self.frame,
                    &self.options,
                    self.screen_size,
                );
            } else {
                Self::start_new_pipes(
                    &mut self.pipes,
                    &mut self.frame,
                    &self.options,
                    self.screen_size,
                    &mut self.pipes_made,
                );
            }
            steps -= 1;
        }
    }

    pub fn new(options: PipesOptions, screen_size: (u16, u16)) -> Self {
        let buffer = Buffer::new(screen_size.0 as usize, screen_size.1 as usize);
        let colors = vec![
            style::Color::Red,
            style::Color::Green,
            style::Color::Blue,
            style::Color::Yellow,
            style::Color::Cyan,
            style::Color::Magenta,
        ];

        let mut pipes = Vec::with_capacity(options.num_lines);
        // The index is folded into each pipe's seed. Giving them all the same
        // generator would make every pipe draw the identical sequence, so they
        // would turn, recolour and restart on the same frame — the animation
        // would look like one pipe, not several.
        for index in 0..options.num_lines {
            pipes.push(Pipe {
                line_type: options.line_type,
                turn_probability: options.turn_probability,
                prev_location: (0, 0),
                prev_node_type: 0,
                next_location: (0, 0),
                curr_color: style::Color::White,
                colors: colors.clone(),
                rng: seeded_rng(options.seed ^ index as u64, "pipes"),
            });
        }

        Self {
            screen_size,
            options,
            frame: buffer.clone(),
            buffer,
            pipes_made: false,
            step_accumulator: 0.0,
            pipes,
        }
    }

    /// Starts each pipe at a random point on an edge.
    ///
    /// Takes the pieces rather than `&mut self` so the caller can pass the pipe
    /// list and the frame it draws into as two disjoint borrows.
    fn start_new_pipes(
        pipes: &mut [Pipe],
        frame: &mut Buffer,
        options: &PipesOptions,
        screen_size: (u16, u16),
        pipes_made: &mut bool,
    ) {
        let (width, height) = (screen_size.0 as usize, screen_size.1 as usize);

        for pipe in pipes.iter_mut() {
            pipe.start_new_pipe(frame, width, height, options.pipe_type_change);
        }

        *pipes_made = true;
    }

    // Continue an existing pipe
    /// Grows every pipe by one cell.
    fn continue_pipes(
        pipes: &mut [Pipe],
        frame: &mut Buffer,
        options: &PipesOptions,
        screen_size: (u16, u16),
    ) {
        let (width, height) = (screen_size.0 as usize, screen_size.1 as usize);

        for pipe in pipes.iter_mut() {
            pipe.continue_pipe(frame, width, height, options.pipe_type_change);
        }
    }
}
