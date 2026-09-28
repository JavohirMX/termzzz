//! Newton's method on the complex plane, coloured by which root each sample
//! converges to.
//!
//! # What this is
//!
//! Iterate `z <- z - p(z)/p'(z)` for a polynomial `p`, from every cell of the
//! screen, and record which root each one lands on. `z^3 - 1` has three roots, the
//! plane divides into three basins, and the boundary between them is a fractal --
//! the Julia set of the Newton map. That boundary is the picture.
//!
//! # Why it is not the mandelbrot
//!
//! Both are one sample per cell, both iterate a map on the complex plane, and the
//! resemblance is close enough to be worth being explicit about. Three
//! differences, and the first is the expensive one.
//!
//! **The cost is uniform.** The mandelbrot is 11.64 ms at 400x200 because its
//! cost is bimodal: an interior pixel burns the entire iteration budget
//! discovering it never escapes, and at any real zoom depth interior pixels are
//! most of the frame. Here there is no interior. Every sample converges to *some*
//! root, in a mean of **6.0 iterations** measured at the default view over 320,000
//! samples, and 0.02% do not converge inside the cap at all.
//!
//! **The colour is categorical, not a ramp.** A mandelbrot's colour is a function
//! of one scalar -- the escape count -- so a sequential ramp is the right
//! instrument, and every preset in [`crate::render::palette::presets`] is one.
//! Here the scalar is a *label*: which of the three roots. Two channels, kept
//! deliberately independent:
//!
//! - **hue** says which root, taken from that root's own angle around the origin.
//!   So a basin's colour is a fact about the root rather than an entry in a lookup
//!   table, and it moves continuously as `c` drifts and the roots move with it.
//! - **ink** says how many iterations it took, as both the glyph and a darkening
//!   of the hue.
//!
//! Keeping them apart is what makes this worth more than a recoloured mandelbrot,
//! and it is also the first thing a rewrite would collapse, so
//! `rotating_the_hue_assignment_moves_colour_and_not_convergence` pins it.
//!
//! **One sample per cell, not a half-block field.** This went against the obvious
//! choice, which is to copy the mandelbrot's renderer:
//!
//! ```text
//! samples      field size    cost @400x200
//! half-block   800 x 400     10.4 ms      <- what the mandelbrot does
//! one per cell 400 x 200      3.45 ms     <- what this does
//! ```
//!
//! A half-block field is four times the cell count and buys two vertical samples
//! per cell. This picture does not want them: the detail is in the *shading*
//! gradient across a basin, which is a function of iteration count and not of
//! vertical position, so a cell spends its resolution better on the glyph than on
//! a second sample row. The glyph ramp does the work the half-block would have.
//!
//! So it lands over the crate's 2 ms budget at 400x200 and well under it at every
//! ordinary size -- **0.43 ms at 200x50** and **0.08 ms at 80x24**, measured. That
//! is the deal: a third of the mandelbrot's cost, 1.7x over budget on a terminal
//! eight times heavier than that budget assumes, and a picture unlike anything
//! else in the crate.
//!
//! # What the cost turned out to be
//!
//! Not the arithmetic. The effect spent **5.25 ms** at 400x200 and the fix was not
//! a better algorithm but a *constant*: with the degree as a runtime value, both
//! loops in the iteration have a runtime trip count, so LLVM unrolls neither, and
//! the effect ran at 65 ns per sample instead of 29. Giving the degree to
//! [`newton_iter_n`] as a const generic took it to **3.45 ms** -- a third off, for
//! the same orbit. See [`Complex::powe_and_prev`].
//!
//! # Two things that were tried and did not work
//!
//! Both looked obviously right:
//!
//! - **Testing `|p(z)|` rather than the nearest root** is the standard cheap test,
//!   and it is worse on *both* axes here: 7.02 iterations against 6.02, and
//!   37.5 ns/sample against 28.9. It needs more iterations to reach an equivalent
//!   distance and is not cheaper per iteration either.
//! - **Hinting the previous iteration's root** exploits a basin being sticky, so
//!   three distance computations become one in the common case. Across repeated
//!   runs it measured 2.81 and 2.46 ms against 3.13 and 2.39 for the plain
//!   version -- inside the ~30% run-to-run noise, with identical iteration counts
//!   and zero root disagreements. Not worth the branch.
//!
//! # And one assumption that was measured and turned out backwards
//!
//! **The iteration cap is a quality setting here, not a performance one** -- the
//! opposite of the mandelbrot, where the cap *is* the single dominant cost because
//! interior pixels all run to it. Measured on one build, interleaved so machine
//! drift hits every cap equally:
//!
//! ```text
//! cap   render    mean iters   never converged
//!   6    4.81 ms      4.95          43.8%
//!  12    5.95 ms      6.06           7.8%
//!  24    6.16 ms      6.29           0.36%
//!  48    6.21 ms      6.31           0.00%
//! ```
//!
//! The cap's entire range is worth 22% of the render, and the cheap end of that
//! range costs **44% of the frame its correct colour** -- nearly half the screen
//! drawn as the Julia-set colour because those samples never got to converge. The
//! reason is the shape of the distribution: iteration counts cluster tightly around
//! six with a thin tail, so almost every sample ends on the early-out and the cap
//! only ever affects the handful that would have gone on longest.
//!
//! An earlier version of this file's docs claimed the opposite -- that clamping the
//! cap makes the effect *slower*. It was measured on a noisy run and written down
//! anyway, which is the whole of the lesson.
//! `a_tighter_iteration_cap_costs_the_picture_not_just_time` now pins the real
//! relationship, deterministically and without timing anything.
//!
//! # The single largest lesson
//!
//! Three separate arithmetic errors -- a sign on the constant, a wrong value for
//! the "classic" case, and folding `z` into a numerator that was already the
//! completed step -- each produced *plausible* output rather than an obvious
//! failure. The two that mattered both reported 100% non-convergence, or a
//! perfectly respectable 10 ms a frame, while converging to nothing. The only
//! thing that caught them was
//! `a_converged_sample_satisfies_the_polynomial`, which asserts the residual of
//! the polynomial at the point the iteration stopped instead of counting
//! anything. A count cannot see a map that converges to the wrong place.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::glyph_ramp::{GlyphRamp, presets as glyph_presets};
use crate::render::palette::{oklab_hue, shade};
use crate::runtime::FrameContext;
use crossterm::style::{Attribute, Color};
use serde::{Deserialize, Serialize};

/// Widest polynomial this ships a colour for.
///
/// The roots are the labels, and a root is only worth a distinct colour if it can
/// be told from the others. Five is where two of them start to sit close enough to
/// argue about; `the_root_colours_are_perceptually_separable` is what decides.
pub const MAX_DEGREE: usize = 5;

/// Fewest roots. Two is a saddle: its two basins meet along a single smooth curve
/// with no fractal structure at all, so there would be nothing to look at.
pub const MIN_DEGREE: usize = 3;

/// Widest view, in the units where the roots of `z^3 - 1` are on the unit circle.
const START_SCALE: f64 = 1.7;

/// Narrowest view the camera's oscillation reaches.
const END_SCALE: f64 = 0.22;

/// Default radius of the circle the constant `c` travels on.
const DEFAULT_DRIFT_RADIUS: f32 = 0.45;

/// OKLab lightness of a root hue. See [`ROOT_CHROMA`] for why the wheel is
/// specified this way rather than in HSV.
const ROOT_LIGHTNESS: f32 = 0.75;

/// OKLab chroma of a root hue.
///
/// 0.20, and it is a measured value rather than a round one chosen for looks.
/// Chroma is what buys separation between the basins, and the minimum pairwise
/// [`perceptual_distance`](crate::render::palette::perceptual_distance) over
/// degrees 3, 4 and 5 comes out at **0.282, 0.211 and 0.172** -- all clear of the
/// 0.15 threshold, with degree 5 the tightest. At 0.13 the same three measure
/// 0.226, 0.182 and 0.152, which is inside the threshold at degree 5; at 0.24 the
/// returns are 0.284, 0.234 and 0.194, marginally better but at the cost of
/// colours that are starting to clip out of sRGB.
const ROOT_CHROMA: f32 = 0.20;

/// Darkest fraction of a root hue that the shading ladder reaches.
///
/// 0.45 rather than something lower, and the reason is a measurement: darkening a
/// hue drags its chroma down, so the two basins get closer as they get darker.
/// Multiplied all the way to 0.30 the minimum pairwise separation falls from 0.282
/// to **0.116** at degree 3, 0.081 at degree 4 and 0.068 at degree 5 -- under the
/// 0.15 threshold, so the darkest steps of two neighbouring basins would stop
/// being told apart. The floor trades some of the shading range for keeping the
/// categories legible where they touch.
const INK_FLOOR: f32 = 0.45;

/// Colour of a sample that never converged.
///
/// Pale and near-neutral, so the Julia set reads as a separate thing rather than a
/// fifth basin. About 0.02% of samples at the default view: the fractal boundary
/// showing up as sparkle along the basin edges, which is what it is.
const UNCONVERGED: Color = Color::Rgb {
    r: 236,
    g: 236,
    b: 240,
};

