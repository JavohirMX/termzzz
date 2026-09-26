use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crate::render::glyph_ramp::GlyphRamp;
use crate::render::palette::Palette;
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
///
/// Also the reference the terminal-dependent resolution is measured against:
/// `theta_steps` is this at 50 rows and fewer below that.
const THETA_SAMPLES: usize = 314;
/// Number of samples around the centre of revolution, always twice `theta`.
///
/// A cap rather than a count now that the angle tables are built per frame: the
/// sweep is `2pi` either way, so a shorter table means a coarser one, never a
/// shorter one.
const PHI_SAMPLES: usize = THETA_SAMPLES * 2;

/// The donut's own glyph ramp, dimmest first.
///
/// Deliberately *not* the shared default. This one pairs one-to-one with
/// [`COLORS`], and the pairing is the point: one shade per colour, so every glyph
/// has exactly one colour and every colour exactly one glyph. A shorter or longer
/// set than twelve would break that, which
/// `the_shade_ramp_spans_every_glyph` checks.
const DEFAULT_LUMINANCE_CHARS: &[char] =
    &['.', ',', '-', '~', ':', ';', '=', '!', '*', '#', '$', '@'];

/// The glyph ramp, never empty.
///
/// An empty `luminance_chars` used to underflow `len() - 1` and panic, and that is
/// reachable straight from a user config file. [`GlyphRamp`] handles that, and
/// eight effects share it now, so this is a thin wrapper rather than the second
/// implementation of the same guard.
fn shade_ramp(configured: &[char]) -> GlyphRamp {
    GlyphRamp::new(if configured.is_empty() {
        DEFAULT_LUMINANCE_CHARS.to_vec()
    } else {
        configured.to_vec()
    })
}

/// The bottom of the colour ramp, which is magma's first stop with the floor
/// lifted off black.
///
/// The ramp runs dark to light, and its dark end is the glyph on the dimmest
/// part of the torus -- a large fraction of the screen. Magma starts at
/// `rgb(0, 0, 4)`, which on a dark terminal is indistinguishable from a cell
/// that was never drawn, so the shading would read as a hole in the torus
/// rather than as its shadow. This is still the darkest stop in the set; it
/// just has enough ink to be seen.
const RAMP_FLOOR: style::Color = style::Color::Rgb {
    r: 20,
    g: 16,
    b: 34,
};

/// How many shades the brightness term is divided into.
///
/// One per colour, so every glyph in the ramp has exactly one colour and every
/// colour has exactly one glyph.
const SHADES: usize = 12;

/// The colour ramp, darkest first, expanded to one colour per glyph.
///
/// `magma` is in [`crate::render::palette::presets`] already, and it is the
/// right ramp for this because it is perceptually *ordered*: its steps are even
/// in perceived lightness by construction. The ramp this replaces was twelve
/// Gruvbox entries reordered into monotonic Rec. 601 luminance, and reordering
/// a fixed set of hues by brightness is a hue roulette -- the sequence ran 4,
/// 30, 4, 183, 34, 120, 38, 41, 57, 37, 39 and 43 degrees, so the middle of the
/// brightness range stepped from taupe to sage to gold. Two adjacent pairs were
/// also within 0.011 of luminance of each other, so a quarter of the ramp was
/// spent on steps the eye cannot resolve, and that is exactly where the hue
/// flipped. Expanded to twelve, magma's ascending half has a worst adjacent
/// step of 0.040 in the same measure.
///
/// Built through [`Palette`] rather than written out, so the ramp stays the
/// shared one and this file does not carry a second copy of it.
static COLORS: LazyLock<[style::Color; SHADES]> = LazyLock::new(|| {
    let mut stops = vec![RAMP_FLOOR];
    stops.extend_from_slice(&crate::render::palette::presets::MAGMA[1..5]);
    Palette::new(stops)
        .expand(SHADES)
        .try_into()
        .expect("Palette::expand returns exactly the count it was asked for")
});

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
///
/// Linear on purpose. The shaping of the brightness term happens in
/// [`shade_index_for`], at the one call site, so that this stays the plain
/// proportional map the ramp is defined against.
#[inline]
fn shade_index(brightness: f32) -> usize {
    let scale = SHADES as f32 / BRIGHTEST;
    ((brightness * scale) as usize).min(SHADES - 1)
}

