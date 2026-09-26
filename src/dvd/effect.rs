use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::render::QuadrantMask;
use crate::render::quadrant::SAMPLES_PER_CELL;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

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
/// The default logo: a seven-row block-letter `DVD`, twenty-three cells wide.
///
/// The previous default was five rows of an *outline* font, which was wrong twice
/// over. The middle glyph was two verticals joined at the bottom, so it was a U
/// and the logo read `DUD`. And an outline font puts every stroke on a cell
/// boundary, so every stroke was rendered with quadrant block glyphs -- the
/// elements least likely to be in a font -- when plain full blocks would have done.
///
/// So: a real V, and strokes three cells thick where the letterform allows. The
/// `D`s are then entirely `█` and blanks, and only the `V`'s taper touches a
/// quadrant glyph. That is the whole reason a solid-and-hollow letterform is
/// better here than a thin one, and it is also why the interior stays legible on a
/// terminal missing the quadrants.
///
/// Modelled on the wordmark in lemonyte's `dvd-screensaver`, which is the
/// reference for this effect: a heavy, slightly forward-slanted `DVD` on black.
/// Seven rows is about a quarter of a 24-row terminal, against the reference's
/// logo at a sixth of its window -- a little larger, because a terminal cell is
/// much coarser than a pixel and a logo needs to be big to survive being made of
/// them.
///
/// Multi-row logos were always supported: `logo` is parsed on `\n` and the draw
/// nests rows. So the size is a default-value change; the smoothness is not.
const DEFAULT_LOGO: &str = "\
███████  ███████  ███████
███  ██  ███  ██  ██  ██
███  ██  ███  ██  ██  ██
███  ██  ███  ██  ██  ██
███  ██   █████    ██  ██
███  ██    ███     ██  ██
███████     █      ███████";

/// Whether a logo character is transparent.
///
/// A space is. A full block is not: with a two-tone mask the ink is the
/// foreground and the background shows through everywhere else, so an ink glyph
/// lights every sample it covers.
fn is_blank(symbol: char) -> bool {
    symbol == ' '
}

/// The colours the logo cycles through.
///
/// Every entry sits at relative luminance 0.17, which is what clears 3:1 against
/// both a white and a black background -- and a terminal profile is whichever of
/// those the user happens to run, so that band is a requirement rather than a
/// preference. `every_colour_reads_on_a_light_and_on_a_dark_profile` enforces it.
///
/// Twelve rather than six, because the logo now changes colour on *every* wall
/// bounce rather than only at corners, so consecutive colours are seen seconds
/// apart and need to be obviously different. Six was enough when a change was a
/// once-a-minute event. Hues are spread roughly every thirty degrees at constant
/// luminance, which means the perceived *brightness* never jumps even though the
/// hue always does -- varying luminance as well would read as a flash.
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
    /// Logo to bounce around the screen. Use `\n` for multi-line logos.
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
    /// periods are `2 * max_x / vx` and `2 * max_y / vy`. At 80x24 those are 5.7
    /// and 3.4 seconds, and they only coincide when `57m = 17n` -- first at
    /// m = 17, n = 57, which is 97 seconds. So the logo sat at one colour for a
    /// minute and a half at a time, which is the same as never changing.
    ///
    /// lemonyte's `dvd-screensaver`, which is the reference for this effect,
    /// recolours on *every* wall hit. That is the right answer: the colour change
    /// becomes the thing the eye tracks between bounces, and it happens every two
    /// or three seconds rather than never.
    pub color_change: ColorChange,
    /// Start the logo from a corner instead of the middle.
    pub start_in_corner: bool,
    /// Seed for the starting corner and colour.
    pub seed: u64,
}

/// When the bouncing logo changes colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorChange {
    /// On every wall hit. What the reference implementation does, and what makes
    /// the logo feel alive rather than inert.
    #[default]
    Bounce,
    /// Only when both axes reverse on the same frame, which is the easter egg the
    /// reference's README jokes about -- "it could hit the corner if you look at
    /// it long enough". True on an 80x24 screen roughly every 97 seconds.
    Corner,
    /// Never. One colour for the whole run.
    Never,
}

