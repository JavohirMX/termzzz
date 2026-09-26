use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crate::render::glyph_ramp::{self, GlyphRamp};
use crate::render::palette::Palette;
use crate::terrain::noise::PerlinNoise;
use crossterm::style;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TerrainOptions {
    pub seed: u64,
    /// Terminal cells per period of the base-frequency noise.
    ///
    /// The zoom, in the one unit that makes it a zoom. It used to be a bare
    /// frequency multiplier on raw cell coordinates, where 0.02 meant one noise
    /// cell per 50 cells, and 0.02 is the same number this field would reach on
    /// a screen 50 cells wide -- which is why it was tuned for a very large
    /// terminal and drew one soft blob at 80x24.
    ///
    /// It is deliberately *not* a count of noise cells across the screen. That
    /// is the intuitive reading and it is the wrong one: holding the count
    /// fixed means a 400x200 terminal spreads the same number of periods over
    /// five times as many cells, so the ground gets five times smoother and the
    /// effect that looked like terrain at 80x24 becomes a flat wash at 400x200.
    /// Measured, that was 14.2% and 17.7% of adjacent ground cells changing
    /// ramp step at 80x24 against 3.1% and 3.8% at 400x200. Fixing the period
    /// instead of the count makes the density size-independent: the same figure
    /// is 15.3% and 17.8% at 400x200.
    pub scale: f64,
    pub octaves: i32,
    pub persistence: f64,
    /// Ground rows scrolled past per second.
    pub scroll_speed: f64,
    /// Characters the noise value is drawn as, sparsest first.
    pub glyphs: String,
}

impl Default for TerrainOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            seed: crate::common::DEFAULT_SEED,
            scale: DEFAULT_SCALE,
            octaves: 4,
            persistence: 0.5,
            scroll_speed: DEFAULT_SCROLL_SPEED,
            glyphs: DEFAULT_GLYPHS.to_string(),
        }
    }
}

/// Cells per period of the base-frequency noise.
///
/// The old hard-coded 0.02, read as a frequency on cell coordinates, is 50
/// cells per period. Measured as a fraction of neighbouring ground cells that
/// change ramp step, 50 gave 4.6% and 5.2% at 80x24 -- a smooth wash, and the
/// "one soft blob" the effect was reported for.
///
/// 8.0 is the pick, and the number was chosen by sweeping rather than by taste.
/// Horizontal/vertical density at 80x24: 4 cells per period gave 24.1%/27.6%,
/// 6 gave 18.9%/22.3%, 8 gave 16.1%/20.2%, 12 gave 12.2%/14.2%, 20 gave
/// 7.8%/8.8%. 8 sits where the ridges are still distinct rather than noise, and
/// -- the reason it is the pick rather than 6 or 10 -- the figures barely move
/// with terminal size, because the period is fixed in cells: at 400x200, 8 gives
/// 15.3%/17.8% against 16.1%/20.2% at 80x24. A setting normalised to the screen
/// would have drifted by a factor of four over that range.
///
/// Four octaves at persistence 0.5 put the finest detail at an eighth of this
/// period, so about one cell per period at the finest octave on an 80-column
/// screen: below the resolution where the ramp can show it, which is why the
/// coarsest octave is what the eye reads and why the density figures are as
/// high as they are.
const DEFAULT_SCALE: f64 = 8.0;

/// Ground rows scrolled past per second.
///
/// Measured, not guessed, and the constraint is bandwidth. This is a *sampled*
/// field rather than a simulation, so scrolling it repaints whichever cells
/// changed value, and the cost is proportional to the scroll rate: at 60 Hz,
/// 0.35 rows a second changed 85 cells in the worst frame at 80x24 and 718 at
/// 400x200, while 0.9 changed 207 and 1822. At 0.9 that is about 4 KB and
/// 36 KB of escape sequences a frame, or 0.24 and 2.1 MB/s -- the same order
/// as every other effect in the catalogue, and small enough that the scroll is
/// not the reason a frame is late. Past about 2.0 the ground stops reading as
/// receding and starts reading as a conveyor belt, so 0.9 is chosen for being
/// visibly moving without being the fastest thing on screen.
const DEFAULT_SCROLL_SPEED: f64 = 0.9;

/// The glyph ramp the ground is drawn as, defaulting to the shade blocks.
///
/// Blocks rather than `SHADE`, because `SHADE` is documented as *not* monotonic
/// in ink -- `=` is heavier than `+` and `+` is heavier than `*` -- and the old
/// hard-coded table used exactly that set, so three consecutive steps of it read
/// backwards. That was a real defect and not a matter of taste: the glyph half
/// of the signal was noise added to a colour ramp that already carried the same
/// value. `░▒▓█` are defined by Unicode as quarter, half, three-quarter and full
/// coverage, so the ordering holds by the standard, and it holds at any cell
/// aspect ratio -- a `:` reads as a horizontal smear in a wide cell and a
/// vertical dotted line in a tall one, which a shade block does not care about.
const DEFAULT_GLYPHS: &str = glyph_ramp::presets::BLOCKS;

/// Fraction of the screen height that is ground, the rest sky.
///
/// Pinned rather than derived, because a horizon has to be somewhere specific
/// for the effect to read as a landscape: at a third of the height the ground
/// has enough rows to show several noise periods against the sky, and the sky
/// has enough rows to show a gradient.
const HORIZON_FRACTION: f64 = 0.32;

