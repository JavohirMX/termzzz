use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::render::glyph_ramp::{self, GlyphRamp};
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, SQRT_2};
use std::sync::LazyLock;

/// `std::f64::consts` carries `SQRT_2` and nothing else, so the other two roots
/// this field needs are spelled out here.
///
/// Each literal is the correctly-rounded `f64` nearest the root, which is all a
/// frequency ratio needs: what matters is that it is irrational and that it is
/// not a rational multiple of any of the others, and both properties survive to
/// the last bit. `sqrt(3)`, `sqrt(5)` and `sqrt(7)` are pairwise irrational in
/// *every* ratio -- `sqrt(5/3)`, `sqrt(7/5)` and the rest are all irrational,
/// because the quotient of two square roots of distinct squarefree integers is
/// irrational. That is the property [`TIME_FREQ_DIAGONAL`] and
/// [`TIME_FREQ_BREATH`] rely on, and it is why the set is those three and not
/// `sqrt(6)` or `sqrt(8)`: `sqrt(8) = 2*sqrt(2)`, and a rational factor in
/// there is a rational ratio between two frequencies and a period with them.
const SQRT_3: f64 = 1.732_050_807_568_877_2;
const SQRT_5: f64 = 2.236_067_977_499_79;
const SQRT_7: f64 = 2.645_751_311_064_590_7;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlasmaOptions {
    pub time_scale: f64,
    pub spatial_scale: f64,
    pub color_speed: f64,
    /// Characters the plasma value is drawn as, sparsest first.
    pub glyphs: String,
    /// Seed for this effect.
    ///
    /// [`DEFAULT_SEED`] does not mean 42 here -- it means **unset**, and
    /// [`Config::randomise_seeds`](crate::config::Config::randomise_seeds)
    /// gives it a fresh value at startup. Which is what makes this effect look
    /// different on every launch, which it did not before it had a seed at all.
    /// `--seed N` pins it.
    pub seed: u64,
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
            seed: DEFAULT_SEED,
            spatial_scale: 1.0,
            color_speed: DEFAULT_COLOR_SPEED,
            glyphs: DEFAULT_GLYPHS.to_string(),
        }
    }
}

/// Palette entries the colour offset travels per second, before `time_scale`.
///
/// Was 20, which with `time_scale` 0.5 put the offset at 10 entries a second --
/// about a sixth of the whole wheel *per frame* at 60 Hz. A cell's colour is
/// `palette[plasma + time * color_speed]`, so a sixth of the wheel moving past
/// each frame means roughly a sixth of the screen repainting every frame. At
/// 400x200 that measured 824 KB of escape sequences per frame, 49 MB/s, and it
/// is the largest single source of the lag.
///
/// At 4 the offset moves 2 entries a second: one whole palette step every half
/// second, which is plainly visible, and about 3% of cells change per frame
/// rather than 17%. A full lap of the 256-entry wheel takes just over two
/// minutes, which is fine for a screensaver -- the field's own spatial phase is
/// what supplies the faster motion, and it is unaffected by this.
const DEFAULT_COLOR_SPEED: f64 = 4.0;

/// Saturation of every palette entry.
const PALETTE_SATURATION: f64 = 1.0;
/// Darkest value on the wheel, as a fraction of full. The old ramp ran each
/// channel from 0 to 255 about mid-grey, so it had no dark end at all and the
/// whole screen sat in a pale haze.
const PALETTE_MIN_VALUE: f64 = 0.40;
/// Entries in the colour wheel. The index is taken modulo this, so it is also
/// the wheel's period, and one lap of `hue` is exactly `PALETTE_LEN` steps --
/// which is what makes the wrap seamless.
const PALETTE_LEN: usize = 256;

/// The glyph ramp the field is drawn as.
///
/// ASCII, and not the shade blocks, because the user asked for it: the blocks
/// "are unnecessary; maybe ASCII characters would be better". The blocks are
/// also a portability hazard -- `░▒▓█` are one fixed block in one fixed range, and
/// a terminal whose font does not cover them is a screensaver that draws a
/// field of replacement characters.
///
/// # What is given up
///
/// This *was* [`glyph_ramp::presets::BLOCKS`], and it was not a careless
/// choice. `░▒▓█` are defined by Unicode as quarter, half, three-quarter and
/// full coverage of the cell box, so their order is guaranteed by the standard
/// rather than by taste, and it holds at any cell aspect ratio. ASCII has
/// neither property, and there is no ASCII set that has them: no amount of
/// choosing makes `:` reliably lighter than `-`, and every font is free to
/// disagree. The ordering below is therefore an *estimate* of ink coverage, and
/// the estimate is written out as a table in
/// `the_default_ramp_is_ordered_by_estimated_ink_coverage` so it can be argued
/// with rather than taken on trust.
///
/// # Why not `presets::SHADE`
///
/// `" .:-=+*#%@"` is the conventional answer, and it was rejected on the very
/// property the blocks were chosen for. Under the same ink table it contains
/// two outright inversions -- `:` before `-`, and `+` before `*` -- so a rising
/// value makes the field *lighter* at those two steps, and the rest of the ramp
/// is noise. The crate's own note on `SHADE` names `=` and `+` and `+` and `*`;
/// this ramp's table puts `=` and `+` within a rounding of each other, so the
/// one it is confident about is `+` and `*`.
///
/// # Why this ordering
///
/// Sixteen steps, up from ten. The count is a *variety* decision and the user
/// asked for it -- "you can use more different characters" -- and it is worth
/// being honest about the ceiling. ASCII runs out of distinguishable weights
/// long before it runs out of characters: below about a tenth of the cell's ink
/// the repertoire is a crowd of one-tick marks, and no choice of sixteen of them
/// makes sixteen *bands*. Nine of the fifteen boundaries below are within the
/// ink table's resolution of their neighbour and are named as ties in the
/// ordering test, which is what "a ramp for a *filled region* cannot begin with
/// a space"-style limits look like from the inside. The consequence is stated
/// where it is paid for, in [`VALUE_RESPONSE`].
///
/// Ordered lightest-first by estimated coverage, and shaped for this effect
/// specifically:
///
/// - The mid-range glyphs differ in *shape* as well as in weight. `SHADE`'s
///   middle is `-`, `=`, `+`, `*`: four variations on a straight stroke, so a
///   whole region of the field is one repeated form and the value it carries is
///   hard to read. `-`, `:`, `=`, `;`, `*`, `+`, `X` are seven different
///   silhouettes, and no two glyphs in the whole set share one.
///
/// - The heavy end adds *shapes* rather than more of the same mark. A
///   punctuation-only ramp past `#` has nowhere to go but a thicker `+`, and the
///   five steps from `O` to `@` are five closed or bounded forms at five weights:
///   a ring, two rings and a slash, a bowl and a loop, four long strokes, and a
///   ring with an inner bowl and a tail. The eye separates silhouettes more
///   reliably than it separates weights, so this is where extra steps buy most.
///
/// - The heavy end is no longer a tail in *area*, which the ten-step version of
///   this note relied on, and the reason is the re-spacing: `@` now covers about
///   a sixteenth of the field rather than none of it. It is still a tail in
///   *value* -- reaching it takes a value above 0.737, which is where the field's
///   bright cores are -- but not in area, and the note that used to lean on area
///   is what [`glyph_value`] had to be written to undo.
///
/// - A monochrome plasma is an unusual thing to want, and worth saying why the
///   two channels are worth separating. The colour is a saturated hue wheel
///   carrying the *raw* value, and it is doing most of the visual work: it never
///   repeats a shade, it has a genuine dark end, and it changes hue as the field
///   breathes. The glyph is a second, much lower-resolution encoding of the same
///   scalar, and at sixteen steps it is the channel that runs out of distinct
///   levels. Calibration belongs on the channel that needs it.
///
/// # The pairs the estimate cannot separate
///
/// **Nine**, up from three, and they are named in the test rather than left to
/// be rediscovered. That is the cost of a sixteen-step ASCII ramp and it is
/// stated here rather than discovered later: see
/// `the_default_ramp_is_ordered_by_estimated_ink_coverage`. The pattern in them
/// is worth seeing on its own -- every one of the nine is at the *light* end and
/// none at the heavy end. Below about a fifth of the cell's ink, ASCII is a
/// crowd of one-mark glyphs, and the repertoire runs out of distinguishable
/// weights long before it runs out of characters. A ramp wanting sixteen
/// *reliably* separated bands would have to be a font, not a character set.
const DEFAULT_GLYPHS: &str = " '.-:=;*+X#O%&M@";

/// The value the glyph ramp is indexed by, remapped from the field's own value.
///
/// # What is wrong with indexing the ramp on the raw value
///
/// It is not that the mapping is wrong. It is *well* ordered: a higher field
/// value still gets a denser character, and every ordering property in the test
/// module survives whatever is done here. What is wrong is the **calibration**,
/// and the measurement is blunt. Over 200 frames at 80x24 the ten-step ramp put
/// `;` and `+` together on 49% of the screen and `@` on **0.00%**: the brightest
/// glyph in the set was never drawn, not once. At 200x50, where the field is
/// better sampled, `+` still took 23.5% and `@` 0.20%. The cause is that a sum
/// of a few sines is a *bell*: its value is concentrated near the middle of its
/// range, so a ramp divided evenly in value spends most of its steps on
/// distinctions the eye cannot find and leaves its extremes unreachable.
///
/// The field's measured quantiles at 1/16 intervals, at 200x50:
///
/// ```text
/// 0.237 0.295 0.337 0.372 0.402 0.431 0.459 0.487 0.515 0.544 0.573 0.604
/// 0.639 0.683 0.737
/// ```
///
/// **Fitted over ten seeds rather than one**, and that is the whole difference
/// from the fit this replaced. [`FieldTuning`] draws the five spatial divisors
/// and the five starting phases from the seed, and
/// [`Config::randomise_seeds`](crate::config::Config::randomise_seeds) hands
/// every launch whose seed is unset a fresh one -- so *every launch has a
/// slightly different field* and a table fitted to one of them is fitted to a
/// launch nobody chose. These are the quantiles of six million samples pooled
/// over seeds 0, 1, 7, 13, 42, 99, 1234, 5150, 65535 and 987654321.
///
/// **The last boundary is not a pooled quantile**, and the reason is that a
/// table's job is the *worst* launch while a pooled quantile is fitted to the
/// median one. Measured over those ten seeds the pooled quantiles leave the
/// sparsest step of the unluckiest launch at 2.37% and let one seed's busiest
/// step reach 13.2%. Lowering the top boundary to 0.737 moves 1.1 points of area
/// off the top step and onto its neighbour, which takes the worst launch's floor
/// to **2.63%** and its ceiling to 13.9%. It is the one number here that is not
/// a measured quantile, and it is a calibration decision rather than a
/// measurement wearing one.
///
/// The previous table was fitted to a field of four sines, and the field's
/// distribution is measurably different now that there are five: a sum of five
/// is less peaked than a sum of four, so the field reaches further at both
/// ends. Carried onto the new field unchanged it leaves the worst of those ten
/// launches with a sparsest step of **1.50%**, against 2.63% for this one, and
/// at the default seed alone it put 9.05% of the screen on `:` and 1.47% on
/// `@` -- a monotone drift down the whole ramp which stayed inside the
/// histogram test's 3% floor without being a good calibration.
/// `the_calibration_holds_across_seeds_not_just_the_default_one` is the
/// assertion, and this table's shares at that same seed are 7.86% and 2.57%.
///
/// The five-term distribution is also much less sensitive to the terminal's
/// size, which was not true of the four-term one. Pooled over nine seeds the
/// 80x24 quantiles run 0.233 to 0.724 against 0.237 to 0.743 at 200x50 -- a
/// gap of 0.004 at the bottom where the old field's was 0.20 -- so the
/// small-terminal case is a smaller correction than it used to be. See
/// `a_short_terminal_thins_the_ends_of_the_ramp` for what is still asserted
/// there.
///
/// # Why a table and not an exponent
///
/// A gamma curve was tried and it is the wrong shape. `t^g` is convex or
/// concave, so it can only tilt a bell, not flatten it: swept over 0.6 to 2.0 at
/// 400x200 the best it managed was 13.6% in the busiest band, against 14.2% with
/// no curve at all, while the *sparsest* band stayed at 0.0% in nine of the
/// eleven exponents tried and 0.1% in the other two -- the field does not reach
/// there, and no power of it will. Flattening a bell needs a curve that is
/// *steep* in the middle and *shallow* at the ends, and a power curve is never
/// S-shaped. Putting the boundaries at the measured quantiles is exactly that
/// curve, and it is what [`VALUE_BOUNDARIES`] is.
///
/// # What it costs
///
/// - **The field's tails are compressed.** A value of 0.0 and a value of 0.24
///   now draw the same glyph, and 0.74 and 1.0 draw the same other one. That is
///   a real loss of contrast at the extremes, paid for in the middle: two
///   characters that used to cover half the screen between them now cover an
///   eighth of it, and the busiest character covers 6.4% against 29.1%. A region
///   of the field reads as banded rather than as a wash. Banded is what "more
///   different characters" asks for, and it is a change of look rather than a
///   fix, so it is worth knowing that it is one.
///
/// - **The heavy end is no longer a tail in *area*,** which the ten-step note on
///   [`DEFAULT_GLYPHS`] relied on. `@` now covers a sixteenth of the field rather
///   than none of it. It is still a tail in *value*: reaching it takes a value
///   above 0.743, which is where the field's bright cores are.
///
/// - **The colour channel is deliberately left alone.** A cell's colour is
///   `palette[plasma + offset]` on the raw value, and remapping that too would
///   change which colour a given brightness gets for no gain in variety -- the
///   hue wheel is already continuous, so there is nothing to even out. The two
///   channels therefore carry *different* transforms of one field, which is
///   visible: a cell and its colour no longer step together. It is the right
///   trade, and it is a trade.
///
/// # Where the boundaries are not
///
/// They are fitted at 200x50, and 400x200 agrees with them to within 0.006 at
/// every one of the fifteen -- which the measurement in
/// `every_glyph_of_the_default_ramp_carries_its_share_of_the_screen` confirms
/// from the other end, by finding every step within a few tenths of a percent
/// of a sixteenth there too.
///
/// **80x24 does not agree, and cannot.** Reaching the field's extreme values is
/// a coincidence in five variables at once, so a small screen does not sample
/// them often enough: over 460,800 samples at 80x24 the field spans 0.017 to
/// 0.994, against 0.003 to 0.996 at 200x50. The bottom of the ramp therefore
/// comes out *thin* rather than empty at that size, and no static table can do
/// better, because the missing mass is in the field and not in the mapping. See
/// `a_short_terminal_thins_the_ends_of_the_ramp` for what *is* asserted there.
/// It used to be a much worse gap than it is: with five terms the field's
/// quantiles at 80x24 sit within 0.02 of the 200x50 ones at both ends, against
/// a 0.20 gap at the bottom when there were four.
const VALUE_BOUNDARIES: [f64; 15] = [
    0.237, 0.295, 0.337, 0.372, 0.402, 0.431, 0.459, 0.487, 0.515, 0.544, 0.573,
    0.604, 0.639, 0.683, 0.737,
];

/// The remap itself: the glyph ramp's own value for a field value.
///
/// Monotonically non-decreasing, and *that* is the whole contract. Every ordering
/// claim about this effect -- higher value means denser ink, the ramp never runs
/// backwards, the darkest and brightest cells differ -- is a statement about the
/// order of this function, and none of them depends on it being linear. A
/// non-monotone curve here would break all of them at once, which is why the
/// shape is a table of measured numbers rather than an expression.
///
/// `VALUE_BOUNDARIES[i]` maps to `(i + 0.5) / 15`, which is the middle of ramp
/// step `i`, and the two ends map to themselves. The alignment is the whole
/// trick and getting it wrong is invisible: a first version mapped each boundary
/// to the *far* edge of its step, `(i + 1) / 15`, which is one half-step out of
/// line. It still produced sixteen distinct glyphs and still satisfied every
/// ordering claim, and it starved the two ends -- the space drew 1.28% of the
/// field where every other step drew about 6.3%, and the top step drew 8.93%,
/// because each segment then straddled a band boundary instead of filling one.
/// With each segment filling exactly one band, every step takes a sixteenth.
///
/// The ends are half-width in output and a full chunk in input, while the middle
/// is full-width in both, so the curve's slope is about fifteen times shallower at
/// the extremes than in the middle. That is the point: the field's mass is in the
/// middle, and an S-curve is the only shape that flattens a bell.
///
/// Cost is a binary search over fifteen entries and one division, per cell,
/// which is not what a terminal screensaver should be spending its frame on but
/// is also a good deal cheaper than the four sines and a square root that produce
/// the value it is applied to.
/// Buckets in [`STEP_LUT`]. Sixteen bits.
///
/// A power of two so the bucket index is a shift rather than a division, and
/// fine enough that quantisation is invisible: the boundaries are about 1/16
/// apart, so this resolves them to five significant figures.
const STEP_LUT_LEN: usize = 1 << 16;

/// The step index for each of [`STEP_LUT_LEN`] evenly spaced inputs.
///
/// Built once from [`VALUE_BOUNDARIES`], which is the only place the
/// calibration is written. Sixteen kilobytes of `u8`, which is cheaper than the
/// branch mispredicts it replaces.
static STEP_LUT: LazyLock<[u8; STEP_LUT_LEN]> = LazyLock::new(|| {
    let mut table = [0u8; STEP_LUT_LEN];
    for (bucket, slot) in table.iter_mut().enumerate() {
        let value = bucket as f64 / (STEP_LUT_LEN - 1) as f64;
        *slot = VALUE_BOUNDARIES.partition_point(|bound| *bound <= value) as u8;
    }
    table
});

/// One entry per ramp step: the input segment's low edge, the reciprocal of its
/// width, and the output segment it maps onto.
#[derive(Clone, Copy)]
struct Segment {
    low: f64,
    /// `1 / (high - low)`, or zero for a zero-width input segment.
    inv_span: f64,
    out_low: f64,
    out_span: f64,
}

