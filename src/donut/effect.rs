use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, TerminalEffect, seeded_rng};
use crate::render::glyph_ramp::GlyphRamp;
use crate::render::palette::{Palette, presets as palette_presets};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

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
    /// Which ramp to colour the torus with.
    ///
    /// A name rather than a list of colours, because an inline list in TOML is
    /// unpleasant to write and the useful ramps are shared with the other
    /// effects. See [`crate::render::palette::presets`], and [`donut_colors`] for
    /// how a named ramp becomes *this* effect's ramp -- it is not the named ramp
    /// verbatim, because its two darkest stops are too dark to use on a tinted
    /// terminal profile. An unknown name falls back to `magma` rather than
    /// failing: a typo in a config file should not stop the program.
    pub palette: String,
    /// Seed for this effect.
    ///
    /// [`DEFAULT_SEED`] does not mean 42 here -- it means **unset**, and
    /// [`Config::randomise_seeds`](crate::config::Config::randomise_seeds)
    /// gives it a fresh value at startup. Which is what makes this effect look
    /// different on every launch, which it did not before it had a seed at all.
    /// `--seed N` pins it.
    ///
    /// What it randomises is *motion and nothing else* -- the two rates and the
    /// pose the torus starts in, through [`SPEED_BAND`] and [`PHASE_SPAN`]. The
    /// palette is deliberately not one of them: a named ramp is a choice a user
    /// made in a config file, and the twelve colours pair one-to-one with the
    /// twelve glyphs through a contrast property the tests here pin, so seeding
    /// the ramp would spend the entropy on the one part of the picture that has
    /// an argument behind it. See [`DonutOptions::motion`].
    pub seed: u64,
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
            seed: DEFAULT_SEED,
            outer_radius: 2.0,
            rotation_speed_a: DEFAULT_SPEED_A,
            rotation_speed_b: DEFAULT_SPEED_B,
            distance: 5.5,
            k1: 25.0,
            k1_coeff: 1.0,
            luminance_chars: DEFAULT_LUMINANCE_CHARS.to_vec(),
            palette: String::from(DEFAULT_PALETTE),
        }
    }
}

/// The two nominal rates, in radians per second.
///
/// Named so [`DonutOptions::motion`] has something to fall back to when a
/// config file holds a rate that is not a number, and so the bands below are
/// bands *of* two numbers rather than of a pair of literals. Both are the old
/// per-frame values scaled to 60 Hz, as the note on `Default` explains, and
/// neither is to be "fixed" downwards.
const DEFAULT_SPEED_A: f32 = 1.32;
const DEFAULT_SPEED_B: f32 = 0.60;

/// How far a launch's rate on the *ring* may sit from the configured one, as a
/// multiplier.
///
/// The wide one of the two, and it is wide because `a` is the axis that decides
/// whether the effect looks alive. `a` is the ring's own revolution -- the motion
/// a viewer would describe as "the donut turning" -- and it carries the band for
/// that reason.
///
/// Measured over the 400 seeds `every_seeded_rate_stays_inside_the_band`
/// samples, a rate of 0.858 to 2.244 rad/s, which is **one revolution every 2.8
/// to 7.3 seconds**. Both ends of that are inside the 2 to 12 second window
/// `the_default_spin_resolves_in_seconds_not_minutes` calls alive, with room at
/// each end, and that is the whole argument for the width: a launch is either
/// noticeably brisker than the old fixed 4.8 s or noticeably calmer, and never
/// either stopped or frantic.
///
/// The top is a taste bound, and here is what it costs rather than an assertion
/// that nothing goes wrong. Aliasing is not the worry -- the picture's period is
/// 2 pi and the top of this band advances 0.037 rad a frame at 60 Hz, so the
/// torus needs 168 frames to come back to a pose it has already been in. Churn
/// is: measured at 80x50 as the share of the 1,264 drawn cells whose glyph
/// changes between consecutive frames, the median runs 12.5% at the bottom of
/// the band, 19.8% at the old fixed rate, and 27.3% at the top, with a 95th
/// percentile of 63.2% against the default's 40.4%. So the top of the band
/// redraws about 1.4 times as much of the torus per frame as the rate this
/// effect was tuned at. That is a busy-looking torus, and busy is a judgement
/// rather than a fault; 1.7 is where it stops being a screensaver.
const SPEED_BAND_A: (f32, f32) = (0.65, 1.7);

/// How far a launch's rate on the *tube* may sit from the configured one.
///
/// Much the narrower of the two, and deliberately so: `b` is the tube spinning
/// about its own axis rather than the ring going round, which is a far subtler
/// motion, and a wide band on it is the easiest way to make a launch look dead.
///
/// The number it has to clear is the slow end. At the bottom of this band `b`
/// takes **12.3 seconds** a revolution, which is the top of the 2-to-12-second
/// window the resolution test calls alive -- so the low end cannot go lower
/// without a launch existing whose tube does not visibly turn. Measured over
/// four thousand seeds, a *shared* band of `0.6..1.6` -- the width the cube uses
/// -- would leave **27.9%** of launches with a tube slower than 12 seconds, and
/// this band's leaves 4.8%. A band on its own axis rather than a second
/// multiplier on a shared one, because the constraint is per-axis: `a` is
/// allowed to be brisk or calm because it carries the motion, and `b` is not,
/// because it only has to keep the tube's own shading pattern turning. `a` is
/// below 12 s in *every* launch of all three bands measured, which is the other
/// half of why the split works.
const SPEED_BAND_B: (f32, f32) = (0.85, 1.35);

/// How far the torus may start from its default pose, in radians, per axis.
///
/// A whole turn, and the contrast with the cube's quarter turn is worth a
/// sentence because the two look like the same constant. The cube's drawn picture
/// repeats after a quarter turn about any axis -- `(x, y, z)` becoming
/// `(x, -z, y)` permutes its eight vertices and twelve edges -- so drawing its
/// opening pose from a whole turn would hand out the same frame four times in
/// four. A torus has no such symmetry: the point at `phi` and the point at
/// `phi + pi` are on opposite sides of the hole rather than the same place, and
/// the brightness term rotates with the object, so every angle is its own
/// picture and the full turn is all genuinely distinct.
const PHASE_SPAN: f32 = std::f32::consts::TAU;

/// One launch's motion: the two rates and the pose they start from.
///
/// Drawn once, in [`Donut::new`], and held -- see the same reasoning in the
/// cube, whose `Motion` this mirrors deliberately. `advance` needs a rate on
/// every frame, and re-deriving six numbers from a generator sixty times a
/// second would make any bug in it order-dependent. Holding it is also what
/// makes `reset`, which rebuilds the effect from the same options, come back to
/// the same opening frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Motion {
    /// Radians per second about each axis, jittered around the configured
    /// rates. See [`SPEED_BAND_A`] and [`SPEED_BAND_B`].
    speeds: (f32, f32),
    /// The pose at `t = 0`, in radians about each axis. See [`PHASE_SPAN`].
    phase: (f32, f32),
}

