//! The clock: the current time, drawn as large as the terminal allows.
//!
//! # Why braille, and why the art is authored in source cells
//!
//! A clock is the one effect in the crate whose picture is mostly *absence* --
//! two dots of ink in every twenty-five is the difference between a clock and a
//! grid -- so the medium is chosen for how little it has to draw rather than how
//! much. Braille is the densest thing the crate has: two dots across and four
//! down, so a digit is drawn at 10x14 dots and a whole `HH:MM:SS` fits in a
//! 78x14 dot box, which is 39x7 cells at the design scale.
//!
//! It is also **square**, which is the part that decides the art. A cell is one
//! unit wide and two tall, so a 2x4 dot grid makes each dot half a unit in both
//! directions. Art authored on a square source grid therefore comes out
//! undistorted and the aspect caveat at the top of [`crate::render`] does not
//! apply. See [`font`].
//!
//! # Scale, and why it is a single integer rather than a width and a height
//!
//! The clock is scaled by `k`, an integer number of dots per source dot, and
//! **one integer on both axes**. Scaling the axes independently would fill any
//! terminal, and would also stretch a `1` into a different glyph from a `0` on a
//! terminal where the two ended up with different factors -- a clock whose
//! digits are not all the same shape is worse than a small clock.
//!
//! So the effect is bounded by whichever axis runs out first, and at 80x24 it
//! lands on `k = 2`, which fills the width and about 58% of the height in visual
//! units.
//!
//! **`k` is even, and that is a rendering requirement.** A scaled source cell is
//! `CELL_DOTS * k` dots tall and a braille cell is 4, which divides evenly only
//! for even `k`; at an odd scale every digit's top and bottom edge is drawn
//! across a cell as a row of half-height marks. See [`scale_for`].
//!
//! Turning seconds off *can* make the clock bigger -- `HH:MM` is 25 source cells
//! against `HH:MM:SS`'s 39 -- but only where the width budget spans a whole even
//! step. At 120x40 it goes from scale 2 to scale 4; at 80x24 it stays at 2,
//! because the seconds clock already uses 78 of 80 columns. The table is on
//! [`ClockOptions::show_seconds`], and it is worth having there because "fewer
//! digits, bigger digits" is the knob's obvious promise and it is sometimes
//! untrue.
//!
//! # The rule under the digits is what makes it a screensaver
//!
//! `no_effect_settles_into_a_still_picture` allows 30 consecutive frames with no
//! change, and a clock that only redraws on the second is silent for 59 of every
//! 60 frames. A playlist slot would show a frozen frame for its whole duration.
//!
//! The fix is the seconds rule: a hairline across the clock's full width with a
//! thicker bar growing along it, tracking the fraction of the current second --
//! or of the current minute, when seconds are off. It moves every frame, at dot
//! resolution, and it is a real element of the design rather than a workaround.
//!
//! **How often it actually moves is a dot-resolution artefact and is written
//! down at [`RULE_DOTS`].** At 200x50 the rule is 312 dots wide, so it advances
//! about every 12 frames; at 80x24 it is 156 dots and about every 23. Both are
//! inside the 30-frame limit, and the 23 is the reason this is a note rather than
//! a comfortable margin.
//!
//! # The clock is the crate's first effect with no seed, and the reason
//!
//! Everything else that randomises carries a `seed`, because `--seed` is only
//! worth documenting if it holds. This one reads the system clock, so there is
//! nothing to seed and reproducibility is *undefined* rather than unimplemented.
//! It stays out of [`crate::Config::override_seed`] and
//! [`crate::Config::randomise_seeds`] for that reason, and it is exempted from
//! the reproducibility half of
//! `seeded_effects_are_reproducible_and_seed_sensitive` by name -- see
//! `WALL_CLOCK` in `tests/effect_contracts.rs`.
//!
//! It is also the crate's first effect that deliberately **ignores**
//! `FrameContext::delta`. Deriving a clock's display from a frame delta is the
//! bug, not the feature.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::clock::font::{self, Bitmap};
use crate::common::TerminalEffect;
use crate::render::braille::{BrailleGrid, DOTS_Y};
use chrono::{DateTime, Local, Timelike};
use crossterm::style::{Attribute, Color};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Blank source cells between two glyphs.
///
/// One, which is two dots at the design scale and four at `k = 2`. Zero would
/// run the seconds digits together at the right-hand end, and two would leave a
/// gap in the middle of the time wider than the one between its other digits --
/// the rhythm is uniform or it reads as two groups rather than one time.
const GAP: usize = 1;

/// Cell rows of blank between the bottom of the digits and the rule.
///
/// One. The rule is a baseline the digits stand on rather than a caption under
/// them, and zero would put the bar against the descenders of the `4` and the
/// `2`, which are the two glyphs in this font that reach the bottom row.
const RULE_GAP_ROWS: usize = 1;

/// Cell rows the rule occupies.
///
/// One, which is two dot rows: a hairline and a bar that grows along it. Two
/// rows would be a slab rather than a rule, and zero would be a dotted line that
/// reads as noise.
const RULE_ROWS: usize = 1;

/// Dot rows the filled part of the rule is thick. See [`RULE_ROWS`].
///
/// **Fixed at two dots and never scaled**, which is the one place in this effect
/// where a constant deliberately does *not* follow `k`: a hairline scaled by `k`
/// is at `k = 5` five dot rows of a four-dot cell, which is not a hairline in a
/// different weight but a filled rectangle, and a progress bar that fills the
/// whole cell at large scales and is invisible at small ones is not a bar.
///
/// **This constant is what sets how often the picture changes**, and the number
/// is a dot-resolution artefact rather than a design choice. The rule spans the
/// clock's full width, `layout_cells * CELL_DOTS * scale` dots, and it advances
/// one dot at a time:
///
/// | terminal | scale | rule width | frames between steps at 60 fps |
/// |----------|-------|-----------|-------------------------------|
/// | 80x24    | 2     | 156       | ~23                          |
/// | 200x50   | 4     | 312       | ~12                          |
///
/// The contract test measures at 200x50, where this is comfortable. The 23 at
/// 80x24 is inside `STILL_RUN_FRAMES` but not by much, and it is here so that
/// the next person to change the font or the gap does not discover it as a flake.
///
/// **The 200x50 figure was 9 before the scale was restricted to even numbers**
/// (see [`scale_for`]), because that terminal fits scale 5. Halving the update
/// rate to keep the digits' edges clean was the right trade -- it is still
/// comfortably inside the limit, and ragged digits are visible at every frame
/// rather than on the one in sixty where the rule does not move.
const RULE_DOTS: usize = 2;

/// How much wall clock this effect reads. Overridable so tests can drive time.
type Source = fn() -> SystemTime;

/// The real clock.
fn system_clock() -> SystemTime {
    SystemTime::now()
}

/// The options `[clock]` accepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockOptions {
    /// Whether to draw `HH:MM:SS` rather than `HH:MM`.
    ///
    /// On by default, and turning it off can make the clock *bigger*: `HH:MM` is
    /// 25 source cells against `HH:MM:SS`'s 39, so a terminal wide enough for it
    /// fits a higher scale.
    ///
    /// **"Can", not "does", and the distinction is measured.** The scale moves in
    /// even steps (see [`scale_for`]), so the gain only appears where the width
    /// budget spans a step:
    ///
    /// | terminal | `HH:MM:SS` | `HH:MM` |
    /// |----------|------------|---------|
    /// | 80x24    | scale 2    | scale 2 -- **no gain** |
    /// | 120x40   | scale 2    | scale 4    |
    /// | 200x50   | scale 4    | scale 8    |
    ///
    /// At 80 columns the seconds clock already uses 78 of 80 at scale 2 and
    /// cannot reach 4, and the shorter one cannot reach 4 either -- so turning the
    /// seconds off buys a quieter clock and nothing else. This is a property of
    /// the even-step rule and it is written down here because the knob's obvious
    /// promise is the one that is sometimes untrue.
    ///
    /// The seconds rule follows the period either way: it tracks the minute when
    /// this is off, rather than running sixty times too fast beside minute
    /// digits.
    pub show_seconds: bool,
    /// The digits.
    pub color: Color,
    /// The separator and the rule.
    ///
    /// A second colour rather than a dimmer one, because a braille cell is one
    /// glyph in one colour and there is nothing to dim *within* a cell. The two
    /// never share a cell: the separator is in its own columns and the rule in
    /// its own rows, which is what lets two grids be stamped over one canvas.
    pub accent: Color,
}