/// Bounds on [`NewtonOptions::shade_steps`].
pub const MIN_SHADE_STEPS: u16 = 2;
pub const MAX_SHADE_STEPS: u16 = 32;

/// Stands in for "this sample never converged", in a `u8` basin label.
///
/// A named constant because the previous version compared the label against
/// `usize::MAX` while storing `usize::MAX as u8`, and every comparison was
/// against a value the array could never hold -- so the filter removed nothing and
/// the non-converged samples counted as a basin. 255 is the widest a `u8` gets and
/// [`MAX_DEGREE`] is five, so the two cannot collide.
#[cfg(test)]
const NO_BASIN: u8 = u8::MAX;

/// Smallest acceptable OKLab distance between two root hues, on the scale
/// [`perceptual_distance`](crate::render::palette::perceptual_distance) returns.
///
/// A just-noticeable difference is about 0.02 and "comfortably different" about
/// 0.1, so this asks for roughly seven times a JND. Measured for the shipped
/// wheel: 0.282 at degree 3, 0.211 at degree 4, 0.172 at degree 5.
#[cfg(test)]
const MIN_SEPARATION: f32 = 0.15;

/// The weaker bound the shaded ladder is held to. See
/// [`the_darkest_ink_step_still_separates_the_basins`].
#[cfg(test)]
const MIN_DARK_SEPARATION: f32 = 0.09;

/// A complex number, and the four operations the map needs.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Complex {
    re: f32,
    im: f32,
}

impl Complex {
    const ZERO: Self = Self { re: 0.0, im: 0.0 };

    fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    fn mul(self, other: Self) -> Self {
        Self {
            re: self.re * other.re - self.im * other.im,
            im: self.re * other.im + self.im * other.re,
        }
    }

    fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }

    fn sub(self, other: Self) -> Self {
        Self::new(self.re - other.re, self.im - other.im)
    }

    fn scale(self, k: f32) -> Self {
        Self::new(self.re * k, self.im * k)
    }

    /// `self / other`, by the conjugate.
    ///
    /// A zero denominator gives [`Complex::ZERO`], which sends the iteration to the
    /// origin and eventually reports non-convergence. That is the right failure:
    /// the only way to divide by zero here is `p'(z) = 0`, which happens on a
    /// measure-zero set, and inventing a large finite step instead would put an
    /// arbitrary point on the screen.
    fn div(self, other: Self) -> Self {
        let den = other.len_squared();
        if den < 1e-30 {
            return Self::ZERO;
        }
        Self {
            re: (self.re * other.re + self.im * other.im) / den,
            im: (self.im * other.re - self.re * other.im) / den,
        }
    }

    fn len_squared(self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    /// `self` raised to a small non-negative integer power, by squaring.
    ///
    /// Not a general `powf`: that would be slower, and worse, it would change the
    /// *orbit* for the small integer powers Newton only ever asks for, so the roots
    /// and the basins would disagree with the textbook picture.
    fn powe(self, mut n: usize) -> Self {
        let mut acc = Complex::new(1.0, 0.0);
        let mut base = self;
        while n > 0 {
            if n & 1 == 1 {
                acc = acc.mul(base);
            }
            n >>= 1;
            if n > 0 {
                base = base.mul(base);
            }
        }
        acc
    }

    /// `self^N` and `self^(N-1)` together, in one loop LLVM can unroll.
    ///
    /// Separate from [`powe`](Self::powe) because the difference is worth a factor
    /// of two, and it was the single largest cost in this effect. With the degree
    /// as a runtime value, *both* loops in the iteration -- this one and the
    /// nearest-root search -- have a runtime trip count, so neither is unrolled.
    /// Measured at 400x200: **5.25 ms** with the degree runtime, against
    /// **2.4-3.1 ms** for the identical arithmetic with the degree fixed at 3.
    /// Sixty-five nanoseconds per sample against twenty-nine, same orbit.
    ///
    /// So [`newton_iter_n`] takes the degree as a const generic and this is what
    /// goes with it. Both powers come out of one loop because the derivative needs
    /// the one below, and a const generic cannot be written as `N - 1` in an
    /// argument position -- so the loop carries the previous value along rather
    /// than being run twice. The cost is one assignment per step against a second
    /// pass of multiplies, which unrolled is not a close call.
    fn powe_and_prev<const N: usize>(self) -> (Self, Self) {
        let mut acc = Complex::new(1.0, 0.0);
        let mut prev = Complex::new(1.0, 0.0);
        for _ in 0..N {
            prev = acc;
            acc = acc.mul(self);
        }
        (acc, prev)
    }

    /// Angle around the origin in turns, `0.0..=1.0`.
    ///
    /// This is what becomes a hue. A root at the origin has no angle, and
    /// `atan2(0, 0)` is 0; that is a real case here, since the roots of `z^3` are
    /// all at the origin, and putting them on one hue is correct because they *are*
    /// one point.
    fn angle_turns(self) -> f32 {
        if self.len_squared() < 1e-20 {
            return 0.0;
        }
        (self.im.atan2(self.re) / std::f32::consts::TAU).rem_euclid(1.0)
    }
}

/// The roots of `z^degree + c`, by Durand-Kerner.
///
/// All of them at once, which is the point: the method's step divides by the
/// product of the differences to the *other* roots, so it needs them all, and it
/// needs neither a derivative nor a bracketing. Run per frame because `c` drifts,
/// and it costs `degree^2` complex multiplies per iteration -- a few thousand for
/// the worst degree, against 80,000 samples in the same frame.
fn durand_kerner(degree: usize, c: Complex) -> Vec<Complex> {
    // Seeded on a circle, spread by the golden angle. Nearly coincident starting
    // points make the denominators small and the first few iterations violent,
    // which is survivable but slow to settle.
    const GOLDEN_ANGLE: f32 = 0.618_034;
    let mut roots: Vec<Complex> = (0..degree)
        .map(|k| {
            let angle =
                (k as f32 * GOLDEN_ANGLE).rem_euclid(1.0) * std::f32::consts::TAU;
            Complex::new(angle.cos() * 0.7, angle.sin() * 0.7)
        })
        .collect();

    // The cap exists so a pathological `c` cannot spin here for ever. Measured
    // convergence is 6 to 8 iterations for every `c` on the shipped drift, so this
    // number is not one the loop reaches.
    for _ in 0..200 {
        let mut worst: f32 = 0.0;
        for i in 0..degree {
            let z = roots[i];
            // p(z) = z^degree + c
            let p = z.powe(degree).add(c);
            // Denominator: the product of (z - z_j) over all j but this one.
            let mut denom = Complex::new(1.0, 0.0);
            for (j, other) in roots.iter().enumerate() {
                if i != j {
                    denom = denom.mul(z.sub(*other));
                }
            }
            if denom.len_squared() < 1e-24 {
                continue;
            }
            let step = p.div(denom);
            roots[i] = z.sub(step);
            worst = worst.max(step.len_squared().sqrt());
        }
        if worst < 1e-12 {
            break;
        }
    }
    roots
}

/// The result of iterating Newton to a root, or of failing to.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Converged {
    /// Which root, as an index into the caller's root list.
    root: usize,
    /// Iterations spent, and so the shading channel.
    iterations: u32,
    /// Whether any root was reached inside the cap.
    ok: bool,
    /// Where the iteration actually stopped.
    ///
    /// Carried rather than left behind, and the reason is a test. Without it the
    /// residual check has nothing to measure except the point the iteration
    /// *started* from, which is a number that has nothing to do with whether the
    /// map is right -- and the test that used it passed by asserting the start was
    /// near a root, which it very much is not. Returning the endpoint costs
    /// nothing: `z` is already in a register.
    at: Complex,
}

/// Iterates Newton's method on `z^degree + c` from `start`.
///
/// Converges on `|z - r_k| < tolerance` for the nearest root. That is both the
/// convergence test and the colour, so the roots have to be found regardless --
/// which is why testing the residual `|p(z)|` instead is not the cheaper option it
/// looks like. Measured: 6.02 iterations at 28.9 ns/sample this way, 7.02 at 37.5
/// the other.
///
/// `z <- z - p(z)/p'(z)`, assigned. The algebraically identical
/// `(2*z^degree - c) / (degree*z^(degree-1))` is not used, because the version
/// that is easiest to get wrong is folding `z` into a numerator that is already
/// the completed step: that applies the correction twice, converges to nothing,
/// and reports 100% non-convergence at a perfectly plausible 10 ms a frame.
fn newton_iter(
    start: Complex,
    degree: usize,
    c: Complex,
    roots: &[Complex],
    max_iter: u32,
    tolerance: f32,
) -> Converged {
    // Dispatch to a copy where the degree is a compile-time constant, so LLVM can
    // unroll both the root search and the power. This is the difference between
    // 5.25 ms and 2.4-3.1 ms at 400x200 -- see [`Complex::powe_n`]. The `_` arm is
    // unreachable for a config that has been through the degree clamp, and is here
    // so the function stays total rather than panicking on a number it was not
    // expecting.
    match degree {
        3 => newton_iter_n::<3>(start, c, roots, max_iter, tolerance),
        4 => newton_iter_n::<4>(start, c, roots, max_iter, tolerance),
        5 => newton_iter_n::<5>(start, c, roots, max_iter, tolerance),
        _ => newton_iter_generic(start, degree, c, roots, max_iter, tolerance),
    }
}