/// The sixteen segments, derived from [`VALUE_BOUNDARIES`].
///
/// `GlyphRamp::index_for` is `round(f * 15)`, so step `index` is the interval
/// from `(index - 0.5) / 15` to `(index + 0.5) / 15`. Step 0's lower edge and
/// step 15's upper edge fall outside `0..=1` and cannot be reached, which is why
/// those two come out half-width rather than full-width.
static SEGMENTS: [Segment; 16] = {
    let mut built = [Segment {
        low: 0.0,
        inv_span: 0.0,
        out_low: 0.0,
        out_span: 0.0,
    }; 16];
    let mut index = 0;
    while index < 16 {
        let (low, high) = match index {
            0 => (0.0, VALUE_BOUNDARIES[0]),
            15 => (VALUE_BOUNDARIES[14], 1.0),
            _ => (VALUE_BOUNDARIES[index - 1], VALUE_BOUNDARIES[index]),
        };
        let (out_low, out_high) = match index {
            0 => (0.0, 0.5 / 15.0),
            15 => (14.5 / 15.0, 1.0),
            _ => ((index as f64 - 0.5) / 15.0, (index as f64 + 0.5) / 15.0),
        };
        built[index] = Segment {
            low,
            // A zero-width input segment is reachable only if two boundaries are
            // equal, and a hand-edited table can do that. Zero rather than a
            // branch, and rather than infinity: `0 * inf` is NaN, and `f64::clamp`
            // propagates a NaN *value* rather than replacing it, so infinity here
            // would poison the glyph index instead of flooring the fraction.
            inv_span: if high > low { 1.0 / (high - low) } else { 0.0 },
            out_low,
            out_span: out_high - out_low,
        };
        index += 1;
    }
    built
};

fn glyph_value(value: f64) -> f64 {
    let value = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };

    // Which step the value falls in, from a table rather than a search.
    //
    // This is on the hot path -- once per cell, so eighty thousand times at
    // 400x200 -- and `partition_point` over fifteen boundaries is a binary
    // search: four unpredictable branches per cell, none of them vectorisable.
    // Measured, it cost 310 us of the render, which is 32% of the total and the
    // difference between this effect being inside the 2 ms budget at 400x200 and
    // outside it.
    //
    // The table is *derived* from `VALUE_BOUNDARIES` rather than being a second
    // place the calibration is written down, so the boundaries remain the single
    // source of truth. Sixteen bits of resolution is 7.6e-6, roughly eight
    // thousand times finer than the gap between two boundaries, so the only
    // values it can move are those sitting within 0.0008% of a boundary -- and
    // because the quantisation is monotone, the output stays monotone, which is
    // the property the tests actually check.
    let bucket = ((value * (STEP_LUT_LEN - 1) as f64).round() as usize)
        .min(STEP_LUT_LEN - 1);
    let index = STEP_LUT[bucket] as usize;

    // The segment's four constants, precomputed. They depend only on which step
    // the value fell in, and the original code recomputed all four per cell --
    // two `match`es on the same index and a division, eighty thousand times.
    //
    // The reciprocal is the point. `fraction` was `(value - low) / (high - low)`,
    // and a divide is fifteen to twenty cycles where a multiply is one, so the
    // division was plausibly most of what was left in the render. Storing
    // `1 / (high - low)` moves it out of the loop entirely.
    let segment = SEGMENTS[index];
    let fraction = ((value - segment.low) * segment.inv_span).clamp(0.0, 1.0);
    segment.out_low + fraction * segment.out_span
}

/// Builds the glyph ramp, never empty and never a character that could shear a
/// cell-indexed grid.
///
/// A ramp that filters down to nothing is replaced with [`DEFAULT_GLYPHS`]
/// rather than with `GlyphRamp`'s own ASCII fallback, because that fallback is
/// SHADE -- non-monotonic in ink, and so the wrong default for the one thing
/// this ramp is for.
fn glyph_ramp(configured: &str) -> GlyphRamp {
    let glyphs: Vec<char> = configured
        .chars()
        .filter(|glyph| {
            !glyph.is_control() && glyph_ramp::is_ambiguous_or_narrow(*glyph)
        })
        .collect();

    if glyphs.is_empty() {
        GlyphRamp::from_text(DEFAULT_GLYPHS)
    } else {
        GlyphRamp::new(glyphs)
    }
}

/// A terminal cell is taller than it is wide, so a shape measured in cell
/// units is an ellipse on screen. DejaVu Sans Mono is about 1 : 1.2 and the
/// usual ideal-square assumption is 1 : 2; 1.2 is the conservative middle,
/// because correcting by 2 would make every feature in the field twice as tall
/// as it is wide on the fonts most people actually have.
///
/// The whole field is measured in this corrected space, which is what makes
/// the radial terms circles on screen. See `the_field_is_not_stretched_
/// vertically` for the measurement.
const CELL_ASPECT: f64 = 1.2;

/// One spatial divisor and the band the seed is allowed to draw it from.
///
/// # Why the band has hard edges
///
/// A divisor is a *spatial frequency*, and both of its degenerate ends are
/// degenerate in opposite directions. Near zero the term is a flat wash -- one
/// broad, almost constant bulge across the whole screen. Near the screen's own
/// width it is a single stripe, because there is not room for a second. Both
/// are the field drawing nothing, and neither is a "randomiser" failure that
/// shows up in a test: the picture still renders, it is just not plasma.
///
/// So the band is stored as a centre and a half-width rather than as two ends,
/// the drawing is a *multiplication* of the centre by a factor in
/// `1 - jitter ..= 1 + jitter` rather than a draw over an open range, and
/// `the_seeded_divisors_stay_inside_their_bands` measures 200 seeds and holds
/// every one of the twenty draws to its own edges. A factor is what makes the
/// floor: `rng.random_range(0.0..base)` would have a floor of zero and a draw
/// of `0.04` would be a legal, invisible, unarguable outcome.
///
/// The test recomputes the edges from `center` and `jitter` rather than reading
/// them off the type. There are no accessors to read: a `low()` here would be
/// used by nothing but the test, and the test asking the code under test where
/// its own boundaries are is a test that cannot fail.
#[derive(Debug, Clone, Copy)]
struct DivisorBand {
    /// What a fresh effect centres on.
    center: f64,
    /// Half-width of the band, as a fraction of `center`.
    jitter: f64,
}

impl DivisorBand {
    const fn new(center: f64, jitter: f64) -> Self {
        Self { center, jitter }
    }

    /// Draws one divisor.
    ///
    /// Multiplicative and not additive because a divisor is a *period*: a
    /// wobble of a fixed size in cells means something completely different at
    /// 4 cells and at 16, and a proportional band keeps the number of bands on
    /// screen the same fraction of the screen at both ends. Additive jitter on
    /// a small divisor produces a different number of features than the same
    /// jitter on a large one, which is the opposite of what a band is for.
    fn draw(&self, rng: &mut EffectRng) -> f64 {
        use rand::RngExt;
        self.center * rng.random_range(1.0 - self.jitter..=1.0 + self.jitter)
    }
}

/// Divisor on the wave term: the spacing of the vertical bands, in cells.
///
/// The widest of the five, because it is the term that has to survive being
/// scaled by `spatial_scale` and still read as bands rather than as texture.
/// At 200 columns the band is 15.7 to 30.0, so 7 to 13 full cycles across the
/// screen; a narrower one and the field's coarsest structure stops being
/// visible as structure at all.
const WAVE_DIVISOR: DivisorBand = DivisorBand::new(8.0, 0.30);

/// Divisor on the sweep term: the spacing of the horizontal bands.
///
/// Wider than [`WAVE_DIVISOR`] and deliberately so, twice over. It was 16.0
/// against the wave's 8.0 before, and the ratio is what keeps the field's
/// vertical spatial rate down: measured over a 400x200 screen, the field
/// changes 0.79x as fast going down a column as across a row, and this divisor
/// is the reason it is not 1.2x. A field that changes as fast vertically as
/// horizontally is *isotropic*, and `the_field_is_not_stretched_vertically` is
/// about the opposite failure -- but the honest reading of that test's band is
/// that 1.2 is the isotropic value and this divisor is what keeps the measured
/// figure below it.
///
/// The seed's band is 11.2 to 20.8, which at 200x50 is 2.9 to 5.1 cycles of
/// horizontal band over the screen's 60 corrected units of height.
const SWEEP_DIVISOR: DivisorBand = DivisorBand::new(16.0, 0.30);

/// Divisor on the first ring term: the spacing of the concentric bands.
///
/// The narrowest of the five, and the reason the field has a *scale* to it. The
/// radial extent of a 200x50 screen is about 104 corrected units, so 3.0 to 5.0
/// puts 21 to 35 rings along the diagonal -- enough that the rings read as
/// rings and not as one gradient.
const RING_DIVISOR: DivisorBand = DivisorBand::new(4.0, 0.25);

/// Divisor on the second ring term, deliberately **not** [`RING_DIVISOR`]'s
/// multiple.
///
/// This is the third of the three structural faults in the field, and it is the
/// one that no test in this file was watching for. The two ring terms used to
/// be `sin(r/4)` and `sin(r/8)` -- the same pattern at exactly 2 : 1, a
/// harmonic. Two sines of one argument in a harmonic ratio are not two waves:
/// they are one wave and its second harmonic, they add *coherently* at every
/// radius, and their sum is a single clean beat with a smooth envelope. Which
/// is exactly what the field looked like: hard concentric bands, evenly spaced,
/// marching outwards, no interference anywhere in them.
///
/// 5.5 against 4.0 is a ratio of 11 : 8. Not harmonic, so the two beat against
/// each other at a visibly different rate and the rings interfere rather than
/// reinforce. It is still a rational ratio, so the pair is jointly periodic in
/// the radius -- at 88*pi, about 276 corrected units, which is more than the
/// screen's own diagonal, so the repeat is off the visible field entirely. The
/// 2 : 1 pair's joint period was 16*pi = 50.3 units, a shade under *half* the
/// 200x50 diagonal and 12 units under the 400x200 one, so its repeat was on
/// screen. That is the whole difference: not that the pattern cannot repeat,
/// but that the repeat is now further away than the screen.
///
/// The band is 3.85 to 7.15, which keeps the ratio to [`RING_DIVISOR`]'s
/// between 11 : 8 and 2.39 : 1, and the low end of that range is deliberately
/// still not 2 : 1.
const BREATH_DIVISOR: DivisorBand = DivisorBand::new(5.5, 0.30);

/// Divisor on the diagonal term: the spacing of the 45-degree bands.
///
/// The fault this fixes is the one with no constant in front of it, because the
/// fix is a term rather than a number. Every term the field had was a function
/// of `x` alone, `y` alone, or `r` alone, so the field was built from
/// axis-aligned stripes and concentric rings and nothing else, and it read as
/// bands sliding past rather than as something with a grain. There is a
/// diagonal term in the canonical demoscene plasma for exactly that reason.
///
/// 12.0 puts 17 to 31 cycles across the 260 corrected units of `x + y` that a
/// 200x50 screen spans. Wider than that and the diagonal stops competing with
/// the wave term, which is at 8.0 and therefore coarser; narrower and it
/// becomes the finest structure on the screen and the field's grain turns to
/// noise. The band's low end, 8.4, is *coarser* than the wave's centre on
/// purpose, so that across the band there are seeds where the diagonal is the
/// coarsest term in the field and seeds where it is not.
const DIAGONAL_DIVISOR: DivisorBand = DivisorBand::new(12.0, 0.30);

/// How far the sweep term's phase travels, in bands of `y / SWEEP_DIVISOR`.
///
/// **This is the number the user's second complaint is about**, and it was 2.0.
///
/// The sweep term is a field of *horizontal* bands whose phase is dragged up
/// and down the screen over time, so its depth is a vertical advection speed and
/// nothing else measures it. At 2.0 the phase swings the full two cycles of
/// `y / 16`, which is 32 corrected units -- more than half the 60 units of
/// height a 200x50 screen has. Half the field's amplitude was travelling
/// vertically, upwards and downwards, once every `2*pi / 0.866` = 7.3 units of
/// `plasma.time` (14.5 seconds of wall clock), and no other term in the field
/// can compete with a motion that size: it is the direct cause of "it just
/// goes up and down", and it is why the *first* fix -- making the time
/// frequencies irrational -- did not touch the complaint. That fix was about
/// repetition and this is about character, and the two are different problems.
///
/// At 0.25 the same term carries the bands 4 corrected units, or three rows,
/// each way -- about one and a half rows a second of wall clock. Measured as
/// the least-squares advection velocity the picture has downwards against the
/// one it has sideways, the field went from 5.81 to 0.98 at 200x50 and from
/// 3.46 to 1.13 at 80x24. See `the_field_does_not_go_up_and_down`.
///
/// **What was given up, because something had to be.** The old term was the
/// fastest-moving thing in the field and a good part of the picture's perceived
/// activity came from it. The field's *net drift* went from 3.14 cells a second
/// to 0.75, which is three quarters of the picture's motion; the mean |change|
/// at a cell over a second of wall clock, which is what churn actually feels
/// like, is about 60% of what it was. So the picture still boils locally and
/// simply no longer marches. The churn it lost is partly recoverable from the
/// seed, which is the other half of this round -- a launch that draws a small
/// [`DIAGONAL_DIVISOR`] is livelier than one that draws a large one.
const SWEEP_DEPTH: f64 = 0.25;

/// How far the first ring term's phase travels, in rings of `r / RING_DIVISOR`.
///
/// Zero before, and a term that does not move is a term whose half the radial
/// structure on the screen is standing still while the other half drifts. That
/// reads as a picture with a frozen component in it, and it is also the term
/// that makes the *pattern* of rings legible in the first place -- an
/// interference pattern needs both of its waves to breathe or it is just a
/// gradient that happens to be curved.
const RING_DEPTH: f64 = 0.9;

/// How far the second ring term's phase travels.
///
/// Was 1.0 against a divisor of 8.0, so a radial speed of 4.0 units per unit of
/// `plasma.time`. [`BREATH_DIVISOR`] is now 5.5 and the divisor's band runs
/// down to 3.85, so 1.0 would have put the speed at 7.3 and the term would have
/// been the fastest-moving thing in the field. Scaled to hold the speed where
/// it was rather than to keep the number, which is the whole point of writing
/// the product down.
const BREATH_DEPTH: f64 = 0.55;

/// How far the diagonal term's phase travels, in cycles of `x + y`.
///
/// 0.6 over a divisor of 12.0 is 8.0 corrected units of `x + y` per unit of
/// `plasma.time`, which is 4.0 cells across and 3.3 rows down. That is the
/// fastest motion left in the field, and it is *diagonal* motion, which is the
/// point: a drift along `x + y` has a vertical component and a horizontal one
/// of nearly the same size, so it contributes to both directions instead of
/// only to the one the user complained about. On screen the two are not the
/// same size, because `y` is already in corrected units: one cell across is 1.0
/// of `x` and one row down is 1.2 of `y`, so a 45-degree drift in field space
/// is 40 degrees on the screen and the diagonal term is the only term in the
/// field that leans *away* from vertical.
///
/// **1.0 was tried and measured, and it is the wrong value.** The diagonal is
/// the only term that moves in both directions at once, so raising its depth is
/// the obvious way to buy liveliness back -- and it does, raising the field's
/// net drift by about a fifth. It also raises the vertical-to-horizontal drift
/// ratio, from 0.98 to 1.8 at 200x50, because the vertical component of a
/// diagonal drift is what a vertical-to-horizontal measure is most sensitive
/// to. At 0.6 the ratio is under 1 -- the field's drift is, on net, sideways --
/// and the picture is still the liveliest it has been since the sweep depth
/// came down. The liveliness is worth having and 1.8 is not.
const DIAGONAL_DEPTH: f64 = 0.6;

/// # The time frequencies, and why the field never repeats
///
/// The user said "right now it just goes up and down; it's not something new"
/// and "make it unique each time and continuous, not a loop". The loop was not
/// a stylistic matter. The four time terms ran at `now/2`, `now`, `0` and
/// `now/4`, so their periods were `4*pi`, `2*pi`, infinite and `8*pi`, and the
/// field's period was the least common multiple of those: **exactly `8*pi` =
/// 25.1 units of `plasma.time`**, which at the default `time_scale` of 0.5 is
/// 50 seconds of wall clock. After that the screen was not merely similar to
/// what it had been, it was *identical*, cell for cell and colour for colour,
/// forever.
///
/// The fix is the frequencies rather than anything about how they are combined.
/// A sum repeats only if a single `T` advances *every* term by a whole number
/// of periods, so what matters is the **ratios** between the frequencies and not
/// their size: rational ratios have a common period, and mutually irrational
/// ratios do not. The five below are `{1, sqrt(2), sqrt(3), sqrt(5),
/// sqrt(7)}` over a common two, and every pair has an irrational ratio, because
/// the quotient of the square roots of two distinct squarefree integers is
/// always irrational. So there is no `T > 0` for which all five come back to
/// where they started, and the field has no period at all.
///
/// Two consequences worth being straight about, because neither is free:
///
/// - **The composition does not rescue a rational set.** Each term is
///   `sin(spatial - c * sin(f*t))` rather than a bare sine, so a term's
///   harmonics sit at *integer multiples* of its own `f`. That makes the field's
///   frequency content the additive group generated by the `f`, which still
///   contains each `f` itself, so a common period would still have to satisfy
///   `f*T = 2*pi*n` for all of them. Wrapping the phase in a sine delays the
///   repeat; it does not remove it.
///
/// - **The speeds barely moved, and that was the design constraint.** Dividing
///   the whole set by two keeps the ordering the field already had -- the first
///   ring term's drift slowest, the wave term's second, the sweep term third --
///   and keeps the total churn within 10% of what it was. Measured as the
///   root-sum-square of `c*f` over the terms, which is what drives mean
///   |change| per second, the five below score 1.06x the three they were
///   extended from. The obvious alternative, `{1, sqrt(2), sqrt(3)}` unscaled,
///   scores 1.67x.
///
/// The two added are the *fastest* pair, which is deliberate. A new term on a
/// slow frequency is nearly invisible, and a field whose only visible motion is
/// the slow one is a field that drifts rather than a screensaver. The diagonal
/// and the second ring term are the two whose motion reads most clearly --
/// one because it crosses the whole screen on a diagonal, one because a radial
/// expansion is the change the eye is best at reading -- so those are the two
/// that carry new frequencies.
const TIME_FREQ_RIPPLE: f64 = 0.5;
const TIME_FREQ_WAVE: f64 = SQRT_2 / 2.0;
const TIME_FREQ_SWEEP: f64 = SQRT_3 / 2.0;
const TIME_FREQ_DIAGONAL: f64 = SQRT_5 / 2.0;
const TIME_FREQ_BREATH: f64 = SQRT_7 / 2.0;

