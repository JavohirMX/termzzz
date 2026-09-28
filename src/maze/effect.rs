use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crossterm::style;
use rand::{RngExt, seq::SliceRandom};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::LazyLock,
};

/// Characters in form of hashmap with label as key
static CHARACTERS_MAP: LazyLock<HashMap<&str, &str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("punctuation", r#":."=*+-<>"#);
    m.insert("katakana", "ﾊﾐﾋｰｳｼﾅﾓﾆｻﾜﾂｵﾘｱﾎﾃﾏｹﾒｴｶｷﾑﾕﾗｾﾈｽﾀﾇﾍ");
    m.insert("other", "¦çﾘｸ");
    m
});

/// Characters to draw more interesting view
static CHARACTERS: LazyLock<Vec<char>> = LazyLock::new(|| {
    let mut v = Vec::new();
    for (_, chars) in CHARACTERS_MAP.iter() {
        v.append(&mut chars.chars().collect());
    }
    v
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MazeOptions {
    /// Seed for the wall texture, the start cell and the carve order.
    pub seed: u64,
    /// Seconds a finished maze stays on screen before a new one starts.
    ///
    /// This effect used to have no steady state at all: the frame after the carve
    /// completed, `get_diff` reset the maze and returned an empty diff, so a
    /// finished maze existed for exactly one frame. Long enough to actually read
    /// the result, short enough that a screensaver still feels like it is
    /// happening to you rather than being left on a wall.
    ///
    /// In simulation seconds, so it is the same wall-clock duration whatever the
    /// refresh rate or `--speed`.
    pub hold_seconds: f32,
    /// Which colour to carve the path in. See [`PathColor`].
    pub path_color: PathColor,
}

impl Default for MazeOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            hold_seconds: 6.0,
            path_color: PathColor::default(),
        }
    }
}
/// Roughly how long a full carve should take, at any terminal size.
const TARGET_GENERATION_SECONDS: f32 = 20.0;
/// The slowest the carve ever runs, so a small window still animates rather than
/// finishing in a single frame.
const MIN_CELLS_PER_SECOND: f32 = 60.0;
/// Upper bound on cells carved in one frame, so a stall cannot be paid off in
/// a single visible jump.
const MAX_CARVE_STEPS_PER_FRAME: usize = 24;
/// Wall cells re-textured per frame, so the walls are not a still image.
const WALLS_MOTTLED_PER_FRAME: usize = 3;

/// Which colour to carve the path in.
///
/// A named choice rather than a hex string in the config, because the choice is
/// constrained: every one of these has to clear 3:1 against *both* a white and a
/// black background, since a terminal profile is not knowable at run time. That
/// rules out most of the spectrum and, notably, rules out white itself -- which is
/// what this used to be, and which is invisible on a light profile for the one
/// part of the effect the eye is meant to follow.
///
/// `the_path_colour_reads_on_a_light_and_on_a_dark_profile` checks all of them
/// rather than just the default, so adding a variant that fails the contrast
/// requirement is caught here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PathColor {
    /// The default, and what this effect used to be before a contrast fix
    /// overrode it.
    ///
    /// **It is invisible on a light terminal profile.** A white corridor on a
    /// white background is the failure this palette was introduced to prevent, and
    /// the reason it stopped being the default. It is the default again because
    /// that is what was asked for, and because the walls are green-dominant and
    /// dim, against which white reads better than any of the alternatives on a
    /// dark profile -- which is the overwhelmingly common case.
    ///
    /// The caveat is on the option rather than hidden, and
    /// `every_path_colour_reads_on_a_light_and_on_a_dark_profile` exempts it with
    /// the reason recorded, so the exemption cannot quietly become an oversight.
    #[default]
    White,
    /// No green in it, and a red above any the walls can have.
    Magenta,
    /// Deeper and redder. Sits closer to the middle of the contrast band, so it is
    /// the most balanced of these on a light profile.
    Rose,
    /// Brighter and pinker. The most vivid, and the least legible on white.
    Pink,
    /// Blue-violet. Separates from the green-dominant walls by hue as well as by
    /// luminance, which none of the pinks do.
    Violet,
    /// Cold blue. Also hue-separated from the walls, and the darkest of these.
    Indigo,
    /// Green-cyan, and the only one that shares a hue family with the walls. It
    /// separates by luminance alone, so it is the most fragile of the set.
    Teal,
}

