use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
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
    canvas: Canvas,
    time: f64,
    palette: Vec<style::Color>,
}

impl TerminalEffect for Plasma {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // Update the plasma field directly (no LUT). The field is fully redrawn
        // every frame, so there is nothing to carry over from the last one.
        self.canvas.clear();
        Self::update_plasma(
            self.screen_size,
            self.time,
            self.options.color_speed,
            self.options.spatial_scale,
            &self.palette,
            self.canvas.surface_mut(),
        );
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas
            .resize(self.screen_size.0.max(1), self.screen_size.1.max(1));
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
        let canvas = Canvas::new(screen_size.0, screen_size.1);
        let time = 0.0;

        // Generate color palette
        let palette = Self::generate_palette();

        Self {
            screen_size,
            options,
            canvas,
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
    fn calc_plasma_value(
        x: f64,
        y: f64,
        now: f64,
        w: f64,
        h: f64,
        scale: f64,
    ) -> u8 {
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
    #[allow(clippy::too_many_arguments)]
    fn update_plasma(
        size: (u16, u16),
        now: f64,
        color_speed: f64,
        spatial_scale: f64,
        palette: &[style::Color],
        buffer: &mut Buffer,
    ) {
        let width = size.0 as usize;
        let height = size.1 as usize;
        let w = width as f64;
        let h = height as f64;

        for y in 0..height {
            for x in 0..width {
                // For each cell, calculate two plasma values (upper and lower half)
                let y_f64 = (y * 2) as f64;
                let x_f64 = x as f64;

                // Calculate plasma values
                let plasma = Self::calc_plasma_value(
                    x_f64,
                    y_f64,
                    now,
                    w,
                    h * 2.0,
                    spatial_scale,
                );

                // Get color indices with time component
                let color_idx =
                    ((plasma as f64) + now * color_speed) as usize % 256;

                let cell_color = palette[color_idx];

                let cell = Cell::new('*', cell_color, style::Attribute::Bold);

                buffer.set(x, y, cell);
            }
        }
    }
}
