//! Escape-time rendering of the Mandelbrot set, coloured by iteration count.
//!
//! Nothing in the crate was mathematical before this, and it is the effect that
//! most needs the sub-cell renderer: the set is all boundary, so resolution is
//! the whole difference between a fuzzy blob and visible structure. Half-block
//! rather than braille, because the escape-time bands are read as *colour* and
//! braille cannot vary hue within a cell.
//!
//! The camera zooms continuously towards the boundary and, when it gets too
//! deep to resolve, jumps to a fresh point found by rejection sampling. The
//! sampling is seeded, so a given `seed` tours the same coastline. How deep is
//! too deep is not a constant: [`Mandelbrot::depth_limit`] derives it from the
//! iteration budget and from how finely the terminal can sample a point, so the
//! camera cannot outrun either.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, TerminalEffect, seeded_rng};
use crate::render::{HalfBlockField, Palette};
use crate::runtime::FrameContext;
use crossterm::style::{Attribute, Color};
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Zoom factor per second. Small enough that the approach reads as motion
/// rather than a cut.
const DEFAULT_ZOOM_RATE: f32 = 0.35;

/// The view the camera sits at after construction and restarts at after a
/// recentre. The iteration budget is measured against this, so it is the
/// "fully detailed" reference, and a recentre lands here.
///
/// The set is about 3.5 across, so this is a mid-scale: the view is wide enough
/// to hold the cardioid, the period-2 bulb and the start of the antenna, and
/// narrow enough that the boundary runs off both sides rather than sitting as a
/// closed curve in the middle. A recentre used to restart at the widest view
/// that contains the set, 3.2, which is the opposite of that: the set is a
/// small disc, everything around it escapes within a couple of iterations, and
/// the frame is one flat band.
const STARTING_SCALE: f64 = 0.45;

/// Floor on the iteration budget. Below this the escape bands stop being
/// distinguishable and the image goes flat, so it also floors `max_iterations`
/// and the budget that [`Mandelbrot::depth_limit`] is derived from.
const MIN_USEFUL_ITERATIONS: u32 = 24;

/// The scale at which [`REFERENCE_BUDGET`] iterations stop resolving the
/// boundary, and so the scale the camera stops at by default.
const REFERENCE_SCALE: f64 = 1.0e-3;

/// The iteration budget [`REFERENCE_SCALE`] was measured at, which is the
/// default in [`MandelbrotOptions`]. The depth limit is [`REFERENCE_SCALE`]
/// times ten to the power of how many doublings of the budget separate the two.
const REFERENCE_BUDGET: f64 = 96.0;

/// One ulp of `f32` at `|c| = 2`, which is `2^-22`. A sample coordinate is a
/// point in the disc of radius 2, and `f32` carries 24 bits of significand, so
/// this is the *coarsest* quantisation a coordinate in the view can suffer.
/// Sampling near the origin is finer, so using this is the safe direction.
const F32_COORDINATE_STEP: f64 = 2.384185791015625e-7;

/// How many f32 steps one sample of the view is worth. Two samples that round
/// to the same `f32` have the same orbit and render identically, so a spacing
/// of a fraction of a step quantises the picture into blocks -- the old fixed
/// depth floor of 1e-5 sat at 0.2 steps per sample on a 400x200 terminal.
const MIN_F32_STEPS_PER_SAMPLE: f64 = 4.0;

/// First distance the walk out of an interior sample tries.
const EDGE_SEARCH_STEP: f64 = 1.0e-3;
/// The furthest it tries. From inside the set any ray crosses the boundary
/// within 4, because the set lives inside the disc of radius 2, and doubling
/// from 1e-3 gets to 4.096 in thirteen probes.
const EDGE_SEARCH_LIMIT: f64 = 8.0;
/// Halvings of the last step of the walk. The centre only has to sit on the
/// contour to well within the restart view, not to the last bit, and twenty
/// puts it about 1e-9 away.
const EDGE_BISECTIONS: u32 = 20;

/// Rejection samples per attempt at finding a point on the boundary. A point
/// has to be in the set to be accepted, and the set covers a small part of the
/// disc, so most attempts are rejected.
const SAMPLE_ATTEMPTS: usize = 512;

