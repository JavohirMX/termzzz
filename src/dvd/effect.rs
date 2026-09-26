use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
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
/// The whole palette had to move, not just that entry: any of the six could be
/// the starting colour, so leaving five of them above luminance 0.30 would have
/// left the bug reachable five times out of six.
const PALETTE: [(u8, u8, u8); 6] = [
    // rose,   luminance 0.19
    (220, 60, 90),
    // amber,  luminance 0.24
    (200, 110, 30),
    // green,   luminance 0.23
    (50, 150, 60),
    // teal,    luminance 0.25
    (30, 150, 160),
    // blue,    luminance 0.15
    (60, 100, 210),
    // violet,  luminance 0.15
    (160, 60, 200),
];

const DT: f64 = 1.0 / 60.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DvdOptions {
    /// Logo to bounce around the screen. Use `\n` for multi-line logos.
    pub logo: String,
    /// Base bounce speed in cells per second.
    pub speed: f32,
    /// Change the logo color when the logo lands in a screen corner.
    pub corner_color_change: bool,
    /// Start the logo from a corner instead of the middle.
    pub start_in_corner: bool,
    /// Seed for the starting corner and colour.
    pub seed: u64,
}

impl Default for DvdOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            logo: String::from("DVD"),
            speed: 9.0,
            corner_color_change: true,
            start_in_corner: true,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Dvd {
    screen_size: (u16, u16),
    options: DvdOptions,
    canvas: Canvas,
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
        self.draw();
        self.canvas.commit()
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
            self.vx = if self.x <= 0.0 { speed } else { -speed };
            self.vy = if self.y <= 0.0 { speed } else { -speed };
        } else {
            self.x = max_x / 2.0;
            self.y = max_y / 2.0;
            self.vx = speed;
            self.vy = speed;
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

        if hit_x && hit_y && self.options.corner_color_change {
            self.color_index = (self.color_index + 1) % PALETTE.len();
        }
    }

    /// Draws the logo at its current position into the canvas.
    fn draw(&mut self) {
        self.canvas.clear();
        let (r, g, b) = PALETTE[self.color_index % PALETTE.len()];

        for (row_index, row) in self.rows.iter().enumerate() {
            let y = self.y as usize + row_index;
            if y >= self.screen_size.1 as usize {
                break;
            }
            for (column_index, symbol) in row.iter().enumerate() {
                let x = self.x as usize + column_index;
                if x >= self.screen_size.0 as usize {
                    break;
                }
                self.canvas.set(
                    x,
                    y,
                    Cell::new(
                        *symbol,
                        style::Color::Rgb { r, g, b },
                        style::Attribute::Bold,
                    ),
                );
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

    #[test]
    fn corner_hits_change_color() {
        let mut effect = Dvd::new(options(), (6, 2));
        effect.options.corner_color_change = true;
        effect.reset();
        let initial = effect.color_index;

        for _ in 0..2000 {
            effect.step(1.0 / 60.0);
            if effect.color_index != initial {
                return;
            }
        }

        panic!("color should change after repeated corner hits");
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