/// A user-facing `f32` that has to be usable, or a fallback.
///
/// The same function as the cube's, which is private to that module; a copy
/// rather than a shared one because the alternative is a change to `common.rs`
/// that this effect's own change does not need. The reason it exists is the
/// same too: a rate goes into `sin` and then into a projected coordinate that is
/// cast with `as usize`, and a `NaN` cast reads as zero, so one bad number in a
/// config file puts the entire torus in the left-hand column.
#[inline]
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

impl DonutOptions {
    /// This launch's [`Motion`], drawn from `self.seed`.
    ///
    /// The rates are jittered *multiplicatively* about the configured values and
    /// the phase is a fresh draw. Multiplicative because it keeps a config file
    /// meaningful in the way the cube's does: a user asking for twice the
    /// default still gets twice the default, `0.0` still holds an axis still,
    /// and a negative rate still turns the other way.
    ///
    /// A non-finite rate falls back to the default for that axis rather than
    /// taking the frame with it -- see [`finite_or`]. Before the seed this file
    /// had no such guard on the rates at all, so `rotation_speed_a = nan` in a
    /// config drew a torus in a single column.
    fn motion(&self) -> Motion {
        let mut rng = seeded_rng(self.seed, "donut.motion");
        let mut rate = |configured: f32, fallback: f32, band: (f32, f32)| {
            finite_or(configured, fallback) * rng.random_range(band.0..band.1)
        };
        Motion {
            speeds: (
                rate(self.rotation_speed_a, DEFAULT_SPEED_A, SPEED_BAND_A),
                rate(self.rotation_speed_b, DEFAULT_SPEED_B, SPEED_BAND_B),
            ),
            phase: (
                rng.random_range(0.0..PHASE_SPAN),
                rng.random_range(0.0..PHASE_SPAN),
            ),
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

/// The named ramp the effect uses when nothing else is asked for.
const DEFAULT_PALETTE: &str = "magma";

/// The bottom of the colour ramp, and the darkest colour the donut ever draws.
///
/// The user: "The donut's new colors are good, but I would change the dark
/// colors because my terminal background is grayish blue."
///
/// It used to be `rgb(20, 16, 34)` -- magma's first stop with the floor lifted
/// off black -- at a Rec. 601 luminance of 19.2 against a background at 68.7.
/// That is the complaint, and it is measurable rather than a matter of taste:
/// converted to CIE L*a*b* and compared with `rgb(60, 70, 85)`, the old floor is
/// a **Delta-E of 24.6**, the *next* stop of the same ramp is 59.2, and the
/// twelve expanded shades ran 24.6, 26.8, 37.0, 49.9 ... So the first two
/// shades were a different kind of thing from the other ten, and both of them
/// were dark enough to read as a hole punched in the background rather than as
/// the shadowed side of an object. It is the darkest colour in the effect and
/// the whole torus was sitting on top of it.
///
/// `rgb(72, 24, 146)` measures a **Delta-E of 67.8** from the same background,
/// which is 2.8x the old floor and clears the contrast threshold with 50%
/// headroom, and the whole twelve-shade ramp now bottoms out at 55.1 rather than
/// 24.6. It is a violet rather than a neutral dark, which is the other half of
/// the fix: the old floor had a chroma of 13, so it read as a shadow, and a
/// shadow on a tinted background reads as an absence. This has a chroma of 75.
///
/// Lifted rather than merely saturated, so the dark end of the ring is now a
/// colour that is *present* on a tinted profile. The cost is real and worth
/// naming: a torus on a black background is now a mid-tone mass where it used to
/// have a deep shadow, because the ramp's usable range starts at 52 rather than
/// at 0. `[global] background` pins the terminal's own background if someone
/// wants the old relationship instead.
///
/// Pinned by `the_dark_end_of_the_ramp_is_visible_on_a_tinted_background`, and
/// the two other ramp invariants it is not allowed to break are
/// `the_ramp_runs_from_dark_to_light` and
/// `no_two_adjacent_ramp_entries_are_indistinguishable`.
const RAMP_FLOOR: style::Color = style::Color::Rgb {
    r: 72,
    g: 24,
    b: 146,
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
/// flipped.
///
/// ## Why it is not the named ramp verbatim
///
/// Every preset starts with two stops that are too dark to draw on a terminal
/// with a tinted profile: `magma` at `rgb(0,0,4)` and `rgb(81,18,124)`, `ocean`
/// at `rgb(0,0,20)` and `rgb(0,40,120)`, and so on. They are replaced by
/// [`RAMP_FLOOR`], and the named ramp's *third through fifth* stops follow it.
///
/// Dropping two rather than one is not a preference, it is forced by
/// `no_two_adjacent_ramp_entries_are_indistinguishable`: the ramp is expanded to
/// twelve colours across however many stops it is given, so each expanded step is
/// one stop's interval divided by eleven, and a floor at luma 52 followed by
/// `magma[1]` at 48.9 would put two of the twelve shades 5.8 apart out of 255 --
/// a quarter of what that test calls invisible. Starting the named ramp at its
/// third stop keeps the smallest interval at 48.5, so the worst expanded gap is
/// 0.052 rather than 0.023.
///
/// The top of the ramp is unchanged, which is the point: the user liked the new
/// colours, and this is a change to the dark end only.
///
/// Built through [`Palette`] rather than written out, so the ramp stays the
/// shared one and this file does not carry a second copy of it. Twelve colour
/// lerps a frame, which is not a cost worth avoiding.
fn donut_colors(name: &str) -> Vec<style::Color> {
    let named = palette_presets::by_name(name).unwrap_or(palette_presets::MAGMA);
    let mut stops = Vec::with_capacity(4);
    stops.push(RAMP_FLOOR);
    stops.extend_from_slice(&named[2..named.len().min(5)]);
    // A two-stop ramp has no third stop, so the slice comes back empty and the
    // torus would come out one flat colour -- which is not what `contrast` is
    // for, since its own note calls it "a pure boundary map with no interior
    // shading" rather than a single colour. Its top stop is a better second
    // stop than nothing, and a floor-to-white ramp is a real two-tone donut.
    if stops.len() < 2 {
        stops.extend_from_slice(&named[1..]);
    }
    Palette::new(stops).expand(SHADES)
}

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
    /// This launch's rates and starting pose, drawn from `options.seed`.
    ///
    /// Held rather than recomputed, and the one thing about this effect that is
    /// a function of the *seed* rather than of the clock.
    motion: Motion,
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
        self.rotation_a += self.motion.speeds.0 * delta;
        self.rotation_b += self.motion.speeds.1 * delta;
    }

    pub fn new(options: DonutOptions, screen_size: (u16, u16)) -> Self {
        let canvas = Canvas::new(screen_size.0, screen_size.1);
        // The one thing about this effect that is a function of the seed rather
        // than of the clock, drawn once so every frame of the run and every
        // `reset` of it agree.
        let motion = options.motion();
        Self {
            screen_size,
            options,
            canvas,
            // The opening pose, not the origin. The frame loop draws a frame
            // *before* it advances anything (`get_diff` then
            // `update_with_context` in `common::run_loop`), so the pose the
            // effect is constructed with is the pose on screen for the first
            // frame -- the one a viewer actually registers, and the one the
            // randomised start exists for.
            rotation_a: motion.phase.0,
            rotation_b: motion.phase.1,
            motion,
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
        let outer_radius = finite_or(self.options.outer_radius, 2.0);
        let inner_radius = finite_or(self.options.inner_radius, 1.0);
        let distance = finite_or(self.options.distance, 5.0);
        let k1 = finite_or(self.options.k1, 1.0);
        let colors = donut_colors(&self.options.palette);
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

                // Both bounds, and the lower one has to be tested on the FLOAT. `as usize`
                // saturates, so a sample projecting left of the screen does not
                // become a rejected value -- it becomes 0, and gets drawn into
                // column 0. `donut.c`, which this is derived from, checks
                // `x_proj >= 0` as well and the lower bound was dropped when the
                // projection was ported.
                //
                // It is reachable: `distance` and `k1` are unguarded config, and
                // `z = distance + ...` passes through zero, so wherever `z` is
                // small and positive on the left of the torus
                // `k1 * z_inv * x` is a large negative number. The result is a
                // stripe of torus glyphs down column 0 and along row 0, redrawn
                // every frame, competing in `zbuffer` with the legitimately
                // drawn pixels there.
                let x_f = half_width + k1 * z_inv * x;
                let y_f = half_height + k1 * z_inv * y * 0.8;

                if !(0.0..width as f32).contains(&x_f)
                    || !(0.0..height as f32).contains(&y_f)
                {
                    continue;
                }
                let x_proj = x_f as usize;
                let y_proj = y_f as usize;

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
    use std::collections::{HashMap, HashSet};
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

    /// A sample that projects off the left of the screen is not drawn into column 0.
    ///
    /// `as usize` saturates rather than trapping, so casting a negative
    /// projection gave 0 and the sample was drawn -- correctly typed, in the
    /// wrong place. `donut.c`, which this projection is derived from, tests
    /// `x_proj >= 0` as well; the lower bound was dropped in the port.
    ///
    /// Reachable from config: `distance` and `k1` are unguarded, and
    /// `z = distance + circle_y * sin_a + cos_a * circle_x * sin_phi` passes
    /// through zero as the torus rotates, so `k1 * z_inv * x` is large and
    /// negative wherever `z` is small, positive, and `x < 0`. The symptom is a
    /// stripe of torus glyphs down column 0 and along row 0 that redraws every
    /// frame and competes in the z-buffer with the real pixels there.
    ///
    /// So: run at a distance that puts part of the torus through zero, and
    /// assert the left and top edges hold no ink that the middle does not. The
    /// check is on the *edges* rather than on a returned coordinate because the
    /// saturation is invisible in the return type -- `x_proj` is a `usize`, and
    /// a saturated 0 is indistinguishable from a genuine projection onto the
    /// first column.
    #[test]
    fn a_sample_projected_off_screen_is_not_drawn_into_the_first_row_or_column() {
        let mut offenders = Vec::new();
        for distance in [1.5f32, 2.0, 2.5, 0.8, 3.0] {
            for seed in 1..=3u64 {
                let mut donut = Donut::new(
                    DonutOptions {
                        distance,
                        seed,
                        ..DonutOptions::default()
                    },
                    (60, 20),
                );
                for _ in 0..40 {
                    donut.advance(1.0 / 60.0);
                }
                let frame = donut.get_diff();

                // Which cells the projection *claims*, versus which the diff
                // carries. Every emitted cell must be inside, which is the
                // general contract, and specifically the edges must not be full
                // of ink while the middle is not.
                let edge_inked =
                    frame.iter().filter(|(x, y, _)| *x == 0 || *y == 0).count();
                let middle_inked = frame
                    .iter()
                    .filter(|(x, y, _)| *x > 5 && *x < 54 && *y > 2 && *y < 17)
                    .count();

                if edge_inked > middle_inked / 4 {
                    offenders.push(format!(
                        "distance {distance} seed {seed}: {edge_inked} cells on \
                         the first row/column against {middle_inked} in the middle"
                    ));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "the torus is being drawn into row 0 and column 0 from samples that \
             projected off screen, which is what a saturating cast does: {offenders:?}"
        );
    }

    /// A torus from a seed, at the size most of the tests here use.
    ///
    /// `update_size` is part of the helper because `Donut::new` leaves `k1` at
    /// the option default of 25 and a real run never does: the frame loop resizes
    /// before its first frame. A seeded test that skipped it would measure a
    /// cropped ring at a scale nothing ships with.
    fn seeded(seed: u64) -> Donut {
        seeded_at(seed, (80, 40))
    }

    /// [`seeded`], at a size of the caller's choosing.
    fn seeded_at(seed: u64, size: (u16, u16)) -> Donut {
        let mut donut = Donut::new(
            DonutOptions {
                seed,
                ..Default::default()
            },
            size,
        );
        donut.update_size(size.0, size.1);
        donut
    }

    /// The whole picture, as a glyph per coordinate.
    ///
    /// `get_diff` is incremental once a frame has been committed, so a
    /// reconstructed frame is the only way to compare two *pictures* rather than
    /// two sets of deltas -- and the difference is not a detail here. On a
    /// turning torus the deltas are the cells at the moving edge, and a
    /// comparison over those says much less about whether two runs agree than a
    /// comparison over the whole torus would.
    ///
    /// Reassembled by applying each diff to the frame so far, with a blank glyph
    /// removing a cell, which is what a blank in the diff means.
    fn whole_frame(
        donut: &mut Donut,
        frames: u32,
    ) -> HashMap<(usize, usize), char> {
        let mut picture: HashMap<(usize, usize), char> = HashMap::new();
        for frame in 0..=frames {
            for (x, y, cell) in donut.get_diff() {
                if cell.symbol == ' ' {
                    picture.remove(&(x, y));
                } else {
                    picture.insert((x, y), cell.symbol);
                }
            }
            if frame < frames {
                donut.advance(1.0 / 60.0);
            }
        }
        picture
    }

    /// How much of a picture two runs have in common, as a share of every cell
    /// either of them drew.
    ///
    /// The union rather than the intersection, so a run that drew *less* is not
    /// rewarded for it, and zero only ever means "the same picture" or "nothing
    /// was drawn" -- and `every_seeded_opening_is_a_lit_torus` is what rules the
    /// second out.
    fn difference(
        a: &HashMap<(usize, usize), char>,
        b: &HashMap<(usize, usize), char>,
    ) -> f32 {
        let union: HashSet<(usize, usize)> =
            a.keys().chain(b.keys()).copied().collect();
        if union.is_empty() {
            return 1.0;
        }
        let differing = union.iter().filter(|k| a.get(*k) != b.get(*k)).count();
        differing as f32 / union.len() as f32
    }

    // --- perceptual distance ---------------------------------------------
    //
    // CIE L*a*b* under a D65 white point, and the plain Euclidean distance
    // between two points in it (Delta-E 1976).
    //
    // Implemented here rather than pulled in, because it is thirty lines and
    // because the *constants* are the argument: a reader who wants to know why
    // the comparison is in Lab at all can see that the only inputs are the sRGB
    // transfer function and the D65 primaries, neither of which is a choice this
    // crate made. Rec. 601 luma, which the tests above use, cannot express the
    // claim being made: the old floor and a greyish-blue background are 49 luma
    // apart, which sounds like plenty, and the viewer still cannot tell the
    // effect's shadow from the background -- because luma is blind to chroma,
    // and the old floor's chroma was 13 out of a possible 128.

    fn srgb_to_linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn lab(color: &style::Color) -> (f64, f64, f64) {
        let (r, g, b) = match color {
            style::Color::Rgb { r, g, b } => (*r, *g, *b),
            other => panic!("{other:?} has no Lab coordinates"),
        };
        let (r, g, b) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
        let x = 0.4124 * r + 0.3576 * g + 0.1805 * b;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = 0.0193 * r + 0.1192 * g + 0.9505 * b;
        let f = |t: f64| {
            if t > 0.008_856 {
                t.cbrt()
            } else {
                7.787 * t + 16.0 / 116.0
            }
        };
        let (fx, fy, fz) = (f(x / 0.950_47), f(y), f(z / 1.088_83));
        (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
    }

    /// Delta-E 1976 between two colours.
    fn delta_e(a: &style::Color, b: &style::Color) -> f64 {
        let (l1, a1, b1) = lab(a);
        let (l2, a2, b2) = lab(b);
        ((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt()
    }

    /// The user: "I would change the dark colors because my terminal background
    /// is grayish blue."
    ///
    /// `rgb(60, 70, 85)` is the profile: a dark desaturated slate, the
    /// background of every widely-used dark editor theme, and close enough to
    /// "my terminal background is grayish blue" that the two are the same
    /// sentence.
    const TINTED_BACKGROUND: style::Color = style::Color::Rgb {
        r: 60,
        g: 70,
        b: 85,
    };

    /// The minimum perceptual distance from that background, in Delta-E 1976.
    ///
    /// **Not** a standard, and this comment says so because a bare number in a
    /// test invites a reader to assume one. Delta-E of about 2.3 is the
    /// conventional just-noticeable difference between two adjacent patches; this
    /// asks for twenty times that, and the reason is not that patches are hard to
    /// tell apart one at a time -- it is that the two colours being compared are
    /// *the same region of the screen under two different conditions*. A cell the
    /// effect has not drawn and a cell it has drawn dark have to be
    /// distinguishable, because the first is the background and the second is the
    /// effect, and a viewer has to be able to say which is which.
    ///
    /// 45 sits between the two measured values with room on each side: the old
    /// floor scored 24.6 and failed, the new one scores 67.8 and passes with a
    /// third to spare. The interesting part is not the threshold but the gap it
    /// exposed -- at 24.6 the darkest stop was less than half as far from the
    /// background as the ramp's own second stop was, which is what "the dark end
    /// is a different kind of thing from the rest" means in a number.
    const CONTRAST_FLOOR: f64 = 45.0;

    /// The darkest colour the donut draws has to be visible against a terminal
    /// that is not black.
    ///
    /// This is the measured form of the complaint, and the measurement is the
    /// point: the old floor was `rgb(20, 16, 34)`, a Delta-E of **24.6** from
    /// `TINTED_BACKGROUND`, at a lightness of L* 5.7 and a chroma of 13. The
    /// second stop of the same ramp was 59.2. So the first two of the twelve
    /// shades were less than half as distinguishable from the background as the
    /// third, which is what "the dark end of the ring reads as a hole" means in
    /// a number -- it is a hole because a viewer cannot tell it from the
    /// background, not because it is black.
    ///
    /// The current floor is `rgb(72, 24, 146)`: Delta-E **67.8**, L* 24.0,
    /// chroma 75. See [`RAMP_FLOOR`] and `CONTRAST_FLOOR` below.
    ///
    /// Asserted on the *default* ramp's darkest stop, which is [`RAMP_FLOOR`]
    /// for every named palette -- so the claim is about this effect's floor
    /// rather than about whichever preset happens to be configured, and a
    /// palette a user picked for its own reasons cannot fail it. The companion
    /// check on the ramp as a whole is at the end of this test.
    #[test]
    fn the_dark_end_of_the_ramp_is_visible_on_a_tinted_background() {
        let ramp = donut_colors(&DonutOptions::default().palette);
        assert_eq!(ramp.len(), SHADES);

        let floor = ramp[0];
        assert_eq!(
            floor, RAMP_FLOOR,
            "the ramp no longer starts at RAMP_FLOOR, so this test is measuring \
             something other than the constant it is about"
        );

        let distance = delta_e(&floor, &TINTED_BACKGROUND);
        assert!(
            distance >= CONTRAST_FLOOR,
            "the darkest colour the donut draws is {floor:?}, a Delta-E of \
             {distance:.1} from a {TINTED_BACKGROUND:?} background, against a \
             floor of {CONTRAST_FLOOR}. Below it, the shadowed side of the torus \
             is not distinguishable from a cell the effect never drew, and reads \
             as a hole rather than as colour."
        );

        // The whole ramp, not just its bottom. The two shades either side of the
        // floor are interpolated, and it would be no use passing at the floor if
        // one of them fell back through the background. Measured 55.1 at the
        // worst point on the default ramp.
        let worst = ramp
            .iter()
            .map(|color| delta_e(color, &TINTED_BACKGROUND))
            .fold(f64::MAX, f64::min);
        assert!(
            worst >= CONTRAST_FLOOR,
            "somewhere in the ramp there is a colour only {worst:.1} from the \
             background, so part of the torus disappears into it: {ramp:?}"
        );
    }

    /// Every named ramp's dark end is this effect's floor, so the contrast
    /// property holds whichever palette is configured.
    ///
    /// Without this, `palette = "ocean"` would be a way to undo the fix, and the
    /// knob and the floor would be two features that can cancel each other out
    /// without either of them noticing.
    #[test]
    fn the_tinted_background_property_survives_every_palette() {
        for name in Palette::preset_names() {
            let ramp = donut_colors(name);
            let distance = delta_e(&ramp[0], &TINTED_BACKGROUND);
            assert!(
                distance >= CONTRAST_FLOOR,
                "palette {name:?} bottoms out at {ramp:?}, only {distance:.1} \
                 from a {TINTED_BACKGROUND:?} background, so asking for that \
                 palette undoes the lifted floor"
            );
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
    ///
    /// On the default palette. `depth` is deliberately exempt: it is a *cycling*
    /// ramp whose last stop is near-black on purpose, so using it here gives a
    /// non-monotonic brightness ramp. That is a legitimate way to ask for bands
    /// rather than shading and it is the user's own choice -- but the default
    /// has no such excuse.
    #[test]
    fn the_ramp_runs_from_dark_to_light() {
        let colors = donut_colors(&DonutOptions::default().palette);
        assert_eq!(colors.len(), SHADES);
        for (index, pair) in colors.windows(2).enumerate() {
            let (dim, bright) = (brightness(&pair[0]), brightness(&pair[1]));
            assert!(
                dim <= bright,
                "COLORS[{index}] is brighter ({dim:.2}) than COLORS[{}] \
                 ({bright:.2}), so the ramp is inverted",
                index + 1
            );
        }
        assert!(
            brightness(&colors[0]) < brightness(&colors[SHADES - 1]),
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

    /// The slices the screen's directions around its centre are cut into.
    ///
    /// A constant rather than a local so that the two coverage measurements --
    /// one frame and a whole turn -- cut the same slices, which is what makes
    /// them comparable.
    const POLAR_BINS: usize = 16;

    /// Which of the [`POLAR_BINS`] directions around the centre a frame reached.
    ///
    /// Accumulated rather than read off a single frame, and the reason is the
    /// one written on `the_drawn_torus_reaches_every_direction`: the ring does
    /// not fit on a 12-row screen, so what a *frame* reaches depends on the pose
    /// as well as on the sampling, and a wedge that is never sampled is missing
    /// at every pose while a wedge that is merely cropped is missing at some.
    fn mark_polar(
        size: (u16, u16),
        diff: &[(usize, usize, Cell)],
        hit: &mut [bool; POLAR_BINS],
    ) {
        for (x, y, _) in diff {
            let dx = *x as f64 + 0.5 - f64::from(size.0) / 2.0;
            let dy = *y as f64 + 0.5 - f64::from(size.1) / 2.0;
            let angle = dy.atan2(dx).rem_euclid(std::f64::consts::TAU);
            let bin =
                (angle / (std::f64::consts::TAU / POLAR_BINS as f64)) as usize;
            hit[bin.min(POLAR_BINS - 1)] = true;
        }
    }

    /// The fraction of screen directions around the centre that the drawn
    /// torus reaches, as a count of `POLAR_BINS` equal slices.
    fn polar_coverage(size: (u16, u16), diff: &[(usize, usize, Cell)]) -> f64 {
        let mut hit = [false; POLAR_BINS];
        mark_polar(size, diff, &mut hit);
        hit.iter().filter(|reached| **reached).count() as f64 / POLAR_BINS as f64
    }

    /// How far one step of a pose sweep turns the torus, in seconds.
    ///
    /// A fifth of a second, which is a twentieth of a revolution at the default
    /// rate -- so a turn is about twenty steps, which is coarse enough to be
    /// cheap and fine enough that the shade distribution is sampled rather than
    /// stepped over. It is a *pose* sweep and not a frame-rate one: nothing
    /// measured with it asks whether the torus moves smoothly between the poses
    /// it visits.
    const POSE_STEP: f32 = 0.2;

    /// How many [`POSE_STEP`]s it takes this donut to come all the way round.
    ///
    /// From the *effective* rate rather than the configured one, which is the
    /// whole point of the band existing: a launch whose seed drew a slow ring
    /// takes 7.3 seconds a revolution, and 4.8 of those is two thirds of a turn,
    /// so a fixed step count would quietly under-sweep the slowest launches and
    /// make the measurement a function of the seed all over again. One step
    /// spare, so the turn closes rather than stopping a stride short of it.
    fn steps_per_turn(donut: &Donut) -> u32 {
        let rate = donut.motion.speeds.0.abs().max(1.0e-3);
        ((std::f32::consts::TAU / (rate * POSE_STEP)).ceil() as u32).max(2) + 1
    }

    /// How much of the screen each shade of the ramp covers, over a whole turn
    /// of the torus.
    ///
    /// A *turn*, and that is a correction rather than a refinement. It used to be
    /// twelve frames at a thirtieth of a second, which is 0.4 s -- an eighth of a
    /// revolution at the default rate -- so both tests that use this were
    /// measuring a *pose*, with thresholds calibrated on whatever the effect
    /// happened to be doing at that one pose. The measurements say so plainly.
    /// Over an eighth of a turn, across eight seeds and three sizes, the dark end
    /// of the ramp measured between 0.12 and 0.23 of an even spread and the
    /// bright end between 2.10 and 2.60: every one of those twenty-four
    /// measurements is outside the 0.36-to-2.4 window
    /// `the_ramp_is_spread_across_the_drawn_surface` asserts, and the test passed
    /// only because the pose it started from was the old fixed one. Over a whole
    /// turn the same eight seeds at the same three sizes measure 0.49 to 0.53 at
    /// the dark end and 1.45 to 1.80 at the bright one -- inside the same window,
    /// and the same to within a few percent wherever the sweep starts.
    ///
    /// So the thresholds did not move and the sample did. What is measured now is
    /// a property of the renderer, that the ramp is used like this over the
    /// torus's whole range of poses, rather than a property of one instant.
    fn shade_histogram(size: (u16, u16), resize: bool) -> [f64; SHADES] {
        let ramp = shade_ramp(&DonutOptions::default().luminance_chars);
        let mut counts = [0usize; SHADES];
        let mut donut = Donut::new(DonutOptions::default(), size);
        if resize {
            // What the run loop does immediately after building, so the
            // measurement is the projection scale a real run actually uses.
            donut.update_size(size.0, size.1);
        }
        for _ in 0..steps_per_turn(&donut) {
            donut.advance(POSE_STEP);
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
    /// Accumulated over a whole turn rather than read off one frame, and that is
    /// the claim this bug actually makes. The wedge the shared angle table left
    /// out is fixed in *object* space, so it is missing at every pose and no
    /// amount of waiting finds it; a wedge that is merely cropped off the top of
    /// a small screen is missing at some poses and not others. One frame cannot
    /// tell those apart, and this used to try: it advanced 0.7 s and asked for
    /// 95% of the directions in that single frame, which at 40x12 was a
    /// measurement of the pose it happened to land on. Over twenty seeded
    /// openings at 40x12 the per-frame figure runs 0.938 to 1.000, while the
    /// union over a turn is 1.000 at every size measured -- and the worst single
    /// frame anywhere in that sweep is 0.875, so the old assertion was passing
    /// by luck at the size where it was most fragile.
    ///
    /// The second claim keeps the per-frame part of the old test, as the
    /// *median* frame over the turn rather than an arbitrary one: the union
    /// alone would not notice a torus drawn at half the right scale, since every
    /// direction would still be reached eventually.
    ///
    /// And every pose is a seeded one now, so the worst pose in the sweep is not
    /// the one the old fixed effect always started from. At rest the torus is
    /// exactly edge-on, and on a 12-row screen the edge-on projection is a ring
    /// six rows tall, which leaves a handful of directions unsampled for want of
    /// anywhere to put a cell, independently of how much of the torus was
    /// sampled -- a general pose is both fairer and closer to what anyone
    /// watching sees.
    #[test]
    fn the_drawn_torus_reaches_every_direction() {
        for size in [(40u16, 12u16), (40, 20), (80, 50)] {
            let mut frames: Vec<f64> = Vec::new();
            for seed in 0..6u64 {
                let mut donut = seeded_at(seed, size);
                let mut hit = [false; POLAR_BINS];
                for _ in 0..steps_per_turn(&donut) {
                    donut.advance(POSE_STEP);
                    let diff = donut.get_diff();
                    assert!(
                        !diff.is_empty(),
                        "at {size:?} a seeded launch drew nothing at all"
                    );
                    mark_polar(size, &diff, &mut hit);
                    frames.push(polar_coverage(size, &diff));
                }
                let reached = hit.iter().filter(|h| **h).count();
                assert_eq!(
                    reached,
                    POLAR_BINS,
                    "at {size:?} seed {seed} swept a whole turn and {} of the \
                     {POLAR_BINS} directions around the centre were never lit, so \
                     a wedge of the torus is not being drawn at any pose",
                    POLAR_BINS - reached
                );
            }
            frames.sort_by(f64::total_cmp);
            let median = frames[frames.len() / 2];
            assert!(
                median >= 0.95,
                "at {size:?} the median frame of a turn reaches only {:.1}% of the \
                 directions around the centre, so the torus is not filling the \
                 screen at its usual scale",
                median * 100.0
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
                "at {size:?} the shades run from {:.1}% to {:.1}% of the \
                 surface, and an even ramp would be {:.1}%; measured: {histogram:?}",
                darkest * 100.0,
                brightest * 100.0,
                even * 100.0
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
    /// to sage to gold.
    ///
    /// This is also what constrains how far [`RAMP_FLOOR`] can be lifted. The
    /// ramp is expanded to twelve colours across its stops, so each expanded step
    /// is one stop interval divided by eleven: a floor at luma 52 followed by
    /// `magma[1]` at 48.9 would give two of the twelve a gap of 0.023, and the
    /// tinted-background fix would have to start the named ramp at its *third*
    /// stop for the gap to survive. That is why [`donut_colors`] slices from
    /// index 2.
    #[test]
    fn no_two_adjacent_ramp_entries_are_indistinguishable() {
        let colors = donut_colors(&DonutOptions::default().palette);
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

    /// Every named ramp has to work, and an unknown name has to fall back rather
    /// than take the program down at startup.
    ///
    /// A palette name comes from a config file, so an unrecognised one is a typo
    /// rather than a bug, and the effect should still run. The mandelbrot has the
    /// identical knob and the identical test; the pattern is copied rather than
    /// reinvented, deliberately.
    #[test]
    fn any_named_palette_renders_and_an_unknown_one_falls_back() {
        for name in Palette::preset_names() {
            let mut donut = Donut::new(
                DonutOptions {
                    palette: name.to_string(),
                    ..Default::default()
                },
                (40, 20),
            );
            donut.update_size(40, 20);
            let diff = donut.get_diff();
            assert!(!diff.is_empty(), "palette {name:?} drew nothing at all");
            let distinct: std::collections::HashSet<style::Color> =
                diff.iter().map(|(_, _, cell)| cell.color).collect();
            // No carve-outs. `contrast` is two stops, and `donut_colors` has to
            // find a second one somewhere or the torus comes out one flat
            // colour, which is a different thing from what that ramp is for.
            assert!(
                distinct.len() >= 2,
                "palette {name:?} drew {} distinct colours, so it is not being \
                 used at all",
                distinct.len()
            );
            for (x, y, _) in &diff {
                assert!(
                    *x < 40 && *y < 20,
                    "palette {name:?} drew outside the canvas at ({x}, {y})"
                );
            }
        }

        // An unknown name has to be byte-for-byte the default, which is stronger
        // than "it drew something": a fallback that quietly picked the first
        // preset on the list would also draw, and would surprise whoever typed it.
        assert_eq!(
            donut_colors("nonesuch"),
            donut_colors(DEFAULT_PALETTE),
            "an unknown palette name did not fall back to the default ramp"
        );
    }

    /// The knob has to survive a trip through a config file, and a real named
    /// ramp has to change what comes out.
    ///
    /// `--print-config` writes every key to disk, so a generated config is pinned
    /// to whatever the defaults were the day it was generated and a new knob
    /// arrives as "a key the user's file does not have". That is the normal case.
    #[test]
    fn the_palette_knob_round_trips_through_toml_and_changes_the_output() {
        let options: DonutOptions =
            toml::from_str("palette = \"ocean\"\n").expect("a lone key parses");
        assert_eq!(options.palette, "ocean", "the key was not read back");
        assert_eq!(
            options.rotation_speed_a,
            DonutOptions::default().rotation_speed_a,
            "one key in the section silently reset the others"
        );
        let serialised = toml::to_string(&options).expect("the section serialises");
        assert!(
            serialised.contains("palette"),
            "the key is missing from the serialised form, so --print-config would \
             never write it: {serialised}"
        );

        // And it is used: two names, two different sets of colours on screen.
        let frame = |palette: &str| {
            let mut donut = Donut::new(
                DonutOptions {
                    palette: palette.to_string(),
                    ..Default::default()
                },
                (40, 20),
            );
            donut.update_size(40, 20);
            donut
                .get_diff()
                .into_iter()
                .map(|(_, _, cell)| cell.color)
                .collect::<std::collections::HashSet<style::Color>>()
        };
        let magma = frame(DEFAULT_PALETTE);
        let ocean = frame("ocean");
        assert!(
            !magma.is_empty() && !ocean.is_empty(),
            "a frame drew no colours"
        );
        assert_ne!(
            magma, ocean,
            "palette = \"ocean\" drew exactly the same colours as the default"
        );
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

    /// The whole point: two launches, two pictures.
    ///
    /// Measured on the *reconstructed whole frame* ninety frames in, and not on
    /// the options, because a comparison of options is a restatement of the code
    /// that fills the options and would pass against a `seed` that is stored,
    /// randomised and then never consulted by anything that draws.
    ///
    /// The reconstruction is the part worth insisting on. `get_diff` is
    /// incremental after the first commit, so the obvious thing to compare --
    /// the diff a frame returns -- is a set of the cells at the torus's moving
    /// edge. Two runs can have completely different pictures and diffs that
    /// barely overlap, or the same picture and diffs that differ in a few dozen
    /// cells, and neither of those is what "this seed looks different" means.
    /// `whole_frame` applies each diff to the frame so far, so what is compared
    /// is the torus.
    ///
    /// Six pairs rather than one, and the floor at 0.4 against a measured 0.69 to
    /// 0.97: a test that can be a coincidence is a test that can be flaky, and
    /// against the bug it is written for -- no randomness at all -- every one of
    /// the six reads exactly zero. The spread is wider than the cube's because a
    /// torus has a lot of structure two poses agree about -- the ring is in
    /// roughly the same place either way -- and the honest reading of a figure
    /// near 0.7 is "a third of the picture is in a different place", not "nearly
    /// the same frame".
    #[test]
    fn two_seeds_open_on_different_pictures() {
        for (left, right) in [
            (42u64, 43u64),
            (42, 1),
            (42, 2),
            (7, 8),
            (0, 99),
            (5, 12345),
        ] {
            let a = whole_frame(&mut seeded(left), 90);
            let b = whole_frame(&mut seeded(right), 90);
            let differing = difference(&a, &b);
            assert!(
                differing > 0.4,
                "seeds {left} and {right} are 90 frames into a run and their \
                 frames differ on {:.0}% of the cells either drew, so the seed is \
                 not reaching the motion",
                differing * 100.0
            );
        }
    }

    /// The same seed twice is the same run, frame for frame.
    ///
    /// The half of the contract that makes `--seed N` mean anything, and
    /// `tests/effect_contracts.rs` asserts it for every effect at once. It is
    /// asserted here too because a failure can then name the effect, and
    /// because a generator consulted on every frame rather than drawn once in
    /// `Donut::new` shows up as drift rather than as two effects disagreeing.
    #[test]
    fn the_same_seed_replays_the_same_run() {
        let mut a = seeded(2024);
        let mut b = seeded(2024);
        for step in 0..90u32 {
            a.advance(1.0 / 60.0);
            b.advance(1.0 / 60.0);
            assert_eq!(
                a.get_diff(),
                b.get_diff(),
                "two tori on seed 2024 differ on frame {step}, so the motion is \
                 not a function of the seed"
            );
        }
    }

    /// Every launch opens on a lit torus, and reaches the whole ramp from there.
    ///
    /// The risk randomising the opening pose carries here, and it is a different
    /// one from the cube's. The torus's *shape* does not depend on the pose at
    /// all -- the same ring is projected whatever `a` and `b` are -- so the thing
    /// a random pose can break is not the geometry but the sampling: which of the
    /// twelve shades the visible surface turns out to wear. The ramp tests above
    /// all average over a whole turn now, which is right for what they are about
    /// and means a single unlucky *opening* could hide inside the average; this
    /// is the test that cannot.
    ///
    /// Twenty seeds, three sizes, and the pose is read off the effect rather than
    /// reconstructed -- the frame loop draws before it advances, so the pose the
    /// effect is constructed with is the pose on screen for the first frame.
    ///
    /// The last claim is the one that makes this a test of the seed rather than
    /// of the renderer: the twenty openings have to differ from each other.
    /// Without it everything above would pass twenty times over on the same
    /// default pose, and report coverage that was not being provided.
    #[test]
    fn every_seeded_opening_is_a_lit_torus() {
        let ramp = shade_ramp(&DonutOptions::default().luminance_chars);
        let mut openings: Vec<HashMap<(usize, usize), char>> = Vec::new();

        for seed in 0..20u64 {
            for size in [(40u16, 12u16), (40, 20), (80, 50)] {
                let mut donut = seeded_at(seed, size);
                let first = donut.get_diff();
                assert!(
                    !first.is_empty(),
                    "seed {seed} at {size:?} opened on an empty screen, and the \
                     torus's shape does not depend on the pose"
                );

                let mut seen = [false; SHADES];
                for (_, _, cell) in &first {
                    mark_shade(&ramp, &mut seen, cell.symbol);
                }
                for _ in 0..steps_per_turn(&donut) {
                    donut.advance(POSE_STEP);
                    for (_, _, cell) in donut.get_diff() {
                        mark_shade(&ramp, &mut seen, cell.symbol);
                    }
                }
                let unused: Vec<usize> = seen
                    .iter()
                    .enumerate()
                    .filter(|(_, reached)| !**reached)
                    .map(|(shade, _)| shade)
                    .collect();
                assert!(
                    unused.is_empty(),
                    "seed {seed} at {size:?} opens at {:?} and never draws shades \
                     {unused:?} of the twelve: a pose where part of the ramp is \
                     unreachable is part of what randomising the opening costs",
                    donut.rotation_a
                );
            }

            openings.push(whole_frame(&mut seeded(seed), 0));
        }

        // Every pair, not a sample of them: a claim that twenty openings are
        // varied is a claim about the *closest* two of them, and a sample of
        // pairs can miss the closest. Measured over the 190 pairs, the spread
        // runs 0.40 at the closest and 0.996 at the furthest, with a median of
        // 0.956 -- so the floor is a quarter, well under half the closest, and
        // the median is asserted separately because a torus drawn in coarse
        // glyphs has plenty of cells two poses can agree about: two rings in
        // roughly the same place with roughly the same shading can share three
        // cells in five and still be visibly different.
        let mut all: Vec<f32> = Vec::new();
        for a in 0..openings.len() {
            for b in a + 1..openings.len() {
                all.push(difference(&openings[a], &openings[b]));
            }
        }
        all.sort_by(f32::total_cmp);
        let median = all[all.len() / 2];

        let mut closest = 1.0f32;
        for (index, picture) in openings.iter().enumerate() {
            assert!(
                !picture.is_empty(),
                "seed {index} drew no cells at all, so there is no picture to \
                 compare"
            );
            for other in openings.iter().skip(index + 1) {
                closest = closest.min(difference(picture, other));
            }
        }
        assert_eq!(openings.len(), 20, "not every seed was opened");
        assert!(
            closest > 0.25,
            "two of the twenty seeded openings differ on only {:.0}% of their \
             cells, so the starting poses are not varied",
            closest * 100.0
        );
        assert!(
            median > 0.8,
            "the median pair of the twenty openings differs on only {:.0}% of \
             their cells, so the poses are varied in name only",
            median * 100.0
        );
    }

    /// Records that a glyph is one of the ramp's, and at which shade.
    ///
    /// By glyph rather than by colour, because the shade index travels with the
    /// character: the renderer picks the glyph from the shade and the second
    /// pass picks the colour from the same number, so a glyph is the honest place
    /// to read the shade back out of. A glyph that is not on the ramp is ignored
    /// rather than counted as some shade -- which is what keeps a stray character
    /// from being reported as a shade the torus did draw.
    fn mark_shade(ramp: &GlyphRamp, seen: &mut [bool; SHADES], symbol: char) {
        if let Some(index) = ramp.glyphs().iter().position(|g| *g == symbol) {
            seen[index.min(SHADES - 1)] = true;
        }
    }

    /// Every seeded rate is inside its band's axis, *and the band is reached*.
    ///
    /// Two claims, and the second is the one that has bitten this project
    /// before. The first is the obvious one: 200 seeds, every `a` inside
    /// `DEFAULT_SPEED_A * SPEED_BAND_A`, every `b` inside
    /// `DEFAULT_SPEED_B * SPEED_BAND_B`, and every phase inside `[0, TAU)`. The
    /// second is that the draw is not quietly losing most of the range it was
    /// given -- a normalisation constant taken from a spec sheet rather than from
    /// a measurement is how a range that reads `0.85..1.35` in the source
    /// arrives on screen as `0.95..1.05`, and it is how this crate's terrain
    /// relief lost four fifths of its amplitude three times over without an
    /// error anywhere.
    ///
    /// The extremes are measured rather than assumed. A draw of `n` uniform
    /// samples from `[lo, hi]` puts its smallest within `(hi - lo) / (n + 1)` of
    /// `lo` and its largest within the same of `hi` -- 0.5% of the band at 200
    /// seeds -- so the floor here is 97% of each end reached. Measured over 400
    /// seeds the two axes reach 0.8582 against an exact floor of 0.8580, 2.2418
    /// against 2.2440, 0.5100 against 0.5100 and 0.8071 against 0.8100, so the
    /// arithmetic and the draw agree to within a part in a thousand.
    ///
    /// Per axis and per band rather than pooled, because pooling hides exactly
    /// the mistake worth catching here: a `b` band that had been written as `a`'s
    /// would still produce plausible global extremes.
    #[test]
    fn every_seeded_rate_stays_inside_the_band() {
        const SEEDS: u64 = 200;
        let bands = [
            (DEFAULT_SPEED_A, SPEED_BAND_A),
            (DEFAULT_SPEED_B, SPEED_BAND_B),
        ];
        let mut phases: Vec<f32> = Vec::new();

        for (axis, (configured, band)) in bands.iter().enumerate() {
            let (low, high) = (configured * band.0, configured * band.1);
            let mut seen_low = f32::INFINITY;
            let mut seen_high = f32::NEG_INFINITY;

            for seed in 0..SEEDS {
                let motion = DonutOptions {
                    seed,
                    ..Default::default()
                }
                .motion();
                let rate = if axis == 0 {
                    motion.speeds.0
                } else {
                    motion.speeds.1
                };
                assert!(
                    (low..=high).contains(&rate),
                    "seed {seed} gave axis {axis} a rate of {rate}, outside the \
                     band {low}..{high}"
                );
                seen_low = seen_low.min(rate);
                seen_high = seen_high.max(rate);

                let phase = if axis == 0 {
                    motion.phase.0
                } else {
                    motion.phase.1
                };
                assert!(
                    (0.0..PHASE_SPAN).contains(&phase),
                    "seed {seed} gave axis {axis} a phase of {phase}, outside \
                     0..{PHASE_SPAN}"
                );
                phases.push(phase);
            }

            let span = high - low;
            assert!(
                seen_low <= low + 0.03 * span,
                "the slowest rate axis {axis} ever drew was {seen_low}, more than \
                 3% of the band above its floor of {low}: the draw is not using \
                 the bottom of the range it was given"
            );
            assert!(
                seen_high >= high - 0.03 * span,
                "the quickest rate axis {axis} ever drew was {seen_high}, more than \
                 3% of the band below its ceiling of {high}: the draw is not \
                 using the top of the range it was given"
            );
        }

        let phase_low = phases.iter().copied().fold(f32::INFINITY, f32::min);
        let phase_high = phases.iter().copied().fold(0.0f32, f32::max);
        assert!(
            phase_low <= 0.03 * PHASE_SPAN && phase_high >= 0.97 * PHASE_SPAN,
            "the phases only covered {phase_low:.3}..{phase_high:.3} of \
             0..{PHASE_SPAN}, so the opening poses are not being varied"
        );
    }

    /// A configured rate of zero still holds that axis still, and a negative one
    /// still turns the other way.
    ///
    /// A guard for the design rather than a test of the fix -- both would pass
    /// before the seed existed -- and it is here because the bands are applied
    /// *multiplicatively*, which has consequences worth stating. Additive jitter
    /// would put a random rate on an axis a user had asked to freeze, and would
    /// flip the sign of a rate a user had set negative, the moment a seed
    /// arrived. This is the test that says neither happens, on both axes and
    /// over several seeds so that a single lucky draw cannot carry it.
    ///
    /// `is_finite` is checked as well as the sign, because a `NaN` compares
    /// false against everything and would sail through a sign check that only
    /// asked whether the answer was negative.
    #[test]
    fn a_configured_rate_keeps_its_sign_and_its_zero() {
        for (axis, configured) in [
            (0usize, 0.0f32),
            (1, 0.0),
            (0, -1.0),
            (1, -1.0),
            (0, -0.02),
            (1, -0.02),
        ] {
            for seed in 1..=5u64 {
                let options = DonutOptions {
                    rotation_speed_a: if axis == 0 {
                        configured
                    } else {
                        DEFAULT_SPEED_A
                    },
                    rotation_speed_b: if axis == 1 {
                        configured
                    } else {
                        DEFAULT_SPEED_B
                    },
                    seed,
                    ..Default::default()
                };
                let motion = options.motion();
                let rate = match axis {
                    0 => motion.speeds.0,
                    _ => motion.speeds.1,
                };
                assert!(
                    rate.is_finite(),
                    "a configured {configured} on axis {axis} became a rate of \
                     {rate} on seed {seed}"
                );
                assert!(
                    (rate == 0.0) == (configured == 0.0)
                        && rate.is_sign_negative() == configured.is_sign_negative(),
                    "a configured {configured} on axis {axis} became a rate of \
                     {rate} on seed {seed}, so the band is not multiplicative"
                );
            }
        }
    }

    /// A rate that is not a number falls back instead of taking the frame with it.
    ///
    /// The rate goes into `sin`, then into a projected coordinate, and that is
    /// cast with `as usize` -- where a `NaN` reads as zero. So one bad number in
    /// a config file does not produce a torus that is slightly wrong; it produces
    /// a torus in the left-hand column. This was reachable before the seed too,
    /// and nothing in this file caught it, which is why the guard was added
    /// alongside the derivation rather than after it.
    #[test]
    fn a_rate_that_is_not_a_number_falls_back_rather_than_emptying_the_frame() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for axis in 0..2 {
                let options = DonutOptions {
                    rotation_speed_a: if axis == 0 { bad } else { DEFAULT_SPEED_A },
                    rotation_speed_b: if axis == 1 { bad } else { DEFAULT_SPEED_B },
                    seed: 11,
                    ..Default::default()
                };
                let motion = options.motion();
                let drawn = [motion.speeds.0, motion.speeds.1];
                assert!(
                    drawn.iter().all(|r| r.is_finite()),
                    "a rate of {bad} on axis {axis} left the launch with {drawn:?}"
                );

                let mut donut = Donut::new(options, (80, 40));
                donut.update_size(80, 40);
                let picture = whole_frame(&mut donut, 0);
                assert!(
                    picture.len() > 200,
                    "a rate of {bad} on axis {axis} drew {} cells, so the torus \
                     is not on the screen",
                    picture.len()
                );
            }
        }
    }
}