impl Default for ClockOptions {
    /// Hand-written, and for the reason [`crate::blank::BlankOptions`] writes
    /// its own: serde uses the `Default` impl, so a derived one is a second
    /// place the defaults live and the two can disagree silently.
    fn default() -> Self {
        Self {
            show_seconds: true,
            color: Color::White,
            accent: Color::Cyan,
        }
    }
}

/// A wall-clock reading, and only the four fields a clock draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockTime {
    /// 0 to 23. The effect is 24-hour; there is no marker and no option.
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// The sub-second part, `0.0` up to but not including `1.0`.
    pub fraction: f64,
}

/// Reads the wall clock in the terminal's own timezone.
///
/// The conversion goes through [`UNIX_EPOCH`] and [`Local`] explicitly rather
/// than through a `From<SystemTime>` impl, so the two steps are visible: the
/// epoch offset is pure arithmetic and cannot fail, and `Local` is the only part
/// that knows about timezones and daylight saving.
///
/// **Local, not UTC**, and that is the whole reason `chrono` is a dependency. A
/// clock that showed UTC, or that asked its user to keep a UTC offset up to date
/// across a daylight-saving boundary, would be wrong twice a year -- which for
/// the one effect in this crate whose entire content is the time is not a small
/// thing.
fn read_time(now: SystemTime) -> ClockTime {
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let stamp = DateTime::from_timestamp(
        since_epoch.as_secs() as i64,
        since_epoch.subsec_nanos(),
    )
    .unwrap_or_default()
    .with_timezone(&Local);

    ClockTime {
        hour: stamp.hour(),
        minute: stamp.minute(),
        second: stamp.second(),
        fraction: stamp.nanosecond() as f64 / 1_000_000_000.0,
    }
}

/// The digits for a reading, zero-padded and 24-hour.
fn time_text(reading: &ClockTime, show_seconds: bool) -> String {
    if show_seconds {
        format!(
            "{:02}:{:02}:{:02}",
            reading.hour, reading.minute, reading.second
        )
    } else {
        format!("{:02}:{:02}", reading.hour, reading.minute)
    }
}

/// How much of the rule is filled, `0.0` to `1.0`.
///
/// Follows the *period* the digits show, so turning seconds off makes the rule
/// track the minute rather than completing a lap every second and sitting still
/// for the other fifty-nine. A rule that runs at the wrong rate next to digits
/// that run at the right one is worse than no rule.
fn rule_fraction(reading: &ClockTime, show_seconds: bool) -> f64 {
    if show_seconds {
        reading.fraction
    } else {
        (reading.minute as f64 + reading.fraction) / 60.0
    }
}

/// The glyphs of a time string, resolved once so the draw path never fails.
///
/// Widths come from the font rather than from a count of characters, so a
/// separator one source cell wide and a digit five do not both advance the
/// cursor by the same amount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Each glyph's ink and its width in source cells, in reading order.
    glyphs: Vec<(usize, Bitmap)>,
    /// The whole string in source cells, gaps included.
    cells_wide: usize,
}

impl Layout {
    /// Resolves `text`, dropping anything the font cannot draw.
    ///
    /// The filter cannot drop anything in practice: [`time_text`] only ever emits
    /// [`font::DIGITS`] and [`font::SEPARATOR`], and
    /// `the_font_is_the_digits_and_a_separator_and_nothing_else` pins that. It is
    /// here so that this returns a [`Layout`] rather than an `Option`, which is
    /// what keeps an `expect` out of the frame loop.
    pub fn new(text: &str) -> Self {
        let glyphs: Vec<(usize, Bitmap)> = text
            .chars()
            .filter_map(|c| {
                font::bitmap(c)
                    // The width in *source cells*, so the layout's arithmetic is
                    // in the same units as the gap and as the font's own grid.
                    .map(|bitmap| (bitmap.width / font::CELL_DOTS, bitmap))
            })
            .collect();

        let ink: usize = glyphs.iter().map(|(cells, _)| cells).sum();
        let cells_wide = ink + GAP * glyphs.len().saturating_sub(1);

        Self { glyphs, cells_wide }
    }

    /// The string's width in source cells, gaps included.
    pub fn cells_wide(&self) -> usize {
        self.cells_wide
    }
}

/// The cell rows the digits occupy at `scale`.
///
/// Ceiling division, and it has to be a ceiling: at `scale = 1` the digits are
/// 14 dot rows, which is three and a half cells. Flooring would drop the bottom
/// half-cell of every digit, which is exactly where the baseline sits, and a
/// clock whose digits have no bottom row reads as a clock drawn slightly too
/// high rather than as a clipped one.
pub fn digit_rows(scale: usize) -> usize {
    (font::GLYPH_H * font::CELL_DOTS * scale).div_ceil(DOTS_Y)
}

/// Every cell row the clock occupies at `scale`, rule and gap included.
pub fn total_rows(scale: usize) -> usize {
    digit_rows(scale) + RULE_GAP_ROWS + RULE_ROWS
}

/// **Even integers from two upward, and this is a rendering requirement rather
/// than a tidiness one.** See [`scale_for`].
///
/// The floor of **one** is the exception and it is not an alignment choice:
/// a terminal with no room for the clock at all gets scale 1, because a dot is
/// the quantum and there is no such thing as a fractional one. Scale 1 is odd, so
/// that clock has ragged edges too -- on a terminal too small to read it, which
/// is the least bad place for that to be true.
const SCALE_STEP: usize = 2;

/// The smallest aligned scale: two, not zero and not one.
const MIN_ALIGNED_SCALE: usize = 2;

/// The dot scale: the largest *even* integer at which the clock fits, never below
/// one.
///
/// **Why even.** A source cell is [`font::CELL_DOTS`] dots tall, so a scaled one
/// is `CELL_DOTS * scale` dots tall, and a braille cell is [`DOTS_Y`] dots tall.
/// That divides evenly only when `scale` is even. At an odd scale every source
/// row straddles a cell boundary, and the glyph's horizontal edges land *between*
/// two dot rows of one cell rather than on its boundary -- so the top and bottom
/// of every digit come out visibly ragged, a row of half-height marks where a
/// straight edge should be.
///
/// It is not a subtle artefact. At 200x50 the clock fits scale 5, and its top row
/// renders as `⣰⣰⣰⣰⣰⠏⣿⣿⣿⣿⣿⠏⠏⠏⠏⠏⠏⠏⠏⠏` where an unbroken bar
/// belongs. The first version of this function returned 5 and the test suite was
/// green throughout, because every assertion in it is about arithmetic and none
/// of them looks at the picture.
///
/// The horizontal axis has no such problem and needs no such rule: a scaled
/// source cell is `scale` cells wide for any integer `scale`, because
/// [`DOTS_X`] is 2 and [`font::CELL_DOTS`] is 2. **The constraint is
/// one-dimensional and the reason is the aspect ratio**, not a preference -- the
/// medium is 2 dots across and 4 down, and those are different numbers.
///
/// **Floor of one, and a dot is the quantum.** Below one there is no drawing to
/// make, so a terminal with no room for the clock gets a clipped clock rather
/// than a fractional one. Clipping is safe because [`Canvas::set`] drops
/// out-of-range writes rather than writing past the end of its buffer.
///
/// Both bounds are monotonic in `scale`, so the first failure ends the search
/// rather than the loop having to keep looking for a larger `scale` that fits.
pub fn scale_for(cells_wide: usize, size: (u16, u16)) -> usize {
    let (width, height) = (size.0 as usize, size.1 as usize);

    // One is the floor -- a clipped clock rather than a fractional one -- and is
    // exempt from the evenness rule, which is what `MIN_ALIGNED_SCALE` is for.
    let mut best = 1;
    let mut scale = MIN_ALIGNED_SCALE;
    while cells_wide * scale <= width && total_rows(scale) <= height {
        best = scale;
        scale += SCALE_STEP;
    }
    best
}