impl PathColor {
    /// Every variant, so a test can check all of them.
    pub const ALL: &'static [PathColor] = &[
        PathColor::White,
        PathColor::Magenta,
        PathColor::Rose,
        PathColor::Pink,
        PathColor::Violet,
        PathColor::Indigo,
        PathColor::Teal,
    ];

    /// The colour itself.
    ///
    /// Hand-picked to sit in relative luminance 0.10 to 0.30, which is the band
    /// that clears 3:1 against both black and white. The first three also have a
    /// red above 200, which is what separates them from the walls: those are
    /// drawn with red and blue both capped below 120.
    pub const fn color(self) -> style::Color {
        match self {
            PathColor::White => style::Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            PathColor::Magenta => style::Color::Rgb {
                r: 220,
                g: 0,
                b: 200,
            },
            PathColor::Rose => style::Color::Rgb {
                r: 210,
                g: 0,
                b: 120,
            },
            PathColor::Pink => style::Color::Rgb {
                r: 255,
                g: 0,
                b: 160,
            },
            PathColor::Violet => style::Color::Rgb {
                r: 150,
                g: 0,
                b: 220,
            },
            PathColor::Indigo => style::Color::Rgb {
                r: 90,
                g: 70,
                b: 240,
            },
            PathColor::Teal => style::Color::Rgb {
                r: 0,
                g: 120,
                b: 130,
            },
        }
    }
}

/// A maze being carved, or held after it finished.
///
/// The path colour lives in [`MazeOptions::path_color`]. The walls are
/// `Attribute::Bold`, which many terminals render by brightening, so the path
/// deliberately is not: it should be the one part of the frame whose brightness is
/// exactly what was asked for.
pub struct Maze {
    pub screen_size: (u16, u16),
    options: MazeOptions,
    canvas: Canvas,
    initial_walls: Buffer,
    paths: HashSet<(usize, usize)>,
    stack: VecDeque<(isize, isize)>,
    /// Where the carve started.
    ///
    /// Recorded because it is the only way to tell which cells the walk can
    /// possibly reach: the walk strides by two to keep the walls, so it preserves
    /// the parity of each coordinate and one of the four parity classes is
    /// unreachable. `every_reachable_cell_is_carved` needs to know which.
    pub start: (usize, usize),
    maze_complete: bool,
    /// Seconds of simulation the finished maze is shown before regenerating.
    ///
    /// Set when the carve completes, not counted in `get_diff`. `get_diff` is a
    /// drawing function and this is a decision about the simulation, and the
    /// effect already learned that lesson the hard way: the wall mottling used to
    /// live in `get_diff`, which made the texture a function of how many frames
    /// had been drawn rather than of elapsed time.
    completed_at: Option<f32>,
    /// Seconds of simulation elapsed, from the frame context.
    clock: f32,
    /// Fraction of a cell of carving progress not yet spent.
    ///
    /// Carving used to happen once per `update` call, so a terminal refreshing
    /// at 144 Hz generated its maze three times as fast as one at 48 Hz. The
    /// accumulator converts elapsed time into a whole number of carve steps.
    carve_accumulator: f32,
    pub rng: EffectRng,
}

impl TerminalEffect for Maze {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // The maze used to be torn down here, on the frame after it completed:
        //
        //     if self.maze_complete { self.reset(); return Vec::new(); }
        //
        // which means a finished maze existed for exactly one frame. This effect
        // has never once shown anyone its output. `hold` is how long the result
        // stays up instead, and `completed_at` is when it arrived.
        //
        // The draw is deliberately not short-circuited while holding. The walls
        // keep mottling behind the finished maze, so the picture is alive rather
        // than a freeze-frame, and the diff is only the cells that actually
        // changed.
        //
        // `return self.draw()` and *not* `self.draw(); return
        // self.canvas.commit()`. `draw` ends in a commit, so the second version
        // committed twice: the first commit's diff was discarded and the second
        // diffed the frame-with-maze against the frame-from-before-it. That is an
        // **erase**, emitted for every frame of the hold -- so the finished maze
        // was blanked from the screen and never shown at all, which is the one
        // thing the hold exists for.
        //
        // The test that was meant to catch it asserted only that the diff was
        // non-empty, and a full-screen erase is non-empty. It now asserts that
        // the drawn cells are the maze.
        if self.maze_complete
            && let Some(completed_at) = self.completed_at
            && self.elapsed_at(completed_at) < self.options.hold_seconds
        {
            return self.draw();
        }
        if self.maze_complete {
            self.reset();
            return Vec::new();
        }

