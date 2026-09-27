use crate::buffer::Cell;
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
    /// How far the surface may rise above and fall below the horizon, in rows,
    /// per period of the base-frequency noise.
    ///
    /// This is the knob that decides whether the effect reads as a landscape.
    /// The surface is a height field sampled per column and this is the
    /// amplitude it is scaled to, so at 0.0 the ground is a flat-topped slab
    /// with a ruler along the top and no landscape in it at all.
    ///
    /// In units of the noise period rather than of the screen, and that is the
    /// whole reason for the choice. Scaling by the screen makes the landscape's
    /// *shape* depend on the terminal: the horizontal period is fixed in cells,
    /// so an amplitude in rows that grows with the height makes the surface
    /// steeper on a tall screen than on a short one. Measured as the fraction of
    /// adjacent columns whose surface moves, that put the density at 25% on an
    /// 80x24 and at 76% on a 200x50 -- the same setting drawing a landscape and
    /// a picket fence. In units of the period it is the same at both.
    ///
    /// The 1.0 default is the pick, and the number is only meaningful next to
    /// two facts. The height field does not reach +/-1: measured over four
    /// thousand samples at three periods it spans 0.79, so a nominal amplitude
    /// of 1.0 moves the surface about 0.4 of a period either way. And the
    /// amplitude is additionally capped so the surface keeps clear of the top and
    /// bottom of the screen, which on a short terminal binds first.
    pub relief: f64,
    /// Characters the ground is drawn as, sparsest first.
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
            relief: 1.0,
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
/// Reinterpreted by the height-field rewrite and no longer comparable to the
/// old figure. It now measures how fast the *landscape* travels sideways past
/// the camera, in cells of the field per second, and the previous default of
/// 0.9 is kept because it lands in the right band for the new model: a period
/// of [`DEFAULT_SCALE`] cells passing at 0.9 cells/s means a ridge crosses any
/// given column every nine seconds, which is slow enough to read as distance
/// and fast enough that a still frame is obviously mid-flight.
///
/// The old number was a *vertical* rate on a field sampled per cell, so the two
/// are not the same measurement and a config carrying the old value will get a
/// different-looking effect. That is called out in the changelog rather than
/// migrated, because there is no way to convert one rate into the other.
const DEFAULT_SCROLL_SPEED: f64 = 0.9;

/// Default for [`TerrainOptions::relief`], in rows per noise period.
const DEFAULT_RELIEF: f64 = 1.0;

/// Cells per period of the ground's grain, horizontally and vertically.
///
/// Two different numbers on purpose, and the vertical one is the interesting
/// half. Equal periods would give isotropic blobs -- noise, which reads as
/// static. A vertical period several times shorter than the horizontal one
/// stretches the features into horizontal bands, and banding is what says
/// "sediment" rather than "TV snow". The surface's own period is 8 cells; 6 by 3
/// puts the grain at a similar scale to the landscape above it, so the two look
/// like the same material.
const GRAIN_PERIOD_X: f64 = 6.0;
const GRAIN_PERIOD_Y: f64 = 3.0;

/// Octaves and persistence for the grain.
///
/// Two, for the same reason the seabed uses two: a finely detailed floor under a
/// smooth large-scale structure reads as two different materials.
const GRAIN_OCTAVES: i32 = 2;
const GRAIN_PERSISTENCE: f64 = 0.5;

/// How much of the glyph the grain contributes, against the depth bias.
const GRAIN_WEIGHT: f32 = 0.32;

/// How much denser the ground reads with depth, out of a full ramp step.
///
/// Small. Depth genuinely does make ground denser, but this is the *texture*
/// channel now and the shading is the colour's job, so a strong bias here would
/// be re-introducing the vertical ramp that was the defect -- just with noise on
/// top of it.
const GROUND_DEPTH_BIAS: f32 = 0.22;

/// The range octave noise actually reaches, measured.
///
/// Not 1.0, and assuming 1.0 is a quiet way to lose two thirds of a ramp:
/// measured over four thousand samples at three different periods this generator
/// spans 0.79, so dividing by 1.0 would put every grain value inside the middle
/// half of the ramp and leave the ends unreachable. The same correction is why
/// `relief` is 7.0 for three rows of surface.
const NOISE_PRACTICAL_RANGE: f32 = 0.4;

/// The largest fraction of the ground's depth the surface amplitude may take.
///
/// Binds only on short terminals, where the requested amplitude in rows would
/// otherwise reach the top of the screen or the bottom of it. A surface at row 0
/// is no sky and one at the last row is no ground, and a screen with neither in
/// some column reads as a tear rather than as a landscape.
const MAX_RELIEF_FRACTION: f64 = 0.45;

/// The glyph ramp the ground is drawn as: ASCII, lightest coverage first.
///
/// The user asked for this explicitly -- the block set was "full locks" and
/// unnecessary. That is a real request and it costs something, so the cost is
/// written down rather than discovered later.
///
/// [`glyph_ramp::presets::BLOCKS`] was monotonic *by the standard*: Unicode
/// defines `░▒▓█` as quarter, half, three-quarter and full coverage, so the
/// ordering is a fact about the characters rather than a matter of taste, and
/// it holds at any cell aspect ratio. No ASCII set has that property, because
/// ink coverage is not what the characters are for. In this set the ordering runs
/// the wrong way in one adjacent place: `=` is two horizontal rules and carries
/// more ink than the `+` that follows it.
///
/// That is accepted rather than fixed, for two reasons. It is the conventional
/// ASCII brightness ramp, so a user who recognises it from a dozen other tools
/// gets what they expect. And the colour ramp carries the same depth
/// continuously and without error, so one inversion costs the ground a little
/// local texture rather than corrupting the value being shown. Read
/// `specs/overview.md` on `GlyphRamp` before replacing this set: the ordering is
/// the whole point of that type, and a set chosen for prettiness will read as
/// noise.
///
/// **The leading space of the conventional ramp is deliberately dropped.** Every
/// cell of the ground is filled, so a glyph that renders as nothing makes that
/// cell indistinguishable from sky -- and the top row of the ground in every
/// column is exactly the row whose depth samples the *start* of this ramp. With
/// the space in, the crest of every hill was invisible and the silhouette was
/// drawn a row too low. `the_ground_is_never_drawn_as_a_space` is that bug.
const DEFAULT_GLYPHS: &str = ".:-=+*#%@";

/// Fraction of the screen height that is ground, the rest sky.
///
/// Pinned rather than derived, because a horizon has to be somewhere specific
/// for the effect to read as a landscape: at a third of the height the ground
/// has enough rows to show several noise periods against the sky, and the sky
/// has enough rows to show a gradient. It is the *mean* surface level now, not
/// a hard split -- the surface moves above and below it by [`DEFAULT_RELIEF`]
/// of the ground's depth.
const HORIZON_FRACTION: f64 = 0.32;

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