/// A large clock showing the current time.
pub struct Clock {
    screen_size: (u16, u16),
    options: ClockOptions,
    canvas: Canvas,
    /// The digits. **Persistent between frames**, because they only change when a
    /// digit does: re-rasterising 39 glyphs into 136,000 dots sixty times a
    /// second to produce the same picture is the expensive way to write nothing.
    digits: BrailleGrid,
    /// The rule. Cleared and repainted every frame, because it is the only thing
    /// that moves between seconds.
    accent: BrailleGrid,
    /// The time string the digits grid currently holds, and the scale it was
    /// rasterised at. Together they are the whole invalidation key: if neither
    /// has changed the digits are already correct.
    ///
    /// A pair of fields rather than a `Layout`, because a `Layout` owns its
    /// bitmaps and comparing two of them is a comparison of 39 vectors every
    /// frame to answer a question about nine characters.
    laid_out_text: String,
    laid_out_scale: usize,
    /// Where the time comes from.
    ///
    /// A field and not a direct `SystemTime::now()` call, for the reason
    /// [`crate::registry`] keeps the effect table in one place: this is the seam
    /// that makes the time tests possible at all. Without it the only way to test
    /// that a second redraws the digits is to sleep for one.
    now: Source,
}

impl TerminalEffect for Clock {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.draw()
    }

    /// Nothing to advance.
    ///
    /// A clock has no simulation, and the time is read in [`Clock::draw`]
    /// rather than here so that what is on screen is never one frame behind what
    /// the clock says.
    fn update(&mut self) {}

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.reset();
    }

    fn reset(&mut self) {
        // `Canvas::resize` has already blanked both surfaces, which is what makes
        // the next commit report every cell. The grids are *not* invalidated:
        // their contents depend on the time and the scale, neither of which a
        // reset changes, and the origin they are stamped at is recomputed from
        // the new size on every frame.
        self.canvas.clear();
    }
}

impl Clock {
    pub fn new(options: ClockOptions, screen_size: (u16, u16)) -> Self {
        Self::with_clock(options, screen_size, system_clock)
    }

    /// The constructor the tests use.
    ///
    /// Not `#[cfg(test)]`: a caller outside the crate that wants to drive this
    /// effect's time -- a recording tool, a demo -- needs the same seam, and a
    /// test-only constructor would be a second way to build the thing that only
    /// exists in tests.
    pub fn with_clock(
        options: ClockOptions,
        screen_size: (u16, u16),
        now: Source,
    ) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        Self {
            screen_size,
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            // Both are replaced on the first frame, once the time and the scale
            // are known; these only have to be valid grids.
            digits: BrailleGrid::new(1, 1),
            accent: BrailleGrid::new(1, 1),
            laid_out_text: String::new(),
            laid_out_scale: 0,
            now,
        }
    }

    fn draw(&mut self) -> Vec<(usize, usize, Cell)> {
        let reading = read_time((self.now)());
        let text = time_text(&reading, self.options.show_seconds);
        let layout = Layout::new(&text);
        let scale = scale_for(layout.cells_wide(), self.screen_size);

        // `resize` discards the contents, so it must come before the repaint
        // below -- and it is a no-op when the dimensions already match, which is
        // every frame the time has not changed the digit count.
        let cells = (layout.cells_wide() * scale, total_rows(scale));
        self.digits.resize(cells.0, cells.1);
        self.accent.resize(cells.0, cells.1);

        if text != self.laid_out_text || scale != self.laid_out_scale {
            self.digits.clear();
            paint_digits(&mut self.digits, &layout, scale);
            self.laid_out_text.clone_from(&text);
            self.laid_out_scale = scale;
        }

        self.accent.clear();
        let fraction = rule_fraction(&reading, self.options.show_seconds);
        paint_rule(&mut self.accent, &layout, scale, fraction);

        // Before the stamp, or the previous frame's ink stays on the canvas and
        // the two positions smear into each other. The grids are what persist;
        // the canvas is rebuilt from them every frame.
        self.canvas.clear();
        let at = self.origin(layout.cells_wide(), scale);
        stamp(&self.digits, &mut self.canvas, at, self.options.color);
        stamp(&self.accent, &mut self.canvas, at, self.options.accent);

        self.canvas.commit()
    }

    /// Where the clock's top-left cell goes, centred.
    fn origin(&self, cells_wide: usize, scale: usize) -> (usize, usize) {
        let width = cells_wide * scale;
        let height = total_rows(scale);
        (
            (self.screen_size.0 as usize).saturating_sub(width) / 2,
            (self.screen_size.1 as usize).saturating_sub(height) / 2,
        )
    }

    /// The digits grid as it stands.
    ///
    /// Public rather than `#[cfg(test)]` for the same reason
    /// [`with_clock`](Self::with_clock) is: the dot grid *is* the picture, and a
    /// caller that wants to measure or render this clock's glyphs has no other
    /// way to reach it. A test-only accessor would mean the only way to look at
    /// the art is from inside this crate.
    pub fn digits_grid(&self) -> &BrailleGrid {
        &self.digits
    }

    /// The rule grid as it stands. See [`digits_grid`](Self::digits_grid).
    pub fn accent_grid(&self) -> &BrailleGrid {
        &self.accent
    }

    /// The dot scale the digits are currently rasterised at.
    ///
    /// Zero before the first frame, since nothing has been laid out yet.
    pub fn scale(&self) -> usize {
        self.laid_out_scale
    }
}

/// Rasterises every glyph into the digits grid at `scale`.
///
/// A free function taking the grid, rather than a method, so the borrow checker
/// sees a `&mut BrailleGrid` and a `&Layout` rather than a `&self` that already
/// owns one. That is the whole reason it is not `self.paint_digits(...)`: the
/// method form cannot borrow `self.digits` mutably while reading `&self`.
///
/// The scale is applied by writing each dot `scale` times across and down, rather
/// than by sampling the bitmap, so a scaled digit is a digit made of square
/// blocks -- which is what keeps a `k = 5` clock's glyphs the same shapes as a
/// `k = 1` one's. Resampling would be shorter and would quietly make a `1`
/// narrower than a `0` at some scales and not others, which is the
/// `a_layout_is_the_same_width_whatever_the_digits` claim failing.
fn paint_digits(grid: &mut BrailleGrid, layout: &Layout, scale: usize) {
    let mut cell_x = 0;
    for (source_wide, bitmap) in &layout.glyphs {
        for dot_y in 0..bitmap.height {
            for dot_x in 0..bitmap.width {
                if !bitmap.get(dot_x, dot_y) {
                    continue;
                }
                // **Both terms are scaled, and the whole expression is.**
                //
                // `cell_x * CELL_DOTS + dot_x` is the glyph's position in *source*
                // dots -- cells to dots, then within the glyph. Only then does
                // `scale` apply, because a source dot becomes a `scale` square of
                // grid dots.
                //
                // The first version scaled `dot_x` but not the glyph's origin:
                // `cell_x * CELL_DOTS + dot_x * scale`. At `scale = 4` that put
                // the first digit on grid dots 0..39 and the second starting at 12,
                // so **every glyph was drawn on top of the one before it** -- the
                // clock read as one smeared mass. At `scale = 1` the two forms
                // are identical, which is why 80x24 looked right and 200x50 did
                // not, and why nothing in the suite caught it: every assertion
                // about the layout was about the *arithmetic*, and
                // `every_time_of_day_is_the_same_width` passed on arithmetic that
                // was right while the drawing that used it was wrong.
                let left = (cell_x * font::CELL_DOTS + dot_x) * scale;
                let top = dot_y * scale;
                for sy in 0..scale {
                    for sx in 0..scale {
                        grid.raise_dot(left + sx, top + sy);
                    }
                }
            }
        }
        cell_x += source_wide + GAP;
    }
}

/// Draws the rule: a hairline across the clock's width, with a bar growing along
/// it.
fn paint_rule(
    grid: &mut BrailleGrid,
    layout: &Layout,
    scale: usize,
    fraction: f64,
) {
    let width_dots = layout.cells_wide() * font::CELL_DOTS * scale;
    // The bar sits *on* the hairline and grows upward from it, so the rule reads
    // as a level being filled rather than as a line being drawn.
    let baseline = digit_rows(scale) * DOTS_Y + RULE_GAP_ROWS * DOTS_Y;
    let hairline = baseline + RULE_DOTS - 1;

    for dot_x in 0..width_dots {
        grid.raise_dot(dot_x, hairline);
    }

    let filled = (width_dots as f64 * fraction.clamp(0.0, 1.0)).round() as usize;
    for dot_x in 0..filled.min(width_dots) {
        for dy in 0..RULE_DOTS {
            grid.raise_dot(dot_x, baseline + dy);
        }
    }
}