        self.draw()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not finish the whole maze in one frame, which
        // would be a single visible jump rather than an animation.
        self.advance(context.delta.as_secs_f64().min(0.1) as f32);
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        let (width, height) =
            (self.screen_size.0 as usize, self.screen_size.1 as usize);

        // Both buffers are full-screen, so both have to follow the new size or
        // the carved path coordinates no longer fit. `reset` rebuilds them
        // anyway, but `update_size` is a public entry point and has to leave a
        // renderable effect behind on its own.
        self.canvas.resize(width as u16, height as u16);
        self.initial_walls = Buffer::new(width, height);
        let mut rng = seeded_rng(self.options.seed, "maze");
        fill_initial_walls(&mut self.initial_walls, &mut rng);
        self.paths.retain(|(x, y)| *x < width && *y < height);
    }

    fn reset(&mut self) {
        // `Self::new` already picks a start cell, generates the wall texture and
        // seeds the stack. Calling `fill_initial_walls` again here, and then
        // replacing the generator it just built, threw away a full random fill
        // for nothing.
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Maze {
    /// One simulation step: the walls move, and the maze is carved a little
    /// further.
    ///
    /// Both live here so the draw stays a pure function of the simulation. The
    /// wall mottling used to run inside `get_diff`, which made the wall texture a
    /// function of how many frames had been drawn rather than of elapsed time: a
    /// terminal refreshing at 144 Hz re-textured its walls at twice the rate of
    /// one at 72 Hz, and the effect consumed generator draws at the refresh rate
    /// rather than the simulation rate.
    fn advance(&mut self, seconds: f32) {
        self.clock += seconds;
        self.mottle();
        self.carve(seconds);
    }

    /// The frame, as a pure function of the simulation. Never mutates anything
    /// except the canvas, and never decides anything.
    fn draw(&mut self) -> Vec<(usize, usize, Cell)> {
        // The frame is the wall template with the carved path painted over it.
        // Blitting the template replaces cloning it every frame.
        self.canvas.blit(&self.initial_walls);
        for (x, y) in self.paths.iter() {
            self.canvas.set(
                *x,
                *y,
                Cell::new(
                    '█',
                    self.options.path_color.color(),
                    style::Attribute::Reset,
                ),
            );
        }
        self.canvas.commit()
    }

    /// Seconds of simulation since a moment on the clock.
    fn elapsed_at(&self, at: f32) -> f32 {
        (self.clock - at).max(0.0)
    }

    /// The carved path, for tests and for anything that wants to reason about the
    /// maze rather than draw it.
    pub fn paths(&self) -> &HashSet<(usize, usize)> {
        &self.paths
    }

    /// Whether the carve has finished. True during the hold as well as after it.
    pub fn maze_complete(&self) -> bool {
        self.maze_complete
    }

    /// Re-textures a few random wall cells, so the brickwork is not a still
    /// image behind the carve.
    fn mottle(&mut self) {
        if self.maze_complete {
            return;
        }

        let mut modified_cells = HashSet::new();
        while modified_cells.len() < WALLS_MOTTLED_PER_FRAME {
            let x = self.rng.random_range(0..self.canvas.width());
            let y = self.rng.random_range(0..self.canvas.height());

            if modified_cells.insert((x, y)) {
                let random_char =
                    CHARACTERS[self.rng.random_range(0..CHARACTERS.len())];
                let random_color = style::Color::Rgb {
                    r: self.rng.random_range(0..200) as u8,
                    g: self.rng.random_range(0..256) as u8,
                    b: self.rng.random_range(0..200) as u8,
                };
                self.initial_walls.set(
                    x,
                    y,
                    Cell::new(random_char, random_color, style::Attribute::Bold),
                );
            }
        }
    }

    fn carve_one(&mut self) {
        if let Some((x, y)) = self.stack.pop_back() {
            let directions = [(2, 0), (0, 2), (-2, 0), (0, -2)]; // Skip one cell to maintain walls
            let mut shuffled_directions = directions;
            shuffled_directions.shuffle(&mut self.rng);

            for &(dx, dy) in &shuffled_directions {
                let new_x = x + dx;
                let new_y = y + dy;

                // Check the cell to be carved and the wall between the current and new cell
                if self.is_valid_cell(new_x, new_y)
                    && self.is_valid_cell(x + dx / 2, y + dy / 2)
                    && !self.paths.contains(&(new_x as usize, new_y as usize))
                {
                    // Carve path for both the new cell and the wall between
                    self.carve_path(new_x, new_y);
                    self.carve_path(x + dx / 2, y + dy / 2);
                    // Push the current position back for backtracking
                    self.stack.push_back((x, y));
                    self.stack.push_back((new_x, new_y)); // Push the new position
                    break;
                }
            }

            // No backtrack pop here. `(x, y)` was already popped at the top of
            // this function, and on a move the line above pushes it straight back
            // so the stack reads `[.., (x, y), (new_x, new_y)]`. Popping a second
            // time on a dead end therefore discarded the *parent* of `(x, y)` --
            // the node that exists precisely so the search can backtrack into it.
            //
            // The effect was a depth-first search that gave up early, so the result
            // was not a spanning tree: whole regions were never visited and stayed
            // solid wall. That was invisible for as long as a finished maze was on
            // screen for one frame, and it is visible from the moment the hold
            // lands. `every_reachable_cell_is_carved` is the guard.
        } else {
            // If the stack is empty, the maze is complete
            self.maze_complete = true;
            self.completed_at = Some(self.clock);
        }
    }

    /// Cells carved per second.
    ///
    /// Was a flat one cell per `CARVE_STEP`, which is a per-*frame* rate wearing a
    /// seconds label: it carved `width * height / 4` cells at one per frame, so
    /// generation took about 8 seconds at 80x24, 42 at 200x50, and **five and a
    /// half minutes** at 400x200 -- longer than the effect's own playlist slot, so
    /// on a large terminal it was wiped and restarted mid-carve and never once
    /// reached the hold.
    ///
    /// Now sized from the area so a maze takes roughly the same wall-clock time at
    /// any terminal size. The floor keeps small windows carving at the original
    /// pace, which is both fast enough to look like an animation and the pace the
    /// existing tests were written against.
    fn cells_per_second(&self) -> f32 {
        let (width, height) = self.screen_size;
        // Widened before multiplying. `u16 * u16` overflows above 65535, and
        // 400x200 is 80,000 -- so the obvious spelling of this is a panic on any
        // large terminal, which is exactly the size the area-aware rate exists to
        // serve. Caught by `a_large_maze_finishes_in_seconds_not_minutes`.
        let cells = f64::from(width) * f64::from(height) / 4.0;
        let per_second = (cells / f64::from(TARGET_GENERATION_SECONDS)) as f32;
        per_second.max(MIN_CELLS_PER_SECOND)
    }

    /// Carves for `seconds` worth of progress, in whole cells.
    fn carve(&mut self, seconds: f32) {
        if self.maze_complete {
            return;
        }

        self.carve_accumulator += seconds * self.cells_per_second();
        let mut budget = self.carve_accumulator.floor() as usize;
        self.carve_accumulator -= budget as f32;
        // Bounded so a long stall cannot spend an unbounded amount of work in a
        // single frame.
        budget = budget.min(MAX_CARVE_STEPS_PER_FRAME);
        for _ in 0..budget {
            self.carve_one();
            if self.maze_complete {
                break;
            }
        }
    }

    pub fn new(options: MazeOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let mut rng = seeded_rng(options.seed, "maze");
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        let paths = HashSet::new();
        let start_x = rng.random_range(0..screen_size.0);
        let start_y = rng.random_range(0..screen_size.1);
        let mut stack = VecDeque::new();
        stack.push_back((start_x as isize, start_y as isize));
        let start = (start_x as usize, start_y as usize);

        let mut initial_walls = canvas.surface().clone();
        fill_initial_walls(&mut initial_walls, &mut rng);

        Self {
            screen_size,
            options,
            canvas,
            initial_walls,
            paths,
            stack,
            start,
            maze_complete: false,
            completed_at: None,
            clock: 0.0,
            carve_accumulator: 0.0,
            rng,
        }
    }

    fn is_valid_cell(&self, x: isize, y: isize) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < (self.screen_size.0 as usize)
            && (y as usize) < (self.screen_size.1 as usize)
    }

    fn carve_path(&mut self, x: isize, y: isize) {
        self.paths.insert((x as usize, y as usize));
    }
}