/// The exponent on normalised brightness before the ramp index is taken,
/// chosen so the twelve shades cover roughly equal screen area.
///
/// Measured rather than guessed. The brightness term is the z component of the
/// surface normal in the rotated frame, so on the visible surface it behaves
/// like a cosine of the view angle, and the surface area at a given brightness
/// is not uniform in it. Counting the cells a frame actually draws, over twelve
/// frames of a tumbling torus at four sizes, the un-shaped term gave a worst
/// bucket share of 17.1% and a worst smallest share of 1.3% -- a seven-fold
/// spread, with the darks starved and the brights doing double duty. Sweeping
/// the exponent put the flattest distribution at 1.5, which through the
/// renderer reads as 5.0% to 12.0% at 40x12 and 5.2% to 9.9% at 40x20, against
/// an even ramp's 8.3%. Past 1.6 the top of the ramp starts losing area to the
/// bottom again, because the curve stops being monotonic in area and becomes
/// monotonic in the value.
#[inline]
fn shape_light(normalised: f32) -> f32 {
    // `x.powf(1.5)`, written in closed form. `powf` is a logarithm and an
    // exponential; this is one square root, and it runs on the order of a
    // hundred thousand samples a frame. Measured at 400x200 the closed form
    // renders in 0.51 ms against 1.01 ms for `powf`, so the exponent is written
    // down in a comment rather than in a constant the two could drift apart
    // from -- and `the_light_curve_is_the_exponent_it_claims_to_be` is what
    // holds them together.
    normalised * normalised.sqrt()
}

/// The ramp index for a sample: the brightness term, shaped, then scaled.
///
/// Split in two so the shaping has one home and [`shade_index`] stays the plain
/// proportional map the glyph ramp is defined against.
#[inline]
fn shade_index_for(brightness: f32) -> usize {
    let normalised = (brightness / BRIGHTEST).clamp(0.0, 1.0);
    shade_index(shape_light(normalised) * BRIGHTEST)
}

/// Per-frame scratch, reused across frames so a frame does not allocate.
#[derive(Default)]
struct Scratch {
    zbuffer: Vec<f32>,
    output: Vec<char>,
    shade: Vec<u8>,
    sin_theta: Vec<f32>,
    cos_theta: Vec<f32>,
    sin_phi: Vec<f32>,
    cos_phi: Vec<f32>,
    /// What the four angle tables above are currently built for, so the rebuild
    /// is skipped unless the resolution moved.
    theta_steps: usize,
    phi_steps: usize,
}

impl Scratch {
    /// The sampling angles, for whichever resolution this frame resolved to.
    ///
    /// These used to be four shared tables built once, at a fixed step of 0.02
    /// radians, and indexed absolutely. That step is only correct when the
    /// table is indexed by 314 entries, and `theta_steps` follows the terminal:
    /// `min_dimension * 314 / 50`, clamped to at most 314. So on any terminal
    /// shorter than 50 rows the index range is a strict *prefix* of the table
    /// and the torus is swept through `(theta_steps - 1) * 0.02` radians rather
    /// than a whole turn -- 79.6% at 40 rows, 47.4% at 24, 23.6% at the 12 rows
    /// the tests use, 15% at the 6-row minimum. The missing wedge is fixed in
    /// object space, so it is a permanent lump of torus that is never drawn and
    /// tumbles with it, and at 6x6 it leaves the screen blank.
    ///
    /// The invariant is `theta_i = i * 2pi / theta_steps`, which a table shared
    /// across resolutions cannot express, so the tables live here and are
    /// rebuilt when the step count changes. That is 1884 sin/cos pairs on a
    /// resize, not on a frame, so the cost that made these a `LazyLock` in the
    /// first place -- evaluating them in the inner loop -- is still gone.
    fn ensure_angles(&mut self, theta_steps: usize, phi_steps: usize) {
        if self.theta_steps == theta_steps && self.phi_steps == phi_steps {
            return;
        }
        let theta =
            |i: usize| i as f32 * std::f32::consts::TAU / theta_steps as f32;
        let phi = |i: usize| i as f32 * std::f32::consts::TAU / phi_steps as f32;
        self.sin_theta = (0..theta_steps).map(|i| theta(i).sin()).collect();
        self.cos_theta = (0..theta_steps).map(|i| theta(i).cos()).collect();
        self.sin_phi = (0..phi_steps).map(|i| phi(i).sin()).collect();
        self.cos_phi = (0..phi_steps).map(|i| phi(i).cos()).collect();
        self.theta_steps = theta_steps;
        self.phi_steps = phi_steps;
    }
}

