use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::render::BrailleGrid;
use crate::render::braille::{DOTS_X, DOTS_Y};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// The DVD wordmark, as a dot bitmap: 60 dots across by 28 down, which is
/// exactly 30 by 7 terminal cells at braille's 2x4.
///
/// Supplied as braille art -- thirty columns of the U+2800 block, which is
/// precisely what braille decodes to -- and transcribed here as `#` for an inked
/// dot and `.` for a blank, one character per dot.
///
/// Stored as dots rather than as the braille characters it came from, for two
/// reasons. The renderer only ever needs the dot grid, so a bitmap needs no
/// decoding step to read. And a literal of 1680 U+28xx codepoints is a
/// thousand glyphs that all look alike, so a typo in it is invisible in a diff
/// and invisible in a code review; the same art as `#` and `.` reads as a
/// picture of the shape in the source file.
///
/// The lower half is not damage. It is a 3D extrusion of the wordmark -- a
/// reflection below the baseline -- and it is most of why this reads as *the*
/// DVD logo rather than as the three letters D, V and D. Do not tidy it away.
///
/// This replaced a hand-invented seven-row block letter, which was wrong in the
/// way an approximation always is: it read as `DVD` only if you were already
/// told that is what it was. There is no `DUD` risk here -- a real wordmark
/// does not have that failure mode -- and there is no outline-font problem
/// either, because at 2x4 there is no longer a choice between a slab and a
/// hairline.
const DVD_WORDMARK: &str = "\
.....#######################.........#################......
.....#######################........#####################...
.....#######################........######################..
....########################.......########################.
....######.....##############.....#############.....########
....######.......############.....######.######......#######
...#######.......############....######.#######......#######
...######........######.######..######..######.......#######
...######........######.######..######.#######.......#######
...######.......#######..############..#######.......######.
...######......#######...###########...#######.....########.
...######....########....##########....######.....########..
###################.....########.....##################.....
#################.......#######......################.......
###############.........#######......##############.........
############.............#####.......############...........
...........................####.............................
...........................###..............................
...........................##...............................
....................#################.......................
.......###########################################..........
...##################################################.......
.###########.##..###.####....####...####.....##########.....
###############.####.####.##.####.#####.####.############...
#############..#####.####.##.####.######.###.###########....
..############.#####.####....####...####....##########......
.....###############################################........
..............#############################.................";

/// Whether a logo character leaves its dot unraised.
///
/// A space is, because that is what the character-grid logos this replaced
/// used, and `.` is, because that is the alphabet the wordmark is written in.
/// Everything else is ink, which is what keeps `logo = "DVD"` -- and any
/// config file still carrying the old block letter, a row of `█` and spaces --
/// drawing something rather than silently becoming 60 blank dots. See
/// [`DvdOptions::logo`].
fn is_blank(symbol: char) -> bool {
    symbol == ' ' || symbol == '.'
}

/// The logo's colours, in the order it cycles through them.
///
/// Every entry sits in a narrow band of relative luminance, 0.12 to 0.25, and
/// that is the whole design constraint. Against a pure white background a
/// colour is legible at 3:1 or better only below relative luminance 0.30, and
/// against pure black only above 0.10. A terminal profile is whichever of the
/// two the user runs, and a screensaver has no way to ask, so the band that
/// works on both is the band every colour has to be in.
///
/// The first entry used to be `(235, 235, 245)` -- a near-white, luminance
/// 0.83. On a light profile that is 1.03:1, which is invisible, and since
/// `color_index` is randomised at construction the logo was invisible for one
/// frame in six. It is now the deep rose, which is 4.3:1 on white and 4.8:1 on
/// black, and it is first because a red logo is what a DVD logo should be.
///
/// `every_colour_reads_on_a_light_and_on_a_dark_profile` enforces the band, for
/// every entry and not only the first, because `color_index` is randomised so
/// any of them can be the colour a run starts on.
///
/// Twelve rather than six, because consecutive colours are seen seconds apart and
/// need to be obviously different. Six was enough when a change was a
/// once-a-minute event, which is what a corner-only rule amounts to. Hues are
/// spread roughly every thirty degrees at constant luminance, which means the
/// perceived *brightness* never jumps even though the hue always does -- varying
/// luminance as well would read as a flash, and a flash is the complaint
/// [`ColorChange`] is now built to avoid.
const PALETTE: [(u8, u8, u8); 12] = [
    (224, 34, 34),
    (171, 98, 26),
    (120, 120, 18),
    (75, 130, 20),
    (20, 135, 20),
    (20, 133, 76),
    (19, 128, 128),
    (31, 117, 204),
    (90, 70, 240),
    (193, 0, 232),
    (194, 31, 209),
    (214, 32, 138),
];

const DT: f64 = 1.0 / 60.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DvdOptions {
    /// Logo to bounce around the screen, as a dot bitmap: one character per dot.
    ///
    /// Rows are separated by `\n`, `#` is an inked dot and `.` is a blank, which
    /// is the alphabet [`DVD_WORDMARK`] is written in. The default is 60 by 28
    /// dots, so 30 by 7 cells.
    ///
    /// The rule is "anything that is not blank is ink" rather than "only `#` is
    /// ink", and that is deliberate, for the sake of every config file already
    /// on disk. `logo = "DVD"` was a legal value under the old character-grid
    /// format and still draws as three inked dots; a row of `█` and spaces still
    /// draws as a row of full-height strokes. A stricter alphabet would have made
    /// both of them a field of blanks -- and per the trap in `AGENTS.md`, a
    /// config file *is* how a user ends up pinned to whatever the default was
    /// when they ran `--print-config`.
    pub logo: String,
    /// Base bounce speed in cells per second, along the horizontal axis.
    pub speed: f32,
    /// Cells travelled horizontally per cell travelled vertically.
    ///
    /// This is the other half of "it should go diagonally". The logo moved one
    /// cell right and one cell down for every step, which on a cell grid is *not*
    /// 45 degrees: a terminal cell is roughly twice as tall as it is wide, so equal
    /// cell deltas draw a line at 50 to 63 degrees. Travelling two cells across for
    /// every one down puts the visual angle back at 45.
    ///
    /// Set to 1.0 for equal cell deltas, which is what the classic bouncing logo
    /// does in a character grid.
    pub slope: f32,
    /// When the logo changes colour.
    ///
    /// Was a bool meaning "on corners", and corners are almost unreachable: a
    /// corner hit needs both axes to reverse on the same frame, and the bounce
    /// periods are `2 * max_x / vx` and `2 * max_y / vy`. On an 80x24 terminal
    /// with the default 30-cell logo those are 5.6 and 3.8 seconds, so the two
    /// only coincide after 25 and 17 of them -- about 95 seconds, and a figure
    /// that moves with the logo's width rather than being a property of anything.
    /// The logo sat at one colour for a minute and a half, which is the same as
    /// never changing.    ///
    /// lemonyte's `dvd-screensaver`, which is the reference for this effect,
    /// recolours on *every* wall hit, and that is what [`ColorChange::Bounce`]
    /// still does. It is no longer the default: with a 30-cell wordmark and a
    /// 3D extrusion under it, a full-slab hue change every second and a half is a
    /// strobe rather than a logo. [`ColorChange::Steady`] is the default and is
    /// the same idea slowed by [`RECOLOR_EVERY_N_BOUNCES`].
    pub color_change: ColorChange,
    /// Start the logo from a corner instead of the middle.
    pub start_in_corner: bool,
    /// Seed for the starting corner and colour.
    pub seed: u64,
}

/// How many wall hits separate two colour changes in [`ColorChange::Steady`].
///
/// Three, and the complaint that set the number is "it flickers". What flickers
/// is not the logo moving -- it is the *whole* slab changing hue at once, which
/// at 30 by 7 cells is a lot of pixels to repaint simultaneously. On an 80x24
/// terminal a wall hit arrives roughly every 1.4 seconds, so one hue change per
/// hit is a change every 1.4 seconds and one per three is a change every 4.3.
///
/// Bounce count rather than elapsed time or distance travelled, because those two
/// are the same thing once `speed` is fixed, and neither is stable across a
/// resize: a distance rule is throttled by time on a small terminal and not at
/// all on a 400x200 one, where wall hits are half a minute apart and every one
/// of them would qualify. Counting hits is one counter, it is the same fraction
/// of the visible event on every terminal, and it makes the behaviour a pure
/// function of state that a test can drive without a stopwatch.
const RECOLOR_EVERY_N_BOUNCES: u32 = 3;

/// When the bouncing logo changes colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorChange {
    /// Every third wall hit: the same idea as [`ColorChange::Bounce`] without
    /// changing colour on every single one.
    ///
    /// It was the default once, on the theory that a recolour of a solid 30x7
    /// slab is a large simultaneous change and therefore a strobe. That was the
    /// wrong diagnosis. The flicker is the *speed*, not the colour: the logo is
    /// a braille bitmap, so it is already snapped to whole dots, and at 18 cells
    /// a second that is a one-dot step on most frames -- and a one-dot shift
    /// rewrites nearly every glyph in a dense letterform, because a braille glyph
    /// *is* its bit pattern. Halving the speed removed the shimmer, and with wall
    /// hits eight seconds apart rather than one and a half, recolouring on every
    /// one of them stopped being a strobe too.
    ///
    /// Kept for anyone who wants fewer colour changes than bounces. The
    /// serialised name is unchanged, so a config saying `"steady"` still means
    /// what it meant.
    Steady,
    /// On every wall hit. The default, and what the reference implementation
    /// does.
    ///
    /// This was briefly *not* the default, on the theory that recolouring every
    /// bounce flickered. See [`ColorChange::Steady`] for why that diagnosis was
    /// wrong, what the real cause turned out to be, and why fixing it made this
    /// safe to put back.
    #[default]
    Bounce,
    /// Only when both axes reverse on the same frame, which is the easter egg the
    /// reference's README jokes about -- "it could hit the corner if you look at
    /// it long enough". On an 80x24 terminal with the default logo that is about
    /// every 95 seconds; see [`DvdOptions::color_change`], which works out why.
    Corner,
    /// Never. One colour for the whole run.
    Never,
}