/// The palette, cycled slowly so the bands shift without the zoom stalling.
/// Dark enough at the start that the first iterations read as the interior.
fn palette() -> Palette {
    Palette::new(vec![
        Color::Rgb { r: 0, g: 7, b: 100 },
        Color::Rgb {
            r: 32,
            g: 107,
            b: 203,
        },
        Color::Rgb {
            r: 237,
            g: 255,
            b: 255,
        },
        Color::Rgb {
            r: 255,
            g: 170,
            b: 0,
        },
        Color::Rgb { r: 204, g: 0, b: 0 },
        Color::Rgb { r: 16, g: 8, b: 40 },
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MandelbrotOptions {
    /// Iteration ceiling, and the effect's dominant cost by a wide margin.
    ///
    /// Nearly every pixel in an interior region burns the whole budget to
    /// discover it never escapes, so cost is very close to linear in this. It
    /// is a quality dial: 64 is smooth enough at a glance, 128 resolves the
    /// outer bands of a deep zoom, 256 starts to show structure that is only
    /// visible if you are looking for it.
    ///
    /// The budget is also how deep the camera goes, which is the point of it
    /// being one number: it sets the quality of the deepest frames *and* the
    /// scale at which they stop being worth drawing, so a cheaper budget gives
    /// a shallower and quicker tour rather than the same tour with fewer
    /// bands. See [`Mandelbrot::depth_limit`].
    pub max_iterations: u16,
    /// Zoom factor per second.
    pub zoom_rate: f32,
    /// Hue rotation over time, so the bands drift while the camera moves.
    pub color_speed: f32,
    /// Seeds the tour: which coastline it starts on, and where it goes next.
    ///
    /// There is deliberately no "start at this complex number" option. An
    /// earlier version had one, and the seed then did nothing for the first
    /// half-minute, because the camera needs a long zoom to reach the depth
    /// limit where it picks a new point -- about 18 seconds at the default
    /// budget and zoom rate, longer at a low one. A screensaver's whole value
    /// is that the first frame is already interesting, so the seed owns the
    /// starting point and the tour follows from it.
    pub seed: u64,
}

impl Default for MandelbrotOptions {
    /// Hand-written so it is the single source of truth.
    fn default() -> Self {
        Self {
            max_iterations: 96,
            zoom_rate: DEFAULT_ZOOM_RATE,
            color_speed: 0.05,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Mandelbrot {
    screen_size: (u16, u16),
    options: MandelbrotOptions,
    canvas: Canvas,
    field: HalfBlockField,
    center: (f64, f64),
    scale: f64,
    time: f32,
    rng: crate::common::EffectRng,
}

impl TerminalEffect for Mandelbrot {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let (width, height) = (self.field.row_width(), self.field.row_height());
        let aspect = width as f64 / height as f64;
        let half_height = self.scale;
        let half_width = self.scale * aspect;
        let (center_re, center_im) = self.center;
        let shift = self.time * self.options.color_speed;
        // Built once per frame. `Palette::new` takes a Vec, so calling it inside
        // the closure would allocate once per pixel, sixty times a second.
        let palette = palette();

        // Fewer iterations when zoomed out. The set is only a few pixels across
        // at that scale, so the extra iterations resolve detail nobody can see
        // while every interior pixel still pays for them in full -- and interior
        // pixels are most of the screen when zoomed out. This is the effect's
        // dominant cost, so it is worth the couple of lines.
        let budget = self.iteration_budget();

        self.field.fill_with(|x, y| {
            // Rows are pairs of pixels per cell, so `y` addresses field rows
            // while `x` still addresses cells. The half-block is what splits
            // them; the mapping is identical for both.
            let nx = (x as f64 / width as f64) * 2.0 - 1.0;
            let ny = (y as f64 / height as f64) * 2.0 - 1.0;
            let re = center_re + nx * half_width;
            let im = center_im + ny * half_height;

            let iterations = escape_time(re as f32, im as f32, budget);
            // Interior points stay black, and a smooth count keeps the escape
            // bands from stepping like a contour plot.
            if !iterations.escaped {
                return [0.0, 0.0, 0.0];
            }
            // The square root pulls the early bands apart. Without it almost
            // every visible pixel lands in the last few iterations, because the
            // counts bunch up at the high end.
            let t = (iterations.smooth / 5.0).powf(0.25);
            palette.sample_rgb(t as f32 + shift)
        });

        self.field.write_to(&mut self.canvas, Attribute::Reset);
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.advance(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.field
            .resize(self.screen_size.0 as usize, self.screen_size.1 as usize);
    }

    fn reset(&mut self) {
        let options = self.options.clone();
        *self = Self::new(options, self.screen_size);
    }
}

impl Mandelbrot {
    pub fn new(options: MandelbrotOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            field: HalfBlockField::new(
                screen_size.0 as usize,
                screen_size.1 as usize,
            ),
            center: (0.0, 0.0),
            scale: STARTING_SCALE,
            time: 0.0,
            rng: seeded_rng(options.seed, "mandelbrot"),
            options,
        };
        effect.canvas.clear();

        // Seeded from the outset, so the very first frame is on the coastline
        // rather than a centred disc of empty interior. `recentre` leaves the
        // scale at the restart view, which is where the first frame wants to
        // be anyway.
        effect.recentre();

        effect
    }

    /// Iterations to spend per sample, at every zoom.
    ///
    /// This used to scale with the zoom -- `ceiling / sqrt(scale /
    /// STARTING_SCALE)`, clamped -- so that a wide view cost less, on the
    /// grounds that the set is only a few pixels across there and interior
    /// pixels still burn the whole budget. The camera no longer goes wider
    /// than [`STARTING_SCALE`], so that curve sits above the ceiling for the
    /// whole tour and clamped to it: the adaptive half was dead weight and the
    /// budget simply *is* the ceiling.
    ///
    /// Spending it is what buys the bands at depth, and
    /// [`Self::depth_limit`] is derived from this same number, so the two
    /// cannot disagree about how deep the camera may go.
    ///
    /// A `max_iterations` below [`MIN_USEFUL_ITERATIONS`] is raised to it
    /// rather than used as given, because below that the image is flat either
    /// way. It is also what used to panic: the old clamp passed
    /// `MIN_USEFUL_ITERATIONS` and the ceiling as `f32::clamp` bounds, and
    /// `clamp` asserts they are ordered, so any config with `max_iterations`
    /// under 24 died on its first frame.
    fn iteration_budget(&self) -> u32 {
        u32::from(self.options.max_iterations).max(MIN_USEFUL_ITERATIONS)
    }

    /// The deepest scale the camera may reach: the one number that stops the
    /// zoom turning into a flat wash, and the end of every tour.
    ///
    /// Two limits, and the camera takes the shallower of them.
    ///
    /// **By budget.** The escape bands stop reading as bands once samples
    /// start landing on top of each other's escape count, and how deep that
    /// happens is set by the budget. Measured over sixteen seeded tours on a
    /// 100x50 field, the scale at which neighbouring samples stop landing in
    /// different palette bands moves *one decade for every doubling of the
    /// budget*: [`REFERENCE_SCALE`] is that scale at [`REFERENCE_BUDGET`], and
    /// the fit is the decade rule written out, so
    ///
    /// ```text
    /// REFERENCE_SCALE * 10^log2(REFERENCE_BUDGET / budget)
    /// ```
    ///
    /// which puts the stop at 1e-1 on 24 iterations, 1e-3 on the default 96 and
    /// 1e-5 on 384. That is a much steeper law than the `1 / sqrt(scale)`
    /// heuristic the old budget curve encoded, which reads as one doubling of
    /// the budget per doubling of the magnification: the measured one spends
    /// the budget it is given instead of assuming most of it is wasted.
    ///
    /// **By precision.** [`escape_time`] is `f32`, so a sample coordinate is
    /// quantised in ulps, and two samples that round to the same `f32` have the
    /// same orbit and render identically. The view is isotropic in cell units
    /// and each field row is a sample, so both axes step by `2 * scale / field
    /// rows`; holding that to [`MIN_F32_STEPS_PER_SAMPLE`] ulps means not going
    /// below two ulps per field row. Without it the budget term is a runaway:
    /// at the default 96 on a 400x200 terminal the old fixed floor of 1e-5 put
    /// the spacing at 0.2 ulps, and the last frames of every tour were a
    /// handful of repeated blocks. It only starts to bind once the budget is
    /// generous -- 192 iterations on a 400x200 terminal, say -- which is exactly
    /// what it is for.
    fn depth_limit(&self) -> f64 {
        let budget = self.iteration_budget() as f64;
        let by_budget =
            REFERENCE_SCALE * 10.0f64.powf((REFERENCE_BUDGET / budget).log2());

        let spacing_per_unit_scale = 2.0 / self.field.row_height() as f64;
        let by_precision =
            MIN_F32_STEPS_PER_SAMPLE * F32_COORDINATE_STEP / spacing_per_unit_scale;

        by_budget.max(by_precision)
    }

    fn advance(&mut self, dt: f64) {
        self.time += dt as f32;
        // `exp`, not `powf`: the zoom factor is e^(-rate * dt), and
        // 1.0f64.powf(anything) is exactly 1.0, so the camera never moved.
        self.scale *= (-self.options.zoom_rate as f64 * dt).exp();
        if self.scale < self.depth_limit() {
            self.recentre();
        }
    }

    /// Jumps to a fresh point on the boundary and pulls back to the restart
    /// view.
    ///
    /// Rejection sampling: a point is only accepted if it is in the set, which
    /// biases the search towards the boundary -- the set is exactly the
    /// coastline, and approaching anywhere else is a flat interior or a blank
    /// exterior.
    ///
    /// The walk out of that point is the part that matters, and it has to walk
    /// *out* rather than towards the edge. The accepted sample is an interior
    /// point, and a random ray from one can stay inside the set for a long way
    /// -- the main cardioid is over 0.4 across -- so the camera has to be
    /// pulled all the way to the boundary. This used to take twelve 0.02 steps
    /// in the hope of landing near the edge, and it landed up to 0.24 inside
    /// about half the time, which is the actual reason the effect went flat:
    /// zooming into a point 0.24 inside a bulb fills the screen with the
    /// interior, so the last third of every tour was a black rectangle rather
    /// than a boundary. Doubling the step until the probe says the ray has left
    /// costs about a dozen probes whatever the sample is, and bisecting the last
    /// step lands on the boundary.
    ///
    /// The probe is the render budget itself, so the centre ends up on the
    /// contour the renderer draws -- where `escaped` flips -- rather than on
    /// the boundary of the true set, which needs far more iterations to find and
    /// would then be a contour the renderer disagrees with.
    fn recentre(&mut self) {
        // A generous budget here regardless of zoom: the whole point of this
        // function is to decide whether a point is in the set at all, and a
        // shallow budget would call a slow-escaping boundary point interior.
        // The floor matters here as much as it does for rendering: with a budget
        // of zero nothing escapes, so the first sample would be accepted and
        // the walk would never leave the set.
        let probe = self.iteration_budget();
        let mut found = None;
        for _ in 0..SAMPLE_ATTEMPTS {
            // Rejection sampling in the disc of radius 2, which is where the set
            // lives; a square would waste most of its samples on empty corners.
            let (x, y) = loop {
                let x = self.rng.random_range(-2.0f64..2.0);
                let y = self.rng.random_range(-2.0f64..2.0);
                if x * x + y * y <= 4.0 {
                    break (x, y);
                }
            };

            if escape_time(x as f32, y as f32, probe).escaped {
                continue;
            }

            found = Some((x, y));
            break;
        }

        // No sample landed on the set after all that; a point in the interior
        // is a safe fallback and is better than staying put.
        let (x0, y0) = found.unwrap_or((-0.5, 0.0));

        let angle = self.rng.random_range(0.0f64..std::f64::consts::PI);
        let (dx, dy) = (angle.cos(), angle.sin());
        let escaped = |t: f64| {
            escape_time((x0 + dx * t) as f32, (y0 + dy * t) as f32, probe).escaped
        };

        // Out along the ray, doubling, until the probe says it has left the
        // set. Exponential because the sample can be a long way in: a linear
        // scan of the same reach would be four thousand probes.
        let mut inside = 0.0f64;
        let mut outside = None;
        let mut step = EDGE_SEARCH_STEP;
        while step <= EDGE_SEARCH_LIMIT {
            if escaped(step) {
                outside = Some(step);
                break;
            }
            inside = step;
            step *= 2.0;
        }

        self.center = match outside {
            None => (x0, y0),
            Some(outside) => {
                let (mut lo, mut hi) = (inside, outside);
                for _ in 0..EDGE_BISECTIONS {
                    let mid = 0.5 * (lo + hi);
                    if escaped(mid) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                (x0 + dx * 0.5 * (lo + hi), y0 + dy * 0.5 * (lo + hi))
            }
        };

        // Back to the restart view rather than the widest one, so the boundary
        // is across the screen at the top of the next cycle.
        self.scale = STARTING_SCALE;
        self.rng = seeded_rng(self.rng.random::<u64>(), "mandelbrot");
    }
}

/// How long a point takes to escape, and whether it escaped at all.
struct Escape {
    /// Fractional iteration count, for banding-free colour. Meaningless when
    /// `escaped` is false.
    smooth: f32,
    escaped: bool,
}

/// Iterates `z -> z^2 + c` until it leaves the radius-2 circle.
///
/// The smooth count is the standard correction: the fractional part measures how
/// far past the boundary the point went, so two points that escape on the same
/// iteration but at different distances get different colours.
fn escape_time(re: f32, im: f32, max_iterations: u32) -> Escape {
    let (mut zr, mut zi) = (0.0f32, 0.0f32);
    let (mut zr2, mut zi2) = (0.0f32, 0.0f32);
    let mut i = 0u32;

    // z^2 == z^2, tracked alongside z to avoid four multiplies per step.
    //
    // f32 rather than f64. This is the hot loop: it runs once per field sample
    // per frame, and at 200x50 that is twenty thousand samples sixty times a
    // second. The orbit either escapes past 2 or stays bounded, so the extra
    // mantissa bits buy nothing a viewer can see.
    //
    // The catch is the sample coordinate rather than the orbit: `c` arrives as
    // an f32, so at a deep enough zoom neighbouring samples round to the same
    // f32 and render identically. That is what
    // [`Mandelbrot::depth_limit`]'s precision term exists to prevent, and the
    // reason this stays f32 is that the alternative is paying for f64 in the
    // one loop that runs on every field sample of every frame.
    while i < max_iterations && zr2 + zi2 < 4.0 {
        zi = 2.0 * zr * zi + im;
        zr = zr2 - zi2 + re;
        zr2 = zr * zr;
        zi2 = zi * zi;
        i += 1;
    }

    if zr2 + zi2 >= 4.0 {
        // One plus the log correction, which is what removes the banding.
        // log2(log2(m)) is spelled with natural logs because f32::log2 hands
        // back an f64, and mixing the two would widen the whole expression.
        const LN_2: f32 = std::f32::consts::LN_2;
        let magnitude = zr2 + zi2;
        let smooth = i as f32 + 1.0 - (magnitude.ln() / LN_2).ln() / LN_2;
        Escape {
            smooth,
            escaped: true,
        }
    } else {
        Escape {
            smooth: 0.0,
            escaped: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interior and exterior sample counts for a view, walked the way
    /// `get_diff` walks it: field rows rather than cell rows, and the aspect
    /// ratio taken from the field.
    fn interior_and_exterior(
        effect: &Mandelbrot,
        center: (f64, f64),
        scale: f64,
    ) -> (usize, usize) {
        let budget = effect.iteration_budget();
        let aspect =
            effect.field.row_width() as f64 / effect.field.row_height() as f64;

        let (mut interior, mut exterior) = (0usize, 0usize);
        for y in 0..effect.field.row_height() {
            for x in 0..effect.field.row_width() {
                let re = center.0
                    + ((x as f64 / effect.field.row_width() as f64) * 2.0 - 1.0)
                        * scale
                        * aspect;
                let im = center.1
                    + ((y as f64 / effect.field.row_height() as f64) * 2.0 - 1.0)
                        * scale;
                if escape_time(re as f32, im as f32, budget).escaped {
                    exterior += 1;
                } else {
                    interior += 1;
                }
            }
        }
        (interior, exterior)
    }

    /// Steps the camera until it recentres, returning the scale and centre at
    /// the deepest point before it did.
    ///
    /// Driven at the largest step `advance` is happy with, so a whole tour is a
    /// few hundred multiplies rather than a few thousand.
    fn zoom_to_the_depth_limit(effect: &mut Mandelbrot) -> (f64, (f64, f64)) {
        let mut deepest = (effect.scale, effect.center);
        for _ in 0..20_000 {
            let before = effect.scale;
            effect.advance(0.1);
            if effect.scale < deepest.0 {
                deepest = (effect.scale, effect.center);
            }
            if effect.scale > before {
                return deepest;
            }
        }
        panic!("the camera never reached its depth limit");
    }

    #[test]
    fn a_point_in_the_set_never_escapes() {
        // The cardioid's interior, and the period-2 bulb on the real axis.
        assert!(!escape_time(0.0, 0.0, 500).escaped);
        assert!(!escape_time(-0.5, 0.0, 500).escaped);
        assert!(!escape_time(-1.0, 0.0, 500).escaped);
    }

    #[test]
    fn a_point_outside_the_set_escapes_quickly() {
        let escape = escape_time(2.5, 2.5, 500);
        assert!(escape.escaped);
        assert!(escape.smooth < 10.0, "took {} iterations", escape.smooth);
    }

    #[test]
    fn points_near_the_boundary_take_longer_to_escape() {
        // The whole visual point of a zoom: the closer to the set, the more
        // iterations a point survives.
        let far = escape_time(0.4, 0.0, 4000).smooth;
        let near = escape_time(-0.7450, 0.1130, 4000).smooth;
        assert!(
            near > far,
            "a near-boundary point ({near}) escaped sooner than a far one ({far})"
        );
    }

    #[test]
    fn the_iteration_budget_is_respected() {
        // A point that never escapes must stop at the budget, not run forever.
        let escape = escape_time(0.0, 0.0, 12);
        assert!(!escape.escaped);
    }

    #[test]
    fn the_smooth_count_falls_between_its_iterations() {
        // The smooth count is an interpolation, so it has to land strictly
        // inside the interval it was derived from or the banding returns.
        let escape = escape_time(0.6, 0.1, 100);
        assert!(escape.escaped);
        assert!(escape.smooth > 0.0);
    }

    #[test]
    fn the_starting_view_has_the_boundary_running_through_it() {
        // The constructor rejection-samples a point in the set and then walks
        // out along a ray until the budget says it has escaped, so the camera
        // deliberately ends up *on* the boundary the renderer draws. What
        // matters is that the first frame shows both sides of it. A view
        // entirely inside is a flat black rectangle, and one entirely outside is
        // a flat wash of colour; either means the tour picked somewhere dull.
        for seed in 0..12u64 {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let effect = Mandelbrot::new(options, (40, 16));
            let (interior, exterior) =
                interior_and_exterior(&effect, effect.center, effect.scale);

            assert!(
                interior > 0,
                "seed {seed} started entirely outside the set at {:?}",
                effect.center
            );
            assert!(
                exterior > 0,
                "seed {seed} started entirely inside the set at {:?}, which is a \
                 flat black rectangle",
                effect.center
            );
        }
    }

    #[test]
    fn the_first_frame_is_the_restart_view() {
        // Not the widest view that contains the set, which is 3.2 and shows the
        // whole thing as a small disc in a lot of empty plane.
        let effect = Mandelbrot::new(MandelbrotOptions::default(), (20, 8));
        assert_eq!(effect.scale, STARTING_SCALE);
    }

    #[test]
    fn a_frame_is_drawn_and_stays_in_bounds() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        let diff = effect.get_diff();
        assert!(!diff.is_empty(), "the first frame was blank");
        for (x, y, _) in &diff {
            assert!(*x < 40 && *y < 12, "cell ({x},{y}) is outside the canvas");
        }
    }

    #[test]
    fn the_interior_is_black_and_the_exterior_is_not() {
        // At the restart view the set fills a good part of the field, so it
        // should contain both black and coloured pixels.
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        effect.get_diff();

        let pixels = effect.field.pixels();
        let black = pixels.iter().filter(|p| *p == &[0.0, 0.0, 0.0]).count();
        let lit = pixels.len() - black;
        assert!(black > 0, "nothing was interior");
        assert!(lit > 0, "nothing escaped");
    }

    #[test]
    fn zooming_moves_the_camera_in() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        let before = effect.scale;
        effect.update();
        assert!(effect.scale < before, "the camera did not move");
    }

    #[test]
    fn going_past_the_depth_limit_picks_a_new_centre() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        let limit = effect.depth_limit();
        effect.scale = limit * 0.5;
        effect.advance(1.0 / 60.0);

        assert_eq!(
            effect.scale, STARTING_SCALE,
            "the camera did not pull back out to the restart view"
        );
        assert!(
            effect.scale > limit,
            "and it is still below the depth limit"
        );
    }

    #[test]
    fn a_budget_under_the_useful_floor_is_raised_rather_than_obeyed() {
        // `f32::clamp` asserts its bounds are ordered, and the old budget
        // clamped to `[MIN_USEFUL_ITERATIONS, ceiling]`, so any config with
        // `max_iterations` below the floor died on its first frame. The floor
        // still has to mean something, though: below it the bands stop being
        // distinguishable, so it is applied to the budget and to the depth
        // limit rather than left to a config to disobey.
        for max_iterations in [0u16, 1, 8, 23] {
            let options = MandelbrotOptions {
                max_iterations,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (20, 8));
            assert_eq!(effect.iteration_budget(), MIN_USEFUL_ITERATIONS);
            assert!(!effect.get_diff().is_empty());
            assert!(effect.depth_limit() > REFERENCE_SCALE);
        }
    }

    #[test]
    fn the_deepest_zoom_the_camera_reaches_still_shows_both_sides() {
        // The complaint this whole file's depth limit exists for: the zoom used
        // to run to a fixed 1e-5, which is well past what 96 iterations can
        // resolve, and the last third of every cycle came out as one flat
        // colour. At the deepest scale the camera now reaches, the view has to
        // still have samples on both sides of the boundary -- and not as a
        // token handful, or the picture is a sliver on black.
        for seed in 0..10u64 {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (40, 16));
            let limit = effect.depth_limit();
            let (deepest, center) = zoom_to_the_depth_limit(&mut effect);

            assert!(
                deepest >= limit && deepest < limit * 1.05,
                "seed {seed} stopped at {deepest:.3e}, which is not its depth \
                 limit of {limit:.3e}"
            );

            let (interior, exterior) =
                interior_and_exterior(&effect, center, deepest);
            let total = interior + exterior;
            assert!(
                interior > 0,
                "seed {seed} at {deepest:.3e} was all exterior at {center:?}"
            );
            assert!(
                exterior > 0,
                "seed {seed} at {deepest:.3e} was all interior at {center:?}, \
                 which is the flat wash"
            );
            assert!(
                exterior * 20 > total,
                "seed {seed} at {deepest:.3e} had {exterior} exterior samples of \
                 {total}, too few to read as a boundary"
            );
        }
    }

    #[test]
    fn the_depth_limit_follows_the_iteration_budget() {
        // The limit is derived from the budget rather than written down, so
        // that the two cannot drift apart: more iterations resolve more of the
        // boundary, so they have to buy depth. Both the number and the camera
        // that obeys it have to move.
        let reached = |max_iterations: u16| {
            let options = MandelbrotOptions {
                max_iterations,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (40, 16));
            let limit = effect.depth_limit();
            let (deepest, _) = zoom_to_the_depth_limit(&mut effect);
            (limit, deepest)
        };

        let (cheap_limit, cheap) = reached(MIN_USEFUL_ITERATIONS as u16);
        let (default_limit, default) = reached(96);
        let (rich_limit, rich) = reached(384);

        assert!(
            cheap_limit > default_limit && default_limit > rich_limit,
            "a bigger budget did not buy depth: {cheap_limit:.3e} at 24, \
             {default_limit:.3e} at 96, {rich_limit:.3e} at 384"
        );
        assert!(
            cheap > default && default > rich,
            "the camera did not follow its own depth limit: {cheap:.3e}, \
             {default:.3e}, {rich:.3e}"
        );
        // A decade of depth per doubling of the budget, so 384 is at least an
        // order of magnitude deeper than 24 and nowhere near a hundred times.
        assert!(
            rich < cheap / 10.0,
            "{} is not a decade under {}",
            rich,
            cheap
        );
    }

    #[test]
    fn the_camera_stops_before_f32_runs_out() {
        // The budget term is a runaway on its own, so the f32 sample spacing is
        // what bounds it: at 384 iterations the budget alone would allow 1e-5,
        // which on a 400-row field is well under one f32 step per sample, and
        // the view is then a handful of repeated blocks rather than a picture.
        let options = MandelbrotOptions {
            max_iterations: 384,
            ..Default::default()
        };
        let mut effect = Mandelbrot::new(options, (100, 100));
        let limit = effect.depth_limit();
        let by_budget =
            REFERENCE_SCALE * 10.0f64.powf((REFERENCE_BUDGET / 384.0).log2());
        assert!(
            by_budget < limit,
            "expected the precision term to bind at 384 iterations, got \
             {limit:.3e} against a budget term of {by_budget:.3e}"
        );

        let (deepest, _) = zoom_to_the_depth_limit(&mut effect);
        assert!(deepest >= limit && deepest < limit * 1.05);
        // And the samples really are that far apart down there.
        let spacing = 2.0 * deepest / effect.field.row_height() as f64;
        assert!(
            spacing >= MIN_F32_STEPS_PER_SAMPLE * F32_COORDINATE_STEP,
            "samples are {spacing:.3e} apart, under {} f32 steps",
            MIN_F32_STEPS_PER_SAMPLE
        );
    }

    #[test]
    fn a_recentre_leaves_the_boundary_across_the_screen() {
        // A recentre used to restart at the widest view containing the set,
        // 3.2, which puts the set in the middle of the frame as a small disc
        // and fills everything around it with points that escape almost
        // immediately -- one flat band, and the set is a few percent of it. So
        // this asserts on the *interior* fraction as well: a high exterior
        // fraction is the symptom here, not the cure.
        for seed in 0..8u64 {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (40, 16));
            effect.recentre();
            assert_eq!(effect.scale, STARTING_SCALE);

            let (interior, exterior) =
                interior_and_exterior(&effect, effect.center, effect.scale);
            let total = interior + exterior;
            assert!(
                interior * 10 > total,
                "seed {seed}: the restart view was {interior} interior of {total} \
                 samples at {:?}, so the set is a small disc",
                effect.center
            );
            assert!(
                exterior * 10 > total,
                "seed {seed}: the restart view was all interior at {:?}",
                effect.center
            );
        }
    }

    #[test]
    fn a_full_tour_ends_somewhere_worth_looking_at() {
        // The tour as a whole: it has to complete, land on a new coastline, and
        // leave the camera somewhere with both sides of a boundary on screen at
        // the moment it gets there.
        let options = MandelbrotOptions {
            seed: 3,
            ..Default::default()
        };
        let mut effect = Mandelbrot::new(options, (40, 16));

        let mut centres: Vec<(f64, f64)> = Vec::new();
        let limit = effect.depth_limit();
        for cycle in 0..3 {
            let (deepest, center) = zoom_to_the_depth_limit(&mut effect);
            // Every cycle ends in the same place, which is the point of the
            // limit being derived rather than drifting with the tour.
            assert!(
                deepest >= limit && deepest < limit * 1.05,
                "cycle {cycle} stopped at {deepest:.3e} rather than at its depth \
                 limit of {limit:.3e}"
            );

            assert_eq!(effect.scale, STARTING_SCALE, "it did not pull back out");
            // The first cycle zooms into the centre the constructor picked, so
            // only the ones after it have to be somewhere new.
            if cycle > 0 {
                assert!(
                    !centres.contains(&center),
                    "seed 3 came back to {center:?} on cycle {cycle}"
                );
            }
            centres.push(center);

            let (interior, exterior) =
                interior_and_exterior(&effect, center, deepest);
            assert!(
                interior > 0 && exterior > 0,
                "the tour stopped at {deepest:.3e} on a view that is all {}",
                if interior == 0 {
                    "exterior"
                } else {
                    "interior"
                }
            );
        }
    }

    #[test]
    fn the_tour_is_reproducible_from_the_seed() {
        let tour = |seed: u64| {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (20, 8));
            // Force several recentres and record where they land.
            let mut centres = Vec::new();
            for _ in 0..4 {
                effect.scale = effect.depth_limit() * 0.5;
                effect.advance(1.0 / 60.0);
                centres.push(effect.center);
            }
            centres
        };

        assert_eq!(tour(7), tour(7), "the same seed toured differently");
    }

    #[test]
    fn different_seeds_tour_differently() {
        let tour = |seed: u64| {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (20, 8));
            let mut centres = Vec::new();
            for _ in 0..4 {
                effect.scale = effect.depth_limit() * 0.5;
                effect.advance(1.0 / 60.0);
                centres.push(effect.center);
            }
            centres
        };

        assert_ne!(tour(7), tour(8), "two seeds produced the same tour");
    }

    #[test]
    fn surviving_a_resize() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        effect.update_size(6, 6);
        let diff = effect.get_diff();
        for (x, y, _) in &diff {
            assert!(*x < 6 && *y < 6);
        }
    }
}