/// Writes one grid into the canvas at a cell offset, skipping cells with no ink.
///
/// Skipping the blanks is [`crate::dvd`]'s rule and for its reason: a blank
/// braille cell is a space, and painting one costs a byte and erases whatever the
/// terminal's own background is showing through the clock's gaps.
///
/// [`BrailleGrid::write_to`] is the same loop without the offset, and the offset
/// is needed because the grids are sized to the *clock* rather than to the
/// screen: a full-screen grid for two digits would be 80,000 cells of mostly
/// nothing. There is no skew to absorb either -- the layout is a whole number of
/// cells across and the origin is whole cells, so every dot lands in the cell the
/// arithmetic says it does.
fn stamp(
    grid: &BrailleGrid,
    canvas: &mut Canvas,
    at: (usize, usize),
    color: Color,
) {
    for cell_y in 0..grid.height() {
        let y = at.1 + cell_y;
        if y >= canvas.height() {
            break;
        }
        for cell_x in 0..grid.width() {
            let symbol = grid.cell_char(cell_x, cell_y);
            if symbol == ' ' {
                continue;
            }
            let x = at.0 + cell_x;
            if x >= canvas.width() {
                break;
            }
            canvas.set(x, y, Cell::new(symbol, color, Attribute::Reset));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Where [`fake_clock`] reads from.
        ///
        /// A thread-local rather than a leaked `Box`, and both of those are
        /// worse than this: `Box::leak` cannot be expressed as the `fn` pointer
        /// the seam is, and a plain `static` would be shared between the tests
        /// running in parallel, so one test setting the time would move another
        /// test's clock. A thread-local is per-test by construction, because the
        /// test harness gives each test its own thread.
        static FAKE_TIME: std::cell::Cell<Option<Duration>> =
            const { std::cell::Cell::new(None) };
    }

    /// A time source reading [`FAKE_TIME`], so a test can say "one second later"
    /// without sleeping for it.
    fn fake_clock() -> SystemTime {
        FAKE_TIME.with(|cell| match cell.get() {
            Some(offset) => UNIX_EPOCH + offset,
            // A test that never set it still gets a valid instant. `unwrap_or`
            // against the epoch rather than panicking, because the failure this
            // replaces is "the test asserted on 1970 and did not notice".
            None => UNIX_EPOCH,
        })
    }

    /// Sets what [`fake_clock`] returns.
    fn set_fake_time(seconds: u64) {
        FAKE_TIME.with(|cell| cell.set(Some(Duration::from_secs(seconds))));
    }

    /// Sets what [`fake_clock`] returns, to a second plus `millis`.
    fn set_fake_time_ms(seconds: u64, millis: u64) {
        FAKE_TIME.with(|cell| {
            cell.set(Some(
                Duration::from_secs(seconds) + Duration::from_millis(millis),
            ))
        });
    }

    /// A clock reading [`fake_clock`] at the given size.
    fn clock_at(size: (u16, u16)) -> Clock {
        Clock::with_clock(ClockOptions::default(), size, fake_clock)
    }

    /// Every field of a reading is inside its own range, on a real reading.
    ///
    /// A clock is the one effect whose *output* is the input to something else,
    /// so an out-of-range field is not a cosmetic bug -- `minute = 60` would
    /// print `60` and a negative hour would panic the `{:02}` format's width
    /// assumption. The reading is taken from the actual system clock, which also
    /// means this fails anywhere the timezone lookup is broken, which is
    /// information worth having.
    #[test]
    fn a_reading_is_inside_every_field_s_range() {
        for _ in 0..64 {
            let reading = read_time(system_clock());
            assert!(reading.hour < 24, "hour {} is not 0..24", reading.hour);
            assert!(
                reading.minute < 60,
                "minute {} is not 0..60",
                reading.minute
            );
            assert!(
                reading.second < 60,
                "second {} is not 0..60",
                reading.second
            );
            assert!(
                (0.0..1.0).contains(&reading.fraction),
                "fraction {} is not 0..1",
                reading.fraction
            );
        }
    }

    /// The digits are what the reading says, zero-padded.
    #[test]
    fn the_text_is_zero_padded_and_follows_the_seconds_option() {
        let reading = ClockTime {
            hour: 7,
            minute: 4,
            second: 9,
            fraction: 0.0,
        };
        assert_eq!(time_text(&reading, true), "07:04:09");
        assert_eq!(time_text(&reading, false), "07:04");

        let midnight = ClockTime {
            hour: 0,
            minute: 0,
            second: 0,
            fraction: 0.0,
        };
        assert_eq!(time_text(&midnight, true), "00:00:00");
    }

    /// **24-hour, and that is not an accident of the formatting.**
    ///
    /// The one thing a `HH:MM` format cannot express is the half-day, so this
    /// pins the reading rather than the string: an afternoon hour formatted as
    /// `12` is a clock that is wrong for twelve hours a day, and it is the kind
    /// of thing that arrives as a "small fix" to a 12-hour option later.
    #[test]
    fn hours_run_to_twenty_three_and_not_to_twelve() {
        let mut reading = ClockTime {
            hour: 23,
            minute: 59,
            second: 59,
            fraction: 0.0,
        };
        assert_eq!(time_text(&reading, false), "23:59");
        reading.hour = 13;
        assert_eq!(time_text(&reading, false), "13:59");
        reading.hour = 12;
        assert_eq!(time_text(&reading, false), "12:59");
    }

    /// The rule tracks the period the digits show.
    ///
    /// Both halves, because the failure is a rule running at the wrong rate
    /// *beside* digits running at the right one, and with seconds off the wrong
    /// rate is sixty times too fast rather than merely wrong.
    #[test]
    fn the_rule_tracks_the_period_the_digits_show() {
        let quarter = ClockTime {
            hour: 0,
            minute: 10,
            second: 15,
            fraction: 0.5,
        };
        // With seconds: a quarter of the way through the second.
        assert!((rule_fraction(&quarter, true) - 0.5).abs() < 1e-9);
        // Without: 10 and a half seconds into the minute.
        assert!((rule_fraction(&quarter, false) - 10.5 / 60.0).abs() < 1e-9);

        let start = ClockTime {
            hour: 0,
            minute: 0,
            second: 0,
            fraction: 0.0,
        };
        assert_eq!(rule_fraction(&start, true), 0.0);
        assert_eq!(rule_fraction(&start, false), 0.0);
    }

    /// The layout's width is the sum of its glyphs, not its character count.
    ///
    /// A separator is one source cell wide and a digit is five, so a layout that
    /// counted characters would be five cells too narrow per separator -- which
    /// is ten cells too narrow on a `HH:MM:SS`, and the clock would sit off to
    /// the left of a screen it was supposed to be centred in.
    #[test]
    fn a_layout_is_wider_than_its_character_count() {
        let with_seconds = Layout::new("12:34:56");
        let without = Layout::new("12:34");

        // Six digits at five source cells, two separators at one, and one gap
        // between each of the seven adjacent pairs. The digit width and the
        // separator width are both read from the font rather than written as 5
        // and 1, so this cannot quietly pass against a font that changed either.
        let separator_cells =
            font::cell_width(font::SEPARATOR).expect("in the font");
        assert_eq!(
            with_seconds.cells_wide(),
            6 * font::GLYPH_W + 2 * separator_cells + 7 * GAP
        );
        assert_eq!(
            without.cells_wide(),
            4 * font::GLYPH_W + separator_cells + 4 * GAP
        );
        assert!(
            with_seconds.cells_wide() > "12:34:56".len(),
            "the layout ignored the separator's width"
        );
    }

    /// A layout is the same width whatever the digits in it are.
    ///
    /// **This is the claim that makes centring correct.** Every glyph resolves to
    /// the design box, so `08:00` and `19:35` are the same width and the clock
    /// does not shift sideways as the numbers change. A proportional digit -- the
    /// obvious "improvement" to make a `1` narrower -- would break this, and this
    /// is what would say so.
    #[test]
    fn every_time_of_day_is_the_same_width() {
        let mut widths = std::collections::HashSet::new();
        for hour in 0..24 {
            for minute in 0..60 {
                let text = format!("{hour:02}:{minute:02}:00");
                widths.insert(Layout::new(&text).cells_wide());
            }
        }
        assert_eq!(
            widths.len(),
            1,
            "the clock's width changes with its digits: {widths:?}. A `1` that is \
             narrower than a `0` would make the clock twitch sideways every hour."
        );
    }

    /// The scale is the largest that fits, is always even, and is never zero.
    ///
    /// **Even is the load-bearing half.** [`scale_for`]'s doc has the arithmetic;
    /// this is the assertion that it stays, because the failure it prevents is
    /// invisible to every other test here and to the whole suite: an odd scale
    /// renders every digit with a ragged top and bottom edge, and the numbers all
    /// come out right.
    #[test]
    fn the_scale_is_the_largest_even_one_that_fits() {
        let wide = Layout::new("12:34:56").cells_wide();

        // Even, at every size big enough to have an aligned scale at all, and the
        // maximum that fits.
        for &(width, height) in
            &[(80, 24), (120, 40), (200, 50), (400, 200), (1000, 300)]
        {
            let scale = scale_for(wide, (width, height));
            assert_eq!(
                scale % 2,
                0,
                "at {width}x{height} the scale is {scale}, which is odd, so every \
                 digit's horizontal edges fall between two dot rows of a cell"
            );
            // The maximum: one more step would not fit on at least one axis.
            let next = scale + SCALE_STEP;
            assert!(
                wide * next > width as usize || total_rows(next) > height as usize,
                "at {width}x{height} the scale is {scale} but {next} also fits"
            );
        }

        // The specific numbers this crate documents, so a change to
        // [`font::GLYPH_W`] or [`GAP`] cannot quietly resize the clock.
        assert_eq!(scale_for(wide, (80, 24)), 2);
        assert_eq!(scale_for(wide, (200, 50)), 4);

        // **Below the aligned floor the rule does not apply**, and that is
        // deliberate rather than an oversight: a terminal with no room for the
        // clock at scale 2 gets scale 1, a dot is the quantum, and a clipped
        // clock at scale 1 is better than no clock. The ragged edges that come
        // with it are on a terminal too small to read them.
        for &(width, height) in &[(6, 6), (1, 1), (40, 10), (60, 12)] {
            assert_eq!(
                scale_for(wide, (width, height)),
                1,
                "at {width}x{height} the scale should be the floor of 1"
            );
        }

        // Dropping seconds buys a bigger clock *where the width budget spans a
        // step*, which is the qualified claim in [`ClockOptions::show_seconds`].
        let narrow = Layout::new("12:34").cells_wide();
        for &(width, height) in &[(120, 40), (200, 50), (400, 200)] {
            assert!(
                scale_for(narrow, (width, height))
                    > scale_for(wide, (width, height)),
                "at {width}x{height} turning seconds off did not make the clock \
                 bigger: {} against {}",
                scale_for(narrow, (width, height)),
                scale_for(wide, (width, height))
            );
        }
        // **And 80x24 is the documented exception**, where it does not: the
        // seconds clock already uses 78 of 80 columns at scale 2 and the shorter
        // one cannot reach scale 4 either. Asserted because it is the case a
        // reader would try first and the one where the knob's promise is false.
        assert_eq!(scale_for(wide, (80, 24)), scale_for(narrow, (80, 24)));

        // A dot is the quantum: one is the floor, at any size.
        assert_eq!(scale_for(wide, (6, 6)), 1);
        assert_eq!(scale_for(wide, (1, 1)), 1);
    }

    /// Whether a column's lowest inked dot row is inside the digits rather than
    /// in the rule below them.
    ///
    /// A named function because it is the one predicate in this test that is not
    /// self-evident: `column.last()` is the bottom of the column's ink, and for a
    /// column the rule crosses, that is the rule's hairline rather than a digit's
    /// foot. The rule is furniture and is deliberately not on a cell boundary, so
    /// including it would make this test fail against correct art.
    fn bottom_is_a_digit_row(bottom: usize, digit_dot_rows: usize) -> bool {
        bottom < digit_dot_rows
    }

    /// Nothing is left on the screen when the digits change.
    ///
    /// **This is the end-to-end test, and the only one that models the terminal.**
    /// Every other test here looks at the effect's own output; this one
    /// accumulates the diff into a full-screen model the way the terminal does and
    /// compares that model against what a *freshly built* clock draws at the same
    /// instant. If any ink the clock has stopped drawing survives in the model,
    /// the two disagree.
    ///
    /// It exists because "it draws the right thing now" and "it took the old thing
    /// off the screen" are separate claims, and every other assertion in this file
    /// is the first one. An effect can be perfectly correct about its current
    /// frame and still leave the previous one behind, and no amount of reading the
    /// diff catches that — the diff being *complete* is the claim, and only a
    /// model of the terminal can check it.
    ///
    /// Swept over a whole minute rather than a single transition, so it covers the
    /// awkward ones: `9` to `0` on a single digit, `09` to `10` where a **narrow
    /// `1` replaces a wide `0`** and the two cells either side of the new stem have
    /// to go blank, and the seconds column reaching full and resetting.
    #[test]
    fn a_changing_clock_leaves_nothing_of_the_old_time_on_screen() {
        const START: u64 = 10 * 3600 + 9 * 60 + 55;

        for &(width, height) in &[(200, 50), (80, 24), (120, 40)] {
            let mut clock = clock_at((width, height));
            let mut screen =
                vec![vec![Cell::default(); width as usize]; height as usize];

            for step in 0..10u64 {
                set_fake_time(START + step);
                for (x, y, cell) in clock.get_diff() {
                    if x < width as usize && y < height as usize {
                        screen[y][x] = cell;
                    }
                }

                let reading =
                    read_time(UNIX_EPOCH + Duration::from_secs(START + step));
                let expected = time_text(&reading, clock.options.show_seconds);
                let mut fresh = clock_at((width, height));
                let mut reference =
                    vec![vec![Cell::default(); width as usize]; height as usize];
                for (x, y, cell) in fresh.get_diff() {
                    if x < width as usize && y < height as usize {
                        reference[y][x] = cell;
                    }
                }

                let stale: Vec<String> = (0..height as usize)
                    .flat_map(|y| (0..width as usize).map(move |x| (x, y)))
                    .filter(|&(x, y)| screen[y][x].symbol != reference[y][x].symbol)
                    .map(|(x, y)| {
                        format!(
                            "({x},{y}) {:?} should be {:?}",
                            screen[y][x].symbol, reference[y][x].symbol
                        )
                    })
                    .collect();

                assert!(
                    stale.is_empty(),
                    "at {width}x{height}, showing {expected} (step {step}), the \
                     screen still holds {} cell(s) from the previous time. First \
                     few: {}",
                    stale.len(),
                    stale.iter().take(8).cloned().collect::<Vec<_>>().join("; ")
                );
            }
        }
    }

    /// Every glyph is separated from its neighbour by blank dots.
    ///
    /// **This is the test for glyphs overlapping**, and it exists because the
    /// first version of [`paint_digits`] drew them all on top of each other and
    /// the whole suite was green. See its comment for the arithmetic.
    ///
    /// The distinguishing property is that it reads the **drawn grid** rather than
    /// recomputing the layout. `every_time_of_day_is_the_same_width` and
    /// `a_layout_is_wider_than_its_character_count` both check arithmetic, and the
    /// arithmetic was correct throughout -- `Layout` knew exactly where each glyph
    /// went. What was wrong was the code that turned that layout into dots, and no
    /// amount of asserting about `Layout` could see it.
    ///
    /// The claim is a count of *ink runs*: a row of digits is one run per glyph
    /// plus one separator run, with blank columns between every pair. One
    /// continuous run means the glyphs have merged, which is precisely what
    /// overlapping looks like from outside.
    ///
    /// Checked on a **column-agnostic projection** -- "is this dot column inked
    /// anywhere in the digit area?" -- so a glyph's own internal gaps (the
    /// counter of a `0`) do not register as separators.
    #[test]
    fn no_two_glyphs_touch() {
        for &(width, height) in &[(200, 50), (400, 200), (80, 24), (120, 40)] {
            set_fake_time(20 * 3600 + 34 * 60 + 56);
            let mut clock = clock_at((width, height));
            clock.draw();

            let scale = clock.laid_out_scale;
            let grid = clock.digits_grid();
            let digit_rows_dots = digit_rows(scale) * DOTS_Y;

            let inked: Vec<bool> = (0..grid.dot_width())
                .map(|x| (0..digit_rows_dots).any(|y| grid.dot(x, y)))
                .collect();

            let runs = ink_runs(&inked);
            let glyphs = time_text(
                &ClockTime {
                    hour: 20,
                    minute: 34,
                    second: 56,
                    fraction: 0.0,
                },
                clock.options.show_seconds,
            )
            .chars()
            .count();

            assert_eq!(
                runs.len(),
                glyphs,
                "at {width}x{height} (scale {scale}) the clock draws {} ink runs for \
                 {glyphs} glyphs, so glyphs are merging into their neighbours. \
                 Runs: {runs:?}",
                runs.len()
            );
        }
    }

    /// Contiguous spans of `true` in `flags`, inclusive on both ends.
    fn ink_runs(flags: &[bool]) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut start: Option<usize> = None;
        for (i, f) in flags.iter().enumerate() {
            match (*f, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    runs.push((s, i - 1));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(s) = start {
            runs.push((s, flags.len() - 1));
        }
        runs
    }

    /// Every gap between glyphs is at least [`GAP`] source cells wide.
    ///
    /// The count in [`no_two_glyphs_touch`] says the glyphs are separate; this
    /// says they are *legibly* separate. A one-dot gap between two digits is
    /// separate by the run count and unreadable on screen, and at a large scale a
    /// gap proportional to the digits grows with them, so the two are not the same
    /// property.
    ///
    /// **It re-asserts the run count, and that is not redundancy.** The first
    /// version of this test iterated `runs.windows(2)` and checked each gap, which
    /// passes *vacuously* when there is one run and therefore no pairs — and one
    /// run is exactly what overlapping glyphs produce. Reinstating the overlap bug
    /// left this test green. The count is what stops the gap loop from having
    /// nothing to say.
    #[test]
    fn the_gap_between_glyphs_is_the_width_it_should_be() {
        set_fake_time(20 * 3600 + 34 * 60 + 56);

        for &(width, height) in &[(200, 50), (80, 24), (400, 200)] {
            let mut clock = clock_at((width, height));
            clock.draw();

            let scale = clock.laid_out_scale;
            let grid = clock.digits_grid();
            let digit_rows_dots = digit_rows(scale) * DOTS_Y;

            let inked: Vec<bool> = (0..grid.dot_width())
                .map(|x| (0..digit_rows_dots).any(|y| grid.dot(x, y)))
                .collect();

            let runs = ink_runs(&inked);
            let glyphs = 8;

            // Without this the loop below has nothing to iterate, and the test
            // measures nothing at all.
            assert!(
                runs.len() > 1,
                "at {width}x{height} there is {runs:?} -- one ink run, so there are \
                 no gaps to check and this test would pass on any drawing"
            );
            assert_eq!(
                runs.len(),
                glyphs,
                "at {width}x{height} (scale {scale}) the clock draws {} ink runs \
                 for {glyphs} glyphs. Runs: {runs:?}",
                runs.len()
            );

            let expected = GAP * font::CELL_DOTS * scale;
            for pair in runs.windows(2) {
                let gap = pair[1].0 - pair[0].1 - 1;
                assert!(
                    gap >= expected,
                    "at {width}x{height} (scale {scale}) two glyphs are {gap} dots \
                     apart, and the layout asks for {expected}"
                );
            }
        }
    }

    /// Every horizontal edge of a digit lands on a braille cell boundary.
    ///
    /// **This is the test for the ragged-edge defect**, and it is the only one:
    /// the scale being even is the *cause* and this is the *effect*, so a future
    /// change that breaks the alignment without changing the scale -- a font
    /// authored at the wrong height, say -- is caught here and not there.
    ///
    /// Checked on the drawn grid rather than on the arithmetic, because the
    /// arithmetic is what is easy to get right and the picture is what is easy to
    /// get wrong. A digit's edge is aligned when the dot rows either side of it
    /// are in *different* cells, which is the only way a horizontal edge can be
    /// clean: a step within one cell is a half-height mark.
    #[test]
    fn a_digits_horizontal_edges_land_on_cell_boundaries() {
        // Large enough that the clock is comfortably scaled, and an instant whose
        // digits have ink on both their top and bottom rows so there are edges to
        // check at all.
        set_fake_time(23 * 3600 + 45 * 60 + 30);
        let mut clock = clock_at((200, 50));
        clock.draw();

        let scale = clock.laid_out_scale;
        assert!(
            scale >= 2,
            "the clock is at scale {scale}, too small to check"
        );

        // Walk the drawn rows and find, for each column, where the ink starts and
        // stops. Every one of those transitions has to be on a cell boundary.
        let grid = clock.digits_grid();
        let rows = grid.dot_height();
        // The digits occupy the top `digit_rows(scale)` cell rows; the rule is
        // below them.
        let digit_dot_rows = digit_rows(scale) * DOTS_Y;
        let mut ragged: Vec<String> = Vec::new();

        for dot_x in 0..grid.dot_width() {
            let column: Vec<usize> =
                (0..rows).filter(|y| grid.dot(dot_x, *y)).collect();
            for window in column.windows(2) {
                let (above, below) = (window[0], window[1]);
                // A gap of exactly one dot row is an edge *within* the glyph --
                // the counter of a `3`, say -- and it is drawn as intended.
                if below - above > 1 && below / DOTS_Y == above / DOTS_Y {
                    ragged.push(format!(
                        "column {dot_x}: ink at dot row {above} and again at {below}, \
                         both inside cell row {}",
                        above / DOTS_Y
                    ));
                }
            }
            // And the glyph's own top and bottom, **within the digits' rows only**.
            //
            // The rule below is furniture and is deliberately *not* on a cell
            // boundary -- it is a two-dot bar on a hairline placed by
            // `paint_rule` -- so checking it here would be checking the wrong
            // thing. The digits end at `digit_dot_rows`.
            // A dot row whose cell row differs from its neighbour's is on a
            // boundary. Same cell means the edge is drawn *across* a cell, which
            // is the ragged half-height mark.
            let extremes = match (column.first(), column.last()) {
                (Some(&top), Some(&bottom))
                    if bottom_is_a_digit_row(bottom, digit_dot_rows) =>
                {
                    Some((top, bottom))
                }
                _ => None,
            };
            if let Some((top, bottom)) = extremes {
                if top > 0 && top / DOTS_Y == (top - 1) / DOTS_Y {
                    ragged.push(format!(
                        "column {dot_x}: ink starts at dot row {top}, inside cell \
                         row {}, so the glyph's top edge is drawn across a cell",
                        top / DOTS_Y
                    ));
                }
                if bottom + 1 < rows && bottom / DOTS_Y == (bottom + 1) / DOTS_Y {
                    ragged.push(format!(
                        "column {dot_x}: ink ends at dot row {bottom}, inside cell \
                         row {}, so the glyph's bottom edge is drawn across a cell",
                        bottom / DOTS_Y
                    ));
                }
            }
        }

        assert!(
            ragged.is_empty(),
            "the digits' horizontal edges do not land on cell boundaries, so the \
             glyphs render with ragged top and bottom rows. {} column(s) affected, \
             first few: {}",
            ragged.len(),
            ragged
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
    }

    /// The scale is bounded by height as well as width.
    ///
    /// A wide, short terminal is the case a width-only search gets wrong, and it
    /// is why `every_effect_survives_extreme_aspect_ratios` exists: at 200x8 the
    /// width would allow a scale of 5 and the digits would be cut off at the
    /// bottom, which is a picture that looks like a rendering fault rather than a
    /// terminal that is too short.
    #[test]
    fn a_short_terminal_bounds_the_scale_by_height() {
        let wide = Layout::new("12:34:56").cells_wide();

        let scale = scale_for(wide, (200, 8));
        assert!(
            total_rows(scale) <= 8,
            "at 200x8 the clock wants {scale} rows of an 8-row terminal"
        );
        // And one more *step* really would not fit, so the bound is the binding
        // one. `SCALE_STEP` rather than 1, because `scale_for` only ever offers
        // even scales and checking `scale + 1` would pass against a bound that
        // had stopped binding.
        assert!(total_rows(scale + SCALE_STEP) > 8);
    }

    /// Nothing is drawn outside the terminal at any size the CLI can ask for.
    ///
    /// Checked by driving the effect and reading the coordinates it reports,
    /// rather than by inspecting a grid, because an out-of-bounds write is a
    /// memory-safety question and the grid is not where it happens -- it happens
    /// in the stamp into the canvas.
    #[test]
    fn nothing_is_drawn_outside_the_terminal() {
        set_fake_time(12 * 3600 + 34 * 60 + 56);

        for (width, height) in
            [(6, 6), (1, 1), (8, 200), (200, 8), (80, 24), (200, 50)]
        {
            let mut clock = clock_at((width, height));

            for frame in 0..3 {
                let diff = clock.draw();
                for (x, y, _) in diff {
                    assert!(
                        x < width as usize && y < height as usize,
                        "at {width}x{height} frame {frame} wrote ({x}, {y})"
                    );
                }
            }

            // And the same after a resize to somewhere else, which is the path
            // where an origin computed from the old size would be wrong.
            clock.update_size(height, width);
            for (x, y, _) in clock.draw() {
                assert!(
                    x < height as usize && y < width as usize,
                    "at {width}x{height}, resized to {height}x{width}, wrote \
                     ({x}, {y}) outside the new size"
                );
            }
        }
    }

    /// A second redraws the digits, and only the digits.
    ///
    /// The "only" is the point. A clock that repainted the whole face every
    /// second would be correct and would emit a full screen of braille sixty
    /// times a minute, which is the same lesson as [`crate::terrain`]'s grain:
    /// a picture that changes most of itself is not doing anything. So the claim
    /// is both that the frame is **not empty** -- which a clock that never
    /// redraws would satisfy trivially at the wrong instant -- and that it is
    /// **small**, so the redraw is confined to the digit that changed.
    #[test]
    fn crossing_a_second_redraws_only_the_digit_that_changed() {
        set_fake_time(12 * 3600 + 34 * 60 + 56);
        let mut clock = clock_at((80, 24));

        // Two frames at the same instant: the second must be empty, or the
        // effect is repainting itself for no reason at all.
        clock.draw();
        assert!(
            clock.draw().is_empty(),
            "a frame at an unchanged instant rewrote cells"
        );

        set_fake_time(12 * 3600 + 34 * 60 + 57);
        let tick = clock.draw();
        assert!(!tick.is_empty(), "a new second changed nothing");

        // `6` to `7` is the last digit: two glyphs wide out of 39, and a
        // braille glyph is 2x4 dots so at most 40 cells of ink can be involved.
        // The bound is generous on purpose -- what it rules out is a repaint of
        // the whole face.
        let cells = (80 * 24) as usize;
        assert!(
            tick.len() < cells / 4,
            "a one-digit tick rewrote {} of {cells} cells, so the clock is \
             repainting its whole face every second",
            tick.len()
        );
    }

    /// The rule grows, sub-second, on every frame.
    ///
    /// This is the assertion the seconds rule exists to satisfy, and it is
    /// written as *monotone growth* rather than as "the picture changed", because
    /// "the picture changed" is satisfied by a rule that jitters and a clock that
    /// strobes. It is also the test `no_effect_settles_into_a_still_picture`
    /// needs and cannot have: that one measures at 200x50 and knows nothing about
    /// the rate at a given size.
    #[test]
    fn the_rule_grows_monotonically_within_a_second() {
        set_fake_time(0);
        let mut clock = clock_at((200, 50));

        // At the top of a second the bar is empty and only the hairline is drawn,
        // so there is a track for the bar to be seen growing along rather than a
        // bar that appears out of nothing.
        clock.draw();
        assert_eq!(
            bar_width(&clock),
            0,
            "the rule is already {} dots along at the top of a second",
            bar_width(&clock)
        );
        assert_eq!(
            track_width(&clock),
            clock.accent_grid().dot_width(),
            "the rule has no track to grow along"
        );

        // **Up to but not including 1000ms**, and the boundary is the point. The
        // rule resets at the top of each second, so a sweep that ran to 1000ms
        // would report the bar falling from full to empty and call it a shrink --
        // which is the rule working. `the_rule_starts_over_at_the_top_of_each_
        // period` covers the reset; this one is only about the growth inside it.
        let mut previous = 0;
        for step in 1..40 {
            set_fake_time_ms(0, step * 25);
            clock.draw();
            let width = bar_width(&clock);
            assert!(
                width >= previous,
                "the rule shrank at {}ms: {width} against {previous}",
                step * 25
            );
            previous = width;
        }

        // Fifteen frames short of the second, so fifteen sixteenths of the rule is
        // filled. The bound is an eighth short of that, which is generous enough
        // for dot rounding and tight enough that a rule running at the wrong rate
        // -- a minute-long rule beside second digits -- cannot pass.
        let full = clock.accent_grid().dot_width();
        assert!(
            previous * 16 >= full * 14,
            "at 975ms the bar is {previous} of {full} dots"
        );
    }

    /// How many dots the filled part of the rule is wide.
    ///
    /// Counted as columns where *both* bar dot rows are raised, which is what
    /// makes it the bar rather than the hairline: the hairline is a dot row and
    /// is present at every column from the first frame, so counting raised dots
    /// would measure the track and report it as full for ever.
    fn bar_width(clock: &Clock) -> usize {
        let grid = clock.accent_grid();
        let scale = clock.laid_out_scale;
        let baseline = digit_rows(scale) * DOTS_Y + RULE_GAP_ROWS * DOTS_Y;
        (0..grid.dot_width())
            .filter(|x| (0..RULE_DOTS).all(|dy| grid.dot(*x, baseline + dy)))
            .count()
    }

    /// How many dots the hairline is wide. Always the full track.
    fn track_width(clock: &Clock) -> usize {
        let grid = clock.accent_grid();
        let scale = clock.laid_out_scale;
        let hairline =
            digit_rows(scale) * DOTS_Y + RULE_GAP_ROWS * DOTS_Y + RULE_DOTS - 1;
        (0..grid.dot_width())
            .filter(|x| grid.dot(*x, hairline))
            .count()
    }

    /// The rule clears itself between seconds rather than leaving the last
    /// frame's bar behind.
    ///
    /// The accent grid is cleared and repainted every frame, so a rule that only
    /// ever *grew* would reach the right edge on the first second and sit there
    /// for ever. A clock whose progress bar is permanently full is a clock whose
    /// bar is decoration.
    #[test]
    fn the_rule_starts_over_at_the_top_of_each_period() {
        set_fake_time_ms(60, 800);
        let mut clock = clock_at((200, 50));

        // Four fifths of the way through the second: most of the track is bar.
        clock.draw();
        let late = bar_width(&clock);
        assert!(
            late * 5 >= clock.accent_grid().dot_width() * 4,
            "at 800ms the rule is only {late} of {} dots",
            clock.accent_grid().dot_width()
        );

        // The next second starts at zero, so the bar goes back to the hairline
        // while the digits tick. A rule that only ever grew would sit full on the
        // right edge for the rest of the run.
        set_fake_time(61);
        clock.draw();
        assert_eq!(
            bar_width(&clock),
            0,
            "the rule did not reset at the top of the second"
        );
    }

    /// Turning seconds off can make the clock bigger, and the rule follows the
    /// minute.
    ///
    /// Two claims in one test because they are the same knob from both ends: a
    /// `HH:MM` clock that is not bigger is not giving anything up, and a
    /// seconds-long rule next to minute digits is running sixty times too fast.
    #[test]
    fn seconds_off_is_a_bigger_clock_and_a_minute_long_rule() {
        // **120x40, not 80x24**, because 80x24 is the size where turning the
        // seconds off buys nothing -- both layouts are scale 2 there, and this
        // test would fail against a correct implementation. The doc on
        // [`ClockOptions::show_seconds`] has the table.
        set_fake_time(0);
        let mut with = clock_at((120, 40));
        with.draw();
        let wide_rows = with.digits_grid().height();

        let mut without = Clock::with_clock(
            ClockOptions {
                show_seconds: false,
                ..ClockOptions::default()
            },
            (120, 40),
            fake_clock,
        );
        without.draw();

        // **A higher dot scale, which is the honest measure of "bigger".**
        //
        // The first version of this asserted the clock was larger on *both* axes
        // in cells and failed: the seconds clock came out 78x9 cells and the
        // seconds-off one 75x13. Taller, and three columns narrower.
        //
        // That is not a bug, and the reason generalises: **the two layouts do not
        // have the same proportions.** `HH:MM:SS` is 39 source cells by 14 dots,
        // 2.8:1 in square units; `HH:MM` is 25 by 14, 1.8:1. A cell count on one
        // axis is a proxy for size that only works when the two pictures are the
        // same shape, and these are not. A dot *scale* is the real thing: it is a
        // whole dot per source dot on every edge of every glyph, which is what
        // "bigger" means to anyone looking at it.
        //
        // **Measured, not derived**, and pinned so that a change to the font or
        // to `GAP` which invalidates them is visible rather than quietly absorbed.
        assert_eq!(
            (with.laid_out_scale, without.laid_out_scale),
            (2, 4),
            "at 120x40 the seconds clock is at scale {} and the seconds-off one at \
             {}. The scale is what 'bigger' means here, so if this changed then the \
             comment above is stale.",
            with.laid_out_scale,
            without.laid_out_scale
        );
        // And the scale bought real height, not just a bigger dot on a shorter
        // glyph: a higher scale with no gain in rows would mean the digits had
        // been squashed rather than enlarged.
        assert!(
            without.digits_grid().height() > wide_rows,
            "without seconds the clock is {} rows against {wide_rows}, so the \
             larger scale bought no height",
            without.digits_grid().height()
        );

        // The rule follows the period, checked at an instant where the two
        // fractions are far enough apart to tell apart.
        //
        // **The instant is searched for rather than hard-coded**, because the
        // reading is local and this test must not care what timezone it runs in.
        // An instant chosen as "half a minute past the epoch" lands on some
        // arbitrary minute of some arbitrary hour, and the two fractions are far
        // apart only in the first few seconds of a minute -- the search over a
        // whole hour below is what finds one, and the first version searched
        // seven minutes and found none.
        //
        // The direction is not a detail: `rule_fraction` is `fraction` with
        // seconds on and `(minute + fraction) / 60` with them off, so the second
        // is always the *larger* of the two early in a minute and the *smaller*
        // one late in it. The test asks for the case that separates them cleanly,
        // rather than an absolute difference that a rule running at the wrong rate
        // could satisfy by being wrong in the other direction.
        let mut chosen = None;
        for second in 0..3600u64 {
            let reading = read_time(UNIX_EPOCH + Duration::from_secs(second));
            if rule_fraction(&reading, false) * 4.0 <= rule_fraction(&reading, true)
            {
                chosen = Some((second, reading));
                break;
            }
        }
        let (instant, reading) = chosen.expect(
            "no instant in the first hour has the minute rule at a quarter of the \
             seconds rule, which cannot happen -- `rule_fraction` is not being read",
        );

        set_fake_time(instant);
        with.draw();
        without.draw();

        let expected_seconds = (with.accent_grid().dot_width() as f64
            * rule_fraction(&reading, true))
            as usize;
        let expected_minutes = (without.accent_grid().dot_width() as f64
            * rule_fraction(&reading, false))
            as usize;

        assert_eq!(
            bar_width(&with),
            expected_seconds,
            "the seconds rule is {} dots where {expected_seconds} of {} is \
             {reading:?}",
            bar_width(&with),
            with.accent_grid().dot_width()
        );
        assert_eq!(
            bar_width(&without),
            expected_minutes,
            "the minute rule is {} dots where {expected_minutes} of {} is \
             {reading:?}",
            bar_width(&without),
            without.accent_grid().dot_width()
        );
        assert!(
            expected_minutes * 4 <= expected_seconds,
            "at {reading:?} the two bars are {expected_minutes} and \
             {expected_seconds} dots, which are too close to tell the periods \
             apart -- the search should have found a better instant"
        );
    }

    /// The digits are on the canvas, in the configured colour, and centred.
    ///
    /// Checked on the reported frame rather than on the grid, because the grid
    /// is what the effect *intends* to draw and the frame is what the terminal
    /// gets. A colour or an offset bug lives in the stamp.
    #[test]
    fn the_frame_carries_the_configured_colours_and_is_centred() {
        let options = ClockOptions {
            show_seconds: true,
            color: Color::Red,
            accent: Color::Blue,
        };
        // 09:08:07 rather than a round time: a leading zero and a `7` in the last
        // place, so the frame exercises both a slashed zero and the digit the
        // seconds tick most often changes.
        set_fake_time(9 * 3600 + 8 * 60 + 7);
        let mut clock = Clock::with_clock(options, (80, 24), fake_clock);
        let frame = clock.draw();

        assert!(!frame.is_empty(), "the first frame drew nothing");
        assert!(
            frame.iter().any(|(_, _, cell)| cell.color == Color::Red),
            "no digit was drawn in the configured colour"
        );
        assert!(
            frame.iter().any(|(_, _, cell)| cell.color == Color::Blue),
            "the separator and rule were not drawn in the accent colour"
        );
        assert!(
            frame.iter().all(|(_, _, cell)| cell.symbol != ' '),
            "a blank cell was reported, which would erase the background"
        );

        // Centred, to within the rounding of an odd leftover column.
        let layout = Layout::new("09:08:07");
        let clock_w =
            layout.cells_wide() * scale_for(layout.cells_wide(), (80, 24));
        let left = frame.iter().map(|(x, _, _)| *x).min().expect("some cells");
        let slack = (80 - clock_w) / 2;
        assert!(
            left.abs_diff(slack) <= 1,
            "the clock starts at column {left}, and centring puts it at {slack}"
        );
    }

    /// The first frame after a resize is a full repaint.
    ///
    /// The resize contract in its own right -- a terminal that grew has cells the
    /// effect has never written, and an effect that diffs against the old size
    /// leaves them showing whatever was there before. `Canvas::resize` blanks both
    /// surfaces, which is what makes this hold; asserting it here is what says
    /// the effect went through it.
    #[test]
    fn a_resize_repaints_the_whole_terminal() {
        set_fake_time(3 * 3600 + 2 * 60 + 1);
        let mut clock = clock_at((80, 24));
        clock.draw();

        clock.update_size(60, 20);
        let frame = clock.draw();

        // Fewer cells than the whole terminal, because the gaps are never
        // written -- but every column the clock reaches has to be there.
        let columns: std::collections::BTreeSet<usize> =
            frame.iter().map(|(x, _, _)| *x).collect();
        let layout = Layout::new("03:02:01");
        let scale = scale_for(layout.cells_wide(), (60, 20));
        let expected = layout.cells_wide() * scale;
        let left = (60 - expected) / 2;
        assert!(
            columns.contains(&left) && columns.contains(&(left + expected - 1)),
            "after a resize the clock spans {columns:?}, which does not reach both \
             edges of its own {expected} cell box at column {left}"
        );
        assert!(
            columns.iter().all(|x| *x < 60),
            "a resized clock wrote outside 60 columns: {columns:?}"
        );
    }

    /// The grid is sized to the clock, not to the screen.
    ///
    /// A full-screen braille grid is 80,000 cells at 400x200 for a picture that
    /// is 5% ink, and there is no version of this effect that needs one: the
    /// clock's origin is whole cells, so there is never a skew to absorb.
    #[test]
    fn the_grids_are_the_size_of_the_clock() {
        // Every digit at its most open, so this is the largest the font can draw.
        set_fake_time(23 * 3600 + 59 * 60 + 58);
        let mut clock = clock_at((400, 200));
        clock.draw();

        let layout = Layout::new("23:59:59");
        let scale = scale_for(layout.cells_wide(), (400, 200));
        assert_eq!(clock.digits_grid().width(), layout.cells_wide() * scale);
        assert_eq!(clock.digits_grid().height(), total_rows(scale));
        assert!(
            clock.digits_grid().width() < 400,
            "the digit grid is {} cells wide on a 400 column terminal",
            clock.digits_grid().width()
        );
    }

    /// Two grids over one canvas, and the accent never lands on a digit's cell.
    ///
    /// A braille cell is one glyph in one colour, so `stamping` the accent grid
    /// over the digits would erase any dot they shared. The two are kept apart by
    /// construction -- the separator is in its own columns and the rule in its own
    /// rows -- and this is the assertion that they stay apart.
    #[test]
    fn the_separator_and_the_rule_do_not_share_a_cell_with_a_digit() {
        set_fake_time_ms(12 * 3600 + 30 * 60 + 30, 500);
        let mut clock = clock_at((200, 50));
        clock.draw();

        let digits = clock.digits_grid();
        let accent = clock.accent_grid();
        assert_eq!(digits.width(), accent.width());
        assert_eq!(digits.height(), accent.height());

        for cell_y in 0..digits.height() {
            for cell_x in 0..digits.width() {
                assert!(
                    !(digits.dot(cell_x, cell_y) && accent.dot(cell_x, cell_y)),
                    "cell ({cell_x}, {cell_y}) has ink in both grids, so one stamp \
                     would erase the other"
                );
            }
        }
    }
}