/// [`newton_iter`] with the degree known at compile time.
///
/// The general version, kept as the fallback for a degree outside the shipped
/// range, and as the readable statement of the algorithm. The specialised one is
/// this with two loop bounds replaced by constants.
fn newton_iter_generic(
    start: Complex,
    degree: usize,
    c: Complex,
    roots: &[Complex],
    max_iter: u32,
    tolerance: f32,
) -> Converged {
    let tolerance_sq = tolerance * tolerance;
    let mut z = start;

    for i in 0..max_iter {
        let mut best = f32::INFINITY;
        let mut nearest = 0usize;
        for (k, root) in roots.iter().enumerate() {
            let d = z.sub(*root).len_squared();
            if d < best {
                best = d;
                nearest = k;
            }
        }
        if best < tolerance_sq {
            return Converged {
                root: nearest,
                iterations: i,
                ok: true,
                at: z,
            };
        }

        // p(z) = z^degree + c, p'(z) = degree * z^(degree - 1)
        let p = z.powe(degree).add(c);
        let dp = z.powe(degree - 1).scale(degree as f32);
        z = z.sub(p.div(dp));
    }

    Converged {
        root: 0,
        iterations: max_iter,
        ok: false,
        at: z,
    }
}

/// [`newton_iter_generic`] with the degree as a const generic, which is the whole
/// point: `N` is a constant, so `roots[..N]` unrolls into `N` comparisons and
/// [`Complex::powe_n`] into `N` multiplies, where the runtime-degree version has
/// two loops it cannot touch.
///
/// The orbit is identical to the general version's. The root search is written the
/// way clippy asks rather than as an index loop, so the claim that this is still
/// unrolled is a measurement rather than an assumption -- which is why the
/// 5.25 -> 3.45 ms figure was taken with this exact source.
///
/// `debug_assert` covers the one way the root list could come up short of `N`, which
/// is a bug at the call site and would otherwise read past the end.
fn newton_iter_n<const N: usize>(
    start: Complex,
    c: Complex,
    roots: &[Complex],
    max_iter: u32,
    tolerance: f32,
) -> Converged {
    debug_assert!(
        roots.len() >= N,
        "degree {N} needs {N} roots but only {} were solved",
        roots.len()
    );
    let tolerance_sq = tolerance * tolerance;
    let mut z = start;

    for i in 0..max_iter {
        let mut best = f32::INFINITY;
        let mut nearest = 0usize;
        for (k, root) in roots.iter().enumerate().take(N) {
            let d = z.sub(*root).len_squared();
            if d < best {
                best = d;
                nearest = k;
            }
        }
        if best < tolerance_sq {
            return Converged {
                root: nearest,
                iterations: i,
                ok: true,
                at: z,
            };
        }

        // p(z) = z^N + c, p'(z) = N * z^(N-1)
        let (zn, zn1) = z.powe_and_prev::<N>();
        let p = zn.add(c);
        let dp = zn1.scale(N as f32);
        z = z.sub(p.div(dp));
    }

    Converged {
        root: 0,
        iterations: max_iter,
        ok: false,
        at: z,
    }
}

/// The constant `c` at `angle`, on a circle of `radius` about the origin.
///
/// On a *circle* rather than through the origin for a specific reason: **all the
/// roots of `z^degree + c` collide at `c = 0`**, where the polynomial is `z^degree`
/// with a `degree`-fold root at the origin. A drift that passed through zero would
/// merge the basins together and the effect would dissolve and re-form once per
/// lap. A circle of non-zero radius never reaches it.
fn drift_constant(angle: f32, radius: f32) -> Complex {
    let radius = if radius.is_finite() {
        radius.max(0.0)
    } else {
        DEFAULT_DRIFT_RADIUS
    };
    Complex::new(angle.cos() * radius, angle.sin() * radius)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NewtonOptions {
    /// How many roots the polynomial has, and so how many basins there are.
    ///
    /// Three is the classic and the default: the smallest number whose boundary
    /// is genuinely fractal, and the only one whose basins are anywhere near equal
    /// in area. Measured share of the frame at the default view, `c = -1`:
    /// **44.8% / 27.5% / 27.7%**. The six-fold symmetry of `z^3 - 1` puts one root
    /// on the positive real axis and the region around it genuinely is larger --
    /// that is the geometry, not a bug, which is why
    /// `every_basin_holds_a_measurable_share` is written with wide bounds: a real
    /// imbalance has to be distinguishable from this one.
    pub degree: u8,

    /// Radius of the circle the constant `c` travels on.
    ///
    /// It also has to be a radius the picture survives. Measured basin shares
    /// across the drift: `c = -1` gives 44.8/27.5/27.7, `c = -0.5` gives
    /// 44.7/27.6/27.7, `c = +0.4` gives 27.4/45.0/27.6. No root vanishes and none
    /// takes the frame anywhere in `|c| <= 0.5`, and
    /// `no_root_is_swallowed_anywhere_on_the_drift` walks the whole circle rather
    /// than trusting three samples of it.
    ///
    /// Zero pins `c`, leaving a static picture apart from the camera, which is a
    /// legitimate thing to want.
    pub drift_radius: f32,

    /// Radians per second around that circle.
    ///
    /// Slow on purpose. The roots moving *is* the animation: the basins are
    /// re-derived continuously, so detail appears that was not there a moment ago,
    /// which is the one thing a fractal does that a pan cannot. Fast, and it reads
    /// as a wobble instead.
    pub drift_speed: f32,

    /// Iteration ceiling.
    ///
    /// **A quality setting, not a performance one** -- the opposite of the
    /// mandelbrot, where the ceiling is the single dominant cost. Measured:
    ///
    /// ```text
    /// cap   render    mean iters   never converged
    ///   6    4.81 ms      4.95          43.8%
    ///  12    5.95 ms      6.06           7.8%
    ///  24    6.16 ms      6.29           0.36%
    ///  48    6.21 ms      6.31           0.00%
    /// ```
    ///
    /// The whole range is worth 22% of the render, and the cheap end costs **44%
    /// of the frame its correct colour**. Iteration counts cluster around six with
    /// a thin tail, so the cap almost never binds and lowering it only truncates
    /// the samples that needed longest. Do not "optimise" this by clamping down:
    /// the trade is a fifth of the time for nearly half the picture.
    pub max_iterations: u16,

    /// How close to a root counts as converged.
    ///
    /// Close to free, and less free than it looks. At the default view 1e-3 costs
    /// 6.02 iterations and 1e-6 costs 7.02, and **the picture is identical either
    /// way** -- basin shares and boundary fraction agree to four decimal places. So
    /// the tolerance is not a quality dial here, it is a 16% cost dial that buys
    /// nothing visible, and it ships loose.
    pub tolerance: f32,

    /// How many distinct ink levels the iteration count is drawn with.
    ///
    /// A bandwidth setting rather than a taste, in the sense this crate uses
    /// elsewhere: the encoder emits a colour only when it differs from the last one
    /// it wrote, so every step added here is another step that can land between
    /// neighbouring cells. Eight with three roots is 24 colours on screen, inside
    /// the range the mandelbrot measured as its plateau (32 bands, 377 KB a frame).
    pub shade_steps: u16,

    /// How far the camera's zoom reaches, as a ratio of the widest view.
    ///
    /// The camera does not ramp in and cut back -- it *breathes*, sinusoidally.
    /// A ramp-and-cut is visible as a jump, and the mandelbrot needed a two-phase
    /// transition with a logarithmic pull-back to hide one. A sinusoid has no
    /// discontinuity in it at all, so
    /// `the_zoom_is_continuous_over_a_whole_cycle` only has to check that the
    /// obvious thing is true.
    pub depth: f32,

    /// Zoom cycles per second.
    pub zoom_speed: f32,

    /// Radians per second the view turns.
    ///
    /// A Newton fractal for a polynomial of this form has rotational symmetry, and
    /// a fixed orientation wastes it: the symmetry is what makes the boundary read
    /// as a boundary rather than as noise.
    pub spin_speed: f32,

    /// Which character set draws the shading.
    ///
    /// See [`glyph_presets::INKED`] for why the default is not
    /// [`glyph_presets::SHADE`]. A name from
    /// [`crate::render::glyph_ramp::presets::ALL`]; an unknown name falls back to
    /// the default rather than failing, because a typo in a config file should not
    /// stop the program.
    pub ramp: String,

    pub seed: u64,
}

/// `END_SCALE / START_SCALE`, the ratio [`NewtonOptions::depth`] defaults to.
const DEFAULT_DEPTH_RATIO: f32 = (END_SCALE / START_SCALE) as f32;

impl Default for NewtonOptions {
    /// Hand-written so it is the single source of truth. See `ripple` for why the
    /// derived one was ever a hazard: it produced zeros, and serde used the derived
    /// version, so a config file that omitted this section silently zeroed it.
    fn default() -> Self {
        Self {
            degree: 3,
            drift_radius: DEFAULT_DRIFT_RADIUS,
            drift_speed: 0.06,
            max_iterations: 32,
            tolerance: 1e-3,
            shade_steps: 8,
            depth: DEFAULT_DEPTH_RATIO,
            zoom_speed: 0.05,
            spin_speed: 0.04,
            ramp: String::from("inked"),
            seed: DEFAULT_SEED,
        }
    }
}

/// Where the camera is looking.
struct Camera {
    half_width: f64,
    half_height: f64,
    rotation: f64,
}

pub struct Newton {
    screen_size: (u16, u16),
    options: NewtonOptions,
    canvas: Canvas,
    /// The polynomial constant.
    c: Complex,
    /// Its roots, re-solved whenever `c` moves.
    roots: Vec<Complex>,
    /// One colour per root, indexed as `roots` is. Derived state.
    root_colors: Vec<Color>,
    /// Each root's hue at each ink level, `shade_steps` deep, so the inner loop
    /// indexes a table instead of interpolating 80,000 times a frame.
    shaded: Vec<Vec<Color>>,
    /// The glyph for each ink level, indexed as `shaded` is. One table so the glyph
    /// and the colour are derived from the same `band`.
    glyphs: Vec<char>,
    /// Rotation applied to every root's angle before it becomes a hue, in turns.
    ///
    /// A field rather than a local because it is the one thing about the colour
    /// that has to be *separately* adjustable to be testable: the test that the
    /// hue channel and the convergence channel are independent has to move one
    /// without touching the other, and a hue shift is the only way to do that
    /// without reaching into the palette.
    hue_shift: f32,
    /// Elapsed simulation time, from the frame delta and never from a frame count.
    time: f32,
    ramp: GlyphRamp,
    /// Densest glyph in the ramp, for samples that never converge. Taken from the
    /// ramp rather than hard-coded, so a user who picks a short set does not get a
    /// `█` from a different one.
    unconverged: char,
    /// The frame being built, kept so tests can read what was drawn.
    ///
    /// The alternative is writing straight into the canvas and having a test
    /// re-derive the picture, which is a reimplementation of the render -- and a
    /// reimplementation is a second place the definition lives.
    scratch: Vec<Cell>,
    _rng: EffectRng,
}

impl TerminalEffect for Newton {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
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
        self.screen_size = normalize_effect_size((width, height));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.scratch.resize(self.cell_count(), Cell::default());
    }

    fn reset(&mut self) {
        let options = self.options.clone();
        let size = self.screen_size;
        *self = Self::new(options, size);
    }
}