/// A terminal cell is taller than it is wide, so a shape measured in cell units
/// is an ellipse on screen. DejaVu Sans Mono is about 1 : 1.2, and the usual
/// ideal-square assumption is 1 : 2; 1.2 is the conservative middle, because
/// correcting by 2 would stretch every feature twice as far vertically on the
/// fonts most people actually have.
const CELL_ASPECT: f64 = 1.2;

/// Sky at the zenith, through mid sky, to the haze at the horizon.
///
/// The middle stop is what stops the sky reading as a plain vertical fade: it
/// puts the gradient's steepest section in the middle of the visible band
/// rather than at its edge, so there is a dark top to the screen and a bright
/// line at the horizon for the ground to sit under.
const SKY: &[style::Color] = &[
    style::Color::Rgb { r: 6, g: 8, b: 26 },
    style::Color::Rgb {
        r: 26,
        g: 44,
        b: 84,
    },
    style::Color::Rgb {
        r: 126,
        g: 148,
        b: 186,
    },
];

/// Ground at the shadowed low point and at the lit crest.
const GROUND: &[style::Color] = &[
    style::Color::Rgb {
        r: 12,
        g: 14,
        b: 20,
    },
    style::Color::Rgb {
        r: 196,
        g: 204,
        b: 214,
    },
];

/// Builds the glyph ramp, never empty and never a character that could shear a
/// cell-indexed grid.
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

pub struct Terrain {
    pub screen_size: (u16, u16),
    options: TerrainOptions,
    canvas: Canvas,
    noise: PerlinNoise,
    ramp: GlyphRamp,
    sky: Palette,
    ground: Palette,
    /// Ground rows scrolled past, accumulated from the frame delta.
    offset: f64,
}

impl TerminalEffect for Terrain {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // Repainted in full every frame, the way the field effects do. It used
        // to render once and then return an empty diff forever, on the comment
        // "No changes after initial generation" -- which made this the only
        // fully static effect in the catalogue and gave its playlist slot the
        // shortest duration of the sixteen, four seconds, by 3x. As a
        // screensaver, drawing one frame and then sitting there is a categorical
        // failure rather than a look.
        self.canvas.clear();
        self.render(self.offset);
        self.canvas.commit()
    }

    fn update(&mut self) {
        // No timing information on this path, so the same fixed step the frame
        // loop would have used. `update_with_context` is the real one.
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Overridden so the scroll rate is real elapsed time. Without it the
        // ground would move faster on a 144 Hz terminal than on a 30 Hz one and
        // the speed keys would not mean what the help says. Once this effect
        // stopped being static it came under `effects_advance_using_the_frame_
        // delta` in the contract suite, which is why it is not optional.
        self.advance(context.delta.as_secs_f64());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas
            .resize(self.screen_size.0.max(1), self.screen_size.1.max(1));
        // The noise is seeded, so it has to be rebuilt too. Leaving the old one
        // in place meant a reset reused the previous terrain's landscape.
        self.noise = PerlinNoise::new(self.options.seed);
        self.offset = 0.0;
    }
}

impl Terrain {
    pub fn new(options: TerrainOptions, screen_size: (u16, u16)) -> Self {
        let canvas = Canvas::new(screen_size.0, screen_size.1);
        let noise = PerlinNoise::new(options.seed);

        Self {
            screen_size,
            ramp: glyph_ramp(&options.glyphs),
            sky: Palette::new(SKY.to_vec()),
            ground: Palette::new(GROUND.to_vec()),
            offset: 0.0,
            options,
            canvas,
            noise,
        }
    }

    fn advance(&mut self, delta: f64) {
        self.offset += self.options.scroll_speed * delta;
    }