/// The five terms' spatial divisors and starting phases, drawn from the seed.
///
/// # What is randomised and what is not
///
/// The five divisors and the five starting phases. **Not** the time
/// frequencies, the sweep depth, or the term structure itself, and each of
/// those exclusions is deliberate:
///
/// - The time frequencies are what make the field aperiodic, and the period
///   search in `the_field_does_not_come_back_to_where_it_was` reads them as
///   named constants. Jittering them per seed would make that test assert
///   something about a set it can no longer name, and -- worse -- a *random*
///   set of five frequencies has a small but real chance of drawing two within
///   a rational ratio, which would quietly reintroduce a period. The set is
///   the property; it is not per-launch decoration.
///
/// - The depths are the *character* of the field, and the character is what
///   this round was about. [`SWEEP_DEPTH`] is 0.25 because at 2.0 the field
///   went up and down; drawing it from a band would mean every launch had a
///   different amount of the exact fault the user reported.
///
/// # Why phases as well as divisors
///
/// Divisors alone give two launches the same *texture* and a different
/// alignment, and a plasma that is recognisably the same field with the bands
/// in a different place is not a different launch. A per-term phase offset is
/// a constant added inside the sine, so it moves that term's bands without
/// touching its spacing, its speed, or the field's periodicity -- which is
/// exactly the property wanted from a randomisation that is not allowed to
/// reintroduce a loop.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FieldTuning {
    wave_divisor: f64,
    sweep_divisor: f64,
    ring_divisor: f64,
    breath_divisor: f64,
    diagonal_divisor: f64,
    /// One starting phase per term, in `0..2*pi`, in the order the terms are
    /// summed in [`FieldTuning::value`].
    phase: [f64; 5],
}

impl FieldTuning {
    /// Draws a field's tuning from its seed.
    ///
    /// The five divisors are drawn first, in the order they are summed, and the
    /// phases after, so a seed maps to one tuning and one tuning only. Two
    /// effects built from the same options therefore draw the same picture,
    /// which is what `tests/effect_contracts.rs` compares them on.
    fn from_seed(seed: u64) -> Self {
        use rand::RngExt;

        let mut rng = seeded_rng(seed, "plasma");
        Self {
            wave_divisor: WAVE_DIVISOR.draw(&mut rng),
            sweep_divisor: SWEEP_DIVISOR.draw(&mut rng),
            ring_divisor: RING_DIVISOR.draw(&mut rng),
            breath_divisor: BREATH_DIVISOR.draw(&mut rng),
            diagonal_divisor: DIAGONAL_DIVISOR.draw(&mut rng),
            phase: std::array::from_fn(|_| rng.random_range(0.0..2.0 * PI)),
        }
    }

    /// Vertical bands, their phase travelling sideways.
    ///
    /// A function of `x` alone, so it is a set of vertical stripes and nothing
    /// else. Its phase is a cosine rather than a sine so that it is at its
    /// fastest at `now = 0`, which is what puts the field's horizontal motion
    /// at full speed on the first frame rather than starting from a dead stop.
    fn wave(&self, x: f64, now: f64) -> f64 {
        ((x / self.wave_divisor - (now * TIME_FREQ_WAVE).cos()) + self.phase[0])
            .sin()
    }

    /// Horizontal bands, their phase travelling downwards.
    ///
    /// The term the user's complaint was about. See [`SWEEP_DEPTH`] -- this was
    /// moving the field's half-amplitude vertically by 32 corrected units and
    /// now moves it by four.
    fn sweep(&self, y: f64, now: f64) -> f64 {
        ((y / self.sweep_divisor - (now * TIME_FREQ_SWEEP).sin() * SWEEP_DEPTH)
            + self.phase[1])
            .sin()
    }

    /// Rings, breathing in and out about the screen centre.
    fn ring(&self, radius: f64, now: f64) -> f64 {
        ((radius / self.ring_divisor - (now * TIME_FREQ_RIPPLE).sin() * RING_DEPTH)
            + self.phase[2])
            .sin()
    }

    /// A second set of rings, at a different rate and a different phase.
    ///
    /// Deliberately out of harmonic relationship with [`ring`]. See
    /// [`BREATH_DIVISOR`]. A sine of the same cosine would be one wave twice
    /// and a phase offset apart; a sine of a *different* frequency is a second
    /// wave, and the pair interfere instead of adding.
    fn breath(&self, radius: f64, now: f64) -> f64 {
        ((radius / self.breath_divisor
            + (now * TIME_FREQ_BREATH).cos() * BREATH_DEPTH)
            + self.phase[3])
            .sin()
    }

    /// Bands on the 45-degree diagonal, phase travelling along it.
    ///
    /// The term with no predecessor. `x + y` rather than `x - y` so the bands
    /// run from the top-left to the bottom-right, which is the diagonal the eye
    /// reads as "down and across" and therefore the one that does not read as
    /// vertical motion in either half of the screen.
    fn diagonal(&self, x: f64, y: f64, now: f64) -> f64 {
        (((x + y) / self.diagonal_divisor
            - (now * TIME_FREQ_DIAGONAL).cos() * DIAGONAL_DEPTH)
            + self.phase[4])
            .sin()
    }

    /// The field at one point, normalised to `0.0..=1.0`.
    ///
    /// Five terms, each `128 + 128 * sin(..)` and so each running 0 to 256 with
    /// a mean of 128. The sum of five has a mean of 640, and dividing by the
    /// number of terms and then by 256 puts the level where the sines cancel
    /// back at 0.5 -- which is why the divisor is the term count and not a
    /// constant: a fifth term added without dividing by five would have put that
    /// level at 0.64 and slid the calibration [`VALUE_BOUNDARIES`] is fitted to.
    ///
    /// The divisor moves with the count so the property cannot silently stop
    /// holding, and `the_field_is_the_five_terms_normalised` holds it against
    /// the terms themselves rather than against the field's own statistics --
    /// which is the only way it can be held, and that test carries the
    /// measurement of why.
    ///
    /// See the [AWK script formula](https://rosettacode.org/wiki/Plasma_effect#AWK),
    /// which this is, and the note on why the result is a float rather than a
    /// byte.
    fn value(&self, x: f64, y: f64, now: f64, w: f64, h: f64, scale: f64) -> f64 {
        // Both radial terms are anchored at the screen centre, and `x`, `y` and
        // `radius` all arrive in the same corrected space. The fourth term used
        // to be anchored at the origin, which is what made the *bottom* of the
        // screen the busy part: its vertical phase gradient is `y / r`, zero
        // along the top edge and maximal along the bottom, so it added to the
        // third term's gradient at the bottom and subtracted at the top.
        // Anchoring both at the centre makes each gradient proportional to
        // `(y - h/2) / r`, so the pair reinforces in the middle of the screen
        // and vanishes at the top and bottom edges equally. There is no bottom
        // edge to be special any more; that is a property of the geometry, not
        // a clamp.
        let radius = ((x - w / 2.0).powi(2) + (y - h / 2.0).powi(2)).sqrt();

        // `scale` multiplies each term's *spatial* argument and nothing else.
        // Folded in here rather than passed to five methods, because it is one
        // multiplication of the two coordinates and the radius -- three of them
        // rather than five, and the radius has to be scaled too or the radial
        // terms would be the only ones `spatial_scale` did not reach.
        let value = (128.0
            + (128.0 * self.wave(x * scale, now))
            + 128.0
            + (128.0 * self.sweep(y * scale, now))
            + 128.0
            + (128.0 * self.ring(radius * scale, now))
            + 128.0
            + (128.0 * self.breath(radius * scale, now))
            + 128.0
            + (128.0 * self.diagonal(x * scale, y * scale, now)))
            / 5.0;

        // A rail is a moving hard iso-line: on one side of it a cell is a
        // clipped flat value and on the other it is riding the field, so
        // everything on the clipped side animates only as the line sweeps past
        // it. Clamping to a float makes the rail unreachable, and it hands the
        // glyph ramp a real 0..1 value instead of a quantised byte. The
        // function used to end in `value as u8`, and while that cast happens to
        // saturate rather than wrap, nothing about the formula stops a future
        // edit to a coefficient from pushing the sum outside `0..=256` and
        // pinning a whole region of the screen against one end of the range.
        (value / 256.0).clamp(0.0, 1.0)
    }
}

pub struct Plasma {
    pub screen_size: (u16, u16),
    options: PlasmaOptions,
    canvas: Canvas,
    time: f64,
    palette: Vec<style::Color>,
    ramp: GlyphRamp,
    /// The field's divisors and phases, drawn once from `options.seed`.
    ///
    /// Built in [`Plasma::new`] and never rebuilt: the seed cannot change
    /// without a new effect, and a field that re-drew its own structure on a
    /// resize would be a different picture on the same terminal. A resize
    /// rebuilds the *canvas* and zeroes the clock and leaves this alone.
    tuning: FieldTuning,
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
            &self.ramp,
            &self.tuning,
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
        let ramp = glyph_ramp(&options.glyphs);
        let tuning = FieldTuning::from_seed(options.seed);

