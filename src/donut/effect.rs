use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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
        // These look absurdly small because they were per-frame values from when
        // the effect advanced a fixed step per rendered frame, and they are still
        // multiplied by 60 here to get the per-second rate that `advance` wants.
        // Read as radians per *second* they are 190x too slow: the torus turned
        // once every 4 minutes 46 seconds, which does not look like an animation
        // at all. As per-frame values at 60 Hz they are 1.32 and 0.60 rad/s, a
        // revolution every 4.8 and 10.5 seconds, which is the calm motion the
        // rest of the catalogue is tuned for. Do not "fix" them downwards.
        Self {
            inner_radius: 1.0,
            outer_radius: 2.0,
            rotation_speed_a: 1.32,
            rotation_speed_b: 0.60,
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

/// Gruvbox gradient, darkest first.
///
/// The ramp has to run from dim to bright: the index comes straight from the
/// brightness of a sample, with no inversion anywhere, so a low index is a dim
/// glyph and a high one is a bright one. This one ran the other way -- its first
/// five entries were the two lightest neutrals in the palette and its last two
/// were orange and red -- which put near-white cream on the `.` and `,` that
/// cover most of the torus and left the highlights dark, and read to the user as
/// a white broken doughnut.
///
/// Non-decreasing in perceived brightness, which `the_ramp_runs_from_dark_to_light`
/// pins. The hues are the palette's; only the order is new.
const COLORS: [style::Color; 12] = [
    style::Color::Rgb {
        r: 40,
        g: 40,
        b: 40,
    },
    style::Color::Rgb {
        r: 80,
        g: 73,
        b: 69,
    },
    style::Color::Rgb {
        r: 204,
        g: 36,
        b: 29,
    },
    style::Color::Rgb {
        r: 69,
        g: 133,
        b: 136,
    },
    style::Color::Rgb {
        r: 146,
        g: 131,
        b: 116,
    },
    style::Color::Rgb {
        r: 104,
        g: 157,
        b: 106,
    },
    style::Color::Rgb {
        r: 168,
        g: 153,
        b: 132,
    },
    style::Color::Rgb {
        r: 215,
        g: 153,
        b: 33,
    },
    style::Color::Rgb {
        r: 184,
        g: 187,
        b: 38,
    },
    style::Color::Rgb {
        r: 213,
        g: 196,
        b: 161,
    },
    style::Color::Rgb {
        r: 235,
        g: 219,
        b: 178,
    },
    style::Color::Rgb {
        r: 251,
        g: 241,
        b: 199,
    },
];

/// How many shades the brightness term is divided into.
///
/// One per colour, so every glyph in the ramp has exactly one colour and every
/// colour has exactly one glyph.
const SHADES: usize = COLORS.len();

/// The largest the brightness term gets.
///
/// The term is the z component of the surface normal, and the torus's geometry
/// pushes it past 1 -- over the sampled angles it reaches `sqrt(2)`. Dividing by
/// a fixed 8.0 therefore over-reaches the ramp: the top entry is reached at
/// 1.375 and everything brighter than that clamps onto it, so the last colour
/// covers a band of the brightest samples instead of being the top of a ramp.
/// This is the scale that puts the top of the ramp exactly at the brightest
/// sample, so every colour is used for the range it is there for.
const BRIGHTEST: f32 = std::f32::consts::SQRT_2;

/// Maps a sample's brightness onto the shade ramp.
///
/// Direct, and the reason the ramp has to run dark to light: nothing here
/// inverts the value, so index 0 is the dimmest thing drawn. Inlined because it
/// is called once per sample, a few hundred thousand times a frame, and the
/// scale is a constant division that folds away.
#[inline]
fn shade_index(brightness: f32) -> usize {
    let scale = SHADES as f32 / BRIGHTEST;
    ((brightness * scale) as usize).min(SHADES - 1)
}

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
    canvas: Canvas,
    rotation_a: f32,
    rotation_b: f32,
    colors: &'static [style::Color; 12],
    scratch: Scratch,
}