impl ColorChange {
    /// Every variant, so a test can check all of them.
    pub const ALL: &'static [ColorChange] =
        &[ColorChange::Bounce, ColorChange::Corner, ColorChange::Never];

    /// Whether a bounce of these two axes should change the colour.
    pub fn should_change(self, hit_x: bool, hit_y: bool) -> bool {
        match self {
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
            logo: String::from(DEFAULT_LOGO),
            // 20 rather than 9. At 9 the logo covered 0.15 cells per frame, so
            // the drawn position changed once every seven frames: still for six
            // frames out of seven, then a jump.
            //
            // The ceiling is a property of the medium, not a taste, and it is
            // worth being precise about because the reference and this cannot
            // match.
            //
            // lemonyte's screensaver moves 50 pixels a second across a 1920-pixel
            // window, and its logo is a sixth of that window. So it crosses one
            // logo width every 6.4 seconds. This logo is 23 cells wide, so the
            // same feel would be 3.6 cells a second.
            //
            // At 3.6 cells a second the logo's position advances 7.2 samples a
            // second horizontally against 60 frames, so more than eight frames in
            // ten land between sample boundaries and do not change. That is the
            // staircase, exactly as reported. The reference has 1920 positions to
            // choose from; this has 160. **A terminal cannot render that feel
            // smoothly, and the only honest trade is to be too quick rather than
            // to stutter.**
            //
            // At 24 the screen changes on about 88 frames in 100, the logo crosses
            // in 2.4 seconds, and a wall hit -- which recolours it -- comes every
            // two to three seconds. Set `speed` in the config for calmer or
            // brisker; below about 12 the stepping returns.
            speed: 24.0,
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
    /// The logo's occupancy at 2x2 per cell. See [`QuadrantMask`].
    mask: QuadrantMask,
    rows: Vec<Vec<char>>,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color_index: usize,
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
        self.mask
            .resize(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.rows = Self::parse_logo(&self.options.logo);

        // Reseeded rather than merely carried over: a resize is a new screen, and
        // re-rolling the corner and colour is what makes a resize feel like a
        // fresh run instead of a jump cut.
        self.rng = seeded_rng(self.options.seed, "dvd");

        let logo_width = self.logo_width();
        let logo_height = self.rows.len().max(1) as f64;
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
    }
}

impl Dvd {
    pub fn new(options: DvdOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let seed = options.seed;
        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            mask: QuadrantMask::new(screen_size.0 as usize, screen_size.1 as usize),
            rows: Vec::new(),
            x: 0.0,
            y: 0.0,
            vx: 1.0,
            vy: 1.0,
            color_index: 0,
            // Replaced by `reset` below; this only has to be a valid generator.
            rng: seeded_rng(seed, "dvd"),
            options,
        };
        effect.reset();
        effect
    }

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

    fn logo_width(&self) -> f64 {
        self.rows
            .iter()
            .map(|row| row.len() as f64)
            .fold(0.0, f64::max)
    }

    fn step(&mut self, delta: f64) {
        let logo_width = self.logo_width();
        let logo_height = self.rows.len().max(1) as f64;
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

        if self.options.color_change.should_change(hit_x, hit_y) {
            self.color_index = (self.color_index + 1) % PALETTE.len();
        }
    }

    /// Draws the logo at its current position into the canvas.
    /// Draws the logo at its current position into the canvas.
    ///
    /// Drawn into a [`QuadrantMask`] rather than straight onto the canvas.
    ///
    /// The old version did `self.x as usize` and `self.y as usize` at the top of
    /// its loops, which threw away every sub-cell part of the position `step` had
    /// carefully integrated. At the old default of nine cells per second that is
    /// 0.15 cells per frame, so the drawn position changed once every seven frames:
    /// the logo sat perfectly still for six frames out of seven and then jumped a
    /// whole cell diagonally. That is the "laggy, like stairs" report.
    ///
    /// Half-block was tried first and is not enough. It gives 2x vertically, which
    /// halves the steps on that axis, but the horizontal stays at one sample per
    /// cell -- and a cell has one foreground and one background, so a vertical
    /// split spends both and the two cannot be combined. The quadrant block
    /// elements are the glyphs that split a cell both ways at once, and this is the
    /// only renderer in the crate that places a shape at 2x on both axes.
    fn draw(&mut self) -> Vec<(usize, usize, Cell)> {
        let (r, g, b) = PALETTE[self.color_index % PALETTE.len()];
        let ink = style::Color::Rgb { r, g, b };
        let background = style::Color::Rgb { r: 0, g: 0, b: 0 };

        self.mask.clear();
        // The mask renderer only writes cells that have ink, so the canvas has to
        // be cleared here or the previous frame's logo stays on screen and the two
        // smear into each other.
        self.canvas.clear();

        // Both axes in field samples, so the sub-cell part of the position is
        // drawn rather than discarded.
        let left = (self.x * SAMPLES_PER_CELL as f64).floor().max(0.0) as usize;
        let top = (self.y * SAMPLES_PER_CELL as f64).floor().max(0.0) as usize;
        for (row_index, row) in self.rows.iter().enumerate() {
            for (offset, symbol) in row.iter().enumerate() {
                if is_blank(*symbol) {
                    continue;
                }
                for dy in 0..SAMPLES_PER_CELL {
                    for dx in 0..SAMPLES_PER_CELL {
                        self.mask.set(
                            left + offset * SAMPLES_PER_CELL + dx,
                            top + row_index * SAMPLES_PER_CELL + dy,
                            true,
                        );
                    }
                }
            }
        }

        self.mask.write_to(
            &mut self.canvas,
            ink,
            background,
            style::Attribute::Reset,
        );
        self.canvas.commit()
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

    #[test]
    fn stays_within_bounds_while_bouncing() {
        let mut effect = Dvd::new(options(), (20, 8));
        for _ in 0..600 {
            effect.step(1.0 / 60.0);
            assert!(effect.x >= 0.0 && effect.x <= (20 - 3) as f64);
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

    /// Every wall hit has to change the colour.
    ///
    /// This is the reference implementation's behaviour and it is the difference
    /// between a logo that feels alive and one that is inert. The previous option
    /// was `corner_color_change`, and corners are almost unreachable: a corner
    /// needs both axes to reverse on the same frame, and the bounce periods are
    /// `2 * max_x / vx` and `2 * max_y / vy`. On an 80x24 screen those are 5.7
    /// and 3.4 seconds and only coincide when `57m = 17n` -- first at m = 17,
    /// n = 57, which is 97 seconds. So the logo sat at one colour for a minute and
    /// a half, which is the same as never changing.
    ///
    /// This test is measured over a long enough run that the old corner-only
    /// behaviour would have had many opportunities to fire and still fails,
    /// because on these dimensions it needs about 97 seconds and this runs 10.
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
        for _ in 0..600 {
            effect.step(1.0 / 60.0);
            if effect.color_index != initial {
                changes += 1;
            }
        }

        assert!(
            changes >= 3,
            "the colour changed {changes} times in ten seconds of bouncing, so the \
             logo is effectively one colour. A wall hit every two or three seconds \
             is what makes this read as motion rather than as a still image \
             drifting."
        );
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
            let mut previous = (effect.x, effect.y);

            for _ in 0..20_000 {
                let before = (effect.x, effect.y);
                effect.step(1.0 / 60.0);
                let reversed = (effect.x - before.0).abs() < 1e-9
                    || (effect.y - before.1).abs() < 1e-9;
                if reversed {
                    bounces += 1;
                }
                if effect.color_index != initial {
                    changes += 1;
                    effect.color_index = initial;
                }
                previous = (effect.x, effect.y);
            }
            let _ = previous;

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

    /// The rule itself, for all three modes, without a simulation in the way.
    #[test]
    fn the_colour_rule_is_what_each_mode_says() {
        assert!(ColorChange::Bounce.should_change(true, false));
        assert!(ColorChange::Bounce.should_change(false, true));
        assert!(ColorChange::Bounce.should_change(true, true));
        assert!(!ColorChange::Bounce.should_change(false, false));

        assert!(!ColorChange::Corner.should_change(true, false));
        assert!(!ColorChange::Corner.should_change(false, true));
        assert!(ColorChange::Corner.should_change(true, true));
        assert!(!ColorChange::Corner.should_change(false, false));

        for (hit_x, hit_y) in [(true, false), (false, true), (true, true)] {
            assert!(!ColorChange::Never.should_change(hit_x, hit_y));
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
    /// The bound is 500 rather than 600 because 2x resolution caps it. At 26 cells
    /// per second the horizontal position advances 52 samples a second and the
    /// vertical 26, against 60 frames, so a handful of frames land between sample
    /// boundaries and genuinely cannot show movement. Reaching every frame needs
    /// braille's 4x, which would turn a solid block letter into a field of dots.
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

        assert!(
            changed > 500,
            "the screen changed on only {changed} of 600 frames, so the logo is \
             still mostly stationary with the occasional jump"
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
    /// Half-block was the first attempt and is not enough: it gives 2x vertically
    /// but the horizontal stays at one sample per cell. This pins the axis that a
    /// half-block renderer would have fixed, and the next test pins the one it
    /// would not.
    #[test]
    fn the_vertical_position_resolves_finer_than_a_cell() {
        let mut dvd = Dvd::new(DvdOptions::default(), (80, 24));
        let mut samples = std::collections::BTreeSet::new();
        let mut cells = std::collections::BTreeSet::new();
        for _ in 0..60 {
            dvd.update();
            samples.insert((dvd.y * SAMPLES_PER_CELL as f64).floor() as i64);
            cells.insert(dvd.y.floor() as i64);
        }
        // The ratio, not an absolute count: the absolute number moves with the
        // speed and the terminal size, and a threshold on it is a threshold on the
        // wrong thing. Two samples per cell means the drawn positions must
        // outnumber the cell positions by close to two to one.
        let ratio = samples.len() as f32 / cells.len().max(1) as f32;
        assert!(
            ratio > 1.8,
            "{} distinct drawn vertical positions against {} at whole-cell \
             resolution, a ratio of {ratio:.2}, so the sub-cell vertical motion \
             is not being drawn",
            samples.len(),
            cells.len()
        );
    }

    /// The logo has to be big.
    ///
    /// It was three characters on one line, which is a text label rather than a
    /// logo. It was previously pinned by name in an integration test too, which
    /// had to be updated with it.
    #[test]
    fn the_default_logo_is_a_multi_row_block_letter() {
        let dvd = Dvd::new(DvdOptions::default(), (80, 24));
        assert!(
            dvd.rows.len() >= 5,
            "the default logo is {} row(s) tall",
            dvd.rows.len()
        );
        let width = dvd.rows.iter().map(Vec::len).max().unwrap_or(0);
        assert!(
            width >= 15,
            "the default logo is {width} cells wide, which is a label rather \
             than a logo"
        );
        // Ink and gaps, or it is a slab rather than letters. An earlier version
        // asserted the logo was *majority* full blocks, on the theory that a
        // sub-cell renderer needs a solid slab to stay solid. That is not how it
        // works: every ink sample lights every sample it covers, so an ink cell
        // comes out as a full block whatever its neighbours are, and a
        // one-cell-wide stroke renders at full size rather than halved.
        let cells: Vec<char> = dvd.rows.iter().flatten().copied().collect();
        assert!(
            cells.contains(&'█') && cells.contains(&' '),
            "the default logo has no gaps or no ink, so it is not legible as \
             letters"
        );
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
