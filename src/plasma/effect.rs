use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crate::render::glyph_ramp::{self, GlyphRamp};
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

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
/// Ten steps, the same count `SHADE` has, so the value resolution is unchanged.
/// Ordered lightest-first by estimated coverage, and shaped for this effect
/// specifically:
///
/// - The heavy end is the tail. A smooth field spends most of its *area* near
///   the middle of its value range, so a ramp that is already heavy at its
///   midpoint makes the majority of the screen the darkest-ink thing on it.
///   Every glyph below `#` is under about a third of the cell.
///
/// - The mid-range glyphs differ in *shape* as well as in weight. `SHADE`'s
///   middle is `-`, `=`, `+`, `*`: four variations on a straight stroke, so a
///   whole region of the field is one repeated form and the value it carries is
///   hard to read. `-`, `:`, `;`, `+`, `X` are five different silhouettes.
///
/// - The step sizes are roughly even, so one ramp step is one visible step.
///   The gaps here are 5, 2, 4, 3, 2, 5, 6, 6 and 13 points, and the last is
///   the price: `@` is a ring, an inner bowl and a tail, and there is nothing
///   between `#` and it that a proportional ramp can use without stepping on
///   `%`.
///
/// # The pairs the estimate cannot separate
///
/// Three, and they are named in the test rather than left to be rediscovered:
/// `.-`, `:;` and `;+` are all within three percentage points of coverage, which
/// is the point at which the table is measuring the font rather than the glyph.
/// Their order is a legibility decision. `-` is the single most widely
/// supported character in the repertoire; `:` is the most widely recognised
/// "slightly more than that". `.` is a dot, and a dot is the lightest mark a
/// font can draw. The pairs that are *not* ties and that this ordering gets
/// right in a way `SHADE` does not: `-` before `:` (one mark against two) and
/// `+` before `X` (a bar reaching the cell in both axes against two diagonals
/// that cut across it, so the diagonals are longer but the total is close).
const DEFAULT_GLYPHS: &str = " .-:;+X#%@";

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
            + (128.0 * ((x / 8.0) * scale - (now / 2.0).cos()).sin())
            + 128.0
            + (128.0 * ((y / 16.0) * scale - now.sin() * 2.0).sin())
            + 128.0
            + (128.0 * ((radius / 4.0 * scale).sin()))
            + 128.0
            + (128.0
                * ((radius / FOURTH_RADIUS_DIVISOR * scale - (now / 4.0).sin())
                    .sin())))
            / 4.0;

        (value / 256.0).clamp(0.0, 1.0)
    }

    /// Repaints the field, one cell per plasma sample.
    ///
    /// The cell's colour *and* its glyph are read from the same value, which is
    /// the point: every cell used to be a hard-coded `*` that carried nothing,
    /// so the glyph channel was decoration and the field had one degree of
    /// freedom less than it looked like it had.
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
                    ramp.sample(value as f32),
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
    /// So the sweep is now a whole cycle at a finer step, which examines four
    /// times as much of the field as before. That makes the clause *harder* to
    /// satisfy rather than easier, and it is the clause that was wrong about the
    /// world: the field does reach its top, it just does not get there inside an
    /// eighth of a cycle.
    #[test]
    fn the_rendered_value_is_not_pinned_to_either_end_of_the_ramp() {
        let size = (200u16, 50u16);
        let mut plasma = Plasma::new(PlasmaOptions::default(), size);
        let ramp = plasma.ramp.clone();
        let steps = ramp.len();

        // Sampled over a whole cycle of the field rather than one, because a
        // field this size is not in the same place twice and a single frame --
        // or a fraction of a cycle -- could sit on one side of a rail by luck.
        // The slowest of the four time terms is the fourth, through `sin(now/4)`,
        // which has a period of `8 * pi` in `plasma.time`. Fifty seconds of wall
        // clock at the default `time_scale` of 0.5.
        const CYCLE: f64 = 8.0 * std::f64::consts::PI;
        const FRAMES: u64 = 48;

        let mut at_bottom = 0usize;
        let mut at_top = 0usize;
        let mut total = 0usize;
        let mut used: HashSet<usize> = HashSet::new();

        for frame in 0..FRAMES {
            plasma.time = CYCLE * frame as f64 / FRAMES as f64;
            for (x, y, cell) in plasma.get_diff() {
                let value = value_at(size, x, y, plasma.time);
                let index = ramp.index_for(value as f32);
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
                    ramp.sample(value as f32),
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
            let now = step as f64 * CYCLE / 400.0;
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
             {vmin:.4} to {vmax:.4} over a cycle, so the ramp divides more than \
             the field uses",
            used.len()
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
            ('.', 5.0),  // one small square at the baseline
            ('-', 7.0),  // one thin bar, the full cell width
            (':', 11.0), // two dots stacked
            ('=', 12.0), // two thin bars, the full cell width each
            (';', 14.0), // a dot and a comma, so a dot plus a tail
            ('*', 15.0), // an asterisk: several short strokes, none reaching
            ('+', 16.0), // one full-width bar and one full-height bar
            ('X', 21.0), // two full-cell diagonals, longer strokes than `+`
            ('#', 27.0), // four strokes, two of them full height
            ('%', 33.0), // two rings and a slash
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

        // And the pairs the estimate genuinely cannot separate. Three
        // percentage points of cell coverage is a shade over half a stroke
        // width; below it the table is measuring the font rather than the
        // glyph. These are ties, not inversions -- the ramp may put them in
        // either order, and the order is a legibility decision:
        //
        // - `.-` -- a period and a hyphen, one dot and one bar. A dot is the
        //   lightest mark a font can draw and a bar is one of the heaviest
        //   single-stroke marks, so this gap is the smallest in the ramp
        //   because the table is least sure of it, not because they are close.
        // - `:;` -- a colon and a semicolon differ by a tail hanging off the
        //   lower dot. A tail is a fraction of a dot.
        // - `;+` -- a semicolon against a plus sign. Both are two marks, and a
        //   plus reaches the cell in both axes.
        //
        // Asserted as an exact set, so a glyph moved across one of these
        // boundaries -- or a new tie quietly created -- has to be argued for
        // rather than slipping past.
        const TIE_RESOLUTION: f32 = 3.0;
        const EXPECTED_TIES: &[&str] = &[".-", ":;", ";+"];
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
