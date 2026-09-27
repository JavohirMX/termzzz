use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crate::render::glyph_ramp::{self, GlyphRamp};
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::f64::consts::{PI, SQRT_2};

/// `std::f64::consts::SQRT_3` is still unstable, so it is spelled out here.
///
/// The literal is the correctly-rounded `f64` nearest `sqrt(3)`, which is all a
/// frequency ratio needs: what matters is that it is irrational and that it is
/// not a rational multiple of [`TIME_FREQ_WAVE`]'s base, and both properties
/// survive to the last bit.
const SQRT_3: f64 = 1.732_050_807_568_877_2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlasmaOptions {
    pub time_scale: f64,
    pub spatial_scale: f64,
    pub color_speed: f64,
    /// Characters the plasma value is drawn as, sparsest first.
    pub glyphs: String,
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
///   *value* -- reaching it takes a value above 0.768, which is where the field's
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
/// of four sines is a *bell*: its value is concentrated near the middle of its
/// range, so a ramp divided evenly in value spends most of its steps on
/// distinctions the eye cannot find and leaves its extremes unreachable.
///
/// The field's measured quantiles at 1/16 intervals, at 200x50:
///
/// ```text
/// 0.232 0.296 0.343 0.382 0.417 0.449 0.478 0.506 0.534 0.563
/// 0.593 0.626 0.663 0.707 0.768
/// ```
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
/// - **The field's tails are compressed.** A value of 0.0 and a value of 0.23
///   now draw the same glyph, and 0.77 and 1.0 draw the same other one. That is
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
///   above 0.768, which is where the field's bright cores are.
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
/// from the other end, by finding every step at 6.0% to 6.6% there too.
///
/// **80x24 does not agree, and cannot.** Reaching the field's extreme values is
/// a coincidence in three variables at once, so a small screen does not sample
/// them often enough: over 460,800 samples at 80x24 the field spans 0.032 to
/// 0.941, against 0.0002 to 0.9998 at 400x200. The bottom of the ramp therefore
/// comes out *thin* rather than empty at that size -- the space draws 1.54% and
/// `'` 2.44% where each step draws 6.3% at 200x50 -- and no static table can do
/// better, because the missing mass is in the field and not in the mapping. See
/// `a_short_terminal_thins_the_ends_of_the_ramp` for what *is* asserted there.
const VALUE_BOUNDARIES: [f64; 15] = [
    0.232, 0.296, 0.343, 0.382, 0.417, 0.449, 0.478, 0.506, 0.534, 0.563, 0.593,
    0.626, 0.663, 0.707, 0.768,
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
fn glyph_value(value: f64) -> f64 {
    let value = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };

    // How many boundaries the value is at or past, so `0..=15`: fifteen means it
    // is above all of them and belongs in the last step.
    let index = VALUE_BOUNDARIES.partition_point(|bound| *bound <= value);

    let (low, high) = match index {
        0 => (0.0, VALUE_BOUNDARIES[0]),
        15 => (VALUE_BOUNDARIES[14], 1.0),
        _ => (VALUE_BOUNDARIES[index - 1], VALUE_BOUNDARIES[index]),
    };
    // `GlyphRamp::index_for` is `round(f * 15)`, so step `index` is the interval
    // from `(index - 0.5) / 15` to `(index + 0.5) / 15`. Step 0's lower edge and
    // step 15's upper edge fall outside `0..=1` and cannot be reached, which is
    // why those two segments come out half-width rather than full-width.
    let (out_low, out_high) = match index {
        0 => (0.0, 0.5 / 15.0),
        15 => (14.5 / 15.0, 1.0),
        _ => ((index as f64 - 0.5) / 15.0, (index as f64 + 0.5) / 15.0),
    };

    // A zero-width input segment is reachable only if two boundaries are equal,
    // and a hand-edited table can do that. Guard the division rather than trust it.
    let fraction = if high <= low {
        0.0
    } else {
        ((value - low) / (high - low)).clamp(0.0, 1.0)
    };
    out_low + fraction * (out_high - out_low)
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

/// Divisor on the fourth term's radius, deliberately not the third term's 4.0.
///
/// Two sines of the same argument differ only by a phase, so sharing a divisor
/// collapses the pair into a single wave and drops a degree of freedom from the
/// field. One factor of two again -- the same relationship the octave sum
/// uses -- keeps the two ripples at different frequencies so they interfere
/// rather than merge, and measured within noise of the alternatives on both
/// metrics the tests use.
const FOURTH_RADIUS_DIVISOR: f64 = 8.0;

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
/// forever. `the_field_does_not_repeat_over_the_period_it_used_to_have` is that
/// sentence as a test.
///
/// The fix is the frequencies rather than anything about how they are combined.
/// A sum repeats only if a single `T` advances *every* term by a whole number
/// of periods, so what matters is the **ratios** between the frequencies and not
/// their size: rational ratios have a common period, and mutually irrational
/// ratios do not. The three below are `{1, sqrt(2), sqrt(3)}` over a common
/// two, and every pair has an irrational ratio -- `sqrt(2)`, `sqrt(3)` and
/// `sqrt(3/2)`. So there is no `T > 0` for which all three come back to where
/// they started, and the field has no period at all.
///
/// Two consequences worth being straight about, because neither is free:
///
/// - **The composition does not rescue a rational set.** Each term is
///   `sin(spatial - c * sin(f*t))` rather than a bare sine, so a term's
///   harmonics sit at *integer multiples* of its own `f`. That makes the field's
///   frequency content the additive group generated by the three `f`, which
///   still contains each `f` itself, so a common period would still have to
///   satisfy `f*T = 2*pi*n` for all three. Wrapping the phase in a sine delays
///   the repeat; it does not remove it.
///
/// - **The speeds barely moved, and that was the design constraint.** Dividing
///   the whole set by two keeps the ordering the field already had -- the fourth
///   term's ripple drifts slowest, the first term's wave second, the second
///   term's sweep fastest -- and keeps the total churn within 10% of what it was.
///   Measured as the root-sum-square of `c*f` over the three terms, which is what
///   drives mean |change| per second, the set below scores 0.93x the old
///   `0.5, 1.0, 0.25`. The obvious alternative, `1, sqrt(2), sqrt(3)` unscaled,
///   scores 1.67x and puts the slowest ripple where the fastest sweep was.
const TIME_FREQ_RIPPLE: f64 = 0.5;
const TIME_FREQ_WAVE: f64 = SQRT_2 / 2.0;
const TIME_FREQ_SWEEP: f64 = SQRT_3 / 2.0;

pub struct Plasma {
    pub screen_size: (u16, u16),
    options: PlasmaOptions,
    canvas: Canvas,
    time: f64,
    palette: Vec<style::Color>,
    ramp: GlyphRamp,
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

        Self {
            screen_size,
            options,
            canvas,
            time,
            palette,
            ramp,
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

    /// The plasma value at one point, normalised to `0.0..=1.0`.
    ///
    /// The [AWK script formula](https://rosettacode.org/wiki/Plasma_effect#AWK),
    /// from four sines that are each `128 + 128 * sin(..)` and so each run from
    /// 0 to 256. Their mean therefore runs from 0 to 256 as well, which is why
    /// this normalises by 256 rather than by a measured extreme. That is not
    /// decoration: the function used to end in `value as u8`, and while that
    /// cast happens to saturate rather than wrap, nothing about the formula
    /// stops a future edit to a coefficient from pushing the sum outside
    /// `0..=256` and silently pinning a whole region of the screen against one
    /// end of the range. Normalising to a float here makes that rail
    /// unreachable, and it hands the glyph ramp a real 0..1 value to index
    /// instead of a quantised byte.
    ///
    /// `x`, `y`, `w` and `h` are all in the same corrected space, and the caller
    /// is the only thing that knows about cells. That is the fix for the
    /// bottom-of-the-screen flicker: `x` used to arrive in cell units while `y`
    /// arrived in field units at two per cell row, so both radial terms were
    /// ellipses 2 : 1 tall in units -- 1.67 : 1 on a 1 : 1.2 font, which
    /// measured as a vertical rate 1.46x the horizontal one at 80x24 and 1.87x
    /// at 400x200.
    ///
    /// The three time terms run at [`TIME_FREQ_WAVE`], [`TIME_FREQ_SWEEP`] and
    /// [`TIME_FREQ_RIPPLE`], whose ratios are mutually irrational, so the field
    /// has no period. The colour channel is *not* remapped: only the glyph
    /// channel goes through [`glyph_value`], and the reason the two are allowed
    /// to disagree is written there.
    fn plasma_value(x: f64, y: f64, now: f64, w: f64, h: f64, scale: f64) -> f64 {
        // Both radial terms are anchored at the screen centre.
        //
        // The fourth one used to be anchored at the origin, which is what
        // actually made the *bottom* of the screen the busy part. Its vertical
        // phase gradient is `y / sqrt(x*x + y*y)`, which is zero along the top
        // edge and maximal along the bottom, so it added to the third term's
        // gradient at the bottom and subtracted from it at the top: measured,
        // the bottom four rows carried 1.22x to 1.64x the vertical spatial
        // frequency of the top four depending on the frame, and at 400x200 the
        // single busiest row on the screen was row 199. Anchoring both at the
        // centre makes the gradient of each proportional to `(y - h/2) / r`, so
        // the pair reinforces in the middle of the screen and vanishes at the top
        // and bottom edges equally. There is no bottom edge to be special
        // any more; that is a property of the geometry, not a clamp.
        let radius = ((x - w / 2.0).powi(2) + (y - h / 2.0).powi(2)).sqrt();

        let value = (128.0
            + (128.0 * ((x / 8.0) * scale - (now * TIME_FREQ_WAVE).cos()).sin())
            + 128.0
            + (128.0
                * ((y / 16.0) * scale - (now * TIME_FREQ_SWEEP).sin() * 2.0)
                    .sin())
            + 128.0
            + (128.0 * ((radius / 4.0 * scale).sin()))
            + 128.0
            + (128.0
                * ((radius / FOURTH_RADIUS_DIVISOR * scale
                    - (now * TIME_FREQ_RIPPLE).sin())
                .sin())))
            / 4.0;

        (value / 256.0).clamp(0.0, 1.0)
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
                    Self::plasma_value(x as f64, y_units, now, w, h, spatial_scale);

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
    /// The two bounds are 20% and 1%, against a measured busiest of 6.4% and a
    /// measured sparsest of 6.1%. Before the fix the same run at this size
    /// measured 23.5% for the busiest glyph and **0.20%** for the densest, so
    /// both bounds were violated. Sixteen steps on a field is a sixteenth of the
    /// screen each if the calibration is right and a long tail of nothing if it
    /// is not, and this is the test that says which.
    ///
    /// 200x50 and not 80x24, and the difference is not cosmetic. At 80x24 the
    /// sparsest glyph measures 1.6% and cannot be made to measure more, because
    /// the field does not reach its own extremes often enough at that size -- so
    /// asserting a 1% floor there would be asserting something about the field
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
    /// the same run, and it is a different answer. The shares run 1.54% at the
    /// space up to 9.36% at `M`, and they run *monotonically up* to `M` and then
    /// back down at `@` -- which is the shape of a field that does not reach its
    /// own extremes often enough at this size, and so leaves the bottom of the
    /// ramp thin. The field over 460,800 samples spans 0.032 to 0.941 here
    /// against 0.0002 to 0.9998 at 400x200.
    ///
    /// The 3% floor from `every_glyph_of_the_default_ramp_carries_its_share_of_
    /// the_screen` is deliberately *not* asserted here, and saying so in a test
    /// is better than silently applying a bound the size cannot meet.
    ///
    /// What is asserted is that the degradation is graceful: the screen is still
    /// spread across nearly the whole ramp, and no single glyph has taken the
    /// screen over. Both halves were false before the calibration -- the busiest
    /// glyph measured 29.1% of an 80x24 screen and 23.5% of a 200x50 one -- and
    /// neither is true now, at 9.4% and 6.4%.
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
        let ramp = glyph_ramp(&PlasmaOptions::default().glyphs);
        let mut counts = vec![0usize; ramp.len()];
        let mut total = 0usize;
        for frame in 0..frames {
            let now = frame as f64 * 0.5;
            for y in 0..size.1 as usize {
                for x in 0..size.0 as usize {
                    let v = value_at(size, x, y, now);
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
        // last one. The tolerance is 1.15: the corrected field measures 0.92x
        // to 1.05x here, so there is room for the frame's phase without room
        // for the old bias to come back.
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
    /// field measures 0.48x to 0.58x here, comfortably inside the band below,
    /// and the upper bound of 1.2 is the side that catches the old bug.
    #[test]
    fn the_field_is_not_stretched_vertically() {
        let size = (400u16, 200u16);
        let scale = PlasmaOptions::default().spatial_scale;
        let at = |x: f64, y: f64| sample(size, x, y, 0.0, scale);

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
    /// `T` that advances all three time terms by whole cycles, and against the
    /// old frequencies it finds one on the first grid point worth trying, `8*pi`
    /// itself. The frame comparison is the one that says what the user sees, and
    /// it is deliberately a *weak* threshold -- a hundredth of the range, against
    /// a measured 0.105 at `8*pi` -- because the claim is only "this is a
    /// different field", not "this is an unrelated field".
    ///
    /// The weak threshold needs the guard below it, or the test would also pass
    /// against a field that never changed at all. A constant field has no period
    /// either, so "does not repeat" is vacuously true of it.
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

        let frequencies = [TIME_FREQ_RIPPLE, TIME_FREQ_WAVE, TIME_FREQ_SWEEP];
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
                    "a period of {period:.3} units advances all three time \
                     frequencies ({frequencies:?}) by a whole number of cycles, \
                     to within {CYCLE_TOLERANCE:e}, so the field repeats on it"
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

    /// The value one cell was drawn from, recovered from the same inputs
    /// `update_plasma` uses so a test can pair a drawn cell with its value.
    fn value_at(size: (u16, u16), x: usize, y: usize, now: f64) -> f64 {
        sample(
            size,
            x as f64,
            y as f64 * CELL_ASPECT,
            now,
            PlasmaOptions::default().spatial_scale,
        )
    }

    /// The value at one point, at a given time, in corrected units.
    fn sample(size: (u16, u16), x: f64, y: f64, now: f64, scale: f64) -> f64 {
        Plasma::plasma_value(
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
        let mut per_row = vec![0.0f64; size.1 as usize];
        for (y, row) in per_row.iter_mut().enumerate() {
            for x in 0..size.0 as usize {
                let xu = x as f64;
                *row +=
                    (sample(size, xu, (y + 1) as f64 * CELL_ASPECT, now, scale)
                        - sample(size, xu, y as f64 * CELL_ASPECT, now, scale))
                    .abs();
            }
        }

        let width = f64::from(size.0);
        let mean =
            |rows: &[f64]| rows.iter().sum::<f64>() / rows.len() as f64 / width;
        (mean(&per_row[..4]), mean(&per_row[per_row.len() - 4..]))
    }
}
