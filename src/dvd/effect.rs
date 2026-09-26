use crate::buffer::{Buffer, Cell};
use crate::common::{DefaultOptions, TerminalEffect};
use crossterm::style;
use derive_builder::Builder;
use rand::RngExt;
use serde::{Deserialize, Serialize};

const PALETTE: [(u8, u8, u8); 6] = [
    (235, 235, 245),
    (255, 95, 120),
    (95, 205, 255),
    (140, 245, 160),
    (255, 205, 95),
    (200, 145, 255),
];

const DT: f64 = 1.0 / 60.0;

#[derive(Builder, Default, Debug, Clone, Serialize, Deserialize)]
#[builder(public, setter(into))]
pub struct DvdOptions {
    /// Logo to bounce around the screen. Use `\n` for multi-line logos.
    #[builder(default = "String::from(\"DVD\")")]
    pub logo: String,
    /// Base bounce speed in cells per second.
    #[builder(default = "9.0")]
    pub speed: f32,
    /// Change the logo color when the logo lands in a screen corner.
    #[builder(default = "true")]
    pub corner_color_change: bool,
    /// Start the logo from a corner instead of the middle.
    #[builder(default = "true")]
    pub start_in_corner: bool,
}

pub struct Dvd {
    screen_size: (u16, u16),
    options: DvdOptions,
    buffer: Buffer,
    rows: Vec<Vec<char>>,
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color_index: usize,
}

impl TerminalEffect for Dvd {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let curr_buffer = self.draw();
        let diff = self.buffer.diff(&curr_buffer);
        self.buffer = curr_buffer;
        diff
    }

    fn update(&mut self) {
        self.step(DT);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.step(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.reset();
    }

    fn reset(&mut self) {
        self.buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.rows = Self::parse_logo(&self.options.logo);

        let logo_width = self.logo_width();
        let logo_height = self.rows.len().max(1) as f64;
        let max_x = (self.screen_size.0 as f64 - logo_width).max(0.0);
        let max_y = (self.screen_size.1 as f64 - logo_height).max(0.0);

        let speed = self.options.speed.max(0.1) as f64;

        if self.options.start_in_corner {
            let corner = rand::rng().random_range(0..4);
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
        self.color_index = rand::rng().random_range(0..PALETTE.len());
    }
}

impl Dvd {
    pub fn new(options: DvdOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let mut effect = Self {
            screen_size,
            buffer: Buffer::new(screen_size.0 as usize, screen_size.1 as usize),
            rows: Vec::new(),
            x: 0.0,
            y: 0.0,
            vx: 1.0,
            vy: 1.0,
            color_index: 0,
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

    fn draw(&self) -> Buffer {
        let mut next =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);
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
                next.set(
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

        next
    }
}

impl DefaultOptions for Dvd {
    type Options = DvdOptions;

    fn default_options(_width: u16, _height: u16) -> Self::Options {
        DvdOptionsBuilder::default()
            .logo(String::from("DVD"))
            .speed(9.0_f32)
            .corner_color_change(true)
            .start_in_corner(true)
            .build()
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn options() -> DvdOptions {
        DvdOptionsBuilder::default()
            .logo(String::from("DVD"))
            .start_in_corner(false)
            .build()
            .unwrap()
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
