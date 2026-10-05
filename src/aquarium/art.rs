//! The fish, as braille dot bitmaps.
//!
//! # Why this file exists at all
//!
//! The first version of this effect drew its fish from hand-written character
//! sprites, and every one of them was the same object: a run of `(` with an `o`
//! and a `><` on the ends, in three lengths. Printed, the tank was a field of
//! horizontal bars.
//!
//! Two things were wrong with that, and only the second one was about the art.
//!
//! **A character sprite is squashed before it is drawn.** A cell is one unit wide
//! and about two tall, so a 13x3 sprite is 13 units wide by 6 tall -- a 2:1
//! streak, not a fish. The old art compensated by making sprites *long and thin*,
//! which is why every species was a bar, and no amount of retuning fixed it.
//!
//! **A string cannot carry a gradient, so a fish is a silhouette or nothing.**
//! The old sprites were one flat colour, which is correct for a logo and wrong
//! for an animal: what makes a fish read as a rounded, lit body is that its back
//! catches the light and its belly is in shadow.
//!
//! Braille fixes the first and half-fixes the second. A cell holds 2 dots across
//! and 4 down, so each dot is 0.5 by 0.5 units -- **square**. Art authored in dot
//! space comes out correctly proportioned on any terminal, and the aspect caveat
//! at the top of [`crate::render`] stops applying to the fish. The second is
//! answered by giving each *cell* a colour sampled from the fish's own shading
//! ramp, which is what [`FishArt::blit`] does: the dots carry the outline and the
//! detail, and the cell colour carries the light.
//!
//! # The fish is a function, not a drawing
//!
//! Each species is a body *shape* -- four numbers describing a skewed sine bell
//! -- plus fins, a tail, and markings. Nothing here is a picture of a fish.
//!
//! That is a deliberate choice over hand-drawn dot art, for one reason that
//! matters more than it looks: **five species built from five different shape
//! parameters cannot accidentally come out as the same bar five times.** The
//! string version could, and did, and nothing in the test suite noticed, because
//! every assertion about the art was about *one* sprite at a time. There is now a
//! test that compares species to each other -- see
//! `the_species_are_not_the_same_fish_at_different_lengths`.
//!
//! `body_shape` is zero at both ends by construction, so a species cannot be a
//! rectangle. The first version of this file let the profile be a hand table, and
//! a table with a jump in it is exactly how the head became a vertical wall.
//!
//! # Tone, and why there is no dither here at all
//!
//! Every dot in a fish is **raised**, and the outline is simply the set of dots
//! inside the shape. That took two attempts to learn.
//!
//! The first faded the ink linearly from the spine to the outline, on the theory
//! that a soft edge looks drawn. At dot resolution it does not: it turned the
//! outer *half* of every fish into a 50% Bayer checkerboard, and a checkerboard
//! is not a soft edge, it is a halftone screen laid over the animal.
//!
//! The second kept the body solid and made the *fins* translucent at 78%, which
//! has the same failure one step along: a 4x4 Bayer matrix repeated across a
//! ten-by-five dot region is a visible texture, not a tone. Dithering produces an
//! intermediate value between two dots; a region of ten dots is not intermediate,
//! it is a pattern.
//!
//! So the fish's light is not in the dots at all. It is the per-cell colour in
//! [`FishArt::cells`], resampled from the per-dot shade, and that is free: a
//! braille cell is one glyph in one colour, so the shading is a property of the
//! cell and the silhouette is a property of the dot. A discus's dorsal fin is a
//! pale solid sail above a gold body, and it reads as a fin because of the *tone*
//! difference, not because you can see the water through it.
//!
//! # Coordinates
//!
//! Three, and confusing them is the usual bug:
//!
//! - **Dots** run `0..length` by `0..height`, and are what [`rasterise`] writes.
//! - **Cells** run `0..ceil(length/2)` by `0..ceil(height/4)`, and are what
//!   [`Canvas::set`] takes.
//! - A fish's position is a float in *cells*, and the dot origin is derived from
//!   it, so a fish moves in quarter-row steps rather than whole cells.
//!
//! [`FishArt::cells`] is the only place the first two meet.

use crate::aquarium::charart::{self, CharArt, CharSpecies};
use crate::render::braille::{BrailleGrid, DOTS_PER_CELL, DOTS_X, DOTS_Y};
use crate::render::palette;
use crossterm::style::Color;

/// Poses per **braille** species, and the length of their tail cycle.
///
/// **Four**, and the thing that set it is the shape of what the poses sample.
///
/// Two was a pendulum argument -- a tail has two extremes, and a third would only
/// repeat the first -- and it is true of a *pendulum* and false of a *wave*. Two
/// poses are two samples of a sine a quarter-period apart, which is a snap between
/// two extremes with nothing in between, and a fish that snaps reads as flickering
/// rather than as swimming. Four sample it at the quarter points, which is the
/// shortest cycle that reads as a sweep.
///
/// **The three you cannot see.** Because `phase` is `pose / POSES`, poses `n` and
/// `n + POSES` are the *same picture*: `sin(TAU * (1 - 0.7t)) == sin(-TAU * 0.7t)`,
/// because the two arguments differ by exactly `TAU`. So a four-pose table has no
/// duplicates, and the first version of this measurement reported **zero dots
/// differing between poses 0 and 2** and nearly forty between 0 and 1 -- which
/// looked like a wave that only worked on alternate frames, and was really a
/// `POSES` edit that had not applied. The count itself is correct and the
/// conclusion drawn from it was wrong; see the note at [`WAVE_AMPLITUDE`].
///
/// The character species have [`charart::POSES`] = 3, and they **do not play any of
/// them** -- their cycle is 1. See [`Source::cycle`], which is where that decision
/// lives; this paragraph is here because the two numbers differing is the sort of
/// thing a reader finds and assumes is a mistake. It is not: three frames is the
/// smallest number that reads as a *beat* rather than as a twitch, and it was drawn
/// and tested as one, and then the beat turned out to be what was wrong with it.
/// The art is kept and the animation is not.
pub const POSES: usize = 4;

/// Where a species' art comes from, and so which medium draws it.
///
/// The tank is not one medium. The split follows **depth**, because depth is what
/// decides it: a near fish is drawn in characters, because you can see its face
/// and a face needs strokes, and a small or far one in braille, because at that
/// size only the silhouette is legible and braille's density is worth more there
/// than its fill is worth less. That makes the split a **depth cue that was
/// free** -- a near fish drawn in `/ \ | _ -` carries more ink and more internal
/// detail than a far fish drawn as sparse dots, which is true of real water.
///
/// A function rather than a third static, because a static list would have to name
/// each entry twice and that is a second place the species list is written down.
/// Two tables stay two tables, and `sources()` is the one place that unions them.
pub fn sources() -> impl Iterator<Item = Source<'static>> + Clone {
    SPECIES
        .iter()
        .map(Source::Dots)
        .chain(CHAR_SPECIES.iter().map(Source::Chars))
}

/// A species' art, tagged with the medium that draws it.
#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    /// A procedural braille shape. See [`Species`].
    Dots(&'a Species),
    /// A hand-drawn character sprite. See [`CharSpecies`] and
    /// [`crate::aquarium::charart`].
    Chars(&'a CharSpecies),
}

impl Source<'_> {
    /// For test failure messages and for the species table's own diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            Source::Dots(s) => s.name,
            Source::Chars(s) => s.name,
        }
    }

    /// The preferred depth band, `0.0` near to `1.0` far. A bias, not a lock.
    pub fn depth(&self) -> f32 {
        match self {
            Source::Dots(s) => s.depth,
            Source::Chars(s) => s.depth,
        }
    }

    /// How much colour survives at depth.
    pub fn chroma(&self) -> f32 {
        match self {
            Source::Dots(s) => s.chroma,
            Source::Chars(s) => s.chroma,
        }
    }

    /// Base colour at the spine, at `depth == 0.0`.
    pub fn color(&self) -> Color {
        match self {
            Source::Dots(s) => s.color,
            Source::Chars(s) => s.color,
        }
    }

    /// Whether this species is drawn in characters.
    pub fn is_chars(&self) -> bool {
        matches!(self, Source::Chars(_))
    }

    /// Frames in this species' **tail cycle**, and the two numbers are not the same
    /// thing.
    ///
    /// **One for the character species.** They do not animate. The hand-drawn beat
    /// still exists in [`CHAR_SPECIES`] -- three frames a species, drawn by hand --
    /// and it is deliberately not played, because a three-frame cycle of *line art*
    /// at four or five beats a second is about fourteen frames a second of strobing
    /// marks on a fish's tail. The braille pair at the same rate read as a swim
    /// because a dot is a quarter of a cell and the eye integrates it; a hand-drawn
    /// tail is a handful of characters and there is nothing to integrate. **The
    /// medium decides this one too**, and in the opposite direction from the last
    /// time: more density is what makes motion legible, and the sparser medium
    /// cannot carry motion at all.
    ///
    /// So `charart::POSES` (three, the frames that exist in the art) and this (one,
    /// the frames the effect plays) have deliberately diverged, and they are named
    /// differently because conflating them is exactly the confusion that would
    /// produce a `poses` of 3 here.
    ///
    /// **Four for the braille pair**: two is a pendulum and reads as a flicker, four
    /// samples the travelling wave at the quarter points. See [`POSES`].
    pub fn cycle(&self) -> usize {
        match self {
            Source::Dots(_) => POSES,
            Source::Chars(_) => 1,
        }
    }

    /// How fast this species cruises, as a multiple of the configured speed.
    pub fn cruise(&self) -> f32 {
        match self {
            Source::Dots(s) => s.cruise,
            Source::Chars(s) => s.cruise,
        }
    }

    /// The sprite, for the character species. `None` for the braille ones, which
    /// are rasterised per phase rather than held whole.
    pub fn char_art(&self) -> Option<CharArt> {
        match self {
            Source::Dots(_) => None,
            Source::Chars(s) => Some(charart::species(s)),
        }
    }
}

/// A species' art, ready to blit, in whichever medium it was authored in.
///
/// One interface over two very different things, which is the point: the effect
/// has to be able to treat a hand-drawn 16x7 character fish and a procedural
/// 30x25 braille fish the same way, because they are two thirds of one tank.
///
/// The two are *not* stored the same way and that asymmetry is deliberate. A
/// braille sprite is materialised once per `(facing, pose, phase)` because
/// shifting dots is cheap but a fish is redrawn every frame. A character sprite
/// holds **all** of its frames and both facings in one value and is indexed by
/// pose at blit time, because a character cell *is* the resolution -- there is
/// nothing to phase -- and because copying 130 characters once a frame to save a
/// field is a bad trade.
pub enum Sprite {
    /// One pre-phased braille bitmap. Yields cell offsets, not dot offsets.
    Dots(Box<FishArt>),
    /// A character sprite at one facing, holding every frame of the tail cycle.
    Chars(Box<CharArt>),
}

impl Sprite {
    /// Width in terminal cells, padding included.
    pub fn cells_wide(&self) -> usize {
        match self {
            Sprite::Dots(a) => a.cells_wide(),
            Sprite::Chars(a) => a.cells_wide(),
        }
    }

    /// Height in terminal cells.
    pub fn cells_tall(&self) -> usize {
        match self {
            Sprite::Dots(a) => a.cells_tall(),
            Sprite::Chars(a) => a.cells_tall(),
        }
    }

    /// Whether this sprite is drawn **nose-left**, in its own coordinates.
    ///
    /// Exists because "which way does this face" is a question the crate asks in
    /// two places and neither can be answered by looking at the artwork:
    /// [`crate::aquarium::effect::Aquarium::art_index`] picks a sprite from a
    /// boolean, and the test that pins that choice has to check the sprite agrees
    /// with the boolean. Without this the ordering of the two build loops is a
    /// comment, and **it was a comment for a long time** -- the braille and
    /// character branches pushed their facings in opposite orders, so every
    /// braille fish in the tank swam backwards and nothing said so.
    ///
    /// # It is the eye, and it is allowed to decline
    ///
    /// "More ink near the nose than near the tail" happens to hold for all seven
    /// fish here and is still the wrong invariant, because it fails the moment a
    /// species grows a heavy peduncle or a long trailing dorsal. The eye is the one
    /// mark every fish has. **But an eye is only a facing signal if it is actually
    /// near one end**, and one of these is not: `fry` is nine columns wide and its
    /// eye is at column five, so "which end is the head" is not a question its art
    /// can answer -- and the first version of this function answered it anyway,
    /// confidently and wrongly, which is worse than declining.
    ///
    /// So this returns `None` when the eye is **not decisive**, rather than
    /// guessing from a threshold that happens to fall on the right side today. The
    /// thresholds are loose and the middle band is wide on purpose: measured eye
    /// positions are 0.00 (`slashback`), 0.08 and 0.10 (the braille pair and
    /// `stipple`), 0.16 (`longfin`), 0.19 (`bigeye`) and **0.56 (`fry`)**, so only
    /// `fry` lands in the band and it lands well inside it.
    ///
    /// # The denominator
    ///
    /// The sprite's own width, not the fish's length. The braille bitmap carries a
    /// dot of shift slack on each side for [`FishArt::shifted`] to spend, so the
    /// fish is not centred in it -- and `body_length()` would be the wrong
    /// denominator anyway, since a fish facing right has its body measured from the
    /// wrong end.
    pub fn faces_left(&self, pose: usize) -> Option<bool> {
        /// At or below this fraction along the sprite, the eye is decisively at the
        /// nose.
        const FRONT: f32 = 0.40;
        /// Or at or beyond this one, decisively at the tail.
        const BACK: f32 = 0.65;

        let (eye, width) = match self {
            Sprite::Dots(a) => {
                let (dx, _) = (0..a.height())
                    .flat_map(|dy| (0..a.width()).map(move |dx| (dx, dy)))
                    .find(|(dx, dy)| a.part_at(*dx, *dy) == Part::Eye)?;
                (dx as f32, a.width() as f32)
            }
            Sprite::Chars(a) => {
                let x = a
                    .cells(pose)
                    .find(|(_, _, _, shade)| *shade == charart::EYE)?
                    .0;
                (x as f32, a.cells_wide() as f32)
            }
        };
        let where_ = eye / width;
        if where_ <= FRONT {
            Some(true)
        } else if where_ >= BACK {
            Some(false)
        } else {
            None
        }
    }

    /// Every cell of one frame: `(x, y, glyph, shade)`, offsets in **cells**.
    ///
    /// A callback rather than a returned iterator because the two media return
    /// different iterator types and boxing one would allocate per fish per frame
    /// to unify them. Both iterators already yield cell offsets, which is the
    /// detail that is easy to get wrong: the braille one used to yield *dot*
    /// offsets and the effect added them to a position in cells, so every fish
    /// was drawn four columns and four rows from where it was.
    pub fn for_each_cell(
        &self,
        pose: usize,
        mut f: impl FnMut(i32, i32, char, f32),
    ) {
        match self {
            Sprite::Dots(a) => {
                for (x, y, ch, shade) in a.cells() {
                    f(x, y, ch, shade);
                }
            }
            Sprite::Chars(a) => {
                for (x, y, ch, shade) in a.cells(pose) {
                    f(x, y, ch, shade);
                }
            }
        }
    }
}

/// Amplitude of the travelling wave that is a fish's swim, as a multiple of its
/// girth, at the tail tip.
///
/// **0.8**, and the two numbers that picked it are a measurement rather than a
/// preference -- and then it was wrong, in the way a single-sided criterion is
/// always eventually wrong.
///
/// **0.35**, and the number that chose it is not "how many dots move". It is **how
/// far the tail tip travels compared with how deep the fish is.** The amplitude
/// that the *previous* version shipped, 0.8, moved 32 of a neon's dots between
/// opposite poses -- 39% of its ink, comfortably past the "ample to read as a beat"
/// bar -- and it was reported as the fish **jiggling**.
///
/// **The arithmetic that replaced it**, against each species' measured flesh depth
/// (5 dots on the neon, 9 on the tetra -- see
/// `the_tail_tip_does_not_travel_further_than_half_the_fish_is_deep` for how that is
/// measured and why the obvious denominator is wrong):
///
/// | amplitude | tip travel | of its own depth |
/// |---|---|---|
/// | 0.05 | 0.26 | 0.05 |
/// | **0.35** | **1.82** | **0.36** |
/// | 0.50 | 2.60 | 0.52 |
/// | 0.80 | 4.16 | 0.83 |
/// | 1.00 | 5.20 | 1.04 |
///
/// The two species agree to two decimal places at every amplitude, which is the
/// check that scaling the amplitude by `girth` is right: they are different-sized
/// animals and one number describes both.
///
/// **0.26 is the swing the design had before the wave replaced it** --
/// `0.30 * caudal.1 * girth` -- and 0.8 was **3.1x** that. So this was a
/// regression dressed as a tuning pass, and the metric that let it through counts
/// dots and has no opinion about whether the fish is swimming or shaking. At 0.8
/// the tip was crossing **83% of the fish's own depth, five times a second**.
///
/// **The lesson is the shape of the mistake, not the number.** A floor with no
/// ceiling is satisfied by every value up to the clipping limit, and "more dots
/// moving" reads as "more visible" right up until it reads as noise. Both ends now
/// have an assertion: `the_wave_moves_enough_dots_to_be_seen` for the floor and
/// `the_tail_tip_does_not_travel_further_than_half_the_fish_is_deep` for the ceiling,
/// and the ceiling is stated as geometry rather than as a fitted dot count, so it
/// cannot be satisfied by recalibrating the thing it measures.
///
/// It is also a *replacing* amplitude and not an addition, so nothing new has to
/// fit in the bitmap.
const WAVE_AMPLITUDE: f32 = 0.35;