impl TerminalEffect for Donut {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.render_donut();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        // The canvas is the size of the screen, so it has to follow it. It used
        // not to: `render_donut` cleared the whole old-size surface and drew
        // only the new region, so the diff was up to as many cells again as the
        // screen -- and on a shrink, the strip between the two sizes was never
        // blanked, so the terminal kept stale pixels. `update_size` is a public
        // entry point and has to leave a renderable effect behind on its own;
        // the same is spelled out in the maze.
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
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
        let canvas = Canvas::new(screen_size.0, screen_size.1);
        Self {
            screen_size,
            options,
            canvas,
            rotation_a: 0.0,
            rotation_b: 0.0,
            colors: &COLORS,
            scratch: Scratch::default(),
        }
    }

    fn render_donut(&mut self) {
        self.canvas.clear();

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
        // A configured ramp is the user's and can be any length. The colour ramp
        // is fixed at twelve, so the shade index is clamped to the glyph ramp's
        // top rather than wrapped: a `min` in the inner loop rather than a
        // modulo by a length the compiler cannot see.
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

                let luminance_index = shade_index(l);

                let x_proj = (half_width + k1 * z_inv * x) as usize;
                let y_proj = (half_height + k1 * z_inv * y * 0.8) as usize;

                if x_proj >= width || y_proj >= height {
                    continue;
                }

                let idx = y_proj * width + x_proj;
                if z_inv > zbuffer[idx] {
                    zbuffer[idx] = z_inv;
                    output[idx] = ramp[luminance_index.min(ramp_top)];
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
                self.canvas.set(
                    x,
                    y,
                    Cell::new(symbol, color, style::Attribute::Bold),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Perceived brightness, so "dim" and "bright" mean something a test can
    /// compare. Rec. 601 luma, which is what a terminal's own rendering is
    /// closest to.
    fn brightness(color: &style::Color) -> f32 {
        match color {
            style::Color::Rgb { r, g, b } => {
                (0.299 * f32::from(*r)
                    + 0.587 * f32::from(*g)
                    + 0.114 * f32::from(*b))
                    / 255.0
            }
            _ => 1.0,
        }
    }

    /// The default spin has to actually resolve on a human timescale.
    ///
    /// `rotation_speed_a` is radians per *second*, and it used to be 0.022 --
    /// a value that reads as perfectly reasonable as radians per *frame*. Read
    /// per second it put one revolution every 4 minutes 46 seconds, and 10
    /// seconds of watching turned the torus 12 degrees. That does not read as a
    /// slow animation, it reads as a broken effect.
    #[test]
    fn the_default_spin_resolves_in_seconds_not_minutes() {
        let options = DonutOptions::default();

        for (name, speed) in [
            ("rotation_speed_a", options.rotation_speed_a),
            ("rotation_speed_b", options.rotation_speed_b),
        ] {
            let revolution = std::f32::consts::TAU / speed;
            assert!(
                (2.0..=12.0).contains(&revolution),
                "{name} is {speed} rad/s, so a revolution takes {revolution:.1}s; \
                 anything past a few seconds reads as a frozen effect"
            );
        }
    }

    /// The defaults are the old per-frame values scaled to per-second.
    ///
    /// Pinned as a ratio rather than as literals, so the relationship survives an
    /// edit to either side. The point of the note in `Default` is that these
    /// numbers look wrong unless you know where they came from.
    #[test]
    fn the_default_speeds_are_the_per_frame_values_at_60hz() {
        let options = DonutOptions::default();
        let frame = std::f32::consts::TAU;
        for (speed, per_frame) in [
            (options.rotation_speed_a, 0.022f32),
            (options.rotation_speed_b, 0.010f32),
        ] {
            assert!(
                (speed - per_frame * 60.0).abs() < 0.001,
                "{speed} is not {per_frame} rad/frame scaled to 60 Hz"
            );
        }
        // Both axes turn, and at different rates, or the torus is not tumbling.
        assert!(frame > 0.0);
        assert_ne!(options.rotation_speed_a, options.rotation_speed_b);
    }

    /// A second of wall clock has to move the torus a visible amount.
    ///
    /// The end-to-end version of the test above, through the same `advance` the
    /// frame loop calls, so it also catches a units regression in the
    /// multiplication rather than only in the default.
    #[test]
    fn a_second_of_elapsed_time_visibly_turns_the_torus() {
        let mut donut = Donut::new(DonutOptions::default(), (80, 40));
        let start = donut.rotation_a;
        donut.advance(1.0);
        let turned = donut.rotation_a - start;

        let degrees = turned.to_degrees();
        assert!(
            degrees > 20.0,
            "one second turned the torus {degrees:.1} degrees, which is not \
             something you can see"
        );
    }

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

    /// `update_size` on its own has to leave a renderable effect behind.
    ///
    /// It used to move `screen_size` and recompute `k1` and nothing else, so
    /// `render_donut` went on clearing the whole old-size canvas and drew only
    /// the new region. The diff then covered every cell of the old surface -- up
    /// to eight times the screen at 40x12, from 80x40 -- and on a shrink the
    /// strip between the two sizes was never blanked at all, so the terminal
    /// kept stale pixels there.
    #[test]
    fn a_resize_on_its_own_leaves_a_canvas_the_size_of_the_screen() {
        let mut donut = Donut::new(DonutOptions::default(), (80, 40));

        donut.update_size(10, 20);

        assert_eq!(
            donut.canvas.size(),
            (10, 20),
            "the canvas kept the old size"
        );
        let diff = donut.get_diff();
        assert!(
            diff.len() <= 10 * 20,
            "a 10x20 frame reported {} cells, which is more than the screen has",
            diff.len()
        );
        for (x, y, _) in &diff {
            assert!(*x < 10 && *y < 20, "drew outside the resized canvas");
        }
    }

    /// The other half of that contract: a shrink must not leave a strip behind.
    #[test]
    fn a_shrink_repaints_the_whole_screen() {
        let mut donut = Donut::new(DonutOptions::default(), (80, 40));
        donut.get_diff();

        donut.update_size(20, 10);
        let diff = donut.get_diff();

        assert!(!diff.is_empty(), "a shrink reported nothing to repaint");
        for (x, y, _) in &diff {
            assert!(*x < 20 && *y < 10, "drew outside the shrunk canvas");
        }
    }

    /// The ramp runs from dark to light.
    ///
    /// The index comes straight from the brightness of a sample, so a ramp that
    /// is lighter at its low indices than at its high ones puts near-white on
    /// the `.` and `,` that cover most of the torus and leaves the highlights
    /// dark. That is what the user was seeing as a white broken doughnut.
    #[test]
    fn the_ramp_runs_from_dark_to_light() {
        for (index, pair) in COLORS.windows(2).enumerate() {
            let (dim, bright) = (brightness(&pair[0]), brightness(&pair[1]));
            assert!(
                dim <= bright,
                "COLORS[{index}] is brighter ({dim:.2}) than COLORS[{}] \
                 ({bright:.2}), so the ramp is inverted",
                index + 1
            );
        }
        assert!(
            brightness(&COLORS[0]) < brightness(&COLORS[SHADES - 1]),
            "the ramp does not span a range of brightness at all"
        );
    }

    /// The whole glyph ramp is reachable, and every glyph has a colour.
    ///
    /// The old index was `l * 8.0` into a twelve-entry ramp with a brightness
    /// term that reaches `sqrt(2)`, so the scale and the ramp had nothing to do
    /// with each other. One shade per colour is what ties them together.
    #[test]
    fn the_shade_ramp_spans_every_glyph() {
        let options = DonutOptions::default();
        let ramp = shade_ramp(&options.luminance_chars);
        assert_eq!(
            SHADES,
            ramp.len(),
            "the colour ramp and the glyph ramp are different lengths, so \
             some glyphs are unreachable and some colours unused"
        );
        assert_eq!(shade_index(0.0), 0, "the dimmest sample is not the first");
        assert_eq!(
            shade_index(BRIGHTEST),
            SHADES - 1,
            "the brightest sample does not reach the top of the ramp"
        );
        // A quarter of the way up the brightness range is a quarter of the way
        // up the ramp, which is what "the whole ramp is used" means.
        for step in 0..SHADES {
            let brightness = BRIGHTEST * step as f32 / (SHADES - 1) as f32;
            assert_eq!(shade_index(brightness), step, "shade {step} skipped");
        }
    }

    /// End to end: a dim glyph is painted a dimmer colour than a bright one.
    #[test]
    fn a_drawn_frame_gets_brighter_with_its_glyphs() {
        let mut donut = Donut::new(DonutOptions::default(), (40, 20));
        let mut dimmest: Option<(char, f32)> = None;
        let mut brightest: Option<(char, f32)> = None;

        for step in 0..6u64 {
            donut.update_with_context(&crate::runtime::FrameContext::new(
                (40, 20),
                step,
                Duration::ZERO,
                Duration::from_secs_f64(1.0 / 30.0),
                crate::runtime::InputState::default(),
            ));
            for (_, _, cell) in donut.get_diff() {
                let level = brightness(&cell.color);
                if cell.symbol == '.' {
                    dimmest = Some((cell.symbol, level));
                }
                if cell.symbol == '@' {
                    brightest = Some((cell.symbol, level));
                }
            }
        }

        // The ramp's ends are the ones a user notices, so if the sampling never
        // reaches one of them there is nothing to compare and the test says so
        // rather than passing on nothing.
        let (dim_glyph, dim) = dimmest.expect("no dim glyph was ever drawn");
        let (bright_glyph, bright) =
            brightest.expect("no bright glyph was ever drawn");
        assert!(
            dim <= bright,
            "{dim_glyph} was painted at {dim:.2} and {bright_glyph} at \
             {bright:.2}: the ramp is inverted"
        );
    }
}