impl ColorChange {
    /// Every variant, so a test can check all of them.
    pub const ALL: &'static [ColorChange] = &[
        ColorChange::Steady,
        ColorChange::Bounce,
        ColorChange::Corner,
        ColorChange::Never,
    ];

    /// Whether a wall hit of these two axes should change the colour.
    ///
    /// `wall_hits` is the caller's own running count of wall hits since the last
    /// recolour. [`ColorChange::Steady`] gates on it and the other three ignore
    /// it. It is a parameter rather than a field so that the rule stays a pure
    /// function and can be checked directly, which
    /// `the_colour_rule_is_what_each_mode_says` does.
    pub fn should_change(self, hit_x: bool, hit_y: bool, wall_hits: u32) -> bool {
        match self {
            ColorChange::Steady => {
                (hit_x || hit_y) && wall_hits >= RECOLOR_EVERY_N_BOUNCES
            }
            ColorChange::Bounce => hit_x || hit_y,
            ColorChange::Corner => hit_x && hit_y,
            ColorChange::Never => false,
        }
    }
}

impl Default for DvdOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            logo: String::from(DVD_WORDMARK),
            // 5.0, and the number comes from the reference implementation rather
            // than from taste.
            //
            // lemonyte's screensaver moves 50 pixels a second across a 1920-pixel
            // window, and its logo is a sixth of that window. So it crosses one
            // logo width every 6.4 seconds, or at 0.156 logo-widths a second.
            // This logo is 30 cells wide, so the same *feel* is 4.7 cells a
            // second, which rounds to 5.
            //
            // The number was 18 for a while and the justification given for it
            // was wrong, twice over. It was derived by scaling 24 by the old logo
            // width over the new, which holds `cells_per_second * width`
            // constant -- a product that is not a quantity that means anything,
            // and which in particular does not preserve the time to cross one
            // logo width. That would want 31, not 18. The second error was more
            // interesting: 18 was defended as "too quick rather than stuttering",
            // on the theory that a slow logo steps visibly from dot to dot.
            //
            // That theory has the braille grid backwards. A step here is one dot
            // out of the logo's 60, so it moves the picture 1.7% of its own
            // width. The block letter this replaced stepped a whole cell out of
            // 23 -- 4.3% -- and did so at 24 cells a second rather than 18. The
            // braille logo is not the coarse one; at a slower rate it is three
            // times finer than the thing that was reported as a staircase.
            //
            // So the stepping was never the problem, and the flicker was. It is
            // this: a braille glyph *is* its bit pattern, so moving the logo one
            // dot rewrites nearly every cell it passes through. At 18 cells a
            // second that is 36 dot-steps against 60 frames -- a whole-logo
            // repaint on three frames in five, which reads as a shimmer rather
            // than as motion. At 5 it is 10 dot-steps, one every six frames, and
            // the logo slides.
            //
            // Which is also what makes [`ColorChange::Bounce`] safe to leave on
            // every hit: at this speed a wall hit is about eight seconds away
            // rather than one and a half, so a colour change is an event again
            // instead of a strobe.
            //
            // Set `speed` in the config for brisker. Above about 24 the shimmer
            // comes back.
            speed: 5.0,
            slope: 2.0,
            color_change: ColorChange::default(),
            start_in_corner: true,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Dvd {
    screen_size: (u16, u16),
    options: DvdOptions,
    canvas: Canvas,
    /// The logo's dots, encoded at 2x4 per cell. See [`BrailleGrid`].
    ///
    /// Sized to the *logo's* bounding box in cells, not to the screen: the draw
    /// reads straight through from the source bitmap at a sub-cell offset, so
    /// there is nothing here for a full-screen grid to hold. At 400x200 that is
    /// 30 by 7 cells rather than 80000.
    grid: BrailleGrid,
    /// The logo source, one character per dot. See [`is_blank`].
    rows: Vec<Vec<char>>,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color_index: usize,
    /// Wall hits since the last recolour. What [`ColorChange::Steady`] counts.
    wall_hits: u32,
    rng: EffectRng,
}

impl TerminalEffect for Dvd {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.draw()
    }

    fn update(&mut self) {
        self.step(DT);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.step(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas
            .resize(self.screen_size.0.max(1), self.screen_size.1.max(1));
        self.rows = Self::parse_logo(&self.options.logo);
        // Sized to the logo, plus the spare cell that absorbs the sub-cell skew;
        // see `grid_w`. The logo is what changed size when this moved to braille:
        // 60 dots across is 30 cells and 28 dots down is 7. `resize` is a no-op
        // when the dimensions already match, so this costs nothing on the frames
        // where they do not.
        self.grid.resize(self.grid_w(), self.grid_h());

        // Reseeded rather than merely carried over: a resize is a new screen, and
        // re-rolling the corner and colour is what makes a resize feel like a
        // fresh run instead of a jump cut.
        self.rng = seeded_rng(self.options.seed, "dvd");

        let logo_width = self.logo_width();
        let logo_height = self.logo_height();
        let max_x = (self.screen_size.0 as f64 - logo_width).max(0.0);
        let max_y = (self.screen_size.1 as f64 - logo_height).max(0.0);

        let speed = self.options.speed.max(0.1) as f64;

        if self.options.start_in_corner {
            let corner = self.rng.random_range(0..4);
            self.x = if corner & 1 == 0 { 0.0 } else { max_x };
            self.y = if corner & 2 == 0 { 0.0 } else { max_y };
            // `slope` cells across per cell down, so the *visual* angle is 45
            // degrees on a cell grid that is taller than it is wide.
            let across = speed;
            let down = speed / f64::from(self.options.slope).max(0.1);
            self.vx = if self.x <= 0.0 { across } else { -across };
            self.vy = if self.y <= 0.0 { down } else { -down };
        } else {
            self.x = max_x / 2.0;
            self.y = max_y / 2.0;
            self.vx = speed;
            self.vy = speed / f64::from(self.options.slope).max(0.1);
        }
        self.color_index = self.rng.random_range(0..PALETTE.len());
        // A fresh run has not bounced yet, so the first wall hit it sees is the
        // first of its `RECOLOR_EVERY_N_BOUNCES`. Carrying the count over a
        // resize would make the first post-resize colour depend on how long the
        // previous run had been going.
        self.wall_hits = 0;
    }
}

