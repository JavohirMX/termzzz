use crate::buffer::{Buffer, Cell};
use crate::common::TerminalEffect;
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlasmaOptions {
    pub time_scale: f64,
    pub spatial_scale: f64,
    pub color_speed: f64,
}

impl Default for PlasmaOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            time_scale: 0.5,
            spatial_scale: 1.0,
            color_speed: 20.0,
        }
    }
}

pub struct Plasma {
    pub screen_size: (u16, u16),
    options: PlasmaOptions,
    buffer: Buffer,
    time: f64,
    palette: Vec<style::Color>,
}

impl TerminalEffect for Plasma {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // Clone the previous buffer to work with
        let mut curr_buffer = self.buffer.clone();

        // Update the plasma field directly (no LUT)
        self.update_plasma(&mut curr_buffer);

        let diff = self.buffer.diff(&curr_buffer);
        self.buffer = curr_buffer;

        diff
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.reset();
    }

    fn reset(&mut self) {
        self.buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.time = 0.0;
    }
}

impl Plasma {
    /// Advances the field clock by the elapsed time. The old code added a fixed
    /// 0.1 per call, which tied the animation to the refresh rate.
    fn advance(&mut self, delta: f32) {
        self.time += self.options.time_scale * delta as f64;
    }

    pub fn new(options: PlasmaOptions, screen_size: (u16, u16)) -> Self {
        let buffer = Buffer::new(screen_size.0 as usize, screen_size.1 as usize);
        let time = 0.0;

        // Generate color palette
        let palette = Self::generate_palette();

        Self {
            screen_size,
            options,
            buffer,
            time,
            palette,
        }
    }

    /// Generate a color palette of 256 colors
    fn generate_palette() -> Vec<style::Color> {
        let mut palette = Vec::with_capacity(256);

        for i in 0..256 {
            let i_f64 = i as f64;
            let r = 128.0 + 128.0 * (PI * i_f64 / 32.0).sin();
            let g = 128.0 + 128.0 * (PI * i_f64 / 64.0).sin();
            let b = 128.0 + 128.0 * (PI * i_f64 / 128.0).sin();

            palette.push(style::Color::Rgb {
                r: Self::clamp(r as u8, 0, 255),
                g: Self::clamp(g as u8, 0, 255),
                b: Self::clamp(b as u8, 0, 255),
            });
        }

        palette
    }

    /// Clamp a value between min and max
    fn clamp(val: u8, min: u8, max: u8) -> u8 {
        if val < min {
            min
        } else if val > max {
            max
        } else {
            val
        }
    }

    /// Calculate plasma value using the [AWK script formula](https://rosettacode.org/wiki/Plasma_effect#AWK)
    fn calc_plasma_value(&self, x: f64, y: f64, now: f64, w: f64, h: f64) -> u8 {
        let scale = self.options.spatial_scale;

        let value = (128.0
            + (128.0 * ((x / 8.0) * scale - (now / 2.0).cos()).sin())
            + 128.0
            + (128.0 * ((y / 16.0) * scale - now.sin() * 2.0).sin())
            + 128.0
            + (128.0
                * (((x - w / 2.0).powi(2) + (y - h / 2.0).powi(2)).sqrt() / 4.0
                    * scale)
                    .sin())
            + 128.0
            + (128.0
                * (((x.powi(2) + y.powi(2)).sqrt() / 4.0) * scale
                    - (now / 4.0).sin())
                .sin()))
            / 4.0;

        value as u8
    }

    /// Update the plasma field in the buffer
    fn update_plasma(&mut self, buffer: &mut Buffer) {
        let width = self.screen_size.0 as usize;
        let height = self.screen_size.1 as usize;
        let w = width as f64;
        let h = height as f64;
        let now = self.time;

        for y in 0..height {
            for x in 0..width {
                // For each cell, calculate two plasma values (upper and lower half)
                let y_f64 = (y * 2) as f64;
                let x_f64 = x as f64;

                // Calculate plasma values
                let plasma = self.calc_plasma_value(x_f64, y_f64, now, w, h * 2.0);

                // Get color indices with time component
                let color_idx = ((plasma as f64) + now * self.options.color_speed)
                    as usize
                    % 256;

                let cell_color = self.palette[color_idx];

                let cell = Cell::new('*', cell_color, style::Attribute::Bold);

                buffer.set(x, y, cell);
            }
        }
    }
}
