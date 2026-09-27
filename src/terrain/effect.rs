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
    /// The far ridge's relief, in rows per noise period. Same units and the same
    /// cap as [`relief`](Self::relief), which is why it is a second copy of the
    /// knob rather than something shared: the two ridges are separate pieces of
    /// landscape and a user who wants a flat near hill and a rolling far one is
    /// asking a question a shared knob cannot answer.
    ///
    /// The 0.5 default is half the near ridge's, because distance flattens. A
    /// ridge a long way off subtends less vertical angle than the same ridge up
    /// close, and a far layer with as much relief as the near one reads as two
    /// cuts of the same hill rather than as distance. The near ridge is not the
    /// reference by accident -- the far ridge is *also* built from two octaves
    /// rather than four ([`FAR_OCTAVES`](Self::far_relief) below), which is the
    /// same statement made about detail rather than about amplitude. Neither
    /// half needs a knob of its own, and that is the point: two knobs for two
    /// ridges, and the smoothness is a constant.
    pub far_relief: f64,
    /// The far ridge's scroll speed as a *fraction* of the near one's.
    ///
    /// Below 1.0 is the whole meaning of the option -- distance scrolls slower,
    /// which is the one cue that separates a far layer from a near one more
    /// reliably than colour does, because a viewer reads rate as depth long
    /// before they read brightness as depth. A value above 1.0 is not rejected;
    /// it produces a far layer that outruns the near one, which is a real
    /// (if uncommon) effect and is the user's business rather than a mistake
    /// worth second-guessing. A negative value is treated as 0.0 and a
    /// non-finite one falls back to the default, for the reason `relief` does:
    /// this is a float in a hand-editable config and a NaN here would reach
    /// every surface row in the frame.
    ///
    /// The 0.35 default is a third of the near ridge's rate, and the number is
    /// pinned by measurement rather than by taste: at `scroll_speed` 0.9 and a
    /// period of 8 cells, the near ridge takes 8.9 seconds to cross the screen
    /// and the far one 25.4, and both sit inside the 2-to-30 second band the
    /// near ridge's own scroll test already holds it to. Anything much lower and
    /// the far ridge stops reading as scenery and starts reading as a still
    /// frame; the ratio is also large enough to be obvious in a side-by-side
    /// frame, which is the property `the_far_ridge_scrolls_more_slowly_than_\
    /// the_near_one` measures rather than asserts.
    pub parallax: f64,
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
            relief: DEFAULT_RELIEF,
            far_relief: DEFAULT_FAR_RELIEF,
            parallax: DEFAULT_PARALLAX,
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

/// Default for [`TerrainOptions::far_relief`], in rows per noise period.
///
/// Half the near ridge's, and the half is the point rather than a round number
/// found by eye: see the field's own documentation, and
/// `neither_ridge_is_a_flat_constant_row` for the measurement that keeps the far
/// ridge from degenerating into a ruler.
const DEFAULT_FAR_RELIEF: f64 = 0.5;

/// Default for [`TerrainOptions::parallax`], a fraction of the near ridge's rate.
const DEFAULT_PARALLAX: f64 = 0.35;

/// Mixed into the seed for the far ridge's own noise stream.
///
/// **This is a second generator, not a second rate on the first one.** The
/// obvious cheap version -- sample the near field at `x * 1.7`, or at
/// `x + 41.0`, or anywhere else -- gives a picture where the far ridge is the
/// *same hill twice*: one field, so the two profiles are the same shape at two
/// different rates, and the eye reads the repetition as a fault long before it
/// reads the colour difference as distance. Multiplying the sample coordinate
/// makes it worse rather than better, because a higher rate on one field is a
/// stretched version of that field, not an independent one.
///
/// So the far ridge gets its own permutation table, the way the crab's seabed
/// does, and the two landscapes are then unrelated rather than merely offset.
/// `the_two_ridges_are_two_landscapes_and_not_one_twice` measures the
/// difference; the salt is what it is measuring.
const FAR_SEED_SALT: u64 = 0xFA4_2AD1_5A9D_0001;

/// Octaves for the far ridge's profile.
///
/// Two, against the near ridge's four -- or rather, against the near ridge's
/// configured [`TerrainOptions::octaves`], which is four by default and which a
/// user may change without moving this. Distance is not only flatter, it is also
/// smoother: a ridge a long way off has no fine detail on its crest, and giving
/// it the same detail as the near one is what makes a distant layer look like a
/// nearer one seen through a colour filter.
///
/// A constant rather than a third knob, because it is a property of *being far*
/// rather than a thing a user is likely to want to change independently. It
/// borrows the crab's argument for a two-octave seabed: a finely detailed floor
/// under a smooth large-scale structure reads as two different materials, and
/// here the two materials are both supposed to be the same land.
const FAR_OCTAVES: i32 = 2;

/// Fraction of the screen height at which the far ridge's mean surface sits.
///
/// Above [`HORIZON_FRACTION`], not below it, and that ordering is the whole
/// constraint here. The two layers are painted far-first and near-second with no
/// depth buffer, so "behind" is decided entirely by which surface is higher on
/// the screen: put the far ridge's mean *below* the near one's and the dim
/// distant layer is drawn in front of the bright near one, which is not a
/// landscape, it is a mistake. The far ridge is therefore always the upper of
/// the two on average, and where a far crest is high enough to clear the near
/// ridge it simply shows through the gap, which is what a distant peak in a
/// valley does.
///
/// 0.16 against the near ridge's 0.32 puts the far ridge halfway up the sky's own
/// gradient, so the dim layer is never drawn against the pale haze at the
/// horizon -- where it would have least contrast -- and the sky still has room
/// above it at every size the effect is used at.
const FAR_RIDGE_FRACTION: f64 = 0.16;

/// Checked when the crate is compiled rather than when a test runs.
///
/// This was a runtime `assert!` in a test, which is `assert!(true)` -- clippy
/// says so, and it is right: the compiler evaluates the condition and drops the
/// check, so the test could never fail. A `const` block is checked for real and
/// fails the build, which is what an invariant between two named constants
/// wants. The message cannot be formatted here, because const panic takes a
/// literal, so it lives on the constants instead.
///
/// The two fractions are what make the layering possible at all: the far ridge
/// has to sit above the near one, or the dim layer is drawn in front of the
/// bright one. See [`FAR_RIDGE_FRACTION`].
const _: () = assert!(
    FAR_RIDGE_FRACTION < HORIZON_FRACTION,
    "the far ridge must sit above the near one, or the dim layer is drawn in front \
     of the bright one and the picture inverts"
);

/// The lit row along the top of the near ridge.
///
/// One row, one colour, and it is the reason this reads as a landscape rather
/// than as a cut-out. A filled region on its own is a silhouette, and a
/// silhouette with a bright edge along its top is a *hill*; without the rim the
/// same picture reads as a block of colour that happens to have a wavy top.
///
/// Brighter than every stop of the [`GROUND`] fill, not just its first. That is
/// the property the rim has to have to do its job, and it is a step rather than
/// a blend: a rim that faded into the fill would be a gradient, and a gradient
/// has no edge in it.
const NEAR_LIT: style::Color = style::Color::Rgb {
    r: 226,
    g: 232,
    b: 242,
};

/// The far ridge's fill: one flat colour, one flat glyph, no depth ramp.
///
/// A distant hill is a silhouette, and giving it shading would be claiming a
/// surface detail the layer exists to deny. The two-tone version of this layer
/// -- [`FAR_FILL`] with [`FAR_LIT`] along its top -- is the same trick as the
/// near ridge's rim, and it is what makes the far layer read as a *shape* rather
/// than as a tint over the sky.
///
/// Dimmer than the near ridge's darkest stop on purpose. It is the layer a
/// viewer should be able to ignore; if it competes with the near ridge for
/// attention the depth ordering stops being legible and both layers flatten
/// into one picture.
const FAR_FILL: style::Color = style::Color::Rgb {
    r: 58,
    g: 74,
    b: 104,
};

/// The lit row along the top of the far ridge.
///
/// Between [`FAR_FILL`] and the near ridge's rim in brightness, and for the same
/// reason each of those is where it is: the three tones have to be ordered far
/// rim, far body, near rim, near body, or the layers stop being separable at a
/// glance. It also sits deliberately close to the sky's brightest stop, the haze
/// at the horizon, which is the *aerial* half of the depth cue: a distant ridge
/// is lit by the same sky the viewer is looking at, so its top edge tends toward
/// the colour of the air above it.
const FAR_LIT: style::Color = style::Color::Rgb {
    r: 104,
    g: 124,
    b: 158,
};