/// Where along the body the wave starts, `0.0` at the snout and `1.0` at the tail.
///
/// **0.45**, so the front 45% is a rigid head and everything visible happens in
/// the rear 55%. Lowering it makes the wave longer and shallower and the front of
/// the fish starts to shimmer without gaining anything, which is the same failure
/// as the character shear in the sibling module: a motion finer than the medium
/// cannot show is a motion that costs bytes and buys nothing.
const WAVE_START: f32 = 0.45;

/// How many wavelengths of the wave fit along the body.
///
/// **0.7**, just under one. A full wavelength would put a node at the tail tip and
/// the caudal -- the one part of a fish whose motion is unmistakable -- would sit
/// still at one end of the cycle. Three quarters of a wavelength puts the tip near
/// its maximum excursion, which is where a tail spends its time.
const WAVE_LENGTH: f32 = 0.7;

/// How far a column of the body has moved, in multiples of `girth`, at fraction
/// `t` along the fish and wave `phase`.
///
/// Split out of [`rasterise`] so the *shape* of the swim can be swept in a test
/// rather than judged by eye in a still. That matters because the thing being
/// tuned is invisible at one number and obvious at another: the amplitude sets
/// both how many dots move between poses and whether the fish still reads as a
/// fish, and it is the pair of those two numbers that picks the constant, not
/// either one alone.
///
/// The wave is `sin` shaped in the phase and `rear` shaped along the body, so it
/// is zero at the snout, grows toward the tail tip, and completes one beat per
/// cycle.
fn wave_offset(t: f32, phase: f32) -> f32 {
    // `rear` is zero over the front 45% and one at the tail tip.
    //
    // **A dot is the quantum.** A column that moves by half a dot moves *no
    // dots*, so a wave with a gentle exponent is a wave you cannot see at all in
    // the front half of the fish -- the arithmetic is there and the raster is
    // unchanged. The amplitude is therefore concentrated into the rear, where
    // there are enough dots above and below to move. This is the same finding as
    // the character shear, in the other medium: **a motion finer than the medium
    // cannot show is a motion that costs bytes and buys nothing.**
    let rear = ((t - WAVE_START) / (1.0 - WAVE_START)).clamp(0.0, 1.0);
    rear * (std::f32::consts::TAU * (phase - WAVE_LENGTH * t)).sin()
}

/// How a fish's belly is shaped, relative to its back.
///
/// Three because a bottom-dweller with a scaled-down copy of a swimfish's belly
/// is a swimfish, and the one thing that makes a cory a cory is that it does not
/// have one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Belly {
    /// As deep as the back. Minnows, tetras.
    Symmetric,
    /// A keel: slightly shallower than the back, deepest under the shoulder.
    /// The shape that makes a fish *streamlined* rather than round.
    Keel,
    /// A flat floor at `level` times the girth, whatever the back is doing.
    /// Corys, and anything that sits on gravel.
    Flat { level: f32 },
}

/// How a fin is shaped.
///
/// A fin drawn as a smooth hump out of the body's profile is a swell in the body.
/// A real fin rises to a point or a sweep, and at dot resolution the difference
/// between a triangular dorsal and a rounded one is three or four dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinStyle {
    /// Full height only at the fin's own middle: a triangle. A cory's, a tetra's.
    Triangle,
    /// Rises behind its leading edge and trails off towards the tail: a sickle.
    /// A discus's, a minnow's.
    Sickle,
    /// Holds its height across the middle third. A neon's.
    Rounded,
}

/// The caudal fin's trailing edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TailFork {
    /// Convex: a lanceolate tail, rounded at the tip. A neon's.
    Rounded,
    /// A notch cut into the middle of the trailing edge, `depth` as a multiple of
    /// the girth. The forked tail of almost every fish that swims fast.
    Forked { depth: f32 },
}

/// What a species wears.
///
/// Stripes and spots, not scales. Scales at this resolution are noise, and a fish
/// whose distinguishing feature is its colour needs it drawn in *tone*, which is
/// what the per-cell colour ramp is for.
#[derive(Debug, Clone, Copy)]
pub enum Markings {
    /// Nothing but the shading ramp.
    None,
    /// A bright band along the flank, at `y` where `-1.0` is the belly and `1.0`
    /// is the back.
    ///
    /// The lateral line, and on a neon it is the whole animal.
    Lateral { y: f32, half: f32 },
    /// Vertical bars across the body.
    Bars {
        /// How many, across the `from`..`to` span.
        count: f32,
        from: f32,
        to: f32,
        /// How dark each bar is, `0.0` for none.
        strength: f32,
    },
    /// Irregular dark patches. Hash-based, so it is stable across frames.
    Mottled { amount: f32 },
}

/// A species: a body shape, some fins, and a colour.
///
/// Not configurable and not seeded. This is art, and art in a table is easier to
/// review than art in a formula. Every dimension the simulation needs is derived
/// from [`Species::length`] and [`Species::girth`], so no size is stated twice.
#[derive(Debug, Clone, Copy)]
pub struct Species {
    /// For test failure messages. Nothing in the effect reads it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub name: &'static str,

    /// Snout to tail tip, in dots.
    pub length: usize,

    /// Body half-height at its deepest, in dots.
    pub girth: f32,

    /// Where the deepest point sits, in `t`. Below `0.5` is a fish whose head
    /// carries its mass; above is one that slopes away behind.
    peak: f32,

    /// How fast the head fills out, as an exponent. `1.2` is a pointed snout,
    /// `2.4` is a blocky forehead, and it is most of what separates a tetra from
    /// a minnow at the same length.
    blunt: f32,

    /// How fast the body narrows behind the peak. Below `1.0` keeps it deep all
    /// the way back, which is what makes a discus circular; `2.5` is a spindle
    /// that pinches to a peduncle.
    taper: f32,

    /// The belly's shape.
    belly: Belly,

    /// The dorsal fin: `(t, height as a multiple of [`Species::girth`])`. Zero
    /// height means no fin there.
    ///
    /// **A multiple of the girth, not a number of dots.** Every dimension of a
    /// fish except its length is a fraction of its depth, and that is what makes a
    /// fish scalable: [`rasterise`] multiplies these by the girth, so a species
    /// drawn at twice the size is the same animal twice as big rather than the
    /// same animal with a different dorsal fin glued on. The tables were in dots
    /// until the tank needed a size scale for large terminals, and at that point
    /// there was no way to scale one without a second, differently-proportioned
    /// copy of every species table.
    pub dorsal: &'static [(f32, f32)],

    /// The anal fin, below the belly. Same shape as [`Species::dorsal`].
    pub anal: &'static [(f32, f32)],

    /// The caudal fin: `(t where it starts, half-height as a multiple of the
    /// girth, trailing edge)`.
    ///
    /// The body's own shape has already tapered to a peduncle by then; the caudal
    /// is the flare from that stalk out to the tip, and it is the only thing the
    /// two poses change.
    pub caudal: (f32, f32, TailFork),

    /// How the fins are shaped.
    pub fin_style: FinStyle,

    /// Barbels: whiskers under the snout. A cory has them and nothing else here
    /// does, and they are a good share of what makes it read as a bottom-dweller.
    pub barbels: bool,

    /// Stripes, a lateral line, or spots.
    pub markings: Markings,

    /// The preferred depth band, `0.0` near to `1.0` far.
    ///
    /// A bias and not a lock -- see the note on depth in the effect's module docs.
    pub depth: f32,

    /// How much colour survives at depth, `0.0`..=`1.0`.
    ///
    /// Per species, because two species that both read as orange are two fish and
    /// one fish is a fish.
    pub chroma: f32,

    /// Base colour at the spine, at `depth == 0.0`.
    ///
    /// The spine and not the back, because [`shade_colour`] runs its ramp out
    /// from this: this is the fish's *mid* tone and the extremes are derived.
    pub color: Color,

    /// How fast this species cruises, as a multiple of the configured speed.
    ///
    /// Per species rather than global because a tank is not one animal's tank: a
    /// neon is a dart and a cory is not.
    pub cruise: f32,
}

/// The species table.
///
/// The shape numbers are calibrated against the dot dump from
/// `examples/aquarium_picture.rs --art` rather than against a text editor, which
/// is the whole reason the first version of this file drew bars: in a text editor
/// a row is one unit tall and on a terminal it is two.
pub static SPECIES: &[Species] = &[
    Species {
        name: "neon",
        // 22x~13 dots, 11x4 cells. Small enough to shoal tightly, and *small* on
        // purpose: a tank whose smallest fish is 11 cells wide has no sense of
        // scale in it, and scale is much of what makes a tank look inhabited.
        length: 22,
        girth: 2.6,
        peak: 0.38,
        blunt: 1.45,
        taper: 1.55,
        belly: Belly::Keel,
        dorsal: &[(0.26, 0.0), (0.46, 0.65), (0.60, 0.0)],
        anal: &[(0.56, 0.0), (0.67, 0.85), (0.78, 0.0)],
        caudal: (0.80, 0.85, TailFork::Rounded),
        fin_style: FinStyle::Rounded,
        barbels: false,
        // The lateral line is the animal. A neon without it is a small silver fish.
        markings: Markings::Lateral { y: -0.1, half: 0.5 },
        // 0.60, and it was 0.55: with the character species arriving at 0.05 the
        // tank's whole depth span came to 0.50 exactly, and
        // `the_species_occupy_separate_depth_bands_with_room_to_spread` wants more
        // than half. A span of exactly half the range is a tank with a top and a
        // bottom and very little in between.
        depth: 0.60,
        chroma: 0.9,
        color: Color::Rgb {
            r: 150,
            g: 235,
            b: 240,
        },
        // 0.70, and it was **1.00** -- the fastest cruise in the tank, on the
        // species that carries 30% of the count. Combined with the tetra's 0.90
        // that put **52% of the tank at its top two speeds**, and because the tail
        // beat is `cruise * speed * TAIL_RATE` it also put the beat at **5.40 Hz**.
        //
        // One number moved both complaints, which is the useful part: the braille
        // pair are now at 4.20 and 3.90 cells/s, beating at 3.78 and 3.51 Hz --
        // in line with `stipple` and below `longfin`, rather than faster than
        // everything. **A species' speed is also its tail rate**, so a cruise that
        // is too high cannot be fixed in `TAIL_RATE` without slowing every other
        // species with it.
        cruise: 0.70,
    },
    Species {
        name: "tetra",
        // 30x~19 dots, 15x5 cells. Deep-bodied and round-headed, which is a
        // different silhouette from the minnow's spindle and is most of what
        // separates the two at a glance.
        length: 28,
        girth: 4.6,
        peak: 0.34,
        blunt: 1.15,
        taper: 1.45,
        belly: Belly::Symmetric,
        // A tall triangular dorsal, set well forward. It is the tetra's outline.
        dorsal: &[(0.28, 0.0), (0.40, 0.57), (0.54, 0.0)],
        anal: &[(0.56, 0.0), (0.68, 0.41), (0.80, 0.0)],
        caudal: (0.82, 0.89, TailFork::Forked { depth: 0.39 }),
        fin_style: FinStyle::Triangle,
        barbels: false,
        // The eye bar. A tetra is one dark vertical stripe through the head and
        // a great deal of colour behind it.
        markings: Markings::Bars {
            count: 1.0,
            from: 0.05,
            to: 0.20,
            strength: 0.85,
        },
        depth: 0.45,
        chroma: 0.85,
        color: Color::Rgb {
            r: 240,
            g: 150,
            b: 60,
        },
        // 0.65, and it was 0.90 -- see the neon's `cruise`. These two carry
        // 52% of the tank between them, so their speeds are the tank's
        // character far more than any single fish's is.
        cruise: 0.65,
    },
];

/// The species drawn in **characters** rather than dots, in the idiom of the ASCII
/// art archives. See [`crate::aquarium::charart`] for why the medium is chosen by
/// depth rather than by taste, and for the one rule the two media share: the
/// snout is at `x = 0` in both, so one facing rule and one `mirrored()` serve the
/// whole crate.
///
/// A [`CharSpecies`] is a *drawing* where a [`Species`] is a *shape*, and the two
/// tables are separate for that reason rather than for tidiness. Five numbers
/// describe a profile; none of them describes an eye.
/// The species drawn in **characters** rather than dots, in the idiom of the ASCII
/// art archives. See [`crate::aquarium::charart`] for why the medium is chosen by
/// depth rather than by taste, and for the one rule the two media share: the
/// snout is at `x = 0` in both, so one facing rule and one `mirrored()` serve the
/// whole crate.
///
/// A [`CharSpecies`] is a *drawing* where a [`Species`] is a *shape*, and the two
/// tables are separate for that reason rather than for tidiness. Five numbers
/// describe a profile; none of them describes an eye.
///
/// # These five are one set, and they are all the same shape
///
/// They come from a single gallery artist and they are **five spindles**:
/// 1.58, 1.75, 1.90, 1.50 and 1.67 to one, once the cell's 2:1 height is
/// accounted for. An earlier set of mine held a round fish and a tall one on
/// purpose, so the tank had shape variety, and this one does not have either. That
/// is a fact about the drawing rather than a defect in it, and it is why
/// `no_two_species_are_the_same_animal` now resamples against a **common** box
/// instead of each species' own: normalising by the ink extent throws the aspect
/// away, and on a set this uniform the aspect is most of what is left to tell
/// them apart.
pub static CHAR_SPECIES: &[CharSpecies] = &[
    CharSpecies {
        name: "longfin",
        depth: 0.08,
        chroma: 1.0,
        color: Color::Rgb {
            r: 250,
            g: 214,
            b: 120,
        },
        cruise: 0.9,
        // The most elongated of the five, and the one whose tail is a pair of `)`
        // with a `{` between them, so the beat is that whole fork rising and falling.
        //
        // 19 columns by 6 rows of ink, so 1.58:1 once the cell's 2:1
        // height is accounted for.
        frames: [
            &[
                r"       /`-._       ",
                r"     _/,.._/       ",
                r"  ,-'   ,  `-:,.-')",
                r" : o ):';     _  { ",
                r"  `-.  `' _,.-\`.\)",
                r"     `\\``\,-'     ",
            ],
            &[
                r"       /`-._       ",
                r"     _/,.._/     ) ",
                r"  ,-'   ,  `-:,.-'{",
                r" : o ):';     _  ) ",
                r"  `-.  `' _,.-\`.\|",
                r"     `\\``\,-'   | ",
            ],
            &[
                r"       /`-._       ",
                r"     _/,.._/       ",
                r"  ,-'   ,  `-:,.-' ",
                r" : o ):';     _  { ",
                r"  `-.  `' _,.-\`.`{",
                r"     `\\``\,-'   ) ",
            ],
        ],
    },
    CharSpecies {
        name: "bigeye",
        depth: 0.14,
        chroma: 0.95,
        color: Color::Rgb {
            r: 228,
            g: 234,
            b: 245,
        },
        cruise: 0.85,
        // The `O` is the largest eye in the tank and the reason this one reads as
        // looking back at you. The tail is `/`, `(` and a doubled backslash.
        //
        // 21 columns by 6 rows of ink, so 1.75:1 once the cell's 2:1
        // height is accounted for.
        frames: [
            &[
                r"      /\             ",
                r"      _/./           ",
                r"   ,-'    `-:.,-'/   ",
                r"  > O )<)    _  (    ",
                r"   `-._  _.:' `-.\\  ",
                r"       `` \;         ",
            ],
            &[
                r"      /\             ",
                r"      _/./       /   ",
                r"   ,-'    `-:.,-'(   ",
                r"  > O )<)    _   \\  ",
                r"   `-._  _.:' `-.    ",
                r"       `` \;         ",
            ],
            &[
                r"      /\             ",
                r"      _/./           ",
                r"   ,-'    `-:.,-'    ",
                r"  > O )<)    _  /    ",
                r"   `-._  _.:' `-.(   ",
                r"       `` \;      \\ ",
            ],
        ],
    },
    CharSpecies {
        name: "slashback",
        depth: 0.20,
        chroma: 0.9,
        color: Color::Rgb {
            r: 150,
            g: 220,
            b: 160,
        },
        cruise: 0.8,
        // A pair of doubled slashes over an underscored flank, and the tail is the
        // `=` hanging off the end of it. There is the most room here for the beat to
        // read, and it is the clearest of the five.
        //
        // 19 columns by 5 rows of ink, so 1.90:1 once the cell's 2:1
        // height is accounted for.
        frames: [
            &[
                r"O  o               ",
                r"          _\_   o  ",
                r">('>   \\  o\ .    ",
                r"       //\___=     ",
                r"          ''       ",
            ],
            &[
                r"O  o            o  ",
                r"          _\_  .   ",
                r">('>   \\  o\ =    ",
                r"       //\___      ",
                r"          ''       ",
            ],
            &[
                r"O  o               ",
                r"          _\_      ",
                r">('>   \\  o\  o   ",
                r"       //\___ .    ",
                r"          ''  =    ",
            ],
        ],
    },
    CharSpecies {
        name: "fry",
        depth: 0.26,
        chroma: 0.5,
        color: Color::Rgb {
            r: 240,
            g: 210,
            b: 150,
        },
        cruise: 0.35,
        // Nine columns by three. Too small for a body wave, so its beat is the tail
        // marks alone and it is the subtlest of the five on purpose: a fish this
        // small darts, it does not swim.
        //
        // 9 columns by 3 rows of ink, so 1.50:1 once the cell's 2:1
        // height is accounted for.
        frames: [
            &[r"|\.-*-.  ", r"|( ( 0 ) ", r"|/`*-*`  "],
            &[r"|\.-*-/  ", r"|( ( 0   ", r"|/`*-*` )"],
            &[r"|\.-*-.  ", r"|( ( 0 ) ", r"|/`*-*-) "],
        ],
    },
    CharSpecies {
        name: "stipple",
        depth: 0.32,
        chroma: 1.0,
        color: Color::Rgb {
            r: 200,
            g: 170,
            b: 235,
        },
        cruise: 0.7,
        // Built from `·`, `¸` and an acute, which are East-Asian **ambiguous** width
        // and two cells wide in a CJK-locale terminal. Fine here, because
        // `render::is_ambiguous_or_narrow` accepts them and `presets::DOTS` already
        // ships a `·` -- but the texture *is* the stipple, so if you ever transliterate
        // it, do that on purpose. The `©` eye is part of the reference drawing and is
        // kept verbatim.
        //
        // 20 columns by 6 rows of ink, so 1.67:1 once the cell's 2:1
        // height is accounted for.
        frames: [
            &[
                r"      /`·.¸         ",
                r"     /¸...¸`:·      ",
                r" ¸.·´  ¸   `·.¸.·´) ",
                r": © ):´;      ¸  {  ",
                r" `·.¸ `·  ¸.·´\`·¸) ",
                r"     `\\´´\¸.·´     ",
            ],
            &[
                r"      /`·.¸         ",
                r"     /¸...¸`:·      ",
                r" ¸.·´  ¸   `·.¸.·´{ ",
                r": © ):´;      ¸  )  ",
                r" `·.¸ `·  ¸.·´\`·¸| ",
                r"     `\\´´\¸.·´   | ",
            ],
            &[
                r"      /`·.¸         ",
                r"     /¸...¸`:·      ",
                r" ¸.·´  ¸   `·.¸.·´  ",
                r": © ):´;      ¸  {  ",
                r" `·.¸ `·  ¸.·´\`·¸`{",
                r"     `\\´´\¸.·´   ) ",
            ],
        ],
    },
];

