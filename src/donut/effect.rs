use crate::buffer::{Buffer, Cell};
use crate::common::{DefaultOptions, TerminalEffect};
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DonutOptions {
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub rotation_speed_a: f32,
    pub rotation_speed_b: f32,
    pub distance: f32,
    #[serde(skip)]
    pub k1: f32,
    pub k1_coeff: f32,
    pub luminance_chars: Vec<char>,
}

impl Default for DonutOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            inner_radius: 1.0,
            outer_radius: 2.0,
            rotation_speed_a: 0.022,
            rotation_speed_b: 0.010,
            distance: 5.5,
            k1: 25.0,
            k1_coeff: 1.0,
            luminance_chars: vec![
                '.', ',', '-', '~', ':', ';', '=', '!', '*', '#', '$', '@',
            ],
        }
    }
}

/// Number of samples around the cross-section of the torus at full resolution.
const THETA_SAMPLES: usize = 314;
/// Number of samples around the centre of revolution, always twice `theta`.
const PHI_SAMPLES: usize = THETA_SAMPLES * 2;

/// The sampling angles are fixed, so their sines and cosines are constants.
/// Evaluating them inside the inner loop was the single largest cost here.
static SIN_THETA: LazyLock<Vec<f32>> = LazyLock::new(|| {
    (0..THETA_SAMPLES)
        .map(|i| (i as f32 * 0.02).sin())
        .collect()
});

static COS_THETA: LazyLock<Vec<f32>> = LazyLock::new(|| {
    (0..THETA_SAMPLES)
        .map(|i| (i as f32 * 0.02).cos())
        .collect()
});

static SIN_PHI: LazyLock<Vec<f32>> =
    LazyLock::new(|| (0..PHI_SAMPLES).map(|i| (i as f32 * 0.01).sin()).collect());

static COS_PHI: LazyLock<Vec<f32>> =
    LazyLock::new(|| (0..PHI_SAMPLES).map(|i| (i as f32 * 0.01).cos()).collect());

/// Fallback ramp, used when a config file supplies an empty one.
const DEFAULT_LUMINANCE_CHARS: &[char] =
    &['.', ',', '-', '~', ':', ';', '=', '!', '*', '#', '$', '@'];

/// The glyph ramp, never empty.
///
/// An empty `luminance_chars` used to underflow `len() - 1` and panic, and that is
/// reachable straight from a user config file.
fn shade_ramp(configured: &[char]) -> &[char] {
    if configured.is_empty() {
        DEFAULT_LUMINANCE_CHARS
    } else {
        configured
    }
}

/// Gruvbox gradient, indexed by shade.
const COLORS: [style::Color; 12] = [
    style::Color::Rgb {
        r: 213,
        g: 196,
        b: 161,
    },
    style::Color::Rgb {
        r: 213,
        g: 196,
        b: 161,
    },
    style::Color::Rgb {
        r: 213,
        g: 196,
        b: 161,
    },
    style::Color::Rgb {
        r: 213,
        g: 196,
        b: 161,
    },
    style::Color::Rgb {
        r: 251,
        g: 241,
        b: 199,
    },
    style::Color::Rgb {
        r: 251,
        g: 241,
        b: 199,
    },
    style::Color::Rgb {
        r: 69,
        g: 133,
        b: 136,
    },
    style::Color::Rgb {
        r: 104,
        g: 157,
        b: 106,
    },
    style::Color::Rgb {
        r: 152,
        g: 151,
        b: 26,
    },
    style::Color::Rgb {
        r: 215,
        g: 153,
        b: 33,
    },
    style::Color::Rgb {
        r: 214,
        g: 93,
        b: 14,
    },
    style::Color::Rgb {
        r: 204,
        g: 36,
        b: 29,
    },
];

/// Per-frame scratch, reused across frames so a frame does not allocate.
#[derive(Default)]
struct Scratch {
    zbuffer: Vec<f32>,
    output: Vec<char>,
    shade: Vec<u8>,
}

pub struct Donut {
    pub screen_size: (u16, u16),
    options: DonutOptions,
    buffer: Buffer,
    rotation_a: f32,
    rotation_b: f32,
    colors: &'static [style::Color; 12],
    scratch: Scratch,
}