impl Dvd {
    pub fn new(options: DvdOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let seed = options.seed;
        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            // Replaced by `reset` below; this only has to be a valid grid.
            grid: BrailleGrid::new(1, 1),
            rows: Vec::new(),
            x: 0.0,
            y: 0.0,
            vx: 1.0,
            vy: 1.0,
            color_index: 0,
            wall_hits: 0,
            // Replaced by `reset` below; this only has to be a valid generator.
            rng: seeded_rng(seed, "dvd"),
            options,
        };
        effect.reset();
        effect
    }

    /// The logo source as a grid of characters, one per dot.
    ///
    /// Blank-only rows are dropped, as they always were, which is what lets a
    /// wordmark be written with leading and trailing blank lines. An empty logo
    /// falls back to a single `*`, so there is always something to draw and
    /// `logo_dot_width` is never zero.
    fn parse_logo(logo: &str) -> Vec<Vec<char>> {
        let rows: Vec<Vec<char>> = logo
            .split('\n')
            .filter(|row| !row.trim().is_empty())
            .map(|row| row.chars().collect())
            .collect();

        if rows.is_empty() {
            vec![vec!['*']]
        } else {
            rows
        }
    }

    /// The widest row of the logo, in dots.
    fn logo_dot_width(&self) -> usize {
        self.rows.iter().map(Vec::len).fold(0, usize::max).max(1)
    }

    /// The logo's height, in dots.
    fn logo_dot_height(&self) -> usize {
        self.rows.len().max(1)
    }

    /// The logo's bounding box in cells.
    fn logo_cells_w(&self) -> usize {
        self.logo_dot_width().div_ceil(DOTS_X)
    }

    fn logo_cells_h(&self) -> usize {
        self.logo_dot_height().div_ceil(DOTS_Y)
    }

    /// The grid's width in cells: the logo's box plus one.
    ///
    /// The spare cell is what absorbs the sub-cell skew. A braille cell's two
    /// dots sit in *one* screen cell, so a logo whose origin is an odd number of
    /// dots across has its last column of dots pushed into the following cell --
    /// and without the spare column that column of ink is silently clipped, on
    /// half the frames, only at the wall.
    ///
    /// It is not a fudge. `x` is clamped to `max_x`, an integer number of cells,
    /// so at the wall the origin is always a whole number of dots and the logo
    /// never does need the spare cell there; it needs it at the skewed positions
    /// just short of the wall, where it lands in the last real cell. The spare
    /// cell is blank whenever the skew is zero, and the write skips blank cells,
    /// so it costs nothing.
    fn grid_w(&self) -> usize {
        self.logo_cells_w() + 1
    }

    fn grid_h(&self) -> usize {
        self.logo_cells_h() + 1
    }

    /// The logo's width in cells: dots across, rounded *up*.    ///
    /// This is the number the bounce has to respect, and it is the thing that
    /// stopped being a count of characters when the logo became a bitmap. A
    /// 60-dot row is 30 cells, not 60, and rounding up rather than down is what
    /// keeps the logo's right-hand column on screen when `x` is at `max_x` --
    /// `the_logo_is_never_clipped_and_never_escapes` is what would catch it the
    /// other way.
    fn logo_width(&self) -> f64 {
        self.logo_cells_w() as f64
    }

    /// The logo's height in cells: dots down, rounded up. Seven for the wordmark.
    fn logo_height(&self) -> f64 {
        self.logo_cells_h() as f64
    }

    /// Whether the logo's dot at `(dot_x, dot_y)` is inked.
    ///
    /// Coordinates are logo-local. Outside the source is blank, which is what
    /// clips the logo at its own bounding box.
    fn ink_at(&self, dot_x: usize, dot_y: usize) -> bool {
        self.rows
            .get(dot_y)
            .and_then(|row| row.get(dot_x))
            .is_some_and(|symbol| !is_blank(*symbol))
    }

    fn step(&mut self, delta: f64) {
        let logo_width = self.logo_width();
        let logo_height = self.logo_height();
        let max_x = (self.screen_size.0 as f64 - logo_width).max(0.0);
        let max_y = (self.screen_size.1 as f64 - logo_height).max(0.0);

        self.x += self.vx * delta;
        self.y += self.vy * delta;

        let mut hit_x = false;
        let mut hit_y = false;

        if max_x > 0.0 {
            if self.x <= 0.0 {
                self.x = 0.0;
                self.vx = self.vx.abs();
                hit_x = true;
            } else if self.x >= max_x {
                self.x = max_x;
                self.vx = -self.vx.abs();
                hit_x = true;
            }
        } else {
            self.x = 0.0;
            self.vx = 0.0;
        }

        if max_y > 0.0 {
            if self.y <= 0.0 {
                self.y = 0.0;
                self.vy = self.vy.abs();
                hit_y = true;
            } else if self.y >= max_y {
                self.y = max_y;
                self.vy = -self.vy.abs();
                hit_y = true;
            }
        } else {
            self.y = 0.0;
            self.vy = 0.0;
        }

        // Counted on every wall hit whatever the mode, so the counter means the
        // same thing to `Steady` as it does to a test reading it. Saturating,
        // because `Never` never resets it: at 60 wall hits a second that is a
        // quarter of a million years, but a debug build panics on the overflow
        // rather than wrapping, and a screensaver is exactly the thing that
        // nobody closes.
        if hit_x || hit_y {
            self.wall_hits = self.wall_hits.saturating_add(1);
        }

        if self
            .options
            .color_change
            .should_change(hit_x, hit_y, self.wall_hits)
        {
            self.color_index = (self.color_index + 1) % PALETTE.len();
            self.wall_hits = 0;
        }
    }

    /// Draws the logo at its current position into the canvas.
    ///
    /// Sampled *through* a [`BrailleGrid`] rather than rasterised into one, and
    /// that direction is the point. The logo moves at a sub-cell rate, so for
    /// each cell of the grid and each of its eight dots the source is asked
    /// whether *that* dot is inked, at the position the logo has actually
    /// reached. Writing the logo into a screen-sized grid first and encoding
    /// that grid works at whole-cell positions and nowhere else.
    ///
    /// The offset convention is the one the quadrant renderer used, and getting it
    /// wrong is the classic bug here because the failure is quiet: `left` and
    /// `top` are the *screen dot* positions of the logo's origin, and source dot
    /// `(i, j)` lands at `origin + (i, j)`. Round the same way in both places or
    /// the logo jitters against the position `step` reports.
    ///
    /// The old version did `self.x as usize` and `self.y as usize` at the top of
    /// its loops, which threw away every sub-cell part of the position `step` had
    /// carefully integrated. At the old default of nine cells per second that is
    /// 0.15 cells per frame, so the drawn position changed once every seven frames:
    /// the logo sat perfectly still for six frames out of seven and then jumped a
    /// whole cell diagonally. That is the "laggy, like stairs" report.
    ///
    /// Why braille, given quadrant was here first and is still in the crate.
    /// Quadrant spends a cell's foreground *and* its background on a two-tone
    /// split, so every edge cell pays an `ESC[48;2;r;g;b` on the wire and paints
    /// its un-inked half solid black -- which is also why the old logo was
    /// invisible against anything but a black terminal. The DVD wordmark is one
    /// flat colour, because a logo is a silhouette and not a gradient, so that
    /// second colour was buying nothing at all. Braille trades it for 2x across
    /// and 4x down rather than 2x and 2x, and an unraised dot leaves the
    /// terminal's own background showing through the logo's gaps.
    fn draw(&mut self) -> Vec<(usize, usize, Cell)> {
        let (r, g, b) = PALETTE[self.color_index % PALETTE.len()];
        let ink = style::Color::Rgb { r, g, b };

        self.grid.clear();
        // The grid is only as big as the logo, so the canvas still has to be
        // cleared here or the previous frame's logo stays on screen and the two
        // smear into each other.
        self.canvas.clear();

        // Position in dots, not cells, so the sub-cell part of the position is
        // drawn rather than discarded. A different multiplier per axis, on
        // purpose: braille is 2 across and 4 down, and one constant for both is
        // how the vertical resolution gets thrown away.
        let left = (self.x * DOTS_X as f64).floor().max(0.0) as usize;
        let top = (self.y * DOTS_Y as f64).floor().max(0.0) as usize;
        self.paint_at(left, top);

        // The grid's origin is the logo's origin in cells. `left` is already a
        // whole number of dots, so this is the screen cell holding the logo's
        // first dot -- and when the origin is skewed the logo's *last* dot is one
        // cell further on, which is what the spare cell in `grid_w` is for.
        let left_cell = left / DOTS_X;
        let top_cell = top / DOTS_Y;
        for cell_y in 0..self.grid.height() {
            let y = top_cell + cell_y;
            if y >= self.canvas.height() {
                break;
            }
            for cell_x in 0..self.grid.width() {
                // Only cells with ink are written. A blank braille cell is a
                // space, and painting one would cost a byte and erase whatever
                // the terminal's background is showing there -- which is what
                // lets `[global] background` show through the wordmark's gaps.
                let symbol = self.grid.cell_char(cell_x, cell_y);
                if symbol == ' ' {
                    continue;
                }
                let x = left_cell + cell_x;
                if x >= self.canvas.width() {
                    continue;
                }
                self.canvas.set(
                    x,
                    y,
                    Cell::new(symbol, ink, style::Attribute::Reset),
                );
            }
        }
        self.canvas.commit()
    }

    /// Raises the logo's dots into `self.grid`, with the logo's top-left dot at
    /// screen dot `(left, top)`.
    ///
    /// The grid is indexed by *screen* cell relative to the logo's own cell, not
    /// by the logo's dot, and the difference is the whole sub-cell offset. A
    /// braille cell's two dots live in one screen cell, so a logo whose origin
    /// falls on an odd dot is straddling a cell boundary: its last column of dots
    /// belongs to the *next* screen cell. Sampling the logo with
    /// `left + cell_x * DOTS_X + dx` instead -- which is the obvious transcription
    /// and what this did first -- indexes the source with a screen coordinate and
    /// therefore misses the logo entirely once `left` exceeds its width, drawing
    /// nothing at all.
    ///
    /// So the source index is the grid's own dot position less the skew: `left % 2`
    /// across and `top % 4` down, each handled as a signed offset so a negative
    /// result is blank rather than a huge `usize`. `the_logo_is_sampled_at_its
    /// _sub_cell_offset` is what catches getting this wrong.
    ///
    /// The loop runs over the grid, which is the logo's box plus one cell, and not
    /// over the terminal: 31 by 8 cells rather than 400 by 200.
    fn paint_at(&mut self, left: usize, top: usize) {
        let skew_x = (left % DOTS_X) as isize;
        let skew_y = (top % DOTS_Y) as isize;

        for cell_y in 0..self.grid.height() {
            for cell_x in 0..self.grid.width() {
                for dy in 0..DOTS_Y {
                    for dx in 0..DOTS_X {
                        let dot_x = (cell_x * DOTS_X + dx) as isize - skew_x;
                        let dot_y = (cell_y * DOTS_Y + dy) as isize - skew_y;
                        let raised = dot_x >= 0
                            && dot_y >= 0
                            && self.ink_at(dot_x as usize, dot_y as usize);
                        self.grid.set_dot(
                            cell_x * DOTS_X + dx,
                            cell_y * DOTS_Y + dy,
                            raised,
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn options() -> DvdOptions {
        DvdOptions {
            logo: String::from("DVD"),
            start_in_corner: false,
            ..Default::default()
        }
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

    fn luminance(color: (u8, u8, u8)) -> f64 {
        0.2126 * linear(color.0)
            + 0.7152 * linear(color.1)
            + 0.0722 * linear(color.2)
    }

    fn contrast(a: f64, b: f64) -> f64 {
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn the_first_colour_is_not_a_near_white_one() {
        // `color_index` is randomised at construction, so this is the colour a
        // run actually starts on one time in six. It used to be
        // `(235, 235, 245)`, which is 1.03:1 against a white background: not
        // merely dull, invisible.
        let (r, g, b) = PALETTE[0];
        assert!(
            r.min(g).min(b) < 200,
            "PALETTE[0] is ({r}, {g}, {b}), close enough to white to disappear on \
             a light profile"
        );
        assert_ne!(PALETTE[0], (235, 235, 245));
    }

    #[test]
    fn every_colour_reads_on_a_light_and_on_a_dark_profile() {
        // Not only the first: `color_index` is randomised, so any entry can be
        // the colour a run starts on. Fixing one and leaving the other five
        // invisible would have left the same bug five times out of six.
        for color in PALETTE {
            let lum = luminance(color);
            let on_white = contrast(lum, 1.0);
            let on_black = contrast(lum, 0.0);

            assert!(
                on_white >= 3.0,
                "{color:?} is only {on_white:.1}:1 on a light profile"
            );
            assert!(
                on_black >= 3.0,
                "{color:?} is only {on_black:.1}:1 on a dark profile"
            );
        }
    }

    #[test]
    fn a_run_always_starts_on_a_legible_colour() {
        // The end-to-end version of the two above: whatever the seed picks, the
        // logo is drawn in something that can be seen.
        for seed in 0..64u64 {
            let mut effect = Dvd::new(
                DvdOptions {
                    seed,
                    start_in_corner: true,
                    ..Default::default()
                },
                (40, 12),
            );
            effect.get_diff();

            let (r, g, b) = PALETTE[effect.color_index];
            let lum = luminance((r, g, b));
            assert!(
                contrast(lum, 1.0) >= 3.0 && contrast(lum, 0.0) >= 3.0,
                "seed {seed} started on {r},{g},{b}"
            );
        }
    }

    #[test]
    fn new_tracks_screen_size() {
        let effect = Dvd::new(options(), (80, 24));
        assert_eq!(effect.screen_size, (80, 24));
        assert_eq!(effect.rows.len(), 1);
    }

    #[test]
    fn new_terminates_at_minimum_size() {
        let effect = Dvd::new(options(), (0, 0));
        assert_eq!(effect.screen_size, (1, 1));
    }

    #[test]
    fn supports_multiline_logos() {
        let mut effect = Dvd::new(options(), (40, 10));
        effect.options.logo = String::from("ab\ncd");
        effect.reset();

        assert_eq!(effect.rows, vec![vec!['a', 'b'], vec!['c', 'd']]);
    }

    #[test]
    fn blank_logo_falls_back_to_a_single_glyph() {
        let mut effect = Dvd::new(options(), (40, 10));
        effect.options.logo = String::from("   \n  ");
        effect.reset();

        assert_eq!(effect.rows, vec![vec!['*']]);
    }

    /// The logo is a bitmap, so its width in cells is a rounding of its width in
    /// dots, and the bounce has to respect the cell count.
    ///
    /// `options()` gives a three-*dot* logo, which is two cells across at 2 dots
    /// to the cell -- not three. An earlier version of this assertion said
    /// `20 - 3`, which was right when a character was a cell and is one cell too
    /// loose now, so a test like this cannot be left asserting a number from the
    /// old coordinate system.
    #[test]
    fn stays_within_bounds_while_bouncing() {
        let mut effect = Dvd::new(options(), (20, 8));
        assert_eq!(effect.logo_width(), 2.0, "three dots is two cells");
        for _ in 0..600 {
            effect.step(1.0 / 60.0);
            assert!(effect.x >= 0.0 && effect.x <= (20 - 2) as f64);
            assert!(effect.y >= 0.0 && effect.y <= (8 - 1) as f64);
        }
    }

    #[test]
    fn oversize_logo_is_pinned_to_the_origin() {
        let mut effect = Dvd::new(options(), (2, 1));
        effect.options.logo = String::from("TOOLONG");
        effect.reset();

        for _ in 0..120 {
            effect.step(1.0 / 60.0);
            assert_eq!(effect.x, 0.0);
            assert_eq!(effect.y, 0.0);
        }
    }

    /// Every wall hit has to change the colour, in [`ColorChange::Bounce`].
    ///
    /// This is the reference implementation's behaviour, and it is a real look
    /// that `Bounce` still offers. It is no longer the default -- see
    /// `the_colour_changes_far_less_often_than_the_logo_bounces` -- so this test
    /// pins the mode rather than the default.
    ///
    /// The previous option was `corner_color_change`, and corners are almost
    /// unreachable: a corner needs both axes to reverse on the same frame, and the
    /// bounce periods are `2 * max_x / vx` and `2 * max_y / vy`. This uses the
    /// three-*dot* `options()` logo rather than the wordmark, so `max_x` is 78 and
    /// `max_y` is 23 and the periods are 8.7 and 5.1 seconds -- the two only
    /// coincide after 39 and 23 of them, which is over three minutes. So
    /// corner-only was the same as never changing.
    ///
    /// The run is 1800 frames rather than 600 because a wall hit is rarer than it
    /// used to be: the cap came down from 24 to 18 cells a second (see
    /// `DvdOptions::default`) and a wall hit is a traversal, so ten seconds now
    /// buys three of them rather than four. Thirty seconds buys about nine, which
    /// clears the bound with room, and is still two orders of magnitude short of
    /// what a corner needs -- so the test would still fail against corner-only.
    #[test]
    fn every_wall_hit_changes_the_colour() {
        let mut effect = Dvd::new(
            DvdOptions {
                color_change: ColorChange::Bounce,
                ..options()
            },
            (80, 24),
        );
        let initial = effect.color_index;

        let mut changes = 0usize;
        for _ in 0..1800 {
            effect.step(1.0 / 60.0);
            if effect.color_index != initial {
                changes += 1;
            }
        }

        assert!(
            changes >= 3,
            "the colour changed {changes} times in thirty seconds of bouncing, so \
             the logo is effectively one colour. A wall hit every few seconds is \
             what makes this read as motion rather than as a still image drifting."
        );
    }

    /// The colour changes on every wall hit, which is what the reference does
    /// and what was asked for.
    ///
    /// This asserted the *opposite* for a while, and the reason it did is worth
    /// keeping: the belief was that a recolour of a solid 30x7 slab is a large
    /// simultaneous change and therefore a strobe. The diagnosis was wrong. The
    /// flicker was the logo moving -- a braille glyph is its bit pattern, so a
    /// one-dot step rewrites the whole letterform -- and at the speed this effect
    /// ran at then, wall hits were 1.4 seconds apart. At 5 cells a second they
    /// are about eight, and a colour change is an event again.
    ///
    /// So this is now a guard in the other direction: it fails if the default
    /// drifts back to a throttled mode, because that would be quietly
    /// reintroducing the decision this commit reversed.
    #[test]
    fn the_colour_changes_on_every_bounce() {
        let mut effect = Dvd::new(DvdOptions::default(), (80, 24));
        assert_eq!(
            effect.options.color_change,
            ColorChange::Bounce,
            "the default has to be a recolour on every wall hit, or the logo sits \
             at one colour for whole stretches of its run"
        );

        let mut seen = std::collections::BTreeSet::new();
        let mut previous = effect.color_index;
        seen.insert(previous);
        let mut bounces = 0u32;
        let mut changes = 0usize;

        // Stop on a bounce count rather than a frame count, so the run is the
        // same measurement whatever the speed and the screen are.
        //
        // A wall hit is detected by the velocity reversing, not by the position
        // standing still. `step` clamps the position to the wall, so on the frame
        // of a bounce the position has still moved by the overshoot -- a tenth of
        // a cell or so -- and a "did not move" test misses every single bounce.
        // Only the velocity is a clean signal, because `step` changes it if and
        // only if it clamped.
        while bounces < 90 {
            let (vx, vy) = (effect.vx, effect.vy);
            effect.step(1.0 / 60.0);
            if effect.vx != vx || effect.vy != vy {
                bounces += 1;
            }
            if effect.color_index != previous {
                previous = effect.color_index;
                changes += 1;
                seen.insert(effect.color_index);
            }
        }

        assert!(
            bounces >= 90,
            "only saw {bounces} wall hits, so the simulation never got going"
        );
        // Every wall hit, and no more than every wall hit: a mode that changed
        // colour more often than it bounced would be one the bounce counter
        // cannot see, which is how a bug here would hide.
        assert!(
            changes as u32 >= bounces,
            "{changes} colour changes over {bounces} wall hits, so some bounces \
             did not change the colour"
        );
        assert!(
            changes as u32 <= bounces * 2,
            "{changes} colour changes over {bounces} wall hits, so the colour is \
             changing on frames that are not bounces"
        );
        // A sanity floor as well as a ceiling. A rate of zero would satisfy the
        // bound above and is the other failure: the logo would be one colour for
        // the whole run, which is what the old corner-only rule was.
        assert!(
            changes >= 3,
            "only {changes} colour changes over {bounces} wall hits, so the logo \
             is effectively one colour"
        );
        assert!(
            seen.len() > 1 && seen.len() <= PALETTE.len(),
            "{} distinct colours seen, which is not a rotation of the palette",
            seen.len()
        );
    }

    /// [`ColorChange::Bounce`] has to keep changing on *every* hit, or the
    /// contrast with [`ColorChange::Steady`] is not a contrast.
    ///
    /// The same simulation as above, one mode over.
    #[test]
    fn the_reference_mode_changes_on_every_bounce() {
        let mut effect = Dvd::new(
            DvdOptions {
                color_change: ColorChange::Bounce,
                ..DvdOptions::default()
            },
            (80, 24),
        );
        let initial = effect.color_index;
        let mut bounces = 0u32;
        let mut changes = 0usize;
        let mut previous = initial;

        while bounces < 90 {
            let (vx, vy) = (effect.vx, effect.vy);
            effect.step(1.0 / 60.0);
            if effect.vx != vx || effect.vy != vy {
                bounces += 1;
            }
            if effect.color_index != previous {
                previous = effect.color_index;
                changes += 1;
            }
        }

        assert_eq!(
            changes, bounces as usize,
            "Bounce mode changed the colour on {changes} of {bounces} wall hits; it \
             is meant to be every one of them"
        );
        assert_ne!(initial, 0, "unreachable, but keeps `initial` read");
    }

    /// The corner mode has to stay available, because it is the easter egg.
    ///
    /// The reference README jokes that "it could hit the corner if you look at it
    /// long enough", and that is a real thing to be able to switch on.
    #[test]
    fn corner_mode_changes_only_when_both_axes_reverse_together() {
        // Small and square-ish, so the two bounce periods are close and coincide
        // quickly. The old test relied on exactly this, at 6x2.
        for (width, height) in [(6u16, 2u16), (20, 20)] {
            let mut effect = Dvd::new(
                DvdOptions {
                    color_change: ColorChange::Corner,
                    ..options()
                },
                (width, height),
            );
            let initial = effect.color_index;
            let mut changes = 0usize;
            let mut bounces = 0usize;

            for _ in 0..20_000 {
                // A wall hit is a velocity reversing, not a position standing
                // still: `step` clamps the position to the wall, so the position
                // has still moved by the overshoot on the frame of a bounce and a
                // "did not move" test misses them all.
                let (vx, vy) = (effect.vx, effect.vy);
                effect.step(1.0 / 60.0);
                if effect.vx != vx || effect.vy != vy {
                    bounces += 1;
                }
                if effect.color_index != initial {
                    changes += 1;
                    effect.color_index = initial;
                }
            }

            assert!(
                bounces > changes,
                "at {width}x{height} the colour changed {changes} times over \
                 {bounces} wall hits, so corner mode is firing on ordinary \
                 bounces"
            );
        }
    }

    /// `Never` has to actually never.
    #[test]
    fn never_mode_holds_one_colour() {
        let mut effect = Dvd::new(
            DvdOptions {
                color_change: ColorChange::Never,
                ..options()
            },
            (80, 24),
        );
        let initial = effect.color_index;
        for _ in 0..5_000 {
            effect.step(1.0 / 60.0);
            assert_eq!(effect.color_index, initial);
        }
    }

    /// The rule itself, for all four modes, without a simulation in the way.
    ///
    /// The third argument is the running count of wall hits since the last
    /// recolour, and only [`ColorChange::Steady`] reads it. The `wall_hits` values
    /// are swept from zero past the gate, so the boundary itself is pinned rather
    /// than assumed, and the modes that ignore the counter are checked at every
    /// one of them -- which is the whole difference between `Bounce` and `Steady`.
    #[test]
    fn the_colour_rule_is_what_each_mode_says() {
        for wall_hits in 0..=RECOLOR_EVERY_N_BOUNCES + 3 {
            // The reference mode ignores the counter entirely, which is the whole
            // difference between it and the default.
            for (hit_x, hit_y) in [(true, false), (false, true), (true, true)] {
                assert!(ColorChange::Bounce.should_change(hit_x, hit_y, wall_hits));
            }
            assert!(!ColorChange::Bounce.should_change(false, false, wall_hits));

            assert!(!ColorChange::Corner.should_change(true, false, wall_hits));
            assert!(!ColorChange::Corner.should_change(false, true, wall_hits));
            assert!(ColorChange::Corner.should_change(true, true, wall_hits));
            assert!(!ColorChange::Corner.should_change(false, false, wall_hits));

            for (hit_x, hit_y) in [(true, false), (false, true), (true, true)] {
                assert!(!ColorChange::Never.should_change(hit_x, hit_y, wall_hits));
            }

            // The gate: a wall hit that came too early is dropped, and a frame
            // that was not a wall hit is dropped whatever the count says.
            let expected = wall_hits >= RECOLOR_EVERY_N_BOUNCES;
            for (hit_x, hit_y) in [(true, false), (false, true), (true, true)] {
                assert_eq!(
                    ColorChange::Steady.should_change(hit_x, hit_y, wall_hits),
                    expected,
                    "a wall hit at {wall_hits} wall hits since the last change"
                );
            }
            assert!(!ColorChange::Steady.should_change(false, false, wall_hits));
        }
    }

    #[test]
    fn update_with_context_scales_with_delta() {
        let mut fast = Dvd::new(options(), (40, 12));
        let mut slow = Dvd::new(options(), (40, 12));
        fast.x = 0.0;
        slow.x = 0.0;
        fast.vx = 10.0;
        slow.vx = 10.0;
        fast.vy = 0.0;
        slow.vy = 0.0;

        let fast_context = crate::runtime::FrameContext::new(
            (40, 12),
            0,
            Duration::ZERO,
            Duration::from_millis(200),
            crate::runtime::InputState::default(),
        );
        let slow_context = crate::runtime::FrameContext::new(
            (40, 12),
            0,
            Duration::ZERO,
            Duration::from_millis(20),
            crate::runtime::InputState::default(),
        );

        fast.update_with_context(&fast_context);
        slow.update_with_context(&slow_context);

        assert!(fast.x > slow.x);
    }

    /// The picture has to change on almost every frame.
    ///
    /// This is the "it feels laggy, like stairs" complaint, measured.
    ///
    /// `draw` used to do `self.x as usize` and `self.y as usize`, discarding every
    /// sub-cell part of the position `step` had integrated. At the old default of
    /// nine cells per second that is 0.15 cells per frame, so the drawn position
    /// changed once every seven frames: still for six frames out of seven, then a
    /// jump. That is 86 changed frames in 600.
    ///
    /// Measured through the effect's own diff rather than by inspecting the canvas,
    /// for two reasons. It is what the terminal actually receives, and it is the
    /// only measure that works for a sub-cell renderer: with the logo placed at 2x2,
    /// the *cell* holding its top-left ink still only changes once per whole cell
    /// of travel, so a test watching the cell reports a staircase even while the
    /// glyph inside it moves every frame. A version of this test did exactly that
    /// and reported 280 of 600 while the screen was in fact changing 545 times.
    ///
    /// The bound is 500 rather than 600 because the sub-cell resolution caps it,
    /// and it is close: at 18 cells a second the horizontal origin advances 36
    /// dots a second and the vertical 36, against 60 frames, so a frame changes the
    /// picture if *either* axis crosses a dot boundary. That is 0.6 of a chance
    /// each, and 1 - 0.4 * 0.4 = 0.84 -- which is 504 of 600, and the measurement
    /// is 503. The bound is the one number here that is genuinely tight, and it is
    /// tight for a reason worth stating: the horizontal is still 2 dots to the cell,
    /// because a cell has one foreground and one background and a vertical split
    /// spends both. Only the vertical went from 2 to 4.
    #[test]
    fn the_picture_changes_on_almost_every_frame() {
        let mut dvd = Dvd::new(DvdOptions::default(), (80, 24));
        let _ = dvd.get_diff();

        let mut changed = 0usize;
        let mut previous = (dvd.x, dvd.y);
        let mut worst_step = 0.0f64;
        for _ in 0..600 {
            dvd.update();
            let diff = dvd.get_diff();
            if !diff.is_empty() {
                changed += 1;
            }
            worst_step = worst_step.max(
                ((dvd.x - previous.0).powi(2) + (dvd.y - previous.1).powi(2))
                    .sqrt(),
            );
            previous = (dvd.x, dvd.y);
        }

        // Two bounds, and the second is the one that matters.
        //
        // The first says the logo is moving often enough to read as motion. At 5
        // cells a second the origin advances 10 dots horizontally and 10
        // vertically against 60 frames, so a frame changes the picture if either
        // axis crosses a dot boundary -- and the measurement is 176 of 600, or
        // 29%. It used to assert 500, which was correct at 18 cells a second and
        // is not a property of anything.
        //
        // The second is the actual anti-staircase claim, and it is the one that
        // was missing. "Changes on most frames" is a proxy for "moves smoothly",
        // and a proxy that can be satisfied by moving a whole cell at a time on
        // rare frames -- which is precisely the defect it was written to catch.
        // The direct statement is that no single frame moves the logo more than
        // a fraction of a cell. At 5 cells a second and 60 Hz that is 0.083 of a
        // cell; a whole-cell jump would be twelve times larger, and the old
        // block letter stepped exactly one cell at four times this rate.
        assert!(
            changed > 150,
            "the screen changed on only {changed} of 600 frames, so the logo is \
             mostly stationary with the occasional move"
        );
        assert!(
            worst_step < 0.2,
            "the logo jumped {worst_step:.3} of a cell in one frame, which is a \
             staircase rather than motion"
        );
        // No single frame may move the logo more than a cell and a half. A
        // bounce clamps the position to the wall, which is a snap of up to one
        // cell, so the bound is above one rather than at it. This is the check
        // that would catch a genuine teleport, and it is on the position rather
        // than on the diff: a bounce legitimately repaints the logo's whole
        // leading edge, which is most of its area, so diff size cannot tell a
        // bounce from a jump.
        assert!(
            worst_step < 1.5,
            "the logo moved {worst_step:.2} cells in one frame, which is a jump \
             rather than motion"
        );
    }

    /// The vertical position has to resolve finer than a whole cell.
    ///
    /// Half-block was the first attempt and gave 2x vertically; quadrant gave 2x
    /// as well; braille gives 4x. This pins the axis, and it is the axis the
    /// *other* renderers could not improve: horizontally everything in the crate
    /// is 2x, because a cell has one foreground and one background and a vertical
    /// split spends both.
    #[test]
    fn the_vertical_position_resolves_finer_than_a_cell() {
        let mut dvd = Dvd::new(DvdOptions::default(), (80, 24));
        let mut dots = std::collections::BTreeSet::new();
        let mut cells = std::collections::BTreeSet::new();
        // 480 frames rather than 60. The ratio is a ratio of two *distinct*
        // counts over a finite window, and at 5 cells a second the vertical
        // travels 20 cells in that time against 2.5 in one second -- so a
        // one-second window is only ten dot boundaries, and the discreteness of
        // two small samples dominated the ratio. Measured 3.33 over 60 frames and
        // 3.8 over 480, both of which are the sub-cell axis working; the short
        // window was just too short to show it.
        for _ in 0..480 {
            dvd.update();
            dots.insert((dvd.y * DOTS_Y as f64).floor() as i64);
            cells.insert(dvd.y.floor() as i64);
        }
        // The ratio, not an absolute count: the absolute number moves with the
        // speed and the terminal size, and a threshold on it is a threshold on the
        // wrong thing. Four dots per cell means the drawn positions must
        // outnumber the cell positions by close to four to one. The bound is 3.5
        // rather than 4 because the ratio is a ratio of two *distinct* counts over
        // a finite window, and a bounce can make the two agree by coincidence.
        let ratio = dots.len() as f32 / cells.len().max(1) as f32;
        assert!(
            ratio > 3.5,
            "{} distinct drawn vertical positions against {} at whole-cell \
             resolution, a ratio of {ratio:.2}, so the sub-cell vertical motion \
             is not being drawn",
            dots.len(),
            cells.len()
        );
    }

    /// The wordmark literal, checked as a literal.
    ///
    /// 1680 characters of `#` and `.` is a shape no eye can check and no review
    /// will read, so the dimensions are asserted instead: a mistyped row is a row
    /// of the wrong length, and one dropped row is one row fewer. Both are
    /// invisible in a diff and both are fatal here, because a short row silently
    /// truncates the logo's right-hand side rather than failing.
    #[test]
    fn the_wordmark_is_sixty_dots_by_twenty_eight() {
        let rows: Vec<&str> = DVD_WORDMARK.split('\n').collect();
        assert_eq!(
            rows.len(),
            28,
            "the wordmark is {} rows, not 28",
            rows.len()
        );
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.chars().count(),
                60,
                "row {index} is {} dots wide, not 60: {row:?}",
                row.chars().count()
            );
        }
        // And nothing else is in there. A stray character would be read as ink,
        // so a stray space would put a hole in the logo and a stray letter would
        // put a lump in it, both without a word about it.
        let alphabet: std::collections::BTreeSet<char> =
            DVD_WORDMARK.chars().filter(|c| *c != '\n').collect();
        assert_eq!(
            alphabet,
            ['#', '.'].into_iter().collect(),
            "the wordmark alphabet is {alphabet:?}, not `#` and `.`"
        );
    }

    /// The wordmark has to be letters, not a slab and not nothing.
    ///
    /// A cheap structural guard on a literal, and the numbers are checked as a
    /// *band* rather than as an exact count: 1004 of 1680 dots is 0.60, which is
    /// about right for a heavy wordmark with a reflection under it -- the two
    /// together are mostly ink, and the gaps are inside the counters of the D and
    /// the V and between the letters. A band of 0.35 to 0.80 catches the two
    /// failures that matter, which are a truncated row (coverage collapses) and a
    /// transposed one (coverage stays plausible but the shape is wrong, which no
    /// coverage test can see -- the round-trip test below is what covers that).
    #[test]
    fn the_wordmark_is_neither_blank_nor_a_solid_slab() {
        let total: usize = DVD_WORDMARK.chars().filter(|c| *c != '\n').count();
        let ink = DVD_WORDMARK.chars().filter(|c| *c == '#').count();
        let coverage = ink as f64 / total as f64;

        assert!(
            coverage > 0.35 && coverage < 0.80,
            "the wordmark is {:.0}% inked ({ink} of {total} dots), which is \
             outside the band a wordmark with a reflection sits in",
            coverage * 100.0
        );
        // Ink and gaps, in the other sense: no row may be entirely one or the
        // other. The reflection rows in the middle are the sparsest, and a
        // mistyped one of those is a whole row gone.
        for (index, row) in DVD_WORDMARK.split('\n').enumerate() {
            let dots = row.chars().count();
            let inked = row.chars().filter(|c| *c == '#').count();
            assert!(
                inked > 0 && inked < dots,
                "row {index} is {inked} of {dots} dots, so it is entirely blank or \
                 entirely ink: {row:?}"
            );
        }
    }

    /// The logo is sampled at its sub-cell offset, not at a cell boundary.
    ///
    /// This is the bug the sub-cell offset invites, and it is worth writing down
    /// because the first version of this renderer had it in the most obvious
    /// possible form: it indexed the source with a *screen* coordinate, so
    /// `ink_at(left + cell_x * 2 + dx, ...)` -- which reads correctly while the
    /// logo is near the origin and then walks straight off the end of a 60-dot
    /// row, drew *nothing at all*. A blank screen, no panic, no warning.
    ///
    /// A braille cell's two dots live in one screen cell, so a logo whose origin
    /// lands on an odd dot straddles a cell boundary: dot 0 is the right-hand dot
    /// of one cell and dot 59 the left-hand dot of the one after. The grid has to
    /// be a whole cell bigger than the logo for that to fit, and it has to be
    /// sampled with a signed skew rather than a saturating one, or the last column
    /// is clipped on half the frames.
    ///
    /// Two assertions, and the first is the one that would have caught it:
    ///
    /// * the number of raised dots is the same for every skew, which says no ink
    ///   is lost off the end and none is duplicated across the boundary, and
    /// * the pattern at skew `s` is the pattern at skew 0 shifted by exactly `s`
    ///   dots, which pins the convention rather than just its result.
    #[test]
    fn the_logo_is_sampled_at_its_sub_cell_offset() {
        let mut dvd = Dvd::new(DvdOptions::default(), (80, 24));
        let (dot_w, dot_h) = (dvd.logo_dot_width(), dvd.logo_dot_height());
        let total_ink = (0..dot_w)
            .flat_map(|x| (0..dot_h).map(move |y| (x, y)))
            .filter(|(x, y)| dvd.ink_at(*x, *y))
            .count();

        dvd.grid.clear();
        dvd.paint_at(0, 0);
        let aligned: Vec<Vec<bool>> = (0..dot_h)
            .map(|y| (0..dot_w).map(|x| dvd.grid.dot(x, y)).collect())
            .collect();

        // Skews are 0 or 1 across and 0 to 3 down: the grid's own granularity.
        for skew_x in 0..DOTS_X {
            for skew_y in 0..DOTS_Y {
                dvd.grid.clear();
                dvd.paint_at(skew_x, skew_y);

                let raised = (0..dvd.grid.dot_width())
                    .flat_map(|x| (0..dvd.grid.dot_height()).map(move |y| (x, y)))
                    .filter(|(x, y)| dvd.grid.dot(*x, *y))
                    .count();
                assert_eq!(
                    raised, total_ink,
                    "at a skew of ({skew_x},{skew_y}) the grid holds {raised} raised \
                     dots rather than the wordmark's {total_ink}, so ink has been \
                     lost or doubled"
                );

                for (y, row) in aligned.iter().enumerate() {
                    for (x, expected) in row.iter().enumerate() {
                        assert_eq!(
                            dvd.grid.dot(x + skew_x, y + skew_y),
                            *expected,
                            "dot ({x},{y}) moved wrong at a skew of \
                             ({skew_x},{skew_y})"
                        );
                    }
                }
            }
        }

        // And the spare cell and row exist for a reason: the grid is one cell
        // bigger than the logo, and that cell has to be able to hold ink, or the
        // skewed pattern above would have been clipped and the dot count would
        // have come out short.
        assert_eq!(dvd.grid.width(), dvd.logo_cells_w() + 1);
        assert_eq!(dvd.grid.height(), dvd.logo_cells_h() + 1);
        dvd.grid.clear();
        dvd.paint_at(1, 1);
        assert!(
            dvd.grid.dot_width() > dot_w && dvd.grid.dot_height() > dot_h,
            "the grid is {}x{} dots, which does not have room for a logo of \
             {dot_w}x{dot_h} shifted by a dot",
            dvd.grid.dot_width(),
            dvd.grid.dot_height()
        );
    }

    /// The wordmark survives the trip through the renderer.
    ///
    /// The art was supplied as braille, so the only way to know the transcription
    /// is right is to encode it back and look at the result. This drives the
    /// effect's own `paint_at` at the origin -- not a reimplementation of it, or
    /// it would agree with a broken `paint_at` -- and then checks the encoding
    /// against the source: the box is 30 by 7 cells, and two named dots land
    /// where the bitmap says. The first of those is the top-left inked dot of the
    /// left stroke of the D and the second is the blank above it, so a renderer
    /// that shifted its origin by a cell or a row would fail on one or both.
    #[test]
    fn the_wordmark_round_trips_through_braille() {
        let mut dvd = Dvd::new(DvdOptions::default(), (80, 24));
        dvd.grid.clear();
        dvd.paint_at(0, 0);

        assert_eq!(dvd.logo_cells_w(), 30, "60 dots across at 2 to the cell");
        assert_eq!(dvd.logo_cells_h(), 7, "28 dots down at 4 to the cell");
        assert_eq!(dvd.grid.width(), 31, "the logo's 30 cells plus the spare");
        assert_eq!(dvd.grid.height(), 8, "seven cells plus the spare");
        assert_eq!(dvd.grid.dot_width(), 62);
        assert_eq!(dvd.grid.dot_height(), 32);

        // Named dots rather than a comparison against the source, because
        // comparing against the source would agree with a broken `paint_at` by
        // construction. These are what a shifted origin, a transposed one, or a
        // row dropped from the literal all break. Row 0 of the wordmark is
        // `.....#######################.........#################......`,
        // row 4 opens the counters, rows 16 to 18 are the point of the V, and
        // rows 19 to 27 are the reflection.
        for (dot_x, dot_y, expected, what) in [
            (0usize, 0usize, false, "the top-left corner"),
            (5, 0, true, "the left stroke of the first D"),
            (4, 0, false, "the blank one dot to its left"),
            (28, 0, false, "the gap between the D and the V at the top"),
            (37, 0, true, "the top of the second letter's left stroke"),
            (53, 0, true, "the far end of that letter's top bar"),
            (54, 0, false, "the blank beyond it"),
            (59, 0, false, "the top-right corner"),
            (10, 4, false, "inside the first D's counter"),
            (34, 4, true, "a stem of the V"),
            (52, 4, true, "the left stroke of the last D"),
            (27, 16, true, "the point of the V"),
            (26, 16, false, "just left of the point"),
            (20, 19, true, "the top bar of the reflection"),
            (19, 19, false, "just left of it"),
            (30, 20, true, "the middle of the reflection"),
            (14, 27, true, "the foot of the reflection"),
            (13, 27, false, "just left of the foot"),
            (43, 27, false, "just right of the foot"),
            (0, 27, false, "the bottom-left corner"),
        ] {
            assert_eq!(
                dvd.grid.dot(dot_x, dot_y),
                expected,
                "dot ({dot_x},{dot_y}) -- {what}"
            );
        }

        // And every dot of the grid agrees with the source, which is what makes
        // the named dots above a summary rather than the whole test.
        for dot_y in 0..dvd.grid.dot_height() {
            for dot_x in 0..dvd.grid.dot_width() {
                assert_eq!(
                    dvd.grid.dot(dot_x, dot_y),
                    dvd.ink_at(dot_x, dot_y),
                    "dot ({dot_x},{dot_y}) disagrees with the wordmark"
                );
            }
        }

        // And the encoding is braille, by name, for three cells whose bit patterns
        // are worked out by hand from the bitmap above. U+28FF is all eight dots,
        // which is the property a logo needs from a sub-cell renderer: a solid
        // interior has to come out solid, or the letter is visibly striped. U+28EA
        // is the right-hand column of a cell raised on all four rows plus the
        // bottom-left dot, which is the slanted left edge of the first D. The
        // fourth is the top-left corner, which has to be a space rather than a
        // blank pattern -- `BrailleGrid` emits a space for an empty cell because
        // it is one byte narrower, and that is the byte-cost half of only writing
        // cells that have ink.
        assert_eq!(dvd.grid.cell_char(2, 1), '\u{28ff}', "a solid interior");
        assert_eq!(dvd.grid.cell_char(2, 0), '\u{28ea}', "the slanted edge");
        assert_eq!(dvd.grid.cell_char(0, 0), ' ', "a cell with no ink");
        assert_eq!(dvd.grid.cell_char(29, 6), ' ', "the far bottom corner");
    }

    /// The logo is big.
    ///
    /// It was three characters on one line, which is a text label rather than a
    /// logo, and then a 23-cell block letter, which is bigger but not the thing.
    /// It was previously pinned by name in an integration test too, which had to
    /// be updated with it. The assertion follows the effect: 30 cells by 7 is the
    /// wordmark, at braille's 2x4, and it is exactly a quarter of an 80x24
    /// terminal's width, which is about what a bouncing logo should occupy.
    #[test]
    fn the_default_logo_is_a_real_wordmark() {
        let dvd = Dvd::new(DvdOptions::default(), (80, 24));
        assert_eq!(dvd.rows.len(), 28, "28 dot rows");
        assert_eq!(
            dvd.rows.iter().map(Vec::len).max().unwrap_or(0),
            60,
            "60 dots across"
        );
        assert_eq!(dvd.logo_width(), 30.0, "60 dots is 30 cells");
        assert_eq!(dvd.logo_height(), 7.0, "28 dots is 7 cells");
        assert_eq!(dvd.grid.width(), 31, "the logo's cells plus the skew spare");
        assert_eq!(dvd.grid.height(), 8);
    }

    /// The logo has to stay whole and stay on screen, at every position.
    ///
    /// `max_x` is `screen_width - logo_cells` and the draw is at
    /// `left_cell + cell_x`, so that bounds math is what keeps the logo's
    /// right-hand column on screen at the wall. If the width in cells were
    /// rounded the other way -- down, to 29 for a 58-dot logo -- the last column
    /// would be off the edge at the far wall, and `Canvas::set` would drop the
    /// write rather than panicking, so the right-hand sliver of the D would
    /// quietly disappear for a moment on every crossing.
    ///
    /// The positions checked are the four corners *and* the skewed positions just
    /// short of them, because those are different cases. At a wall `x` is a whole
    /// number of cells, so the origin is a whole number of dots and the logo
    /// occupies exactly its 30 cells. Half a cell short, the origin is an odd dot,
    /// the logo's last column of dots moves into the 31st cell, and that cell has
    /// to be the last *real* one rather than off the end.
    ///
    /// The check decodes what was written rather than counting it. A braille
    /// glyph is its own bit pattern, so a cell's glyph says exactly which of its
    /// eight dots are raised -- which means the whole claim can be checked dot by
    /// dot: every dot of the wordmark must be raised in the cell that contains it,
    /// and nothing else may be. That is stronger than a cell count (which changes
    /// with the skew, because shifting a pattern one dot moves ink between cells)
    /// and it catches clipping, doubling and a mis-signed offset alike.
    #[test]
    fn the_logo_is_never_clipped_and_never_escapes() {
        /// The raised dots of a braille glyph, or zero for anything else -- a
        /// space saturates to nothing rather than wrapping.
        fn dots_of(symbol: char) -> u8 {
            u32::from(symbol).saturating_sub(0x2800) as u8
        }

        let (width, height) = (80u16, 24u16);
        // Painted rather than read off a fresh grid: `draw` clears the grid before
        // filling it, so a `Dvd` that has never been drawn has an empty one.
        let mut template = Dvd::new(DvdOptions::default(), (width, height));
        template.grid.clear();
        template.paint_at(0, 0);
        let max_x = width as f64 - template.logo_width();
        let max_y = height as f64 - template.logo_height();
        let (dot_w, dot_h) =
            (template.logo_dot_width(), template.logo_dot_height());

        for x in [0.0, 0.5, max_x - 0.5, max_x] {
            for y in [0.0, 0.25, max_y - 0.25, max_y] {
                let mut dvd = Dvd::new(DvdOptions::default(), (width, height));
                dvd.x = x;
                dvd.y = y;
                let diff = dvd.get_diff();

                let mut written: std::collections::BTreeMap<(usize, usize), u8> =
                    Default::default();
                for (cx, cy, cell) in diff {
                    assert!(
                        cx < width as usize && cy < height as usize,
                        "at ({x}, {y}) the draw emitted ({cx},{cy}), outside the \
                         terminal"
                    );
                    assert_ne!(cell.symbol, ' ', "a blank cell was written");
                    written.insert((cx, cy), dots_of(cell.symbol));
                }

                let left = (x * DOTS_X as f64).floor() as usize;
                let top = (y * DOTS_Y as f64).floor() as usize;
                for dot_j in 0..dot_h {
                    for dot_i in 0..dot_w {
                        // Where this dot of the wordmark lands: the screen cell
                        // containing its screen dot, and its position *within*
                        // that cell. Both are offset by the origin, which is the
                        // whole point -- `dot_i % DOTS_X` would be the unskewed
                        // answer and would be wrong on every odd frame.
                        let screen_x = left + dot_i;
                        let screen_y = top + dot_j;
                        let cell_x = screen_x / DOTS_X;
                        let cell_y = screen_y / DOTS_Y;
                        let bit = 1u8
                            << ((screen_y % DOTS_Y) * DOTS_X + screen_x % DOTS_X);
                        let raised = written
                            .get(&(cell_x, cell_y))
                            .is_some_and(|bits| bits & bit != 0);
                        assert_eq!(
                            raised,
                            dvd.ink_at(dot_i, dot_j),
                            "at ({x}, {y}) the wordmark's dot ({dot_i},{dot_j}) is \
                             not drawn as the bitmap says"
                        );
                    }
                }
            }
        }
    }

    /// The diagonal has to be a diagonal, and 45 degrees visually.
    ///
    /// One cell right and one cell down is *not* 45 degrees on a cell grid: a
    /// terminal cell is roughly twice as tall as it is wide, so equal cell deltas
    /// draw a line at 50 to 63 degrees from horizontal.
    #[test]
    fn the_diagonal_is_corrected_for_the_cell_aspect_ratio() {
        let dvd = Dvd::new(DvdOptions::default(), (80, 24));
        let across_per_down =
            (dvd.vx.abs() / dvd.vy.abs().max(f64::MIN_POSITIVE)) as f32;
        assert!(
            (across_per_down - dvd.options.slope).abs() < 0.01,
            "the logo travels {across_per_down} cells across per cell down, but \
             slope is {}",
            dvd.options.slope
        );
        assert!(
            dvd.options.slope > 1.0,
            "a slope of {} draws equal cell deltas, which on this cell aspect \
             ratio is a line at 50 to 63 degrees rather than a diagonal",
            dvd.options.slope
        );
    }

    /// The speed cap matches the reference implementation's *feel*.
    ///
    /// This test used to pin a scaling rule -- `24 * 23 / 30`, the old cap
    /// against the old logo's width over the new one -- and that rule was wrong.
    /// It holds `cells_per_second * width` constant, which is not a quantity that
    /// means anything, and it does not preserve the time taken to cross one logo
    /// width: that would want 31, and the test asserted 18 while claiming to
    /// preserve it. The number it produced was roughly right for a while by
    /// accident, which is the worst way for a derivation to be right.
    ///
    /// What the cap is actually for is *feel*, and the reference has one to copy.
    /// lemonyte's screensaver moves 50 pixels a second across a 1920-pixel window
    /// with a logo a sixth of that width, so it crosses one logo width every 6.4
    /// seconds. Converting that rate to this logo's 30 cells gives 4.7 cells a
    /// second.
    ///
    /// Asserted as a band, because the derivation rounds and because a cap is
    /// something a user will want to move. It fails in the direction that
    /// matters most: back at 18 the logo is three and a half times too quick and
    /// shimmers, which is the whole of the report this commit answers.
    #[test]
    fn the_speed_cap_matches_the_reference_feel() {
        let options = DvdOptions::default();
        let logo_cells = options
            .logo
            .split('\n')
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0)
            .div_ceil(DOTS_X);

        // The reference, spelled out rather than quoted.
        const REFERENCE_PX_PER_SECOND: f64 = 50.0;
        const REFERENCE_WINDOW_PX: f64 = 1920.0;
        const REFERENCE_LOGO_FRACTION: f64 = 6.0;

        let widths_per_second = REFERENCE_PX_PER_SECOND
            / (REFERENCE_WINDOW_PX / REFERENCE_LOGO_FRACTION);
        let derived = widths_per_second * logo_cells as f64;

        assert!(
            (f64::from(options.speed) - derived).abs() < 1.0,
            "the cap is {} and the reference's {widths_per_second:.3} \
             logo-widths a second over this {logo_cells}-cell logo is {derived:.1}",
            options.speed
        );
        assert!(
            (4.0..=8.0).contains(&options.speed),
            "the cap is {}, which is outside 4 to 8: at 18 the logo is three times \
             too quick and shimmers, and above about 24 it is worse",
            options.speed
        );
    }

    /// Only cells with ink are written, and none of them carry a background.
    ///
    /// This is the "only write cells that have ink" half of the braille change,
    /// and it is the half that shows up on the wire and on screen.
    ///
    /// A blank braille cell is a space, so painting one costs a byte and erases
    /// whatever the terminal's background is showing there. The wordmark is 60% ink
    /// inside its bounding box, and the 40% that is not is the counters of the
    /// letters and the space around the reflection -- which is exactly the part
    /// that should show the terminal's own background through.
    ///
    /// Measured on the *opening* frame, which is the only one where the claim is
    /// exact: the canvas starts blank, so every cell the draw reports is one it
    /// chose to write, and the count is the number of cells in the grid carrying
    /// at least one raised dot. A later frame also reports the cells the logo has
    /// just *left* -- those are erasures, they have to be reported or the old logo
    /// stays on screen, and they are not the same thing.
    ///
    /// The background half is the one that fails against the old renderer. The
    /// quadrant blocks spend a cell's foreground *and* its background on a
    /// two-tone split, so every edge cell came out as
    /// `Cell::with_bg(glyph, ink, Color::Rgb { 0, 0, 0 }, ..)`: an
    /// `ESC[48;2;0;0;0` per edge cell on the wire, and a hard black rectangle
    /// painted over the un-inked half of every one of them. That is also why the
    /// old logo was invisible on anything but a black terminal, and why
    /// `[global] background` never showed through it.
    #[test]
    fn only_inked_cells_are_written_and_none_of_them_carry_a_background() {
        let mut dvd = Dvd::new(DvdOptions::default(), (80, 24));
        let opening = dvd.get_diff();

        let inked_cells = (0..dvd.grid.height())
            .flat_map(|cell_y| (0..dvd.grid.width()).map(move |cx| (cx, cell_y)))
            .filter(|(cx, cy)| dvd.grid.cell_char(*cx, *cy) != ' ')
            .count();
        let bounding_box = dvd.grid.width() * dvd.grid.height();

        assert_eq!(
            opening.len(),
            inked_cells,
            "the opening frame wrote {} cells but only {inked_cells} of the logo's \
             {bounding_box} have ink, so blank cells are being painted",
            opening.len()
        );
        assert!(
            inked_cells < bounding_box,
            "{inked_cells} of {bounding_box} cells have ink, so this logo is a slab \
             and the test is not measuring what it thinks it is"
        );

        for (_, _, cell) in &opening {
            assert_ne!(cell.symbol, ' ', "a blank cell was written");
            assert_eq!(
                cell.bg,
                style::Color::Reset,
                "wrote {:?} with a background of {:?}; braille is one colour per \
                 cell and the wordmark is a single flat one",
                cell.symbol,
                cell.bg
            );
        }

        // And over a run, nothing ever acquires a background either -- including
        // the erasures, which are the cells a naive implementation would blank with
        // a hard black rather than with the terminal's own background.
        for _ in 0..200 {
            dvd.update();
            for (_, _, cell) in dvd.get_diff() {
                assert_eq!(cell.bg, style::Color::Reset);
            }
        }
    }

    /// What one frame costs on the wire, and what it does *not* scale with.
    ///
    /// This is the measurement `frame_times` reports, run inline so a regression
    /// is caught by `cargo test` rather than only by looking at a table.
    ///
    /// The structural claim is the second one and the one that matters: the cost
    /// is a property of the logo, not of the terminal. A 400x200 screen is
    /// eighty times the area of an 80x24 one and the logo still moves at the same
    /// rate, so the two must cost the same. A draw that walked the whole canvas,
    /// or a renderer that painted every cell in the logo's bounding box, would
    /// show up here as a factor of the terminal size.
    ///
    /// The absolute number is around 650 bytes a frame, and the reason is worth
    /// knowing: a braille cell *is* its bit pattern, so moving the logo one dot
    /// sideways changes every glyph in it and the diff is the whole logo rather
    /// than its leading edge. That is inherent to placing a shape at 8x, and it
    /// buys the dot of resolution that the old 2x2 renderer did not have.
    #[test]
    fn a_frame_costs_the_same_whatever_size_the_terminal_is() {
        fn mean_frame_bytes(size: (u16, u16)) -> (usize, usize) {
            let mut dvd = Dvd::new(DvdOptions::default(), size);
            let opening = {
                let mut out: Vec<u8> = Vec::new();
                crate::common::write_cells(&mut out, size, &dvd.get_diff())
                    .expect("writing to a Vec cannot fail");
                out.len()
            };

            let mut total = 0usize;
            for _ in 0..120 {
                dvd.update();
                let mut out: Vec<u8> = Vec::new();
                crate::common::write_cells(&mut out, size, &dvd.get_diff())
                    .expect("writing to a Vec cannot fail");
                total += out.len();
            }
            (opening, total / 120)
        }

        let (small_open, small) = mean_frame_bytes((80, 24));
        let (big_open, big) = mean_frame_bytes((400, 200));

        // A ceiling as well, so that a frame which happened to be small at 80x24
        // for the wrong reason would not pass the ratio below.
        assert!(
            small < 1_500 && big < 1_500,
            "a frame costs {small} bytes at 80x24 and {big} at 400x200, which is \
             more than a 210-cell logo moving a dot at a time should"
        );
        assert!(
            big * 10 <= small * 12,
            "a frame costs {small} bytes at 80x24 and {big} at 400x200, so the cost \
             scales with the terminal rather than with the logo"
        );
        assert!(
            small_open < 1_500 && big_open < 1_500,
            "the opening frame costs {small_open} bytes at 80x24 and {big_open} at \
             400x200"
        );
    }

    #[test]
    fn diff_stays_in_bounds() {
        let mut effect = Dvd::new(options(), (10, 4));
        for _ in 0..30 {
            effect.update();
            for (x, y, _) in effect.get_diff() {
                assert!(x < 10 && y < 4);
            }
        }
    }
}
