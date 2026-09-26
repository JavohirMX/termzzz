use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crossterm::style;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlasmaOptions {
    pub time_scale: f64,
    pub spatial_scale: f64,
    pub color_speed: f64,
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

pub struct Plasma {
    pub screen_size: (u16, u16),
    options: PlasmaOptions,
    canvas: Canvas,
    time: f64,
    palette: Vec<style::Color>,
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

        Self {
            screen_size,
            options,
            canvas,
            time,
            palette,
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

    /// Calculate plasma value using the [AWK script formula](https://rosettacode.org/wiki/Plasma_effect#AWK)
    fn calc_plasma_value(
        x: f64,
        y: f64,
        now: f64,
        w: f64,
        h: f64,
        scale: f64,
    ) -> u8 {
        let value = (128.0
            + (128.0 * ((x / 8.0) * scale - (now / 2.0).cos()).sin())
            + 128.0
            + (128.0 * ((y / 16.0) * scale - now.sin() * 2.0).sin())
            + 128.0
            + (128.0
                * (((x - w / 2.0).powi(2) + (y - h / 2.0).powi(2)).sqrt() / 4.0
                    * scale)
                    .sin())
            + 128.0
            + (128.0
                * (((x.powi(2) + y.powi(2)).sqrt() / 4.0) * scale
                    - (now / 4.0).sin())
                .sin()))
            / 4.0;

        value as u8
    }

    /// Update the plasma field in the buffer
    #[allow(clippy::too_many_arguments)]
    fn update_plasma(
        size: (u16, u16),
        now: f64,
        color_speed: f64,
        spatial_scale: f64,
        palette: &[style::Color],
        buffer: &mut Buffer,
    ) {
        let width = size.0 as usize;
        let height = size.1 as usize;
        let w = width as f64;
        let h = height as f64;

        for y in 0..height {
            for x in 0..width {
                // For each cell, calculate two plasma values (upper and lower half)
                let y_f64 = (y * 2) as f64;
                let x_f64 = x as f64;

                // Calculate plasma values
                let plasma = Self::calc_plasma_value(
                    x_f64,
                    y_f64,
                    now,
                    w,
                    h * 2.0,
                    spatial_scale,
                );

                // Get color indices with time component. Wrapped rather than
                // cast-and-clamped: the offset grows without bound, and an
                // out-of-range index is a panic waiting for a long session.
                let color_idx = (plasma as f64 + now * color_speed)
                    .rem_euclid(PALETTE_LEN as f64)
                    as usize;

                let cell_color = palette[color_idx];

                // `Attribute::Reset`, not `Attribute::Bold`. Every cell used to be
                // bold, and bold on a truecolor foreground is a rendering hint
                // that many terminals answer by brightening the colour -- which
                // is how a saturated cell ends up reading as white. The palette
                // is already saturated, so it does not need the hint.
                let cell = Cell::new('*', cell_color, style::Attribute::Reset);

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
}