/// Ground fill, from just under the lit rim down into shadow.
///
/// Ordered for the depth ramp, which runs *downwards*: the first stop is the
/// first row of fill in every column and the last is the bottom of the screen.
///
/// Every stop here is *below* [`NEAR_LIT`], which is the constraint the rim
/// imposes, and the top stop is well below it rather than just under it -- a
/// fill that starts almost as bright as its own rim has no rim. The bottom
/// stops short of black, as the previous version did: the deepest rows are the
/// ones furthest from any light in the fiction *and* the ones furthest from the
/// horizon, and a near-black there is indistinguishable from a hole in the
/// screen on a terminal whose background is not black.
///
/// Lighter at the bottom than the cross-section's body was, and deliberately. A
/// fill that runs to near-black over the lower two thirds of the screen is a
/// *hole* wearing a silhouette, which is a large part of why the cutaway read
/// as a cross-section rather than as ground you could stand on. Ground seen from
/// the side is lit, not buried.
const GROUND: &[style::Color] = &[
    style::Color::Rgb {
        r: 148,
        g: 156,
        b: 170,
    },
    style::Color::Rgb {
        r: 112,
        g: 120,
        b: 134,
    },
    style::Color::Rgb {
        r: 82,
        g: 88,
        b: 100,
    },
    style::Color::Rgb {
        r: 62,
        g: 67,
        b: 78,
    },
];

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
/// cell indistinguishable from sky -- and the topmost row of *fill* in every
/// column is exactly the row whose depth samples the start of this ramp. With
/// the space in, the row under the crest of every hill drew as a space and the
/// ground was a row of holes along its own silhouette.
///
/// The lit edge of each ridge is now drawn at the *densest* step of this ramp
/// rather than the start of it, so the rim is a solid line whatever the ramp is
/// and the ground below it still starts at the sparse end. Both facts are
/// pinned: `the_ground_is_never_drawn_as_a_space` is the bug above, and
/// `the_ascii_ground_ramp_documents_the_two_pairs_that_run_backwards` is the set
/// itself.
const DEFAULT_GLYPHS: &str = ".:-=+*#%@";

/// Fraction of the screen height at which the *near* ridge's mean surface sits.
///
/// Pinned rather than derived, because a horizon has to be somewhere specific
/// for the effect to read as a landscape: at a third of the height the ground
/// has enough rows to show several noise periods against the sky, and the sky
/// has enough rows to show a gradient. It is the *mean* surface level, not a hard
/// split -- the surface moves above and below it by [`DEFAULT_RELIEF`] of the
/// ground's depth, and the far ridge sits above it entirely; see
/// [`FAR_RIDGE_FRACTION`].
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
    /// The near ridge's noise stream, built from the seed alone.
    near_noise: PerlinNoise,
    /// The far ridge's, built from `seed ^ FAR_SEED_SALT`. A *second generator*,
    /// not a second rate on the first -- see [`FAR_SEED_SALT`] for why that
    /// distinction is the difference between two landscapes and one hill twice.
    far_noise: PerlinNoise,
    ramp: GlyphRamp,
    sky: Palette,
    ground: Palette,
    /// Cells of the near landscape scrolled past, from the frame delta.
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
        // in place meant a reset reused the previous terrain's landscape. Both
        // streams, for the same reason: a reset that rebuilt one of them would
        // swap a landscape for its own past.
        self.near_noise = PerlinNoise::new(self.options.seed);
        self.far_noise = PerlinNoise::new(self.options.seed ^ FAR_SEED_SALT);
        self.offset = 0.0;
    }
}

impl Terrain {
    pub fn new(options: TerrainOptions, screen_size: (u16, u16)) -> Self {
        let canvas = Canvas::new(screen_size.0, screen_size.1);
        let near_noise = PerlinNoise::new(options.seed);
        let far_noise = PerlinNoise::new(options.seed ^ FAR_SEED_SALT);

        Self {
            screen_size,
            ramp: glyph_ramp(&options.glyphs),
            sky: Palette::new(SKY.to_vec()),
            ground: Palette::new(GROUND.to_vec()),
            offset: 0.0,
            options,
            canvas,
            near_noise,
            far_noise,
        }
    }

    fn advance(&mut self, delta: f64) {
        self.offset += self.options.scroll_speed * delta;
    }

    /// The far ridge's scroll rate as a multiple of the near one, guarded.
    ///
    /// `parallax` is a float in a hand-editable config, so it can be NaN, and a
    /// NaN here would reach every far surface row in the frame and then every
    /// fill between two of them. Falling back to the documented default is the
    /// same decision `relief` makes, and for the same reason.
    fn parallax_rate(&self) -> f64 {
        if self.options.parallax.is_finite() {
            self.options.parallax.max(0.0)
        } else {
            DEFAULT_PARALLAX
        }
    }