/// The body outline at `t`, as a fraction of [`Species::girth`].
///
/// **Two arcs, not one bell.** The head is a quarter sine rising to the deepest
/// point and the tail is a quarter cosine falling away from it, and they meet
/// where both derivatives are zero, so the join is smooth.
///
/// The first version used a single skewed sine, on the reasonable grounds that a
/// fish is fusiform. It is not: a sine is *flat at both ends*, so a fish built on
/// one has no head and no peduncle. A neon came out 39% of full depth a third of
/// the way along with a one-dot spike in front of it and a wall behind, and its
/// eye landed 40% from the snout, on its shoulder. A fish whose head is a spike is
/// not a fish with a pointy nose, it is a fish with no head.
///
/// The three numbers are each one decision:
///
/// - [`Species::peak`] -- where the deepest point sits. A fish whose mass is
///   forward has a big head; one that slopes away behind is a different animal.
/// - [`Species::blunt`] -- how fast the head fills out. Low is a pointed snout,
///   high is a blocky forehead, and it is most of what separates a tetra from a
///   minnow at the same length.
/// - [`Species::taper`] -- how fast it narrows behind the peak. Low keeps it fat
///   all the way back, which is what makes a discus circular; high is a spindle
///   that pinches to nothing.
///
/// Zero at both ends by construction, which is the point of a formula over a
/// table of numbers: a hand table can be a rectangle, and this one was.
fn body_shape(t: f32, peak: f32, blunt: f32, taper: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let peak = peak.clamp(0.05, 0.95);
    if t <= 0.0 || t >= 1.0 {
        return 0.0;
    }
    if t < peak {
        // A quarter sine, so the nose leaves zero at an angle rather than
        // crawling off horizontally, and `blunt` bends it.
        let u = (t / peak).clamp(0.0, 1.0);
        (std::f32::consts::FRAC_PI_2 * u)
            .sin()
            .powf(1.0 / blunt.max(0.2))
    } else {
        // A quarter cosine down to the peduncle: one at the peak, zero at the
        // tail tip. `taper` below 1 keeps the body deep all the way back, which
        // is the whole difference between a discus and a minnow.
        let v = ((t - peak) / (1.0 - peak)).clamp(0.0, 1.0);
        (std::f32::consts::FRAC_PI_2 * v).cos().powf(taper.max(0.2))
    }
}

/// What a dot *is*, as opposed to how bright it is.
///
/// Stored rather than inferred from the shade, which is the point. Three separate
/// tests had to guess whether a dot was an eye or an anal fin by comparing
/// `0.03` against a threshold, and the anal fin's tone is `0.0`, so every anal dot
/// in the crate read as "beside the eye" and was excluded from the count -- which
/// is how a species with a perfectly good anal fin was reported as having none.
///
/// The crate's rule is to store the state that is known rather than re-derive it
/// from a measurement. `rasterise` knows which branch of the shape test it is in;
/// that knowledge was being thrown away and then guessed back from a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Body.
    Flesh,
    /// The fin above the back.
    Dorsal,
    /// The fin below the belly.
    Anal,
    /// The pupil. Darker than anything else on the fish.
    Eye,
    /// One dot of light on the pupil.
    Catchlight,
    /// A whisker under a bottom-dweller's chin.
    Barbel,
    /// The caudal fin, which is neither above nor below the body.
    Tail,
    /// A lateral line: a marking painted onto the body, not the body's own
    /// shading. Its own part so that the tone tests do not read a marking as
    /// flesh -- a neon's flank stripe is the brightest thing on the animal, which
    /// is correct, and it is not a fish lit from underneath.
    Stripe,
    /// A vertical bar. Darker than any shading, and for the same reason: a tetra's
    /// eye-bar is 0.04 of the ramp and would otherwise read as a fish whose belly
    /// reaches the bottom of it.
    Bar,
}

impl Part {
    /// A single-character tag, for a dump that shows what each dot is.
    pub fn tag(self) -> char {
        match self {
            Part::Flesh => '.',
            Part::Dorsal => 'D',
            Part::Anal => 'A',
            Part::Eye => 'O',
            Part::Catchlight => '*',
            Part::Barbel => 'b',
            Part::Tail => 'T',
            Part::Stripe => '=',
            Part::Bar => '|',
        }
    }
}

/// One rasterised fish.
///
/// A [`BrailleGrid`] for the dots, plus a parallel per-dot *shade* in `0.0..=1.0`
/// running from the belly to the back. The shade is what [`FishArt::cells`] turns
/// into a cell colour, and it is why a fish is lit rather than flat: a braille
/// cell is one colour, so the light has to be resampled from the dots to the cell,
/// and the dots are the only place it exists.
pub struct FishArt {
    grid: BrailleGrid,
    /// Row-major over *dots*, `length` by `height`.
    shade: Vec<f32>,
    /// Row-major over the same dots: what each one is.
    part: Vec<Part>,
    length: usize,
    height: usize,
    /// The fish's own dot size, before the slack [`FishArt::shifted`] spends.
    natural: (usize, usize),
    cells_w: usize,
    cells_h: usize,
}

impl FishArt {
    /// The master bitmap's dot width, **including the slack** that
    /// [`FishArt::shifted`] spends. The fish's own length is
    /// `cells_wide() * DOTS_X - DOTS_X`, rounded up; use [`FishArt::cells_wide`]
    /// for anything the simulation needs.
    pub fn width(&self) -> usize {
        self.length
    }

    /// The master bitmap's dot height, including the same slack.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The fish's own length in dots, without the shift slack.
    pub fn body_length(&self) -> usize {
        self.natural.0
    }

    /// Every **raised** dot, in the master bitmap's coordinates.
    ///
    /// The shape of the animal rather than its light, which is what the
    /// cross-medium silhouette comparison in the tests needs. `parts()` cannot
    /// answer it: the fill for an unraised dot is [`Part::Flesh`], the same value
    /// a raised one carries, so "is there ink here" is a question about the grid
    /// and not about the part.
    pub fn raised_dots(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let (w, h) = (self.length, self.height);
        (0..h).flat_map(move |dy| {
            (0..w).filter_map(move |dx| self.grid.dot(dx, dy).then_some((dx, dy)))
        })
    }

    /// The fish's own height in dots, without the shift slack.
    pub fn body_height(&self) -> usize {
        self.natural.1
    }

    /// Width in terminal cells, which is what the separation and the wall margin
    /// are computed from.
    pub fn cells_wide(&self) -> usize {
        self.cells_w
    }

    /// Height in terminal cells.
    pub fn cells_tall(&self) -> usize {
        self.cells_h
    }

    /// What a dot is. Out-of-grid reads as [`Part::Flesh`].
    pub fn part_at(&self, dot_x: usize, dot_y: usize) -> Part {
        if dot_x >= self.length || dot_y >= self.height {
            return Part::Flesh;
        }
        self.part[dot_y * self.length + dot_x]
    }

    /// The shading value at a dot, `0.0` at the belly and `1.0` at the back.
    pub fn shade_at(&self, dot_x: usize, dot_y: usize) -> f32 {
        if dot_x >= self.length || dot_y >= self.height {
            return 0.0;
        }
        self.shade[dot_y * self.length + dot_x]
    }

    /// Whether a dot is raised. Out-of-grid reads as lowered.
    pub fn dot(&self, dot_x: usize, dot_y: usize) -> bool {
        self.grid.dot(dot_x, dot_y)
    }

    /// The braille character for one cell of the fish's own grid.
    pub fn cell_char(&self, cell_x: usize, cell_y: usize) -> char {
        self.grid.cell_char(cell_x, cell_y)
    }