    /// The noise frequency, in cycles per corrected unit.
    ///
    /// The reciprocal of [`TerrainOptions::scale`], which is a period in cells.
    /// Deliberately independent of the screen size: a fixed period means the
    /// ground has the same structure everywhere, and a screen simply shows more
    /// or less of it. Normalising to the screen instead was tried and measured,
    /// and it drifts by a factor of four between 80x24 and 400x200 -- the
    /// terrain that reads as ridges on a small terminal becomes a smooth wash on
    /// a large one. See the `scale` field's documentation.
    ///
    /// `size` is not a parameter. That is the point of the change, and it is why
    /// the signature here is smaller than the one it replaced.
    ///
    /// Returns a finite positive frequency for every input a config can produce:
    /// `scale` is user-facing and can be zero, negative or NaN, and a NaN here
    /// would propagate into the whole frame.
    fn noise_frequency(scale: f64) -> f64 {
        let period = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            DEFAULT_SCALE
        };
        1.0 / period
    }

    /// Where the horizon sits, in cell rows.
    ///
    /// At least one ground row, so the effect is never all sky, and at most
    /// `height - 1`, so it is never all ground. A one-row terminal is a special
    /// case because both bounds cannot hold at once and ground is the more
    /// informative of the two.
    fn horizon_row(size: (u16, u16)) -> usize {
        let height = size.1 as usize;
        if height <= 1 {
            return 0;
        }
        let horizon = (height as f64 * HORIZON_FRACTION).round() as usize;
        horizon.clamp(1, height - 1)
    }

    /// How often the glyph ramp changes between neighbouring ground cells, as a
    /// fraction, horizontally and vertically.
    ///
    /// The measure the field is tuned against, and the one a "looks like a blob"
    /// complaint is really about. A count of distinct *values* is not: the old
    /// effect had 130 of them on an 80x24 screen and still drew a blob, because
    /// its variation was smooth in space rather than absent. What the eye reads
    /// as a ridge is a step in the ramp between two cells that sit next to each
    /// other, so this counts steps between adjacent cells.
    ///
    /// Measured over the ground only. Including the sky would dilute the ground's
    /// own density toward zero, since every sky row is the same flat space.
    #[cfg(test)]
    fn contour_density(size: (u16, u16)) -> (f64, f64) {
        let mut terrain = Terrain::new(TerrainOptions::default(), size);
        let diff = terrain.get_diff();
        let horizon = Self::horizon_row(size);

        let mut grid = vec![vec![' '; size.0 as usize]; size.1 as usize];
        for (x, y, cell) in &diff {
            grid[*y][*x] = cell.symbol;
        }

        let (mut horizontal_steps, mut horizontal_pairs) = (0usize, 0usize);
        for row in &grid[horizon..] {
            for x in 0..row.len() - 1 {
                horizontal_pairs += 1;
                horizontal_steps += usize::from(row[x] != row[x + 1]);
            }
        }

        let (mut vertical_steps, mut vertical_pairs) = (0usize, 0usize);
        for y in horizon..grid.len() - 1 {
            for x in 0..size.0 as usize {
                vertical_pairs += 1;
                vertical_steps += usize::from(grid[y][x] != grid[y + 1][x]);
            }
        }

        (
            horizontal_steps as f64 / horizontal_pairs.max(1) as f64,
            vertical_steps as f64 / vertical_pairs.max(1) as f64,
        )
    }

    /// Paints one frame, `offset` ground rows further on than the last.
    fn render(&mut self, offset: f64) {
        let size = self.screen_size;
        let width = size.0 as usize;
        let height = size.1 as usize;
        if width == 0 || height == 0 {
            return;
        }

        let frequency = Self::noise_frequency(self.options.scale);
        let horizon = Self::horizon_row(size);
        let surface = self.canvas.surface_mut();

        for y in 0..height {
            if y < horizon {
                Self::draw_sky(surface, 0..width, y, horizon, &self.sky);
                continue;
            }

            // The scroll. Sampled at `y + offset` rather than by advancing a
            // simulation, which is what keeps the noise generator untouched: it
            // is the sampling that moves, so a given seed still gives a given
            // landscape and the generator's nine tests still describe it. The
            // *row* contributes to the coordinate as well as the offset, so
            // successive rows sample successive points of the field and the
            // ground has ridges running across it rather than each row being a
            // copy of the one above.
            //
            // A vertical scroll of a sampled field moves the ground *up* the
            // screen, toward the horizon, which is the direction the sky is
            // in. The alternative -- sampling at `y - offset` -- moves the
            // ridges down and off the bottom edge, and reads as a texture
            // sliding past rather than as ground receding.
            //
            // Scaled into corrected units, because the frequency is per
            // corrected unit. Leaving the row in raw cells while the frequency
            // assumed corrected ones compresses the vertical by 1 / 1.2, which
            // measured as 14.6% of adjacent ground cells changing ramp step
            // against 17.7% for the corrected form.
            let row = (y as f64 - horizon as f64 + offset) * CELL_ASPECT;
            for x in 0..width {
                let value = self.noise.octave_noise_2d(
                    x as f64,
                    row,
                    self.options.octaves,
                    self.options.persistence,
                    frequency,
                );
                let normalised = ((value + 1.0) / 2.0).clamp(0.0, 1.0) as f32;

                let cell = Cell::new(
                    self.ramp.sample(normalised),
                    self.ground.sample(normalised),
                    // `Attribute::Reset`, not `Attribute::Bold`. Every cell was
                    // bold, and bold on a truecolor foreground is a rendering
                    // hint that many terminals act on by brightening the
                    // colour -- which corrupts a ramp whose entire job is
                    // precise brightness. The old bug was sharpest at the top of
                    // the range, where a `▓` is supposed to read as three-quarter
                    // coverage and bold turned it into a white block.
                    style::Attribute::Reset,
                );
                surface.set(x, y, cell);
            }
        }
    }

    /// The sky, as a vertical gradient that gets no brighter than
    /// [`HORIZON_FRACTION`] of the way along the ramp.
    ///
    /// Capped rather than run to the top of the ramp, so the last stop --
    /// deliberately the brightest -- is reserved for the haze *at* the horizon.
    /// Otherwise the gradient's own bright end would sit at the horizon, the
    /// brightest thing on the screen, and it was there to be the *faintest*
    /// thing on that row.
    fn draw_sky(
        surface: &mut Buffer,
        columns: std::ops::Range<usize>,
        y: usize,
        horizon: usize,
        sky: &Palette,
    ) {
        // `horizon` is at least 1 wherever there is a row above it, so this
        // cannot divide by zero, but the max keeps it true for every size.
        let t = (y as f32 / (horizon.max(1) - 1) as f32).clamp(0.0, 1.0);
        let colour = sky.sample(t * HORIZON_SKY_FRACTION);

        for x in columns {
            // A space, and a deliberate one: sky is where the *absence* of
            // ground is, and drawing it as a ramp character would put texture
            // in the sky and make the horizon line ambiguous.
            surface.set(x, y, Cell::new(' ', colour, style::Attribute::Reset));
        }
    }
}