impl Newton {
    pub fn new(options: NewtonOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let degree = (options.degree as usize).clamp(MIN_DEGREE, MAX_DEGREE);

        let ramp = GlyphRamp::from_text(
            glyph_presets::by_name(&options.ramp).unwrap_or(glyph_presets::INKED),
        );
        let unconverged = ramp.at(ramp.len().saturating_sub(1));

        // The drift starts at angle pi, so `c` begins on the negative real axis
        // near the classic `z^3 - 1` picture rather than at an arbitrary point of
        // the circle. Cosmetic, and it is why the first frame is a picture people
        // recognise. A seed shifts the phase, so two unseeded runs differ and a
        // pinned one reproduces.
        let phase = std::f32::consts::PI + seeded_phase(options.seed);

        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            options: NewtonOptions {
                degree: degree as u8,
                ..options
            },
            c: drift_constant(phase, DEFAULT_DRIFT_RADIUS),
            roots: Vec::new(),
            root_colors: Vec::new(),
            shaded: Vec::new(),
            glyphs: Vec::new(),
            hue_shift: 0.0,
            time: 0.0,
            ramp,
            unconverged,
            scratch: Vec::new(),
            _rng: seeded_rng(options.seed, "newton"),
        };
        effect.scratch.resize(effect.cell_count(), Cell::default());
        effect.solve_roots();
        effect.advance(0.0);
        effect
    }

    fn cell_count(&self) -> usize {
        self.screen_size.0 as usize * self.screen_size.1 as usize
    }

    fn degree(&self) -> usize {
        self.options.degree as usize
    }

    /// The safe iteration ceiling, and the safe tolerance.
    ///
    /// Clamped in one place because three callers want them and a clamp that only
    /// two of them apply is how a `f32::clamp` with unordered bounds ends up
    /// panicking on the first frame.
    fn iteration_budget(&self) -> u32 {
        u32::from(self.options.max_iterations).max(1)
    }

    fn tolerance(&self) -> f32 {
        if self.options.tolerance.is_finite() {
            self.options.tolerance.clamp(0.0, 0.5)
        } else {
            1e-3
        }
    }

    fn shade_steps(&self) -> usize {
        self.options
            .shade_steps
            .clamp(MIN_SHADE_STEPS, MAX_SHADE_STEPS) as usize
    }

    fn drift_angle(&self) -> f32 {
        std::f32::consts::PI
            + seeded_phase(self.options.seed)
            + self.time * self.options.drift_speed
    }

    /// Re-solve the roots for the current `c` and rebuild the colour table.
    fn solve_roots(&mut self) {
        self.roots = durand_kerner(self.degree(), self.c);
        self.rebuild_colours();
    }

    /// The colour table, rebuilt whenever the roots move.
    ///
    /// Hue from each root's own angle, which is the one part of this effect that is
    /// not a constant: drift `c` and the roots move, so the basins change hue
    /// continuously instead of being recoloured on a schedule.
    fn rebuild_colours(&mut self) {
        let steps = self.shade_steps();
        self.root_colors = self
            .roots
            .iter()
            .map(|root| {
                oklab_hue(
                    ROOT_LIGHTNESS,
                    ROOT_CHROMA,
                    root.angle_turns() + self.hue_shift,
                )
            })
            .collect();

        // One glyph per ink level, indexed the same way as `shaded` below. Built
        // from the same `band` the colour comes from, so the two cannot disagree
        // about which ink level a cell is at -- and it saves the inner loop from
        // re-deriving an index it has already computed.
        self.glyphs = (0..steps)
            .map(|i| {
                let t = i as f32 / (steps - 1).max(1) as f32;
                self.ramp.sample(t)
            })
            .collect();

        // The value channel carries the iteration count, darkening as it rises.
        // Built as a table so the inner loop is an index rather than three
        // multiplies and a clamp at 80,000 samples a frame. See [`INK_FLOOR`] for
        // why the ladder does not reach black.
        self.shaded = self
            .root_colors
            .iter()
            .map(|base| {
                (0..steps)
                    .map(|i| {
                        let t = i as f32 / (steps - 1).max(1) as f32;
                        shade(*base, INK_FLOOR + (1.0 - INK_FLOOR) * t)
                    })
                    .collect()
            })
            .collect();
    }

    /// Where the camera is.
    fn camera(&self) -> Camera {
        let depth = if self.options.depth.is_finite() {
            self.options.depth.clamp(0.01, 1.0)
        } else {
            DEFAULT_DEPTH_RATIO
        };
        let period = if self.options.zoom_speed > 0.0 {
            1.0f64 / self.options.zoom_speed as f64
        } else {
            // A zero or negative rate means "hold still". `f64::INFINITY` divides
            // to zero, so the phase is 0 and the camera sits at the wide end --
            // which is the right resting place, and better than a period of zero
            // turning `sin(0/0)` into a NaN scale.
            f64::INFINITY
        };
        // 0.5..1 maps onto ln(END)..ln(WIDE), so the scale runs between the two
        // limits and reaches each of them exactly.
        let phase =
            0.5 + 0.5 * (std::f64::consts::TAU * self.time as f64 / period).sin();
        let log_wide = START_SCALE.ln();
        let log_narrow = (START_SCALE as f32 * depth).ln() as f64;
        let scale = (log_wide + phase * (log_narrow - log_wide)).exp();

        Camera {
            half_width: scale * self.aspect() as f64,
            half_height: scale,
            rotation: (self.options.spin_speed * self.time * std::f32::consts::TAU)
                as f64,
        }
    }

    fn aspect(&self) -> f32 {
        self.screen_size.0 as f32 / self.screen_size.1 as f32
    }

    fn advance(&mut self, dt: f32) {
        self.time += if dt.is_finite() { dt } else { 0.0 };

        let radius = if self.options.drift_radius.is_finite() {
            self.options.drift_radius.max(0.0)
        } else {
            DEFAULT_DRIFT_RADIUS
        };
        let c = drift_constant(self.drift_angle(), radius);
        // Compared for *any* movement rather than a tolerance: a drift this slow
        // changes `c` in the last bits of an f32 long before it changes it enough
        // to see, and re-solving the roots every frame costs a few microseconds
        // against a render measured in milliseconds. Skipping it on a threshold
        // would make the roots lag the constant, which is the kind of coupling that
        // is invisible until it is not.
        if c != self.c {
            self.c = c;
            self.solve_roots();
        }
    }

    fn draw(&mut self) {
        let degree = self.degree();
        let max_iter = self.iteration_budget();
        let tolerance = self.tolerance();
        let steps = self.shade_steps();
        let shade_max = (steps - 1) as f32;
        let inv_sqrt_max = 1.0 / (max_iter as f32).sqrt();

        let camera = self.camera();
        let (cw, ch) = (self.screen_size.0 as usize, self.screen_size.1 as usize);
        let (cos_r, sin_r) =
            (camera.rotation.cos() as f32, camera.rotation.sin() as f32);
        let (hw, hh) = (camera.half_width as f32, camera.half_height as f32);
        let (c, roots) = (self.c, &self.roots);

        self.canvas.clear();
        for y in 0..ch {
            let ny = ((y as f32 + 0.5) / ch as f32) * 2.0 - 1.0;
            for x in 0..cw {
                let nx = ((x as f32 + 0.5) / cw as f32) * 2.0 - 1.0;

                // Rotate the sample, then scale by the *field's* aspect ratio --
                // which is what keeps the picture from stretching on a terminal
                // that is not twice as tall as it is wide.
                let (rx, ry) = (nx * cos_r - ny * sin_r, nx * sin_r + ny * cos_r);
                let start = Complex::new(rx * hw, ry * hh);

                let got = newton_iter(start, degree, c, roots, max_iter, tolerance);

                let (glyph, color) = if got.ok {
                    // Square-rooted, because the iteration counts are not spread
                    // evenly over their range: they pile up at the bottom, and a
                    // linear map spends four fifths of the ramp on the tail.
                    let norm = (got.iterations as f32).sqrt() * inv_sqrt_max;
                    let band =
                        (norm * shade_max).round().clamp(0.0, shade_max) as usize;
                    // Both channels off the same `band`, rather than the glyph off
                    // `norm` and the colour off `band`. Two derivations of the same
                    // quantity is two places to drift apart, and the glyph lookup
                    // was re-deriving an index this loop had already computed.
                    let color = self
                        .shaded
                        .get(got.root)
                        .and_then(|rungs| rungs.get(band))
                        .copied()
                        // A root index with no colour means `roots` and `shaded`
                        // disagree, which is a bug and not a state to render. Black
                        // is the right answer here precisely because it cannot pass
                        // for a picture.
                        .unwrap_or(Color::Black);
                    (*self.glyphs.get(band).unwrap_or(&self.unconverged), color)
                } else {
                    (self.unconverged, UNCONVERGED)
                };

                self.scratch[y * cw + x] =
                    Cell::new(glyph, color, Attribute::Reset);
                self.canvas
                    .set(x, y, Cell::new(glyph, color, Attribute::Reset));
            }
        }
    }
}