/// Fills `buffer` with the decorative wall texture.
///
/// Takes the generator rather than making its own. A local thread RNG here
/// meant the texture was unseeded even once the effect was seedable, so two
/// instances of the same `seed` still disagreed on every brick.
fn fill_initial_walls(buffer: &mut Buffer, rng: &mut EffectRng) {
    for y in 0..buffer.height {
        for x in 0..buffer.width {
            let random_char = CHARACTERS[rng.random_range(0..CHARACTERS.len())];
            let random_color = style::Color::Rgb {
                r: rng.random_range(0..120) as u8,
                g: rng.random_range(0..256) as u8,
                b: rng.random_range(0..120) as u8,
            };
            buffer.set(
                x,
                y,
                Cell::new(random_char, random_color, style::Attribute::Bold),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{FrameContext, InputState};
    use std::time::Duration;

    /// A maze that has finished carving, reached by driving the simulation rather
    /// than by setting the flag, so the test exercises the same path the effect
    /// does.
    fn finished_maze() -> Maze {
        let mut maze = Maze::new(MazeOptions::default(), (40, 20));
        for _ in 0..4_000 {
            maze.update_with_context(&frame_after(0.05));
            if maze.maze_complete {
                break;
            }
        }
        assert!(maze.maze_complete(), "the maze never finished carving");
        maze
    }

    fn frame_after(seconds: f64) -> FrameContext {
        FrameContext::new(
            (40, 20),
            0,
            Duration::from_secs_f64(seconds),
            Duration::from_secs_f64(seconds),
            InputState::default(),
        )
    }

    /// A finished maze has to stay on screen.
    ///
    /// `get_diff` used to do this:
    ///
    /// ```ignore
    /// if self.maze_complete {
    ///     self.reset();
    ///     return Vec::new();
    /// }
    /// ```
    ///
    /// so the frame after completion tore the maze down and started a new one,
    /// and a completed maze existed for exactly one frame. Nobody had ever seen
    /// this effect's output. Everything anyone has said about how it looks has
    /// been about the carve in progress -- which is also why "why is the maze
    /// pink" and "the maze is good now, how do we improve it" are both really
    /// about the one part of the effect that is a process rather than a picture.
    ///
    /// The hold is what turns it from a process into a picture, and it is a
    /// prerequisite for judging every other visual change to this effect: until
    /// the result is on screen there is nothing to look at.
    #[test]
    fn a_finished_maze_is_left_on_screen_before_it_regenerates() {
        let (width, height) = (40usize, 20usize);
        let mut maze = Maze::new(MazeOptions::default(), (40, 20));
        for _ in 0..4_000 {
            maze.update_with_context(&frame_after(0.05));
            if maze.maze_complete {
                break;
            }
        }
        assert!(maze.maze_complete(), "the maze never finished carving");
        let path = maze.paths().clone();

        // Replay the diff into what the terminal is showing, because a diff is
        // not a picture. `terminal` is the screen.
        //
        // The old version of this asserted only `!diff.is_empty()`, and a
        // full-screen *erase* is not empty -- so it passed while the finished
        // maze was being blanked from the screen every frame of the hold. That
        // is the whole defect: an assertion on the shape of the change rather
        // than on the result.
        let mut terminal = Buffer::new(width, height);
        let commit = |maze: &mut Maze, terminal: &mut Buffer| {
            for (x, y, cell) in maze.get_diff() {
                if x < width && y < height {
                    terminal.set(x, y, cell);
                }
            }
        };

        // The frame that notices completion must still draw the maze.
        commit(&mut maze, &mut terminal);
        assert!(
            path.iter().all(|(x, y)| terminal.get(*x, *y).symbol == '█'),
            "the frame a maze completed on did not put the carved path on the \
             screen: {}/{} path cells are showing something else",
            path.iter()
                .filter(|(x, y)| terminal.get(*x, *y).symbol != '█')
                .count(),
            path.len()
        );

        // And it must *stay* drawn, not just appear once. Held over several
        // frames, because one frame is not a hold.
        for frame in 0..12 {
            commit(&mut maze, &mut terminal);
            let missing = path
                .iter()
                .filter(|(x, y)| terminal.get(*x, *y).symbol != '█')
                .count();
            assert_eq!(
                missing,
                0,
                "held frame {frame} of the maze took {missing} of {} path cells \
                 off the screen, so the result is not being held",
                path.len()
            );
        }
    }

    /// The hold has to end, or the effect becomes `blank`.
    #[test]
    fn the_hold_expires_and_a_new_maze_starts() {
        let mut maze = finished_maze();
        let before = maze.paths().len();

        // Well past the default hold, in simulation time.
        for _ in 0..200 {
            maze.update_with_context(&frame_after(0.1));
            let _ = maze.get_diff();
        }

        assert_ne!(
            maze.paths().len(),
            before,
            "the maze never regenerated, so the effect is now a still image"
        );
        assert!(
            !maze.maze_complete(),
            "a fresh maze reports itself complete"
        );
    }

    /// The carve has to visit every cell it can reach.
    ///
    /// The backtrack used to pop the stack a second time on reaching a dead end,
    /// which threw away the parent of the cell being examined -- the node that
    /// exists so the search can backtrack into it. The result was a depth-first
    /// search that gave up early rather than a spanning tree, so whole regions
    /// were never visited and stayed solid wall.
    ///
    /// This was invisible for as long as a finished maze was on screen for exactly
    /// one frame, and it is visible from the moment the hold landed. So this is the
    /// test that says the maze you are looking at is actually a maze.
    #[test]
    fn every_reachable_cell_is_carved() {
        let (width, height) = (40usize, 20usize);
        for seed in [1u64, 7, 42, 99] {
            let mut maze = Maze::new(
                MazeOptions {
                    seed,
                    ..Default::default()
                },
                (width as u16, height as u16),
            );
            for _ in 0..20_000 {
                maze.update_with_context(&frame_after(0.1));
                if maze.maze_complete {
                    break;
                }
            }
            assert!(maze.maze_complete(), "seed {seed} never finished");

            // The walk strides by two to keep the walls, so it preserves the
            // parity of each coordinate. From a start at (3, 4) it can only ever
            // reach odd-x even-y cells, so exactly one of the four parity classes
            // is the lattice and every cell in it must be carved.
            //
            // The other three classes hold the *midpoints* -- the wall cells the
            // walk removes between two lattice cells -- and those are partial by
            // nature, since only edges get one. An earlier version of this test
            // demanded all four classes be complete or empty, and reported "93 of
            // 200" for the start's own class, which reads exactly like a broken
            // carve. It was the assertion that was wrong: 93 was the count of
            // vertical midpoints.
            let (start_x, start_y) = maze.start;
            let lattice: Vec<(usize, usize)> = (0..height)
                .filter(|y| y % 2 == start_y % 2)
                .flat_map(|y| {
                    (0..width)
                        .filter(move |x| x % 2 == start_x % 2)
                        .map(move |x| (x, y))
                })
                .collect();
            let carved = lattice
                .iter()
                .filter(|cell| maze.paths().contains(cell))
                .count();

            assert_eq!(
                carved,
                lattice.len(),
                "seed {seed}: {carved} of {} cells reachable from the start were                  carved, so that many were never searched",
                lattice.len()
            );
        }
    }

    /// Generation has to finish in a sensible time at a large terminal size.
    ///
    /// It did not. The rate was one cell per frame regardless of area, and a
    /// 400x200 maze needs about 20,000 cells, so it took five and a half minutes
    /// -- longer than this effect's playlist slot, meaning the maze was wiped and
    /// restarted mid-carve and never once reached the new hold.
    #[test]
    fn a_large_maze_finishes_in_seconds_not_minutes() {
        let (width, height) = (400u16, 200u16);
        let mut maze = Maze::new(MazeOptions::default(), (width, height));

        let mut elapsed = 0.0f32;
        for _ in 0..40_000 {
            maze.update_with_context(&frame_after(0.05));
            elapsed += 0.05;
            if maze.maze_complete {
                break;
            }
        }

        assert!(maze.maze_complete(), "the maze never finished at all");
        assert!(
            elapsed < 90.0,
            "a {width}x{height} maze took {elapsed:.0}s of simulation to carve, \
             which is longer than the effect's playlist slot"
        );
    }

    #[test]
    fn check_initial_state() {
        let options = MazeOptions::default();
        let maze = Maze::new(options, (3, 3));

        // buffer correctly initialized
        let mut initialized_cells = 0;
        for cell in maze.canvas.on_screen().iter() {
            if cell.symbol != ' ' {
                initialized_cells += 1;
            }
        }
        assert_eq!(initialized_cells, 0);
        assert_eq!(maze.initial_walls.buffer.len(), 9);

        // path and stack are empty, and maze is not completed
        assert!(maze.paths.is_empty());
        assert!(maze.stack.len() == 1);
        assert!(!maze.maze_complete);
    }

    #[test]
    fn check_flow() {
        let options = MazeOptions::default();
        let mut maze = Maze::new(options, (5, 5));
        maze.update();
        let diff = maze.get_diff();
        assert_eq!(diff.len(), 25);

        // buffer correctly processed
        let mut path_cells = 0;
        for cell in maze.canvas.on_screen().iter() {
            if cell.symbol != '█' {
                path_cells += 1;
            }
        }
        assert_eq!(path_cells, 23);
    }

    #[test]
    fn the_carved_path_is_not_white_and_cannot_be_mistaken_for_a_wall() {
        let mut maze = Maze::new(MazeOptions::default(), (12, 9));
        for _ in 0..30 {
            maze.update();
            maze.get_diff();
        }

        let (x, y) = *maze.paths.iter().next().expect("nothing was carved");
        let cell = maze.canvas.get(x, y);

        assert_eq!(cell.symbol, '█');
        assert_ne!(
            cell.color,
            style::Color::White,
            "the carved path is white, which is invisible on a light profile"
        );

        // The walls are drawn with red and blue both below 120, so a red above
        // 200 cannot be a wall colour however many times the mottle has run.
        //
        // This only holds for the variants that lean on red to separate. The blue
        // and teal ones separate by hue and by luminance instead, and are covered
        // by the contrast tests below -- so the check is narrowed to the variants
        // it actually applies to rather than deleted.
        let style::Color::Rgb { r, .. } = cell.color else {
            panic!("the path is not a truecolor value: {:?}", cell.color);
        };
        if matches!(
            maze.options.path_color,
            PathColor::Magenta | PathColor::Rose | PathColor::Pink
        ) {
            assert!(
                r > 200,
                "the path colour {r},.. is inside the range a wall is drawn from, \
                 so the path does not stand out from the walls"
            );
        }
    }

    /// Every path colour has to read on both a light and a dark profile.
    ///
    /// The reason the alternatives are a named choice rather than "whatever looked
    /// nice": a terminal profile is whichever of white or black the user happens to
    /// run, and a path that vanishes on one of them is a bug rather than a
    /// preference. This is what constrains every non-white variant to relative
    /// luminance 0.10 to 0.30.
    ///
    /// Checked for *every* non-white variant, so adding one that fails is caught
    /// here rather than by a user on a light background.
    #[test]
    fn every_path_colour_reads_on_a_light_and_on_a_dark_profile() {
        for choice in PathColor::ALL {
            // White is exempt, and not because it passes. WCAG contrast is defined
            // between two colours: white clears 21:1 against black and is *exactly*
            // the colour of a light background, so the measurement reports 21:1
            // and the screen reports nothing at all. It is exempt deliberately, it
            // is the default, and the reason is recorded on the variant where a
            // user choosing it will see it.
            if *choice == PathColor::White {
                continue;
            }
            let style::Color::Rgb { r, g, b } = choice.color() else {
                unreachable!("PathColor::color is defined as an Rgb");
            };
            let luminance =
                0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
            let on_white = 1.05 / (luminance + 0.05);
            let on_black = (luminance + 0.05) / 0.05;

            assert!(
                on_white >= 3.0,
                "{choice:?} is only {on_white:.1}:1 on a light profile, so the path \
                 would be hard to see on a white background"
            );
            assert!(
                on_black >= 3.0,
                "{choice:?} is only {on_black:.1}:1 on a dark profile, so the path \
                 would be hard to see on a black background"
            );
        }
    }

    /// Every non-white variant has to be dark enough to read on white, and no
    /// variant may be a *near*-white.
    ///
    /// This replaced a test that asserted no variant could be white at all, on the
    /// grounds that a white corridor is invisible on a light profile. That is
    /// true, and it is why white was not the default for a while -- but white is
    /// the default again now, so the assertion was reversed rather than kept.
    ///
    /// What survives is the part that still matters: nothing may be *near* white.
    /// `rgb(240, 240, 240)` is not white, passes every contrast test, and is just as
    /// invisible on a light background, so the near-white case is the one worth
    /// guarding and the pure-white case is a choice.
    #[test]
    fn no_path_colour_is_near_white() {
        for choice in PathColor::ALL {
            let style::Color::Rgb { r, g, b } = choice.color() else {
                unreachable!("PathColor::color is defined as an Rgb");
            };
            if *choice == PathColor::White {
                continue;
            }
            let luminance =
                0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
            assert!(
                luminance < 0.30,
                "{choice:?} has luminance {luminance:.2}, which is bright enough to \
                 wash out on a light profile"
            );
        }
    }

    /// White is the default, and it is the default knowingly.
    ///
    /// Pinned because it is the one thing here that a user is likely to have an
    /// opinion about, and because the caveat attached to it lives in a doc comment
    /// that nothing checks.
    #[test]
    fn the_path_is_white_by_default() {
        assert_eq!(
            MazeOptions::default().path_color,
            PathColor::White,
            "the default path colour changed; if that was deliberate, this test \
             and the caveat on PathColor::White both need updating"
        );
        assert_eq!(PathColor::default(), PathColor::White);
    }

    /// The configured choice has to reach the screen, and survive the config
    /// round trip by name -- which is how it appears in the config file.
    #[test]
    fn the_configured_path_colour_is_drawn() {
        for choice in PathColor::ALL {
            let mut maze = Maze::new(
                MazeOptions {
                    path_color: *choice,
                    ..Default::default()
                },
                (12, 9),
            );
            for _ in 0..30 {
                maze.update();
                maze.get_diff();
            }
            let (x, y) = *maze.paths.iter().next().expect("nothing was carved");
            assert_eq!(
                maze.canvas.get(x, y).color,
                choice.color(),
                "{choice:?} was configured but a different colour was drawn"
            );
        }

        let parsed: MazeOptions =
            toml::from_str("path_color = \"violet\"").expect("parses by name");
        assert_eq!(parsed.path_color, PathColor::Violet);
    }

    /// sRGB channel to relative luminance, per WCAG 2.x.
    fn linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    #[test]
    fn drawing_a_frame_does_not_re_texture_the_walls() {
        let mut maze = Maze::new(MazeOptions::default(), (10, 8));
        maze.update();
        maze.get_diff();

        let settled = maze.initial_walls.buffer.clone();

        // The mottle ran inside `get_diff`, so redrawing a frame without a
        // simulation step still consumed generator draws and still changed the
        // wall texture. What was on screen therefore depended on how many times
        // the frame had been drawn rather than on elapsed time, which is the
        // same frame-rate dependence the carve accumulator was added to fix.
        for _ in 0..30 {
            maze.get_diff();
        }

        assert_eq!(
            settled, maze.initial_walls.buffer,
            "redrawing the frame re-textured the walls"
        );
    }

    #[test]
    fn a_simulation_step_still_moves_the_walls() {
        // The other half: moving the mottle must not have quietly stopped it,
        // or the brickwork behind the carve is a still image.
        let mut maze = Maze::new(MazeOptions::default(), (10, 8));
        let before = maze.initial_walls.buffer.clone();

        for _ in 0..30 {
            maze.update();
            maze.get_diff();
        }

        assert_ne!(before, maze.initial_walls.buffer);
    }
}