impl TerminalEffect for Donut {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let mut curr_buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);

        self.render_donut(&mut curr_buffer);

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
        self.screen_size = (width.max(1), height.max(1));
        let min_dimension = self.screen_size.0.min(self.screen_size.1) as f32;
        self.options.k1 = min_dimension * 0.8 * self.options.k1_coeff;
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Donut {
    /// Rotates by however far the elapsed time says, rather than by a fixed
    /// step per call. Without this the donut spins at whatever rate the
    /// terminal happens to refresh at, and the global speed keys do nothing.
    fn advance(&mut self, delta: f32) {
        self.rotation_a += self.options.rotation_speed_a * delta;
        self.rotation_b += self.options.rotation_speed_b * delta;
    }

    pub fn new(options: DonutOptions, screen_size: (u16, u16)) -> Self {
        let buffer = Buffer::new(screen_size.0 as usize, screen_size.1 as usize);
        Self {
            screen_size,
            options,
            buffer,
            rotation_a: 0.0,
            rotation_b: 0.0,
            colors: &COLORS,
            scratch: Scratch::default(),
        }
    }

    fn render_donut(&mut self, buffer: &mut Buffer) {
        buffer.fill_with(&Cell::default());

        let width = self.screen_size.0 as usize;
        let height = self.screen_size.1 as usize;

        let sin_a = self.rotation_a.sin();
        let cos_a = self.rotation_a.cos();
        let sin_b = self.rotation_b.sin();
        let cos_b = self.rotation_b.cos();

        // Resized up front so the scratch borrows below do not overlap any
        // access to `self`.
        let cells = width * height;
        if self.scratch.zbuffer.len() != cells {
            self.scratch.zbuffer = vec![0.0; cells];
            self.scratch.output = vec![' '; cells];
            self.scratch.shade = vec![0u8; cells];
        } else {
            self.scratch.zbuffer.fill(0.0);
            self.scratch.output.fill(' ');
            self.scratch.shade.fill(0);
        }

        // Everything below is loop invariant, so it is hoisted out. The previous
        // version re-read the option struct, recomputed the screen midpoint and
        // recomputed the pre-revolution circle on every one of the ~197k inner
        // iterations, and derived the sine and cosine of the loop counters from
        // scratch each time even though the ranges are fixed.
        let outer_radius = self.options.outer_radius;
        let inner_radius = self.options.inner_radius;
        let distance = self.options.distance;
        let k1 = self.options.k1;
        let colors = self.colors;
        let half_width = width as f32 / 2.0;
        let half_height = height as f32 / 2.0;
        let ramp = shade_ramp(&self.options.luminance_chars);
        let ramp_top = ramp.len().saturating_sub(1);

        let scratch = &mut self.scratch;
        let zbuffer = &mut scratch.zbuffer;
        let output = &mut scratch.output;
        let shade = &mut scratch.shade;

        // Resolution follows the terminal, so a small window does not pay for a
        // large one. `min_dimension * 314 / 50` reproduces the previous fixed 314
        // samples at the reference size of 50 rows.
        let min_dimension = width.min(height);
        let theta_steps =
            ((min_dimension * THETA_SAMPLES) / 50).clamp(48, THETA_SAMPLES);
        let phi_steps = (theta_steps * 2).min(PHI_SAMPLES);

        for theta_index in 0..theta_steps {
            let sin_theta = SIN_THETA[theta_index];
            let cos_theta = COS_THETA[theta_index];

            // Invariant with respect to phi, but the old code recomputed it
            // once per phi sample rather than once per theta sample.
            let circle_x = outer_radius + inner_radius * cos_theta;
            let circle_y = inner_radius * sin_theta;

            // Terms that genuinely do not depend on phi. The rotation algebra
            // itself is left in its original factored form: an earlier pass
            // tried to hoist more of it, dropped a `sin_b` and a `cos_b`, and
            // silently collapsed the whole torus into a single column.
            let cos_a_circle_x = cos_a * circle_x;
            let z_base = distance + circle_y * sin_a;
            let y_circle_term = circle_y * cos_a * cos_b;
            let x_circle_term = circle_y * cos_a * sin_b;

            for phi_index in 0..phi_steps {
                let sin_phi = SIN_PHI[phi_index];
                let cos_phi = COS_PHI[phi_index];

                let x = circle_x * (cos_b * cos_phi + sin_a * sin_b * sin_phi)
                    - x_circle_term;
                let y = circle_x * (sin_b * cos_phi - sin_a * cos_b * sin_phi)
                    + y_circle_term;
                let z = z_base + cos_a_circle_x * sin_phi;
                let z_inv = 1.0 / z;

                let l = cos_phi * cos_theta * sin_b
                    - cos_a * cos_theta * sin_phi
                    - sin_a * sin_theta
                    + cos_b * (cos_a * sin_theta - cos_theta * sin_a * sin_phi);

                if l <= 0.0 {
                    continue;
                }

                let luminance_index = ((l * 8.0) as usize).min(ramp_top);

                let x_proj = (half_width + k1 * z_inv * x) as usize;
                let y_proj = (half_height + k1 * z_inv * y * 0.8) as usize;

                if x_proj >= width || y_proj >= height {
                    continue;
                }

                let idx = y_proj * width + x_proj;
                if z_inv > zbuffer[idx] {
                    zbuffer[idx] = z_inv;
                    output[idx] = ramp[luminance_index];
                    // The shade index travels with the glyph, so the second pass
                    // does not have to search the ramp for a character whose
                    // index it already knew.
                    shade[idx] = luminance_index as u8;
                }
            }
        }

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                let symbol = output[idx];
                if symbol == ' ' {
                    continue;
                }
                let color = colors[shade[idx] as usize % colors.len()];
                buffer.set(x, y, Cell::new(symbol, color, style::Attribute::Bold));
            }
        }
    }
}

impl DefaultOptions for Donut {
    type Options = DonutOptions;

    fn default_options(width: u16, height: u16) -> Self::Options {
        DonutOptions {
            inner_radius: 1.0,
            outer_radius: 2.0,
            rotation_speed_a: 0.07,
            rotation_speed_b: 0.03,
            distance: 5.5,
            k1: (width.min(height) as f32) * 0.8,
            luminance_chars: vec![
                '.', ',', '-', '~', ':', ';', '=', '!', '*', '#', '$', '@',
            ],
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_recomputes_projection_scale() {
        let options = DonutOptions {
            k1: 99.0,
            k1_coeff: 1.0,
            ..Default::default()
        };
        let mut donut = Donut::new(options, (80, 40));

        donut.update_size(10, 20);

        assert_eq!(donut.screen_size, (10, 20));
        assert_eq!(donut.options.k1, 8.0);
    }
}