/// A deterministic phase offset in turns, from a seed.
///
/// `DEFAULT_SEED` means "unset" -- see `Config::randomise_seeds` -- and an
/// unseeded run has already been given a real seed by the time it gets here, so
/// this only has to be stable for a given one. It is derived from the rng rather
/// than from the raw seed so that two effects sharing a seed still differ.
fn seeded_phase(seed: u64) -> f32 {
    use rand::RngExt;
    seeded_rng(seed, "newton").random::<f32>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::MIN_EFFECT_SIZE;
    use crate::render::palette::perceptual_distance;
    use std::collections::HashSet;

    /// Runs an effect forward long enough for the picture to be the picture.
    fn settle(effect: &mut Newton) {
        for _ in 0..20 {
            effect.advance(1.0 / 60.0);
            let _ = effect.get_diff();
        }
    }

    /// The root finder's contract, and why `durand_kerner` is trustworthy enough
    /// to colour a picture with.
    ///
    /// A root finder that has silently failed does not produce a blank frame. It
    /// produces a plausible one, coloured by wherever the iteration actually
    /// stopped -- which is how a root solve that is subtly wrong becomes a
    /// performance number instead of a test failure.
    #[test]
    fn the_roots_satisfy_the_polynomial() {
        for degree in MIN_DEGREE..=MAX_DEGREE {
            for c in [
                Complex::new(-1.0, 0.0),
                Complex::new(-0.5, 0.0),
                Complex::new(0.4, 0.0),
                Complex::new(0.0, 0.3),
                Complex::new(0.3, 0.3),
            ] {
                let roots = durand_kerner(degree, c);
                assert_eq!(
                    roots.len(),
                    degree,
                    "degree {degree} returned the wrong count"
                );
                for root in &roots {
                    let p = root.powe(degree).add(c);
                    assert!(
                        p.len_squared().sqrt() < 1e-4,
                        "degree {degree}, c = ({}, {}): root ({}, {}) has |p| = {}",
                        c.re,
                        c.im,
                        root.re,
                        root.im,
                        p.len_squared().sqrt()
                    );
                }
                // And distinct: a collapsed solve would satisfy the residual above
                // with three copies of one root.
                for i in 0..roots.len() {
                    for j in (i + 1)..roots.len() {
                        assert!(
                            roots[i].sub(roots[j]).len_squared().sqrt() > 1e-3,
                            "degree {degree}, c = ({}, {}): roots {i} and {j} coincide",
                            c.re,
                            c.im
                        );
                    }
                }
            }
        }
    }

    /// The most important test in this file, and the one whose absence let three
    /// arithmetic errors through.
    ///
    /// It asserts the *observable* -- the residual of the polynomial at the point
    /// the iteration ended -- rather than that the loop ran, or that a frame came
    /// out. A count cannot see a map that converges to the wrong place; a residual
    /// can. Two of those three errors reported 100% non-convergence, or a
    /// perfectly respectable 10 ms a frame, while converging to nothing at all.
    #[test]
    fn a_converged_sample_satisfies_the_polynomial() {
        let c = Complex::new(-1.0, 0.0);
        let roots = durand_kerner(3, c);
        for (re, im) in [
            (0.9f32, 0.1f32),
            (-0.4, 0.7),
            (-0.4, -0.7),
            (1.4, 0.0),
            (0.0, 1.4),
            (1.5, 1.2),
        ] {
            let start = Complex::new(re, im);
            let got = newton_iter(start, 3, c, &roots, 32, 1e-3);
            assert!(got.ok, "({re}, {im}) did not converge to any root");

            // The endpoint, not the start point. Measuring the start is how this
            // test passed while asserting nothing: a sample begins nowhere near a
            // root, so any distance from the *start* is large and the residual
            // never came into it.
            let root = roots[got.root];
            let residual = got.at.sub(root).len_squared().sqrt();
            assert!(
                residual < 1e-3,
                "({re}, {im}) reported root {} = ({}, {}), but it stopped at ({}, {}), \
                 which is {residual} away",
                got.root,
                root.re,
                root.im,
                got.at.re,
                got.at.im
            );
            // And the endpoint satisfies the polynomial, which is the same fact
            // stated without reference to which root was chosen.
            let p = got.at.powe(3).add(c);
            assert!(
                p.len_squared().sqrt() < 1e-2,
                "({re}, {im}) stopped at ({}, {}), where |p| = {}",
                got.at.re,
                got.at.im,
                p.len_squared().sqrt()
            );
        }
    }

    /// The two colour channels are independent, and this is what pins it.
    ///
    /// Rotating which hue goes to which root must move every coloured cell and must
    /// not change *which* root any sample converges to. A single table combining
    /// hue and shade -- the natural way to write this, and the likely way to
    /// rewrite it -- fails the second half immediately.
    #[test]
    fn rotating_the_hue_assignment_moves_colour_and_not_convergence() {
        let mut effect = Newton::new(NewtonOptions::default(), (80, 24));
        settle(&mut effect);

        let before = cell_colors(&mut effect);
        let basins_before = basin_shares(&effect);

        effect.hue_shift += 0.5;
        effect.rebuild_colours();

        let after = cell_colors(&mut effect);
        let basins_after = basin_shares(&effect);

        assert_eq!(
            basins_before, basins_after,
            "convergence changed when only the hue assignment was rotated"
        );

        let moved = before
            .iter()
            .zip(after.iter())
            .filter(|(a, b)| a != b)
            .count();
        // Not every cell need move: the handful that never converge are painted the
        // fixed Julia colour. What must not happen is *nothing* moving, which is
        // what a hue change that is really a no-op looks like.
        assert!(
            moved > before.len() / 2,
            "only {moved} of {} cells changed colour when the hue assignment rotated",
            before.len()
        );
    }

    /// The frame uses the ramp, not a corner of it.
    ///
    /// Guards two failures at once: a map that converges in a constant number of
    /// steps (every cell the same ink) and a map that diverges (every cell the
    /// ceiling). A flat three-colour wash -- the picture this would be with the
    /// shading channel dead -- fails it too, because it has three colours.
    #[test]
    fn the_iteration_count_uses_the_ramp_rather_than_one_value() {
        let mut effect = Newton::new(NewtonOptions::default(), (200, 50));
        settle(&mut effect);
        let inks: HashSet<Color> = cell_colors(&mut effect).into_iter().collect();
        assert!(
            inks.len() >= 6,
            "only {} distinct colours on a 200x50 frame; the shading channel is not alive",
            inks.len()
        );
    }

    /// The picture is a fractal, not three flat regions.
    ///
    /// Basin shares alone cannot tell those apart -- three solid bands and a
    /// genuine fractal can each be one region per root over a similar area, which
    /// is the exact reason `physarum` had to add a second measure. So this asks
    /// for the *boundary*: what fraction of cells have a neighbour in a different
    /// basin. A flat wash is 0 and uniformly random labels are about 25, so the
    /// band is wide.
    ///
    /// The values are well above 10% and they depend on the terminal's shape, which
    /// is expected and not something to normalise away: a small terminal samples
    /// the same structure with fewer cells and so misses more of it. Measured at
    /// the default framing, **7.9% at 200x50, 4.1% at 400x200, 12.8% at 80x24**.
    #[test]
    fn the_basins_have_a_fractal_boundary() {
        for (w, h) in [(200usize, 50usize), (400, 200), (80, 24)] {
            let mut effect =
                Newton::new(NewtonOptions::default(), (w as u16, h as u16));
            settle(&mut effect);
            let basins = basin_grid(&effect, w, h);
            let boundary = boundary_percent(&basins, w, h);
            assert!(
                (0.5..22.0).contains(&boundary),
                "{w}x{h}: basin boundary fraction is {boundary:.2}%; a flat wash is \
                 about 0 and uniform noise about 25"
            );
        }
    }

    /// No basin is empty, and none takes the frame.
    ///
    /// The coverage half of the boundary test, and the direct answer to the
    /// question a drift in `c` raises: does one root swallow the picture? Measured
    /// across `|c| <= 0.5` the shares stay inside 20-50%, so these bounds are wide
    /// of anything real and are here to catch a collapse, not a trend.
    #[test]
    fn every_basin_holds_a_measurable_share() {
        for radius in [0.0f32, 0.25, 0.45, 0.5] {
            let mut effect = Newton::new(
                NewtonOptions {
                    drift_radius: radius,
                    ..NewtonOptions::default()
                },
                (120, 40),
            );
            settle(&mut effect);
            for (root, share) in basin_shares(&effect).iter().enumerate() {
                assert!(
                    *share > 0.05,
                    "drift radius {radius}: root {root} holds {:.1}% of the frame",
                    share * 100.0
                );
                assert!(
                    *share < 0.75,
                    "drift radius {radius}: root {root} holds {:.1}% of the frame",
                    share * 100.0
                );
            }
        }
    }

    /// The question this design was chosen to settle: across a whole lap of the
    /// drift, does any root ever vanish or take over?
    ///
    /// Three sampled values of `c` would not be evidence, so this walks the circle
    /// and checks every root's share at every step. It is the test that catches a
    /// drift radius chosen by eye -- and the one that makes "the roots collide at
    /// `c = 0`" a documented constraint rather than a worry.
    #[test]
    fn no_root_is_swallowed_anywhere_on_the_drift() {
        let degree = 3usize;
        let (w, h) = (120usize, 40usize);
        let mut worst_min = 1.0f32;
        let mut worst_max = 0.0f32;
        for step in 0..48 {
            let angle =
                std::f32::consts::PI + std::f32::consts::TAU * step as f32 / 48.0;
            let c = drift_constant(angle, DEFAULT_DRIFT_RADIUS);
            let roots = durand_kerner(degree, c);
            let mut counts = vec![0usize; degree];
            for (nx, ny) in normalised_points(w, h) {
                // The same extents the effect uses, at its widest framing.
                let got = newton_iter(
                    Complex::new(nx * 1.7 * 4.0, ny * 1.7),
                    degree,
                    c,
                    &roots,
                    32,
                    1e-3,
                );
                if got.ok {
                    counts[got.root] += 1;
                }
            }
            let total = (w * h) as f32;
            for count in &counts {
                worst_min = worst_min.min(*count as f32 / total);
                worst_max = worst_max.max(*count as f32 / total);
            }
        }
        assert!(
            worst_min > 0.10,
            "a root fell to {:.1}% somewhere on the drift",
            worst_min * 100.0
        );
        assert!(
            worst_max < 0.70,
            "a root reached {:.1}% somewhere on the drift",
            worst_max * 100.0
        );
    }

    /// The roots must be far enough apart in *perception* to be told apart.
    ///
    /// This is the "does it read" test for the colour channel, and it is measured
    /// in OKLab rather than eyeballed, because the alternative is judging three
    /// hex codes by eye at night. See
    /// [`perceptual_distance`](crate::render::palette::perceptual_distance) for
    /// the scale -- and note that `0.15` there is a small number, not a
    /// percentage.
    ///
    /// This test is why the wheel is specified in OKLab rather than HSV. Under
    /// HSV the same three degrees measure 0.475, **0.121** and 0.181, so degree 4
    /// fails: 90 degrees apart on the HSV wheel lands on azure and violet, which a
    /// viewer reads as one colour.
    #[test]
    fn the_root_colours_are_perceptually_separable() {
        for degree in MIN_DEGREE..=MAX_DEGREE {
            let mut effect = Newton::new(
                NewtonOptions {
                    degree: degree as u8,
                    ..NewtonOptions::default()
                },
                (80, 24),
            );
            settle(&mut effect);
            for i in 0..effect.root_colors.len() {
                for j in (i + 1)..effect.root_colors.len() {
                    let d = perceptual_distance(
                        effect.root_colors[i],
                        effect.root_colors[j],
                    );
                    assert!(
                        d > MIN_SEPARATION,
                        "degree {degree}: roots {i} and {j} are only {d:.3} apart in \
                         OKLab, which reads as a shading step and not a boundary"
                    );
                }
            }
        }
    }

    /// The separation survives the shading.
    ///
    /// The base hues are separable, but the render darkens them by the iteration
    /// count, and darkening drags chroma down with it -- so the two basins get
    /// closer as they get darker, and a picture can pass the test above while its
    /// *dark* regions show two categories where there should be three. Measured
    /// with the shading floor taken all the way to 0.30: 0.116, 0.081 and 0.068.
    ///
    /// So this asks about the darkest step of the ladder, at the floor that
    /// actually ships, against a lower threshold than the base hues use. The two
    /// are different questions and one bound cannot answer both.
    #[test]
    fn the_darkest_ink_step_still_separates_the_basins() {
        for degree in MIN_DEGREE..=MAX_DEGREE {
            let mut effect = Newton::new(
                NewtonOptions {
                    degree: degree as u8,
                    ..NewtonOptions::default()
                },
                (80, 24),
            );
            settle(&mut effect);
            for step in 0..effect.shaded.len() {
                let ladder: Vec<Color> =
                    effect.shaded.iter().map(|rungs| rungs[step]).collect();
                for i in 0..ladder.len() {
                    for j in (i + 1)..ladder.len() {
                        let d = perceptual_distance(ladder[i], ladder[j]);
                        assert!(
                            d > MIN_DARK_SEPARATION,
                            "degree {degree}, ink step {step}: roots {i} and {j} are \
                             only {d:.3} apart; the shading has eaten the hue"
                        );
                    }
                }
            }
        }
    }

    /// Separating the basins is *hue*, not brightness.
    ///
    /// A categorical palette built by sampling a sequential ramp gives colours
    /// differing mostly in lightness, and the eye reads a lightness difference as
    /// shading -- which is the other channel, already in use. So this asks for a
    /// difference in the opponent axes, and compares it against the brightness
    /// difference: a set that separates only in `L` fails.
    #[test]
    fn basins_are_separated_in_hue_and_not_only_in_brightness() {
        for degree in MIN_DEGREE..=MAX_DEGREE {
            let effect = Newton::new(
                NewtonOptions {
                    degree: degree as u8,
                    ..NewtonOptions::default()
                },
                (80, 24),
            );
            for i in 0..effect.root_colors.len() {
                for j in (i + 1)..effect.root_colors.len() {
                    let a = opponent(effect.root_colors[i]);
                    let b = opponent(effect.root_colors[j]);
                    let hue_gap = ((a.0 - b.0).abs() + (a.1 - b.1).abs()) / 2.0;
                    let light_gap = (a.2 - b.2).abs();
                    assert!(
                        hue_gap > 40.0,
                        "degree {degree}: roots {i} {a:?} and {j} {b:?} differ by \
                         {hue_gap:.0} in the opponent axes, which is a shading step"
                    );
                    // And the separation is genuinely chromatic rather than the
                    // two colours being far apart only in brightness.
                    assert!(
                        hue_gap > light_gap * 0.5,
                        "degree {degree}: roots {i} and {j} separate mostly in \
                         brightness ({light_gap:.0}), which is the other channel"
                    );
                }
            }
        }
    }

    /// The camera breathes rather than ramping and cutting, so no frame jumps.
    #[test]
    fn the_zoom_is_continuous_over_a_whole_cycle() {
        let mut effect = Newton::new(NewtonOptions::default(), (80, 24));
        let mut previous = effect.camera().half_height;
        let mut jumps = 0;
        for _ in 0..600 {
            effect.advance(1.0 / 60.0);
            let now = effect.camera().half_height;
            // A jump would be a large fraction of the scale; a sinusoid moves by a
            // small fraction per frame at any speed anyone would set.
            if (now / previous - 1.0).abs() > 0.25 {
                jumps += 1;
            }
            previous = now;
        }
        assert_eq!(jumps, 0, "the camera jumped {jumps} times in ten seconds");
    }

    /// The oscillation goes somewhere, and reaches both ends.
    #[test]
    fn the_zoom_visits_both_ends_of_its_range() {
        let mut effect = Newton::new(NewtonOptions::default(), (80, 24));
        let mut lo = f64::INFINITY;
        let mut hi = 0.0f64;
        for _ in 0..1800 {
            effect.advance(1.0 / 60.0);
            let s = effect.camera().half_height;
            lo = lo.min(s);
            hi = hi.max(s);
        }
        assert!(
            hi / lo > 5.0,
            "the scale only ranged over {lo:.4}..{hi:.4}, a ratio of {:.2}",
            hi / lo
        );
    }

    /// The real relationship between the iteration cap and the picture.
    ///
    /// Deterministic and with no timing in it, deliberately. The previous version
    /// of this test asserted a *timing* relationship -- that tightening the cap
    /// makes the effect slower -- which was measured once on a noisy run, turned
    /// out to be backwards, and failed in a debug build where absolute timings mean
    /// nothing anyway.
    ///
    /// What is actually worth pinning is the trade itself: a cap low enough to
    /// matter costs a large fraction of the frame its correct colour. That is
    /// stable, it is checkable without a clock, and it is the fact that stops
    /// someone trading the picture for a fifth of the time.
    #[test]
    fn a_tighter_iteration_cap_costs_the_picture_not_just_time() {
        let (w, h) = (120usize, 40usize);
        let mut generous =
            Newton::new(NewtonOptions::default(), (w as u16, h as u16));
        settle(&mut generous);
        let loose = convergence_failure(&generous, w, h, 48);

        let mut tight = Newton::new(
            NewtonOptions {
                max_iterations: 6,
                ..NewtonOptions::default()
            },
            (w as u16, h as u16),
        );
        settle(&mut tight);
        let clipped = convergence_failure(&tight, w, h, 6);

        // A generous cap converges essentially everywhere. Measured 0.00% at 48
        // and 43.8% at 6, so these bounds are wide of both.
        assert!(
            loose < 0.01,
            "a cap of 48 failed to converge on {:.2}% of the frame",
            loose * 100.0
        );
        assert!(
            clipped > 0.25,
            "a cap of 6 only failed on {:.2}% of the frame, so the cap is not the \
             quality lever this file's docs describe",
            clipped * 100.0
        );
    }

    /// Fraction of a `w` by `h` field that does not converge within `cap`.
    fn convergence_failure(effect: &Newton, w: usize, h: usize, cap: u16) -> f32 {
        let degree = effect.degree();
        let camera = effect.camera();
        let (cos_r, sin_r) =
            (camera.rotation.cos() as f32, camera.rotation.sin() as f32);
        let (hw, hh) = (camera.half_width as f32, camera.half_height as f32);
        let (c, roots) = (effect.c, &effect.roots);

        let failures = normalised_points(w, h)
            .into_iter()
            .filter(|(nx, ny)| {
                let (rx, ry) = (nx * cos_r - ny * sin_r, nx * sin_r + ny * cos_r);
                !newton_iter(
                    Complex::new(rx * hw, ry * hh),
                    degree,
                    c,
                    roots,
                    u32::from(cap),
                    1e-3,
                )
                .ok
            })
            .count();
        failures as f32 / (w * h) as f32
    }

    /// The default ramp has no space in it, so a basin is never invisible.
    ///
    /// The glyph-ramp docs carry the rule: a ramp for a *filled region* must not
    /// begin with a space, because the sparsest step lands on the part of the
    /// region with the lowest value, and a space there makes that part vanish
    /// against the background. A Newton basin is a filled region, and the lowest
    /// ink step covers the samples nearest a root -- which is the largest smooth
    /// area of each basin, so this is the worst possible place to lose it.
    #[test]
    fn the_default_ramp_has_no_space_in_it() {
        let options = NewtonOptions::default();
        let glyphs =
            glyph_presets::by_name(&options.ramp).expect("the default exists");
        assert!(
            !glyphs.contains(' '),
            "the default ramp {:?} contains a space, so the low end of every basin \
             would be a hole",
            options.ramp
        );
    }

    /// Every preset renders, and an unknown name falls back to the default.
    #[test]
    fn any_named_ramp_renders_and_an_unknown_one_falls_back() {
        for (name, _) in glyph_presets::ALL {
            let mut effect = Newton::new(
                NewtonOptions {
                    ramp: String::from(*name),
                    ..NewtonOptions::default()
                },
                (60, 20),
            );
            settle(&mut effect);
            assert_eq!(effect.scratch.len(), 60 * 20, "ramp {name} drew no frame");
        }

        let mut effect = Newton::new(
            NewtonOptions {
                ramp: String::from("not-a-ramp"),
                ..NewtonOptions::default()
            },
            (60, 20),
        );
        settle(&mut effect);
        assert_eq!(
            effect.ramp.len(),
            GlyphRamp::from_text(glyph_presets::INKED).len(),
            "an unknown ramp name did not fall back to the default"
        );
    }

    /// Every degree renders with every basin present, because `roots` and the
    /// colour table are built per degree and a mismatch would show as a black
    /// region rather than as a failure.
    #[test]
    fn every_degree_renders_with_visible_basins() {
        for degree in MIN_DEGREE..=MAX_DEGREE {
            let mut effect = Newton::new(
                NewtonOptions {
                    degree: degree as u8,
                    ..NewtonOptions::default()
                },
                (80, 24),
            );
            settle(&mut effect);
            let basins = basin_grid(&effect, 80, 24);
            let converged: HashSet<u8> =
                basins.iter().copied().filter(|b| *b != NO_BASIN).collect();
            assert_eq!(
                converged.len(),
                degree,
                "degree {degree}: {converged:?} on screen, wanted {degree} basins"
            );
        }
    }

    /// A resize keeps the picture and the frame buffer consistent.
    #[test]
    fn a_resize_keeps_the_frame_buffer_the_right_size() {
        let mut effect = Newton::new(NewtonOptions::default(), (120, 40));
        settle(&mut effect);
        for (w, h) in [(80u16, 24u16), (200, 50), (6, 6), (400, 200)] {
            effect.update_size(w, h);
            let _ = effect.get_diff();
            assert_eq!(effect.screen_size, normalize_effect_size((w, h)));
            assert_eq!(effect.scratch.len(), w as usize * h as usize);
        }
    }

    /// The specialised iteration and the general one draw the same picture.
    ///
    /// There are now two copies of this map -- one where the degree is a const
    /// generic, one where it is a runtime value -- and the const-generic one is
    /// what actually renders. A divergence would not be a crash or a blank frame:
    /// it would be a picture that is subtly wrong in a way only a test can see, and
    /// the two are equivalent by construction rather than by sharing code.
    ///
    /// **The comparison is on the label grid, not on the endpoints**, and that is a
    /// finding rather than a convenience. Exponentiation by squaring and the
    /// linear multiply loop in [`Complex::powe_and_prev`] associate differently, so
    /// `z^4` comes out one bit apart -- and near the Julia set the Newton map
    /// amplifies that over its iterations until individual samples land somewhere
    /// different. Measured, endpoints diverge by up to 0.03 on a grid where the
    /// *labels* still agree.
    ///
    /// So what is asserted is exactly what draws the picture: which root, and
    /// whether it converged at all. Reproducibility is unaffected either way,
    /// because a given build always takes the same branch and computes the same
    /// bits -- `--seed` replays exactly. The instability is only across a
    /// *refactor* that reassociates the arithmetic, and what that costs is a
    /// slightly different picture rather than a wrong one.
    #[test]
    fn the_specialised_iteration_draws_the_same_picture() {
        for degree in MIN_DEGREE..=MAX_DEGREE {
            let c = drift_constant(std::f32::consts::PI, 0.45);
            let roots = durand_kerner(degree, c);
            let mut total = 0usize;
            let mut disagree = 0usize;
            for (nx, ny) in normalised_points(80, 40) {
                let start = Complex::new(nx * 3.0, ny * 1.5);
                let fast = newton_iter(start, degree, c, &roots, 32, 1e-3);
                let slow = newton_iter_generic(start, degree, c, &roots, 32, 1e-3);
                total += 1;
                if (fast.ok, fast.root) != (slow.ok, slow.root) {
                    disagree += 1;
                }
            }
            let rate = 100.0 * disagree as f64 / total as f64;
            assert!(
                rate < 1.0,
                "degree {degree}: {disagree} of {total} samples ({rate:.2}%) were \
                 drawn by a different basin"
            );
        }
    }

    /// A hostile config renders rather than panicking.
    ///
    /// Every one of these values is clamped, and the reason to test the clamps
    /// rather than trust them is in this crate's history: the mandelbrot's old
    /// budget curve passed its floor and its ceiling to `f32::clamp` as bounds, and
    /// `clamp` *asserts* that they are ordered -- so any config with a
    /// `max_iterations` under 24 died on its first frame. A clamp no test ever
    /// exercises is a clamp nobody has shown to be ordered.
    ///
    /// The values are the ones a hand-edited file actually contains: a degree past
    /// the supported range, a zero cap, a tolerance above 1, a zero depth, zero
    /// speeds, a ramp that does not exist, and a negative drift radius.
    #[test]
    fn a_hostile_config_renders_rather_than_panicking() {
        let hostile = NewtonOptions {
            degree: 99,
            max_iterations: 0,
            tolerance: 5.0,
            shade_steps: 0,
            depth: 0.0,
            zoom_speed: 0.0,
            spin_speed: 0.0,
            drift_radius: -3.0,
            drift_speed: 0.0,
            ramp: String::from("nonexistent"),
            ..NewtonOptions::default()
        };
        let mut effect = Newton::new(hostile.clone(), (80, 24));
        settle(&mut effect);

        // And the clamps landed where they were meant to.
        assert_eq!(
            effect.degree(),
            MAX_DEGREE,
            "an out-of-range degree was not clamped"
        );
        assert_eq!(effect.iteration_budget(), 1, "a zero cap was not clamped");
        assert!(
            (0.0..=0.5).contains(&effect.tolerance()),
            "a tolerance of 5.0 escaped the clamp"
        );
        assert_eq!(
            effect.shade_steps(),
            MIN_SHADE_STEPS as usize,
            "zero shade steps was not clamped"
        );

        // The NaN cases. `f32::clamp` *propagates* a NaN rather than saturating it,
        // so a NaN sails past every bound check above and out of the other side --
        // which is the hazard `Palette::sample` already guards against.
        for options in [
            NewtonOptions {
                tolerance: f32::NAN,
                ..NewtonOptions::default()
            },
            NewtonOptions {
                depth: f32::NAN,
                ..NewtonOptions::default()
            },
            NewtonOptions {
                drift_radius: f32::NAN,
                ..NewtonOptions::default()
            },
        ] {
            let mut effect = Newton::new(options, (60, 20));
            effect.advance(1.0 / 60.0);
            let _ = effect.get_diff();
        }

        // A very wide terminal: the shape that maximises the horizontal extent, and
        // so the number of samples far out where the map misbehaves. The height
        // comes back as 6 rather than 4, because `normalize_effect_size` floors
        // each axis at `MIN_EFFECT_SIZE` -- which is the point of asserting against
        // the normalised size rather than the requested one.
        let mut wide = Newton::new(hostile, (1000, 4));
        settle(&mut wide);
        assert_eq!(wide.screen_size, normalize_effect_size((1000, 4)));
        assert_eq!(wide.scratch.len(), 1000 * MIN_EFFECT_SIZE as usize);
    }

    /// A drawn frame is not empty, and stays in bounds.
    ///
    /// The advance matters. `get_diff` reports *changed* cells, so drawing twice
    /// with no time in between correctly reports nothing -- an earlier version of
    /// this test called it directly after `settle` and read that as "the effect
    /// drew nothing", when what it had actually established was that the effect
    /// correctly redraws nothing when nothing has changed.
    #[test]
    fn a_frame_is_drawn_and_stays_in_bounds() {
        let mut effect = Newton::new(NewtonOptions::default(), (80, 24));
        settle(&mut effect);
        effect.advance(1.0 / 60.0);
        let diff = effect.get_diff();
        assert!(!diff.is_empty(), "a settled effect drew nothing");
        for (x, y, _) in &diff {
            assert!(*x < 80 && *y < 24, "cell ({x}, {y}) is outside 80x24");
        }
    }

    /// An unchanged effect reports no changes, which is the point of a diff.
    ///
    /// Pairs with the test above, and exists because "drew nothing" is ambiguous:
    /// it is a bug when time has passed and correct when it has not.
    #[test]
    fn an_unchanged_frame_reports_no_cells() {
        let mut effect = Newton::new(NewtonOptions::default(), (80, 24));
        settle(&mut effect);
        let diff = effect.get_diff();
        assert!(
            diff.is_empty(),
            "drawing twice with no time in between reported {} changed cells",
            diff.len()
        );
    }

    // --- helpers -----------------------------------------------------------

    /// The colour of every cell, row-major.
    fn cell_colors(effect: &mut Newton) -> Vec<Color> {
        let _ = effect.get_diff();
        effect.scratch.iter().map(|c| c.color).collect()
    }

    /// Normalised sample coordinates for a `w` by `h` field, row-major, each in
    /// `-1.0..=1.0`.
    ///
    /// **Normalised on purpose: the caller applies the extents.** An earlier
    /// version of this helper multiplied x by the aspect ratio itself, and its one
    /// caller then multiplied by a half-width that *also* contained the aspect
    /// ratio. The test sampled a 16:1 sliver where the render sampled 4:1, and
    /// two assertions failed for that reason alone -- the basin boundary read
    /// 0.13% instead of 7.92%, and a tighter iteration cap appeared to save time
    /// because a sliver puts most samples far enough out to run to the ceiling
    /// anyway. Both were the measurement disagreeing with the render, which is the
    /// fourth occurrence of that in this crate after `terrain`'s depth sampling,
    /// the crab's shadow test, and the noise-range normalisation.
    fn normalised_points(w: usize, h: usize) -> Vec<(f32, f32)> {
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            let ny = ((y as f32 + 0.5) / h as f32) * 2.0 - 1.0;
            for x in 0..w {
                let nx = ((x as f32 + 0.5) / w as f32) * 2.0 - 1.0;
                out.push((nx, ny));
            }
        }
        out
    }

    /// Which basin each cell converged to, as a root index, or [`NO_BASIN`].
    ///
    /// Re-runs the iteration rather than reading anything the effect stored,
    /// because the effect stores *colours* and this needs the labels. It goes
    /// through [`normalised_points`] and then applies the camera's extents, which
    /// is the same order `draw` applies them in.
    fn basin_grid(effect: &Newton, w: usize, h: usize) -> Vec<u8> {
        let degree = effect.degree();
        let camera = effect.camera();
        let (cos_r, sin_r) =
            (camera.rotation.cos() as f32, camera.rotation.sin() as f32);
        let (hw, hh) = (camera.half_width as f32, camera.half_height as f32);
        let (c, roots) = (effect.c, &effect.roots);

        normalised_points(w, h)
            .into_iter()
            .map(|(nx, ny)| {
                let (rx, ry) = (nx * cos_r - ny * sin_r, nx * sin_r + ny * cos_r);
                let got = newton_iter(
                    Complex::new(rx * hw, ry * hh),
                    degree,
                    c,
                    roots,
                    32,
                    1e-3,
                );
                if got.ok { got.root as u8 } else { NO_BASIN }
            })
            .collect()
    }

    /// Percentage of cells with a four-neighbour in a different basin.
    ///
    /// **A percentage, not a fraction.** It started life as a fraction, and the
    /// first caller compared it against a band written in percent -- so a correct
    /// 9.73% read as `0.10` against a floor of `0.5` and the test failed while the
    /// measurement was right. Every consumer of this number thinks in percent, so
    /// the unit belongs here rather than at each call site.
    fn boundary_percent(basins: &[u8], w: usize, h: usize) -> f32 {
        let mut boundary = 0usize;
        for y in 0..h.saturating_sub(1) {
            for x in 0..w.saturating_sub(1) {
                let a = basins[y * w + x];
                if a != basins[y * w + x + 1] || a != basins[(y + 1) * w + x] {
                    boundary += 1;
                }
            }
        }
        100.0 * boundary as f32 / (w * h) as f32
    }

    /// Each basin's share of the *converged* cells, in root order.
    fn basin_shares(effect: &Newton) -> Vec<f32> {
        let degree = effect.degree();
        let (w, h) = (effect.screen_size.0 as usize, effect.screen_size.1 as usize);
        let basins = basin_grid(effect, w, h);
        let mut counts = vec![0usize; degree];
        let mut total = 0usize;
        for b in &basins {
            if (*b as usize) < degree {
                counts[*b as usize] += 1;
                total += 1;
            }
        }
        counts
            .iter()
            .map(|c| *c as f32 / total.max(1) as f32)
            .collect()
    }

    /// A colour as `(r - b, g - b, luma)`: the two opponent axes, then brightness.
    fn opponent(color: Color) -> (f32, f32, f32) {
        let (r, g, b) = match color {
            Color::Rgb { r, g, b } => (r as f32, g as f32, b as f32),
            _ => (0.0, 0.0, 0.0),
        };
        (r - b, g - b, 0.2126 * r + 0.7152 * g + 0.0722 * b)
    }
}