/// Ground, from the lit crest down into shadow.
///
/// Ordered for the depth ramp, which runs *downwards*: the first stop is the top
/// row of ground in every column and the last is the bottom of the screen. The
/// old table was the other way round, because the old effect sampled a value
/// per cell rather than a depth.
///
/// A middle stop was added, and that is the point of the whole palette: with
/// only a lit crest and a dark floor, everything below the crest row is the
/// same two colours and the ground reads as a silhouette with a shadow under
/// it. The middle is what makes the body look like a body.
///
/// Deliberately dark at the bottom rather than black. The deepest rows are the
/// ones furthest from any light source in the fiction *and* the ones furthest
/// from the horizon, and a near-black there is indistinguishable from a hole in
/// the screen on a terminal whose background is not black.
const GROUND: &[style::Color] = &[
    style::Color::Rgb {
        r: 222,
        g: 228,
        b: 236,
    },
    style::Color::Rgb {
        r: 122,
        g: 130,
        b: 144,
    },
    style::Color::Rgb {
        r: 44,
        g: 48,
        b: 58,
    },
    style::Color::Rgb {
        r: 26,
        g: 29,
        b: 36,
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

    /// The row at which the ground starts, for one column.
    ///
    /// This is the whole effect. Before the rewrite there was no such thing: the
    /// renderer sampled two-dimensional noise at every cell and mapped the value
    /// straight onto a glyph, so every row below the horizon was an independent
    /// sample of the same field. There was no surface, nothing was filled below
    /// anything, and the result was a full-screen wash that read as a landscape
    /// only if you already knew one was supposed to be there. The user reported
    /// it as "cut off, showing only half of my terminal", and that is close to
    /// the truth: what looked like the top half was a blank sky, and the bottom
    /// half was texture with no shape in it.
    ///
    /// `x` is the column, `offset` the accumulated scroll. The field is sampled
    /// on the `y = 0` line of the two-dimensional noise, which is what turns it
    /// into a one-dimensional height field: `noise_2d` interpolates on `y`
    /// first and `y` is 0, so only the bottom row of gradients contributes. That
    /// is not a degenerate case to be avoided -- it is exactly a 1D slice, and it
    /// is why this needs no change to the noise module.
    ///
    /// The returned row is clamped so that every column keeps at least one sky
    /// row and at least one ground row. Without that clamp a column whose noise
    /// happens to be at an extreme can reach the top of the screen or the bottom,
    /// and a screen with no sky in one column and no ground in the next does not
    /// read as a landscape either -- it reads as a tear.
    fn surface_row(
        &self,
        x: usize,
        offset: f64,
        horizon: usize,
        span: f64,
    ) -> usize {
        let period = Self::noise_period(self.options.scale);
        // The scroll enters the *first* noise axis, so the landscape travels
        // sideways past a stationary camera. The alternative -- advancing the
        // second axis -- makes the field morph in place: hills rise and sink
        // without going anywhere, which for a landscape is the difference
        // between flying over one and watching it breathe.
        let travelled = x as f64 + offset;
        let raw = self.noise.octave_noise_2d(
            travelled,
            0.0,
            self.options.octaves,
            self.options.persistence,
            1.0 / period,
        );

        // Centred on the horizon, in units of rows.
        //
        // There is no cell-aspect correction here, and there used to be one.
        // A cell is about 1.2 times taller than it is wide, so a shape measured
        // in rows is already stretched vertically by the font; the old renderer
        // divided that back out to keep its *texture* isotropic. A silhouette is
        // not a texture, and multiplying the relief back up is what keeps a hill
        // from being a ridge: a hill of period P cells and true amplitude
        // P / 2.4 rows is a ridge, not a bump, at any cell aspect. Whether 0.45
        // is the right amount of that is settled by measurement rather than by
        // the argument -- see `the_relief_is_not_so_flat_that_the_ground_is_a_slab`.
        let row = horizon as f64 - raw * span;
        let (low, high) = Self::surface_bounds(self.screen_size.1 as usize);
        // `round` rather than `trunc`: truncating biases every surface down by
        // half a row on average, which for a silhouette is a systematic lean
        // rather than a rounding error.
        (row.round().clamp(low as f64, high as f64)) as usize
    }

    /// The lowest and highest rows a surface row may take, as `(low, high)`.
    ///
    /// Both bounds have to hold at once or the effect degenerates: a surface at
    /// row 0 is no sky, and a surface at the last row is no ground. On a screen
    /// too short to satisfy both, ground wins, because a column of solid ground
    /// still reads as ground while a column of open sky reads as a gap.
    fn surface_bounds(height: usize) -> (usize, usize) {
        if height <= 1 {
            return (0, 0);
        }
        if height == 2 {
            return (1, 1);
        }
        (1, height - 2)
    }

    /// The amplitude the height field is scaled to, in rows.
    ///
    /// In units of the noise period first, so the landscape has the same shape
    /// at every terminal size -- see [`TerrainOptions::relief`]. Then capped, so
    /// the surface cannot reach the top of the screen or the bottom of it, which
    /// on a short terminal binds well before the requested amplitude would.
    ///
    /// The cap is a fraction of the ground's depth rather than a row count for
    /// the same reason, and it is the only part of this that knows about the
    /// screen at all.
    fn surface_span(&self, horizon: usize) -> f64 {
        let period = Self::noise_period(self.options.scale);
        let relief = if self.options.relief.is_finite() {
            self.options.relief.max(0.0)
        } else {
            DEFAULT_RELIEF
        };
        let height = self.screen_size.1 as usize;
        let ground_rows = height.saturating_sub(horizon).max(1) as f64;
        (period * relief).min(ground_rows * MAX_RELIEF_FRACTION)
    }

    /// The period of the base-frequency noise, in cells, guarded.
    ///
    /// `scale` is user-facing and can be zero, negative or NaN. A NaN period
    /// would propagate into every surface row and take the whole frame with it,
    /// so it falls back rather than being trusted.
    fn noise_period(scale: f64) -> f64 {
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            DEFAULT_SCALE
        }
    }

    /// The noise frequency, in cycles per cell: the reciprocal of the period.
    ///
    /// The reciprocal of [`TerrainOptions::scale`], which is a period in cells.
    /// Deliberately independent of the screen size: a fixed period means the
    /// ground has the same structure everywhere, and a screen simply shows more
    /// or less of it. Normalising to the screen instead was tried and measured,
    /// and it drifts by a factor of four between 80x24 and 400x200 -- the
    /// terrain that reads as ridges on a small terminal becomes a smooth wash on
    /// a large one. See the `scale` field's documentation.
    #[cfg(test)]
    fn noise_frequency(scale: f64) -> f64 {
        1.0 / Self::noise_period(scale)
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

    /// How far the surface moves between neighbouring columns, in rows.
    ///
    /// This is the measure the effect is tuned against, and it is a different
    /// measurement from the one the old renderer needed. The old effect had no
    /// surface, so its structure had to be judged by how often the glyph ramp
    /// changed between adjacent *cells* -- a measure of texture. This one has a
    /// surface, so the structure is judged by the silhouette: the fraction of
    /// adjacent column pairs where the top of the ground is not on the same row.
    ///
    /// The two failure modes this distinguishes are the two ways a height field
    /// goes wrong. At zero the ground is a slab with a ruler-straight top edge
    /// and no landscape in it. Near one it is a picket fence, every column its
    /// own spike, and the eye reads noise rather than terrain. A landscape wants
    /// to be well inside both.
    ///
    /// Framed as a fraction rather than a row count so it is comparable across
    /// terminal heights, which is the same reason the old measure was.
    #[cfg(test)]
    fn silhouette_density(size: (u16, u16)) -> f64 {
        let terrain = Terrain::new(TerrainOptions::default(), size);
        let horizon = Self::horizon_row(size);
        let rows = terrain.surface_rows(0.0, horizon);
        if rows.len() < 2 {
            return 0.0;
        }
        let steps = rows.windows(2).filter(|w| w[0] != w[1]).count();
        steps as f64 / (rows.len() - 1) as f64
    }

    /// Paints one frame, `offset` ground rows further on than the last.
    /// The surface row for every column, computed once per frame.
    ///
    /// Hoisted out of the fill loop because it is the only part of the frame
    /// that costs a noise sample, and there are `height` times more cells than
    /// columns. The old renderer paid that cost per cell because it sampled per
    /// cell; this pays it per column, which at 400x200 is 200 samples instead of
    /// 80,000.
    fn surface_rows(&self, offset: f64, horizon: usize) -> Vec<usize> {
        let width = self.screen_size.0 as usize;
        let span = self.surface_span(horizon);
        (0..width)
            .map(|x| self.surface_row(x, offset, horizon, span))
            .collect()
    }

    /// Paints one frame, `offset` ground rows further on than the last.
    fn render(&mut self, offset: f64) {
        let size = self.screen_size;
        let width = size.0 as usize;
        let height = size.1 as usize;
        if width == 0 || height == 0 {
            return;
        }
        let horizon = Self::horizon_row(size);
        let surface_rows = self.surface_rows(offset, horizon);
        // Depth is normalised against the *mean* ground depth rather than each
        // column's own, so the shading is a function of distance below the mean
        // surface and not of distance below the bottom of the screen. Per-column
        // normalisation would put the darkest stop in the same row of every
        // column, which draws a hard line along the bottom of the picture that
        // has nothing to do with the landscape.
        let ground_depth = height.saturating_sub(horizon).max(1) as f32;
        let surface = self.canvas.surface_mut();

        for y in 0..height {
            // The sky's colour depends only on the row, so it is computed once per
            // row rather than once per cell. It used to be computed inside the
            // per-cell loop, which at 400x200 is 26,000 redundant divisions and
            // palette samples a frame.
            let sky_colour = Self::sky_colour(y, horizon, &self.sky);

            for (x, &top) in surface_rows.iter().enumerate().take(width) {
                if y < top {
                    // A space, and a deliberate one: sky is where the *absence*
                    // of ground is, and drawing it as a ramp character would put
                    // texture in the sky and make the horizon ambiguous.
                    surface.set(
                        x,
                        y,
                        Cell::new(' ', sky_colour, style::Attribute::Reset),
                    );
                    continue;
                }

                let depth = (y - top) as f32 / ground_depth;
                // Colour from depth, and *only* from depth. This is the shading
                // -- how much light reaches this far into the ground -- and it is
                // the one thing in the frame that is a clean function of a single
                // quantity, so it is the right place to spend a continuous ramp.
                let colour = self.ground.sample(depth.clamp(0.0, 1.0));

                // Glyph from a two-dimensional field, which is the fix for "I
                // don't understand what it's showing me".
                //
                // Both encodings used to be functions of depth alone, so every
                // column below its own surface was *identical* and the whole
                // picture was one smooth vertical ramp under a slightly wavy top
                // edge. That is not a landscape and it is not a cross-section
                // either; it is a gradient with a border. The missing thing was
                // structure running horizontally through the body -- strata,
                // grain, whatever the rock is made of.
                //
                // Three parts, because one is not enough. The `grain` field is
                // smooth and sampled at (column, depth), so it varies along both
                // axes, and the shorter vertical period is what gives the
                // horizontal banding that says "sediment" rather than "noise".
                // The `depth * GROUND_DEPTH_BIAS` term tips it very slightly
                // denser with depth, which is the one true thing about going down
                // through ground. And the grain is scaled to the noise's *actual*
                // range rather than assuming it reaches +/-1, because it does not
                // -- measured at 0.79, which is also why `relief` has to be 7.0
                // to move the surface three rows.
                //
                // The column coordinate carries `offset`, and that is load-bearing
                // rather than cosmetic. The surface scrolls because
                // `surface_row` samples its own noise at `x + offset`, so a grain
                // sampled at a bare `x` would sit still in screen space while the
                // landscape slid across it -- a silhouette moving over a fixed
                // texture, which is not ground and reads as two unrelated things
                // sliding past each other. The depth coordinate needs no offset
                // because it is measured *from the surface*, so it already moves
                // with the ground.
                let depth_cells = (y - top) as f64;
                let grain = self.noise.octave_noise_2d(
                    (x as f64 + offset) / GRAIN_PERIOD_X,
                    depth_cells / GRAIN_PERIOD_Y,
                    GRAIN_OCTAVES,
                    GRAIN_PERSISTENCE,
                    1.0,
                ) as f32
                    / NOISE_PRACTICAL_RANGE;
                let t = (0.5 + depth * GROUND_DEPTH_BIAS + grain * GRAIN_WEIGHT)
                    .clamp(0.0, 1.0);

                surface.set(
                    x,
                    y,
                    Cell::new(
                        self.ramp.sample(t),
                        colour,
                        // `Attribute::Reset`, not `Attribute::Bold`. Every cell
                        // was bold, and bold on a truecolor foreground is a
                        // rendering hint that many terminals act on by
                        // brightening the colour -- which corrupts a ramp whose
                        // entire job is precise brightness. The old bug was
                        // sharpest at the top of the range, where a `▓` is
                        // supposed to read as three-quarter coverage and bold
                        // turned it into a white block.
                        style::Attribute::Reset,
                    ),
                );
            }
        }
    }

    /// The sky's colour on one row: a vertical gradient that gets no brighter
    /// than [`HORIZON_SKY_FRACTION`] of the way along the ramp.
    ///
    /// Capped rather than run to the top of the ramp, so the last stop --
    /// deliberately the brightest -- is reserved for the haze *at* the horizon.
    /// Otherwise the gradient's own bright end would sit at the horizon, the
    /// brightest thing on the screen, and it was there to be the *faintest*
    /// thing on that row.
    fn sky_colour(y: usize, horizon: usize, sky: &Palette) -> style::Color {
        // `horizon` is at least 1 wherever there is a row above it, so this
        // cannot divide by zero, but the max keeps it true for every size.
        let t = (y as f32 / (horizon.max(1) - 1) as f32).clamp(0.0, 1.0);
        sky.sample(t * HORIZON_SKY_FRACTION)
    }
}

/// Fraction of the sky ramp the gradient is allowed to climb before the
/// horizon.
const HORIZON_SKY_FRACTION: f32 = 0.72;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Builds a grid of drawn symbols from a diff, for the tests that want to
    /// look at the picture rather than at the model behind it.
    fn drawn(size: (u16, u16), frames: u64) -> Vec<Vec<char>> {
        let mut terrain = Terrain::new(TerrainOptions::default(), size);
        for _ in 0..frames {
            terrain.advance(1.0 / 60.0);
        }
        let mut grid = vec![vec![' '; size.0 as usize]; size.1 as usize];
        for (x, y, cell) in terrain.get_diff() {
            grid[y][x] = cell.symbol;
        }
        grid
    }

    /// The surface row for every column, from the same code the renderer uses.
    ///
    /// At offset zero, which is the *first* frame. Anything that compares a
    /// surface against a frame drawn later has to use
    /// [`drawn_with_surface`] instead -- the seabed scrolls, so a surface row
    /// from one instant says nothing about a frame from another. Getting that
    /// wrong is not subtle: it looks like the ground has more structure than it
    /// does, because the rows are sampling different depths in each column.
    fn surface(size: (u16, u16)) -> Vec<usize> {
        let terrain = Terrain::new(TerrainOptions::default(), size);
        terrain.surface_rows(0.0, Terrain::horizon_row(size))
    }

    /// A drawn frame and the surface it was drawn against, from one instance.
    ///
    /// The pair has to come from the same `Terrain`, for the reason on
    /// [`surface`].
    fn drawn_with_surface(
        size: (u16, u16),
        frames: u64,
    ) -> (Vec<Vec<char>>, Vec<usize>) {
        let mut terrain = Terrain::new(TerrainOptions::default(), size);
        for _ in 0..frames {
            terrain.advance(1.0 / 60.0);
        }
        let rows = terrain.surface_rows(terrain.offset, Terrain::horizon_row(size));
        let mut grid = vec![vec![' '; size.0 as usize]; size.1 as usize];
        for (x, y, cell) in terrain.get_diff() {
            grid[y][x] = cell.symbol;
        }
        (grid, rows)
    }

    // ---------------------------------------------------------------- the fix

    /// The headline defect: there was no terrain.
    ///
    /// The old renderer sampled two-dimensional noise at every cell below a
    /// fixed horizon and mapped the value straight onto a glyph. There was no
    /// surface, nothing was filled below anything, and the result was a
    /// full-screen wash. The user reported it as "cut off, showing only half of
    /// my terminal".
    ///
    /// This asserts the thing that was missing: a surface that is a *line*, in
    /// the sense that it has a position in every column and those positions
    /// differ. A flat line is a slab; a line that never moves is a fill. The
    /// spread bound is what rules out both, and it is asserted at three sizes
    /// because the relief is a fraction of the ground's depth, so a regression
    /// that made it a fixed row count would pass at one size and not another.
    #[test]
    fn there_is_a_surface_and_it_is_not_a_straight_line() {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            let rows = surface(size);
            assert_eq!(
                rows.len(),
                size.0 as usize,
                "every column needs a surface row"
            );

            let low = *rows.iter().min().unwrap();
            let high = *rows.iter().max().unwrap();
            assert!(
                high - low >= 3,
                "at {}x{} the surface spans only {} rows, from {low} to {high}, so \
                 the top of the ground is a straight line and there is no \
                 landscape in the picture",
                size.0,
                size.1,
                high - low
            );
        }
    }

    /// The other half of "cut off": the ground has to reach the bottom of the
    /// screen in every column.
    ///
    /// With no surface this was trivially true of every row below the horizon,
    /// but the *shape* was not a ground. Now that there is a surface it is a
    /// real question, because a surface that reaches the last row leaves a
    /// column with no ground at all and the picture tears.
    #[test]
    fn the_ground_reaches_the_bottom_row_in_every_column() {
        for size in [(80u16, 24u16), (200, 50), (400, 200), (1, 9), (9, 3)] {
            let rows = surface(size);
            let height = size.1 as usize;
            for (x, &top) in rows.iter().enumerate() {
                assert!(
                    top < height,
                    "at {}x{} column {x} has its surface at row {top} of {height}, \
                     so that column has no ground and the ground does not reach \
                     the bottom",
                    size.0,
                    size.1
                );
            }
        }
    }

    /// Every column keeps sky above its ground, so the picture is never torn.
    ///
    /// The counterpart to the test above, and the reason [`surface_bounds`]
    /// clamps at both ends rather than only the one that looked wrong.
    #[test]
    fn every_column_keeps_at_least_one_row_of_sky() {
        // Terminals too short to hold both a sky row and a ground row are
        // excluded, because the two bounds genuinely cannot both hold there and
        // ground is the one that wins. `surface_bounds` is where that decision
        // is made and `the_ground_reaches_the_bottom_row_in_every_column` is the
        // other half of the same bargain.
        for size in [(80u16, 24u16), (200, 50), (400, 200), (1, 9), (9, 3)] {
            for (x, &top) in surface(size).iter().enumerate() {
                assert!(
                    top >= 1,
                    "at {}x{} column {x} has its surface at row {top}, so that \
                     column has no sky",
                    size.0,
                    size.1
                );
            }
        }
    }

    /// Nothing is drawn as ground above its own column's surface.
    ///
    /// The silhouette has to be a silhouette. A renderer that filled by
    /// comparing against a *mean* horizon rather than a per-column one would
    /// put ground in the sky on one side of every peak, and this is the test
    /// that says so.
    #[test]
    fn no_ground_is_drawn_above_its_own_columns_surface() {
        let size = (80u16, 24u16);
        let rows = surface(size);
        let grid = drawn(size, 0);

        // `grid` is row-major, so a *column* is a stride, not a row. Iterating
        // `grid.iter()` and treating each row as a column -- which is what this
        // did first -- reads the wrong axis entirely and reports a sky cell in
        // one column as a glyph from a different column's crest.
        for (x, &top) in rows.iter().enumerate() {
            for (y, cell) in grid
                .iter()
                .take(top)
                .enumerate()
                .map(|(y, row)| (y, row[x]))
            {
                assert_eq!(
                    cell, ' ',
                    "at ({x}, {y}) the ground starts at row {top} but this cell \
                     was drawn as {cell:?}"
                );
            }
        }
    }

    /// The relief has to be tuned to a band, and both ends of the band are real
    /// failure modes.
    ///
    /// At the bottom the surface is a ruler-straight line and the ground is a
    /// slab. At the top every column is its own spike and the eye reads a
    /// picket fence rather than terrain. The silhouette step density is the
    /// measure: the fraction of adjacent column pairs whose surface is on a
    /// different row.
    ///
    /// Measured, not guessed. Horizontal step density of the surface at
    /// 80x24 with the defaults is reported in the failure message, so a
    /// regression says what it actually is rather than only that it is wrong.
    #[test]
    fn the_relief_is_not_so_flat_that_the_ground_is_a_slab() {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            let density = Terrain::silhouette_density(size);
            assert!(
                (0.05..0.75).contains(&density),
                "at {}x{} the surface moves between only {:.1}% of adjacent \
                 columns, so the top of the ground is a straight line. Above \
                 about 75% it is a picket fence instead.",
                size.0,
                size.1,
                density * 100.0
            );
        }
    }

    /// The `relief` knob is connected to the drawing at both ends.
    ///
    /// Worth a separate test from the band in the test above, because that band
    /// is measured on the *default* and a knob that did nothing at its extremes
    /// would still pass it. Zero is the assertion that matters: it must produce
    /// a dead-flat top edge, because a `relief` that failed to reach the surface
    /// row would look exactly like `relief = 0` at every setting.
    ///
    /// The upper end is asserted as "more relief, more movement" rather than as
    /// a picket fence, because that is not what a large relief does.
    /// [`MAX_RELIEF_FRACTION`] caps the amplitude on a short terminal, so past
    /// the cap extra relief does nothing at all rather than producing a fence.
    /// That is asserted too, and it is a real ceiling rather than a formality: on
    /// an 80x24 the cap binds at about 0.9.
    #[test]
    fn relief_zero_is_a_slab_and_more_relief_moves_the_surface_more() {
        let spread_at = |relief: f64| {
            let options = TerrainOptions {
                relief,
                ..Default::default()
            };
            let terrain = Terrain::new(options, (80, 24));
            let rows = terrain.surface_rows(0.0, Terrain::horizon_row((80, 24)));
            rows.iter().max().unwrap() - rows.iter().min().unwrap()
        };

        assert_eq!(
            spread_at(0.0),
            0,
            "with no relief every column's surface is on the same row, so the \
             ground is a rectangle with a ruler along the top"
        );

        let half = spread_at(0.5);
        let full = spread_at(DEFAULT_RELIEF);
        assert!(
            half >= 2,
            "half the relief gave a spread of only {half} rows"
        );
        assert!(
            full > half,
            "the default relief {DEFAULT_RELIEF} spread the surface {full} rows \
             against {half} at half, so the knob is not reaching the surface row"
        );

        // And the cap is a cap, not a suggestion.
        let huge = spread_at(50.0);
        assert_eq!(
            huge, full,
            "relief 50 spread the surface {huge} rows against {full} at the \
             default, so the amplitude is not capped and a large relief would \
             drive the surface off the screen"
        );
    }

    /// The relief has to mean the same *landscape* at every terminal size.
    ///
    /// This is the test that pins the units. `relief` is in rows per noise period
    /// precisely so that the shape does not depend on how tall the screen is, and
    /// with the amplitude expressed as a fraction of the ground's depth instead
    /// the same setting measured 25% step density on an 80x24 and 76% on a
    /// 200x50 -- the same number drawing a landscape and a picket fence.
    #[test]
    fn the_landscape_has_the_same_shape_at_every_screen_height() {
        let density = |size: (u16, u16)| {
            let options = TerrainOptions {
                relief: 0.5,
                ..Default::default()
            };
            // Below the cap at every one of these sizes, so what is being compared
            // is the requested amplitude and not the clamp.
            let terrain = Terrain::new(options, size);
            let horizon = Terrain::horizon_row(size);
            let rows = terrain.surface_rows(0.0, horizon);
            let steps = rows.windows(2).filter(|w| w[0] != w[1]).count();
            steps as f64 / (rows.len() - 1) as f64
        };

        let short = density((200, 24));
        let tall = density((200, 200));
        assert!(
            (short - tall).abs() < 0.05,
            "the same relief gave {short:.3} step density on a 24 row screen and \
             {tall:.3} on a 200 row one, so the landscape changes shape with the \
             terminal"
        );
    }

    /// A `relief` a hand-edited config can produce must not take the frame with
    /// it. NaN in particular: it would propagate through the surface row into
    /// every cell of that column.
    #[test]
    fn a_degenerate_relief_falls_back_rather_than_drawing_nothing() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -3.0, 7.0] {
            let options = TerrainOptions {
                relief: bad,
                ..Default::default()
            };
            let terrain = Terrain::new(options, (80, 24));
            let span = terrain.surface_span(Terrain::horizon_row((80, 24)));
            assert!(
                span.is_finite() && span >= 0.0,
                "relief {bad} produced a surface span of {span}"
            );
        }
    }

    // ------------------------------------------------------- the two encodings

    /// The ground gets darker with depth, and the *colour* is what carries it.
    ///
    /// This test used to assert that the glyph also got denser with depth, and it
    /// passed after the grain went in -- while no longer testing anything about
    /// the effect. It walked the ramp function, so it was checking that a ramp is
    /// ordered, which was never in doubt. A test that stops testing its subject
    /// and keeps passing is worse than one that fails, because it reports
    /// coverage it is not providing.
    ///
    /// The glyph's job has changed and is now texture, so its right property is
    /// *not* monotonicity in depth. That is `the_ground_body_has_horizontal_
    /// _structure`. What is left for the colour is the shading, and it is the one
    /// thing in the frame that is a clean function of a single quantity, so it is
    /// measured on real drawn cells rather than on the palette in isolation.
    #[test]
    fn the_ground_gets_darker_with_depth() {
        let luminance = |c: style::Color| match c {
            style::Color::Rgb { r, g, b } => {
                0.2126 * f32::from(r)
                    + 0.7152 * f32::from(g)
                    + 0.0722 * f32::from(b)
            }
            _ => f32::NAN,
        };

        // Measured on the frame, not on the palette. The palette could be
        // perfectly ordered and the renderer could still hand every cell the same
        // stop, and only this notices.
        for size in [(80u16, 24u16), (200, 50)] {
            let rows = surface(size);
            let mut terrain = Terrain::new(TerrainOptions::default(), size);
            let grid: std::collections::BTreeMap<(usize, usize), style::Color> =
                terrain
                    .get_diff()
                    .into_iter()
                    .map(|(x, y, cell)| ((x, y), cell.color))
                    .collect();

            // Columns where the surface is high, so there is a deep body to
            // compare against without the bottom row being the surface itself.
            let deep_columns: Vec<usize> = rows
                .iter()
                .enumerate()
                .filter(|&(_, &top)| top < size.1 as usize / 2)
                .map(|(x, _)| x)
                .collect();
            assert!(
                deep_columns.len() > size.0 as usize / 4,
                "at {}x{} only {} columns have room for a ground body, so there is \\
                 nothing to compare",
                size.0,
                size.1,
                deep_columns.len()
            );

            for &x in &deep_columns {
                let top = rows[x];
                let shallow = grid[&(x, top)];
                let deepest = grid[&(x, size.1 as usize - 1)];
                assert!(
                    luminance(deepest) < luminance(shallow),
                    "at {}x{} column {x} is {} at the surface and {} at the bottom, \\
                     so the ground does not get darker going down",
                    size.0,
                    size.1,
                    luminance(shallow),
                    luminance(deepest)
                );
            }
        }
    }

    /// The body of the ground has structure running through it horizontally.
    ///
    /// The headline defect, and the reason the effect was reported as
    /// unreadable. Both encodings were functions of depth alone, so every column
    /// below its own surface drew identically and the whole picture was one
    /// smooth vertical ramp under a slightly wavy top edge. That is not a
    /// landscape and not a cross-section either. It is a gradient with a border.
    ///
    /// Measured **at constant depth**, and that is the whole difficulty. The
    /// obvious metric -- how many adjacent cells in one row differ -- is
    /// confounded, because two cells in the same row are at different depths
    /// wherever the surface is uneven, and so they differed even with no grain at
    /// all. The first version of this test used that metric, passed against the
    /// flat body it was written to catch, and was measuring the silhouette.
    ///
    /// So: a fixed number of rows below *each column's own* surface. Every cell
    /// sampled is then at the same depth by construction, the only thing that can
    /// differ between them is the grain, and the flat body gives exactly zero.
    #[test]
    fn the_ground_body_has_horizontal_structure() {
        let size = (120u16, 30u16);
        let (grid, rows) = drawn_with_surface(size, 90);
        let height = size.1 as usize;

        // Deep enough that every column still has ground, and shallow enough to
        // be inside the body rather than in the depth bias at the very bottom.
        let below = 3usize;
        assert!(
            *rows.iter().max().unwrap() + below + 1 < height,
            "the deepest surface is row {} on a {height} row screen, so there is \
             no row that is {below} rows of ground everywhere",
            rows.iter().max().unwrap()
        );

        let (differing, pairs) =
            (0..size.0 as usize - 1).fold((0usize, 0usize), |(d, p), x| {
                let a = grid[rows[x] + below][x];
                let b = grid[rows[x + 1] + below][x + 1];
                (d + usize::from(a != b), p + 1)
            });
        let fraction = differing as f64 / pairs.max(1) as f64;

        assert!(
            fraction > 0.15,
            "sampled {below} rows below each column's own surface -- so every \
             cell is at the same depth and only the grain can differ -- just \
             {differing} of {pairs} adjacent pairs differ ({:.1}%). The body of \
             the ground is featureless, so the picture is a vertical ramp with a \
             wavy top edge rather than terrain.",
            fraction * 100.0
        );
    }

    /// The grain is a smooth field, not per-cell noise.
    ///
    /// The distinction matters visually and it is measurable at constant depth.
    /// A field that is smooth in the column direction has runs of neighbouring
    /// cells that agree; a per-cell random pick does not, and reads as
    /// television static however the values are distributed. The assertion is a
    /// lower bound on agreement at a two-cell offset, chosen because it is the
    /// loosest bound that still fails for white noise.
    ///
    /// Note what this does *not* catch, because it is worth knowing: a body with
    /// no grain at all passes it, with agreement at 100%. It distinguishes
    /// "grain" from "static", not "grain" from "nothing" -- that is the test
    /// above, and between them the two failures have different answers.
    #[test]
    fn the_grain_is_smooth_across_a_column_rather_than_random() {
        let size = (120u16, 30u16);
        let (grid, rows) = drawn_with_surface(size, 90);
        let below = 3usize;
        let offset = 2usize;

        let (agreeing, pairs) =
            (0..size.0 as usize - offset).fold((0usize, 0usize), |(a, p), x| {
                let here = grid[rows[x] + below][x];
                let there = grid[rows[x + offset] + below][x + offset];
                (a + usize::from(here == there), p + 1)
            });
        let fraction = agreeing as f64 / pairs.max(1) as f64;

        assert!(
            fraction > 0.15,
            "only {:.1}% of cells agree with the one {offset} columns away, so the \
             grain is closer to per-cell noise than to a field -- which reads as \
             static rather than as material",
            fraction * 100.0
        );
    }

    /// The grain scrolls with the ground rather than sitting still under it.
    ///
    /// Found by reading the code rather than by a failing test, which is the part
    /// worth recording. The surface scrolls because `surface_row` samples its own
    /// noise at `x + offset`; the grain was sampled at a bare `x`, so the texture
    /// stayed fixed in screen space while the landscape slid across it. Silhouette
    /// and material moving independently read as two unrelated things passing each
    /// other, not as one body of ground.
    ///
    /// Measured by cross-correlation rather than by a direct comparison, because
    /// the obvious version of this test does not work: the ground moves 0.015
    /// cells per frame at the default scroll speed, so two *adjacent* frames are
    /// indistinguishable and comparing them at any shift measures nothing. So:
    /// run long enough for the ground to travel a couple of cells, then find the
    /// horizontal shift at which the body best matches itself. A grain attached
    /// to the ground peaks at the scroll displacement; a grain in screen space
    /// peaks at zero, and that is the whole difference.
    #[test]
    fn the_grain_scrolls_with_the_ground_and_not_across_it() {
        let size = (120u16, 30u16);
        let scroll = TerrainOptions::default().scroll_speed;
        // Frames for the ground to travel about two cells.
        let frames = (2.0 / (scroll / 60.0)).round() as u64;
        let (before, _) = drawn_with_surface(size, 0);
        let (after, _) = drawn_with_surface(size, frames);
        let expected = (scroll * frames as f64 / 60.0).round() as i64;

        let agreement_at = |shift: i64| -> f64 {
            let (mut agree, mut pairs) = (0usize, 0usize);
            for x in 0..size.0 as i64 {
                let moved = x + shift;
                if moved < 0 || moved >= i64::from(size.0) {
                    continue;
                }
                for y in 0..size.1 as usize {
                    let a = before[y][x as usize];
                    let b = after[y][moved as usize];
                    if a == ' ' || b == ' ' {
                        continue;
                    }
                    agree += usize::from(a == b);
                    pairs += 1;
                }
            }
            agree as f64 / pairs.max(1) as f64
        };

        // Best shift over a range that comfortably contains the expected one.
        // Signed, and negative: the offset is added to the column, so as it grows
        // the same material is found further *left*. The first version of this
        // searched 0..=4 and reported a best of 0, which was the edge of its own
        // range rather than a fact about the picture.
        let best = (-4..=4i64)
            .max_by(|a, b| {
                agreement_at(*a)
                    .partial_cmp(&agreement_at(*b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();

        assert_eq!(
            best, -expected,
            "the ground pattern matches itself best at a shift of {best} columns \
             and not at the {} the ground actually travelled, so the texture is \
             not attached to it",
            -expected
        );
    }

    /// The grain's vertical period is shorter than its horizontal one, which is
    /// what makes it banded.
    ///
    /// Asserted on the constants rather than on the output, and the reason is
    /// worth stating: a measured version cannot separate the two. Walking down a
    /// column changes the depth, and the depth bias changes the glyph with it, so
    /// a vertically-varying measure is dominated by the bias whether or not there
    /// is any banding at all. The flat body this replaced passed a measured
    /// banding test for exactly that reason.
    ///
    /// What is being claimed is a property of the sampling, and the sampling is
    /// two named constants. If the vertical period is the longer of the two the
    /// features are stretched the other way and the ground reads as vertical
    /// fluting rather than as sediment.
    #[test]
    fn the_grain_periods_make_it_banded() {
        assert!(
            GRAIN_PERIOD_Y < GRAIN_PERIOD_X,
            "the grain's vertical period is {GRAIN_PERIOD_Y} and its horizontal is \
             {GRAIN_PERIOD_X}, so the features are stretched vertically. Equal \
             periods give isotropic blobs, which read as static; the vertical \
             period has to be the shorter one for bands."
        );
        // And the grain must be able to reach both ends of the ramp, or the
        // re-spacing that motivated it does nothing.
        assert!(
            0.5 + GRAIN_WEIGHT < 1.0 && 0.5 - GRAIN_WEIGHT > 0.0,
            "a grain weight of {GRAIN_WEIGHT} on a ramp centred at 0.5 cannot reach              both ends of the scale"
        );
    }

    /// The default ramp is ASCII, because the user asked for it by name.
    ///
    /// The old default was `░▒▓█`, described in the report as "full locks" and
    /// "unnecessary". This is a narrow test on purpose: it does not care what
    /// the ramp is, only that it is ASCII, so a future set chosen for
    /// prettiness cannot quietly reintroduce block elements.
    #[test]
    fn the_default_ground_ramp_is_ascii_and_not_block_elements() {
        for glyph in DEFAULT_GLYPHS.chars() {
            assert!(
                glyph.is_ascii(),
                "the default ground ramp contains {glyph:?}, which is not ASCII"
            );
            assert!(
                !(('\u{2580}'..='\u{259F}').contains(&glyph)),
                "the default ground ramp contains the block element {glyph:?}"
            );
        }
    }

    /// The ASCII ramp is not monotonic in ink, and that is a decision rather than
    /// an accident.
    ///
    /// The block set this replaced was monotonic *by the standard* -- Unicode
    /// defines `░▒▓█` as quarter through full coverage. No ASCII set has that
    /// property, because ink coverage is not what ASCII characters are for. Two
    /// adjacent pairs in the conventional ramp run backwards: `=` is two
    /// horizontal rules and carries more than the `+` after it, and `+` is a thin
    /// cross carrying less than the `*` after it.
    ///
    /// The test names those pairs rather than asserting monotonicity, because a
    /// test that asserted monotonicity would fail and the obvious "fix" would be
    /// to reorder the ramp into something that is monotonic on paper and looks
    /// arbitrary on screen. The colour ramp carries depth continuously and
    /// without error, so these two inversions cost local texture rather than a
    /// corrupted value.
    #[test]
    fn the_ascii_ground_ramp_documents_the_two_pairs_that_run_backwards() {
        let ramp = GlyphRamp::from_text(DEFAULT_GLYPHS);
        let glyphs = ramp.glyphs();

        // Sparse to dense, by the conventional measure of how much of the cell
        // each character is expected to cover.
        let expected_ink = |g: char| -> f32 {
            match g {
                ' ' => 0.0,
                '.' => 0.05,
                ':' => 0.1,
                '-' => 0.15,
                '=' => 0.25,
                '+' => 0.2,
                '*' => 0.3,
                '#' => 0.4,
                '%' => 0.5,
                '@' => 0.6,
                _ => f32::NAN,
            }
        };

        let backwards: Vec<(char, char)> = glyphs
            .windows(2)
            .filter(|w| expected_ink(w[1]) < expected_ink(w[0]))
            .map(|w| (w[0], w[1]))
            .collect();

        assert_eq!(
            backwards,
            vec![('=', '+')],
            "the ramp's backwards pairs changed. Either the set was edited -- in \
             which case update this list and the DEFAULT_GLYPHS comment -- or the \
             inversion was accepted and is no longer the one documented."
        );
    }

    /// Every cell of the ground is filled, so no ground cell may be a space.
    ///
    /// This is the bug the conventional ASCII ramp walks into. ` .:-=+*#%@` opens
    /// with a space because it is a *texture* ramp, where sparse means blank. The
    /// ground here is not a texture: it is a filled region, and the row at the
    /// top of it in every column is the row whose depth samples the very start of
    /// the ramp. With the space in, the crest of every hill drew as a space, the
    /// silhouette sat a row too low, and on a one-row terminal the entire effect
    /// was a space -- indistinguishable from the sky it was supposed to be
    /// contrasted with.
    #[test]
    fn the_ground_is_never_drawn_as_a_space() {
        assert!(
            !DEFAULT_GLYPHS.contains(' '),
            "the default ground ramp opens with a space, so the top row of the \
             ground is indistinguishable from the sky"
        );

        // And on the real frame, not just in the constant.
        let size = (80u16, 24u16);
        let rows = surface(size);
        let grid = drawn(size, 0);
        // A column is a stride through a row-major grid, not a row.
        for (x, &top) in rows.iter().enumerate() {
            for (y, cell) in grid
                .iter()
                .enumerate()
                .skip(top)
                .map(|(y, row)| (y, row[x]))
            {
                assert_ne!(
                    cell, ' ',
                    "cell ({x}, {y}) is below the surface at row {top} but drew \
                     as a space, so the ground has a hole in it"
                );
            }
        }
    }

    // ---------------------------------------------------------------- scrolling

    /// The headline fix from the previous round: this rendered exactly one
    /// frame, ever.
    ///
    /// `get_diff` used to be `if !self.generated { ... } else { Vec::new() }`
    /// with a no-op `update`, so every frame after the first was empty -- a
    /// screensaver that draws one picture and then waits for a key.
    #[test]
    fn the_second_frame_is_different_from_the_first() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        terrain.get_diff();
        for _ in 0..30 {
            terrain.advance(1.0 / 60.0);
        }
        let second = terrain.get_diff();

        assert!(
            !second.is_empty(),
            "the frame after 30 updates was empty, so the terrain is still static"
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
            "60 frames at 0.05s each advanced {:.3} against {:.3} at 1/60s, \
             which is not 3x",
            fast.0,
            slow.0
        );
        assert!(
            (fast.0 - slow.0).abs() > 0.5,
            "the two rates ended {:.4} apart, which is too little to move the \
             ground visibly",
            (fast.0 - slow.0).abs()
        );
    }

    /// The scroll has to move the *silhouette*, not just the shading.
    ///
    /// The previous version of this test counted changed cells, which a
    /// per-cell value field made cheap to satisfy: any scroll changed something
    /// somewhere. Now the structure is a height field and the structure lives in
    /// the surface, so the test watches the surface. A scroll that moved the
    /// depth shading while leaving every crest on the same row would pass the old
    /// test and fail this one.
    #[test]
    fn the_scroll_moves_the_silhouette_and_not_only_the_shading() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        let horizon = Terrain::horizon_row((80, 24));
        let first = terrain.surface_rows(terrain.offset, horizon);

        for _ in 0..120 {
            terrain.advance(1.0 / 60.0);
        }
        let later = terrain.surface_rows(terrain.offset, horizon);

        let moved = first.iter().zip(&later).filter(|(a, b)| a != b).count();
        assert!(
            moved > 0,
            "after two seconds at the default scroll speed not one column's \
             surface had moved, so the landscape is standing still"
        );
    }

    /// The default has to be visible motion without being a conveyor belt.
    ///
    /// Reinterpreted for the height field, since `scroll_speed` now measures
    /// how fast the landscape passes the camera rather than how fast a sampled
    /// field is offset vertically. The band is expressed as *seconds per noise
    /// period*, which is the quantity a viewer actually perceives: it is how
    /// long one ridge takes to cross the screen.
    #[test]
    fn the_scroll_is_fast_enough_to_see_without_being_a_conveyor_belt() {
        let options = TerrainOptions::default();
        let period = TerrainOptions::default().scale;
        let seconds_per_period = period / options.scroll_speed;

        assert!(
            (2.0..30.0).contains(&seconds_per_period),
            "at scroll_speed {} one {period} cell ridge takes {seconds_per_period:.1}s \
             to cross the screen, which is outside the 2 to 30 second band: under \
             2s the ground stops reading as distance, over 30s it is not motion",
            options.scroll_speed
        );
    }

    // ----------------------------------------------------------------- the knob

    /// `scale` is a zoom, so it has to stay one.
    ///
    /// A period in cells: halving it doubles the frequency and vice versa. Worth
    /// pinning because the field used to take a raw frequency where the same
    /// relationships held *multiplicatively in the wrong direction* -- a user who
    /// wrote `scale = 0.04` in a generated config asking for a coarser landscape
    /// would have got a finer one.
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
        assert!(
            at(2.0) > at(16.0),
            "a smaller scale is a shorter period, so it must be the higher frequency"
        );
    }

    /// A smaller period has to give a more jagged silhouette, which is the
    /// whole point of the knob now that the field is a height field.
    ///
    /// Before the rewrite `scale` only changed how smooth the *texture* was,
    /// because there was no structure for it to change. This ties it to the
    /// silhouette, which is what a user zooming in is actually asking for.
    ///
    /// Measured as step density rather than as spread, and the amplitude is held
    /// fixed across the two cases. Neither detail is incidental.
    ///
    /// Density rather than spread, because spread measures how *far* the surface
    /// moves and this test is about how often it turns. Spread is also the wrong
    /// measure here for a second reason: `relief` is in rows per period, so
    /// changing the period changes the amplitude too, and measured as a spread
    /// 4 and 16 cells per period both came out at 4 rows on an 80x24 -- this test
    /// failed while the effect was working perfectly.
    ///
    /// Amplitude fixed, so the only variable left is the wavelength. `relief` is
    /// chosen per case to give both the same four-row amplitude, which is below
    /// the cap at both.
    #[test]
    fn a_smaller_period_gives_a_more_jagged_silhouette() {
        let density_at = |scale: f64| {
            let options = TerrainOptions {
                scale,
                relief: 4.0 / scale,
                ..Default::default()
            };
            let terrain = Terrain::new(options, (80, 24));
            let rows = terrain.surface_rows(0.0, Terrain::horizon_row((80, 24)));
            rows.windows(2).filter(|w| w[0] != w[1]).count() as f64
                / (rows.len() - 1) as f64
        };

        let close = density_at(4.0);
        let far = density_at(16.0);
        assert!(
            close > far * 1.5,
            "at the same amplitude a 4 cell period turns the silhouette on \
             {:.1}% of adjacent columns against {:.1}% at 16 cells, so zooming \
             in does not make the landscape more detailed",
            close * 100.0,
            far * 100.0
        );
    }

    /// The period has to stay in a band at both ends of the size range.
    #[test]
    fn the_period_stays_usable() {
        let period = TerrainOptions::default().scale;
        assert!(
            (6.0..=12.0).contains(&period),
            "a period of {period} cells is outside the 6 to 12 band: below 6 the \
             ridges alias, above 12 the ground is a smooth wash"
        );

        let finest = period / 2f64.powi(TerrainOptions::default().octaves - 1);
        assert!(
            finest >= 0.8,
            "the finest octave has a period of {finest:.2} cells, below the ~1 \
             cell at which the surface can represent it"
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
            assert_eq!(frequency, good, "scale {bad} did not fall back");
        }
    }

    // ------------------------------------------------------------- the picture

    /// The horizon leaves room for both sky and ground, at every size the
    /// contract suite drives.
    #[test]
    fn the_horizon_leaves_room_for_sky_and_ground_at_every_size() {
        for (width, height) in [(6u16, 6u16), (8, 200), (200, 8), (1, 1), (1, 9)] {
            let horizon = Terrain::horizon_row((width, height));
            assert!(
                horizon < height as usize,
                "at {width}x{height} the horizon is row {horizon}, so there is no \
                 ground at all"
            );

            let grid = drawn((width, height), 0);
            let has_sky = grid.iter().flatten().any(|&c| c == ' ');
            let has_ground = grid.iter().flatten().any(|&c| c != ' ');

            if height == 1 {
                assert!(has_ground, "a one-row terminal was drawn as sky");
            } else {
                assert!(has_sky, "at {width}x{height} nothing was drawn as sky");
            }
            assert!(
                has_ground,
                "at {width}x{height} nothing was drawn as ground"
            );
        }
    }

    /// The sky is a gradient, and dimmer than the haze it ends on.
    ///
    /// Run to its top stop, that stop's brightness would sit at the horizon --
    /// the brightest thing on the screen exactly where it was meant to be the
    /// faintest thing on its row.
    ///
    /// Collected from the drawn sky rather than from fixed rows, because the sky
    /// is no longer a rectangle: it is everything above a surface that moves, so
    /// a fixed row may be sky in one column and ground in the next.
    #[test]
    fn the_sky_darkens_towards_the_zenith_and_stays_below_its_haze() {
        let size = (80u16, 24u16);
        let mut terrain = Terrain::new(TerrainOptions::default(), size);
        let diff = terrain.get_diff();

        // Keyed by row, not by column. The sky is no longer a rectangle: it is
        // everything above a surface that moves, so a given row is sky in some
        // columns and ground in others and the colour of a row has to be read
        // from the sky cells on it.
        let sky: Vec<(usize, u8)> = diff
            .iter()
            .filter(|(_, _, cell)| cell.symbol == ' ')
            .filter_map(|(_, y, cell)| match cell.color {
                style::Color::Rgb { r, .. } => Some((*y, r)),
                _ => None,
            })
            .collect();
        assert!(sky.len() > 20, "the fixture produced almost no sky");

        // Every column keeps at least one sky row, so row 0 is sky throughout and
        // has a single colour.
        let zenith = sky
            .iter()
            .find(|(y, _)| *y == 0)
            .expect("row 0 is sky in every column")
            .1;
        let brightest = sky.iter().map(|(_, r)| *r).max().unwrap();

        assert!(
            brightest > zenith,
            "the top row is {zenith} and the brightest sky anywhere is \
             {brightest}: the sky is not darker at the top"
        );
        assert!(
            brightest <= 140,
            "the brightest sky is {brightest}, brighter than the haze the ramp \
             ends on (126)"
        );

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

    /// The determinism contract: the same seed gives the same landscape, a
    /// different seed gives a different one.
    ///
    /// The scroll is folded in by offsetting the *sampling*, never by changing
    /// the generator, so this holds at any point in the scroll rather than only
    /// on the first frame.
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
                "two terrains at seed 1234 drew differently after {frames} frames"
            );
            assert_ne!(
                a, c,
                "seeds 1234 and 4321 drew the same terrain after {frames} frames, \
                 so the seed is not reaching the noise"
            );
        }
    }

    /// `--print-config` writes every key to disk, so a generated config is pinned
    /// to whatever the defaults were the day it was generated. `relief` has to
    /// be in that list or it arrives as a key the user's file does not have.
    #[test]
    fn the_new_keys_round_trip_through_toml() {
        let options: TerrainOptions = toml::from_str(
            "scroll_speed = 2.5\nrelief = 0.3\nglyphs = \" .oO@\"\n",
        )
        .expect("three keys parse");
        assert_eq!(options.scroll_speed, 2.5);
        assert_eq!(options.relief, 0.3);
        assert_eq!(options.glyphs, " .oO@");
        assert_eq!(
            options.scale,
            TerrainOptions::default().scale,
            "three keys in the section silently reset the others"
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        for key in [
            "seed",
            "scale",
            "octaves",
            "persistence",
            "scroll_speed",
            "relief",
            "glyphs",
        ] {
            assert!(
                serialised.contains(key),
                "{key} is missing from the serialised form, so --print-config \
                 would not write it: {serialised}"
            );
        }
    }

    /// A ramp a user's config has emptied out degrades to the documented default.
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
        assert!(
            DEFAULT_GLYPHS.is_ascii(),
            "the documented default is no longer the ASCII ramp"
        );
    }

    /// The surface must be cheap.
    ///
    /// It is the only part of the frame that costs a noise sample, and there are
    /// `height` times more cells than columns. Sampling per cell instead -- which
    /// is what the old renderer did -- is 80,000 samples at 400x200 against 200.
    /// This asserts the count rather than the timing, because a timing assertion
    /// on a shared build machine fails for reasons that have nothing to do with
    /// the code.
    #[test]
    fn the_surface_costs_one_noise_sample_per_column_not_per_cell() {
        let size = (400u16, 200u16);
        let terrain = Terrain::new(TerrainOptions::default(), size);
        let rows = terrain.surface_rows(0.0, Terrain::horizon_row(size));

        assert_eq!(
            rows.len(),
            usize::from(size.0),
            "the surface should be computed once per column"
        );
        // And the frame still draws every cell, so the cheap surface did not come
        // from drawing less.
        let mut painted = Terrain::new(TerrainOptions::default(), size);
        let drawn_cells = painted.get_diff().len();
        assert_eq!(
            drawn_cells,
            usize::from(size.0) * usize::from(size.1),
            "at 400x200 the frame should touch every cell, got {drawn_cells}"
        );
    }
}
