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
pub struct MazeOptions {
    /// Seed for the wall texture, the start cell and the carve order.
    pub seed: u64,
}

impl Default for MazeOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self { seed: DEFAULT_SEED }
    }
}
/// Seconds of carving per cell, i.e. how fast the maze draws itself.
const CARVE_STEP: f32 = 1.0 / 60.0;
/// Upper bound on cells carved in one frame, so a stall cannot be paid off in
/// a single visible jump.
const MAX_CARVE_STEPS_PER_FRAME: usize = 8;
/// Wall cells re-textured per frame, so the walls are not a still image.
const WALLS_MOTTLED_PER_FRAME: usize = 3;

/// The carved path's colour.
///
/// Was `style::Color::White`, which is invisible on a light terminal profile --
/// a white logo on a white background -- for the one part of the effect the
/// eye is meant to follow.
///
/// Magenta instead, and that is a contrast calculation rather than a preference.
/// The walls are drawn with red and blue both below 120 and green up to 255, so
/// they are green-dominant colours of low-to-medium luminance. Magenta has no
/// green in it and a red above any wall can have, so it separates from them by
/// hue on a dark profile and by luminance on a light one:
///
/// * relative luminance 0.19, which is 4.3:1 against white and 4.8:1 against
///   black -- both comfortably over the 3:1 that WCAG asks of a graphical
///   object, which is what a wall of `█` is.
///
/// The walls themselves are `Attribute::Bold`, which many terminals render by
/// brightening, so the path deliberately is not: it should be the one part of
/// the frame whose brightness is exactly what was asked for.
const PATH_COLOR: style::Color = style::Color::Rgb {
    r: 220,
    g: 0,
    b: 200,
};

pub struct Maze {
    pub screen_size: (u16, u16),
    options: MazeOptions,
    canvas: Canvas,
    initial_walls: Buffer,
    paths: HashSet<(usize, usize)>,
    stack: VecDeque<(isize, isize)>,
    maze_complete: bool,
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
        if self.maze_complete {
            self.reset();
            return Vec::new();
        }

        // The frame is the wall template with the carved path painted over it.
        // Blitting the template replaces cloning it every frame.
        self.canvas.blit(&self.initial_walls);
        for (x, y) in self.paths.iter() {
            self.canvas.set(
                *x,
                *y,
                Cell::new('█', PATH_COLOR, style::Attribute::Reset),
            )
        }

        self.canvas.commit()
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
        self.mottle();
        self.carve(seconds);
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

            let mut moved = false;
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
                    moved = true;
                    break;
                }
            }

            if !moved {
                // If we didn't move, it means we're at a dead-end and need to backtrack
                self.stack.pop_back();
            }
        } else {
            // If the stack is empty, the maze is complete
            self.maze_complete = true;
        }
    }

    /// Carves for `seconds` worth of progress, in whole cells.
    fn carve(&mut self, seconds: f32) {
        if self.maze_complete {
            return;
        }

        self.carve_accumulator += seconds / CARVE_STEP;
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

        let mut initial_walls = canvas.surface().clone();
        fill_initial_walls(&mut initial_walls, &mut rng);

        Self {
            screen_size,
            options,
            canvas,
            initial_walls,
            paths,
            stack,
            maze_complete: false,
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
        let style::Color::Rgb { r, .. } = cell.color else {
            panic!("the path is not a truecolor value: {:?}", cell.color);
        };
        assert!(
            r > 200,
            "the path colour {r},.. is inside the range a wall is drawn from, so \
             the path does not stand out from the walls"
        );
    }

    #[test]
    fn the_path_colour_reads_on_a_light_and_on_a_dark_profile() {
        // The reason the path colour is not "whatever looked nice": it has to
        // clear 3:1 against both a white and a black background, because a
        // terminal profile is whichever of those the user happens to run.
        let style::Color::Rgb { r, g, b } = PATH_COLOR else {
            unreachable!("PATH_COLOR is defined as an Rgb");
        };
        let luminance =
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
        let on_white = 1.05 / (luminance + 0.05);
        let on_black = (luminance + 0.05) / 0.05;

        assert!(
            on_white >= 3.0,
            "the path is only {on_white:.1}:1 on a light profile"
        );
        assert!(
            on_black >= 3.0,
            "the path is only {on_black:.1}:1 on a dark profile"
        );
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