    /// How far the far ridge has travelled, in cells, at the current offset.
    ///
    /// The whole of the parallax effect, and it is one multiplication. The near
    /// ridge's offset is in cells of *its own* field and the far ridge's is the
    /// same figure scaled by [`Self::parallax_rate`], so the two layers separate
    /// faster and faster as the run goes on rather than drifting apart at a
    /// constant distance -- which is the other thing that makes a pair of layers
    /// read as one landscape instead of as a loop of two.
    fn far_offset(&self) -> f64 {
        self.offset * self.parallax_rate()
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
    /// The row at which one ridge's surface starts, for one column.
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
    /// It is a height field and not a value, so it is computed per *column* and
    /// reused down the whole column rather than sampled per cell. That is the
    /// reason this is a function and not an inline expression in the fill loop.
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
        noise: &PerlinNoise,
        octaves: i32,
        x: usize,
        offset: f64,
        centre: usize,
        span: f64,
    ) -> usize {
        let period = Self::noise_period(self.options.scale);
        // The scroll enters the *first* noise axis, so the landscape travels
        // sideways past a stationary camera. The alternative -- advancing the
        // second axis -- makes the field morph in place: hills rise and sink
        // without going anywhere, which for a landscape is the difference
        // between flying over one and watching it breathe.
        let travelled = x as f64 + offset;
        let raw = noise.octave_noise_2d(
            travelled,
            0.0,
            octaves,
            self.options.persistence,
            1.0 / period,
        );

        // Centred on the ridge's own mean level, in units of rows.
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
        let row = centre as f64 - raw * span;
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

    /// The amplitude a ridge's height field is scaled to, in rows.
    ///
    /// In units of the noise period first, so the landscape has the same shape
    /// at every terminal size -- see [`TerrainOptions::relief`]. Then capped, so
    /// the surface cannot reach the top of the screen or the bottom of it, which
    /// on a short terminal binds well before the requested amplitude would.
    ///
    /// The cap is a fraction of the ground's depth rather than a row count for
    /// the same reason, and it is the only part of this that knows about the
    /// screen at all.
    ///
    /// `relief` is the near ridge's and `far_relief` the far one's, through the
    /// same function and under the same cap: the cap is what stops a short
    /// terminal from turning either layer into a stripe, and it has to bind on
    /// both or a small window would show one of them filling the screen.
    fn surface_span(&self, relief: f64, horizon: usize) -> f64 {
        let period = Self::noise_period(self.options.scale);
        let relief = if relief.is_finite() {
            relief.max(0.0)
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

    /// Where the near ridge's mean surface sits, in cell rows.
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

    /// Where the far ridge's mean surface sits, in cell rows.
    ///
    /// Strictly above [`Self::horizon_row`] by construction, and that ordering is
    /// the one invariant the whole layering rests on -- see
    /// [`FAR_RIDGE_FRACTION`], which pins the fraction and the `const` block
    /// below it checks the ordering at compile time.
    fn far_ridge_row(size: (u16, u16)) -> usize {
        let height = size.1 as usize;
        if height <= 1 {
            return 0;
        }
        let row = (height as f64 * FAR_RIDGE_FRACTION).round() as usize;
        // At most the near ridge's own row, so the two can never swap places on
        // a terminal too short for the fractions to be distinguishable. Below
        // row 1 there is no sky left at all, which the near ridge's clamp is
        // there to prevent.
        row.clamp(1, Self::horizon_row(size))
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
        let rows = terrain.near_surface_rows(0.0);
        if rows.len() < 2 {
            return 0.0;
        }
        let steps = rows.windows(2).filter(|w| w[0] != w[1]).count();
        steps as f64 / (rows.len() - 1) as f64
    }

    /// The near ridge's surface row for every column, at a given scroll.
    ///
    /// Hoisted out of the fill loop because it is the only part of the frame
    /// that costs a noise sample, and there are `height` times more cells than
    /// columns. The old renderer paid that cost per cell because it sampled per
    /// cell; this pays it per column, which at 400x200 is 200 samples instead of
    /// 80,000. The far ridge is the same trade on the same terms, so the frame
    /// costs two samples per column rather than one.
    fn near_surface_rows(&self, offset: f64) -> Vec<usize> {
        let centre = Self::horizon_row(self.screen_size);
        let span = self.surface_span(self.options.relief, centre);
        let noise = &self.near_noise;
        let octaves = self.options.octaves;
        (0..self.screen_size.0 as usize)
            .map(|x| self.surface_row(noise, octaves, x, offset, centre, span))
            .collect()
    }

    /// The far ridge's surface row for every column, at a given travelled
    /// distance.
    ///
    /// The argument is the *far* ridge's own distance, not the near one's, and it
    /// comes from [`Self::far_offset`] at every call site outside the tests. The
    /// multiplication that makes it the far ridge's lives in one place for that
    /// reason: a caller that passed `offset` here would get the near ridge's
    /// surface drawn with the far ridge's noise, which is a picture with no
    /// parallax in it rather than an error anything would notice.
    fn far_surface_rows(&self, offset: f64) -> Vec<usize> {
        let centre = Self::far_ridge_row(self.screen_size);
        let span = self.surface_span(self.options.far_relief, centre);
        let noise = &self.far_noise;
        (0..self.screen_size.0 as usize)
            .map(|x| self.surface_row(noise, FAR_OCTAVES, x, offset, centre, span))
            .collect()
    }

    /// Paints one frame, `offset` ground rows further on than the last.
    ///
    /// Three bands per column, in this order and with no depth buffer: sky above
    /// whichever ridge is higher, the far ridge between the two surfaces, and the
    /// near ridge from its own surface to the bottom of the screen. Which ridge
    /// wins a cell is decided by arithmetic on the two row arrays rather than by
    /// a draw order, which is why it costs nothing and why it cannot get out of
    /// step with the surfaces the same frame computed.
    ///
    /// The far ridge is allowed to be *below* the near one in a given column, and
    /// when it is it is simply not drawn there: the near ground covers it. That
    /// is not a special case bolted on, it is what a far ridge behind a near
    /// valley looks like, and a layer that could never be occluded would read as
    /// a band painted across the picture rather than as land behind other land.
    fn render(&mut self, offset: f64) {
        let size = self.screen_size;
        let width = size.0 as usize;
        let height = size.1 as usize;
        if width == 0 || height == 0 {
            return;
        }
        let horizon = Self::horizon_row(size);
        let near_rows = self.near_surface_rows(offset);
        let far_rows = self.far_surface_rows(self.far_offset());
        // Depth is measured from *this column's own* surface and normalised
        // against the mean ground depth, so the shading follows the ridge as the
        // ridge moves instead of being nailed to a row of the screen.
        //
        // That is a real decision rather than the obvious one, and it has a cost
        // worth naming: a column whose surface moves a row re-shades its whole
        // depth, because every cell in it is now one row further from the
        // surface. Measured, that is most of what this frame still emits -- 526
        // changed cells a frame at 400x200, against 77 if the fill were shaded by
        // row instead. It is kept because the alternative draws the same colour
        // on row 12 whether the ground there is the crest of a hill or the floor
        // of a valley, and a hilltop in shadow reads as a mistake.
        let ground_depth = height.saturating_sub(horizon).max(1) as f32;
        // The two glyphs the layers are drawn with, sampled once a frame rather
        // than per cell. The rim of each ridge takes the *densest* step, so the
        // lit edge is a solid line rather than a dotted one whatever the user has
        // configured the ramp to be; the far body takes the *sparsest*, because a
        // distant layer with texture in it reads as a nearer one seen through a
        // colour filter, which is the one thing it must not do.
        let lit_glyph = self.ramp.sample(1.0);
        let far_glyph = self.ramp.sample(0.0);
        let surface = self.canvas.surface_mut();

        for y in 0..height {
            // The sky's colour depends only on the row, so it is computed once per
            // row rather than once per cell. It used to be computed inside the
            // per-cell loop, which at 400x200 is 26,000 redundant divisions and
            // palette samples a frame.
            let sky_colour = Self::sky_colour(y, horizon, &self.sky);

            for x in 0..width {
                let near_top = near_rows[x];
                let far_top = far_rows[x];
                let cell = if y < near_top.min(far_top) {
                    // A space, and a deliberate one: sky is where the *absence*
                    // of ground is, and drawing it as a ramp character would put
                    // texture in the sky and make the horizon ambiguous.
                    Cell::new(' ', sky_colour, style::Attribute::Reset)
                } else if y < near_top {
                    // Between the two surfaces, so this cell belongs to the far
                    // ridge. Its own rim is the top row of the band.
                    let colour = if y == far_top { FAR_LIT } else { FAR_FILL };
                    Cell::new(far_glyph, colour, style::Attribute::Reset)
                } else if y == near_top {
                    Cell::new(lit_glyph, NEAR_LIT, style::Attribute::Reset)
                } else {
                    // Colour and glyph both from depth, and *only* from depth.
                    // The shading is how much light reaches this far down the
                    // near ground, and it is the one thing in the frame that is a
                    // clean function of a single quantity, so it is the right
                    // place to spend a continuous ramp.
                    //
                    // The old renderer also ran a two-dimensional grain field
                    // through here, to give the ground a body. There is no body:
                    // this is a view from the side, the ground is a filled region,
                    // and a body was never the thing that was missing. What was
                    // missing was a *second ridge*, and it cost one noise sample
                    // per ground cell to buy texture that read as noise -- 54,000
                    // samples a frame at 400x200 against 800 for both surfaces.
                    let depth =
                        ((y - near_top) as f32 / ground_depth).clamp(0.0, 1.0);
                    Cell::new(
                        self.ramp.sample(depth),
                        self.ground.sample(depth),
                        // `Attribute::Reset`, not `Attribute::Bold`. Every cell
                        // was bold, and bold on a truecolor foreground is a
                        // rendering hint that many terminals act on by
                        // brightening the colour -- which corrupts a ramp whose
                        // entire job is precise brightness. The old bug was
                        // sharpest at the top of the range, where a `▓` is
                        // supposed to read as three-quarter coverage and bold
                        // turned it into a white block.
                        style::Attribute::Reset,
                    )
                };
                surface.set(x, y, cell);
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
    use std::collections::{BTreeMap, HashSet};

    /// Rec. 709 relative luminance of a drawn colour, for the tests that compare
    /// one layer's brightness against another's.
    ///
    /// Weighted rather than an average of the channels, because the layers in
    /// this effect differ mostly in the *blue* channel -- they are all cool
    /// colours -- and a plain mean of the three would compress the differences
    /// the tests are looking for into a fraction of a step.
    fn luminance(colour: style::Color) -> f32 {
        match colour {
            style::Color::Rgb { r, g, b } => {
                0.2126 * f32::from(r)
                    + 0.7152 * f32::from(g)
                    + 0.0722 * f32::from(b)
            }
            _ => f32::NAN,
        }
    }

    /// How much brighter one tone has to be than the one below it to count as a
    /// separate tone at all.
    ///
    /// A bare `>` is not enough, and this number exists because a test using one
    /// passed against the defect it was written for. The near rim was set to
    /// exactly the first stop of the ground ramp -- the *same* colour as the top
    /// row of fill under it -- and every brightness assertion in the module still
    /// passed, because the row below samples the ramp a fraction of a step down
    /// and rounded back to within one unit. A ratio cannot be faked that way: at
    /// 1.0 the margin is zero, and the smallest real difference that survives
    /// `>` in this palette is about 1.004.
    ///
    /// 1.25 is the pick, and it sits well inside what the palette actually
    /// delivers: measured at the defaults the near rim is 1.50x the brightest
    /// cell of the near fill and the far rim 1.68x its own fill, while a rim set
    /// to its fill's first stop is 1.00x.
    const TONE_CONTRAST: f32 = 1.25;

    fn is_brighter(lit: style::Color, under: style::Color) -> bool {
        luminance(lit) > luminance(under) * TONE_CONTRAST
    }

    /// The near ridge's surface row for every column, at offset zero.
    ///
    /// Offset zero is the *first* frame. Anything that compares a surface against
    /// a frame drawn later has to use [`Frame`] instead -- the landscape scrolls,
    /// so a surface row from one instant says nothing about a frame from another.
    fn surface(size: (u16, u16)) -> Vec<usize> {
        Terrain::new(TerrainOptions::default(), size).near_surface_rows(0.0)
    }

    /// The far ridge's surface row for every column, at its own offset zero.
    ///
    /// Zero is the far ridge's zero as well as the near one's, since
    /// `far_offset` is a multiple of the near offset and both are zero at the
    /// start of a run.
    fn far_surface(size: (u16, u16)) -> Vec<usize> {
        Terrain::new(TerrainOptions::default(), size).far_surface_rows(0.0)
    }

    /// The mean number of changed cells per frame, after the opening frame.
    ///
    /// The opening frame is excluded because it is a full repaint by
    /// construction -- the canvas has no previous frame to diff against -- and
    /// averaging it in would hide the property this measures. What is left is
    /// the cost of *motion*, which is the part that decides how big the frame is
    /// on the wire.
    fn mean_frame_diff(size: (u16, u16), frames: u64) -> f64 {
        let mut terrain = Terrain::new(TerrainOptions::default(), size);
        // Establish the baseline, so the first measured frame is an ordinary
        // frame and not the opening repaint.
        terrain.get_diff();

        let mut total = 0usize;
        for _ in 0..frames {
            terrain.advance(1.0 / 60.0);
            total += terrain.get_diff().len();
        }
        total as f64 / frames as f64
    }

    /// A drawn frame together with the two surfaces it was drawn against.
    ///
    /// All three have to come from one `Terrain` instance, and the reason is not
    /// tidiness. The landscape scrolls, so a surface row from one instant says
    /// nothing about a frame from another; a test that reads a surface from a
    /// fresh effect at offset zero and compares it against a frame drawn at
    /// offset 1.5 is sampling every column at a different depth, and the
    /// resulting "measurement" is a property of the scroll rather than of
    /// anything it was written to look at.
    ///
    /// Colours are kept as well as glyphs because in this effect the two layers
    /// are told apart by colour and almost not at all by glyph: the far ridge is
    /// a sparse character and the near ground runs from sparse to dense down the
    /// screen, so any test about *which layer* a cell belongs to has to read the
    /// colour.
    struct Frame {
        glyphs: Vec<Vec<char>>,
        colours: BTreeMap<(usize, usize), style::Color>,
        /// The near ridge's surface row, per column.
        near: Vec<usize>,
        /// The far ridge's, per column.
        far: Vec<usize>,
    }

    impl Frame {
        fn new(size: (u16, u16), frames: u64) -> Self {
            let mut terrain = Terrain::new(TerrainOptions::default(), size);
            for _ in 0..frames {
                terrain.advance(1.0 / 60.0);
            }
            Self::with(&mut terrain)
        }

        /// A frame from an already-configured and already-advanced `Terrain`.
        ///
        /// The two surfaces are read from the same instance at the same offset
        /// the frame is drawn at, which is the only pairing that means anything.
        fn with(terrain: &mut Terrain) -> Self {
            let size = terrain.screen_size;
            let near = terrain.near_surface_rows(terrain.offset);
            let far = terrain.far_surface_rows(terrain.far_offset());
            let mut glyphs = vec![vec![' '; size.0 as usize]; size.1 as usize];
            let mut colours = BTreeMap::new();
            for (x, y, cell) in terrain.get_diff() {
                glyphs[y][x] = cell.symbol;
                colours.insert((x, y), cell.color);
            }
            Self {
                glyphs,
                colours,
                near,
                far,
            }
        }

        /// The glyph at one cell. Row-major storage, so a *column* is a stride
        /// through it rather than a row of it -- see the note on
        /// `no_ground_is_drawn_above_its_own_columns_surface`, which got that
        /// wrong and reported a sky cell in one column as a glyph from another
        /// column's crest.
        fn glyph(&self, x: usize, y: usize) -> char {
            self.glyphs[y][x]
        }

        fn colour(&self, x: usize, y: usize) -> style::Color {
            self.colours
                .get(&(x, y))
                .copied()
                .unwrap_or(style::Color::Reset)
        }

        /// How many columns show the far ridge rather than the sky, as a fraction.
        ///
        /// The far ridge is behind the near one, so it is *meant* to be hidden in
        /// some columns -- a layer that was never occluded would read as a band
        /// painted over the picture rather than as land behind land. This is the
        /// measure of how much of it is left, and it is the thing that has to
        /// stay well clear of both zero (no second layer at all) and one (a flat
        /// wall of dim colour with no near ridge in front of it).
        fn far_visible_fraction(&self) -> f64 {
            let width = self.near.len();
            if width == 0 {
                return 0.0;
            }
            let shown = self
                .far
                .iter()
                .zip(&self.near)
                .filter(|(far, near)| far < near)
                .count();
            shown as f64 / width as f64
        }
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
    ///
    /// Asserted for **both** ridges now, and the bound is looser for the far one
    /// because it has to be: `far_relief` defaults to half `relief` and two
    /// octaves rather than four, so a far surface that moves as much as the near
    /// one would be a distant layer with no distance in it. Two rows of spread
    /// is the floor below which the far ridge stops being a ridge at all, and
    /// `neither_ridge_is_a_flat_constant_row` is the same claim measured the
    /// other way round.
    #[test]
    fn there_is_a_surface_and_it_is_not_a_straight_line() {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            for (label, rows, minimum) in [
                ("near", surface(size), 3usize),
                ("far", far_surface(size), 2),
            ] {
                assert_eq!(
                    rows.len(),
                    size.0 as usize,
                    "every column needs a {label} surface row"
                );

                let low = *rows.iter().min().unwrap();
                let high = *rows.iter().max().unwrap();
                assert!(
                    high - low >= minimum,
                    "at {}x{} the {label} surface spans only {} rows, from {low} to \
                     {high}, so the top of that ridge is a straight line and there \
                     is no landscape in the picture",
                    size.0,
                    size.1,
                    high - low
                );
            }
        }
    }

    /// The other half of "cut off": the ground has to reach the bottom of the
    /// screen in every column.
    ///
    /// With no surface this was trivially true of every row below the horizon,
    /// but the *shape* was not a ground. Now that there is a surface it is a
    /// real question, because a surface that reaches the last row leaves a
    /// column with no ground at all and the picture tears.
    ///
    /// The far ridge is in the same assertion for the same reason, and this is
    /// where the two layers stop being symmetric: the far one is allowed to
    /// vanish in a column, because it is behind the near one, but the near one
    /// is not -- a column of near ground that stopped short of the bottom would
    /// be a hole in the front layer, and a hole in the front layer is the one
    /// thing the layering is supposed to make impossible.
    #[test]
    fn the_ground_reaches_the_bottom_row_in_every_column() {
        for size in [
            (80u16, 24u16),
            (200, 50),
            (400, 200),
            (1, 9),
            (9, 3),
            (6, 6),
            (20, 8),
        ] {
            let height = size.1 as usize;
            for (label, rows) in
                [("near", surface(size)), ("far", far_surface(size))]
            {
                for (x, &top) in rows.iter().enumerate() {
                    assert!(
                        top < height,
                        "at {}x{} column {x} has its {label} surface at row {top} of \
                         {height}, so that column has no ground at all",
                        size.0,
                        size.1
                    );
                }
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
            for (label, rows) in
                [("near", surface(size)), ("far", far_surface(size))]
            {
                for (x, &top) in rows.iter().enumerate() {
                    assert!(
                        top >= 1,
                        "at {}x{} column {x} has its {label} surface at row {top}, so \
                         that column has no sky",
                        size.0,
                        size.1
                    );
                }
            }
        }
    }

    /// Nothing is drawn as ground above the *higher* of the two surfaces.
    ///
    /// The silhouette has to be a silhouette. A renderer that filled by
    /// comparing against a *mean* horizon rather than a per-column one would
    /// put ground in the sky on one side of every peak, and this is the test
    /// that says so.
    ///
    /// The bound is the *minimum* of the two surfaces per column, not the near
    /// one, and that is the whole change from the single-ridge version. Where the
    /// far ridge is above the near one it is *supposed* to be drawn there, and a
    /// test that asserted "sky everywhere above the near surface" would be
    /// asserting the absence of the second layer -- it would pass against a
    /// renderer that drew no far ridge at all, which is the bug this rewrite
    /// exists to fix.
    #[test]
    fn no_ground_is_drawn_above_its_own_columns_surface() {
        let size = (80u16, 24u16);
        let frame = Frame::new(size, 0);

        for x in 0..size.0 as usize {
            let highest = frame.far[x].min(frame.near[x]);
            for y in 0..highest {
                let cell = frame.glyph(x, y);
                assert_eq!(
                    cell, ' ',
                    "at ({x}, {y}) the ground starts at row {highest} -- the far \
                     ridge is on {} and the near one on {} -- but this cell was \
                     drawn as {cell:?}",
                    frame.far[x], frame.near[x]
                );
            }
        }
    }

    /// The three bands are in the right order in every column, and the near ridge
    /// covers the far one completely wherever it is in front.
    ///
    /// This is the test that says what the effect *is*, as opposed to what it is
    /// made of. Read down any column and it has to be: sky, then the far ridge
    /// where the far ridge is above the near one, then the near ridge to the
    /// bottom of the screen. Nothing else, in any order, at any size.
    ///
    /// Asserted on drawn colours rather than on glyphs, because the two layers
    /// are almost indistinguishable by glyph -- the far body and the top of the
    /// near ground are both the sparsest step of the same ramp. The colours are
    /// constants rather than a gradient, so this can be an exact comparison,
    /// which is what makes it worth having: a renderer that put the far fill
    /// below the near surface, or drew the far ridge over the near one, would be
    /// caught by an equality test that a luminance band would have let through.
    #[test]
    fn every_column_is_sky_then_far_then_near_and_stops_there() {
        for size in [(80u16, 24u16), (200, 50), (400, 200), (20, 8)] {
            let frame = Frame::new(size, 0);
            let height = size.1 as usize;

            for x in 0..size.0 as usize {
                let (far, near) = (frame.far[x], frame.near[x]);
                for y in 0..height {
                    let colour = frame.colour(x, y);
                    if y >= near && y > near {
                        // The near fill is a depth ramp rather than a constant, so
                        // it is checked by range instead of by equality. Two
                        // properties, and the first is the important one: the
                        // near ridge occludes the far one completely, so no cell
                        // below its own surface may carry either of the far
                        // layer's colours. The second is that the fill stays well
                        // under its own rim, which is what makes the rim a rim --
                        // see `TONE_CONTRAST` for why that is a margin and not an
                        // inequality.
                        assert!(
                            colour != FAR_FILL && colour != FAR_LIT,
                            "at {}x{} cell ({x}, {y}) is below the near surface on row \
                             {near} but was drawn in the far layer's colour, so the \
                             near ridge is not occluding the far one",
                            size.0,
                            size.1
                        );
                        assert!(
                            is_brighter(NEAR_LIT, colour),
                            "at {}x{} cell ({x}, {y}) in the near fill is at {} against \
                             a lit rim at {}, so the fill is as bright as its own rim",
                            size.0,
                            size.1,
                            luminance(colour),
                            luminance(NEAR_LIT)
                        );
                        continue;
                    }

                    // Everything else is one of three constant colours, chosen
                    // purely by which band the row is in.
                    let (expected, band) = if y < far.min(near) {
                        // Sky. Compared as a glyph because the sky's colour is a
                        // gradient and there is nothing to compare it *to*; a
                        // space is the only thing a sky cell may be drawn as.
                        assert_eq!(
                            frame.glyph(x, y),
                            ' ',
                            "at {}x{} cell ({x}, {y}) is above both surfaces and was \
                             drawn as {:?} rather than sky",
                            size.0,
                            size.1,
                            frame.glyph(x, y)
                        );
                        continue;
                    } else if y < near {
                        (if y == far { FAR_LIT } else { FAR_FILL }, "far")
                    } else {
                        (NEAR_LIT, "near")
                    };
                    assert_eq!(
                        colour, expected,
                        "at {}x{} cell ({x}, {y}) belongs to the {band} band -- the \
                         far ridge is on {far} and the near one on {near} -- and was \
                         drawn in the other one",
                        size.0, size.1
                    );
                }
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
            let rows = Terrain::new(options, (80, 24)).near_surface_rows(0.0);
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

    /// `far_relief` is connected to the far ridge's surface, at both ends.
    ///
    /// The same three assertions as the near knob, and for the same reason: the
    /// band in `the_relief_is_not_so_flat_that_the_ground_is_a_slab` is measured
    /// on the *default*, so a second relief knob that was accepted by serde and
    /// then ignored would pass that band perfectly. Zero is the assertion that
    /// matters most -- a `far_relief` that never reached the far surface would
    /// look exactly like `far_relief = 0` at every setting, which is a dim band
    /// with a ruler along the top of it.
    ///
    /// The cap is asserted here too, and it is a *different* cap from the near
    /// ridge's in one respect: the far one is measured against the ground's
    /// depth as well, so on a short terminal both ridges flatten together rather
    /// than the far one taking the whole screen.
    #[test]
    fn far_relief_reaches_the_far_ridge_and_is_capped_like_the_near_one() {
        let spread_at = |far_relief: f64| {
            let options = TerrainOptions {
                far_relief,
                ..Default::default()
            };
            let rows = Terrain::new(options, (80, 24)).far_surface_rows(0.0);
            rows.iter().max().unwrap() - rows.iter().min().unwrap()
        };

        assert_eq!(
            spread_at(0.0),
            0,
            "with no far_relief every column's far surface is on the same row, so \
             the distant layer is a band with a ruler along the top"
        );

        // Both of these are *below* the cap, which is the whole difficulty on an
        // 80x24: `MAX_RELIEF_FRACTION` of this screen's ground depth is 7.2 rows,
        // so a period of 8 cells times the default 0.5 already asks for 4 of them
        // and twice the default asks for 8, which is over. The monotonic
        // comparison therefore runs at a quarter and a half rather than at a half
        // and a whole.
        let quarter = spread_at(DEFAULT_FAR_RELIEF / 2.0);
        let default = spread_at(DEFAULT_FAR_RELIEF);
        assert!(
            default >= 2,
            "the default far_relief {DEFAULT_FAR_RELIEF} spread the far surface only \
             {default} rows, so the distant layer has no shape in it"
        );
        assert!(
            default > quarter,
            "the default far_relief spread the far surface {default} rows against \
             {quarter} at half of it, so the knob is not reaching the far surface"
        );

        // And the cap is a cap, not a suggestion: two requests well past it give
        // the same surface, and the same one as the smallest request past it.
        let capped = spread_at(2.0);
        assert_eq!(
            spread_at(50.0),
            capped,
            "far_relief 50 spread the far surface differently from far_relief 2, so \
             the amplitude is not capped and a large far_relief would drive the \
             distant layer off the top of the screen"
        );
        assert!(
            capped > default,
            "past the cap the far surface spread {capped} rows against {default} at \
             the default, so the cap is not binding where it should"
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
            let rows = Terrain::new(options, size).near_surface_rows(0.0);
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
            let span = terrain.surface_span(bad, Terrain::horizon_row((80, 24)));
            assert!(
                span.is_finite() && span >= 0.0,
                "relief {bad} produced a surface span of {span}"
            );
        }
    }

    /// The same bargain for the two knobs the second ridge brought with it.
    ///
    /// `parallax` is the more dangerous of the two, and for a reason the
    /// `relief` case does not have: a NaN relief produces a NaN *span*, which
    /// `surface_row` rounds and clamps into a legal row and so degenerates
    /// quietly into a flat slab, whereas a NaN parallax reaches
    /// [`Terrain::far_offset`] and from there every far surface row, and a NaN
    /// that reaches a row index is a NaN that reaches the whole band below it.
    ///
    /// Asserted through the public path -- the effective rate and the span --
    /// rather than through the internals, because the fallback *is* the
    /// behaviour being pinned and reading the field back would pass even if the
    /// renderer used a different one.
    #[test]
    fn a_degenerate_far_relief_or_parallax_falls_back_rather_than_drawing_nothing()
    {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -3.0, 7.0] {
            let terrain = Terrain::new(
                TerrainOptions {
                    far_relief: bad,
                    ..Default::default()
                },
                (80, 24),
            );
            let span = terrain.surface_span(bad, Terrain::far_ridge_row((80, 24)));
            assert!(
                span.is_finite() && span >= 0.0,
                "far_relief {bad} produced a surface span of {span}"
            );

            let terrain = Terrain::new(
                TerrainOptions {
                    parallax: bad,
                    ..Default::default()
                },
                (80, 24),
            );
            let rate = terrain.parallax_rate();
            assert!(
                rate.is_finite() && rate >= 0.0,
                "parallax {bad} produced a rate of {rate}"
            );
            assert!(
                terrain.far_offset().is_finite(),
                "parallax {bad} produced a far offset of {}",
                terrain.far_offset()
            );
        }

        // And the fallbacks are the documented ones, not merely finite values.
        let terrain = Terrain::new(
            TerrainOptions {
                parallax: f64::NAN,
                ..Default::default()
            },
            (80, 24),
        );
        assert_eq!(
            terrain.parallax_rate(),
            DEFAULT_PARALLAX,
            "a NaN parallax did not fall back to the documented default"
        );
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
            let frame = Frame::new(size, 0);

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
                "at {}x{} only {} columns have room for a ground body, so there is \
                 nothing to compare",
                size.0,
                size.1,
                deep_columns.len()
            );

            for &x in &deep_columns {
                let top = rows[x];
                // The rim, not the first row of fill. This is the one thing that
                // changed in this test's subject, and it is worth being explicit
                // about: the top row of the near ground *is* its lit edge now, so
                // a version of this test that stopped one row lower would still
                // pass and would be measuring the rim-to-rim drop rather than the
                // shading. The assertion below is deliberately the widest drop in
                // the column -- rim to floor -- because that is the one that has
                // to exist for the fill to read as shaded ground at all, and it
                // is a margin rather than an inequality for the reason on
                // `TONE_CONTRAST`.
                let shallow = frame.colour(x, top);
                let deepest = frame.colour(x, size.1 as usize - 1);
                assert!(
                    is_brighter(shallow, deepest),
                    "at {}x{} column {x} is {} at the surface and {} at the bottom, \
                     so the ground does not get darker going down",
                    size.0,
                    size.1,
                    luminance(shallow),
                    luminance(deepest)
                );
            }
        }
    }

    /// There are two ridges, and they are two landscapes rather than one hill
    /// twice.
    ///
    /// The failure this is written against is a specific and easy one to commit:
    /// derive the far ridge from the *same* noise as the near one and rely on the
    /// rate to make it look different. It does not. One field sampled at two
    /// different offsets is one landscape, and the eye reports the repetition as
    /// a fault well before it reads the colour as distance -- which is why
    /// [`FAR_SEED_SALT`] is a second permutation table rather than a multiplier on
    /// the sample coordinate.
    ///
    /// Measured as correlation, and as the **worst** correlation over a range of
    /// horizontal shifts rather than the correlation at shift zero. The shift
    /// sweep is the part that matters: two copies of one profile will line up
    /// beautifully at *some* offset, and a test that only looked at shift zero
    /// would score that as "uncorrelated" because the layers happen to be at
    /// different mean heights. So the claim is that no shift makes them agree.
    ///
    /// Reported in the failure message, because a correlation near the bound is
    /// the thing to look at and a bare "not the same" is not.
    #[test]
    fn the_two_ridges_are_two_landscapes_and_not_one_twice() {
        /// Pearson correlation of two series.
        fn correlation(a: &[f64], b: &[f64]) -> f64 {
            let n = a.len() as f64;
            let mean_a = a.iter().sum::<f64>() / n;
            let mean_b = b.iter().sum::<f64>() / n;
            let covariance: f64 = a
                .iter()
                .zip(b)
                .map(|(x, y)| (x - mean_a) * (y - mean_b))
                .sum();
            let spread_a: f64 = a.iter().map(|x| (x - mean_a).powi(2)).sum();
            let spread_b: f64 = b.iter().map(|y| (y - mean_b).powi(2)).sum();
            covariance / (spread_a * spread_b).sqrt()
        }

        let size = (400u16, 50u16);
        let frame = Frame::new(size, 0);
        let near: Vec<f64> = frame.near.iter().map(|&r| r as f64).collect();
        let far: Vec<f64> = frame.far.iter().map(|&r| r as f64).collect();

        let worst = (-24i64..=24)
            .map(|shift| {
                let distance = shift.unsigned_abs() as usize;
                let base = if shift < 0 {
                    0..far.len() - distance
                } else {
                    distance..far.len()
                };
                let shifted: Vec<f64> = far[distance..].to_vec();
                correlation(&near[base], &shifted).abs()
            })
            .fold(0.0f64, f64::max);

        assert!(
            worst < 0.5,
            "the far ridge's profile correlates with the near one's at {worst:.2} at \
             the worst of 49 horizontal shifts, so the two layers are the same \
             landscape drawn twice rather than two landscapes"
        );
    }

    /// The far ridge is on screen, and it is behind the near one.
    ///
    /// What this can and cannot claim is worth being precise about, because the
    /// obvious version of the assertion is false. The far ridge is *never* hidden
    /// in most columns at the default settings, and the test that says so would
    /// be measuring the layout rather than the layering: `relief` is in rows per
    /// noise period, so on a 200 row terminal the near ridge's whole excursion is
    /// about six rows, and the far ridge's mean level is thirty-two rows above
    /// it. Two ridges that close cannot both be visible.
    ///
    /// So the bound here is only the half that must hold -- the far ridge is
    /// drawn across the frame rather than in a corner of it -- and the occlusion
    /// half is asserted separately, in a configuration built to produce it:
    /// `a_far_ridge_above_the_near_one_is_occluded_rather_than_drawn_over_it`.
    #[test]
    fn the_far_ridge_is_drawn_across_the_frame_at_the_default_settings() {
        for size in [(80u16, 24u16), (200, 50), (400, 200), (6, 6), (20, 8)] {
            let frame = Frame::new(size, 0);
            let visible = frame.far_visible_fraction();
            assert!(
                visible > 0.5,
                "at {}x{} the far ridge is above the near one in only {:.0}% of \
                 columns, so the second layer is a scrap at one edge of the frame",
                size.0,
                size.1,
                visible * 100.0
            );
        }
    }

    /// Where the far ridge is behind the near one, it is occluded -- not drawn
    /// over the top of it.
    ///
    /// The occlusion branch of the renderer is unreachable at the default
    /// settings, for the reason on the test above: the two mean levels are too far
    /// apart on a tall screen for either surface to reach the other. A code path
    /// that never runs is a path nobody has tested, and this one is the one that
    /// decides the *order* of the two layers, so it is built a configuration that
    /// reaches it -- both reliefs at 6.0, which puts the far surface's highest
    /// crest well below the near surface's lowest valley on a 200 row screen.
    ///
    /// Two assertions. That the occlusion actually happened, or the test is
    /// measuring a configuration that did not do what it was built to do; and that
    /// every occluded cell is near-ground, so a renderer that painted the far
    /// ridge last would be caught even though it would look almost identical in
    /// the columns where the far ridge is on top.
    #[test]
    fn a_far_ridge_above_the_near_one_is_occluded_rather_than_drawn_over_it() {
        let size = (200u16, 50u16);
        let options = TerrainOptions {
            relief: 6.0,
            far_relief: 6.0,
            ..Default::default()
        };
        let mut terrain = Terrain::new(options, size);
        let near = terrain.near_surface_rows(0.0);
        let far = terrain.far_surface_rows(0.0);
        let height = size.1 as usize;

        let occluded = far.iter().zip(&near).filter(|(f, n)| **f > **n).count();
        assert!(
            occluded > 0,
            "no column has the far ridge behind the near one, so this \
             configuration did not reach the occlusion path at all"
        );

        let painted = Frame::with(&mut terrain);
        for (x, (&far, &near)) in far.iter().zip(&near).enumerate() {
            for y in near..height {
                let colour = painted.colour(x, y);
                assert_ne!(
                    colour, FAR_FILL,
                    "cell ({x}, {y}) is below the near surface on row {near} and the \
                     far surface is on {far}, so the near ridge is in front, but the \
                     cell was drawn in the far layer's fill"
                );
                assert_ne!(
                    colour, FAR_LIT,
                    "cell ({x}, {y}) is below the near surface on row {near} but was \
                     drawn in the far layer's lit edge"
                );
            }
        }
    }

    /// Each ridge is lit along its own edge, and each rim is brighter than the
    /// fill under it.
    ///
    /// This is the part that makes a filled region read as a landscape. A
    /// silhouette with a bright edge along its top is a hill; the same silhouette
    /// without one is a block of colour that happens to have a wavy top, and that
    /// is a large part of why the first version of this effect read as
    /// "terrain" the user could not place.
    ///
    /// Measured on drawn cells at the *modelled* surface row, per column, so it
    /// cannot be satisfied by a rim drawn along a mean horizon. The far ridge's
    /// rim is only checked in the columns where the far ridge is visible at all,
    /// which is the honest form of the assertion: the far ridge is behind the near
    /// one and there is nothing to light where it is hidden.
    #[test]
    fn each_ridge_is_lit_along_its_own_edge_and_the_rim_is_brighter_than_its_fill()
    {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            let frame = Frame::new(size, 0);
            let height = size.1 as usize;

            for (x, (&near, &far)) in frame.near.iter().zip(&frame.far).enumerate()
            {
                assert_eq!(
                    frame.colour(x, near),
                    NEAR_LIT,
                    "at {}x{} column {x} has its near surface on row {near}, and that \
                     row is the lit edge, but it was not drawn in the rim colour",
                    size.0,
                    size.1
                );
                if near + 1 < height {
                    assert!(
                        is_brighter(NEAR_LIT, frame.colour(x, near + 1)),
                        "at {}x{} column {x} has a rim at {} and the row under it at \
                         {}, so the rim is not {}x brighter than its own fill",
                        size.0,
                        size.1,
                        luminance(NEAR_LIT),
                        luminance(frame.colour(x, near + 1)),
                        TONE_CONTRAST
                    );
                }

                if far < near {
                    assert_eq!(
                        frame.colour(x, far),
                        FAR_LIT,
                        "at {}x{} column {x} shows the far ridge from row {far}, and \
                         that row is its lit edge, but it was not drawn in the far \
                         rim colour",
                        size.0,
                        size.1
                    );
                    if far + 1 < near {
                        assert!(
                            is_brighter(FAR_LIT, frame.colour(x, far + 1)),
                            "at {}x{} the far rim in column {x} is at {} and the row \
                             under it at {}, so the far rim is not {}x brighter than \
                             the far layer's own fill",
                            size.0,
                            size.1,
                            luminance(FAR_LIT),
                            luminance(frame.colour(x, far + 1)),
                            TONE_CONTRAST
                        );
                    }
                }
            }
        }
    }

    /// The four tones are ordered, so the layers are separable at a glance.
    ///
    /// Far fill, then far rim, then near rim, then near fill. Read bottom to top
    /// in that order and the near ridge is a bright edge with a shaded body
    /// under it; get any one of the pairs the wrong way round and one of the two
    /// edges disappears into the thing it is supposed to be standing in front of.
    ///
    /// Every comparison is a *margin* rather than an inequality, for the reason on
    /// [`TONE_CONTRAST`]: with a bare `>` this test passed against a near rim set
    /// to exactly the ground ramp's first stop, which is a rim with no rim in it.
    ///
    /// The near fill's *brightest* value is what the first stop of the [`GROUND`]
    /// ramp produces, because the renderer samples that ramp by depth and the
    /// first row of fill in a column is the shallowest depth. Measured on drawn
    /// cells, so a palette that is ordered but never reached is caught.
    #[test]
    fn the_layer_tones_are_ordered_far_fill_far_rim_near_rim_near_fill() {
        let size = (200u16, 50u16);
        let frame = Frame::new(size, 0);
        let height = size.1 as usize;

        let mut brightest_fill = f32::NEG_INFINITY;
        for (x, (&near, &far)) in frame.near.iter().zip(&frame.far).enumerate() {
            if far < near {
                for y in (far + 1)..near {
                    brightest_fill =
                        brightest_fill.max(luminance(frame.colour(x, y)));
                }
            }
            for y in (near + 1)..height {
                brightest_fill = brightest_fill.max(luminance(frame.colour(x, y)));
            }
        }

        assert!(
            is_brighter(FAR_LIT, FAR_FILL),
            "the far layer's fill is at {} against its own rim at {}, so the distant \
             ridge has no edge",
            luminance(FAR_FILL),
            luminance(FAR_LIT)
        );
        assert!(
            is_brighter(NEAR_LIT, FAR_LIT),
            "the far rim is at {} against the near rim at {}, so the two ridges do \
             not separate by brightness",
            luminance(FAR_LIT),
            luminance(NEAR_LIT)
        );
        assert!(
            brightest_fill > luminance(FAR_FILL) * TONE_CONTRAST,
            "the far fill is at {} against the brightest cell of the near ground at \
             {brightest_fill:.0}, so the distant layer competes with the one in front \
             of it",
            luminance(FAR_FILL)
        );
        assert!(
            luminance(NEAR_LIT) > brightest_fill * TONE_CONTRAST,
            "the brightest cell of the near ground is at {brightest_fill:.0} against a \
             rim at {}, so the near ridge has no edge",
            luminance(NEAR_LIT)
        );
    }

    /// Each ridge is one unbroken run down its column, and the top of it is
    /// where the model says it is.
    ///
    /// Two things at once, and they are the same thing. Scanning down a column,
    /// the *first* non-space cell has to be the surface row and every cell from
    /// there to the bottom of the screen has to be ground. A hole anywhere in
    /// that run and a ridge whose top is a row away from its own surface are the
    /// same defect seen from two ends, and both were reachable before: the old
    /// renderer drew sky wherever a sampled value happened to be blank, so a
    /// column could have ground, then sky, then ground again.
    ///
    /// Measured on the frame against the surfaces the same frame was drawn from,
    /// which is the only pairing that means anything -- the landscape scrolls, so a
    /// surface from one instant and a frame from another are two different
    /// pictures.
    #[test]
    fn the_ground_is_one_unbroken_run_from_its_surface_to_the_bottom() {
        for size in [(80u16, 24u16), (200, 50), (400, 200), (20, 8)] {
            let frame = Frame::new(size, 0);
            let height = size.1 as usize;

            for (x, &top) in frame.near.iter().enumerate() {
                let first = (0..height)
                    .find(|&y| frame.glyph(x, y) != ' ')
                    .unwrap_or(height);
                assert_eq!(
                    first,
                    frame.far[x].min(top),
                    "at {}x{} column {x} draws its first ground cell on row {first}, \
                     but the far surface is on {} and the near one on {top}",
                    size.0,
                    size.1,
                    frame.far[x]
                );
                for y in top..height {
                    assert_ne!(
                        frame.glyph(x, y),
                        ' ',
                        "at {}x{} cell ({x}, {y}) is below the near surface on row \
                         {top} but drew as a space, so the ground has a hole in it",
                        size.0,
                        size.1
                    );
                }
            }
        }
    }

    /// Neither ridge's top edge is a straight line.
    ///
    /// The same failure the crab had, and the same reason it is worth a separate
    /// test rather than a clause in the one above. A renderer that filled against
    /// a *mean* horizon rather than a per-column surface would produce a frame
    /// that is perfectly contiguous -- every cell of the run filled, nothing
    /// above the surface drawn -- and completely flat. The contiguity test above
    /// passes it. This one does not, and it is measured on the frame's own
    /// topmost-drawn row per column rather than on the model's surface array,
    /// because a model that is flat and a picture that is flat are the same defect
    /// and the picture is the one the user sees.
    ///
    /// Three distinct top rows is the floor, and it is the same bound the single
    /// ridge version used, held against *both* layers. The far ridge's own relief
    /// is half the near one's, so if this ever needs loosening the near ridge is
    /// the one to look at.
    #[test]
    fn neither_ridge_is_a_flat_constant_row() {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            let frame = Frame::new(size, 0);
            let height = size.1 as usize;
            let mut near_tops: HashSet<usize> = HashSet::new();
            let mut far_tops: HashSet<usize> = HashSet::new();

            for x in 0..size.0 as usize {
                if let Some(first) = (0..height).find(|&y| frame.glyph(x, y) != ' ')
                {
                    near_tops.insert(first);
                    // The far surface, where the far ridge is the one on top. In
                    // the columns where the near ridge is higher, the first drawn
                    // row is the near one and the far ridge's own top is hidden,
                    // so it is not part of the set.
                    if frame.far[x] < frame.near[x] {
                        far_tops.insert(frame.far[x]);
                    }
                }
            }

            assert!(
                near_tops.len() >= 3,
                "at {}x{} the top edge of the near ground sits on only {} distinct \
                 rows, so it is a straight line and there is no landscape in the \
                 picture",
                size.0,
                size.1,
                near_tops.len()
            );
            assert!(
                far_tops.len() >= 3,
                "at {}x{} the top edge of the far ridge sits on only {} distinct \
                 rows, so the distant layer is a band with a ruler along the top",
                size.0,
                size.1,
                far_tops.len()
            );
        }
    }

    /// The far ridge scrolls measurably more slowly than the near one.
    ///
    /// This is the parallax, measured rather than asserted, and it replaces the
    /// test that asked whether the ground's *texture* scrolled with its
    /// silhouette. The old question is subsumed: if the surface that defines the
    /// texture moves at two different rates, then so does the texture.
    ///
    /// Three parts, each of which has to be there or the test is measuring
    /// something else:
    ///
    /// * The displacement is found by cross-correlating the surface rows and then
    ///   refining to a fraction of a cell with a parabola through the best
    ///   whole-cell shift. The fraction matters: at the default rate the near
    ///   ridge travels 0.015 cells a frame and the far one 0.005, so a whole-cell
    ///   answer would be a measurement of how far the ground happened to travel
    ///   and not of how fast it travels.
    /// * The search range is ±12 cells and both optima are asserted to be
    ///   *inside* it. This repo has been bitten by the opposite twice: a
    ///   correlation that reported its best shift at the edge of the range it
    ///   searched, which is a fact about the range and not about the picture. A
    ///   test that reported zero for a frozen effect would also have reported
    ///   zero here.
    /// * The ratio is compared against the configured `parallax` and separately
    ///   bounded well below 1. The second assertion is the one that would survive
    ///   a change of default; the first is the one that catches a knob that is
    ///   accepted by the config and then ignored.
    #[test]
    fn the_far_ridge_scrolls_more_slowly_than_the_near_one() {
        /// The shift at which `before` best matches `after`, in cells, signed.
        ///
        /// Negative, because the offset is *added* to the column: as it grows the
        /// same material is found further to the left.
        fn best_shift(before: &[usize], after: &[usize], limit: i64) -> f64 {
            let mean_error = |shift: i64| -> f64 {
                let (mut total, mut pairs) = (0.0, 0.0);
                for (x, &row) in before.iter().enumerate() {
                    let moved = x as i64 + shift;
                    if moved < 0 || moved >= after.len() as i64 {
                        continue;
                    }
                    let difference = row as f64 - after[moved as usize] as f64;
                    total += difference * difference;
                    pairs += 1.0;
                }
                total / pairs
            };

            let mut best = 0;
            let mut best_error = f64::INFINITY;
            for shift in -limit..=limit {
                let error = mean_error(shift);
                if error < best_error {
                    best_error = error;
                    best = shift;
                }
            }
            // A parabola through the best shift and its two neighbours, whose
            // vertex is the sub-cell optimum. The mean squared error against a
            // shifted surface is smooth in the shift, which is what makes this
            // legitimate rather than a guess.
            let (left, middle, right) =
                (mean_error(best - 1), mean_error(best), mean_error(best + 1));
            let curvature = left - 2.0 * middle + right;
            if curvature <= 0.0 {
                return best as f64;
            }
            best as f64 + 0.5 * (left - right) / curvature
        }

        let size = (200u16, 50u16);
        let options = TerrainOptions::default();
        // Nine seconds, which is about eight cells for the near ridge and under
        // three for the far one. Both comfortably inside the search range, and
        // both far enough that a one-frame rounding difference is a small part of
        // the answer.
        let seconds = 9.0;
        let frames = (seconds * 60.0) as u64;

        let mut terrain = Terrain::new(options.clone(), size);
        let near_before = terrain.near_surface_rows(0.0);
        let far_before = terrain.far_surface_rows(0.0);
        for _ in 0..frames {
            terrain.advance(1.0 / 60.0);
        }
        let near_after = terrain.near_surface_rows(terrain.offset);
        let far_after = terrain.far_surface_rows(terrain.far_offset());

        let limit = 12i64;
        let near_shift = best_shift(&near_before, &near_after, limit);
        let far_shift = best_shift(&far_before, &far_after, limit);

        for (label, shift) in [("near", near_shift), ("far", far_shift)] {
            assert!(
                shift.abs() < limit as f64 - 1.0,
                "the {label} ridge's best-matching shift is {shift:.2} cells, which is \
                 at the edge of the -{limit}..={limit} range that was searched, so the \
                 measurement is a fact about the range rather than about the picture"
            );
        }

        let expected = options.scroll_speed * seconds;
        assert!(
            (near_shift + expected).abs() < 0.5,
            "the near ridge travelled {expected:.1} cells in {seconds}s but its \
             surface matches itself best at a shift of {near_shift:.2}"
        );
        assert!(
            (far_shift + expected * options.parallax).abs() < 0.4,
            "the far ridge should have travelled {:.1} cells at parallax {} but its \
             surface matches itself best at a shift of {far_shift:.2}",
            expected * options.parallax,
            options.parallax
        );

        let ratio = far_shift / near_shift;
        assert!(
            (0.0..0.6).contains(&ratio),
            "the far ridge travels at {ratio:.2} of the near ridge's rate, which is \
             not visibly slower"
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
    ///
    /// Two rows are now exempt from the depth ramp rather than one: each ridge's
    /// lit edge is drawn at the *densest* step, so the rim is a solid line rather
    /// than a dotted one. Both are asserted below, which is what keeps the
    /// exemption from becoming the next version of this bug.
    #[test]
    fn the_ground_is_never_drawn_as_a_space() {
        assert!(
            !DEFAULT_GLYPHS.contains(' '),
            "the default ground ramp opens with a space, so the top row of the \
             ground is indistinguishable from the sky"
        );

        // And on the real frame, not just in the constant. The near ridge is the
        // one that has to reach the bottom of the screen, so the band between the
        // two surfaces is checked as well -- a far ridge with a hole in it would
        // be a hole in the layer behind the one you are looking at.
        let size = (80u16, 24u16);
        let frame = Frame::new(size, 0);
        for (x, (&near, &far)) in frame.near.iter().zip(&frame.far).enumerate() {
            for y in far.min(near)..near {
                assert_ne!(
                    frame.glyph(x, y),
                    ' ',
                    "cell ({x}, {y}) is inside the far ridge, between its surface on \
                     {far} and the near one on {near}, but drew as a space, so the \
                     distant layer has a hole in it"
                );
            }
            for y in near..size.1 as usize {
                assert_ne!(
                    frame.glyph(x, y),
                    ' ',
                    "cell ({x}, {y}) is below the surface at row {near} but drew \
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
    ///
    /// Asserted for both ridges, and the far one is the interesting half: a
    /// `parallax` of zero would leave the distant layer standing still while the
    /// near one slid past it, which is not a landscape at any rate and is also
    /// the failure that a test watching only the near ridge would report as a
    /// pass.
    #[test]
    fn the_scroll_moves_the_silhouette_and_not_only_the_shading() {
        let mut terrain = Terrain::new(TerrainOptions::default(), (80, 24));
        let near_first = terrain.near_surface_rows(terrain.offset);
        let far_first = terrain.far_surface_rows(terrain.far_offset());

        for _ in 0..120 {
            terrain.advance(1.0 / 60.0);
        }
        let near_later = terrain.near_surface_rows(terrain.offset);
        let far_later = terrain.far_surface_rows(terrain.far_offset());

        for (label, (first, later)) in [
            ("near", (&near_first, &near_later)),
            ("far", (&far_first, &far_later)),
        ] {
            let moved = first.iter().zip(later).filter(|(a, b)| a != b).count();
            assert!(
                moved > 0,
                "after two seconds at the default scroll speed not one column's \
                 {label} surface had moved, so that layer is standing still"
            );
        }
    }

    /// The default has to be visible motion without being a conveyor belt.
    ///
    /// Reinterpreted for the height field, since `scroll_speed` now measures
    /// how fast the landscape passes the camera rather than how fast a sampled
    /// field is offset vertically. The band is expressed as *seconds per noise
    /// period*, which is the quantity a viewer actually perceives: it is how
    /// long one ridge takes to cross the screen.
    ///
    /// The far ridge is held to the same band, and that is where the 0.35
    /// default comes from: at `parallax` 0.35 the distant ridge takes 25.4
    /// seconds to cross against the near one's 8.9, so both are inside it. Halve
    /// the parallax again and the far ridge takes 51 seconds to move a screen's
    /// width, which is not a landscape scrolling slowly, it is a still frame
    /// with the near one moving in front of it.
    #[test]
    fn the_scroll_is_fast_enough_to_see_without_being_a_conveyor_belt() {
        let options = TerrainOptions::default();
        let period = TerrainOptions::default().scale;
        let seconds_per_period = period / options.scroll_speed;
        let far_seconds = seconds_per_period / options.parallax;

        assert!(
            (2.0..30.0).contains(&seconds_per_period),
            "at scroll_speed {} one {period} cell ridge takes {seconds_per_period:.1}s \
             to cross the screen, which is outside the 2 to 30 second band: under \
             2s the ground stops reading as distance, over 30s it is not motion",
            options.scroll_speed
        );
        assert!(
            (2.0..30.0).contains(&far_seconds),
            "at parallax {} the far ridge takes {far_seconds:.1}s to cross the \
             screen, which is outside the same 2 to 30 second band the near ridge \
             is held to",
            options.parallax
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
            let rows = Terrain::new(options, (80, 24)).near_surface_rows(0.0);
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
    ///
    /// The far ridge is checked as well, and on a terminal too short to hold all
    /// three bands the two are allowed to collapse into each other rather than to
    /// be squeezed off the screen: a two-row terminal cannot show a sky, a
    /// distant ridge and a near one, and the near one is the one worth keeping.
    /// `far_ridge_row` is where that decision is made, and this is the test that
    /// says it was made on purpose.
    #[test]
    fn the_horizon_leaves_room_for_sky_and_ground_at_every_size() {
        for (width, height) in [(6u16, 6u16), (8, 200), (200, 8), (1, 1), (1, 9)] {
            let horizon = Terrain::horizon_row((width, height));
            let far = Terrain::far_ridge_row((width, height));
            assert!(
                horizon < height as usize,
                "at {width}x{height} the horizon is row {horizon}, so there is no \
                 ground at all"
            );
            assert!(
                far <= horizon,
                "at {width}x{height} the far ridge is row {far} and the near one \
                 {horizon}, so the dim layer would be drawn in front of the bright one"
            );

            let frame = Frame::new((width, height), 0);
            let has_sky = frame.glyphs.iter().flatten().any(|&c| c == ' ');
            let has_ground = frame.glyphs.iter().flatten().any(|&c| c != ' ');

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
    /// be in that list or it arrives as a key the user's file does not have --
    /// and a key that arrives as its default is a key that never reaches anyone
    /// the moment the default moves, which is how this project ended up with a
    /// donut rotating sixty times too slowly for a release.
    ///
    /// The two new keys are asserted by *value* as well as by presence. A
    /// presence check alone is satisfied by a field that serde happens to
    /// serialise and that the renderer never reads, and that is exactly the
    /// failure a new option is most likely to have.
    #[test]
    fn the_new_keys_round_trip_through_toml() {
        let options: TerrainOptions = toml::from_str(
            "scroll_speed = 2.5\nrelief = 0.3\nfar_relief = 0.9\nparallax = 0.2\nglyphs = \" .oO@\"\n",
        )
        .expect("five keys parse");
        assert_eq!(options.scroll_speed, 2.5);
        assert_eq!(options.relief, 0.3);
        assert_eq!(options.far_relief, 0.9);
        assert_eq!(options.parallax, 0.2);
        assert_eq!(options.glyphs, " .oO@");
        assert_eq!(
            options.scale,
            TerrainOptions::default().scale,
            "five keys in the section silently reset the others"
        );

        // And the defaults, which is what --print-config writes.
        let defaults = TerrainOptions::default();
        assert_eq!(defaults.far_relief, DEFAULT_FAR_RELIEF);
        assert_eq!(defaults.parallax, DEFAULT_PARALLAX);
        assert!(
            defaults.parallax < 1.0,
            "the default parallax is {} , so the far ridge scrolls at the same rate \
             as the near one and there is no parallax at all",
            defaults.parallax
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        for key in [
            "seed",
            "scale",
            "octaves",
            "persistence",
            "scroll_speed",
            "relief",
            "far_relief",
            "parallax",
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

    /// The surfaces must be cheap.
    ///
    /// They are the only part of the frame that costs a noise sample, and there
    /// are `height` times more cells than columns. Sampling per cell instead --
    /// which is what the old renderer did, and what the grain it grew needed --
    /// is 80,000 samples a frame at 400x200 against 800 for both ridges.
    ///
    /// This asserts the *array lengths* rather than the sample count, and that
    /// is worth being honest about: a per-column sample is a structural property
    /// of the renderer and the only thing a test can see is that the arrays the
    /// fill loop walks are one long rather than one long per row. A per-cell
    /// sample would have to make one of them `width * height`.
    ///
    /// The second half is the one that catches a real regression: a frame that
    /// drew *fewer* cells would satisfy the first half too, and the canvas is
    /// blanked at the start of every frame, so skipping cells it believes are
    /// unchanged would leave holes rather than save work.
    #[test]
    fn the_surfaces_cost_one_noise_sample_per_column_per_ridge_not_per_cell() {
        let size = (400u16, 200u16);
        let terrain = Terrain::new(TerrainOptions::default(), size);

        assert_eq!(
            terrain.near_surface_rows(0.0).len(),
            usize::from(size.0),
            "the near surface should be computed once per column"
        );
        assert_eq!(
            terrain.far_surface_rows(0.0).len(),
            usize::from(size.0),
            "the far surface should be computed once per column"
        );

        // And the frame still draws every cell, so the cheap surfaces did not
        // come from drawing less.
        let mut painted = Terrain::new(TerrainOptions::default(), size);
        let drawn_cells = painted.get_diff().len();
        assert_eq!(
            drawn_cells,
            usize::from(size.0) * usize::from(size.1),
            "at 400x200 the frame should touch every cell, got {drawn_cells}"
        );
    }

    /// A silhouette is cheap to emit, and the two ridges together still are.
    ///
    /// The deleted grain cost more than it bought, and this is the measurement
    /// that says so. A cell only reaches the wire when it *changes*, and a cell
    /// that is a flat fill below a moving surface does not change until the
    /// surface reaches it: measured over 300 frames at 400x200 the previous
    /// version changed 1,161 cells a frame and this one changes fewer than a
    /// third of that, because a body whose glyph was a two-dimensional sample
    /// rewrote most of the ground every frame while a fill does not.
    ///
    /// A byte count is only meaningful next to what is on screen, and this one
    /// is on screen being *less* structured -- so the assertion is deliberately
    /// loose. It is not a performance budget; it is a tripwire on the *shape* of
    /// the frame. A renderer that went back to a per-cell field would put this
    /// straight back over 3,000, and one that drew a third ridge would put it
    /// over too, which is the other thing worth knowing.
    #[test]
    fn a_frame_of_two_silhouettes_changes_few_cells() {
        let mean = mean_frame_diff((400, 200), 300);
        assert!(
            mean < 900.0,
            "the frame changed {mean:.0} cells on average at 400x200, so something \
             is being recomputed per cell rather than per column -- the removed \
             grain did this at 1,161"
        );
    }
}