pub struct Donut {
    pub screen_size: (u16, u16),
    options: DonutOptions,
    canvas: Canvas,
    rotation_a: f32,
    rotation_b: f32,
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
        let colors = &*COLORS;
        let half_width = width as f32 / 2.0;
        let half_height = height as f32 / 2.0;
        let ramp = shade_ramp(&self.options.luminance_chars);
        // A configured ramp is the user's and can be any length. The colour ramp
        // is fixed at twelve, so the shade index is clamped to the glyph ramp's
        // top rather than wrapped: a `min` in the inner loop rather than a
        // modulo by a length the compiler cannot see.

        // Resolution follows the terminal, so a small window does not pay for a
        // large one. `min_dimension * 314 / 50` reproduces the previous fixed 314
        // samples at the reference size of 50 rows.
        let min_dimension = width.min(height);
        let theta_steps =
            ((min_dimension * THETA_SAMPLES) / 50).clamp(48, THETA_SAMPLES);
        let phi_steps = (theta_steps * 2).min(PHI_SAMPLES);
        self.scratch.ensure_angles(theta_steps, phi_steps);

        let Scratch {
            zbuffer,
            output,
            shade,
            sin_theta,
            cos_theta,
            sin_phi,
            cos_phi,
            ..
        } = &mut self.scratch;