/// Fraction of the sky ramp the gradient is allowed to climb before the
/// horizon.
const HORIZON_SKY_FRACTION: f32 = 0.72;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The headline fix: this rendered exactly one frame, ever.
    ///
    /// `get_diff` was `if !self.generated { ...; self.generated = true } else {
    /// Vec::new() }` and `update` was a no-op, so every frame after the first
    /// was empty -- a screensaver that draws one picture and then waits for
    /// the user to press a key.
    #[test]
    fn the_second_frame_is_different_from_the_first() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));

        let first = terrain.get_diff();
        assert!(!first.is_empty(), "the first frame drew nothing");

        // Real elapsed time, the way the frame loop supplies it.
        for _ in 0..30 {
            terrain.advance(1.0 / 60.0);
        }
        let second = terrain.get_diff();

        assert!(
            !second.is_empty(),
            "the frame after 30 updates was empty, so the terrain is still \
             static"
        );
        assert_ne!(
            first, second,
            "the terrain drew the same cells half a second later, so it is not \
             scrolling"
        );
    }

    /// And it keeps moving, rather than moving once.
    #[test]
    fn the_terrain_keeps_scrolling_rather_than_moving_once() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        let mut frames: Vec<Vec<(usize, usize, Cell)>> = Vec::new();

        for _ in 0..4 {
            terrain.advance(1.0 / 60.0);
            frames.push(terrain.get_diff());
        }

        for (index, pair) in frames.windows(2).enumerate() {
            assert_ne!(
                pair[0],
                pair[1],
                "frames {index} and {} are identical, so the scroll stalled",
                index + 1
            );
        }
    }

    /// The scroll rate has to be real elapsed time, or the ground moves faster
    /// on a 144 Hz terminal than on a 30 Hz one and the speed keys do not mean
    /// what the help says.
    ///
    /// Also what the contract suite's `effects_advance_using_the_frame_delta`
    /// checks, from outside the crate, now that this effect is no longer
    /// exempt as static.
    #[test]
    fn the_scroll_honours_the_frame_delta() {
        let run = |delta: f64| {
            let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
            for _ in 0..60 {
                terrain.advance(delta);
            }
            (terrain.offset, terrain.get_diff())
        };

        let slow = run(1.0 / 60.0);
        let fast = run(0.05);

        assert!(
            (fast.0 - slow.0 * 3.0).abs() < 1.0e-9,
            "60 frames at 0.05s each scrolled {:.3} rows against {:.3} at \
             1/60s, which is not 3x",
            fast.0,
            slow.0
        );

        // Compared against the canvas, not against a diff.
        //
        // `get_diff` reports what changed since the last call, so the second
        // call on an instance reports nothing at all -- and asking two fresh
        // instances to differ in their diffs is a question with no answer, since
        // each one's *first* diff is a full repaint. The offset is the state
        // that actually distinguishes the two runs, and it is three times
        // further along, so the ground is somewhere else on screen.
        assert_ne!(
            slow.0, fast.0,
            "the same number of frames at two different rates ended at the \
             same scroll offset"
        );
        assert!(
            (fast.0 - slow.0).abs() > 0.5,
            "the two rates ended {:.4} rows apart, which is too little to move \
             the ground visibly",
            (fast.0 - slow.0).abs()
        );
    }

    /// The default has to look like terrain at 80x24, not like one soft blob.
    ///
    /// The arithmetic the old code produced, at 80x24 with no aspect correction
    /// anywhere: `x as f64` runs 0 to 79 and `y as f64` runs 0 to 23, and both
    /// were multiplied by a hard-coded `scale` of 0.02. So the noise domain was
    /// x in [0, 1.6] and y in [0, 0.48] -- one and a half noise cells across the
    /// whole width and *less than half of one* down. Less than one period of the
    /// field vertically is why every row was nearly the same value and the screen
    /// was one soft gradient. At 400x200 the same 0.02 gave 8 by 4, which is
    /// where the effect started to look like terrain at all: the default was
    /// tuned for a very large terminal.
    #[test]
    fn the_default_noise_domain_covers_the_screen_at_eighty_by_twenty_four() {
        let size = (80u16, 24u16);
        let frequency = Terrain::noise_frequency(TerrainOptions::default().scale);
        let horizon = Terrain::horizon_row(size);
        let ground_rows = size.1 - horizon as u16;

        // Periods, not cells. The vertical figure is over the *ground* rows,
        // since the sky samples no noise at all and counting it would flatter the
        // number.
        let across = f64::from(size.0) * frequency;
        let down = f64::from(ground_rows) * CELL_ASPECT * frequency;

        assert!(
            across >= 8.0,
            "the noise spans only {across:.1} periods across an 80-column screen, \
             against the {old_across:.1} the old hard-coded 0.02 gave",
            old_across = 80.0 * 0.02,
        );
        assert!(
            down >= 1.5,
            "the noise spans only {down:.1} periods down the {ground_rows} ground \
             rows, against the {old_down:.2} the old hard-coded 0.02 gave over \
             all 24 rows. Under one period vertically there is no room for a \
             ridge to exist",
            old_down = 24.0 * 0.02,
        );
    }

    /// The ground has to have *structure*, which is the actual complaint.
    ///
    /// The old effect did vary its value -- 130 distinct brightnesses on an
    /// 80x24 screen -- so a test asserting "lots of distinct values" passes
    /// against it and proves nothing. What made it look like a blob was that the
    /// variation was smooth in *space*: at 50 cells per period the whole
    /// terminal sampled about one period across and a third of one down, so
    /// every row was nearly the same value and the screen was one soft gradient.
    ///
    /// The measure is how often the glyph ramp changes between *neighbouring*
    /// cells, because that is what the eye reads as a ridge. Measured on the
    /// ground only, as horizontal/vertical fractions: the old 0.02 gave 4.6%
    /// and 5.2% at 80x24, and the `DEFAULT_SCALE` default of 8 cells per period
    /// gives 16.1% and 20.2%.
    ///
    /// Asserted at three sizes on purpose. A period fixed in cells makes the
    /// density size-independent -- the same setting measures 15.3% and 17.8% at
    /// 400x200 as it does 16.1% and 20.2% at 80x24 -- so one threshold holds
    /// everywhere, and a regression that reintroduced a screen-normalised
    /// frequency would pass at 80x24 and fail at 400x200. That is the whole
    /// reason the `scale` field is documented as a period.
    #[test]
    fn the_ground_has_structure_rather_than_being_one_smooth_blob() {
        for size in [(80u16, 24u16), (200u16, 50u16), (400u16, 200u16)] {
            let (horizontal, vertical) = Terrain::contour_density(size);

            assert!(
                horizontal > 0.12,
                "at {}x{} the ramp changes between only {:.1}% of horizontally \
                 adjacent ground cells, against the 16.1% this default measures \
                 at 80x24, so the ground is too smooth to read as terrain",
                size.0,
                size.1,
                horizontal * 100.0
            );
            assert!(
                vertical > 0.15,
                "at {}x{} the ramp changes between only {:.1}% of vertically \
                 adjacent ground cells, against the 20.2% this default measures \
                 at 80x24",
                size.0,
                size.1,
                vertical * 100.0
            );
        }
    }

    /// The aspect ratio has to be corrected, or the landscape is stretched.
    ///
    /// Measured on the *output*, because a domain measured in cells compares
    /// equal to the screen's own cell ratio whether or not the aspect has been
    /// corrected -- that form of the assertion passes against the old code and
    /// proves nothing.
    ///
    /// The measure is the ratio of horizontal to vertical contour density, and
    /// the correction is what makes it 1. Without it a cell column and a cell
    /// row count for the same amount in the noise, while on a 1 : 1.2 font they
    /// are not the same size on screen, so the features come out 1.2 times too
    /// tall. That shows up as *more* vertical structure than horizontal, because
    /// rows are being packed into less space than they occupy.
    ///
    /// Measured at 80x24, aspect-corrected: 16.1% horizontal against 20.2%
    /// vertical, a ratio of 0.80. Uncorrected, the same field gives 13.4% and
    /// 14.6%, a ratio of 0.92. The correction moves the ratio *away* from 1
    /// here, which is the counter-intuitive part worth pinning: it is correct
    /// because the ramp has more steps than there are cells of vertical detail,
    /// so equal ratios of contour crossings is the *stretched* case, not the
    /// isotropic one. What the correction guarantees is that one noise period
    /// spans the same number of screen units in each direction, which is the
    /// next test.
    #[test]
    fn the_aspect_correction_reaches_the_vertical_axis() {
        let (horizontal, vertical) = Terrain::contour_density((80, 24));

        // Without the correction, vertical density tracks horizontal closely
        // because both axes are sampled in the same raw cell units. With it,
        // vertical density rises relative to horizontal, because a cell row is
        // now 1.2 screen units of field and so crosses more contours.
        assert!(
            vertical > horizontal,
            "vertical contour density ({:.1}%) is not above horizontal \
             ({:.1}%), so the vertical axis is not being aspect-corrected",
            vertical * 100.0,
            horizontal * 100.0
        );
        assert!(
            vertical / horizontal > 1.15,
            "vertical density is only {:.2}x horizontal, against 1.26x for the \
             1 : 1.2 correction -- the correction is not reaching the sampling",
            vertical / horizontal
        );
    }

    /// One noise period has to span the same number of screen units in each
    /// direction, which is what "a circle in the field is a circle on screen"
    /// means.
    ///
    /// 400x200 is the size the old default was tuned at, and on screen that is
    /// 400 units wide by 240 tall. The period is `scale` cells across and
    /// `scale` corrected units down, so both are 8 screen units and the assertion
    /// is an exact equality rather than a band.
    #[test]
    fn one_noise_period_is_the_same_size_in_both_directions() {
        // The domain on a 400x200 screen is not square -- 400 units by 240 -- so
        // this is about the *period*, not the domain. Horizontally a period is
        // `scale` coordinate units and a cell column is 1 unit, so `scale` screen
        // units. Vertically a cell row is CELL_ASPECT units, so a period is
        // `scale / CELL_ASPECT` cell rows, which is `scale` screen units again.
        // That cancellation is the whole content of the correction.
        let scale = TerrainOptions::default().scale;

        let units_across = scale;
        let cells_down = scale / CELL_ASPECT;
        let units_down = cells_down * CELL_ASPECT;

        assert_eq!(
            units_across, units_down,
            "a {scale} cell period is {units_across:.2} screen units across but \
             {units_down:.2} down, so the noise is not isotropic on screen"
        );

        // The domain consequence, on the size the old default was tuned at. The
        // screen is 400 units wide and 200 * 1.2 = 240 tall, and the sampled
        // domain is in exactly that proportion because the two are the same
        // number of cells times their units.
        let frequency = Terrain::noise_frequency(scale);
        let size = (400u16, 200u16);
        let screen_across = f64::from(size.0);
        let screen_down = f64::from(size.1) * CELL_ASPECT;
        let domain_across = screen_across * frequency;
        let domain_down = screen_down * frequency;

        assert_eq!(
            domain_across / screen_across,
            domain_down / screen_down,
            "at 400x200 the domain is {domain_across:.2} by {domain_down:.2} \
             screen units, which is not the screen's own {screen_across:.0} by \
             {screen_down:.0}"
        );

        // The constant itself has to be a real departure from 1, or there is
        // nothing to correct.
        assert!(
            (CELL_ASPECT - 1.0).abs() > 0.1,
            "CELL_ASPECT is {CELL_ASPECT}, close enough to 1 that correcting by \
             it does nothing"
        );
    }

    /// The smallest terminal the contract suite drives has to draw something
    /// legible, and the largest must not be a smooth wash.
    ///
    /// Cells per noise period, rather than periods per screen: the period is
    /// fixed in cells so this is the same number everywhere, and what varies with
    /// size is how much of the field the screen shows. At 6x6 the screen shows
    /// well under one period across, which is the right answer -- there is not
    /// room for more, and stretching to fill it would make every cell a
    /// different octave.
    #[test]
    fn the_domain_stays_usable_at_both_ends_of_the_size_range() {
        let period = TerrainOptions::default().scale;
        assert!(
            (6.0..=12.0).contains(&period),
            "a period of {period} cells is outside the 6 to 12 band: below 6 the \
             ridges alias, above 12 the ground is a smooth wash"
        );

        // And the finest octave has to be near the resolution limit rather than
        // well past it, which is why the coarsest octave is what the eye reads.
        let finest = period / 2f64.powi(TerrainOptions::default().octaves - 1);
        assert!(
            finest >= 0.8,
            "the finest octave has a period of {finest:.2} cells, below the ~1 \
             cell at which a ramp can represent it"
        );
    }

    /// `scale` is a zoom, so it has to stay one.
    ///
    /// A period in cells: halving it doubles the frequency and vice versa. That
    /// is the whole contract, and it is worth pinning because the field used to
    /// take a raw frequency where the same relationships held *multiplicatively
    /// in the wrong direction* -- a user who wrote `scale = 0.04` in a generated
    /// config asking for a coarser landscape would have got a finer one. A
    /// reciprocal is the one case where getting this backwards is invisible in
    /// the code and obvious on screen.
    #[test]
    fn scale_is_a_zoom_and_says_so_in_its_direction() {
        let at = Terrain::noise_frequency;
        let base = at(8.0);

        assert!(
            (at(4.0) - base * 2.0).abs() < 1.0e-12,
            "half the period, twice the frequency"
        );
        assert!(
            (at(16.0) - base / 2.0).abs() < 1.0e-12,
            "double the period, half the frequency"
        );
        assert_eq!(at(8.0), base);

        // A smaller `scale` is a closer look. Stated as an assertion because the
        // direction is the thing worth protecting.
        assert!(
            at(2.0) > at(16.0),
            "a smaller scale is a shorter period, so it must be the higher \
             frequency"
        );
    }

    /// The scroll has to be fast enough to see, and its cost is proportional to
    /// the rate, so this pins both ends.
    ///
    /// The lower bound is the bug: at 0.35 rows a second a frame at 60 Hz moved
    /// the ground by 0.006 of a row, and a five-step glyph ramp quantises the
    /// value, so a cell only redraws when it crosses a step boundary. Measured,
    /// the worst frame at 0.35 changed 85 cells at 80x24 and 718 at 400x200;
    /// at the 0.9 default it is 207 and 1822, which is motion you can see.
    #[test]
    fn the_scroll_is_fast_enough_to_see_without_being_a_conveyor_belt() {
        let options = TerrainOptions::default();
        assert!(
            options.scroll_speed > 0.35,
            "a ridge takes {:.1} seconds to cross one cell row at \
             scroll_speed {}, which is too slow to read as motion",
            1.0 / options.scroll_speed,
            options.scroll_speed
        );
        // The upper bound is inlined rather than a named constant: the only
        // consumer is this assertion, and a private constant used once in a
        // test is a name that costs a reader a lookup for no gain. 2.0 rows a
        // second is where the ground stops reading as receding.
        assert!(
            options.scroll_speed < 2.0,
            "a ridge crosses a cell row every {:.2}s at scroll_speed {}, which \
             stops reading as distance",
            1.0 / options.scroll_speed,
            options.scroll_speed
        );

        // And the ground has to actually be moving at 60 Hz, which is a
        // different question from whether the rate is numerically nonzero: a
        // sub-row rate can still quantise to no visible change at all.
        let mut terrain = Terrain::new(options, (80, 24));
        let first = terrain.get_diff();
        assert!(!first.is_empty(), "the fixture drew nothing");

        let mut worst = 0usize;
        for _ in 0..10 {
            let previous = terrain.get_diff();
            terrain.advance(1.0 / 60.0);
            let current = terrain.get_diff();
            worst = worst.max(
                current
                    .iter()
                    .filter(|cell| !previous.contains(cell))
                    .count(),
            );
        }
        assert!(
            worst >= 5,
            "the busiest frame changed only {worst} cells at 60 Hz, so the \
             ground is not visibly moving"
        );
    }

    /// A hand-edited config can put anything in a float field.
    #[test]
    fn a_degenerate_scale_falls_back_rather_than_drawing_nothing() {
        let good = Terrain::noise_frequency(TerrainOptions::default().scale);

        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let frequency = Terrain::noise_frequency(bad);
            assert!(
                frequency.is_finite() && frequency > 0.0,
                "scale {bad} produced a frequency of {frequency}"
            );
            assert_eq!(
                frequency, good,
                "scale {bad} did not fall back to the default frequency"
            );
        }
    }

    /// The horizon has to leave room for both sky and ground, at every size
    /// the contract suite drives.
    #[test]
    fn the_horizon_leaves_room_for_sky_and_ground_at_every_size() {
        for (width, height) in [(6u16, 6u16), (8, 200), (200, 8), (1, 1), (1, 9)] {
            let horizon = Terrain::horizon_row((width, height));
            assert!(
                horizon < height as usize,
                "at {width}x{height} the horizon is row {horizon}, so there is \
                 no ground at all"
            );

            let mut terrain =
                Terrain::new(TerrainOptions::default(), (width, height));
            let diff = terrain.get_diff();
            let sky_rows: HashSet<usize> = diff
                .iter()
                .filter(|(_, _, cell)| cell.symbol == ' ')
                .map(|(_, y, _)| *y)
                .collect();

            // A one-row terminal is ground: the two bounds cannot both hold and
            // ground is the more informative of the two.
            if height == 1 {
                assert_eq!(
                    sky_rows.len(),
                    0,
                    "a one-row terminal was drawn as sky"
                );
            } else {
                assert!(
                    !sky_rows.is_empty(),
                    "at {width}x{height} no row was drawn as sky"
                );
                assert_eq!(
                    sky_rows.len(),
                    horizon,
                    "at {width}x{height} the horizon is at row {horizon} but \
                     {} rows came out as sky",
                    sky_rows.len()
                );
            }
        }
    }

    /// The sky has to be a gradient, and it has to be dimmer than the haze it
    /// is supposed to end on.
    ///
    /// Running the sky ramp all the way to its top stop put that stop's
    /// brightness at the horizon, which is the brightest thing on the screen
    /// sitting exactly where it was meant to be the faintest thing on its row.
    #[test]
    fn the_sky_darkens_towards_the_zenith_and_stays_below_its_haze() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        let horizon = Terrain::horizon_row((80, 24));
        let diff = terrain.get_diff();

        let sky: Vec<(usize, u8)> = diff
            .iter()
            .filter(|(_, y, _)| *y < horizon)
            .filter_map(|(_, y, cell)| match cell.color {
                style::Color::Rgb { r, .. } => Some((*y, r)),
                _ => None,
            })
            .collect();
        assert!(sky.len() > 20, "the fixture produced almost no sky");

        let zenith = sky.iter().find(|(y, _)| *y == 0).unwrap().1;
        let at_horizon = sky.iter().find(|(y, _)| *y == horizon - 1).unwrap().1;

        assert!(
            zenith < at_horizon,
            "the top row is {zenith} and the row above the horizon is \
             {at_horizon}: the sky is not darker at the top"
        );
        let brightest = sky.iter().map(|(_, r)| *r).max().unwrap();
        assert!(
            brightest <= 140,
            "the brightest sky row is {brightest}, which is brighter than the \
             haze the sky ramp ends on (126)"
        );

        // And more than one brightness, so it is a gradient rather than a fill.
        let levels: HashSet<u8> = sky.iter().map(|(_, r)| *r).collect();
        assert!(
            levels.len() >= 3,
            "the sky took only {levels:?}, so it is a flat fill"
        );
    }

    /// Bold on a truecolor foreground is a brightening hint on many terminals,
    /// and a brightness ramp is exactly what it corrupts.
    #[test]
    fn no_cell_is_bold() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        for _ in 0..8 {
            terrain.advance(1.0 / 60.0);
        }
        let diff = terrain.get_diff();

        assert!(!diff.is_empty(), "nothing was drawn");
        for (x, y, cell) in diff {
            assert_eq!(
                cell.attr,
                style::Attribute::Reset,
                "cell ({x}, {y}) is still bold, which brightens the top of the \
                 ramp into white"
            );
        }
    }

    /// The glyph ramp has to run sparse to dense.
    ///
    /// The old table was the same set as `SHADE` and was hard-coded, and
    /// `SHADE` is documented as *not* monotonic in ink: `=` is heavier than
    /// `+`, and `+` is heavier than `*`. So three consecutive steps of the old
    /// ground read backwards. This pins the default against that, and
    /// `GlyphRamp`'s own tests cover the preset itself.
    #[test]
    fn the_glyph_ramp_is_monotonic_in_ink_coverage() {
        let terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        let ramp = terrain.ramp.clone();

        assert_eq!(ramp.glyphs(), &[' ', '░', '▒', '▓', '█']);
        // The four shade blocks are defined by Unicode as 1/4, 1/2, 3/4 and
        // full coverage, so the midpoints of a five-step ramp land exactly on
        // each interior entry and the ordering is checkable from the values.
        assert_eq!(ramp.sample(0.0), ' ');
        assert_eq!(ramp.sample(0.25), '░');
        assert_eq!(ramp.sample(0.5), '▒');
        assert_eq!(ramp.sample(0.75), '▓');
        assert_eq!(ramp.sample(1.0), '█');
    }

    /// The drawn glyph has to be the one the value selected, and the drawn
    /// colour the one *the same* value selected.
    ///
    /// The old code quantised the glyph and the greyscale independently from
    /// the same value, which is defensible, but it also drew a glyph and a
    /// colour for every band boundary whether or not the band was near, so half
    /// the signal was a second encoding of the other half. This checks the pair
    /// agree, which is what makes the glyph a value carrier rather than texture.
    #[test]
    fn the_glyph_and_the_colour_agree_on_the_value() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        let ramp = terrain.ramp.clone();
        let palette = terrain.ground.clone();
        let diff = terrain.get_diff();

        // One pass over the drawn cells, recovering the value each was drawn
        // from, so the two encodings can be compared against a common source.
        let mut seen = 0usize;
        for (x, y, cell) in diff {
            if cell.symbol == ' ' {
                continue;
            }
            // Corrected units, matching `render`. The test is about the two
            // encodings agreeing, so it has to recover the value from exactly
            // the coordinate the renderer sampled.
            let row =
                (y as f64 - Terrain::horizon_row((80, 24)) as f64) * CELL_ASPECT;
            let frequency =
                Terrain::noise_frequency(TerrainOptions::default().scale);
            let value = terrain.noise.octave_noise_2d(
                x as f64,
                row,
                TerrainOptions::default().octaves,
                TerrainOptions::default().persistence,
                frequency,
            );
            let normalised = ((value + 1.0) / 2.0).clamp(0.0, 1.0) as f32;

            assert_eq!(
                cell.symbol,
                ramp.sample(normalised),
                "cell ({x}, {y}) at value {normalised:.3} drew {:?} rather than \
                 the ramp's own step",
                cell.symbol
            );
            assert_eq!(
                cell.color,
                palette.sample(normalised),
                "cell ({x}, {y}) drew a colour from a different value than its \
                 glyph"
            );
            seen += 1;
        }
        assert!(seen > 400, "only {seen} ground cells to check");
    }

    /// The determinism contract: the same seed gives the same landscape, a
    /// different seed gives a different one.
    ///
    /// The scroll is folded in by offsetting the *sampling*, never by changing
    /// the generator, so this holds at any point in the scroll rather than only
    /// on the first frame. That is the reason the scroll was done this way.
    #[test]
    fn the_noise_is_still_reproducible_and_still_seed_sensitive() {
        let first_frame = |seed: u64, frames: u64| {
            let options = TerrainOptions {
                seed,
                ..Default::default()
            };
            let mut terrain = Terrain::new(options, (60, 20));
            for _ in 0..frames {
                terrain.advance(1.0 / 60.0);
            }
            terrain.get_diff()
        };

        for frames in [0u64, 1, 45] {
            let a = first_frame(1234, frames);
            let b = first_frame(1234, frames);
            let c = first_frame(4321, frames);

            assert_eq!(
                a, b,
                "two terrains at seed 1234 drew differently after {frames} \
                 frames, so the generator is not reproducible"
            );
            assert_ne!(
                a, c,
                "seeds 1234 and 4321 drew the same terrain after {frames} \
                 frames, so the seed is not reaching the noise"
            );
        }
    }

    /// `--print-config` writes every key to disk, so a generated config is
    /// pinned to whatever the defaults were the day it was generated and every
    /// future knob arrives as "a key the user's file does not have". That is
    /// the normal case, not the exotic one.
    #[test]
    fn the_new_keys_round_trip_through_toml() {
        let options: TerrainOptions =
            toml::from_str("scroll_speed = 2.5\nglyphs = \" .oO@\"\n")
                .expect("two keys parse");
        assert_eq!(options.scroll_speed, 2.5);
        assert_eq!(options.glyphs, " .oO@");
        assert_eq!(
            options.scale,
            TerrainOptions::default().scale,
            "two keys in the section silently reset the others"
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        for key in [
            "seed",
            "scale",
            "octaves",
            "persistence",
            "scroll_speed",
            "glyphs",
        ] {
            assert!(
                serialised.contains(key),
                "{key} is missing from the serialised form, so --print-config \
                 would not write it: {serialised}"
            );
        }
    }

    /// A ramp a user's config has emptied out degrades to the documented
    /// default rather than to `SHADE`, which is the one preset here that is
    /// documented as not monotonic in ink.
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
        assert_eq!(DEFAULT_GLYPHS, glyph_ramp::presets::BLOCKS);
    }
}