        Self {
            screen_size,
            options,
            canvas,
            time,
            palette,
            ramp,
            tuning,
        }
    }

    /// Builds the colour wheel the field is mapped onto.
    ///
    /// A hue wheel rather than three independent sine waves. The old ramp was
    /// `128 + 128*sin(...)` per channel at three different periods, so every
    /// channel oscillated about mid-grey with the full +/-128 swing available:
    /// whatever the field value, the sum of the three was high, and the screen
    /// came out a pale, desaturated wash with no dark end anywhere in it.
    ///
    /// Going round the hue circle fixes both halves of that. Full saturation
    /// means at least one channel is always 0, so no entry is a near-grey, and
    /// the value is modulated from `PALETTE_MIN_VALUE` to 1 across the lap, so
    /// the wheel passes through a genuine dark. `sin(PI * hue)` is used for that
    /// modulation because it is 0 at both ends of the lap and 1 in the middle,
    /// so the wheel wraps seamlessly as the offset cycles -- which it has to,
    /// since the index is taken modulo the palette length.
    fn generate_palette() -> Vec<style::Color> {
        (0..PALETTE_LEN)
            .map(|i| {
                let hue = i as f64 / PALETTE_LEN as f64;
                let value = PALETTE_MIN_VALUE
                    + (1.0 - PALETTE_MIN_VALUE) * (PI * hue).sin().abs();
                let (r, g, b) = Self::hsv_to_rgb(hue, PALETTE_SATURATION, value);
                style::Color::Rgb { r, g, b }
            })
            .collect()
    }

    /// Standard HSV to RGB, for hue in 0..1. Returns channels that are always in
    /// range: the value is the largest of them and the third is always 0 at full
    /// saturation, so the `round` cannot land outside `0..=255`.
    fn hsv_to_rgb(hue: f64, saturation: f64, value: f64) -> (u8, u8, u8) {
        let sector = (hue * 6.0).floor();
        let f = hue * 6.0 - sector;
        let p = value * (1.0 - saturation);
        let q = value * (1.0 - f * saturation);
        let t = value * (1.0 - (1.0 - f) * saturation);

        let (r, g, b) = match (sector as i64).rem_euclid(6) {
            0 => (value, t, p),
            1 => (q, value, p),
            2 => (p, value, t),
            3 => (p, q, value),
            4 => (t, p, value),
            _ => (value, p, q),
        };

        // The three channels are 0..1, as HSV defines them, so they are scaled
        // here. Clamped as well, so a value a rounding step pushes past 1 cannot
        // truncate the way an unclamped `as u8` would.
        let to_byte =
            |channel: f64| (channel * 255.0).round().clamp(0.0, 255.0) as u8;
        (to_byte(r), to_byte(g), to_byte(b))
    }

    /// Repaints the field, one cell per plasma sample.
    ///
    /// The cell's colour *and* its glyph are read from the same value, which is
    /// the point: every cell used to be a hard-coded `*` that carried nothing,
    /// so the glyph channel was decoration and the field had one degree of
    /// freedom less than it looked like it had. They are read from *different
    /// transforms* of that value, which is [`glyph_value`]'s whole subject: the
    /// colour indexes the wheel on the raw value and the ramp indexes the
    /// calibrated one.
    #[allow(clippy::too_many_arguments)]
    fn update_plasma(
        size: (u16, u16),
        now: f64,
        color_speed: f64,
        spatial_scale: f64,
        palette: &[style::Color],
        ramp: &GlyphRamp,
        tuning: &FieldTuning,
        buffer: &mut Buffer,
    ) {
        let width = size.0 as usize;
        let height = size.1 as usize;
        let w = width as f64;
        let h = height as f64 * CELL_ASPECT;

        for y in 0..height {
            // Cells in, corrected units out. See `CELL_ASPECT`.
            let y_units = y as f64 * CELL_ASPECT;

            for x in 0..width {
                let value =
                    tuning.value(x as f64, y_units, now, w, h, spatial_scale);

                // Get color indices with time component. Wrapped rather than
                // cast-and-clamped: the offset grows without bound, and an
                // out-of-range index is a panic waiting for a long session.
                let color_idx = (value * (PALETTE_LEN - 1) as f64
                    + now * color_speed)
                    .rem_euclid(PALETTE_LEN as f64)
                    as usize;

                let cell_color = palette[color_idx];

                // `Attribute::Reset`, not `Attribute::Bold`. Every cell used to be
                // bold, and bold on a truecolor foreground is a rendering hint
                // that many terminals answer by brightening the colour -- which
                // is how a saturated cell ends up reading as white. The palette
                // is already saturated, so it does not need the hint, and with
                // the glyph now carrying brightness a brightening hint would
                // corrupt that too.
                let cell = Cell::new(
                    ramp.sample(glyph_value(value) as f32),
                    cell_color,
                    style::Attribute::Reset,
                );

                buffer.set(x, y, cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_palette_is_saturated_rather_than_a_pale_wash() {
        let palette = Plasma::generate_palette();

        for (i, color) in palette.iter().enumerate() {
            let style::Color::Rgb { r, g, b } = *color else {
                panic!("palette entry {i} is not a truecolor value: {color:?}");
            };
            let (hi, lo) = (i32::from(r.max(g).max(b)), i32::from(r.min(g).min(b)));

            // Every channel of the old ramp oscillated about 128 with the whole
            // +/-128 swing available, so entries came out as near-greys. At
            // index 16 it was 255,218,177 -- a beige. Chroma is the direct
            // measure of that, and it is what has to be there.
            assert!(
                hi - lo >= 64,
                "palette entry {i} is ({r}, {g}, {b}): a chroma of {} is a pale wash, \
                 not a colour",
                hi - lo
            );

            // And a dark end. `lo <= 128` also rules out any entry within a
            // small band of full brightness on all three channels, which is the
            // blown-out cell the pale ramp produced.
            assert!(
                lo <= 128,
                "palette entry {i} is ({r}, {g}, {b}): nothing in it is dark"
            );
        }
    }

    #[test]
    fn every_drawn_colour_comes_from_the_palette() {
        // The palette index is `plasma + now * color_speed`, and `now` grows
        // without bound, so it has to be wrapped rather than cast.
        let palette = Plasma::generate_palette();
        let mut plasma = Plasma::new(PlasmaOptions::default(), (24, 8));

        for frame in 0..3000u64 {
            plasma.time = frame as f64 * 0.5;
            assert_palette_colours(&plasma.get_diff(), &palette, frame);
        }
    }

    #[test]
    fn a_negative_time_still_cycles_the_palette() {
        // A hand-edited config can set a negative `time_scale`, and the offset
        // then runs backwards. Casting a negative float to `usize` saturates at
        // zero, so the old `as usize % 256` pinned every cell to entry 0 and the
        // effect froze on one colour instead of cycling.
        let mut plasma = Plasma::new(PlasmaOptions::default(), (24, 8));

        plasma.time = -500.0;
        let colours: HashSet<style::Color> = plasma
            .get_diff()
            .iter()
            .map(|(_, _, cell)| cell.color)
            .collect();

        assert!(
            colours.len() > 1,
            "a negative time froze the whole screen on one palette entry: \
             {colours:?}"
        );
    }

    fn assert_palette_colours(
        cells: &[(usize, usize, crate::buffer::Cell)],
        palette: &[style::Color],
        frame: u64,
    ) {
        for (_, _, cell) in cells {
            assert!(
                palette.contains(&cell.color),
                "frame {frame} drew {:?}, which is not a palette entry",
                cell.color
            );
        }
    }

    #[test]
    fn the_default_palette_speed_leaves_the_screen_alone_between_frames() {
        let options = PlasmaOptions::default();

        // How far the colour offset travels between two frames, in palette
        // entries. Every cell is `palette[plasma + offset]`, so this is also the
        // fraction of the wheel -- and therefore roughly the fraction of the
        // screen -- that has to be repainted.
        let per_frame = options.color_speed * options.time_scale / 60.0;
        assert!(
            per_frame < 1.0 / 24.0,
            "the palette moves {per_frame:.3} entries per frame, so about {:.0}% \
             of the screen changes colour every frame",
            per_frame * 100.0
        );
        // ...and the colour still visibly cycles. `per_frame` is entries per
        // *frame*, so the time to cross one entry is a frame over that; at the
        // default it is about half a second, which is plainly visible, and a full
        // lap of the wheel is a bit over two minutes.
        let seconds_per_entry = (1.0 / 60.0) / per_frame;
        assert!(
            seconds_per_entry < 1.0,
            "one palette step takes {seconds_per_entry:.1}s, which is too slow to see"
        );
    }

    #[test]
    fn plasma_cells_are_not_blanket_bold() {
        // Bold on a truecolor foreground is a brightening hint on many
        // terminals, which is how a saturated colour turns into a white one.
        let mut plasma = Plasma::new(PlasmaOptions::default(), (40, 12));
        let diff = plasma.get_diff();

        assert!(!diff.is_empty(), "nothing was drawn");
        for (x, y, cell) in diff {
            assert_eq!(
                cell.attr,
                style::Attribute::Reset,
                "cell ({x}, {y}) is still bold"
            );
        }
    }

    /// The glyph has to be a *function of the value*, not decoration.
    ///
    /// Every cell used to be a hard-coded `*`, so the glyph channel was a
    /// constant: it made the field look twice as detailed as it was, and any
    /// tuning of `spatial_scale` moved colour without moving character. Two
    /// cells with very different values must draw different characters, or the
    /// glyph is still not carrying anything.
    #[test]
    fn the_glyph_is_a_function_of_the_plasma_value() {
        let size = (120u16, 40u16);
        let mut plasma = Plasma::new(PlasmaOptions::default(), size);

        // Sorted by the value the cell was drawn from, so the extremes do not
        // depend on guessing which corner of the frame is the bright one.
        let by_value = drawn_by_value(&mut plasma, size);
        let lowest = by_value.first().expect("nothing was drawn");
        let highest = by_value.last().expect("nothing was drawn");

        assert_ne!(
            lowest.1, highest.1,
            "the dimmest cell ({:.3}) and the brightest ({:.3}) both drew {:?}, \
             so the glyph is not a function of the value",
            lowest.0, highest.0, lowest.1
        );

        // And at least four of the five ramp steps are in use, which is the
        // weakest version of "a function" that still means the glyph carries
        // something across a whole frame.
        let distinct: HashSet<char> = by_value.iter().map(|(_, g)| *g).collect();
        assert!(
            distinct.len() >= 4,
            "a 120x40 frame drew only {} distinct glyphs ({distinct:?}), so the \
             value is barely reaching the ramp at all",
            distinct.len()
        );
    }

    /// The mapping has to run sparse-to-dense in the ramp's own order.
    ///
    /// A glyph ramp indexed by an absolute value, or indexed the wrong way
    /// round, still draws *something*: the donut put near-white cream on the
    /// `.` and `,` that cover most of its torus and read to the user as a white
    /// broken doughnut. So this walks the drawn cells in value order and
    /// requires the ramp index never to go backwards.
    #[test]
    fn the_glyph_ramp_runs_sparse_to_dense_with_the_value() {
        let size = (120u16, 40u16);
        let mut plasma = Plasma::new(PlasmaOptions::default(), size);
        let by_value = drawn_by_value(&mut plasma, size);

        let mut highest_seen = 0usize;
        for (value, glyph) in &by_value {
            let index = plasma
                .ramp
                .glyphs()
                .iter()
                .position(|candidate| candidate == glyph)
                .unwrap_or_else(|| panic!("{glyph:?} is not in the ramp"));

            assert!(
                index >= highest_seen,
                "value {value:.3} drew {glyph:?} (ramp step {index}) after the \
                 sparser step {highest_seen} had already been used, so the ramp \
                 is running backwards"
            );
            highest_seen = index;
        }
    }

    /// The value must not be pinned against either end of the ramp.
    ///
    /// This is the check that fails on a field with a saturation rail, and a
    /// rail is a *moving hard iso-line*: on one side of it a cell is a clipped
    /// flat value and on the other it is riding the field, so everything on the
    /// clipped side animates only as the line sweeps past it. Both ends matter,
    /// not only the low one.
    ///
    /// The completeness clause at the end -- every step of the ramp reached, not
    /// just the two ends -- is a statement about how finely the ramp divides the
    /// field's range, and it only holds if the sweep covers the field's *whole*
    /// range of values. It used to sweep 12 frames at 0.37, which is 4.4 units
    /// of `plasma.time` against a period of 8*pi = 25.1, so 18% of a cycle --
    /// enough for a five-step ramp, whose top band is a fifth of the range wide,
    /// and not enough for a finer one, whose top band is narrower than the part
    /// of the cycle the sweep looked at. Measured over a full cycle the field
    /// spans 0.0006 to 0.9974; measured over the old 18% it spans to 0.913, and
    /// a ten-step ramp's ninth step is never reached.
    ///
    /// So the sweep widened twice since. First to a whole cycle, and now that
    /// there is no cycle, to a **window eight times as long** -- 64*pi = 201
    /// units, which at the default `time_scale` is about six and a half minutes
    /// of wall clock. A window rather than a period is the honest word for it:
    /// naming a period would reintroduce the assumption this effect no longer
    /// has.
    ///
    /// **What the field reaches inside the window is now recorded too, because
    /// the top of it is what the completeness clause depends on.** Measured at
    /// 200x50 the five-term field spans **0.003 to 0.921** over the window,
    /// against 0.0006 to 0.9974 for the four-term one over a full cycle. The top
    /// is lower and that is the cost of a field that moves less: the extreme
    /// values are a coincidence of five terms' phases rather than four, and a
    /// quieter field hits that coincidence less often. It does not threaten the
    /// clause -- the last boundary is 0.737, comfortably under 0.921 -- but it
    /// is the number to re-measure first if the top step ever goes missing, and
    /// it is not the number the assertion reports when it does.
    ///
    /// The bound on the two ends changed with the calibration, and the reason is
    /// not that it got looser. It used to be 10% and it still is, but the number
    /// it is compared against moved: the ends used to hold a few tenths of a
    /// percent of the field's *values* while the busy middle held a quarter of
    /// them, and now the ends hold a sixth of the *field's area* while the middle
    /// holds a sixteenth each. 10% is a real constraint on that -- it is the
    /// bound that would catch a remap which flattened one end and left the other
    /// alone -- but it is a constraint on a different quantity than it was.
    /// `every_glyph_of_the_default_ramp_carries_its_share_of_the_screen` is the
    /// test that holds all sixteen steps to the same bound rather than two of
    /// them.
    #[test]
    fn the_rendered_value_is_not_pinned_to_either_end_of_the_ramp() {
        let size = (200u16, 50u16);
        let mut plasma = Plasma::new(PlasmaOptions::default(), size);
        let ramp = plasma.ramp.clone();
        let steps = ramp.len();

        // Eight of what the field's period used to be, which is the only sense in
        // which a span of time can be described here now.
        const WINDOW: f64 = 64.0 * std::f64::consts::PI;
        const FRAMES: u64 = 48;

        let mut at_bottom = 0usize;
        let mut at_top = 0usize;
        let mut total = 0usize;
        let mut used: HashSet<usize> = HashSet::new();

        for frame in 0..FRAMES {
            plasma.time = WINDOW * frame as f64 / FRAMES as f64;
            for (x, y, cell) in plasma.get_diff() {
                let value = value_at(size, x, y, plasma.time);
                // The glyph is indexed by the *remapped* value. It was indexed
                // by the raw one when this test was written, and it is worth
                // naming the difference: a rail in the field is a property of the
                // value, and a rail in the calibration is a property of
                // `glyph_value`. The completeness clause below is about the
                // first and the shares are about the second, and conflating
                // them is how a test ends up checking that a table is sorted.
                let index = ramp.index_for(glyph_value(value) as f32);
                used.insert(index);
                if index == 0 {
                    at_bottom += 1;
                }
                if index == steps - 1 {
                    at_top += 1;
                }
                total += 1;

                // The drawn glyph has to be the one the value selected, or the
                // two encodings have drifted apart and the test above measures
                // the wrong thing.
                assert_eq!(
                    cell.symbol,
                    ramp.sample(glyph_value(value) as f32),
                    "cell ({x}, {y}) at value {value:.3} drew {:?} rather than \
                     the ramp's own step",
                    cell.symbol
                );
            }
        }

        let bottom = at_bottom as f64 / total as f64;
        let top = at_top as f64 / total as f64;
        assert!(
            bottom < 0.10,
            "{:.1}% of samples sit on ramp step 0, so a large region of the \
             screen is pinned against the bottom of the range",
            bottom * 100.0
        );
        assert!(
            top < 0.10,
            "{:.1}% of samples sit on ramp step {}, the top of the ramp",
            top * 100.0,
            steps - 1
        );
        let mut vmin = 1.0f64;
        let mut vmax = 0.0f64;
        for step in 0..400 {
            let now = step as f64 * WINDOW / 400.0;
            for y in 0..size.1 as usize {
                for x in 0..size.0 as usize {
                    let v = value_at(size, x, y, now);
                    vmin = vmin.min(v);
                    vmax = vmax.max(v);
                }
            }
        }
        assert_eq!(
            used.len(),
            steps,
            "only {} of {steps} ramp steps were reached; the field spans \
             {vmin:.4} to {vmax:.4} over the window, so the ramp divides more \
             than the field uses",
            used.len()
        );
    }

    /// Every glyph of the ramp has to earn its place, and the report has to say
    /// by how much it failed.
    ///
    /// The user's second complaint: "you can use more different characters".
    /// Sixteen characters on a field where two of them covered half the screen
    /// is not sixteen characters. The numbers are in the failure message because
    /// a histogram assertion whose message is only "assertion failed" is a test
    /// that gets deleted rather than fixed.
    ///
    /// The two bounds are 20% and 1%, against a measured busiest of 7.9% and a
    /// measured sparsest of 2.6%. Sixteen steps on a field is a sixteenth of the
    /// screen each if the calibration is right and a long tail of nothing if it
    /// is not, and this is the test that says which.
    ///
    /// **A word on the floor, because 1% is doing less work than it looks.** The
    /// bound is 1% and the measurement is 2.6%, so the test as written would
    /// have passed a table that starved `@` to 1.5% -- which is exactly what the
    /// table this one replaced did when carried onto the five-term field. It is
    /// not loosened here, because the histogram at the default seed is not the
    /// question any more: there is a different field on every launch, and the
    /// seed the table is fitted to is one of them.
    /// `the_calibration_holds_across_seeds_not_just_the_default_one` is the
    /// test that holds the floor over ten seeds, and it is the one that goes
    /// red.
    ///
    /// 200x50 and not 80x24, and the difference is not cosmetic. At 80x24 the
    /// sparsest glyph measures 0.6% and cannot be made to measure more, because
    /// the field does not reach its own extremes often enough at that size -- so
    /// asserting a floor there would be asserting something about the field
    /// that is not true. `a_short_terminal_thins_the_ends_of_the_ramp` covers
    /// the small case on the terms it can actually meet.
    #[test]
    fn every_glyph_of_the_default_ramp_carries_its_share_of_the_screen() {
        let size = (200u16, 50u16);
        let shares = glyph_shares(size, 200);
        let measured = describe_shares(&shares);

        for (step, share) in shares.iter().enumerate() {
            assert!(
                *share <= 0.20,
                "ramp step {step} covers {share:.2}% of a {size:?} screen over 200 \
                 frames, so it is a shade in name only; the whole distribution is \
                 [{measured}]"
            );
            assert!(
                *share >= 0.01,
                "ramp step {step} covers {share:.2}% of a {size:?} screen over 200 \
                 frames, so it is effectively not drawn at all; the whole \
                 distribution is [{measured}]"
            );
        }
    }

    /// What a small terminal gets, stated rather than assumed.
    ///
    /// The calibration is fitted at 200x50 and this is the 80x24 measurement of
    /// the same run, and it is still a different answer -- but a good deal less
    /// different than it was. The shares run 3.23% at the space up to 10.10% at
    /// `=`, and they run *monotonically up* to `=` and then back down to 0.62% at
    /// `@`, which is the shape of a field that does not reach its own extremes
    /// often enough at this size, and so leaves the bottom of the ramp thin. The
    /// field over 460,800 samples spans 0.017 to 0.994 here against 0.003 to
    /// 0.996 at 200x50.
    ///
    /// **The gap narrowed a lot, and the five terms are why.** The space was
    /// 1.54% here before this round and 3.23% now, and the field's quantiles at
    /// 80x24 sit within 0.02 of the 200x50 ones at both ends where they used to
    /// be 0.20 apart at the bottom. A sum of five sines has a flatter
    /// distribution than a sum of four, so it samples its own tails on a small
    /// screen more often. The end of the ramp is still the thinnest part and
    /// `@` is still six hundredths of the screen -- a small screen still does
    /// not get the full ramp -- but it is a much smaller shortfall.
    ///
    /// The 1% floor from `every_glyph_of_the_default_ramp_carries_its_share_of_
    /// the_screen` is deliberately *not* asserted here, and saying so in a test
    /// is better than silently applying a bound the size cannot meet.
    ///
    /// What is asserted is that the degradation is graceful: the screen is still
    /// spread across the whole ramp, and no single glyph has taken the screen
    /// over. Both halves were false before the calibration -- the busiest glyph
    /// measured 29.1% of an 80x24 screen and 23.5% of a 200x50 one -- and
    /// neither is true now, at 10.1% and 7.9%.
    #[test]
    fn a_short_terminal_thins_the_ends_of_the_ramp() {
        let size = (80u16, 24u16);
        let shares = glyph_shares(size, 200);
        let measured = describe_shares(&shares);
        let steps = shares.len();

        let busiest = shares.iter().copied().fold(0.0f64, f64::max);
        assert!(
            busiest <= 0.20,
            "at {size:?} one glyph covers {busiest:.2}% of the screen, so the \
             calibration is not reaching the small case at all; the whole \
             distribution is [{measured}]"
        );

        // Fourteen of sixteen, a step below what it measures, so this catches a
        // calibration that has stopped applying rather than one whose tail has
        // thinned slightly.
        let used = shares.iter().filter(|share| **share > 0.0).count();
        assert!(
            used >= 14,
            "only {used} of {steps} ramp steps were drawn at {size:?}, so the ramp \
             is barely being used on a small screen; the whole distribution is \
             [{measured}]"
        );
    }

    /// The distribution as one string, for a failure message.
    ///
    /// A histogram test that prints its own distribution only when it fails is a
    /// histogram test nobody can check when it passes, and the thing being
    /// asserted here is the *shape*. Printing it always costs one line of test
    /// output and makes a regression in the calibration visible as a drifting
    /// series rather than as a threshold finally tripping.
    fn describe_shares(shares: &[f64]) -> String {
        let ramp = glyph_ramp(&PlasmaOptions::default().glyphs);
        let described: Vec<String> = ramp
            .glyphs()
            .iter()
            .zip(shares.iter())
            // Times a hundred, because a share is a fraction and a "%" in a
            // format string is a literal. The first version of this printed
            // "0.06%" for a step that covers 6.25% of the screen, which is a
            // number small enough to look like a fault and is not one.
            .map(|(glyph, share)| format!("{glyph:?} {:.2}%", share * 100.0))
            .collect();
        let line = described.join(", ");
        println!("plasma glyph shares: {line}");
        line
    }

    /// The share of the screen each ramp step takes, over a fixed run.
    ///
    /// Reads the *value* rather than the drawn cell, and that is a deliberate
    /// choice worth defending. `get_diff` returns only the cells that changed
    /// from the previous frame, so counting its glyphs measures which characters
    /// are *animating* -- which is a different question, and one this crate has
    /// been caught by before. Plasma repaints every cell every frame, so the two
    /// ought to agree, and they do not: the field is smooth enough that a large
    /// region holds the same character across consecutive frames, and those cells
    /// drop out of the diff entirely. Counting the diff gives a distribution
    /// weighted towards the *boundaries* between characters, which is where the
    /// field is changing fastest and nowhere near where it spends its area.
    ///
    /// Measured, the difference is a factor of fifteen: this reports 6.3% per
    /// step and the diff-based version reported 0.06%, which is a distribution
    /// that looks plausible and is a histogram of a different thing. `total` is
    /// the full cell count, not the diff's length, so a diff that quietly
    /// returned nothing would show up as every step at zero rather than as a
    /// passing test.
    ///
    /// The step is 0.5 units of `plasma.time` between frames, so 200 frames is
    /// 100 units -- four times the period this field used to have, so the old
    /// code would have measured the same frame four times over.
    fn glyph_shares(size: (u16, u16), frames: u64) -> Vec<f64> {
        glyph_shares_for(size, frames, PlasmaOptions::default().seed)
    }

    /// [`glyph_shares`] against an explicitly named seed.
    ///
    /// There is more than one field now, and a histogram of "the field" that
    /// silently means "seed 42's field" is a histogram of a launch nobody
    /// chose. The default-seed version is a thin wrapper over this rather than
    /// the other way round, so the tests that want the default and the test that
    /// wants a spread go through one implementation.
    fn glyph_shares_for(size: (u16, u16), frames: u64, seed: u64) -> Vec<f64> {
        let ramp = glyph_ramp(&PlasmaOptions::default().glyphs);
        let tuning = FieldTuning::from_seed(seed);
        let mut counts = vec![0usize; ramp.len()];
        let mut total = 0usize;
        for frame in 0..frames {
            let now = frame as f64 * 0.5;
            for y in 0..size.1 as usize {
                for x in 0..size.0 as usize {
                    let v = value_at_with(&tuning, size, x, y, now);
                    let g = glyph_value(v);
                    let i = ramp.index_for(g as f32);
                    counts[i] += 1;
                    total += 1;
                }
            }
        }
        assert!(total > 0, "the run drew nothing at all");
        counts.iter().map(|c| *c as f64 / total as f64).collect()
    }

    /// The remap's own contract, checked on the remap rather than on the screen.
    ///
    /// Every ordering claim in this module is downstream of this one. A ramp that
    /// ran backwards, a `glyph_value` that dipped, a table that stopped being
    /// sorted -- each of those would still draw a plausible-looking field, and
    /// `the_glyph_ramp_runs_sparse_to_dense_with_the_value` would catch it at the
    /// cost of a whole rendered frame per sample. This is the same property at
    /// the cost of a loop over a thousand values.
    ///
    /// A remap that is *not* strictly increasing is the subtler failure, and it
    /// is a real one for a table like this: two boundaries within a float apart
    /// produce a segment of zero width, and the division there is guarded, so
    /// the function stays defined and quietly discards a step. Non-decreasing is
    /// the contract; *strictly* increasing over the interior is what makes each
    /// ramp step reachable, and both are asserted.
    /// The lookup tables agree with the boundary table they are derived from.
    ///
    /// `STEP_LUT` and `SEGMENTS` are a performance rewrite of a search over
    /// `VALUE_BOUNDARIES`, and a derived table is a second place for the
    /// calibration to be wrong in. Nothing else checks them: the monotonicity and
    /// reachability tests would both still pass against a table that quantised
    /// coarsely enough to flatten the middle of the curve, because a flattened
    /// curve is still monotone and still reaches both ends.
    ///
    /// So this compares the tables against the search they replaced, at every
    /// boundary and either side of it -- the places they are allowed to disagree,
    /// and where the resolution is being asked to work hardest -- plus a sweep
    /// over the whole range, which is where they are not.
    #[test]
    fn the_lookup_tables_agree_with_the_boundaries_they_replace() {
        let search = |value: f64| {
            let index = VALUE_BOUNDARIES.partition_point(|b| *b <= value);
            let (low, high) = match index {
                0 => (0.0, VALUE_BOUNDARIES[0]),
                15 => (VALUE_BOUNDARIES[14], 1.0),
                _ => (VALUE_BOUNDARIES[index - 1], VALUE_BOUNDARIES[index]),
            };
            let (out_low, out_high) = match index {
                0 => (0.0, 0.5 / 15.0),
                15 => (14.5 / 15.0, 1.0),
                _ => ((index as f64 - 0.5) / 15.0, (index as f64 + 0.5) / 15.0),
            };
            let fraction = if high <= low {
                0.0
            } else {
                ((value - low) / (high - low)).clamp(0.0, 1.0)
            };
            out_low + fraction * (out_high - out_low)
        };

        let mut probes: Vec<f64> = vec![0.0, 1.0, 0.5, 1.0 / 3.0];
        for bound in VALUE_BOUNDARIES {
            for delta in [-1e-9f64, -1e-7, 0.0, 1e-9, 1e-7] {
                probes.push((bound + delta).clamp(0.0, 1.0));
            }
        }
        for step in 0..=2000 {
            probes.push(f64::from(step) / 2000.0);
        }

        // One bucket is 1/65535 of the range, so a value within half a bucket of
        // a boundary may legitimately round to the other side of it. That is the
        // whole allowance, and it is the table's own resolution rather than a
        // number chosen to make the test pass.
        let tolerance = 0.5 / (STEP_LUT_LEN - 1) as f64;
        let count = probes.len();
        let mut worst = 0.0f64;
        for value in &probes {
            worst = worst.max((glyph_value(*value) - search(*value)).abs());
        }
        assert!(
            worst <= tolerance,
            "the lookup tables disagree with the boundary search by {worst:.9} over \
             {count} probes, which is more than the {tolerance:.9} their resolution \
             allows"
        );
    }

    #[test]
    fn the_value_remap_is_monotone_and_reaches_both_ends() {
        let samples = 20_000;
        let mut previous = f64::NEG_INFINITY;
        for step in 0..=samples {
            let value = step as f64 / samples as f64;
            let remapped = glyph_value(value);
            assert!(
                remapped >= previous,
                "glyph_value went backwards: {value:.5} mapped to {remapped:.5} \
                 after {previous:.5}"
            );
            previous = remapped;
        }

        // And the two ends exactly, because a remap that starts or ends short
        // quietly loses a ramp step without any of the ordering checks noticing:
        // the value is clamped, so the step is still reachable, just by nothing.
        assert_eq!(glyph_value(0.0), 0.0, "the bottom of the range is not 0");
        assert_eq!(glyph_value(1.0), 1.0, "the top of the range is not 1");

        // A NaN has to read as the bottom rather than sail through the bounds
        // check, the same way `GlyphRamp::index_for` treats it. The sum of four
        // sines cannot produce one, but a hand-edited `spatial_scale` of NaN can,
        // and a NaN that reached `partition_point` would be an unspecified index.
        assert_eq!(
            glyph_value(f64::NAN),
            glyph_value(0.0),
            "a NaN value did not read as the bottom of the range"
        );

        // Every step reachable, and strictly so. A step with an empty preimage is
        // a character the ramp will never draw, which is the thing the
        // histogram test above is about -- asserted here on the function so a
        // failure points at the table rather than at the screen.
        let ramp = glyph_ramp(&PlasmaOptions::default().glyphs);
        let mut reached: HashSet<usize> = HashSet::new();
        for step in 0..=samples {
            reached.insert(
                ramp.index_for(glyph_value(step as f64 / samples as f64) as f32),
            );
        }
        assert_eq!(
            reached.len(),
            ramp.len(),
            "only {} of {} ramp steps have a non-empty preimage under glyph_value, \
             so the calibration has a step the field can never draw",
            reached.len(),
            ramp.len()
        );
    }

    /// The table is a calibration, so its inputs have to be sane.
    ///
    /// `VALUE_BOUNDARIES` is fifteen measured numbers, and a table is the easiest
    /// kind of constant to edit wrongly without noticing: swap two, leave one out
    /// of order, and `glyph_value` still returns a value in `0..=1`, still
    /// monotone, and still draws a field that looks like plasma. None of the
    /// ordering tests can see it and the histogram test would report a shifted
    /// distribution rather than a fault.
    ///
    /// So the three properties the numbers have to have are asserted on the
    /// numbers. Length is the one that catches an accidental edit, since the
    /// array's type fixes it and the *semantics* need the rest: strictly
    /// increasing, inside the open interval, and symmetric about the middle
    /// enough that the top of the ramp is not starved relative to the bottom.
    #[test]
    fn the_value_boundaries_are_a_sorted_interior_table() {
        assert_eq!(
            VALUE_BOUNDARIES.len(),
            15,
            "there are sixteen ramp steps, so fifteen boundaries separate them"
        );

        for pair in VALUE_BOUNDARIES.windows(2) {
            assert!(
                pair[1] > pair[0],
                "the boundaries run backwards: {} then {}",
                pair[0],
                pair[1]
            );
        }

        for bound in VALUE_BOUNDARIES {
            assert!(
                (0.0..1.0).contains(&bound),
                "the boundary {bound} is not inside the value range, so a step of \
                 the ramp can never be reached on one side of it"
            );
        }

        // The two end steps are the ones a mis-shaped table starves, because
        // they are the only ones whose preimage is a *half* segment. Measured
        // shares at 200x50 are 6.19% for the bottom step and 6.20% for the top,
        // and this is the cheap check that the table has not been edited into
        // an asymmetry since.
        let lowest = VALUE_BOUNDARIES[0];
        let highest = 1.0 - VALUE_BOUNDARIES[VALUE_BOUNDARIES.len() - 1];
        let ratio = highest / lowest;
        assert!(
            (0.7..1.4).contains(&ratio),
            "the value range below the first boundary is {lowest:.3} and above the \
             last is {highest:.3}, a ratio of {ratio:.2}, so the calibration is \
             lopsided: one end of the ramp covers a different amount of the field \
             from the other"
        );
    }

    /// The bottom of the screen must not be the busiest part of it.
    ///
    /// This is the user's complaint, written as a measurement. What the eye
    /// reads as flickering is the *vertical spatial frequency*: a region with
    /// twice the bands per row shows twice the motion for the same amplitude,
    /// and high-frequency motion reads as flicker where low-frequency motion
    /// reads as drift.
    ///
    /// Measured as the mean |change| between adjacent rows, averaged over the
    /// four rows at the top and the four at the bottom. It is the spatial term
    /// and not the saturation rail, and the two were separated by measurement
    /// rather than by reading: instrumenting per-row |plasma - previous_plasma|
    /// over time gave a bottom-to-top ratio of 0.90 to 0.96, i.e. the bottom
    /// rows changed *less* in time, and the busiest row moved between frames.
    ///
    /// The asymmetry was in the maths and not a boundary condition. The fourth
    /// term was anchored at the origin, so its vertical phase gradient was zero
    /// along the top edge and maximal along the bottom, adding to the third
    /// term's at the bottom and subtracting from it at the top. There is no
    /// division by the height and no bottom-edge special case to remove.
    #[test]
    fn the_bottom_rows_are_not_the_busiest_rows() {
        let size = (200u16, 50u16);

        // Before the fix this measured 1.22x to 1.64x depending on the frame,
        // and at 400x200 the single busiest row on the screen was row 199, the
        // last one. The tolerance is 1.15 and is unchanged: the five-term field
        // measures 0.93x to 1.01x here, so there is room for the frame's phase
        // without room for the old bias to come back. The fifth term did not
        // move this, and it is worth knowing why rather than assuming it: the
        // diagonal's contribution to the vertical gradient is
        // `cos((x+y)/D)/D`, and the screen-average of a cosine over a rectangle
        // is near zero, so it adds to the *variance* of the vertical rate
        // without adding to its mean.
        for now in [0.0, 0.37, 1.1, 2.6, 4.3, 7.1] {
            let (top, bottom) = row_change_ratio(size, now);
            assert!(
                bottom < top * 1.15,
                "at t={now} the bottom four rows changed {:.2}x as fast as the top \
                 four, so the bottom of the screen is flickering",
                bottom / top
            );
        }
    }

    /// The radial terms have to measure in the same units as each other and as
    /// the linear terms, or the field's circles are ellipses.
    ///
    /// The value used to be fed `x` in cell units and `y` in field units at two
    /// per cell row, so one unit horizontally was half a unit vertically and
    /// both radial terms drew 2 : 1 ellipses in units -- which on a 1 : 1.2 font
    /// is 1.67 : 1 on screen. That measured as a vertical rate 1.46x the
    /// horizontal rate at 80x24, 1.26x at 200x50 and 1.87x at 400x200.
    ///
    /// A cell is taller than it is wide, so a *round* field on a 1 : 1.2 font
    /// has a vertical rate 1 / 1.2 = 0.83x its horizontal one. The corrected
    /// field measures 0.88 here at 400x200 and 0.57 at 200x50, and the two
    /// differ for a reason worth having: the measurement steps a *row*, which is
    /// 1.2 corrected units, and a screen eight times taller has four times as
    /// many rows for the same number of vertical periods, so it averages the
    /// cosine over more of them and lands closer to the isotropic value. The
    /// band below is unchanged -- 1.2 is the side that catches the old bug, and
    /// 0.88 is now only 27% under it, which is the thinnest it has been.
    #[test]
    fn the_field_is_not_stretched_vertically() {
        let size = (400u16, 200u16);
        let scale = PlasmaOptions::default().spatial_scale;
        let tuning = default_tuning();
        let at = |x: f64, y: f64| sample(&tuning, size, x, y, 0.0, scale);

        let mut horizontal = 0.0f64;
        let mut horizontal_samples = 0.0f64;
        let mut vertical = 0.0f64;
        let mut vertical_samples = 0.0f64;

        // Strided by two. The value is smooth, so a stride of two is not a
        // meaningful loss and it halves the work in the largest test here.
        for y in (0..size.1).step_by(2) {
            for x in (0..size.0 - 1).step_by(2) {
                let y_units = f64::from(y) * CELL_ASPECT;
                horizontal += (at(f64::from(x + 1), y_units)
                    - at(f64::from(x), y_units))
                .abs();
                horizontal_samples += 1.0;
            }
        }
        for y in (0..size.1 - 1).step_by(2) {
            for x in (0..size.0).step_by(2) {
                let xu = f64::from(x);
                vertical += (at(xu, f64::from(y + 1) * CELL_ASPECT)
                    - at(xu, f64::from(y) * CELL_ASPECT))
                .abs();
                vertical_samples += 1.0;
            }
        }

        let ratio =
            (vertical / vertical_samples) / (horizontal / horizontal_samples);
        assert!(
            (0.4..1.2).contains(&ratio),
            "the field changes {ratio:.2}x as fast going down a column as it \
             does going across a row, so the radial terms are ellipses rather \
             than circles"
        );
    }

    /// The new knob has to survive a trip through a config file.
    ///
    /// `--print-config` writes every key to disk, so a generated config is
    /// pinned to whatever the defaults were the day it was generated and every
    /// future knob arrives as "a key the user's file does not have". That is
    /// the normal case, not the exotic one, and it is why each options struct
    /// in this crate carries `#[serde(default)]` and why the contract suite
    /// deletes one key at a time and re-parses.
    #[test]
    fn the_glyph_ramp_round_trips_through_toml() {
        let options: PlasmaOptions =
            toml::from_str("glyphs = \" .oO@\"").expect("a lone key parses");
        assert_eq!(options.glyphs, " .oO@", "the key was not read back");
        assert_eq!(
            options.time_scale,
            PlasmaOptions::default().time_scale,
            "one key in the section silently reset the others"
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        assert!(
            serialised.contains("glyphs"),
            "the key is missing from the serialised form, so --print-config \
             would never write it: {serialised}"
        );
    }

    /// A ramp a user's config has emptied out has to degrade to the documented
    /// default rather than to `GlyphRamp`'s own fallback.
    ///
    /// `GlyphRamp` never returns an empty ramp, but its fallback is ASCII
    /// `SHADE`, which is documented as *not* monotonic in ink. Falling back to
    /// it would put a different -- and worse -- default back exactly when the
    /// configured one has failed, which is when it matters most.
    #[test]
    fn an_unusable_glyph_config_falls_back_to_the_default_ramp() {
        for configured in ["", "\u{7}\u{1}", "\u{4E2D}"] {
            let ramp = glyph_ramp(configured);
            assert_eq!(
                ramp.glyphs(),
                DEFAULT_GLYPHS.chars().collect::<Vec<char>>(),
                "{configured:?} did not fall back to the documented default"
            );
        }

        // The fallback is specifically *not* `GlyphRamp`'s, which is SHADE, and
        // specifically not the block elements either. Both are argued about in
        // `the_default_ramp_is_ordered_by_estimated_ink_coverage` and
        // `the_default_ramp_is_ascii_and_contains_no_block_elements`; pinning
        // it here is what stops this test from passing against whichever ramp
        // happens to be the default.
        assert_ne!(
            DEFAULT_GLYPHS,
            glyph_ramp::presets::SHADE,
            "the default has become SHADE, which is documented as non-monotonic"
        );
        assert_ne!(
            DEFAULT_GLYPHS,
            glyph_ramp::presets::BLOCKS,
            "the default has become the block elements again"
        );
    }

    /// A configured ramp is the user's decision, used in the order given.
    #[test]
    fn a_configured_glyph_ramp_is_drawn_in_the_order_given() {
        let options = PlasmaOptions {
            glyphs: " .:oO@".to_string(),
            ..Default::default()
        };
        let mut plasma = Plasma::new(options, (120, 40));

        let ramp = plasma.ramp.clone();
        assert_eq!(ramp.glyphs(), &[' ', '.', ':', 'o', 'O', '@']);
        let drawn: HashSet<char> = plasma
            .get_diff()
            .iter()
            .map(|(_, _, cell)| cell.symbol)
            .collect();
        for glyph in &drawn {
            assert!(
                ramp.glyphs().contains(glyph),
                "{glyph:?} is not in the configured ramp"
            );
        }
        assert!(
            drawn.len() >= 4,
            "the configured ramp was barely used: {drawn:?}"
        );
    }

    /// The default ramp is ASCII, and the block elements are out.
    ///
    /// The user's report, unedited: plasma "uses this kind of locks [blocks].
    /// Full locks are unnecessary; maybe ASCII characters would be better."
    ///
    /// Block elements were the default here for a good reason -- they are the one
    /// ramp whose ordering Unicode defines -- so replacing them is a real trade
    /// and the note on [`DEFAULT_GLYPHS`] is where the cost is written down.
    /// What is *not* a trade is the repertoire: `░▒▓█` is one fixed block in one
    /// fixed range, and a terminal whose font does not cover it gets a field of
    /// replacement characters.
    #[test]
    fn the_default_ramp_is_ascii_and_contains_no_block_elements() {
        let configured = PlasmaOptions::default().glyphs;

        for glyph in configured.chars() {
            assert!(
                glyph.is_ascii(),
                "the default ramp contains {glyph:?} (U+{:04X}); the user asked \
                 for ASCII characters",
                glyph as u32
            );
            assert!(
                !('\u{2580}'..='\u{259F}').contains(&glyph),
                "the default ramp contains the block element {glyph:?} \
                 (U+{:04X}), which is what was asked to be taken out",
                glyph as u32
            );
        }

        // And the effect really draws from it, rather than filtering it away on
        // the way in -- a ramp that was rejected here and silently replaced by
        // `GlyphRamp`'s own fallback would pass the check above and lose the
        // ink ordering the ordering test depends on.
        let mut plasma = Plasma::new(PlasmaOptions::default(), (120, 40));
        for glyph in plasma.ramp.glyphs() {
            assert!(
                glyph.is_ascii() && !('\u{2580}'..='\u{259F}').contains(glyph),
                "the ramp the effect draws from is {glyph:?} (U+{:04X})",
                *glyph as u32
            );
        }
        let drawn: HashSet<char> = plasma
            .get_diff()
            .iter()
            .map(|(_, _, cell)| cell.symbol)
            .collect();
        assert!(
            drawn.len() >= 4,
            "the ASCII default ramp was barely used: {drawn:?}"
        );
    }

    /// The default ramp is ordered by how much ink each glyph puts down.
    ///
    /// ASCII cannot be ink-monotonic, which is the entire reason
    /// [`glyph_ramp::presets::BLOCKS`] exists. So what is asserted here is not
    /// monotonicity but *non-decreasing under a stated estimate*, with the
    /// estimate written out instead of being asserted to be true, and with the
    /// pairs the estimate cannot separate named and pinned.
    ///
    /// The estimates are ink coverage as a percentage of the cell box, in a
    /// font of ordinary weight, ranked by how many strokes there are and how
    /// much of the cell they reach. They are hand estimates and they are wrong
    /// at the margins -- a monospace font is free to draw `:` heavier than `-`
    /// or lighter, and plenty do -- so this is an oracle for the *ramp*, not
    /// for the font. A reader who disagrees with one number changes one number
    /// here and sees what it does to the argument, which is the point of
    /// writing it down.
    #[test]
    fn the_default_ramp_is_ordered_by_estimated_ink_coverage() {
        /// Ink coverage as a percentage of the cell box, and why.
        ///
        /// Ranked by how many strokes the glyph is drawn with and how much of
        /// the cell those strokes reach, which is what the number stands in
        /// for. Deliberately not derived from any one font: the point is the
        /// *order*, and a single font's real coverage would be an argument
        /// about that font.
        const INK_BY_COVERAGE: &[(char, f32)] = &[
            (' ', 0.0),  // nothing
            ('\'', 2.0), // a short tick, the lightest mark in the set
            ('.', 5.0),  // one small square at the baseline
            ('-', 7.0),  // one thin bar, the full cell width
            (':', 11.0), // two dots stacked
            ('=', 12.0), // two thin bars, the full cell width each
            (';', 14.0), // a dot and a comma, so a dot plus a tail
            ('*', 15.0), // an asterisk: several short strokes, none reaching
            ('+', 16.0), // one full-width bar and one full-height bar
            ('X', 21.0), // two full-cell diagonals, longer strokes than `+`
            ('#', 27.0), // four strokes, two of them full height
            ('O', 30.0), // one closed ring at full cell height
            ('%', 33.0), // two rings and a slash
            ('&', 38.0), // a bowl, a loop and a leg
            ('M', 42.0), // four strokes, two of them full height and two
            // full-cell diagonals
            ('@', 46.0), // a ring, an inner bowl and a tail
        ];

        let ink = |glyph: char| -> f32 {
            match INK_BY_COVERAGE.iter().find(|(c, _)| *c == glyph) {
                Some((_, coverage)) => *coverage,
                None => panic!(
                    "{glyph:?} (U+{:04X}) is not in the ink table, so the ramp has \
                     grown a character whose weight has not been argued about. Add \
                     it, with a reason.",
                    glyph as u32
                ),
            }
        };

        // The oracle has to be able to say *no*, or it is not an oracle.
        //
        // `SHADE` is the conventional ASCII ramp and this crate documents it as
        // non-monotonic, listing `=`/`+` and `+`/`*`. This table reproduces
        // `+`/`*` and not `=`/`+`, and that disagreement is itself worth
        // recording: `=` is two thin bars and `+` is one thin bar plus one
        // full-height bar, which is close enough to a rounding that calling `=`
        // the heavier of the two is a statement about a particular font. What
        // every stroke-count model agrees on is the third pair, which the
        // crate's own note does not list: `:` is two dots and `-` is one bar.
        let shade_inversions: Vec<String> = glyph_ramp::presets::SHADE
            .chars()
            .collect::<Vec<char>>()
            .windows(2)
            .filter(|pair| ink(pair[1]) < ink(pair[0]))
            .map(|pair| format!("{}{}", pair[0], pair[1]))
            .collect();
        assert_eq!(
            shade_inversions,
            [":-", "+*"],
            "the ink table no longer finds the inversions it is supposed to find \
             in SHADE, so it cannot be trusted to clear the default ramp"
        );

        // The ramp actually in use.
        let ramp = glyph_ramp(&PlasmaOptions::default().glyphs);
        let glyphs = ramp.glyphs();
        assert!(
            glyphs.len() >= 8,
            "the default ramp has {} steps, too few to carry a smooth value \
             across a whole screen",
            glyphs.len()
        );
        for pair in glyphs.windows(2) {
            assert!(
                ink(pair[1]) >= ink(pair[0]),
                "ramp steps {}{} run backwards -- {}% then {}% -- so a rising \
                 value makes the field lighter there",
                pair[0],
                pair[1],
                ink(pair[0]),
                ink(pair[1])
            );
        }

        // And the pairs the estimate genuinely cannot separate. Three percentage
        // points of cell coverage is a shade over half a stroke width; below it
        // the table is measuring the font rather than the glyph. These are ties,
        // not inversions -- the ramp may put them in either order, and the order
        // is a legibility decision.
        //
        // **Nine of them, up from three**, and the growth is the finding rather
        // than an oversight. Six of the six new boundaries are at the light end
        // -- ` '`, `'.`, `.-`, `:=`, `=;`, `;*` -- and two more sit at the heavy
        // end where the two new ring glyphs went in. The three that were already
        // ties are all at the light end too. There are only so many
        // distinguishable weights below a quarter of a cell, and a sixteen-step
        // ramp has to spend six of its fifteen boundaries down there. The
        // estimates are the argument for the order, not a claim of resolution the
        // set does not have.
        //
        // What can be said about each:
        //
        // - ` '` -- a space and an apostrophe, two points apart. This is the
        //   weakest claim in the table: "nothing" and "one short tick" differ by
        //   the width of the tick's stroke, and the two points are the table
        //   saying that a tick is a small thing rather than measuring it. It is
        //   still an improvement on leaving the space and the next glyph five
        //   points apart, which is what the ten-step ramp did.
        // - `'.` -- an apostrophe and a period, three points apart. Both are one
        //   small mark; `'` is a narrow tick and `.` is a square blob, and most
        //   fonts draw the square heavier. The table is least sure of this one.
        // - `.-` -- a period and a hyphen, two points apart. A dot is the
        //   lightest mark a font can draw and a bar is one of the heaviest
        //   single-stroke marks, so this gap is small because the table is least
        //   sure of it, not because they are close.
        // - `:=` -- a colon and an equals sign, one point apart. Two dots against
        //   two bars, and the table cannot tell which is heavier. The order here
        //   is a legibility call: `:` is the most widely recognised "slightly
        //   more than nothing" and `=` the most widely recognised "slightly more
        //   than that".
        // - `=;` -- an equals sign and a semicolon, two points apart, and the two
        //   disagree about *how* to spend a second mark: two bars, or a dot and
        //   a tail. `;` is given the heavier reading because a tail hangs below
        //   the baseline and a font's baseline strokes are its heaviest.
        // - `;*` -- a semicolon and an asterisk, one point apart. Both are two or
        //   three short marks; an asterisk's strokes are each shorter than a tail.
        // - `*+` -- an asterisk and a plus sign, one point apart. An asterisk is
        //   several strokes none of which reaches the cell; a plus is two bars
        //   that both do, and the table cannot resolve the difference.
        // - `#O` -- a hash and a capital O, three points apart. Four thin strokes
        //   against one closed ring. `O` is a longer path but a single stroke,
        //   and which covers more of the cell depends on the font. This is the
        //   first of the two places the table gives up at the *heavy* end.
        // - `O%` -- a capital O and a percent sign, three points apart, and the
        //   same argument with one more ring: the percent's two small rings are
        //   not the O's one large one, so a font that draws a bold `O` can put
        //   them either way round.
        //
        // The seven boundaries it *is* confident about are `-:`, `;+`, `+X`, `X#`,
        // `%&`, `&M` and `M@`, and four of those are the heavy end, where the
        // repertoire is not crowded. That asymmetry is the argument for saying
        // the ramp is ordered: the crowd is at one end and the other end is
        // solid.
        //
        // Asserted as an exact set, so a glyph moved across one of these
        // boundaries -- or a new tie quietly created -- has to be argued for
        // rather than slipping past.
        const TIE_RESOLUTION: f32 = 3.0;
        const EXPECTED_TIES: &[&str] =
            &[" '", "'.", ".-", ":=", "=;", ";*", "*+", "#O", "O%"];
        let mut measured: Vec<String> = glyphs
            .windows(2)
            .filter(|pair| (ink(pair[1]) - ink(pair[0])).abs() <= TIE_RESOLUTION)
            .map(|pair| format!("{}{}", pair[0], pair[1]))
            .collect();
        measured.sort();
        let mut named: Vec<String> =
            EXPECTED_TIES.iter().map(|t| t.to_string()).collect();
        named.sort();
        assert_eq!(
            measured, named,
            "the set of adjacent ramp pairs within the ink table's resolution \
             has changed, so either a glyph crossed a boundary or a new tie \
             appeared; both need saying out loud"
        );
    }

    /// The value at a point is the five terms, normalised -- and nothing else.
    ///
    /// `FieldTuning::value` is `(128 + 128*s0 + 128 + 128*s1 + ... ) / 5.0 / 256.0`
    /// over five terms of `128 + 128*sin(..)`, which is `0.5 + (s0 + .. + s4) / 10`
    /// for five sines each running -1 to 1. **The `/5` is the number of terms and
    /// nothing else**, and the mistake a fifth term invites is leaving it at four:
    /// which puts the level where the sines cancel at 0.625 rather than 0.5 and
    /// slides the calibration [`VALUE_BOUNDARIES`] is fitted to up by an eighth of
    /// the range.
    ///
    /// **This is asserted against the terms rather than against the field's own
    /// statistics, and the reason is worth recording.** The obvious test is "the
    /// field's mean is 0.5", and the field's mean is not 0.5: measured over 200
    /// frames at 200x50 it reads 0.451, 0.516 and 0.433 on three seeds, and the
    /// *median* reads 0.449. So a bound on either would have to be 0.05 wide, it
    /// would be 0.06 wide to be safe across seeds, and a 0.06 band around 0.5 does
    /// not exclude the 0.625 it is meant to exclude. The offset is real and it is
    /// not the divisor's fault: the two radial terms average `sin(r/D + phi)` over
    /// a *rectangle* rather than over a disc, and the mean of an exponential over
    /// a rectangle is not zero however many periods cross it, so the phase offsets
    /// the seed draws decide which way the residual points. A time average does not
    /// rescue it either -- the modulation inside each sine is `-depth*sin(f*t)`,
    /// and the mean of `sin(A - c*sin(theta))` over `theta` is `J0(c)*sin(A)`, not
    /// `sin(A)`, so the phase sweep shrinks each term towards zero rather than
    /// averaging it out.
    ///
    /// That is also why [`VALUE_BOUNDARIES`] is fitted to measured *quantiles*
    /// rather than to a mean, and it is worth knowing before anyone "simplifies"
    /// the fit to normalise by the mean: the empirical centre of this field is
    /// 0.45, not 0.5, and a table built on the mean would be 0.05 out on every
    /// boundary.
    ///
    /// The other half of the same failure is a term added to the sum and not to
    /// the count, or dropped from the sum and not from it. Both are caught here
    /// and neither is caught by any statistical bound: with six terms summed and
    /// a divisor of five, the field's mean moves by a twentieth and every
    /// threshold in this file still passes.
    #[test]
    fn the_field_is_the_five_terms_normalised() {
        let size = (200u16, 50u16);
        let w = f64::from(size.0);
        let h = f64::from(size.1) * CELL_ASPECT;
        let scale = PlasmaOptions::default().spatial_scale;

        for seed in [DEFAULT_SEED, 0, 1, 987_654_321] {
            let tuning = FieldTuning::from_seed(seed);
            for now in [0.0, 0.5, 3.7] {
                for y in [0usize, 13, 39] {
                    for x in [0usize, 41, 119] {
                        let xu = x as f64;
                        let yu = y as f64 * CELL_ASPECT;
                        let radius = ((xu - w / 2.0).powi(2)
                            + (yu - h / 2.0).powi(2))
                        .sqrt();
                        let sines = tuning.wave(xu * scale, now)
                            + tuning.sweep(yu * scale, now)
                            + tuning.ring(radius * scale, now)
                            + tuning.breath(radius * scale, now)
                            + tuning.diagonal(xu * scale, yu * scale, now);
                        let expected = (0.5 + sines / 10.0).clamp(0.0, 1.0);
                        let drawn = tuning.value(xu, yu, now, w, h, scale);
                        assert!(
                            (drawn - expected).abs() < 1e-12,
                            "seed {seed} at ({x}, {y}) and t={now}: the value is \
                             {drawn:.6} where the five terms normalised give \
                             {expected:.6}, so the sum's divisor is not its term \
                             count"
                        );
                    }
                }
            }
        }
    }

    // --- character: does it go up and down? -------------------------------
    /// The field must not march up and down the screen. The user's complaint,
    /// as a number.
    ///
    /// "Right now it just goes up and down; it's not something new" -- said
    /// twice, and the first fix did not touch it, which is the finding. The
    /// first fix made the field's time frequencies mutually irrational, so the
    /// field stopped *repeating*. Repetition and character are different
    /// problems: a field can be aperiodic and still go up and down forever, and
    /// this one did, because one term was carrying half the amplitude
    /// vertically at 32 corrected units a second.
    ///
    /// **What is measured.** Not "how fast does the value change" -- that
    /// answers a question about churn, and a field can churn fast and not
    /// travel anywhere. What is measured is the field's net *drift*: the
    /// velocity, in rows and columns per second, at which the whole picture
    /// slides. [`advection_velocity`] recovers it by regressing the time
    /// derivative onto the two spatial derivatives over every cell, which is the
    /// statement "the picture moved by this much, this way".
    ///
    /// **The ratio and not the speeds, because the speeds are the wrong
    /// question.** A field that is still has no drift and passes every upper
    /// bound there is; so has one that has stopped entirely. The claim is about
    /// direction -- a field that goes up and down has a large vertical rate and
    /// a small horizontal one -- and 1.0 is the value a field whose drift is
    /// purely sideways would read.
    ///
    /// Measured, over [`SAMPLE_SEEDS`] and [`DRIFT_SAMPLES`]:
    ///
    /// ```text
    ///             old    new
    ///   200x50    5.81   0.98
    ///   400x200   2.24   0.50
    ///   80x24     3.46   1.13
    /// ```
    ///
    /// The bound is 1.6, which clears the largest new figure by 42% and the
    /// smallest old one by 30%. That the old field's *smallest* is 2.24 and not
    /// 5.8 is worth knowing: 400x200 is eight times the size of 200x50, the
    /// sweep term's phase travels the same number of units either way, and the
    /// horizontal terms' contribution to the fit grows with the screen. The
    /// complaint was reported at ordinary terminal sizes and the fix is measured
    /// at all three.
    ///
    /// `dt` is half a unit of `plasma.time`, which is one second of wall clock
    /// at the default `time_scale` of 0.5 -- the gap between the two frames the
    /// user is actually comparing when they say the field is going up and down.
    ///
    /// **The margin is thinner than the table suggests, and that is deliberate.**
    /// The per-configuration figure ranges from 0.0 to 2.3 at 200x50 depending
    /// on which term happens to dominate the fit at that instant, so a bound
    /// tight enough to catch the old field exactly would trip on unlucky seeds
    /// of the new one. The mean over seeds is the quantity that separates them by
    /// a factor of six, and it is the one asserted. See
    /// [`vertical_to_horizontal_drift`].
    ///
    /// **What this cannot see, named because it is a real blind spot.** A field
    /// with the diagonal term *removed* measures better on this, 0.49 at
    /// 200x50, because the diagonal is the only term that moves in both
    /// directions at once and dropping it leaves the field's remaining motion
    /// almost entirely sideways. So this test says the field does not go up and
    /// down; it cannot say the field has something else to go on instead. That
    /// is `the_field_has_a_diagonal`, and the two are deliberately separate.
    #[test]
    fn the_field_does_not_go_up_and_down() {
        /// Half a unit of `plasma.time`: one second of wall clock at the default
        /// `time_scale`. See the note on the test.
        const ONE_SECOND: f64 = 0.5;
        const MAX_VERTICAL_DRIFT: f64 = 1.6;

        for size in [(200u16, 50u16), (400, 200), (80, 24)] {
            let ratio = vertical_to_horizontal_drift(size, ONE_SECOND);
            assert!(
                ratio < MAX_VERTICAL_DRIFT,
                "at {size:?} the field drifts {ratio:.2}x as far downwards every \
                 second as it does sideways, so it is going up and down. The old \
                 field measured 5.81 here, and putting the sweep term's depth \
                 back to 2.0 while leaving everything else alone measures 1.88."
            );
        }

        // ...and the lower guard, because a field with no drift at all reads 0
        // and would sail through the bound above. A quarter of a cell a second
        // is well under a tenth of what the field measures.
        let speed = drift_speed((200, 50), ONE_SECOND);
        assert!(
            speed > 0.25,
            "the field's pattern is drifting at {speed:.3} cells a second, so it \
             has stopped travelling altogether -- which satisfies every bound \
             above while drawing a still picture"
        );
    }

    /// The field's net drift in cells a second, which is the quantity
    /// [`vertical_to_horizontal_drift`] normalises away.
    ///
    /// **It used to be 3.14 at 200x50 and is now 0.75**, so the fix cost
    /// three quarters of the picture's net motion. That is the price and it is
    /// worth being blunt about, because it is not a small one: the old field's
    /// motion was 3.1 cells a second and the complaint was that all of it was
    /// in one direction. The field still churns -- the mean |change| at a cell
    /// over a second of wall clock is about 60% of what it was -- so the picture
    /// still boils; it just no longer marches.
    ///
    /// So the number is asserted rather than left implicit, because a field that
    /// has been slowed too far is a screensaver that has stopped, and the only
    /// way to notice is to say how fast it is *supposed* to be. The bound is
    /// 0.25, a third of what it measures and a quarter of what the old field
    /// did, so it catches a field that has been flattened without objecting to
    /// one that has merely been calmed.
    fn drift_speed(size: (u16, u16), dt: f64) -> f64 {
        let mut total = 0.0f64;
        let mut count = 0.0f64;
        for seed in SAMPLE_SEEDS {
            let tuning = FieldTuning::from_seed(seed);
            for now in DRIFT_SAMPLES {
                let (ux, uy) = advection_velocity(size, now, dt, &tuning);
                total += (ux * ux + uy * uy).sqrt();
                count += 1.0;
            }
        }
        total / count
    }

    /// There has to be something in the field that is not stripes and not rings.
    ///
    /// Every term the field had was a function of `x` alone, `y` alone, or `r`
    /// alone, so the picture was assembled from axis-aligned bands and
    /// concentric ones and read as bands sliding past rather than as something
    /// with a grain. The canonical demoscene plasma has an `(x + y) / 2` term for
    /// exactly that reason and this one did not have one.
    ///
    /// **How a missing diagonal is detected without looking for one.** A field
    /// made of `x`, `y` and `r` has no preferred diagonal: walk a cell down and
    /// right, or up and right, and on average you cross the same amount of the
    /// field's structure either way, because reversing the sign of the `y` step
    /// changes which part of a band you land on and not how much of it you
    /// cross. A term in `x + y` breaks that, because `x + y` increases on one of
    /// those two walks and decreases on the other. So the asymmetry between the
    /// two diagonals is the term's fingerprint, and it is zero for a field
    /// without one.
    ///
    /// Measured over [`SAMPLE_SEEDS`] and [`DRIFT_SAMPLES`]:
    ///
    /// ```text
    ///             old    new
    ///   200x50    0.005  0.111
    ///   400x200   0.002  0.112
    ///   80x24     0.006  0.112
    /// ```
    ///
    /// **The new figures are the same at all three sizes to three decimals**,
    /// which is not a coincidence and is the reason to believe them. The
    /// asymmetry is a property of the *term*, and the term's contribution to it
    /// is the same fraction of the field's total rate wherever it is measured;
    /// the sizes differ in how much of the screen the other four terms cover, and
    /// the diagonal is a small share of the total at all of them.
    ///
    /// The old figures are not exactly zero and the reason is worth having,
    /// because it is a trap: the measurement is a *finite* screen, and a
    /// rectangle's mean |change| along two mirrored diagonals is not
    /// identically equal even for a field with no preferred diagonal. So 0.005
    /// is the noise floor of the measurement, and it is a quarter of the bound.
    ///
    /// What does **not** produce a false positive is worth stating as well,
    /// because it looked like it might: a *static* term in `x + y` breaks the
    /// symmetry in principle, since `x + y` increases on one walk and decreases
    /// on the other. It does not in practice, and the reason is that the mean
    /// |change| of a periodic function over many periods is the same wherever
    /// the window starts, so a term that is not moving contributes equally to
    /// both walks and cancels. The asymmetry is evidence of a *moving*
    /// diagonal, which is the one that shows up on screen.
    ///
    /// The bound is 0.05: a factor of five below what the field measures and a
    /// factor of eight above the old field's largest. Per-configuration the
    /// figure ranges from 0.006 to 0.195 at 80x24, so it is the mean over seeds
    /// that is asserted rather than every draw; see
    /// [`mean_diagonal_asymmetry`].
    #[test]
    fn the_field_has_a_diagonal() {
        const MIN_ASYMMETRY: f64 = 0.05;

        for size in [(200u16, 50u16), (400, 200), (80, 24)] {
            let asymmetry = mean_diagonal_asymmetry(size);
            assert!(
                asymmetry > MIN_ASYMMETRY,
                "at {size:?} the field changes by the same amount along the two \
                 diagonals to within {:.1}%, so its structure is still only \
                 stripes and rings: every term is a function of x, of y or of \
                 the radius, and the field has no grain",
                asymmetry * 100.0
            );
        }
    }

    /// The two ring terms must not be one wave and its second harmonic.
    ///
    /// They were `sin(r/4)` and `sin(r/8)`: the same pattern at exactly 2 : 1.
    /// Two sines of one argument in a harmonic ratio are not two waves. They
    /// add coherently at every radius, and their sum is a single clean beat with
    /// a smooth envelope -- which is what the screen showed, hard concentric
    /// bands marching outwards with no interference anywhere in them.
    ///
    /// **Measured as the repeat, not as the ratio.** "Not 2 : 1" is a claim about
    /// two numbers and it is the wrong one to assert: the seed now draws both
    /// divisors, so *any* pair can come up, and a test that forbade 2 : 1 would
    /// be forbidding a legal draw rather than a defect. What is not a matter of
    /// luck is the distance at which the pair comes back to itself, because a
    /// pair that repeats inside the screen draws the same rings twice and a pair
    /// that repeats outside it does not. That distance is what is asserted.
    ///
    /// The old pair repeated every `16*pi` = 50.3 corrected units. A 200x50
    /// screen is 209 units corner to corner, so the old field drew its radial
    /// structure twice across its own diagonal -- once near the centre and again
    /// in the corners, identical. The centres now give 4.0 and 5.5, a ratio of
    /// 11 : 8, whose joint repeat is `88*pi` = 276 units: past the corner of the
    /// size the calibration is fitted at.
    ///
    /// It is worth being straight about the limit of that. 88 is still a number
    /// a terminal can reach: at 400x200, which is 466 units corner to corner,
    /// the repeat is 59% of the way across. What the change bought is 5.5x the
    /// distance, not an infinite one, and the arithmetic is against an infinite
    /// one -- every `f64` is a dyadic rational, so *any* two of them have a
    /// rational ratio and a joint period, and the seeded draws put theirs
    /// thousands of times further out than 276. The search below is what turns
    /// that from an argument into a measurement.
    #[test]
    fn the_two_ring_terms_do_not_repeat_inside_the_screen() {
        // Searched to a bit over the screen's own diagonal at the size the
        // calibration is fitted at. A thousandth of a unit is 0.0008 of a row,
        // so a repeat is either found or it is not there.
        const CAP: f64 = 240.0;
        const STEP: f64 = 0.001;

        // The centres, first: this is the number the constant note quotes and it
        // is the clearest statement of the change.
        assert_eq!(
            radial_joint_period(
                RING_DIVISOR.center,
                BREATH_DIVISOR.center,
                CAP,
                STEP
            ),
            None,
            "the two ring divisors' centres still come back to themselves within \
             {CAP} corrected units, so they are still in a harmonic relationship"
        );

        // The search finds the old pair's repeat, and at the right distance.
        // Without this the assertion above is worth nothing: a search that finds
        // nothing is indistinguishable from a search that cannot see.
        let old = radial_joint_period(4.0, 8.0, CAP, STEP).expect(
            "the search did not find the old pair's repeat at 16*pi, so it cannot \
             be trusted to report that the new pair has none",
        );
        assert!(
            (old - 16.0 * PI).abs() <= STEP,
            "the old pair's repeat was found at {old:.4} rather than at 16*pi = {}, \
             so the search is measuring something other than the joint period",
            16.0 * PI
        );

        // And then every draw. The centres are not what a launch gets.
        for seed in 0..200u64 {
            let tuning =
                FieldTuning::from_seed(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
            assert_eq!(
                radial_joint_period(
                    tuning.ring_divisor,
                    tuning.breath_divisor,
                    CAP,
                    STEP
                ),
                None,
                "seed {seed} drew ring divisors {:.4} and {:.4}, whose rings \
                 repeat within {CAP} corrected units -- inside the screen",
                tuning.ring_divisor,
                tuning.breath_divisor
            );
        }
    }

    // --- the seed --------------------------------------------------------

    /// The whole point of a seed: a different launch looks different.
    ///
    /// `PlasmaOptions::seed` was added in `b8d85db` and read by nothing. The
    /// field is a pure function of time and so was identical on every launch,
    /// which is the one thing a screensaver cannot be.
    ///
    /// Asserted on the **first frame**, which is the strict version. A test
    /// that let the field run for a few seconds would be satisfied by a seed
    /// that only shifts a phase, and a phase shift is not what "a plasma looks
    /// different on every launch" means -- the launch that matters is the one
    /// where the terminal is still filling up.
    #[test]
    fn two_seeds_draw_different_fields_on_the_first_frame() {
        let size = (120u16, 40u16);
        let first_frame = |seed: u64| {
            let tuning = FieldTuning::from_seed(seed);
            (0..size.1 as usize)
                .flat_map(|y| {
                    (0..size.0 as usize)
                        .map(move |x| value_at_with(&tuning, size, x, y, 0.0))
                })
                .collect::<Vec<f64>>()
        };

        let baseline = first_frame(DEFAULT_SEED);
        for other in [0u64, 1, 43, 987_654_321] {
            let candidate = first_frame(other);
            assert_ne!(
                candidate, baseline,
                "seed {other} draws the same first frame as the default, so the \
                 seed is not reaching the field"
            );

            // Not merely "not equal": measurably different. An `assert_ne` on a
            // Vec of floats passes on a last-bit change, and a field that
            // differed by 1e-16 would be the same picture. A tenth of the range
            // is the bar, which the divisors' 30% bands clear by a wide margin.
            let mean: f64 = candidate
                .iter()
                .zip(&baseline)
                .map(|(a, b)| (a - b).abs())
                .sum::<f64>()
                / baseline.len() as f64;
            assert!(
                mean > 0.1,
                "seed {other} differs from the default by a mean of {mean:.4} on \
                 the first frame, which is the same picture"
            );
        }
    }

    /// The other half of the contract, and the half that is easy to break by
    /// accident.
    ///
    /// A seed that is read *differently* twice -- because the tuning is drawn
    /// from a generator that is rebuilt rather than stored, or because the
    /// divisors are drawn in an order that depends on a hash iteration -- gives
    /// a field that cannot be reproduced, and `--seed` becomes a lie. The
    /// contract suite in `tests/effect_contracts.rs` checks this across two
    /// separately built instances; this checks it against the field function
    /// directly, so a failure points at [`FieldTuning`] rather than at the
    /// runtime.
    #[test]
    fn one_seed_always_draws_the_same_field() {
        let size = (120u16, 40u16);
        for seed in [DEFAULT_SEED, 0, 1, 43, u64::MAX] {
            let first = FieldTuning::from_seed(seed);
            let second = FieldTuning::from_seed(seed);
            assert_eq!(first, second, "seed {seed} drew two different tunings");

            for now in [0.0, 0.5, 3.7] {
                for y in [0usize, 13, 39] {
                    for x in [0usize, 41, 119] {
                        assert_eq!(
                            value_at_with(&first, size, x, y, now),
                            value_at_with(&second, size, x, y, now),
                            "seed {seed} drew different values at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    /// The randomisation cannot produce a degenerate field, on any draw.
    ///
    /// This is the test that a random number with no floor fails, and it fails
    /// it *quietly*: a divisor drawn near zero is a term that is a flat wash
    /// across the whole screen, and a divisor drawn near the screen's width is
    /// a single band. Neither is a wrong answer, a NaN, or an assertion failure
    /// anywhere else in this file -- the picture renders, it is just not plasma,
    /// and a test that only looked for crashes would call it a pass. So the
    /// bounds are the assertion.
    ///
    /// Two hundred seeds, and every one of the five divisors from every one of
    /// them. The bands are multiplicative for the reason on [`DivisorBand`]: a
    /// divisor is a period, so a wobble of a fixed size in cells means something
    /// different at 4 and at 16, and a proportional band keeps the number of
    /// bands on the screen the same fraction of the screen at both ends.
    #[test]
    fn the_seeded_divisors_stay_inside_their_bands() {
        const SEEDS: u64 = 200;

        let bands = [
            ("wave", WAVE_DIVISOR),
            ("sweep", SWEEP_DIVISOR),
            ("ring", RING_DIVISOR),
            ("breath", BREATH_DIVISOR),
            ("diagonal", DIAGONAL_DIVISOR),
        ];

        // The range each band covers across the draws, for the failure message.
        let mut measured: Vec<[f64; 5]> = Vec::new();
        for seed in 0..SEEDS {
            measured.push(draw_divisors(seed));
        }

        for (index, (name, band)) in bands.iter().enumerate() {
            let column: Vec<f64> = measured.iter().map(|row| row[index]).collect();
            let low = column.iter().copied().fold(f64::INFINITY, f64::min);
            let high = column.iter().copied().fold(0.0f64, f64::max);

            // The band's own edges, recomputed here rather than read off the
            // type. If `draw` ever stopped multiplying by `1 - jitter ..=
            // 1 + jitter` -- which is the mistake this test exists for -- the
            // range it draws is still described by the *band*, and these are the
            // numbers it has to stay inside.
            let band_low = band.center * (1.0 - band.jitter);
            let band_high = band.center * (1.0 + band.jitter);
            assert!(
                low >= band_low && high <= band_high,
                "the {name} divisor ranged over {low:.3} to {high:.3} over {SEEDS} \
                 seeds, outside its band of {band_low:.3} to {band_high:.3}: a \
                 divisor near zero is a flat wash and one near the screen's width \
                 is a single band, and neither is a plasma"
            );
            assert!(
                low > 0.0,
                "the {name} divisor was drawn as {low}, so the term has no \
                 structure at all -- the band has lost its floor"
            );
        }

        // The five are not all the same number, which is a different failure:
        // five terms at one frequency are one term five times.
        for (index, (name, _)) in bands.iter().enumerate() {
            let others: Vec<f64> = bands
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, (_, band))| band.center)
                .collect();
            let mine = bands[index].1.center;
            assert!(
                others.iter().all(|other| (other - mine).abs() > 0.5),
                "the {name} divisor's centre is {mine}, which is the same as \
                 another term's to within half a cell, so the two terms are one \
                 term drawn twice"
            );
        }
    }

    /// The five divisors a seed produced, in the order they are summed.
    fn draw_divisors(seed: u64) -> [f64; 5] {
        let tuning =
            FieldTuning::from_seed(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        [
            tuning.wave_divisor,
            tuning.sweep_divisor,
            tuning.ring_divisor,
            tuning.breath_divisor,
            tuning.diagonal_divisor,
        ]
    }

    /// The calibration has to hold for a launch the user did not choose.
    ///
    /// [`VALUE_BOUNDARIES`] is fitted at 200x50, and it used to be fitted to one
    /// field -- which was the whole field, because there was only one. There are
    /// now as many fields as there are seeds, and `randomise_seeds` hands every
    /// unconfigured launch a fresh one, so a table that only suits seed 42 suits
    /// exactly one launch in the world.
    ///
    /// **The floor is the assertion; the ceiling is a guard.** A step that
    /// covers too little of the screen is a character the field effectively does
    /// not draw, and that is the failure a table fitted to the wrong field
    /// produces first: the old boundaries leave the worst of ten launches with
    /// 1.50% on its sparsest step against 2.63% for these. The ceiling cannot
    /// do the same job -- the old table's worst launch peaks at 11.3% and these
    /// at 13.9%, so a bound between them would be asserting that the *new* fit
    /// is worse, which is the opposite of true. The refit buys its floor with
    /// ceiling, and the note on the table says so; the 20% ceiling here is the
    /// same bar `every_glyph_of_the_default_ramp_carries_its_share_of_the_
    /// screen` uses, kept as a backstop rather than as the discriminator.
    ///
    /// Ten seeds, sixty frames each. Not a random sample -- the first ten seeds
    /// anyone would write down, plus the default and the largest `u64` a config
    /// can carry, which is a deliberate attempt to include a bad one. The
    /// default is in the list because it is the one an unconfigured run gets
    /// before `randomise_seeds` fires, and because it is the seed the table was
    /// originally fitted to: leaving it out of the list that checks whether the
    /// table reaches beyond it is precisely the mistake worth naming.
    #[test]
    fn the_calibration_holds_across_seeds_not_just_the_default_one() {
        /// A step covering less than a fiftieth of the screen is not a band.
        ///
        /// A fiftieth, not a sixteenth, because this is a *worst launch* bound
        /// and the fit is over ten different distributions: measured, they run
        /// from 2.6% to 13.9% on the sparsest and busiest steps, and a bound at
        /// a sixteenth would be asserting a uniformity that nine of them do not
        /// have. It sits 24% below what the table achieves and 33% above what
        /// the table it replaced achieved on the same ten seeds.
        const MIN_SHARE: f64 = 0.02;
        const MAX_SHARE: f64 = 0.20;

        let size = (200u16, 50u16);
        for seed in [
            DEFAULT_SEED,
            0u64,
            1,
            7,
            13,
            99,
            1234,
            5150,
            65535,
            987_654_321,
        ] {
            let shares = glyph_shares_for(size, 60, seed);
            let busiest = shares.iter().copied().fold(0.0f64, f64::max);
            let sparsest = shares.iter().copied().fold(1.0f64, f64::min);
            assert!(
                busiest <= MAX_SHARE && sparsest >= MIN_SHARE,
                "seed {seed} puts {busiest:.1}% of the screen on one glyph and \
                 {sparsest:.1}% on another, so the calibration fitted at 200x50 \
                 does not reach every launch: [{shares:?}]"
            );
        }
    }

    // --- repetition -------------------------------------------------------

    /// The field has to stop coming back. The user's complaint, as a number.
    ///
    /// "Right now it just goes up and down; it's not something new" and "make it
    /// unique each time and continuous, not a loop". The loop was exact, not
    /// approximate: the four time terms ran at `now/2`, `now`, `0` and `now/4`,
    /// so their periods were `4pi`, `2pi`, infinite and `8pi`, and the field's
    /// was the least common multiple, **`8*pi` = 25.13 units of `plasma.time`**.
    /// After 50 seconds of wall clock at the default `time_scale`, every cell was
    /// drawing the same character in the same colour it had drawn 50 seconds
    /// earlier, and it would keep doing so indefinitely.
    ///
    /// Asserted twice over, because the two halves fail differently. The
    /// frequency search is the one that catches the bug directly: it looks for a
    /// `T` that advances all five time terms by whole cycles, and against the
    /// old frequencies it finds one on the first grid point worth trying, `8*pi`
    /// itself. The frame comparison is the one that says what the user sees, and
    /// it is deliberately a *weak* threshold -- a hundredth of the range, against
    /// a measured 0.105 at `8*pi` -- because the claim is only "this is a
    /// different field", not "this is an unrelated field".
    ///
    /// The weak threshold needs the guard below it, or the test would also pass
    /// against a field that never changed at all. A constant field has no period
    /// either, so "does not repeat" is vacuously true of it.
    ///
    /// **The search is over five frequencies now, and that is not free.** The
    /// five are `1, sqrt(2), sqrt(3), sqrt(5), sqrt(7)` over a common two, and
    /// the two added are for the diagonal term and the second ring term. Every
    /// pair's ratio is irrational, so there is no period; but "no period" is a
    /// statement about the *exact* reals, and the grid is a finite search
    /// through `f64`s. The closest approach it finds over 0.05 to 200 units is
    /// 0.0105 of a cycle off -- four orders of magnitude outside the tolerance,
    /// and the margin is better than it was with three frequencies, because
    /// every extra frequency is another near-independent constraint. That is the
    /// argument for adding them rather than an accident of the search.
    #[test]
    fn the_field_does_not_come_back_to_where_it_was() {
        // A period `T` has to satisfy `f * T = 2*pi * n` for every time
        // frequency, so `f * T / 2*pi` has to be very nearly an integer. Searched
        // rather than proved: irrationality is not something an `f64` can
        // assert, and a grid at this resolution with this tolerance is the
        // strongest statement available without an arbitrary-precision library.
        // The tolerance is a millionth of a cycle, and the grid is a
        // thousandth of a unit of `plasma.time` -- 0.1s of wall clock at the
        // default scale.
        const SEARCH_FROM: f64 = 0.05;
        const SEARCH_TO: f64 = 200.0;
        const SEARCH_STEP: f64 = 0.001;
        /// How close to a whole cycle counts as one. Generous, because the point
        /// is to be beaten by the *old* frequencies and the old ones miss by
        /// nothing at all.
        const CYCLE_TOLERANCE: f64 = 1.0e-6;

        let frequencies = [
            TIME_FREQ_RIPPLE,
            TIME_FREQ_WAVE,
            TIME_FREQ_SWEEP,
            TIME_FREQ_DIAGONAL,
            TIME_FREQ_BREATH,
        ];
        let mut steps = (SEARCH_TO - SEARCH_FROM) / SEARCH_STEP;
        let mut best: Option<(f64, f64)> = None;
        while steps >= 0.0 {
            let period = SEARCH_FROM + steps * SEARCH_STEP;
            let worst = frequencies
                .iter()
                .map(|f| {
                    let cycles = f * period / (2.0 * PI);
                    (cycles - cycles.round()).abs()
                })
                .fold(0.0f64, f64::max);
            if best.is_none_or(|(_, previous)| worst < previous) {
                best = Some((period, worst));
            }
            if worst <= CYCLE_TOLERANCE {
                panic!(
                    "a period of {period:.3} units advances all {} time \
                     frequencies ({frequencies:?}) by a whole number of cycles, \
                     to within {CYCLE_TOLERANCE:e}, so the field repeats on it",
                    frequencies.len()
                );
            }
            steps -= 1.0;
        }
        let (period, worst) = best.expect("the search ran at least once");
        assert!(
            worst > CYCLE_TOLERANCE,
            "the closest approach to a common period in the whole search was \
             {period:.3} units, off by {worst:.2e} cycles"
        );

        // And what that means on screen. 8*pi is the period the field used to
        // have; 2*pi and pi are two of its divisors, and a field with a period of
        // 8*pi also "repeats" at those in the weak sense that the screen looks
        // the same, so all three are checked.
        for (label, period) in [
            ("8*pi, the period it had", 8.0 * PI),
            ("2*pi", 2.0 * PI),
            ("pi", PI),
        ] {
            let mut total = 0.0f64;
            let mut samples = 0.0f64;
            for now in [0.0, 3.1, 7.7, 11.3, 19.9, 26.3] {
                for y in 0..24usize {
                    for x in 0..80usize {
                        total += (value_at((80, 24), x, y, now)
                            - value_at((80, 24), x, y, now + period))
                        .abs();
                        samples += 1.0;
                    }
                }
            }
            let mean = total / samples;
            assert!(
                mean > 0.01,
                "at {label} the field is back where it was: the mean absolute \
                 difference across a whole 80x24 screen is {mean:.5}, so the \
                 screen looks the same after {period:.2} units of `plasma.time`"
            );
        }

        // The guard the weak threshold needs. Against the old frequencies this
        // was satisfied at every pair above by a mean difference of exactly
        // zero, so a field that had stopped moving would pass all of it.
        let mut drift = 0.0f64;
        let mut samples = 0.0f64;
        for now in [0.0, 3.1, 7.7, 11.3, 19.9, 26.3] {
            for y in 0..24usize {
                for x in 0..80usize {
                    drift += (value_at((80, 24), x, y, now)
                        - value_at((80, 24), x, y, now + 0.5))
                    .abs();
                    samples += 1.0;
                }
            }
        }
        let mean = drift / samples;
        assert!(
            mean > 0.001,
            "the field changes by {mean:.6} in half a unit of `plasma.time`, so it \
             has stopped moving altogether -- which would satisfy every check \
             above while drawing nothing"
        );
    }

    // --- helpers ---------------------------------------------------------

    /// The instants every "does the field go up and down" measurement is taken
    /// at.
    ///
    /// Not consecutive and not on a round step: a round step of `plasma.time`
    /// can land every sample on the same phase of one term's cycle, and the
    /// measurement would then be a statement about that phase rather than about
    /// the field. These are spread over more than the 25.1 units of the period
    /// the field used to have, so the set sees a full turn of every one.
    const DRIFT_SAMPLES: [f64; 6] = [0.0, 0.37, 2.6, 7.1, 19.9, 33.7];

    /// The seeds a field property is measured over.
    ///
    /// [`FieldTuning`] makes every launch a different field, so a property of
    /// "the field" is a property of a *distribution* of fields, and measuring
    /// one seed measures one draw from it. Four is a small sample and the tests
    /// that use it average over them for exactly that reason. 42 is in the set
    /// because it is the default, and 0 and 1 are in it because they are the
    /// seeds a hand-written config is most likely to carry.
    const SAMPLE_SEEDS: [u64; 4] = [DEFAULT_SEED, 0, 1, 987_654_321];

    /// The field's net drift, in cells across and rows down per unit of
    /// `plasma.time`.
    ///
    /// A pattern translating rigidly at `(u_x, u_y)` satisfies
    /// `dv/dt = -(u_x dv/dx + u_y dv/dy)`, so the two-by-two normal equations on
    /// the two spatial derivatives recover the velocity from the whole screen at
    /// once. Three things about that are worth being explicit about:
    ///
    /// - **It is an aggregate.** No single term decides the answer, which is
    ///   what makes it a statement about the picture rather than about the
    ///   formula. A measurement of one term's phase would be a measurement of
    ///   that term.
    ///
    /// - **It is in units, and the units differ.** `u_x` comes back in cells,
    ///   `u_y` in corrected units, and a row is 1.2 of the latter, so `u_y` is
    ///   divided by [`CELL_ASPECT`] before the two are compared. Leaving that
    ///   out is not a rounding error: it inflates every vertical figure in this
    ///   file by 20%, and it does so on both sides of the comparison, so the
    ///   ordering survives and the *numbers quoted in the constant notes* do
    ///   not.
    ///
    /// - **It is a first-order reading.** The field is a sum of five waves at
    ///   different frequencies and they do not translate together, so there is
    ///   no exact velocity; this is the one that best explains the change the
    ///   viewer sees, least squares over every cell. That it is a fit and not an
    ///   identity is why the drift it reports for a *still* field is zero and
    ///   why the test below needs the lower guard as well as the upper one.
    fn advection_velocity(
        size: (u16, u16),
        now: f64,
        dt: f64,
        tuning: &FieldTuning,
    ) -> (f64, f64) {
        /// Half the finite-difference step for the spatial derivatives, in
        /// corrected units. A thousandth of a cell: the value is smooth, so this
        /// is a derivative to six figures and not a difference between two
        /// samples a cell apart.
        const H: f64 = 1.0e-3;

        let (mut sxx, mut sxy, mut syy) = (0.0f64, 0.0f64, 0.0f64);
        let (mut sx, mut sy) = (0.0f64, 0.0f64);
        let scale = PlasmaOptions::default().spatial_scale;
        for y in 0..size.1 as usize {
            for x in 0..size.0 as usize {
                let xu = x as f64;
                let yu = y as f64 * CELL_ASPECT;
                let rate = (sample(tuning, size, xu, yu, now + dt, scale)
                    - sample(tuning, size, xu, yu, now, scale))
                    / dt;
                let dvdx = (sample(tuning, size, xu + H, yu, now, scale)
                    - sample(tuning, size, xu - H, yu, now, scale))
                    / (2.0 * H);
                let dvdy = (sample(tuning, size, xu, yu + H, now, scale)
                    - sample(tuning, size, xu, yu - H, now, scale))
                    / (2.0 * H);
                sxx += dvdx * dvdx;
                sxy += dvdx * dvdy;
                syy += dvdy * dvdy;
                sx += dvdx * rate;
                sy += dvdy * rate;
            }
        }

        let det = sxx * syy - sxy * sxy;
        if det <= f64::EPSILON {
            // No gradient variance at all: a constant field, or a screen too
            // small to have any structure in it. Zero is the honest answer and
            // the test's lower guard is what notices.
            return (0.0, 0.0);
        }
        (
            -(syy * sx - sxy * sy) / det,
            -(-sxy * sx + sxx * sy) / det / CELL_ASPECT,
        )
    }

    /// The field's drift downwards for every row it drifts across, averaged
    /// over [`SAMPLE_SEEDS`] and [`DRIFT_SAMPLES`].
    ///
    /// Averaged, and that is a design decision rather than a convenience. The
    /// per-configuration figure is high-variance -- it depends on which term
    /// happens to dominate the least-squares fit at that instant, which depends
    /// on the phase, which the seed draws -- and measured over the divisor
    /// bands it ranges from 0.0 to 2.3 at 200x50. A bound on a quantity with
    /// that spread would have to be set at its maximum, and a maximum over
    /// configurations is a statement about the unluckiest launch rather than
    /// about the effect. The mean over seeds says what the field does, which is
    /// the question.
    fn vertical_to_horizontal_drift(size: (u16, u16), dt: f64) -> f64 {
        let mut across = 0.0f64;
        let mut down = 0.0f64;
        for seed in SAMPLE_SEEDS {
            let tuning = FieldTuning::from_seed(seed);
            for now in DRIFT_SAMPLES {
                let (ux, uy) = advection_velocity(size, now, dt, &tuning);
                across += ux.abs();
                down += uy.abs();
            }
        }
        let samples = (SAMPLE_SEEDS.len() * DRIFT_SAMPLES.len()) as f64;
        (down / samples) / (across / samples).max(1e-9)
    }

    /// How differently the field changes along the two diagonals.
    ///
    /// The mean |change| from a cell one row down and one column right, against
    /// the same for one row *up* and one column right. A field built only from
    /// `x`, `y` and `r` reads the same both ways -- there is no preferred
    /// diagonal in it, and reversing the sign of the `y` step changes which
    /// part of a band you land on but not how much of it you cross -- so this is
    /// zero for the old field to within the finite screen's own asymmetry. A
    /// term that is a function of `x + y` breaks exactly that, and by exactly
    /// this much.
    fn diagonal_asymmetry(size: (u16, u16), now: f64, tuning: &FieldTuning) -> f64 {
        let scale = PlasmaOptions::default().spatial_scale;
        let (mut down_right, mut up_right) = (0.0f64, 0.0f64);
        for y in 0..size.1 as usize {
            for x in 0..size.0 as usize {
                let xu = x as f64;
                let yu = y as f64 * CELL_ASPECT;
                let here = sample(tuning, size, xu, yu, now, scale);
                down_right +=
                    (sample(tuning, size, xu + 1.0, yu + CELL_ASPECT, now, scale)
                        - here)
                        .abs();
                up_right +=
                    (sample(tuning, size, xu + 1.0, yu - CELL_ASPECT, now, scale)
                        - here)
                        .abs();
            }
        }
        let mean = (down_right + up_right) / 2.0;
        if mean <= 0.0 {
            return 0.0;
        }
        (down_right - up_right).abs() / mean
    }

    /// [`diagonal_asymmetry`] averaged over [`SAMPLE_SEEDS`] and
    /// [`DRIFT_SAMPLES`].
    ///
    /// Averaged for the same reason the drift is, and for one more: measured
    /// per configuration this ranges from 0.006 to 0.195 at 80x24, and the low
    /// end is a real configuration rather than a fluke. A launch can draw a set
    /// of divisors on which the diagonal term's cross-terms happen to cancel,
    /// and the field on that screen is genuinely no more diagonal than the old
    /// one. The claim worth making is that the field is diagonal *in general*,
    /// not that every draw of it is.
    fn mean_diagonal_asymmetry(size: (u16, u16)) -> f64 {
        let mut total = 0.0f64;
        for seed in SAMPLE_SEEDS {
            let tuning = FieldTuning::from_seed(seed);
            for now in DRIFT_SAMPLES {
                total += diagonal_asymmetry(size, now, &tuning);
            }
        }
        total / (SAMPLE_SEEDS.len() * DRIFT_SAMPLES.len()) as f64
    }

    /// The smallest shift in `r` at which both ring terms come back to where
    /// they were, or `None` if there is none within the search.
    ///
    /// `ring(r)` and `breath(r)` are sines of `r / divisor`, so each repeats
    /// every `2*pi*divisor` and the pair repeats every common multiple of the
    /// two. Found by search rather than by least common multiple, because the
    /// divisors are drawn from the seed and are arbitrary `f64`s: their ratio
    /// is a ratio of dyadic rationals, so the arithmetic answer is enormous and
    /// the interesting question is whether a repeat is anywhere near the screen.
    ///
    /// **The tolerance is the grid's, not a number.** A search at step `s` can
    /// only find a repeat at a distance `p` if some grid point lands within
    /// `s / 2` of it, and `s / 2` of shift is `s / (4*pi*divisor)` of cycle for
    /// the coarser of the two. A fixed cycle tolerance smaller than that
    /// searches for a repeat it cannot represent and finds nothing -- which is
    /// how the first version of this passed against the *old* pair, whose
    /// repeat at `16*pi` = 50.2655 is not a multiple of the 0.001 grid. The
    /// tolerance is derived from the step so the search and the grid cannot
    /// disagree about what the grid can see.
    fn radial_joint_period(
        first: f64,
        second: f64,
        cap: f64,
        step: f64,
    ) -> Option<f64> {
        let coarsest = first.min(second);
        let tolerance = 0.75 * step / (2.0 * PI * coarsest);
        let mut shift = step;
        while shift <= cap {
            let cycles = |divisor: f64| {
                let c = shift / (2.0 * PI * divisor);
                (c - c.round()).abs()
            };
            if cycles(first) <= tolerance && cycles(second) <= tolerance {
                return Some(shift);
            }
            shift += step;
        }
        None
    }

    /// The tuning the default options produce, rebuilt on each call.
    ///
    /// Not a `static` or a cached `OnceLock`: a shared one would be a mutable
    /// global, and the point of `FieldTuning` is that a field is a value two
    /// effects can hold at once without either of them seeing the other's seed.
    fn default_tuning() -> FieldTuning {
        FieldTuning::from_seed(PlasmaOptions::default().seed)
    }

    /// The value one cell was drawn from, recovered from the same inputs
    /// `update_plasma` uses so a test can pair a drawn cell with its value.
    ///
    /// Reads the *drawn* effect's tuning rather than a default, so a test that
    /// builds a seeded effect and one that uses the default agree on what the
    /// screen was drawn from. Callers that only ever use the default are
    /// unaffected; callers that do not have an effect in hand want
    /// [`value_at_with`].
    fn value_at(size: (u16, u16), x: usize, y: usize, now: f64) -> f64 {
        value_at_with(&default_tuning(), size, x, y, now)
    }

    /// [`value_at`] against an explicitly named tuning.
    fn value_at_with(
        tuning: &FieldTuning,
        size: (u16, u16),
        x: usize,
        y: usize,
        now: f64,
    ) -> f64 {
        sample(
            tuning,
            size,
            x as f64,
            y as f64 * CELL_ASPECT,
            now,
            PlasmaOptions::default().spatial_scale,
        )
    }

    /// The value at one point, at a given time, in corrected units.
    fn sample(
        tuning: &FieldTuning,
        size: (u16, u16),
        x: f64,
        y: f64,
        now: f64,
        scale: f64,
    ) -> f64 {
        tuning.value(
            x,
            y,
            now,
            f64::from(size.0),
            f64::from(size.1) * CELL_ASPECT,
            scale,
        )
    }

    /// Draws a frame and returns every cell paired with its value, ascending.
    fn drawn_by_value(plasma: &mut Plasma, size: (u16, u16)) -> Vec<(f64, char)> {
        let now = plasma.time;
        let mut by_value: Vec<(f64, char)> = plasma
            .get_diff()
            .iter()
            .map(|(x, y, cell)| (value_at(size, *x, *y, now), cell.symbol))
            .collect();
        by_value.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        by_value
    }

    /// Mean |change| between adjacent rows, over the top four rows and over the
    /// bottom four, at one instant.
    fn row_change_ratio(size: (u16, u16), now: f64) -> (f64, f64) {
        let scale = PlasmaOptions::default().spatial_scale;
        let tuning = default_tuning();
        let mut per_row = vec![0.0f64; size.1 as usize];
        for (y, row) in per_row.iter_mut().enumerate() {
            for x in 0..size.0 as usize {
                let xu = x as f64;
                *row += (sample(
                    &tuning,
                    size,
                    xu,
                    (y + 1) as f64 * CELL_ASPECT,
                    now,
                    scale,
                ) - sample(
                    &tuning,
                    size,
                    xu,
                    y as f64 * CELL_ASPECT,
                    now,
                    scale,
                ))
                .abs();
            }
        }

        let width = f64::from(size.0);
        let mean =
            |rows: &[f64]| rows.iter().sum::<f64>() / rows.len() as f64 / width;
        (mean(&per_row[..4]), mean(&per_row[per_row.len() - 4..]))
    }
}