    /// The fish's own cells, for the effect to paint.
    ///
    /// Yields `(cell_x, cell_y, glyph, shade)` **in cell offsets** for every
    /// non-blank cell, and the caller adds the fish's position in cells.
    ///
    /// Cell offsets and not dot offsets, which is the mistake this iterator
    /// shipped with first. The glyph is one per cell and the shade has already
    /// been resampled from the cell's eight dots, so there is nothing left in the
    /// result that is finer than a cell -- and yielding dots put every fish up to
    /// four times and two rows away from where it was, which on a 30-row tank is
    /// most of them off the bottom of the screen. The tank rendered as water and
    /// gravel with no fish in it, and the only clue was that the frame was 3,000
    /// cells of churn.
    ///
    /// An iterator rather than a `blit` onto a `&mut Canvas` because of what the
    /// caller needs per cell: the water colour for the row the cell lands on, and
    /// the fish's depth, to mix the aerial perspective. Both live on the effect,
    /// and a `&Canvas` borrow plus two closures over `self` is a borrowck fight
    /// for no gain.
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32, char, f32)> + '_ {
        (0..self.cells_h).flat_map(move |cell_y| {
            (0..self.cells_w).filter_map(move |cell_x| {
                let symbol = self.grid.cell_char(cell_x, cell_y);
                if symbol == ' ' {
                    return None;
                }
                Some((
                    cell_x as i32,
                    cell_y as i32,
                    symbol,
                    self.cell_shade(cell_x, cell_y),
                ))
            })
        })
    }

    /// A copy displaced by `(px, py)` dots, for sub-cell motion.
    ///
    /// A braille cell is the atomic paint unit, so a fish cannot be drawn at
    /// "three quarters of the way across column 40" -- there is no such cell. It
    /// can be drawn in column 40 with its dots displaced by one, which is half a
    /// cell, and that is what this is for.
    ///
    /// Without it, a fish's position rounds to a whole cell and it steps one
    /// column at a time, which reads as sliding along a grid rather than as
    /// swimming. With `2` horizontal phases and `4` vertical ones the finest step
    /// is half a column and a quarter of a row.
    ///
    /// The master bitmap from [`rasterise`] carries one spare dot column and four
    /// spare dot rows precisely so this never reads out of bounds; what it costs
    /// is the outermost dot column at the snout and up to three rows off the
    /// belly, both of which are the thin ends of a fish and both of which are
    /// gone at a different phase on a different frame, so nothing sits still long
    /// enough to show it.
    pub fn shifted(&self, px: usize, py: usize) -> FishArt {
        let (cw, ch) = (self.cells_w, self.cells_h);
        let (out_w, out_h) = (cw * DOTS_X, ch * DOTS_Y);
        let mut out = FishArt {
            grid: BrailleGrid::new(cw, ch),
            shade: vec![0.0; out_w * out_h],
            part: vec![Part::Flesh; out_w * out_h],
            length: out_w,
            height: out_h,
            natural: self.natural,
            cells_w: cw,
            cells_h: ch,
        };
        for oy in 0..out_h {
            for ox in 0..out_w {
                let (mx, my) = (ox + px, oy + py);
                if mx < self.length && my < self.height && self.grid.dot(mx, my) {
                    out.grid.raise_dot(ox, oy);
                    out.shade[oy * out_w + ox] = self.shade[my * self.length + mx];
                    out.part[oy * out_w + ox] = self.part[my * self.length + mx];
                }
            }
        }
        out
    }

    /// A left-facing copy.
    ///
    /// Derived by mirroring, for the crab's reason and with the crab's warning
    /// attached: a hand-mirrored sprite has to be redrawn by hand every time the
    /// art changes, and when that was skipped there the left-facing clap's first
    /// three lines came out byte-identical to the right-facing ones, so its claws
    /// and eyes were both on the wrong side and only its legs were flipped.
    /// Deriving it cannot happen.
    ///
    /// Mirrored *dots* rather than glyphs, so the bit pattern inside a cell is
    /// mirrored dot by dot rather than the cell being flipped as a unit.
    pub fn mirrored(&self) -> FishArt {
        let mut out = FishArt {
            grid: BrailleGrid::new(self.cells_w + 1, self.cells_h + 1),
            shade: vec![0.0; self.length * self.height],
            part: vec![Part::Flesh; self.length * self.height],
            length: self.length,
            height: self.height,
            natural: self.natural,
            cells_w: self.cells_w,
            cells_h: self.cells_h,
        };
        for y in 0..self.height {
            for x in 0..self.length {
                // The far end of the row becomes the near end.
                let dst_x = self.length - 1 - x;
                if self.grid.dot(x, y) {
                    out.grid.raise_dot(dst_x, y);
                    out.shade[y * self.length + dst_x] =
                        self.shade[y * self.length + x];
                    out.part[y * self.length + dst_x] =
                        self.part[y * self.length + x];
                }
            }
        }
        out
    }

    /// Mean shade of a cell's raised dots, or `0.0` for an empty one.
    ///
    /// Mean rather than max, because a cell straddling the fish's back should be
    /// the average of what it covers, which is what puts the boundary between the
    /// lit back and the shaded flank in the right place. A cell with a single
    /// raised dot takes that dot's shade rather than dividing by eight.
    fn cell_shade(&self, cell_x: usize, cell_y: usize) -> f32 {
        let (dx0, dy0) = (cell_x * DOTS_X, cell_y * DOTS_Y);
        let (mut sum, mut n) = (0.0f32, 0usize);
        for sub in 0..DOTS_PER_CELL {
            let (dx, dy) = (dx0 + sub % DOTS_X, dy0 + sub / DOTS_X);
            if self.grid.dot(dx, dy) {
                sum += self.shade_at(dx, dy);
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { sum / n as f32 }
    }
}

/// Resamples a fish's shade into a colour: belly, flank, back.
///
/// Three stops rather than a straight lerp, because a two-stop ramp from a dark
/// belly to a lit back puts most of the fish in the middle and gives it no flank
/// -- and the flank is where a fish's colour lives. The dorsal stop is pulled
/// toward white rather than merely brightened, because light on a wet back is a
/// *specular* highlight, and brightening a saturated hue does not look like one.
pub fn shade_colour(base: Color, shade: f32) -> Color {
    let t = shade.clamp(0.0, 1.0);
    let (belly, dorsal) = if let Color::Rgb { r, g, b } = base {
        (
            // **Darker than the flank**, which is the opposite of countershading
            // and the whole point. A real fish is pale underneath to hide from
            // things looking up at it, but this tank is lit from above, and the
            // ramp's job is to say so. A belly at 0.70 of the base plus an offset
            // came out at OKLab luminance 0.61 against the flank's 0.51 -- the
            // fish was lit from underneath, which is a fish lit from nowhere, and
            // `the_shading_ramp_has_a_flank` failed on it immediately. The belly
            // also sinks toward the water's own colour, which is what makes a
            // fish look like it is *in* the water rather than pasted on it.
            Color::Rgb {
                r: (r as f32 * 0.52 + 18.0).clamp(0.0, 255.0) as u8,
                g: (g as f32 * 0.56 + 24.0).clamp(0.0, 255.0) as u8,
                b: (b as f32 * 0.58 + 30.0).clamp(0.0, 255.0) as u8,
            },
            Color::Rgb {
                r: (r as f32 * 0.42 + 152.0).clamp(0.0, 255.0) as u8,
                g: (g as f32 * 0.42 + 166.0).clamp(0.0, 255.0) as u8,
                b: (b as f32 * 0.42 + 176.0).clamp(0.0, 255.0) as u8,
            },
        )
    } else {
        (base, base)
    };
    if t < 0.5 {
        palette::lerp(belly, base, t * 2.0)
    } else {
        palette::lerp(base, dorsal, (t - 0.5) * 2.0)
    }
}

/// Builds a fish bitmap from a species and a pose, at the shipped amplitude.
///
/// `pose` is the index into [`POSES`] and the only thing it changes is where
/// [`wave_offset`] has put each column of the rear half, which is what
/// `the_head_does_not_move_and_the_rear_does` checks.
pub fn rasterise(species: &Species, pose: usize) -> FishArt {
    rasterise_at(species, pose, WAVE_AMPLITUDE)
}

/// [`rasterise`] with the wave amplitude as an argument.
///
/// Exists so the amplitude can be swept against a measurement instead of chosen by
/// eye -- see [`wave_offset`]. Two knobs in one renderer is not a seam, it is the
/// measurement.
pub fn rasterise_at(species: &Species, pose: usize, amplitude: f32) -> FishArt {
    let length = species.length.max(8);

    // The bitmap has to hold the body *and* everything the fins reach past it, or
    // a discus loses the trailing half of its dorsal and becomes a different
    // animal from the one the table describes.
    //
    // The padding is half a dot, not a dot and a half: it only has to keep the
    // outline off the bitmap's edge so the shape is not clipped, and every extra
    // row is a row the fish's centre is pushed away from, which shows up as a
    // fish that sits low in its own bounding box.
    let girth = species.girth;
    let dorsal_max = species
        .dorsal
        .iter()
        .map(|(_, h)| *h * girth)
        .fold(0.0f32, f32::max);
    let anal_max = species
        .anal
        .iter()
        .map(|(_, h)| *h * girth)
        .fold(0.0f32, f32::max);
    let above = species.girth + dorsal_max + 0.5;
    let below = species.girth + anal_max + 0.5;
    let height = ((above + below).ceil() as usize).max(8);
    // The spine sits below the tallest dorsal rather than in the middle, because
    // a fish with a big dorsal is not vertically symmetric about its own centre.
    let spine = above;

    let (cells_w, cells_h) = (length.div_ceil(DOTS_X), height.div_ceil(DOTS_Y));
    // The grid is deliberately a dot column and four dot rows larger than the
    // fish needs. That slack is what [`FishArt::shifted`] spends: it re-reads the
    // master through a window offset by up to one dot across and three down, and
    // without the slack the window runs off the end of the bitmap on three of the
    // four vertical phases.
    // The padded dot dimensions. `stride` is the *padded* width, and every write
    // into `shade` has to use it: the vector is allocated and read at the padded
    // stride, and indexing it at the natural one silently scatters every fish's
    // shading across the wrong dots. Nothing crashed and every dot was still
    // raised, so the tank rendered -- with a dorsal fin at the belly's tone, a
    // catchlight at the bottom of the ramp, and an eye on the fish's back. Three
    // of the tests below found it in one run; the picture had looked *fine*.
    let stride = length + DOTS_X;
    let padded_h = height + DOTS_Y - 1;
    let mut art = FishArt {
        grid: BrailleGrid::new(cells_w + 1, cells_h + 1),
        shade: vec![0.0; stride * padded_h],
        part: vec![Part::Flesh; stride * padded_h],
        length: stride,
        height: padded_h,
        natural: (length, height),
        cells_w,
        cells_h,
    };

    // The peduncle: the stalk the tail hangs off. Read at the point the caudal
    // starts, because past that the body's own shape stops mattering.
    let tail_start = ((species.caudal.0).clamp(0.3, 0.96) * (length - 1) as f32)
        .round() as usize;
    let tail_start = tail_start.min(length - 2);
    let ped = body_shape(
        tail_start as f32 / (length - 1) as f32,
        species.peak,
        species.blunt,
        species.taper,
    ) * species.girth;
    let (ped_up, ped_down) = half_extents(species, ped);

    // The eye, sized to the fish rather than fixed: an eye the same size on a
    // neon and a discus makes one of them wrong.
    //
    // Its *position* is found rather than stated, and this was the second bug in
    // this file. A fixed `t = 0.115` puts the eye on a small fish's water: the
    // head of a 22-dot neon is four dots long, so at that fraction the body is
    // not yet deep enough to hold an eye and the dot lands outside the silhouette
    // where nothing is ever drawn. So the eye goes at the first column whose body
    // is deep enough to hold it, which is the one definition of "on the head"
    // that stays true from a 22-dot neon to a 34-dot discus.
    // The eye's radius, and the floor is the number that matters.
    //
    // **A mark has to be big enough for the medium to show it, and this one was
    // not.** At `1.0` the pupil is a five-dot plus shape. A braille cell is eight
    // dots and renders as **one colour**, so that plus is averaged with the body
    // dots sharing its cell and comes out a shade or two off the flank -- which
    // is nothing. The art *had* an eye the whole time: the part map said `O` and
    // the shade map said `0.03`. Neither reaches the terminal, because
    // `FishArt::cells` resamples eight dots into one colour and the eye is one
    // of them.
    //
    // `1.5` is a nine-dot block, wider than a cell, so it lands in two, and two
    // cells carrying a third-dark dot each is something you can see. On an
    // eight-dot-deep neon that is a large eye -- a third of the fish's height --
    // and that is *correct here*, because these are the far species. A distant
    // animal's eye is a disproportionately large dark spot, and it is most of why
    // a far fish reads as a fish at all; the near species in `charart` get a small
    // precise one for the same reason in the other direction. **The eye's size is
    // another thing the medium split decided for free.**
    //
    // It is also the same lesson the character art needed for the same reason, and
    // that is worth noticing: both media hide a mark that is too small, and both
    // hide it in a way no assertion about the *data* would ever catch.
    let eye_r = (species.girth * 0.28).max(1.5);
    let mut eye_x = ((length as f32) * 0.10).round() as usize;
    for dx in 0..length {
        let s = body_shape(
            dx as f32 / (length - 1) as f32,
            species.peak,
            species.blunt,
            species.taper,
        ) * species.girth;
        if s >= eye_r * 1.6 {
            eye_x = dx;
            break;
        }
    }
    // In the upper third of the head, which is where an eye is on every fish
    // that has one.
    let eye_depth = body_shape(
        eye_x as f32 / (length - 1) as f32,
        species.peak,
        species.blunt,
        species.taper,
    ) * species.girth;
    let eye_y = spine - (eye_depth * 0.42).max(0.5);

    // Where each fin's tallest point is, read off its own control points so the
    // shape function and the table cannot disagree about where a fin is.
    let dorsal_centre = fin_centre(species.dorsal);
    let anal_centre = fin_centre(species.anal);

    // The pose's job is to sample a **travelling wave** along the spine, and the
    // sample point is the pose.
    //
    // This replaced a rigid body with a caudal fin swinging about its root, which
    // is a pendulum, and a pendulum at two poses is a snap. A fish swimming is a
    // wave travelling from head to tail: the head barely moves, the middle bends,
    // and the tail sweeps. All three, from one expression.
    let phase = pose as f32 / POSES as f32;

    for dx in 0..length {
        let t = dx as f32 / (length - 1) as f32;
        let in_tail = dx >= tail_start;
        let tail_len = (length - 1 - tail_start).max(1) as f32;

        // The body, or the tail's flare out of the peduncle.
        let (body_up, body_down, dorsal_h, anal_h) = if in_tail {
            // A square root, so the fin opens *fast* out of the stalk and then
            // holds its width. Both earlier curves were wrong in the same
            // direction: linear gives a long wedge, and squaring gives a wedge
            // that only reaches full width on its final column, so the tail was
            // a triangle balanced on a thread rather than a fin.
            let f = ((dx - tail_start) as f32 / tail_len).clamp(0.0, 1.0);
            let flare = f.sqrt() * (1.0 - 0.12 * f.powi(6));
            let reach = species.caudal.1 * girth;
            (
                ped_up + (reach - ped_up).max(0.0) * flare,
                ped_down + (reach - ped_down).max(0.0) * flare,
                0.0,
                0.0,
            )
        } else {
            let s =
                body_shape(t, species.peak, species.blunt, species.taper) * girth;
            let (up, down) = half_extents(species, s);
            (
                up,
                down,
                shape_fin(
                    species.fin_style,
                    t,
                    interpolate(species.dorsal, t) * girth,
                    dorsal_centre.0,
                    dorsal_centre.1,
                ),
                shape_fin(
                    species.fin_style,
                    t,
                    interpolate(species.anal, t) * girth,
                    anal_centre.0,
                    anal_centre.1,
                ),
            )
        };

        // Where this column's centre sits, which is the wave. See [`wave_offset`]
        // for its shape and for the sweep that chose [`WAVE_AMPLITUDE`].
        let t = dx as f32 / (length - 1).max(1) as f32;
        let axis = spine + amplitude * girth * wave_offset(t, phase);
        let top = axis - body_up;
        let bottom = axis + body_down;

        for dy in 0..height {
            let y = dy as f32;
            let d = y - axis;

            // Barbels first, because they live *outside* the body and every
            // region test below ends in a `continue` for a dot that is outside
            // all of them. Tested second, they are unreachable -- a whisker
            // under a cory's chin is by definition below its belly, which is
            // exactly where the region test says there is nothing.
            // A whisker, not a beard. Following `bottom` put four rows of it under
            // a cory's chin, because a flat-bellied fish's belly drops three dots
            // in the two columns of its snout -- and a beard reads as a second jaw.
            // Anchored to the spine instead, it is one row wherever it lands.
            if species.barbels
                && !in_tail
                && dx <= eye_x
                && (y - (axis + species.girth * 0.74)).abs() < 0.6
            {
                art.grid.raise_dot(dx, dy);
                art.shade[dy * stride + dx] = 0.28;
                art.part[dy * stride + dx] = Part::Barbel;
                continue;
            }

            // Which of the three regions, and the shade that goes with it.
            // Every region is **solid**; see the module docs on why there is no
            // dither anywhere in a fish. Tone is the cell colour's job.
            let (shade, part) = if y >= top && y <= bottom {
                let signed = if body_up + body_down > 0.0 {
                    (-d / (body_up + body_down) * 2.0).clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                // The body's own ramp deliberately does **not** reach 0 or 1.
                //
                // It used to, and the fins were drawn at 0.92 and 0.08, which put
                // a dorsal fin within 8% of the tone of the back underneath it --
                // so the two merged and a discus came out as one round blob with a
                // bump. A fin is legible as a fin because of the *step* in tone
                // where it leaves the body, so the body gets the middle of the
                // range and the fins get the ends, which is also how a
                // translucent membrane behaves: it shows the lit water above it
                // through itself, and the dark water below it through itself.
                let mut base = 0.51 + 0.29 * signed;
                let mut part = if in_tail { Part::Tail } else { Part::Flesh };

                match species.markings {
                    Markings::None => {}
                    Markings::Lateral { y: ly, half } => {
                        if (signed / 2.0 - ly / 2.0).abs() < half / 2.0 {
                            // Bright, but *not* as bright as a dorsal fin: a stripe
                            // at 0.99 against a fin at 1.0 is a step of one
                            // percent, and on a fish whose whole identity is that
                            // stripe, the dorsal has to stay visible.
                            base = 0.88;
                            part = Part::Stripe;
                        }
                    }
                    Markings::Bars {
                        count,
                        from,
                        to,
                        strength,
                    } => {
                        if t >= from && t <= to && count >= 1.0 {
                            let period = (to - from).max(1e-3) / count;
                            let phase = ((t - from) / period).fract();
                            // A third of each period. A fifth was too narrow to
                            // survive the resampling to a cell: a discus's bars
                            // are two dots apart at this length, so a 0.6-dot bar
                            // averaged with two dots of body and came back as a
                            // tenth of a tone step, which is nothing.
                            if phase < 0.34 {
                                base *= 1.0 - strength;
                                part = Part::Bar;
                            }
                        }
                    }
                    Markings::Mottled { amount } => {
                        if hash_dot(dx as i32, dy as i32) < amount {
                            base *= 0.80;
                        }
                    }
                }

                // The eye, and its catchlight. Over the markings, because an eye
                // with a bar through it is still an eye.
                let ex = dx as f32 - eye_x as f32;
                let ey = y - eye_y;
                if (ex * ex + ey * ey).sqrt() <= eye_r {
                    base = 0.03;
                    part = Part::Eye;
                    // One dot of catchlight, up and forward of the pupil, placed
                    // so it lands in a *different cell*: dots one apart
                    // horizontally are different cells, and a catchlight sharing
                    // a cell with the pupil is averaged away and invisible.
                    if ex <= -0.5 && ey <= -0.5 && eye_r >= 1.2 {
                        base = 1.0;
                        part = Part::Catchlight;
                    }
                }
                (base, part)
            } else if y < top && y >= top - dorsal_h {
                // The top of the range, which the body never reaches. See above.
                (1.0, Part::Dorsal)
            } else if y > bottom && y <= bottom + anal_h {
                // And the bottom of it, for the same reason.
                (0.0, Part::Anal)
            } else {
                continue;
            };

            // A forked tail has a notch cut into the middle of its trailing edge.
            // Concave is the only way to draw one in a raster: a fin that is
            // simply "the tail" is a triangle.
            if in_tail && let TailFork::Forked { depth } = species.caudal.2 {
                let to_tip = (length - 1 - dx) as f32;
                let notch =
                    depth * girth * (1.0 - to_tip / (tail_len * 0.7).max(1.0));
                if to_tip < tail_len * 0.7 && d.abs() < notch.max(0.0) {
                    continue;
                }
            }

            // Barbels are handled above the region tests; see there.
            art.grid.raise_dot(dx, dy);
            art.shade[dy * stride + dx] = shade;
            art.part[dy * stride + dx] = part;
        }
    }

    art
}

/// The body is symmetric above and below, so a species asks only how deep it is.
///
/// `Belly` is applied by the caller through [`half_extents`].
fn half_extents(species: &Species, shape: f32) -> (f32, f32) {
    let up = shape;
    let down = match species.belly {
        Belly::Symmetric => shape,
        // A keel is deepest under the shoulder, which is what the body shape
        // already is, so all it takes is a shave.
        Belly::Keel => shape * 0.88,
        Belly::Flat { level } => shape.min(species.girth * level),
    };
    (up, down)
}

/// Applies a fin's shape to how far out it reaches.
///
/// The control points say where a fin starts and stops and how tall it is at its
/// tallest. What they cannot say is whether the fin comes to a *point* or stays
/// rounded, and at dot resolution that is the difference between a fin and a
/// swell: a triangle reaches full height at one `t`, a sickle trails off towards
/// the tail, and a rounded one holds its height across the middle.
///
/// The fin's centre comes from [`fin_centre`] rather than a constant. It was a
/// constant -- `t = 0.40` -- and every anal fin in the crate has its control
/// points around `t = 0.67`, so the shape function multiplied them by zero and
/// **the anal fins were not drawn at all.** A dorsal and an anal are on the same
/// body at different places; a fin-shape function with the fin's position
/// hard-coded in it can only ever draw one of them.
fn shape_fin(
    style: FinStyle,
    t: f32,
    raw: f32,
    centre: f32,
    half_span: f32,
) -> f32 {
    if raw <= 0.0 || half_span <= 1e-4 {
        return 0.0;
    }
    match style {
        // Full height only at the fin's own middle, falling to nothing at both
        // ends on a smooth curve, so the edges are diagonals rather than steps.
        FinStyle::Triangle => {
            let u = ((t - centre) / (half_span * 0.85)).abs().clamp(0.0, 1.0);
            (1.0 - u * u) * raw
        }
        // Holds its height across the middle and falls off at both ends, so a
        // small rounded fin stays a fin instead of becoming a bump.
        FinStyle::Rounded => {
            let u = ((t - centre) / (half_span * 0.65)).abs().clamp(0.0, 1.0);
            (1.0 - u * u) * raw
        }
        // Rises steeply behind its leading edge and trails off towards the tail.
        // Asymmetric, because that is what a sickle is: a fin that is as long
        // behind its peak as in front of it is a lump.
        FinStyle::Sickle => {
            let before =
                ((t - (centre - half_span)) / half_span.max(1e-4)).clamp(0.0, 1.0);
            let after =
                (1.0 - ((t - centre) / half_span.max(1e-4)).clamp(0.0, 1.0)) * 1.6;
            before.min(1.0).min(after.clamp(0.0, 1.0)).powf(0.7) * raw
        }
    }
}

/// Where a fin's tallest point is, and how far it spreads either side of it.
///
/// Derived from the control points so that a fin's *position* is stated once, in
/// the table, and the shape function cannot disagree with it.
///
/// Every point counts towards the extent, **including the zero-height ones at
/// each end** -- those are what say where the fin starts and stops. Filtering
/// them out, which the first version did, leaves a fin whose control points all
/// sit at its peak, so its span is zero, so [`shape_fin`] returns nothing and
/// **every fin in the crate disappears.** Zero is a position, not an absence.
fn fin_centre(points: &[(f32, f32)]) -> (f32, f32) {
    let mut best = (0.0f32, 0.0f32);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for (t, h) in points {
        if *h > best.1 {
            best = (*t, *h);
        }
        lo = lo.min(*t);
        hi = hi.max(*t);
    }
    (best.0, ((hi - lo) * 0.5).max(1e-4))
}

/// Piecewise-linear lookup over `(t, value)` control points.
fn interpolate(points: &[(f32, f32)], t: f32) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    if t <= points[0].0 {
        return points[0].1;
    }
    let last = points[points.len() - 1];
    if t >= last.0 {
        return last.1;
    }
    for pair in points.windows(2) {
        let (t0, v0) = pair[0];
        let (t1, v1) = pair[1];
        if t >= t0 && t <= t1 {
            let span = t1 - t0;
            if span <= f32::EPSILON {
                return v1;
            }
            return v0 + (v1 - v0) * ((t - t0) / span);
        }
    }
    last.1
}

/// A stable per-dot hash in `0.0..=1.0`.
///
/// The same dot gets the same value for ever, so a mottled pattern does not crawl
/// when the fish holds still. The crab's rule about a `rand` in a draw, applied
/// to a pattern rather than a floor.
fn hash_dot(x: i32, y: i32) -> f32 {
    let mut h =
        (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 13;
    (h & 0xFFFF) as f32 / 65535.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Side of the square grid every species' ink is resampled onto before two
    /// species are compared. See [`no_two_species_are_the_same_animal`].
    const GRID: usize = 32;

    /// The mean density difference two species must exceed to count as different
    /// animals.
    ///
    /// A measurement, and both ends of it matter more than the number. A duplicate
    /// species scores **0.0000** by construction, and the twenty-one real pairs
    /// measure between **0.019** (`longfin` against `stipple`, the tightest) and
    /// **0.175** (`tetra` against `fry`, the loosest), with a median of 0.065.
    ///
    /// So the threshold sits well clear of a duplicate and below the tightest real
    /// pair, with about 37% of headroom against the pair it is closest to catching.
    ///
    /// 0.012 is a **weak** threshold and saying so is part of the finding. The
    /// previous set of drawings -- a round fish, a tall one and three low ones --
    /// had a tightest pair of 0.055. This set has one at 0.019, because **these are
    /// five spindles by one hand**: 1.33 to 1.90, no round fish and no tall one. Two
    /// drawings in one idiom at the same size really are that similar, and a density
    /// comparison is the right instrument and it has a small number to report.
    ///
    /// The threshold is low *because the art is uniform*, and
    /// [`MEDIAN_DISTINCT`] is what stops that being an excuse.
    const MIN_DISTINCT: f32 = 0.012;

    /// The median pair must clear this, so a uniformly similar set fails.
    ///
    /// Measured 0.065. The bound is 0.04, under half of it, and this is the
    /// assertion that carries the weight: [`MIN_DISTINCT`] only has to separate a
    /// duplicate from a near-miss, and this says the set as a whole is not made of
    /// near-misses.
    const MEDIAN_DISTINCT: f32 = 0.04;

    /// The widest proportion any species is scaled by when it is laid into the
    /// shared grid, so a fish at the top of the aspect band fills the whole width
    /// and a narrower one fills proportionally less.
    ///
    /// 2.0 rather than the set's actual maximum, on purpose: rounding the
    /// reference up to a round number means the scale does not have to change when
    /// a future drawing is a little wider, and no species can reach the right-hand
    /// edge of the grid and wrap.
    const ASPECT_NORM: f32 = 2.0;
    use crate::render::palette::perceptual_distance;

    /// Ink raised per dot column, resampled to a common width and normalised.
    ///
    /// The measurement behind `the_species_are_not_the_same_fish_at_different_lengths`.
    /// It is deliberately scale-free: two species of different lengths are compared
    /// as *shapes*, because "is this the same fish" is a question about the
    /// profile and not about how many cells it spans.
    fn normalised_profile(species: &Species, samples: usize) -> Vec<f32> {
        let fish = rasterise(species, 0);
        let (w, h) = (fish.width(), fish.height());
        let mut per_column = vec![0.0f32; w];
        for (x, slot) in per_column.iter_mut().enumerate() {
            *slot = (0..h).filter(|y| fish.dot(x, *y)).count() as f32;
        }
        let peak = per_column.iter().copied().fold(0.0f32, f32::max).max(1.0);
        (0..samples)
            .map(|i| {
                // Sample the *middle* of each source column rather than its edge, so
                // a resampling that straddles a step does not read as a ramp.
                let t = (i as f32 + 0.5) / samples as f32;
                let x = ((t * w as f32) as usize).min(w - 1);
                per_column[x] / peak
            })
            .collect()
    }

    /// Root-mean-square difference between two profiles.
    fn profile_distance(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let sum: f32 = (0..n).map(|i| (a[i] - b[i]).powi(2)).sum();
        (sum / n.max(1) as f32).sqrt()
    }

    /// Every species is a different animal.
    ///
    /// **This is the test whose absence let three bars ship.** Every assertion the
    /// old art had was about *one* sprite at a time -- this one tapers to its
    /// tail, that one is the right aspect ratio -- and all of them passed on three
    /// sprites that were the same object in three lengths. Nothing compared one
    /// species to another, so "they are all runs of `(`" was not a fact anything
    /// could see.
    ///
    /// The measurement is the ink profile per dot column, resampled to a common
    /// width and normalised by its own peak, so it compares *shape* and not size.
    /// The threshold is a measurement and not a round number: the three old
    /// sprites sat within 0.02 of each other on this, and the five current ones
    /// are 0.09 apart at the closest.
    #[test]
    fn the_species_are_not_the_same_fish_at_different_lengths() {
        const SAMPLES: usize = 48;
        const MIN_DISTANCE: f32 = 0.055;
        let profiles: Vec<Vec<f32>> = SPECIES
            .iter()
            .map(|s| normalised_profile(s, SAMPLES))
            .collect();
        let mut closest = f32::INFINITY;
        let mut closest_pair = ("", "");
        for i in 0..SPECIES.len() {
            for j in (i + 1)..SPECIES.len() {
                let d = profile_distance(&profiles[i], &profiles[j]);
                if d < closest {
                    closest = d;
                    closest_pair = (SPECIES[i].name, SPECIES[j].name);
                }
                assert!(
                    d > MIN_DISTANCE,
                    "{} and {} differ by only {d:.3} in body profile, so they are \
                     the same fish at two lengths; a tank of one animal is a \
                     pattern, not a shoal",
                    SPECIES[i].name,
                    SPECIES[j].name
                );
            }
        }
        assert!(
            closest > MIN_DISTANCE,
            "closest pair is {} and {} at {closest:.3}",
            closest_pair.0,
            closest_pair.1
        );
    }

    /// A species is not a bar: it is deepest in the middle and pointed at the nose.
    ///
    /// The old art was three rectangles of different lengths and every one of its
    /// per-species tests passed. Two properties catch it, and both are about the
    /// *ends* rather than the middle:
    ///
    /// - the front of the fish has to be much shallower than its deepest point, or
    ///   it has a wall instead of a head;
    /// - the back has to come back up again, or it has no tail.
    #[test]
    fn every_species_is_pointed_at_the_nose_and_tapered_at_the_tail() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            let per_column: Vec<f32> = (0..w)
                .map(|x| (0..h).filter(|y| fish.dot(x, *y)).count() as f32)
                .collect();
            let peak = per_column.iter().copied().fold(0.0f32, f32::max);
            assert!(
                peak > 3.0,
                "{}: its deepest column holds {peak} dots; there is no fish here",
                species.name
            );
            // The snout is a *point* whatever the species' bluntness: a blocky
            // cory and a pointed minnow both come to nothing at column zero, and a
            // fish that does not is a rectangle with a tail on it.
            assert!(
                per_column[0] < peak * 0.4,
                "{}: its first column holds {:.0} dots against a peak of {peak}, so \
                 the head is a vertical wall",
                species.name,
                per_column[0]
            );
            // The first fifth is looser, because bluntness lives exactly here and
            // `blunt: 1.1` on a cory is meant to be a full-depth forehead.
            let fifth = (w / 5).max(1);
            let nose = per_column[..fifth].iter().sum::<f32>() / fifth as f32;
            let tail = per_column[w - fifth..].iter().sum::<f32>() / fifth as f32;
            assert!(
                nose < peak * 0.8,
                "{}: the first fifth of its length averages {nose:.1} dots against \
                 a peak of {peak}",
                species.name
            );
            assert!(
                tail < peak * 0.7,
                "{}: the last fifth averages {tail:.1} dots against a peak of \
                 {peak}; the fish does not taper to anything",
                species.name
            );
        }
    }

    /// The body is solid, not a halftone screen.
    ///
    /// The second version of this file made the fins 78% ink, on the theory that
    /// a translucent fin is a dithered one. At dot resolution that is not a tone,
    /// it is a 4x4 Bayer matrix tiled across a ten-dot region, and the printed
    /// result was a fish wearing a screen door. The rule is absolute: **nothing in
    /// a fish is dithered**, and the ink fraction of anything more than one dot
    /// from the outline is 1.
    /// The body is solid, and nothing in a fish is dithered.
    ///
    /// The second version of this file made the fins 78% ink, on the theory that
    /// a translucent fin is a dithered one. At dot resolution that is not a tone,
    /// it is a 4x4 Bayer matrix tiled across a ten-dot region, and the printed
    /// result was a fish wearing a screen door. The first version of this test
    /// then tried to measure "the interior" and had nothing to measure on the
    /// smallest species.
    ///
    /// So it measures a **hole**: a dot with raised dots on all four sides of it.
    /// A dither produces one at every unraised dot in the region it covers, and
    /// nothing else does. The fork in a tail is not a hole, because it has nothing
    /// above or below it; the gap between a dorsal fin and a back is not a hole,
    /// for the same reason.
    #[test]
    fn the_body_is_solid_and_nothing_in_a_fish_is_dithered() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            let mut ink = 0usize;
            let mut holes: Vec<(usize, usize)> = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    if fish.dot(x, y) {
                        ink += 1;
                        continue;
                    }
                    let up = y > 0 && fish.dot(x, y - 1);
                    let down = y + 1 < h && fish.dot(x, y + 1);
                    let left = x > 0 && fish.dot(x - 1, y);
                    let right = x + 1 < w && fish.dot(x + 1, y);
                    if up && down && left && right {
                        holes.push((x, y));
                    }
                }
            }
            assert!(
                ink > 40,
                "{}: only {ink} dots of ink; there is no fish here",
                species.name
            );
            assert!(
                holes.is_empty(),
                "{}: {} dots are enclosed by its own ink, the first at {:?}. A \
                 fish whose flesh is a dither pattern is a halftone screen laid \
                 over the animal.",
                species.name,
                holes.len(),
                holes.first()
            );
        }
    }

    /// Both fins are drawn, they are attached, and they are a different tone from
    /// the body.
    ///
    /// Three separate bugs, one test, because all three produced the same picture:
    ///
    /// 1. `shape_fin` hard-coded the fin's centre at `t = 0.40` while every anal
    ///    fin's control points sit near `t = 0.67`, so it multiplied them by zero
    ///    and **no anal fin in the crate was drawn**.
    /// 2. The fin's tone was 0.92 against a body's back of 1.0, so a discus's
    ///    dorsal merged into its back and the fish came out as one round blob.
    /// 3. The first profile table put full body depth two dots behind the nose, so
    ///    every species had a vertical wall for a head.
    ///
    /// A fin is defined by its **tone** -- exactly the top or bottom of the ramp,
    /// which the body never reaches -- and attached means it has body flesh within
    /// one dot. That is deliberately not measured by rasterising the species again
    /// with its fin tables emptied: emptying them changes the girth envelope, so
    /// the spine moves, so the two bitmaps are not in a common frame and the
    /// subtraction compares a fish against a differently-shaped one.
    #[test]
    fn every_species_draws_two_attached_fins_a_tone_step_from_its_body() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            let is_flesh = |x: isize, y: isize| -> bool {
                x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && {
                    let (x, y) = (x as usize, y as usize);
                    fish.dot(x, y) && fish.part_at(x, y) == Part::Flesh
                }
            };
            // Attached means **reachable**, not adjacent. A fin two dots tall has
            // an outermost dot two rows from the flesh, which is correct and looks
            // attached; the first version of this checked for flesh within one dot
            // and reported every tall fin as floating.
            let reaches_flesh = |x: usize, y: usize| -> bool {
                const REACH: usize = 3;
                let mut seen = std::collections::HashSet::new();
                let mut frontier = vec![(x as isize, y as isize)];
                seen.insert((x, y));
                for _ in 0..REACH * REACH {
                    let mut next = Vec::new();
                    for (cx, cy) in frontier {
                        if is_flesh(cx, cy) {
                            return true;
                        }
                        for (dx, dy) in [(-1isize, 0isize), (1, 0), (0, -1), (0, 1)]
                        {
                            let (nx, ny) = (cx + dx, cy + dy);
                            if nx < 0
                                || ny < 0
                                || nx as usize >= w
                                || ny as usize >= h
                            {
                                continue;
                            }
                            if seen.insert((nx as usize, ny as usize))
                                && fish.dot(nx as usize, ny as usize)
                            {
                                next.push((nx, ny));
                            }
                        }
                    }
                    frontier = next;
                }
                false
            };

            let mut dorsal = 0usize;
            let mut anal = 0usize;
            let mut floating: Vec<(usize, usize)> = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let part = fish.part_at(x, y);
                    if !matches!(part, Part::Dorsal | Part::Anal) {
                        continue;
                    }
                    if part == Part::Dorsal {
                        dorsal += 1
                    } else {
                        anal += 1
                    };
                    if !reaches_flesh(x, y) {
                        floating.push((x, y));
                    }
                }
            }
            assert!(
                floating.is_empty(),
                "{}: {} fin dots cannot reach the body at all, the first at {:?}. A \
                 fin that is not joined to the animal is a bar floating above it.",
                species.name,
                floating.len(),
                floating.first()
            );
            assert!(
                dorsal >= 3,
                "{}: only {dorsal} dorsal-fin dots",
                species.name
            );
            assert!(
                anal >= 3,
                "{}: only {anal} anal-fin dots, so it has no bottom fin at all",
                species.name
            );
        }
    }

    /// A fin is a step in tone away from the body it sits on.
    ///
    /// The other half of the discus-blob bug, and the reason the fins are drawn
    /// at the ends of the ramp rather than merely "a lighter grey". The fin's
    /// tone was 0.92 and the body's own back reached 1.0, so the two were within
    /// eight percent of each other and a discus came out as one round lump with a
    /// bump on it. Legibility here is a *step*, not a value.
    #[test]
    fn a_fin_is_a_step_away_from_the_flesh_it_sits_on() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            let mut flesh = (f32::INFINITY, f32::NEG_INFINITY);
            for y in 0..h {
                for x in 0..w {
                    if fish.dot(x, y) && fish.part_at(x, y) == Part::Flesh {
                        let s = fish.shade_at(x, y);
                        flesh.0 = flesh.0.min(s);
                        flesh.1 = flesh.1.max(s);
                    }
                }
            }
            assert!(
                flesh.0 > 0.10,
                "{}: its flesh reaches down to {:.2} of the ramp, so it is already \
                 as dark as an anal fin",
                species.name,
                flesh.0
            );
            assert!(
                flesh.1 < 0.90,
                "{}: its flesh reaches up to {:.2} of the ramp, so it is already as \
                 light as a dorsal fin",
                species.name,
                flesh.1
            );
        }
    }

    /// No fin is so short that it lands between two rows.
    ///
    /// A fin's region is `body_bottom < y <= body_bottom + height`, and `y` is an
    /// integer row. A fin 1.0 dot tall therefore occupies a one-unit range whose
    /// position depends on where the body happens to end, and whether it contains
    /// a row at all is a coin flip: the neon's anal was drawn on some columns and
    /// not others, which reads as a fin that flickers as the fish swims. The
    /// threshold is 1.5 dots, which is the smallest height guaranteed to contain a
    /// row at either end of a body's range.
    #[test]
    fn no_fin_is_too_short_to_land_on_a_row() {
        const MIN_FIN: f32 = 1.5;
        for species in SPECIES {
            for (what, points) in
                [("dorsal", species.dorsal), ("anal", species.anal)]
            {
                // The table is in girths; the threshold is in dots.
                let tallest = points
                    .iter()
                    .map(|(_, h)| *h * species.girth)
                    .fold(0.0f32, f32::max);
                if tallest == 0.0 {
                    continue; // this species has no such fin, which is allowed
                }
                assert!(
                    tallest >= MIN_FIN,
                    "{}: its {what} fin peaks at {tallest:.1} dots, under {MIN_FIN}. A \
                     fin that short falls between rows and is drawn on some \
                     columns and not others.",
                    species.name
                );
            }
        }
    }
    /// A big fish has a catchlight, and the catchlight does what it can.
    ///
    /// A braille cell is one colour, so a catchlight is only visible if it lifts
    /// the mean of the eight dots around it. It is therefore **exactly one dot**,
    /// up and forward of the pupil, and it has to earn its place by brightening
    /// the cell it lands in.
    ///
    /// # It cannot be very bright, and that is geometry
    ///
    /// **A visible catchlight is impossible for a far species, and the reason is
    /// the eye's size relative to the fish's.** These fish are 8 and 12 dots deep,
    /// so a fish is two or three cells tall and its eye has to be about one cell
    /// tall to leave a body around it. An eye that is one cell tall cannot have a
    /// catchlight in a cell of its own -- the catchlight's cell necessarily holds
    /// three or four pupil dots beside it.
    ///
    /// Measured with the eye at `1.5` dots' radius -- a six-dot pupil spanning two
    /// cells wide and one tall -- the catchlight lifts its cell's mean from 0.78 to
    /// **0.89**, a difference of **0.121**, for both species, because the geometry
    /// is identical. The threshold is 0.10, which is 83% of the measurement, and it
    /// still fails if the catchlight is moved into the pupil or dropped.
    ///
    /// That is a weak glint, and it is the right one here. These are the *far*
    /// species -- the whole reason [`crate::aquarium::charart`] exists for the other
    /// three -- and a distant animal's eye is a dark spot with a faint sheen, not a
    /// wet one. The near species get a real `o` with a real highlight, because
    /// characters can. **The medium decides how much of an eye you can afford, and
    /// this is the second place that has now come up.**
    ///
    /// The two earlier versions of this test were both wrong in instructive ways.
    /// One demanded three catchlight dots, which is the one thing a catchlight must
    /// not be. The other demanded that it share a cell with nothing, which is
    /// impossible at this eye size and would have been satisfied by an eye too
    /// small to see.
    #[test]
    fn a_large_fish_has_one_catchlight_that_lifts_its_cell() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            assert!(
                (0..w)
                    .flat_map(|x| (0..h).map(move |y| (x, y)))
                    .any(|(x, y)| fish.part_at(x, y) == Part::Eye),
                "{}: no pupil, so nothing for a catchlight to be on",
                species.name
            );
            let glint: Vec<(usize, usize)> = (0..w)
                .flat_map(|x| (0..h).map(move |y| (x, y)))
                .filter(|(x, y)| fish.part_at(*x, *y) == Part::Catchlight)
                .collect();
            assert_eq!(
                glint.len(),
                1,
                "{}: {} catchlight dots. It must be exactly one, because a \
                 catchlight is only visible if it is alone in its cell.",
                species.name,
                glint.len()
            );
            // How much it lifts its own cell's mean, measured rather than asserted
            // from the data: resample the cell without the glint, and with it.
            let (gx, gy) = glint[0];
            let (cx, cy) = (gx / DOTS_X, gy / DOTS_Y);
            let mut with = 0.0f32;
            let mut without = 0.0f32;
            let mut n = 0.0f32;
            for dy in 0..DOTS_Y {
                for dx in 0..DOTS_X {
                    let (px, py) = (cx * DOTS_X + dx, cy * DOTS_Y + dy);
                    if px >= fish.length || py >= fish.height {
                        continue;
                    }
                    let shade = fish.shade_at(px, py);
                    with += shade;
                    n += 1.0;
                    // What the cell would have been with the pupil's own value
                    // there instead: the glint replaced a pupil dot.
                    without += if (px, py) == (gx, gy) { 0.03 } else { shade };
                }
            }
            let lift = with / n - without / n;
            assert!(
                lift > 0.10,
                "{}: its catchlight at ({gx},{gy}) lifts its cell's mean by only \
                 {lift:.3}. One dot of light is only visible if it brightens the \
                 cell, and this one does not.",
                species.name
            );
        }
    }

    /// The rear of a dot fish undulates and the front of it does not.
    ///
    /// This replaced `the_tail_poses_differ_only_in_the_tail`, which asserted that
    /// only the **caudal fin** moves between poses -- and that assertion *was* the
    /// design at the time, a rigid body with a swinging tail. The travelling wave
    /// moved the boundary back to `WAVE_START` and the test had to change with it.
    /// **A test that encodes a design decision has to be rewritten when the decision
    /// changes, not loosened.** The companion test
    /// `the_wave_bends_the_body_and_not_only_the_caudal_fin` is what carries the
    /// part that is genuinely new.
    ///
    /// **What this does and does not guard, stated plainly because it was wrong at
    /// first.** It derives its boundary from `WAVE_START`, and `wave_offset` is zero
    /// at `WAVE_START` *by construction* -- so setting `WAVE_START` to 0.0 leaves
    /// this passing, because `rear` is still clamped to zero at the snout. It is not
    /// a tautology, but it is weaker than it looks: it guards the front of the
    /// fish against **anything else** perturbing it -- the barbels, the eye, the
    /// markings, a future edit to the shading -- and not against the wave's own
    /// start. Pushing `WAVE_START` up is caught by the test above instead, because
    /// that is what stops the body bending.
    ///
    /// Two failures, and the second is the sneaky one. Poses that do not differ at
    /// all give a fish that slides. Poses that differ in the **head** give a fish
    /// that twitches, which is worse than one that slides because it reads as
    /// broken rather than as stiff. A fish's head leads and does not swim, so the
    /// front being identical across the whole cycle is the property.
    ///
    /// Every pair of poses, not just 0 against 1: a wave can be periodic enough
    /// that adjacent samples differ while opposite ones coincide.
    #[test]
    fn the_head_does_not_move_and_the_rear_does() {
        for species in SPECIES {
            let natural = rasterise(species, 0).body_length();
            // The front `WAVE_START` of the body, in dot columns. Off-by-one here
            // would move the boundary a dot, so it is derived from the constant
            // rather than retyped.
            let rigid_end = (WAVE_START * (natural - 1) as f32).round() as usize;

            for a in 0..POSES {
                for b in (a + 1)..POSES {
                    let fa = rasterise(species, a);
                    let fb = rasterise(species, b);
                    let mut head_moved = None;
                    let mut rear_moved = false;
                    for x in 0..fa.width() {
                        for y in 0..fa.height() {
                            if fa.dot(x, y) == fb.dot(x, y) {
                                continue;
                            }
                            if x < rigid_end {
                                head_moved = Some(x);
                            } else {
                                rear_moved = true;
                            }
                        }
                    }
                    assert!(
                        head_moved.is_none(),
                        "{}: poses {a} and {b} differ at dot column {}, in front of \
                         the wave's start at {rigid_end} -- a fish whose head changes \
                         between poses reads as twitching",
                        species.name,
                        head_moved.unwrap_or(0)
                    );
                    assert!(
                        rear_moved,
                        "{}: poses {a} and {b} are identical behind column {rigid_end}, \
                         so the fish does not swim",
                        species.name
                    );
                }
            }
        }
    }

    /// The wave bends the **body**, not just the caudal fin.
    ///
    /// This is the assertion that separates a travelling wave from the pendulum it
    /// replaced, and it is deliberately stated in **absolute columns** rather than
    /// as "everything behind `WAVE_START` moves", which would be true by
    /// construction and so would assert nothing.
    ///
    /// A rigid body with a swinging caudal was the design before, and `the_tail_
    /// poses_differ_only_in_the_tail` described it perfectly. The only way to tell
    /// the two designs apart from the pictures is to ask whether any dot changes
    /// **in front of the caudal fin** -- and the fin is a quarter of the fish, so
    /// a wave that only reached into it would be a tail wag wearing a wave's
    /// arithmetic.
    ///
    /// **Measured at the shipped amplitude**, between poses 0 and 1: **3** columns
    /// move ahead of the caudal on the neon and **6** on the tetra, and the
    /// frontmost column that moves at all is 0.48 along the body in both species --
    /// `WAVE_START` at 0.45 plus the sub-dot rounding, and identical across two
    /// species of different lengths, which is the sign that the wave is scaled to
    /// the animal rather than to a dot count.
    #[test]
    fn the_wave_bends_the_body_and_not_only_the_caudal_fin() {
        for species in SPECIES {
            let a = rasterise(species, 0);
            let b = rasterise(species, 1);
            // The caudal's own start, in dot columns of the **natural** length. The
            // padded bitmap width puts this a dot or two late, which would count a
            // peduncle column as body -- the first version of this measurement did.
            let tail_start = ((species.caudal.0).clamp(0.3, 0.96)
                * (a.body_length() - 1) as f32)
                .round() as usize;

            let moving_in_front = (0..tail_start)
                .filter(|&x| (0..a.height()).any(|y| a.dot(x, y) != b.dot(x, y)))
                .count();
            assert!(
                moving_in_front >= 2,
                "{}: only {moving_in_front} dot columns in front of the caudal at \
                 {tail_start} change between poses. The body ahead of the fin is \
                 rigid, so this is a fish with a swinging tail rather than a fish \
                 swimming, which is the pendulum this replaced.",
                species.name
            );
        }
    }

    /// The wave is visible, which is a measurement and not an intention.
    ///
    /// [`WAVE_AMPLITUDE`] cannot be quietly turned down to nothing. A dot is the
    /// quantum, so an amplitude small enough that the columns move less than a dot
    /// each moves *no dots at all* -- the arithmetic is in the picture, the pixels
    /// are not, and the only symptom is a constant that reads as a plausible number
    /// and a fish that does not swim. Asserting on the arithmetic would pass; this
    /// asserts on the ink.
    ///
    /// **This floor was itself wrong once and the way it was wrong is the useful
    /// part.** It sat at 25%, fitted to the 39% that the too-large amplitude
    /// produced -- and a floor fitted to a value is a floor that *pushes toward that
    /// value*, so it was quietly insisting the tail jiggled. Measured at the shipped
    /// amplitude the two species move 17% and 24%, so the floor is now **10%**:
    /// low enough to stop being a preference, high enough to catch a wave that has
    /// stopped (an amplitude of 0.05 moves 2%, a fortieth of what ships). The upper
    /// half of this pair of bounds is
    /// `the_tail_tip_does_not_travel_further_than_half_the_fish_is_deep`, and a floor
    /// with no ceiling is satisfied by every value up to the clipping limit.
    #[test]
    fn the_wave_moves_enough_dots_to_be_seen() {
        /// A tenth of the ink. Deliberately far below both measured values, because
        /// its job is to catch a wave that has stopped and nothing else.
        const MIN_FRACTION_MOVING: f32 = 0.10;

        for species in SPECIES {
            let a = rasterise(species, 0);
            let b = rasterise(species, POSES / 2);
            let ink = a.raised_dots().count().max(b.raised_dots().count());
            let moved = (0..a.height())
                .flat_map(|y| (0..a.width()).map(move |x| (x, y)))
                .filter(|&(x, y)| a.dot(x, y) != b.dot(x, y))
                .count();
            let fraction = moved as f32 / ink as f32;
            assert!(
                fraction >= MIN_FRACTION_MOVING,
                "{}: only {moved} of {ink} dots change between opposite poses \
                 ({:.0}%, floor {:.0}%). The tail is moving less than a dot, so no \
                 dots move and the fish slides.",
                species.name,
                fraction * 100.0,
                MIN_FRACTION_MOVING * 100.0
            );
        }
    }

    /// The tail tip does not travel further in one beat than half the fish is deep.
    ///
    /// The **ceiling** that was missing, and the reason the amplitude was 0.8 for a
    /// round. That value moved 39% of a neon's dots between poses, which cleared
    /// every lower bound the suite had -- and it was reported as the fish
    /// **jiggling**, because the tip was crossing **83% of the fish's own body
    /// depth, five times a second**. More motion, more dots changed, and the only
    /// thing wrong with it was that it was not a swim.
    ///
    /// **Stated as geometry rather than as a dot count, deliberately.** The previous
    /// bound was a fraction of the ink, and a fraction of the ink is satisfiable by
    /// recalibrating the very thing it measures. This one is a ratio of the tail's
    /// excursion to the body's own depth, so neither number can move without the
    /// fish changing shape.
    ///
    /// # The denominator, which is the whole difficulty
    ///
    /// It has to be the **flesh**, measured off the raster, and both halves of that
    /// matter:
    ///
    /// - **Not the finned silhouette.** With the dorsal and anal included, the
    ///   deepest column of a neon is 7 dots against 5 of body, and a bound built on
    ///   it is too loose to catch the value it exists to catch.
    /// - **Not `part_at` alone.** An *unraised* dot's part is also
    ///   [`Part::Flesh`], so the measurement has to ask for a dot that is both
    ///   raised **and** flesh -- otherwise it is the height of the whole bitmap
    ///   column, which is to say nothing at all. That is why `dot(x, y) &&` is in
    ///   there rather than looking like belt and braces.
    ///
    /// **Measured**: the neon's deepest flesh column is 5 dots and the tetra's is 9.
    /// The two are not in proportion to `girth` exactly and do not need to be --
    /// what matters is that the *ratio* agrees across two differently-proportioned
    /// animals, and it does, to two decimal places, at every amplitude.
    ///
    /// (The first version of this test measured **2** and **4**, from reading
    /// `peak` as the maximum of the shape function. It is about half that -- the
    /// function peaks near twice `peak`. That error made the jiggling sound like
    /// 2.1x the fish's depth rather than 83% of it, which was a dramatic
    /// overstatement of a real fault. **The number went in the direction of making
    /// the bug sound worse, and it was still wrong.**)
    #[test]
    fn the_tail_tip_does_not_travel_further_than_half_the_fish_is_deep() {
        /// How much of its own depth the tip may travel per beat. The shipped value
        /// measures 0.36; 0.50 measures 0.52 and is rejected. So the bound pins the
        /// amplitude to about 0.47, which is a narrow band on purpose -- this
        /// constant has been wrong in two consecutive rounds.
        const MAX_TIP_TRAVEL_IN_BODY_DEPTHS: f32 = 0.5;

        for species in SPECIES {
            let fish = rasterise(species, 0);
            let natural = fish.body_length();
            // Only the body, so the tail's own excursion cannot inflate the very
            // depth it is being compared against.
            let tail_start = ((species.caudal.0).clamp(0.3, 0.96)
                * (natural - 1) as f32)
                .round() as usize;

            let flesh_depth = (0..tail_start.min(fish.width()))
                .map(|x| {
                    let ys: Vec<usize> = (0..fish.height())
                        .filter(|&y| {
                            fish.dot(x, y) && fish.part_at(x, y) == Part::Flesh
                        })
                        .collect();
                    ys.last().map(|hi| hi + 1 - ys[0]).unwrap_or(0)
                })
                .max()
                .unwrap_or(0);
            assert!(
                flesh_depth > 0,
                "{}: no raised flesh at all in front of its caudal, so there is no \
                 body depth to measure.",
                species.name
            );

            let travel = 2.0 * WAVE_AMPLITUDE * species.girth;
            let ratio = travel / flesh_depth as f32;
            assert!(
                ratio <= MAX_TIP_TRAVEL_IN_BODY_DEPTHS,
                "{}: its tail tip travels {travel:.2} dots per beat against a body \
                 {flesh_depth} dots deep -- {ratio:.2} of its own depth, over the \
                 {MAX_TIP_TRAVEL_IN_BODY_DEPTHS} limit. That is a vibration, not a \
                 swim.",
                species.name
            );
        }
    }

    /// The beat **sweeps**. It does not snap between two extremes.
    ///
    /// Asserted on [`wave_offset`] itself rather than on the rasterised dots, and
    /// this is the **second** version. The first counted differing dots between each
    /// pair of poses and required every consecutive pair to differ by less than the
    /// widest pair -- the right idea, the wrong instrument, because **it measures
    /// the quantisation rather than the wave.** At the amplitude that used to ship
    /// it had clean margins (neon 29, 17 and 28 against a widest of 37); drop the
    /// amplitude to 0.35 and consecutive poses 2 and 3 tie at 18 with the widest
    /// pair. Nothing had got worse. Two samples of a sine a quarter-period apart
    /// *is* a snap, but at dot resolution "a snap" and "a small difference" are the
    /// same observation.
    ///
    /// The exact statement, in arithmetic with no quantisation in it: over a cycle
    /// the tail tip must occupy at least one position **strictly between** the two
    /// extremes. Two poses give the extremes and nothing else, which is the
    /// definition of a pendulum; four give the extremes and two in between.
    ///
    /// **Measured at the tail tip**, as fractions of [`WAVE_AMPLITUDE`] * `girth`:
    /// `+0.95`, `-0.99`, `-0.59`, `+0.16` -- two extremes and two real
    /// intermediates. The same arithmetic at two poses gives `-0.95` and `+0.95`
    /// and nothing between them.
    #[test]
    fn the_beat_sweeps_rather_than_snapping_between_two_extremes() {
        for species in SPECIES {
            let tip: Vec<f32> = (0..POSES)
                .map(|pose| wave_offset(1.0, pose as f32 / POSES as f32))
                .collect();
            let lo = tip.iter().cloned().fold(f32::MAX, f32::min);
            let hi = tip.iter().cloned().fold(f32::MIN, f32::max);
            let between = tip.iter().filter(|&&o| o > lo && o < hi).count();
            assert!(
                between >= 2,
                "{}: over the cycle its tail tip never sits strictly between the \
                 extremes {lo:.3} and {hi:.3} -- it visits {between} intermediate \
                 position(s) out of {POSES}: {tip:?}. That is a pendulum, a fish \
                 alternating between two extremes, and not a wave. POSES = {POSES} \
                 cannot sample a beat finely enough.",
                species.name
            )
        }
    }

    /// No two poses of a dot fish are the same picture.
    ///
    /// The floor under the sweep above, and a different job. This catches a wave that
    /// has been flattened, a species whose poses were never varied, and any future
    /// edit that makes `rasterise` ignore `pose`.
    ///
    /// **What this deliberately does not do**, because it is worth being explicit
    /// about a test that does not catch the thing its neighbours' docs mention: it
    /// loops `0..POSES`, so with `POSES = 2` it compares only poses 0 and 1, finds
    /// them different, and **passes**. It cannot catch a `POSES` that was left at 2.
    /// That is `the_beat_sweeps_rather_than_snapping_between_two_extremes`'s job, and
    /// it got it because the two have genuinely different strengths.
    #[test]
    fn no_two_poses_are_the_same_picture() {
        for species in SPECIES {
            let sets: Vec<_> = (0..POSES).map(|p| rasterise(species, p)).collect();
            for a in 0..POSES {
                for b in (a + 1)..POSES {
                    let same = (0..sets[a].height()).all(|y| {
                        (0..sets[a].width())
                            .all(|x| sets[a].dot(x, y) == sets[b].dot(x, y))
                    });
                    assert!(
                        !same,
                        "{}: pose {a} and pose {b} are the same picture, so the beat \
                         has no motion in it at all",
                        species.name
                    );
                }
            }
        }
    }

    /// The bitmap is big enough for the fish, and the fish does not touch its edge.
    ///
    /// Clipping is silent -- a lost dorsal is a different animal from the one the
    /// table describes, and the difference is only visible if you know what the
    /// table said.
    #[test]
    fn nothing_is_clipped_by_its_own_bitmap() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            let (x0, x1, _y0, y1) = ink_extent(&fish);
            // The slack is on the **right and bottom**, because a shift re-reads
            // the master through a window moved right and down. Ink reaching the
            // left edge or the top is correct and expected -- that is the snout.
            assert!(
                x0 == 0 || x1 < w,
                "{}: its ink starts at {x0}, so the bitmap is wider than the fish",
                species.name
            );
            assert!(
                x1 + 1 < w,
                "{}: its ink ends at {x1} of {w} dots, so the one-dot horizontal \
                 shift phase has no slack and will clip the fish",
                species.name
            );
            assert!(
                y1 + 3 < h,
                "{}: its ink ends at {y1} of {h} dots, so the three-row vertical \
                 shift phases have no slack and will clip the fish",
                species.name
            );
        }
    }

    /// The mirroring is a mirroring.
    ///
    /// A hand-mirrored sprite has to be redrawn by hand every time the art changes,
    /// and when that was skipped in the crab the left-facing clap's first three
    /// lines came out byte-identical to the right-facing ones, so its claws and
    /// eyes were on the wrong side and only its legs were flipped. This is the
    /// guard for the fact that the aquarium *derives* it instead.
    #[test]
    fn a_mirrored_fish_is_the_same_fish_facing_the_other_way() {
        for species in SPECIES {
            let right = rasterise(species, 0);
            let left = right.mirrored();
            let (w, h) = (right.width(), right.height());
            let mut ink = 0usize;
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        right.dot(x, y),
                        left.dot(w - 1 - x, y),
                        "{}: row {y} is not mirrored at column {x}",
                        species.name
                    );
                    if right.dot(x, y) {
                        ink += 1;
                    }
                }
            }
            assert_eq!(
                ink,
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .filter(|(x, y)| left.dot(*x, *y))
                    .count(),
                "{}: mirroring changed how much ink there is",
                species.name
            );
        }
    }

    /// Every phase is the same fish, moved by a dot.
    ///
    /// A phase is a sub-cell offset, so the worst one may cost a dot column at the
    /// snout and three rows off the belly. More than that and a fish visibly loses
    /// a fin as it swims, which is worse than the judder the phases exist to fix.
    #[test]
    fn a_shifted_fish_loses_at_most_its_thin_edges() {
        for species in SPECIES {
            let base = rasterise(species, 0);
            let (w, h) = (base.width(), base.height());
            let base_ink = (0..h)
                .flat_map(|y| (0..w).map(move |x| (x, y)))
                .filter(|(x, y)| base.dot(*x, *y))
                .count();
            for py in 0..DOTS_Y {
                for px in 0..DOTS_X {
                    let shifted = base.shifted(px, py);
                    let (sw, sh) = (shifted.width(), shifted.height());
                    let ink = (0..sh)
                        .flat_map(|y| (0..sw).map(move |x| (x, y)))
                        .filter(|(x, y)| shifted.dot(*x, *y))
                        .count();
                    let kept = ink as f32 / base_ink as f32;
                    assert!(
                        kept > 0.80,
                        "{}: phase ({px},{py}) kept only {:.0}% of its ink \
                         ({ink} of {base_ink}); the sub-cell shift is cutting into \
                         the fish, not moving it",
                        species.name,
                        kept * 100.0
                    );
                }
            }
        }
    }

    /// A fish is lit from above.
    ///
    /// The whole reason a fish is not a silhouette, and it is a *direction*, not a
    /// range: the top of the body has to be brighter than the bottom. A fish whose
    /// ramp runs the other way is a fish lit from underneath, which reads as a
    /// fish lit from nowhere.
    ///
    /// Measured as the top and bottom quartiles of the flesh's shade rather than
    /// the means of the upper and lower halves. The halves version needed enough
    /// flesh on both sides of the spine to average, and the smallest species has
    /// three dots of body between a dorsal fin and an anal fin -- so it was
    /// measuring the two rows either side of a boundary and reporting a
    /// four-hundredth of a tone step as "lit from nowhere".
    #[test]
    fn a_fish_is_lit_from_above() {
        for species in SPECIES {
            let fish = rasterise(species, 0);
            let (w, h) = (fish.width(), fish.height());
            // The `dot` check is not optional. `part_at` reads a `Vec<Part>` whose
            // default is `Part::Flesh`, so every dot that was never drawn -- the
            // whole empty space around the fish -- also reads as flesh, and the
            // test was averaging the aquarium instead of the animal and reporting
            // a darkest quartile of 0.00.
            let mut shades: Vec<f32> = (0..w)
                .flat_map(|x| (0..h).map(move |y| (x, y)))
                .filter(|(x, y)| {
                    fish.dot(*x, *y) && fish.part_at(*x, *y) == Part::Flesh
                })
                .map(|(x, y)| fish.shade_at(x, y))
                .collect();
            assert!(
                shades.len() > 20,
                "{}: only {} dots of unadorned body",
                species.name,
                shades.len()
            );
            shades.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let quartile = (shades.len() / 4).max(1);
            let bottom: f32 =
                shades[..quartile].iter().sum::<f32>() / quartile as f32;
            let top: f32 = shades[shades.len() - quartile..].iter().sum::<f32>()
                / quartile as f32;
            assert!(
                top > bottom + 0.25,
                "{}: its darkest flesh averages {bottom:.2} and its lightest {top:.2}. \
                 A fish lit from below is a fish lit from nowhere.",
                species.name
            );
        }
    }

    /// The ramp is belly, flank, back -- three stops, in that order.
    ///
    /// A two-stop ramp from a dark belly to a lit back puts most of the fish in the
    /// middle and gives it no flank, and the flank is where a fish's colour lives.
    #[test]
    fn the_shading_ramp_has_a_flank() {
        let base = crossterm::style::Color::Rgb {
            r: 200,
            g: 120,
            b: 40,
        };
        let belly = shade_colour(base, 0.0);
        let low = shade_colour(base, 0.25);
        let flank = shade_colour(base, 0.5);
        let high = shade_colour(base, 0.75);
        let back = shade_colour(base, 1.0);
        // Monotonic in luminance all the way up.
        let lums = [belly, low, flank, high, back].map(palette::luminance);
        for pair in lums.windows(2) {
            assert!(
                pair[1] > pair[0],
                "the ramp is not monotonic: {lums:?} in OKLab luminance"
            );
        }
        // And the flank is a real colour in its own right, not a waypoint the ramp
        // passes through on its way to the back.
        assert_ne!(
            flank, back,
            "the flank and the back are the same colour, so the ramp has two stops \
             wearing three names"
        );
        assert!(
            perceptual_distance(flank, back) > 0.05,
            "the flank and the back are only {:.3} apart",
            perceptual_distance(flank, back)
        );
    }

    /// The species cover the depth range, and none of them is at the same depth.
    ///
    /// The `ants` mirrored-palette bug on the other axis: a categorical axis that
    /// has quietly become ordinal. If every species' preferred depth band overlaps
    /// every other's, the shoal is one band of fish and the tank has no volume in
    /// it. The *spread* of a fish's depth has to exceed the gap between the
    /// species bands, or the categories are not categories.
    #[test]
    fn the_species_occupy_separate_depth_bands_with_room_to_spread() {
        const SPREAD: f32 = 0.44;
        // Over **both** tables, and that is the substantive change. This test read
        // `SPECIES` alone, so when the tank became two media it was measuring the
        // spread of the two far species alone -- 0.45 to 0.55, a span of 0.10 --
        // and failed the "no volume in this tank" assertion while the tank
        // demonstrably had one. A test that only ever looks at one row of a table
        // cannot notice that the other rows exist.
        let mut bands: Vec<(&str, f32)> =
            sources().map(|s| (s.name(), s.depth())).collect();
        bands.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        for pair in bands.windows(2) {
            let gap = (pair[1].1 - pair[0].1).abs();
            assert!(
                gap < SPREAD,
                "{} and {} are {gap:.2} apart in depth and a fish's depth is \
                 jittered by {SPREAD}, so they overlap -- which is the point. But a \
                 gap *larger* than the spread would put each species in its own \
                 horizontal stripe and the tank would read as rows.",
                pair[0].0,
                pair[1].0
            );
        }
        // And they must span the range, or there is no depth to speak of.
        let (lo, hi) = (bands[0].1, bands[bands.len() - 1].1);
        assert!(
            hi - lo > 0.5,
            "the species' depths span only {hi:.2} - {lo:.2} = {:.2}; there is no \
             volume in this tank",
            hi - lo
        );
    }

    /// The medium split falls where the depth split is, and there is a reason.
    ///
    /// The character species are the *near* ones. That is not a taste decision and
    /// not an accident of which art was drawn first: a character fish is strokes
    /// and an eye, and you can only see an eye on something close, and a braille
    /// fish is a silhouette and braille's density is worth more at size. So the
    /// tables are not free to swap media, and this is what stops them doing it
    /// silently.
    #[test]
    fn the_character_species_are_the_near_ones() {
        for s in sources() {
            let (name, depth, chars) = (s.name(), s.depth(), s.is_chars());
            if chars {
                assert!(
                    depth < 0.35,
                    "{name} is drawn in characters but sits at depth {depth:.2}. \
                     Characters carry a face, and a face is only legible up close."
                );
            } else {
                assert!(
                    depth > 0.35,
                    "{name} is drawn in braille but sits at depth {depth:.2}. Braille \
                     is a silhouette, and a silhouette is all a distant fish has."
                );
            }
        }
    }

    /// The braille species are proportioned for **square** dots.
    ///
    /// The trap this replaces is the aspect caveat at the top of `render`: a
    /// *character* sprite is squashed, because a cell is twice as tall as it is
    /// wide, and the first aquarium compensated by drawing everything long and
    /// thin. A braille dot is **square** -- 2 across and 4 down in a cell of 1 by 2
    /// -- so the dot-grid aspect *is* the visual aspect and the caveat does not
    /// apply.
    ///
    /// Per species rather than one band for all of them, because a discus is
    /// circular and a neon is a spindle and a single number cannot describe both.
    /// The first version asserted `1.8..=3.4` for everything, which is what forced
    /// the discus to be drawn as a long oval -- a big tetra.
    #[test]
    fn the_dot_species_are_proportioned_for_square_dots() {
        /// Each species' own accepted width-to-height, and why.
        //
        // Calibrated against the measured ink extents, not chosen. Measured across
        // the **whole cycle** rather than at pose 0, because a travelling wave makes
        // the per-pose extent a property of the pose: the neon's four poses measure
        // 2.62, 3.00, 2.62 and 2.33, and the tetras 1.93, 2.25, 2.25 and 2.08. A
        // shape assertion that reads pose 0 is asserting about one frame of an
        // animation, and it moved when the wave did. The union over the cycle is
        // what the fish occupies in the tank and what has to fit the bitmap:
        // **21x9 = 2.33:1** for the neon and **27x14 = 1.93:1** for the tetra.
        //
        // The bands are wide enough to take the art and narrow enough that swapping
        // the two species' numbers would fail -- which is the point, since a discus
        // and a neon need different bands and one shared number is what made this
        // file draw bars. Both carry room around the measurement, because the union
        // moves when the wave amplitude does and a band two hundredths from its
        // measurement is a tripwire rather than a specification.
        const BANDS: &[(&str, f32, f32)] = &[
            // A neon tetra is a spindle, and a slenderer one than a tetra.
            // Measured 2.33.
            ("neon", 2.10, 2.70),
            // A tetra is deep-bodied and round-headed. Measured 1.93.
            ("tetra", 1.75, 2.20),
        ];
        for (i, species) in SPECIES.iter().enumerate() {
            // The union over every pose, not one frame of the beat.
            let (mut x0, mut y0) = (usize::MAX, usize::MAX);
            let (mut x1, mut y1) = (0usize, 0usize);
            for pose in 0..POSES {
                for (x, y) in rasterise(species, pose).raised_dots() {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
            let (w, h) = (
                rasterise(species, 0).width(),
                rasterise(species, 0).height(),
            );
            let ratio = (x1 - x0 + 1) as f32 / (y1 - y0 + 1) as f32;
            let (name, lo, hi) = BANDS[i];
            assert_eq!(name, species.name, "the band table is out of order");
            assert!(
                (lo..=hi).contains(&ratio),
                "{}: {w}x{h} dots of bitmap hold a {ratio:.2}:1 animal, outside \
                 {lo}-{hi}. One number for a round fish and a spindle is what made \
                 this file draw bars.",
                species.name
            );
        }
    }

    /// The character species are proportioned for a **squashed** cell, and this
    /// set is **five spindles**.
    ///
    /// A cell is one unit wide and two tall, so a sprite of `w` columns and `h`
    /// rows is `w` wide by `2h` tall on screen. A character fish is therefore
    /// *twice as tall as its own character count suggests*, and getting this wrong
    /// is what produced the long thin bars this effect was rewritten to escape.
    ///
    /// # One band for all five, and that is the finding
    ///
    /// The previous set had **three** bands on purpose: a round fish at 1.14, a
    /// tall one at 0.88, and a low one at 1.42, so the tank had shape variety and
    /// `no_two_species_are_the_same_animal` had something to measure. This set
    /// measures 1.58, 1.75, 1.90, **1.33** and 1.58 -- one band, five spindles, no
    /// round fish and no tall one. `fry` is the stubby one and the reason the band
    /// is 1.25 rather than 1.45.
    ///
    /// That is a fact about five drawings from one hand rather than a defect in
    /// them, and the honest response is a test that says so. The band is tight
    /// enough to reject a stubby or a stretched sprite, and it is *shared*, and
    /// the comment records why a shared band is the correct answer here rather than
    /// a loosened one.
    ///
    /// The character band also has to measure the **ink** extent rather than the
    /// padded grid: `stipple` is 20 columns of sprite and 19 of animal, and
    /// measuring the sprite read it as 1.67 when its shape is 1.58.
    #[test]
    fn the_character_species_are_all_spindles_of_the_same_proportion() {
        /// Visual width-to-height, i.e. `cols / (rows * 2)`, for **every** species.
        const BAND: (f32, f32) = (1.25, 2.00);
        let mut measured: Vec<(&str, f32)> = Vec::new();
        for s in CHAR_SPECIES {
            let a = crate::aquarium::charart::species(s);
            let (x0, x1, y0, y1) = char_ink_extent(&a, 0);
            let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
            let ratio = w as f32 / (h as f32 * 2.0);
            assert!(
                (BAND.0..=BAND.1).contains(&ratio),
                "{}: {w} columns by {h} rows of ink is {ratio:.2}:1 once the cell's \
                 2:1 height is accounted for, outside {:.2}-{:.2}. This set is five \
                 spindles and this is the band that says so.",
                s.name,
                BAND.0,
                BAND.1
            );
            measured.push((s.name, ratio));
        }
        // And the spread is bounded, so the shared band is not hiding a species
        // that does not belong. Measured 1.33 to 1.90; the bound is 0.65, which is
        // most of the band, and that is the honest state of it: **this is a narrow
        // set of proportions and the test says so rather than pretending
        // otherwise.** If a future drawing lands outside, it is a different animal
        // and the set wants reconsidering rather than a wider band.
        let lo = measured.iter().map(|(_, r)| *r).fold(f32::MAX, f32::min);
        let hi = measured.iter().map(|(_, r)| *r).fold(f32::MIN, f32::max);
        assert!(
            hi - lo < 0.65,
            "the set's aspect spread is {:.2}, from {lo:.2} to {hi:.2} in {measured:?}. \
             A band wide enough for that is not a band.",
            hi - lo
        );
    }

    /// **No two species are the same animal.**
    ///
    /// The test round one needed and could not write, because the old art was five
    /// shapes and every assertion about it was about *one species at a time*. A
    /// fish that tapers to its tail, a fish with the right aspect ratio and a fish
    /// with an eye all pass in isolation while being the same drawing three times,
    /// and the replacement resamples each species' ink profile to a common width
    /// and requires the pairs to differ. It failed against the old art and it is
    /// about forty lines.
    ///
    /// **And it has to work across the two media**, which is the new problem. The
    /// species are not all drawn the same way, so a comparison in raw pixels would
    /// be a comparison of a 30x25 dot bitmap against a 19x6 grid of characters and
    /// would say nothing. So both are resampled to the same square grid first, and
    /// what is compared is *where the animal's mass sits inside that grid*.
    ///
    /// # The grid is COMMON, and that is the second fix
    ///
    /// The first version normalised every species by **its own** ink extent. That
    /// asks the right question -- "what shape is this animal" -- and it worked on a
    /// set with a round fish and a tall one, because there the shape *was* the
    /// difference. **It does not work on five spindles.** Normalising by the ink
    /// extent throws the aspect away, so two fish a tenth apart in proportion
    /// resample to nearly the same grid: `longfin` against `bigeye` came out at
    /// 0.025, inside the threshold, and the test was asserting nothing about the
    /// pair it was written for.
    ///
    /// So the ink is laid into one shared grid **at its natural proportion** rather
    /// than stretched to fill it. Same grid, same cell count, and the aspect is now
    /// part of what is compared. See [`silhouette`].
    ///
    /// # Two assertions, because a low floor is not enough
    ///
    /// [`MIN_DISTINCT`] has to be low on this set (see its comment), and a low
    /// floor is exactly the kind of thing that lets a set of near-identical animals
    /// pass. So the **median** pair has to clear a much higher bound, and that is
    /// the assertion that says the set as a whole is a shoal.
    #[test]
    fn no_two_species_are_the_same_animal() {
        let profiles: Vec<(&str, Vec<f32>)> =
            sources().map(|s| (s.name(), silhouette(s).0)).collect();
        assert_eq!(
            profiles.len(),
            SPECIES.len() + CHAR_SPECIES.len(),
            "the tank's species list changed shape"
        );
        let mut every: Vec<f32> = Vec::new();
        for (i, (name_a, a)) in profiles.iter().enumerate() {
            for (name_b, b) in profiles.iter().skip(i + 1) {
                let difference =
                    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>()
                        / a.len() as f32;
                assert!(
                    difference > MIN_DISTINCT,
                    "{name_a} and {name_b} differ by only {difference:.4} of mean \
                     density once both are resampled to {GRID}x{GRID}. Two species \
                     that are the same drawing twice is not a shoal, it is a \
                     pattern."
                );
                every.push(difference);
            }
        }
        every.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = every[every.len() / 2];
        assert!(
            median > MEDIAN_DISTINCT,
            "the median pair differs by only {median:.4} over {} pairs. Every pair \
             clears a floor of {MIN_DISTINCT} and the set is still one animal.",
            every.len()
        );
    }

    /// **The silhouette comparator can tell two copies of one species apart from
    /// two species.**
    ///
    /// The obvious way to check a metric is to make the thing it measures go
    /// wrong, and that is a two-line edit to the species table which turns out to
    /// be surprisingly hard to make without breaking the file -- an angelfish
    /// whose `frames` are the discus's is a duplicate struct field, not a
    /// duplicate drawing. So the sensitivity is asserted directly instead, and
    /// permanently.
    ///
    /// This is the test that stops [`no_two_species_are_the_same_animal`] from
    /// being vacuous. A mean-absolute-difference threshold on a blurred 32x32
    /// grid is a number somebody could have invented and never checked, and an
    /// assertion that passes for *any* pair of inputs measures nothing. Two copies
    /// of the same species must score **zero** difference, and the tightest real
    /// pair in the tank must clear the threshold the test above enforces -- so
    /// both ends of the scale are pinned.
    #[test]
    fn the_silhouette_comparator_can_tell_one_species_from_two() {
        let profiles: Vec<(&str, Vec<f32>)> =
            sources().map(|s| (s.name(), silhouette(s).0)).collect();
        let difference = |a: &[f32], b: &[f32]| {
            a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>()
                / a.len() as f32
        };
        for (i, (name, a)) in profiles.iter().enumerate() {
            for (other, b) in profiles.iter().skip(i + 1) {
                assert!(
                    difference(a, b) > MIN_DISTINCT,
                    "{name} and {other} differ by {:.4}, which is inside the \
                     threshold of {MIN_DISTINCT}, so the real pairs have nothing to \
                     clear",
                    difference(a, b)
                );
            }
        }
        // The whole point: identical input, zero difference. If this is not zero
        // then `silhouette` is not a function of its species, and the comparison
        // above is comparing noise.
        let (name, a) = &profiles[0];
        let again = silhouette(sources().next().unwrap()).0;
        assert_eq!(
            *name, profiles[0].0,
            "the same species produced two different names"
        );
        assert!(
            difference(a, &again) < f32::EPSILON,
            "the same species silhouettes to {:.6} of difference. `silhouette` is \
             not a function of its species.",
            difference(a, &again)
        );
        // And the resample is not degenerate: a flat field and a full field are
        // maximally different, which is the upper bound the pairs above sit under.
        let (name, a) = &profiles[0];
        let ones = vec![1.0f32; a.len()];
        let mean = a.iter().sum::<f32>() / a.len() as f32;
        let flat = vec![mean; a.len()];
        assert!(
            difference(&flat, &ones) > 0.04,
            "a flat {name} and a full one differ by only {:.4}, so the metric's \
             whole range is below its own threshold",
            difference(&flat, &ones)
        );
    }

    /// A species' ink, resampled to a `GRID x GRID` density map, and the visual
    /// aspect it was drawn at.
    ///
    /// Both media go through this, which is the only way the comparison means
    /// anything. The medium's own resolution is preserved inside the resample --
    /// braille samples its dots, characters their glyphs -- because squaring a dot
    /// fish into character cells before measuring would throw away the only thing
    /// that makes braille worth using.
    ///
    /// # The box is COMMON, and that is the whole fix
    ///
    /// The first version normalised every species by **its own** ink extent, which
    /// asks the right question -- "what shape is this animal" -- and on the previous
    /// set of drawings it worked, because that set had a round fish and a tall one
    /// and the shape *was* the difference.
    ///
    /// **This set is five spindles**, 1.33 to 1.90, and normalising by the ink
    /// extent throws the aspect away, so two fish 0.2 apart in proportion resample
    /// to nearly the same grid. Measured: `longfin` against `bigeye` differ by
    /// **0.025** of mean density, which is inside the threshold, so the test could
    /// not tell two different drawings apart and was asserting nothing.
    ///
    /// So the ink is laid into one shared square grid **at its natural proportion**
    /// rather than stretched to fill it: a 1.9:1 fish spans nearly the whole width
    /// and half the height, a 1.33:1 fish two thirds of the width and three
    /// quarters of it. Same grid, same cell count, and the aspect is now part of
    /// what is being compared. It is a weaker statement than "these are different
    /// animals" in the abstract and a *much* stronger one in practice, because it is
    /// the difference between a test that discriminates and one that does not.
    fn silhouette(s: Source<'_>) -> (Vec<f32>, f32) {
        let n = GRID * GRID;
        #[allow(clippy::needless_late_init)]
        let marks: Vec<(f32, f32)>;
        let (ink_w, ink_h) = match s {
            Source::Dots(sp) => {
                let fish = rasterise(sp, 0);
                let (x0, x1, y0, y1) = ink_extent(&fish);
                let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
                marks = fish
                    .raised_dots()
                    .map(|(dx, dy)| {
                        ((dx - x0) as f32 / w as f32, (dy - y0) as f32 / h as f32)
                    })
                    .collect();
                (fish.cells_wide(), fish.cells_tall())
            }
            Source::Chars(sp) => {
                let a = crate::aquarium::charart::species(sp);
                let (x0, x1, y0, y1) = char_ink_extent(&a, 0);
                let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
                marks = a
                    .cells(0)
                    .map(|(x, y, _, _)| {
                        let (dx, dy) = (x as usize - x0, y as usize - y0);
                        (dx as f32 / w as f32, dy as f32 / h as f32)
                    })
                    .collect();
                (a.cells_wide(), a.cells_tall())
            }
        };
        // The *visual* aspect, `cols / (rows * 2)`, in both media: a cell is one
        // unit wide and two tall whichever glyph it holds.
        let aspect = ink_w as f32 / (ink_h as f32 * 2.0);
        let mut grid = vec![0.0f32; n];
        for (fx, fy) in marks {
            // x scaled by the proportion, so a wide animal is a wide animal here.
            let gx =
                ((fx * aspect / ASPECT_NORM * GRID as f32) as usize).min(GRID - 1);
            let gy = ((fy * GRID as f32) as usize).min(GRID - 1);
            grid[gy * GRID + gx] = 1.0;
        }
        // One box blur, so the comparison is of masses rather than of which
        // integer a dot happened to land in.
        let mut out = vec![0.0f32; n];
        for y in 0..GRID {
            for x in 0..GRID {
                let mut sum = 0.0;
                let mut count = 0.0;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if (0..GRID as i32).contains(&nx)
                            && (0..GRID as i32).contains(&ny)
                        {
                            sum += grid[ny as usize * GRID + nx as usize];
                            count += 1.0;
                        }
                    }
                }
                out[y * GRID + x] = sum / count;
            }
        }
        (out, aspect)
    }

    /// The character sprite's own ink extent, in cells.
    fn char_ink_extent(
        a: &crate::aquarium::charart::CharArt,
        pose: usize,
    ) -> (usize, usize, usize, usize) {
        let (w, h) = (a.cells_wide(), a.cells_tall());
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0usize, h, 0usize);
        for (x, y, _, _) in a.cells(pose) {
            let (x, y) = (x as usize, y as usize);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        (x0, x1, y0, y1)
    }

    /// **The eye is visible, which is a claim about the render and not about the
    /// art.**
    ///
    /// The braille species had an eye the entire time this crate existed: the part
    /// map said `O`, the shade map said `0.03`, and there is a test asserting a
    /// fish has an eye. None of it reached the terminal. A braille cell is eight
    /// dots and **one colour**, so a one-dot pupil is averaged with the body dots
    /// sharing its cell and comes out a shade or two off the flank -- nothing. The
    /// fish were blobs with an eye in the source code.
    ///
    /// So the assertion is on [`FishArt::cells`], which is the resample, and it
    /// asks the only question that matters: **is the cell carrying the eye
    /// measurably darker than the flank?** A number in the data is not a thing the
    /// user can see, and a test about the data cannot tell the two apart. This is
    /// the dot-resolution twin of `charart`'s
    /// `a_character_fish_is_drawn_and_not_stamped`, and both exist because a mark
    /// that is too small for its medium is invisible in a way that is perfectly
    /// well-formed in the source.
    #[test]
    fn the_eye_reaches_the_terminal() {
        for species in SPECIES {
            for pose in 0..POSES {
                let fish = rasterise(species, pose);
                // The eye's dots, mapped to the **cells** they land in. This is
                // the step that is easy to get wrong and it is the same class of
                // bug as `FishArt::cells` yielding dot offsets once: `part_at` is
                // a dot accessor and `cells()` yields cell offsets, so feeding one
                // to the other finds no eye at all.
                let eye_cells: std::collections::BTreeSet<(usize, usize)> = (0
                    ..fish.width())
                    .flat_map(|dx| (0..fish.height()).map(move |dy| (dx, dy)))
                    .filter(|(dx, dy)| fish.part_at(*dx, *dy) == Part::Eye)
                    .map(|(dx, dy)| (dx / DOTS_X, dy / DOTS_Y))
                    .collect();
                assert!(
                    !eye_cells.is_empty(),
                    "{} pose {pose} has no eye dots at all",
                    species.name
                );
                let mut eye_shade = f32::MAX;
                let mut flank_shade = 0.0f32;
                let mut flank_cells = 0usize;
                for (x, y, _glyph, shade) in fish.cells() {
                    let key = (x as usize, y as usize);
                    if eye_cells.contains(&key) {
                        eye_shade = eye_shade.min(shade);
                    } else {
                        flank_shade += shade;
                        flank_cells += 1;
                    }
                }
                let flank = flank_shade / flank_cells.max(1) as f32;
                assert!(
                    eye_shade < f32::MAX,
                    "{}: {} eye dots fall outside every cell the fish draws",
                    species.name,
                    eye_cells.len()
                );
                assert!(
                    eye_shade < flank - 0.25,
                    "{}: the darkest cell on its eye resamples to {eye_shade:.2} \
                     against a flank of {flank:.2}. The eye is in the part map and \
                     the shade map and it is not on the screen: a braille cell is \
                     eight dots and one colour, so a pupil smaller than a cell is \
                     averaged away.",
                    species.name
                );
            }
        }
    }

    /// A hostile species does not panic and does not draw nothing.
    ///
    /// Every clamp exercised with the values a hand-edited table would contain.
    /// The crate's rule is that a clamp no test exercises is a clamp nobody has
    /// shown to be ordered.
    #[test]
    fn a_hostile_species_renders_rather_than_panicking() {
        for hostile in [
            Species {
                length: 0,
                girth: 0.0,
                peak: 0.0,
                blunt: 0.0,
                taper: 0.0,
                dorsal: &[],
                anal: &[],
                caudal: (1.0, 0.0, TailFork::Rounded),
                markings: Markings::Bars {
                    count: 0.0,
                    from: 1.0,
                    to: 0.0,
                    strength: 2.0,
                },
                ..SPECIES[0]
            },
            Species {
                length: 4,
                girth: f32::NAN,
                peak: 1.0,
                blunt: -1.0,
                taper: f32::INFINITY,
                dorsal: &[(0.0, f32::NAN)],
                caudal: (0.0, 1e9, TailFork::Forked { depth: 1e9 }),
                ..SPECIES[0]
            },
            Species {
                markings: Markings::Mottled { amount: 5.0 },
                barbels: true,
                ..SPECIES[0]
            },
        ] {
            let fish = rasterise(&hostile, 0);
            // No panic is the assertion; the shape is not, because a species with
            // `girth: NaN` has no shape to assert about.
            let (w, h) = (fish.width(), fish.height());
            assert!(w > 0 && h > 0);
            let _ = fish.cells().count();
            let _ = fish.mirrored().cells().count();
            let _ = fish.shifted(1, 3).cells().count();
        }
    }

    /// The bounding box of a fish's ink.
    fn ink_extent(fish: &FishArt) -> (usize, usize, usize, usize) {
        let (w, h) = (fish.width(), fish.height());
        let (mut x0, mut x1, mut y0, mut y1) = (w, 0usize, h, 0usize);
        for y in 0..h {
            for x in 0..w {
                if fish.dot(x, y) {
                    x0 = x0.min(x);
                    x1 = x1.max(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        (x0, x1, y0, y1)
    }
}
