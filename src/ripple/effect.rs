use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::halfblock::{HalfBlockField, ROWS_PER_CELL};
use crate::render::palette::{Palette, presets as palette_presets};
use crate::runtime::FrameContext;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Resolution of the sine lookup, over one full turn.
///
/// A wave from a point source is `sin(k * r - w * t)`, and `r` is a square root
/// of a sum of squares, so it cannot be factored per row or per column the way a
/// `sin(a + b)` can. The alternative is one `sin` call per source per pixel: at
/// three sources and a 400x200 terminal that is 480,000 calls a frame, which is
/// the same cost class as the plasma effect already at the top of this crate's
/// frame table.
///
/// Quantising the *argument* rather than the distance is what makes the table
/// work at all. Each source's own phase is folded into the index, so one table
/// serves all of them -- there is no need for a table per source.
///
/// 8192 rather than 4096, for the accuracy and not the coverage. One step of the
/// table is `2 * PI / 8192`, and the nearest entry is therefore at most half a
/// step away, so the worst error is about 3.8e-4 -- comfortably under anything a
/// colour ramp can show. At 4096 it was 7.7e-4, which is still visible as a
/// staircase on a smooth gradient, and the table is 32 KB either way.
const LUT_SIZE: usize = 8192;
const TWO_PI: f32 = std::f32::consts::TAU;

static SINE_LUT: LazyLock<Vec<f32>> = LazyLock::new(|| {
    (0..LUT_SIZE)
        .map(|i| (i as f32 * TWO_PI / LUT_SIZE as f32).sin())
        .collect()
});

/// The shipped colour depth of the ramp. See [`RippleOptions::levels`] for why
/// this is a performance lever rather than a taste.
const DEFAULT_LEVELS: usize = 16;

/// The table's entries per radian, as a multiplier.
///
/// The reciprocal, deliberately: this is used to turn an *angle* into a fraction
/// of a turn, and the table index is then that fraction times [`LUT_SIZE`]. The
/// tempting alternative is `LUT_SIZE / TAU`, which is a scale from radians
/// straight to an index -- and using it here multiplied by `LUT_SIZE` a second
/// time, so the argument was scaled by 4096 too many and `sine(0.001)` answered
/// `sin(4.1)`. The cast-based version before this one was accidentally correct,
/// because scaling to index units and then masking by the table size happens to
/// land on the same entry.
const INV_TWO_PI: f32 = 1.0 / TWO_PI;

/// Samples the table.
///
/// The reduction is `fractional part`, not a cast and not `%`. Both of the cheaper
/// options are wrong, and wrong in a way that looked fine: casting a negative
/// float to an integer saturates to zero in Rust, so every argument below minus one
/// turn read the table's first entry and came out as 0.0 -- and the field is
/// `k * r - time + phase`, so `- time` is negative for every pixel and the whole
/// effect was drawing a flat wash on the descending half of every wave.
/// `the_table_answers_for_every_argument_the_effect_produces` is what catches that
/// kind of thing; a test of the *shape* would not, because a wrong scale still
/// makes waves travel.
#[inline]
fn sine(argument: f32) -> f32 {
    let turns = argument * INV_TWO_PI;
    // `NaN` fails the comparison and would index at NaN and panic. Falling back to
    // zero is a flat wave, which is what a NaN deserves.
    let fraction = if turns.is_finite() {
        turns - turns.floor()
    } else {
        0.0
    };
    // Rounded, not truncated. Truncating biases every answer the same way by up
    // to a whole step -- a systematic negative bias of up to 1.5e-3, which is a
    // visible staircase on a smooth gradient rather than a rounding error one
    // cannot see.
    let index = ((fraction * LUT_SIZE as f32) + 0.5) as usize & (LUT_SIZE - 1);
    SINE_LUT[index]
}

/// One source of rings.
///
/// `phase` is what makes several sources interfere rather than merely overlap,
/// and it is also the whole of the cancellation case the tests use: two sources
/// at the same point half a turn apart sum to zero at every pixel.
#[derive(Debug, Clone, Copy)]
struct Source {
    /// Centre, in field-row coordinates.
    x: f32,
    y: f32,
    phase: f32,
    /// Drift, in field rows per second.
    vx: f32,
    vy: f32,
    /// Extra phase per second. Sources on different rates never re-synchronise,
    /// so a playlist entry does not visibly loop.
    rate: f32,
}