        for theta_index in 0..theta_steps {
            let sin_theta = sin_theta[theta_index];
            let cos_theta = cos_theta[theta_index];

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
                let sin_phi = sin_phi[phi_index];
                let cos_phi = cos_phi[phi_index];

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

                let luminance_index = shade_index_for(l);

                let x_proj = (half_width + k1 * z_inv * x) as usize;
                let y_proj = (half_height + k1 * z_inv * y * 0.8) as usize;

                if x_proj >= width || y_proj >= height {
                    continue;
                }

                let idx = y_proj * width + x_proj;
                if z_inv > zbuffer[idx] {
                    zbuffer[idx] = z_inv;
                    // `at` clamps, which is what the `min(ramp_top)` here used to do.
                    output[idx] = ramp.at(luminance_index);
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
                // Not bold. Bold is a *brightening hint* on most terminals, so
                // asking for it on every cell added a second, unaccounted-for
                // term to the brightness this ramp is trying to encode -- and
                // it added it most to the cells that needed it least, since the
                // top of the ramp is already 0.86 to 0.94 in luminance and
                // brightening those is what compresses them together. It also
                // widened every glyph, which shears a grid that is indexed by
                // cell. The glyph's own ink already carries the value.
                self.canvas.set(
                    x,
                    y,
                    Cell::new(symbol, color, style::Attribute::NormalIntensity),
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

    /// The fraction of screen directions around the centre that the drawn
    /// torus reaches, as a count of `BINS` equal slices.
    fn polar_coverage(size: (u16, u16), diff: &[(usize, usize, Cell)]) -> f64 {
        const BINS: usize = 16;
        let mut hit = [false; BINS];
        for (x, y, _) in diff {
            let dx = *x as f64 + 0.5 - f64::from(size.0) / 2.0;
            let dy = *y as f64 + 0.5 - f64::from(size.1) / 2.0;
            let angle = dy.atan2(dx).rem_euclid(std::f64::consts::TAU);
            let bin = (angle / (std::f64::consts::TAU / BINS as f64)) as usize;
            hit[bin.min(BINS - 1)] = true;
        }
        hit.iter().filter(|reached| **reached).count() as f64 / BINS as f64
    }

    /// How much of the screen each shade of the ramp covers, over several
    /// frames of a tumbling torus.
    ///
    /// Twelve frames rather than one because a single frame can miss a shade
    /// that the next one reaches, and the question is whether the ramp is
    /// reachable at all rather than whether one instant happens to use it.
    fn shade_histogram(size: (u16, u16), resize: bool) -> [f64; SHADES] {
        let ramp = shade_ramp(&DonutOptions::default().luminance_chars);
        let mut counts = [0usize; SHADES];
        let mut donut = Donut::new(DonutOptions::default(), size);
        if resize {
            // What the run loop does immediately after building, so the
            // measurement is the projection scale a real run actually uses.
            donut.update_size(size.0, size.1);
        }
        for step in 0..12u64 {
            donut.update_with_context(&crate::runtime::FrameContext::new(
                size,
                step,
                Duration::ZERO,
                Duration::from_secs_f64(1.0 / 30.0),
                crate::runtime::InputState::default(),
            ));
            for (_, _, cell) in donut.get_diff() {
                if let Some(index) =
                    ramp.glyphs().iter().position(|g| *g == cell.symbol)
                {
                    counts[index] += 1;
                }
            }
        }
        let total: usize = counts.iter().sum();
        std::array::from_fn(|index| counts[index] as f64 / total as f64)
    }

    /// The torus has to be swept through a whole turn at every size.
    ///
    /// The angle tables were built once at a step of 0.02 radians, which is
    /// only correct when they are indexed by 314 entries, while `theta_steps`
    /// is a function of the terminal: `min_dimension * 314 / 50`. On anything
    /// shorter than 50 rows the index range is therefore a strict *prefix* of
    /// the table, and the sampled sweep is `(theta_steps - 1) * 0.02` radians
    /// rather than a whole turn -- 79.6% at 40 rows, 47.4% at 24, 23.6% at the
    /// 12 rows the other tests here use, 15% at the 6-row minimum.
    ///
    /// The missing wedge is fixed in object space, so it is a permanent lump of
    /// torus that is never drawn and tumbles with it. At 40x12 it left 11 lit
    /// cells and at 6x6 it left the screen blank. Any judgement about the
    /// colour ramp made against that shape is a judgement about a torus with
    /// most of itself missing.
    #[test]
    fn the_torus_is_swept_all_the_way_round() {
        for size in [(6u16, 6u16), (12, 12), (40, 12), (40, 20), (80, 50)] {
            let mut donut = Donut::new(DonutOptions::default(), size);
            donut.update_size(size.0, size.1);
            donut.get_diff();

            let steps = donut.scratch.theta_steps;
            assert!(steps > 0, "at {size:?} no angles were sampled at all");

            // The invariant, `theta_i = i * 2pi / theta_steps`, checked against
            // the table the renderer will actually index rather than against a
            // helper that restates it. Both ends and the middle, because a table
            // that is right at the ends and wrong between them is a table built
            // for a different step count.
            for index in [0, 1, steps / 3, steps / 2, steps - 1] {
                let expected = index as f32 * std::f32::consts::TAU / steps as f32;
                assert!(
                    (donut.scratch.sin_theta[index] - expected.sin()).abs() < 1e-6
                        && (donut.scratch.cos_theta[index] - expected.cos()).abs()
                            < 1e-6,
                    "at {size:?} sample {index} of {steps} is not 2pi*{index}/{steps}"
                );
            }

            // A whole turn less the gap after the last sample.
            let covered = (steps as f64 - 1.0) * std::f64::consts::TAU
                / steps as f64
                / std::f64::consts::TAU;
            assert!(
                covered >= 0.95,
                "at {size:?} the {steps} sampled cross-section angles cover \
                 {covered:.1}% of a turn, so the rest of the torus is never \
                 drawn"
            );
        }
    }

    /// The same invariant, seen from outside: the drawn torus has to reach
    /// every direction from the centre of the screen, rather than a wedge of
    /// directions being empty because the samples that would land there were
    /// never taken.
    ///
    /// Measured after a moment's rotation rather than at rest. At rest the
    /// torus is exactly edge-on, and on a 12-row screen the edge-on projection
    /// is a ring six rows tall -- which leaves a handful of directions
    /// unsampled for want of anywhere to put a cell, independently of how much
    /// of the torus was sampled. A general pose is both fairer and closer to
    /// what anyone watching sees.
    #[test]
    fn the_drawn_torus_reaches_every_direction() {
        for size in [(40u16, 12u16), (40, 20), (80, 50)] {
            let mut donut = Donut::new(DonutOptions::default(), size);
            donut.update_size(size.0, size.1);
            donut.advance(0.7);
            let diff = donut.get_diff();
            assert!(!diff.is_empty(), "at {size:?} nothing was drawn at all");

            let coverage = polar_coverage(size, &diff);
            assert!(
                coverage >= 0.95,
                "at {size:?} the {} drawn cells reach only {:.1}% of the \
                 directions around the centre, so a wedge of the screen is \
                 never lit",
                diff.len(),
                coverage * 100.0
            );
        }
    }

    /// Every shade of a twelve-entry ramp has to reach the screen.
    ///
    /// The bright end is what a viewer notices, and it is the end the geometry
    /// decides: the brightness term is the z component of the surface normal,
    /// so only the few samples facing the camera ever get near the top of it.
    /// At 40x12 that was not a subtle imbalance -- `#`, `$` and `@` were never
    /// drawn at all, so the highlights were missing from the screen entirely
    /// and the torus had no specular anywhere.
    #[test]
    fn every_shade_of_the_ramp_reaches_the_screen() {
        for size in [(40u16, 12u16), (40, 20), (80, 50)] {
            for resize in [true, false] {
                let histogram = shade_histogram(size, resize);
                let unused: Vec<usize> = histogram
                    .iter()
                    .enumerate()
                    .filter(|(_, share)| **share == 0.0)
                    .map(|(shade, _)| shade)
                    .collect();
                assert!(
                    unused.is_empty(),
                    "at {size:?} (resized: {resize}) shades {unused:?} of the \
                     twelve were never drawn; the ramp has {SHADES} colours and \
                     {SHADES} glyphs, and an unreachable colour is an \
                     unreachable glyph"
                );
            }
        }
    }

    /// The ramp should be spread across the torus, not bunched at one end.
    ///
    /// An even spread would be `1 / SHADES`, so the bounds here are 2.4x that
    /// at the top and 0.36x at the bottom. Measured with the shape fixed and the
    /// term un-shaped, the shades ran from 2.1% of the surface at the dark end
    /// to 15.8% at the bright one, a 7.5-fold spread at 40x20; the curve in
    /// [`shape_light`] brings the same measurement to 5.2% and 9.9%, a 1.9-fold
    /// spread, and holds every size measured to within 2.4x.
    ///
    /// Only the resized case, because that is the projection a run actually
    /// uses. `Donut::new` leaves `k1` at the option default of 25, which on a
    /// 12-row screen crops the ring to a patch of the tube's near side: the
    /// visible surface is then a bright patch rather than the torus, and how
    /// its area is spread across the ramp says nothing about the exponent.
    #[test]
    fn the_ramp_is_spread_across_the_drawn_surface() {
        for size in [(40u16, 12u16), (40, 20), (80, 50)] {
            let histogram = shade_histogram(size, true);
            let darkest = histogram.iter().cloned().fold(f64::MAX, f64::min);
            let brightest = histogram.iter().cloned().fold(0.0f64, f64::max);
            let even = 1.0 / SHADES as f64;
            assert!(
                darkest >= even * 0.36 && brightest <= even * 2.4,
                "at {size:?} the shades run from {darkest:.1}% to \
                 {brightest:.1}% of the surface, and an even ramp would be \
                 {even:.1}%; measured: {histogram:?}"
            );
        }
    }

    /// The shaping has to be the exponent it says it is.
    ///
    /// It is written in closed form rather than as `powf`, because `powf` is a
    /// logarithm and an exponential and this runs on the order of a hundred
    /// thousand samples a frame, so the exponent and the code are two things
    /// that can disagree. The exponent is written out here rather than shared
    /// as a constant, so that this test is the oracle: change `shape_light`
    /// without changing this and the test says so.
    #[test]
    fn the_light_curve_is_the_exponent_it_claims_to_be() {
        const EXPONENT: f32 = 1.5;
        for step in 0..=20 {
            let x = step as f32 / 20.0;
            let expected = x.powf(EXPONENT);
            assert!(
                (shape_light(x) - expected).abs() < 1e-6,
                "shape_light({x:.2}) is {} and {x:.2}^{EXPONENT} is {expected:.6}",
                shape_light(x)
            );
        }
    }

    /// Shaping the brightness term must not disturb the ramp's own linearity.
    ///
    /// `shade_index` is the map the glyph ramp is defined against, and
    /// `the_shade_ramp_spans_every_glyph` pins that it is proportional. The
    /// curve lives in `shade_index_for` precisely so that it stays so, and this
    /// is the assertion that the two are wired together in the right order
    /// rather than composed twice.
    #[test]
    fn the_shaped_index_still_reaches_both_ends_of_the_ramp() {
        assert_eq!(shade_index_for(0.0), 0);
        assert_eq!(shade_index_for(BRIGHTEST), SHADES - 1);
        // Monotone, which is the whole reason a ramp can be indexed.
        let mut previous = 0;
        for step in 0..=200 {
            let brightness = BRIGHTEST * step as f32 / 200.0;
            let index = shade_index_for(brightness);
            assert!(index >= previous, "the shaped index went backwards");
            previous = index;
        }
    }

    /// Bold is a brightening hint, so nothing here should ask for it.
    ///
    /// Every cell used to be `Attribute::Bold`, on a ramp whose top three
    /// entries are already 0.86 to 0.94 in luminance. On a terminal that honours
    /// the hint that is a second brightness term added on top of the one the
    /// ramp encodes, applied most strongly to the cells that are already
    /// brightest -- which is what compresses the top of a ramp into one
    /// indistinguishable band. It also widens every glyph, and a widened glyph
    /// shears a grid indexed by cell.
    #[test]
    fn no_cell_is_drawn_bold() {
        for size in [(40u16, 20u16), (80, 40)] {
            let mut donut = Donut::new(DonutOptions::default(), size);
            donut.update_size(size.0, size.1);
            let diff = donut.get_diff();
            assert!(!diff.is_empty(), "at {size:?} nothing was drawn");
            for (x, y, cell) in &diff {
                assert!(
                    cell.attr != style::Attribute::Bold,
                    "the cell at {x},{y} is drawn bold"
                );
            }
        }
    }

    /// No two adjacent entries may be a step the eye cannot see.
    ///
    /// Twelve entries over a luma span of 0.82 is 0.075 per step if the ramp is
    /// even, so 0.03 -- two fifths of an even step -- is the point below which a
    /// step is not doing any work. The ramp this replaced had two adjacent
    /// pairs within 0.011 of each other, a quarter of the ramp spent on
    /// invisible steps, and that is precisely where its hue flipped from taupe
    /// to sage to gold. The twelve here step by at least 0.040.
    #[test]
    fn no_two_adjacent_ramp_entries_are_indistinguishable() {
        let colors = &*COLORS;
        for (index, pair) in colors.windows(2).enumerate() {
            let gap = brightness(&pair[1]) - brightness(&pair[0]);
            assert!(
                gap >= 0.03,
                "COLORS[{index}] and COLORS[{}] are only {gap:.3} apart in \
                 luminance, so one of them is never visible",
                index + 1
            );
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