impl Source {
    /// Moves the source along its drift path, reflecting off the field edges.
    ///
    /// Reflected rather than wrapped: a source that teleports across the screen
    /// takes a ring pattern with it and the whole picture blinks.
    fn advance(&mut self, delta: f32, width: f32, height: f32) {
        self.x += self.vx * delta;
        self.y += self.vy * delta;
        if self.x < 0.0 {
            self.x = -self.x;
            self.vx = -self.vx;
        } else if self.x > width {
            self.x = 2.0 * width - self.x;
            self.vx = -self.vx;
        }
        if self.y < 0.0 {
            self.y = -self.y;
            self.vy = -self.vy;
        } else if self.y > height {
            self.y = 2.0 * height - self.y;
            self.vy = -self.vy;
        }
        // A field this small can reflect a source outside its own bounds, and an
        // out-of-range sample is not clamped by the renderer.
        self.x = self.x.clamp(0.0, width);
        self.y = self.y.clamp(0.0, height);
        self.phase += self.rate * delta;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RippleOptions {
    /// How many point sources there are.
    ///
    /// Two is the interesting minimum -- interference needs at least two waves to
    /// disagree -- and three is where the pattern stops reading as rings. More
    /// than that is a smear, and the cost is linear in this number.
    pub sources: u16,

    /// Rings per unit distance. Larger is a tighter pattern.
    ///
    /// 0.22, which is a wavelength of about 28 field rows, and that is set by the
    /// output path rather than by taste. The encoder emits a colour only when it
    /// differs from the last one written, so the byte volume is governed by how
    /// often the colour changes *between neighbouring cells*. At the 0.55 this
    /// started with, a ring was eleven cells across and a colour step landed
    /// roughly every two of them: measured at 400x200 that was 2.7 MB of escape
    /// sequences a frame and 4.8 ms encoding it, which is more than the whole rest
    /// of the crate's effects put together. At 0.22 a step lands every seventh
    /// cell, and the rings are wide enough to read as rings.
    pub wave_number: f32,

    /// Radians of phase per second, shared by every source: the speed of the
    /// rings travelling outwards, as opposed to the speed of the sources.
    pub speed: f32,

    /// Multiplies the built-in source drift. Zero pins the sources still, which
    /// leaves a standing interference pattern rather than a travelling one.
    pub drift: f32,

    /// A named ramp from [`crate::render::palette`]. `ocean` unless it says
    /// otherwise, and `ocean` is not a taste: it is the one ramp in the crate
    /// that is dark at the bottom and light at the top *and* wraps smoothly, so
    /// the crest of a ring is the bright part of the frame rather than whichever
    /// end of the ramp happens to land on the zero crossing.
    pub palette: String,

    /// How many distinct colours the ramp is quantised to.
    ///
    /// **This is the largest single lever on how much the effect writes to the
    /// terminal, and it is not a quality setting.** The output path emits a colour
    /// only when it differs from the last one it wrote, so a continuously
    /// interpolated gradient means *every* cell is a colour change. Measured at
    /// 400x200: unquantised, this effect emitted 2.79 MB of escape sequences a
    /// frame, forty times the mandelbrot's, and spent 5 ms encoding it. At 24
    /// levels the same picture is a handful of run-lengths.
    ///
    /// The banding is not visible because a half-block cell is two pixels and the
    /// ramp is smooth: 24 steps across a 400-column gradient is a step every
    /// seventeen cells.
    pub levels: usize,

    pub seed: u64,
}

impl Default for RippleOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file that
    /// omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            sources: 3,
            wave_number: 0.22,
            speed: 2.4,
            drift: 1.0,
            palette: "ocean".to_string(),
            levels: DEFAULT_LEVELS,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Ripple {
    screen_size: (u16, u16),
    options: RippleOptions,
    canvas: Canvas,
    field: HalfBlockField,
    sources: Vec<Source>,
    /// Elapsed simulation time. Advanced from the frame delta, never from a
    /// frame count, so the pattern travels at the same rate on any terminal.
    time: f32,
    /// Per-source horizontal squared offsets. `sources * width` floats, hoisted
    /// out of the inner loop because it does not depend on the row.
    dx_squared: Vec<f32>,
    /// One row of accumulated wave, `width` floats.
    row: Vec<f32>,
    rng: EffectRng,
    palette: Palette,
}

impl TerminalEffect for Ripple {
    fn get_diff(&mut self) -> Vec<(usize, usize, crate::buffer::Cell)> {
        self.draw();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.field
            .resize(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.time = 0.0;
        self.rng = seeded_rng(self.options.seed, "ripple");
        self.spawn_sources();
    }
}

impl Ripple {
    pub fn new(options: RippleOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);

        // Expanded to `levels` discrete stops so `sample_index` picks one of them
        // rather than interpolating. See `RippleOptions::levels`.
        let levels = options.levels.max(2);
        let palette = Palette::new(
            Palette::new(
                palette_presets::by_name(&options.palette)
                    .unwrap_or(palette_presets::OCEAN)
                    .to_vec(),
            )
            .expand(levels),
        );

        let mut ripple = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            field: HalfBlockField::new(
                screen_size.0 as usize,
                screen_size.1 as usize,
            ),
            options,
            sources: Vec::new(),
            time: 0.0,
            // Both scratch buffers are sized by `spawn_sources`, which is the
            // only thing that knows how many sources there are. Left empty here
            // just to satisfy the struct literal; it replaces them.
            dx_squared: Vec::new(),
            row: vec![0.0; screen_size.0 as usize],
            rng: seeded_rng(DEFAULT_SEED, "ripple"),
            palette,
        };
        ripple.rng = seeded_rng(ripple.options.seed, "ripple");
        ripple.spawn_sources();
        ripple
    }

    fn field_width(&self) -> usize {
        self.screen_size.0 as usize
    }

    fn field_height(&self) -> usize {
        self.screen_size.1 as usize * ROWS_PER_CELL
    }

    fn spawn_sources(&mut self) {
        // Clamped to two because interference needs at least two waves to
        // disagree, and to six because past that the rings stop being rings and
        // the cost is linear in a number that buys nothing.
        let count = self.options.sources.clamp(2, 6);
        let (w, h) = (self.field_width() as f32, self.field_height() as f32);
        let drift = self.options.drift;

        self.sources = (0..count)
            .map(|i| {
                // Spread the starting phases so the sources begin disagreeing
                // rather than all in step, and give each a different drift rate
                // so no two of them retrace the other's path.
                let spread = i as f32 / count as f32;
                Source {
                    x: self.rng.random_range(0.15..0.85) * w,
                    y: self.rng.random_range(0.15..0.85) * h,
                    phase: spread * TWO_PI + self.rng.random_range(0.0..0.5),
                    vx: (0.06 + 0.05 * spread) * drift,
                    vy: (0.04 + 0.07 * (1.0 - spread)) * drift,
                    rate: self.options.speed * (0.8 + 0.3 * spread),
                }
            })
            .collect();

        let wanted = self.sources.len() * self.field_width();
        if self.dx_squared.len() != wanted {
            self.dx_squared = vec![0.0; wanted];
        }
        let width = self.field_width();
        if self.row.len() != width {
            self.row = vec![0.0; width];
        }
    }

    fn advance(&mut self, delta: f32) {
        let (w, h) = (self.field_width() as f32, self.field_height() as f32);
        for source in &mut self.sources {
            source.advance(delta, w, h);
        }
        self.time += delta * self.options.speed;
    }

    /// One source's contribution at an offset from its centre.
    ///
    /// Takes the horizontal offset as an already-squared distance, because the
    /// render loop hoists that out and the tests do not: two copies of this
    /// expression is exactly how a hoisting optimisation ends up quietly computing
    /// something other than what the tests checked.
    #[inline]
    fn wave(source: &Source, dx_squared: f32, dy: f32, k: f32, time: f32) -> f32 {
        sine(k * (dx_squared + dy * dy).sqrt() - time + source.phase)
    }

    /// The summed wave at one pixel, in `-sources..sources`.
    ///
    /// Test-only, and deliberately so: nothing in the frame path needs a wave at
    /// an arbitrary point, because the frame path is the row pass. What the tests
    /// need is a way to ask about a single pixel, and this is it.
    ///
    /// It is not a second implementation. It calls the same [`Ripple::wave`] the
    /// row pass does, and `the_rendered_row_matches_the_sampled_field` is what
    /// holds the two together -- so an interference assertion here is an assertion
    /// about the arithmetic the picture is actually drawn from.
    #[cfg(test)]
    fn sample(&self, x: f32, y: f32, time: f32) -> f32 {
        let k = self.options.wave_number;
        self.sources
            .iter()
            .map(|source| {
                let dx = x - source.x;
                Self::wave(source, dx * dx, y - source.y, k, time)
            })
            .sum()
    }

    /// Maps a summed wave onto a colour.
    ///
    /// The scale is the source count rather than a constant, so the full palette
    /// is used whether there are two sources or six. Normalising by a fixed number
    /// leaves a two-source picture as a narrow band in the middle of the ramp,
    /// which reads as washed out rather than as a deliberate choice.
    fn colour_of(&self, value: f32) -> [f32; 3] {
        let scale = self.sources.len().max(1) as f32;
        let t = (value / scale * 0.5 + 0.5).clamp(0.0, 1.0);
        let levels = self.palette.len().max(2);
        let index = ((t * (levels - 1) as f32).round() as usize).min(levels - 1);
        match self.palette.sample_index(index) {
            crossterm::style::Color::Rgb { r, g, b } => {
                [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
            }
            _ => [0.0, 0.0, 0.0],
        }
    }

    fn draw(&mut self) {
        self.canvas.clear();

        let row_height = self.field_height();
        let time = self.time;
        self.prepare_offsets();

        for y in 0..row_height {
            self.accumulate_row(y as f32, time);
            for x in 0..self.field_width() {
                let rgb = self.colour_of(self.row[x]);
                self.field.set_row(x, y, rgb);
            }
        }

        self.field
            .write_to(&mut self.canvas, crossterm::style::Attribute::Reset);
    }

    /// Per-source horizontal squared offsets for every column.
    ///
    /// Hoisted out of both loops in the row pass because `(x - source.x)^2` does
    /// not depend on the row, and recomputed every frame because the sources move.
    /// Without it the inner loop does a subtract and a multiply per pixel per
    /// source for a value that was already known.
    fn prepare_offsets(&mut self) {
        let width = self.field_width();
        for (i, source) in self.sources.iter().enumerate() {
            let base = i * width;
            for x in 0..width {
                let dx = x as f32 - source.x;
                self.dx_squared[base + x] = dx * dx;
            }
        }
    }

    /// Sums every source into `row` for one field row.
    fn accumulate_row(&mut self, y: f32, time: f32) {
        let width = self.field_width();
        let k = self.options.wave_number;

        for value in self.row.iter_mut() {
            *value = 0.0;
        }
        for (i, source) in self.sources.iter().enumerate() {
            let base = i * width;
            let dy = y - source.y;
            for x in 0..width {
                self.row[x] +=
                    Self::wave(source, self.dx_squared[base + x], dy, k, time);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(sources: u16, seed: u64) -> RippleOptions {
        RippleOptions {
            sources,
            seed,
            ..RippleOptions::default()
        }
    }

    fn still(x: f32, y: f32, phase: f32) -> Source {
        Source {
            x,
            y,
            phase,
            vx: 0.0,
            vy: 0.0,
            rate: 0.0,
        }
    }

    /// Replaces the sources with an exact arrangement.
    ///
    /// The effect clamps to two sources because two is the minimum that
    /// interferes, so a one-source fixture has to replace the vector rather than
    /// ask for one.
    fn with_sources(ripple: &mut Ripple, sources: Vec<Source>) {
        ripple.sources = sources;
        let wanted = ripple.sources.len() * ripple.field_width();
        if ripple.dx_squared.len() != wanted {
            ripple.dx_squared = vec![0.0; wanted];
        }
    }

    /// Distance from the origin to the first crest along a ray, in field rows.
    fn first_crest(ripple: &Ripple) -> f32 {
        let (cx, cy) = (ripple.sources[0].x, ripple.sources[0].y);
        let step = 0.25;
        let mut r = 0.0f32;
        let mut previous = ripple.sample(cx + r, cy, ripple.time);
        while r < 40.0 {
            r += step;
            let current = ripple.sample(cx + r, cy, ripple.time);
            if current > previous && current > 0.9 {
                return r;
            }
            previous = current;
        }
        f32::NAN
    }

    #[test]
    fn the_rendered_row_matches_the_sampled_field() {
        // The render loop hoists `(x - source.x)^2` out of the row pass and the
        // tests sample without the hoist, so this is the one assertion that keeps
        // the two the same computation. Without it the interference tests below
        // would be testing a function the picture is not drawn from, which is the
        // "a test that stopped testing its subject" failure: it would keep passing
        // while the renderer drifted.
        let mut ripple = Ripple::new(options(3, 21), (48, 16));
        ripple.advance(0.7);
        ripple.prepare_offsets();

        for y in [0.0f32, 7.5, 31.0] {
            ripple.accumulate_row(y, ripple.time);
            for x in 0..ripple.field_width() {
                let rendered = ripple.row[x];
                let sampled = ripple.sample(x as f32, y, ripple.time);
                assert!(
                    (rendered - sampled).abs() < 1e-4,
                    "at ({x}, {y}) the row pass gave {rendered} and the sampler \
                     gave {sampled}"
                );
            }
        }
    }

    #[test]
    fn a_crest_moves_outwards_as_time_passes() {
        // The direct assertion of propagation. A standing pattern of rings that
        // never travels passes any check of the form "does the picture look like
        // ripples" and fails this one.
        let mut ripple = Ripple::new(options(2, 1), (80, 24));
        with_sources(&mut ripple, vec![still(40.0, 24.0, 0.0)]);

        let early = first_crest(&ripple);
        ripple.advance(1.0);
        let later = first_crest(&ripple);

        assert!(
            early.is_finite() && later.is_finite(),
            "no crest was found to track (early {early}, later {later})"
        );
        assert!(
            later > early + 0.5,
            "the first crest sat at {early} and then {later} a second later: the \
             pattern is standing still rather than travelling"
        );
    }

    #[test]
    fn two_sources_half_a_turn_apart_cancel_everywhere() {
        // The sharpest statement of interference available, and a direct one:
        // identical geometry, opposed phase, zero field.
        let mut ripple = Ripple::new(options(2, 5), (60, 20));
        with_sources(
            &mut ripple,
            vec![
                still(30.0, 20.0, 0.0),
                still(30.0, 20.0, std::f32::consts::PI),
            ],
        );

        let mut worst = 0.0f32;
        for y in 0..40 {
            for x in 0..60 {
                worst = worst.max(ripple.sample(x as f32, y as f32, 3.0).abs());
            }
        }
        assert!(
            worst < 1e-3,
            "two opposed sources summed to {worst} at their worst; they should \
             cancel to nothing"
        );
    }

    #[test]
    fn two_sources_in_step_double_the_amplitude() {
        let mut ripple = Ripple::new(options(2, 5), (60, 20));
        with_sources(
            &mut ripple,
            vec![still(30.0, 20.0, 0.0), still(30.0, 20.0, 0.0)],
        );

        let mut peak = 0.0f32;
        for y in 0..40 {
            for x in 0..60 {
                peak = peak.max(ripple.sample(x as f32, y as f32, 0.0).abs());
            }
        }
        // Two identical waves, so the peak is two rather than one. Anything much
        // below that means the second source is not being summed at all.
        assert!(peak > 1.9, "the doubled peak was only {peak}");
    }

    #[test]
    fn moving_a_source_changes_the_field_far_from_it() {
        // Interference is a property of the *separation*, not of the absolute
        // positions, so a cell far from both must still change when one moves.
        let mut ripple = Ripple::new(options(2, 6), (80, 24));
        with_sources(
            &mut ripple,
            vec![still(20.0, 20.0, 0.0), still(60.0, 20.0, 0.0)],
        );
        let far = (4.0, 4.0);
        let before = ripple.sample(far.0, far.1, 0.0);
        ripple.sources[1].x = 70.0;
        let after = ripple.sample(far.0, far.1, 0.0);
        assert!(
            (before - after).abs() > 1e-3,
            "moving a source left a far cell unchanged ({before} -> {after})"
        );
    }

    #[test]
    fn the_sine_table_agrees_with_the_real_function() {
        // The table is the whole reason this effect is affordable, and an
        // inaccurate one would quietly change the physics. Every entry, not a
        // sampled subset.
        for i in 0..LUT_SIZE {
            let expected = (i as f32 * TWO_PI / LUT_SIZE as f32).sin();
            assert!(
                (expected - SINE_LUT[i]).abs() < 1e-6,
                "table entry {i} is {}, expected {expected}",
                SINE_LUT[i]
            );
        }
    }

    #[test]
    fn the_table_answers_for_every_argument_the_effect_produces() {
        // Accuracy over the argument range the effect actually generates, which is
        // `k * r` for a radius up to the diagonal of the field plus the phase and
        // the elapsed time: a few thousand at the shipped settings.
        //
        // The tolerance is **one table step**, and it is worth being precise about
        // why that and not the half step the rounding alone would give. Rounding to
        // the nearest entry bounds the *table's* error at half a step, but the
        // reduction from radians to a fraction of a turn happens in `f32`, and at
        // an argument of a few thousand that arithmetic carries error of its own.
        // Measured over a dense sweep of the real range, the worst total is 0.77
        // steps.
        //
        // 0.77 steps is 5.9e-4 radians. Against the shortest wavelength the effect
        // can draw -- a ring one pixel wide at the shipped wave number -- that is
        // two hundredths of a pixel, so it is not a thing anyone can see. Loosening
        // the tolerance past what the measurement supports would be the wrong
        // trade; tightening it below what the f32 arithmetic allows would fail for
        // a reason that has nothing to do with the table.
        let step = TWO_PI / LUT_SIZE as f32;
        for argument in [
            0.0f32, 0.001, -0.001, 1.0, -1.0, 7.5, -7.5, 100.0, -100.0, 999.0,
            -999.0, 4096.0, -4096.0,
        ] {
            let value = sine(argument);
            assert!(
                (value - argument.sin()).abs() <= step,
                "sine({argument}) was {value}, real function says {}, which is more \
                 than a table step ({step}) away",
                argument.sin()
            );
        }
    }

    #[test]
    fn a_huge_argument_gives_a_finite_answer_rather_than_a_nan() {
        // `turns as usize` saturates rather than wrapping, so an argument past
        // `usize::MAX` has to still return something in range. It will not be the
        // *right* answer -- an f32 that large has no fractional part left to
        // resolve, so `f32::sin` is itself meaningless there -- and this test says
        // so rather than pretending otherwise. What it checks is that the table
        // cannot hand the renderer a NaN, which would propagate into a colour and
        // out to the wire as a nonsense escape sequence.
        for argument in [1.0e12f32, -1.0e12, f32::MAX, f32::MIN] {
            let value = sine(argument);
            assert!(
                value.is_finite() && (-1.0..=1.0).contains(&value),
                "sine({argument}) was {value}"
            );
        }
    }

    #[test]
    fn a_source_bounced_off_an_edge_stays_inside_the_field() {
        let mut source = Source {
            vx: -100.0,
            vy: -100.0,
            ..still(5.0, 5.0, 0.0)
        };
        for _ in 0..500 {
            source.advance(0.1, 20.0, 20.0);
            assert!(
                (0.0..=20.0).contains(&source.x)
                    && (0.0..=20.0).contains(&source.y),
                "a source escaped to ({}, {})",
                source.x,
                source.y
            );
        }
    }

    #[test]
    fn the_whole_ramp_is_used_at_two_sources_and_at_six() {
        // Normalising by a constant instead of the source count leaves a
        // two-source picture as a narrow band in the middle of the ramp, which
        // reads as washed out rather than as a choice.
        for count in [2u16, 6] {
            let ripple = Ripple::new(options(count, 2), (40, 12));
            let mut lowest = f32::INFINITY;
            let mut highest = f32::NEG_INFINITY;
            for y in 0..24 {
                for x in 0..40 {
                    let value = ripple.sample(x as f32, y as f32, 0.0);
                    lowest = lowest.min(value);
                    highest = highest.max(value);
                }
            }
            assert!(
                lowest < -0.9 && highest > 0.9,
                "with {count} sources the field spans {lowest}..{highest}, so the \
                 ramp is not being driven end to end"
            );
        }
    }

    #[test]
    fn a_pinned_palette_actually_changes_the_colours() {
        let a = Ripple::new(options(3, 3), (40, 12));
        let b = Ripple::new(
            RippleOptions {
                palette: "magma".to_string(),
                ..options(3, 3)
            },
            (40, 12),
        );
        assert_ne!(a.colour_of(0.5), b.colour_of(0.5));
    }

    #[test]
    fn an_unknown_palette_name_falls_back_rather_than_failing() {
        let ripple = Ripple::new(
            RippleOptions {
                palette: "not-a-palette".to_string(),
                ..options(3, 4)
            },
            (40, 12),
        );
        assert!(
            ripple.palette.len() > 1,
            "an unknown palette name left the ramp empty"
        );
    }

    #[test]
    fn every_emitted_cell_is_inside_the_canvas() {
        let mut ripple = Ripple::new(options(3, 8), (64, 20));
        for _ in 0..40 {
            ripple.advance(1.0 / 60.0);
            for (x, y, _) in ripple.get_diff() {
                assert!(x < 64 && y < 20, "({x}, {y}) is outside a 64x20 canvas");
            }
        }
    }
}
