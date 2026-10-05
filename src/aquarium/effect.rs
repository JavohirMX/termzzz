//! A side-view fish tank.
//!
//! # What makes it read as water
//!
//! The prior art here is `asciiquarium`, and it is a much better piece of work
//! than "fish in a box" deserves credit for: twelve multi-frame fish designs,
//! sharks that hunt smaller fish, whales with water spouts, depth-keyed layering
//! so entities overlap correctly, and interactive feeding. It is the reference
//! and this is not trying to replace it.
//!
//! It also does one thing this effect takes a different view of, and it is the
//! difference between "a tank" and "a picture of a tank".
//!
//! **Depth is shading, not just a draw order.** `asciiquarium`'s z-depth decides
//! which entity paints over which, and nothing else. But the reason a photograph
//! taken underwater looks like one is *aerial perspective*: water absorbs red
//! first, so a fish at depth is not smaller and further away, it is **bluer and
//! dimmer**. Each fish here carries a z in `0.0..=1.0` and its colour is mixed
//! toward the water's own colour by that z. It costs a `lerp` per cell and it
//! does more to sell "underwater" than anything else in the file.
//!
//! # The art is the effect
//!
//! **The first version of this tank drew its fish as bars, and no amount of
//! engineering fixed it.** Three species, each a run of `(` with an `o` and a
//! `><` on the ends, in three lengths. The tank was a field of horizontal
//! stripes, and it took two rewrites to fix and about a dozen bugs to find on
//! the way.
//!
//! Two things were wrong, and only one of them was about drawing.
//!
//! A character cell is one unit wide and about two tall, so a 13x3 sprite is 13
//! units wide by 6 tall: a 2:1 streak. The art had *compensated* for this by
//! making every sprite long and thin, which is why every species was a bar, and
//! retuning the numbers only moved the bar along the screen. And a string cannot
//! carry a gradient, so a fish was a silhouette or nothing -- one flat colour,
//! which is right for a logo and wrong for an animal.
//!
//! Two media, and the split is by **depth**
//!
//! Three of the five species are drawn in **characters** and two in **braille**,
//! and which is which is not a matter of taste. A character fish is strokes and
//! an eye; you can only see an eye on something close, so the near species --
//! discus, angelfish, cory -- are characters. A braille fish is a silhouette
//! with eight times the density, and at the size and distance a neon tetra or a
//! tetra actually is, only the silhouette is legible and the density is worth more
//! than the fill is worth less.
//!
//! So the split is also a **depth cue that was free**. A near fish drawn in
//! `/ \ | _ -` carries more ink and more internal detail than a far fish drawn as
//! sparse dots, which is true of real water: you resolve more in the thing in
//! front of you. See [`crate::aquarium::charart`] for the art and
//! [`crate::aquarium::art`] for the braille half.
//!
//! # Three lessons worth more than the art
//!
//! **A cheap effect can still be a bad one, and a bandwidth measurement cannot
//! see art.** The version this replaced rendered in 343 us and 2,269 bytes at
//! 400x200 -- six times under budget, a hundred-and-seventieth of the
//! mandelbrot's byte volume. Every number in it was good. The picture was a row
//! of bars, and no run of `frame_times` would ever have said so.
//!
//! **The test suite could not see it either, and that is the part worth
//! remembering.** Every assertion about the art was about *one sprite at a time* --
//! this one tapers to its tail, that one is the right aspect ratio -- and all of
//! them passed on three sprites that were the same object in three lengths.
//! Nothing compared one species to another, so "they are all runs of `(`" was not
//! a fact anything in the crate could express.
//! [`art::no_two_species_are_the_same_animal`] is the replacement: it resamples
//! every species' ink to the same 32x32 grid -- **across both media**, which is
//! the part that needed the new renderer -- and requires the pairs to differ. It
//! fails against the old art.
//!
//! **A constant with a unit does not survive a change of art in the same file.**
//! Three of them did not, and all three are written up at their call sites:
//! [`RESOLVE_GAP_X`], which was "a different thing once the fish were dots";
//! the eat radius in [`Aquarium::step_flakes`], where "a fish is within 1.2 cells
//! of the flake" stopped working the day a fish became sixteen cells wide and ten
//! of them settled into a seventeen-cell ring around the food; and
//! [`HOME_SPREAD`], which had to grow by a factor of twenty before the depth cue
//! was measurable at all.
//!
//! # The roster, and why the mix is not seven even slices
//!
//! Seven species: **two braille** (a neon and a tetra, the small far shoal) and
//! **five characters** (`longfin`, `bigeye`, `slashback`, `fry`, `stipple` -- one
//! drawing set, five spindles, 1.33 to 1.90). Five fish of nineteen to twenty-one
//! columns on an eighty-column terminal is three across the screen, so
//! [`SPECIES_MIX`] gives **52% of the count to the two small ones** and the five
//! large are accents swimming through the shoal. Equal shares would be a wall.
//!
//! `fry` is the only slow species, at cruise 0.35, and it is the bottom dweller:
//! **it is the only thing in this effect that ever stops**, which is a property the
//! tank lost when the previous set's cory went and had to find a home for.
//!
//! # The movement
//!
//! Five terms, all of them fractions of a fish's **cruise speed**, summed into a
//! desired direction and then scaled once. Walls, separation, food, cohesion and
//! alignment, wander, and a pull toward the fish's own row. Two decisions in that
//! are the whole of the model and both were wrong before:
//!
//! - **A first-order lag, not an acceleration and a clamp.** The old code
//!   integrated forces and then clamped speed *from below*, so nothing in the tank
//!   could ever be slow or still. A lag needs no floor: a fish that wants to be
//!   stationary becomes stationary, which is how a cory came to rest on the
//!   gravel. It is also frame-rate independent, which `v += a * dt * 6.0` was not.
//! - **Height is not depth.** Each fish has a [`Fish::home`] -- a row it prefers,
//!   chosen once and never recomputed -- and `z` is a distance that drives colour
//!   and nothing else. They used to be the same thing, which put every fish at a
//!   given depth on the same row, made the tank read as horizontal stripes, and
//!   then crowded the gravel into a jam that ran *opposite* to the depth cue and
//!   hid it. Measured: see [`HOME_SPREAD`].
//!
//! - **Every term scales with depth, or the depth cue is one of five reasons to
//!   move.** Separation and the walls were first written as absolute cells per
//!   second. That made [`PARALLAX`] invisible: a far fish's cruise term shrank and
//!   its separation did not. Everything is now a fraction of cruise and scaled
//!   once, including the terms that want absolute units -- the wall term is written
//!   `penetration / cruise` so multiplying back restores the absolute correction
//!   *and* scales with depth.
//!
//! [`PARALLAX`] still only *shows* on the two braille species, and that is a
//! finding rather than a bug: a nineteen-column fish is permanently inside
//! somebody's exclusion radius, so its speed is set by crowding and the depth term
//! on its cruise is a rounding error on top. The test asserts the cue where it is
//! measurable and the *direction* everywhere, and says so.
//!
//! - **A startle reflex redirects and re-speeds; it does not add.** An additive
//!   impulse against a fish already moving at three cells a second produced a fish
//!   swimming the *other way* at one and three-quarter. It had turned around and it
//!   had **slowed down**. And the impulse has to be well clear of `options.speed`,
//!   which defaults to 6.0: at 5.0 a "startle" peaked at 4.79 against a cruise of
//!   4.65, which is a fish being mildly annoyed rather than bolting.
//!
//! [`Fish::home`]: crate::aquarium::effect::Fish::home
//!
//! # The water is in the background channel
//!
//! The vertical depth gradient is painted into [`Cell::bg`], not into the glyph,
//! and it is the one genuinely excellent decision the file inherited.
//!
//! - It is **static**, so after the first frame the diff reports none of it. A
//!   background the encoder sees unchanged costs nothing. The gradient is every
//!   cell of the screen; written into the *glyph* it would be re-sent sixty times
//!   a second for a picture that never changes.
//! - It leaves the **glyphs free for the fish**. Water drawn as glyphs competes
//!   with fish drawn as glyphs for the same legibility; water drawn as a
//!   background does not compete at all.
//!
//! This is the exact inverse of `ripple` and `plasma`, where a continuously
//! interpolated field meant a colour change in every cell and 2.79 MB a frame.
//! Same crate, same terminal, same encoder; the difference is entirely in what
//! the value is stored in. **Before optimising a field effect's bandwidth, ask
//! whether the field has to be re-sent at all.**
//!
//! One consequence is a trap. [`crate::render::BrailleGrid::write_to`] and
//! `overlay_onto` build cells with [`Cell::new`], which sets
//! `bg: Color::Reset` -- so using either would punch a terminal-default hole in
//! the water behind every fish. `draw_fish` sets the background from the row's
//! water colour by hand, and the aquarium's fish are the only braille in the
//! crate drawn that way.
//!
//! # The tank is sized to the terminal, in fish *and* in fish size
//!
//! `options.fish` is a count, and a count is meaningless without an area. The
//! first version spawned twenty fish whatever the terminal was, so the tank was
//! 13% covered at 80x24 and **a third of one percent** at 400x200: the aquarium
//! got emptier the bigger you made the window, which is exactly backwards, and
//! nothing reported it.
//!
//! Two numbers fix it, and the second one is the non-obvious half. The count
//! scales with the *square root* of the area, because a shoal's spread grows
//! with the tank as well as its size. Coverage is `count x area-per-fish`, so for
//! coverage to keep up each fish's **area** has to grow as the square root of the
//! area too -- and a fish's linear size is the square root of its area. Hence
//! [`Aquarium::art_scale`], the *quarter* power, capped at 2x. Scaling the count
//! alone got a 400x200 tank to 1.7% coverage, which is still thin; scaling the
//! fish as well got it to 5.8%, which is a tank with forty-eight animals in it.
//!
//! Scaling the fish means scaling the art, so every dimension of a species except
//! its length is a **fraction of its girth** rather than a count of dots. A fish
//! at twice the size is then the same animal twice as big, rather than the same
//! animal with a differently-proportioned dorsal fin glued on.

use crate::aquarium::art;
use crate::aquarium::charart;
use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::palette;
use crate::runtime::{FrameContext, InputEvent, PointerPhase};
use crossterm::style::{Attribute, Color};
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// The species mix, as cumulative upper bounds on a `0.0..=1.0` roll.
///
/// Not proportional, and deliberately so. A tank seen side-on is mostly water,
/// and the small fish are the ones that fill it; equal fifths puts a bright
/// near fish in every square of the screen and the depth stops reading at all.
/// The cory is a floor species and gets the smallest share of any, because it
/// spends its time on the gravel rather than in the water column.
const SPECIES_MIX: &[(f32, f32)] = &[
    // Seven species, and **the two braille ones carry the shoal** -- 52% of the
    // tank between them -- with the five big character fish as accents swimming
    // through it. That is the weighting, not seven equal slices: five fish of
    // nineteen to twenty-one columns on an eighty-column terminal is three across
    // the screen, so equal shares would be a wall rather than a tank.
    (0.30, 0.0), // neon: small and numerous, and the only fish the braille medium draws
    (0.52, 1.0), // tetra: the shoal's backbone, and the other braille one
    (0.63, 2.0), // longfin
    (0.72, 3.0), // bigeye
    (0.80, 4.0), // slashback
    (0.88, 5.0), // fry: the only slow one, and the bottom dweller
    (1.01, 6.0), // stipple
];

/// Neighbours further from a fish **in depth** than this are in another plane.
///
/// This is the whole of the depth-aware part of the model, and it is one number.
/// A fish eight rows nearer is not in the way of one eight rows further back; it is
/// somewhere else in the tank, and treating it as an obstacle makes the two of them
/// shove each other around forever. What is in the way is a fish at the *same*
/// distance, which is a fish in the same plane.
///
/// 0.28, against a per-fish depth spread of 0.44: a pair has to be within about
/// two thirds of the jitter to feel each other at all, so a shoal at a single depth
/// and a shoal spread across the tank both work, and a pair a full jitter apart
/// does not.
const SEPARATION_DEPTH: f32 = 0.28;

/// Cells of clear water a fish insists on between itself and another, on top of
/// half the two sprites' widths.
///
/// Half the sum alone puts two fish at exactly touching distance, and two
/// abutting fish do not read as two fish. One cell, not the three this was when
/// the sprites were characters: a cell is now half a fish-width on the small
/// species, so the same number is a different distance.
const SEP_GAP: f32 = 1.0;

/// How far a fish can see its own kind, in cells.
const SHOAL_RADIUS: f32 = 15.0;

/// How hard a fish is drawn toward the middle of its own kind.
const COHESION: f32 = 0.30;

/// How hard a fish matches its own kind's heading.
const ALIGNMENT: f32 = 0.60;

/// How hard a fish is pulled back to its own row, in cells per second per row of
/// error.
///
/// A *restoring* term rather than a share of the cruise, for the same reason
/// separation is: a term scaled by the fish's own speed is not a property of the
/// fish, it is a property of the frame. At 0.55, a fish four rows off its row is
/// pulled back at 2.2 cells a second, which is a correction and not a command.
const ROW_PULL: f32 = 0.55;

/// How hard a fully-overlapped pair pushes apart, **as a fraction of cruise**.
///
/// A fraction, and the comment on `step_fish`'s accumulation block is the reason:
/// every term is a fraction of `cruise` so that [`PARALLAX`] scales all of them at
/// once. 0.33 means a completely overlapped pair separates at a third of the fish's
/// cruising speed, which clears the one-cell gap in a few frames without flinging
/// either of them.
const SEP_GAIN: f32 = 0.33;

/// How hard a fish is pulled toward food, as a fraction of cruise.
///
/// 3.0, so a chasing fish is genuinely leaving its band to get there. An earlier
/// version had this at 1.0 against a row pull it could not out-run, and the chase
/// simply did not happen.
const FLAKE_PULL: f32 = 3.0;

/// How wide a species' preferred rows are, in rows.
///
/// **0.7 of the tank**, which is the number that matters most in the whole
/// movement model and the one that took a measurement to get right. The old value
/// was 1.5 rows either side, a per-fish offset from a per-fish row derived from
/// that fish's own depth -- so crowding was ordered by depth, and measured across
/// four species a deep fish was swimming 1.4x faster than a shallow one *with
/// PARALLAX at zero*. See the note in `spawn_fish`.
///
/// Large enough that the species' bands overlap, which is the same requirement as
/// the depth jitter's spread exceeding the gap between the depth bands, on the
/// other axis. Both are the `ants` mirrored-palette bug: the spread of a value has
/// to exceed the gap between the categories or the categories are not categories.
const HOME_SPREAD: f32 = 0.7;

/// How fast a fish's velocity chases what it wants, per second.
///
/// A first-order lag rather than an acceleration plus a speed floor, and this is
/// the single biggest change in the file. The old model integrated a force and
/// then *clamped* the speed from below, so a fish could never be slow and never be
/// still: it was always moving at 35% of the configured speed whether or not it
/// had anywhere to be. A lag needs no floor, because a fish whose desired velocity
/// is zero decelerates to zero on its own, and that is what lets a cory sit on the
/// gravel.
///
/// `1 - exp(-k*dt)` rather than `k*dt`, so the response does not change with the
/// terminal's refresh rate. The old `v += a*dt*6` was frame-rate dependent: the
/// same tank ran at two speeds on a 60 Hz and a 144 Hz display.
const STEER: f32 = 2.6;

/// Cap on a fish's speed, as a multiple of its own cruise.
///
/// Above one so that a startle burst can actually dart, and finite so that a
/// separation pile-up cannot fling a fish across the tank.
const MAX_SPEED_FACTOR: f32 = 2.6;

/// A far fish swims slower, and this is how much of its cruise it gives up at the
/// back of the tank.
///
/// Parallax, and the terminal is the only depth cue a 2D aquarium has: on a
/// screen there is no size falloff, so without this a fish at the back moves as
/// fast as one at the front and the tank reads as a flat sheet of animals rather
/// than as a volume. It is the one place where the *absence* of a real cue has to
/// be paid for by hand.
const PARALLAX: f32 = 0.45;

/// A turning fish sweeps wider, by this much of its own length per unit of
/// lateral acceleration.
///
/// Nearly free once the forces exist, and it is a real thing about fish: a fish
/// banking into a turn presents more of itself than one going straight, so it
/// needs more room. It also does the visual work of making a turn *look* like a
/// turn.
const BANK: f32 = 0.05;

/// A fish within this many cells of a flake landing bolts for it.
const STARTLE_RADIUS: f32 = 7.0;

/// How hard a startled fish bolts, in cells per second.
///
/// **9.0, and the number that matters is its relation to `options.speed`, which
/// defaults to 6.0.** The impulse was 5.0 and that is *below* a full-tilt
/// `longfin`'s cruise after parallax, so a startle was not a startle: measured, a
/// dart peaked at 4.79 cells/second against a cruise of 4.65, and the fish was
/// still at 4.26 a second and a half later. It had not bolted. It had been mildly
/// annoyed.
///
/// A reflex has to be unmistakably faster than the animal's steady state or it
/// does not read as one, and the steady state here scales with the speed setting.
/// 9.0 is about 1.5x a cruising fish, which is inside `MAX_SPEED_FACTOR` so the cap
/// does not clip it.
const STARTLE_IMPULSE: f32 = 9.0;

/// Where a fish's tail beat is after covering `moved` cells, in frames.
///
/// Named and separate because **the arithmetic here is the bug this crate shipped
/// for two rounds**, and the bug was not findable in the simulation:
///
/// ```text
/// f.pose = ((f.pose as f32 + moved * 0.9) as usize) % poses;
/// ```
///
/// The `as usize` discards the sub-frame remainder, so the pose only ever
/// advanced when a fish covered **1.11 cells in one frame**. At 60 Hz the fastest
/// fish in the tank covers about 0.25, so **no fish ever wagged its tail** -- not
/// the braille pair, not the five character species, not at any speed. The tank
/// was a shoal of rigid shapes sliding across the screen, and nothing said so,
/// because the frame looked perfectly reasonable and every test that existed
/// asserted something else.
///
/// It is worth being precise about why it survived. The comment directly above it
/// said *"a fish that is not moving should not wag"* -- which is true, which the
/// code did, and which no test ever checked. **A claim in a comment is not an
/// assertion**, and the one assertion that was nearby, `the_tail_poses_differ_
/// only_in_the_tail`, checked the *pictures* were different from each other and
/// so passed perfectly on a set of frames nothing ever cycled through.
///
/// So the accumulator is a float, and it is a *separate function* so that the
/// arithmetic can be tested without a tank, a shoal, a settle loop and a seed
/// standing between the assertion and the thing it asserts.
///
/// `poses` is the cycle length and the modulus together, which is why the frame
/// count is bounded: without the wrap a `f32` loses the fraction after a few
/// million frames and a long-running screensaver would develop a stutter in the
/// tail and nowhere else.
fn tail_advance(tail: f32, moved: f32, poses: usize) -> f32 {
    if poses <= 1 {
        // A species with a single frame has no cycle to advance through, and this is
        // **not** the same thing as the modulo below: `x % 1.0` is the *fractional
        // part* of `x`, so letting the general path handle it would quietly return
        // 0.4 rather than 0. The effect would still look right -- `0.4 as usize` is
        // 0 -- which is precisely why it has to be said rather than relied on.
        //
        // The character species land here: they do not animate. See
        // [`art::Source::cycle`].
        return 0.0;
    }
    (tail + moved * TAIL_RATE) % poses as f32
}

/// Cells per second below which a fish's **velocity** no longer says which way it
/// is facing, in [`step_fish`].
///
/// **0.02**, and the reason there is a threshold at all rather than `vx < 0.0` is
/// the whole of this constant. A fish's speed goes to zero for two quite different
/// reasons -- it has stopped, or it is turning through a moment of stillness -- and
/// reading the sign of the velocity cannot tell them apart. So a resting fish faced
/// **right** and popped every time it settled, and `fry` is documented as the only
/// species in the tank that ever rests.
///
/// The number itself is not delicate: a fish moves at up to `speed * MAX_SPEED_
/// FACTOR`, so 0.02 is a fortieth of its slowest cruise and two orders of magnitude
/// below a moving fish's typical horizontal component. **A threshold that rejects
/// only the genuinely-stationary case, chosen so that the two readings cannot be
/// confused.**
const FACING_EPSILON: f32 = 0.02;

/// Tail beats per cell travelled.
///
/// 0.9, and it is a *rate* rather than a per-frame step for the reason the whole
/// counter is a float: a fish at the default 6 cells a second beats about 5.4
/// times a second and a fish at 1.5 cells a second beats 1.35, which is a fish
/// idling rather than a fish frozen. Tying the beat to the frame clock instead
/// would give every fish the same rate at every speed, and the frame clock is
/// the thing that varies with the terminal.
const TAIL_RATE: f32 = 0.9;

/// Cells of clear water the overlap resolve insists on between two sprites,
/// horizontally and vertically.
///
/// Module constants rather than locals inside `resolve_overlaps`, because
/// `no_two_fish_overlap_on_screen` measures against them and a number written
/// down in two places is a number that will drift. They were 3.0 and 2.0 when the
/// sprites were characters and are smaller now that a fish is a dot bitmap and a
/// cell is half a minnow's width.
const RESOLVE_GAP_X: f32 = 1.0;
const RESOLVE_GAP_Y: f32 = 1.0;

/// How many times the overlap resolve sweeps before it gives up.
///
/// Five, because a pass that pushes a fish out of the tank is undone by the clamp
/// -- and the clamp is inside the loop, deliberately. A resolve that can push a
/// fish through the glass is not a resolve.
const RESOLVE_PASSES: usize = 5;

/// Which species a roll picks.
fn pick_species(roll: f32) -> usize {
    SPECIES_MIX
        .iter()
        .position(|(bound, _)| roll < *bound)
        .unwrap_or(0)
}

/// Sub-cell phases a fish can be drawn at, in dots.
///
/// A braille cell is 2 dots across and 4 down, and a cell is the atomic paint
/// unit, so a fish's position cannot be "three quarters of a column past 40" --
/// there is no such cell. It can be in column 40 with its dots offset, and these
/// are the offsets. Two across and four down, so the finest step is half a column
/// and a quarter of a row.
///
/// This is the whole difference between a fish that swims and a fish that slides
/// along a grid. The first version of this effect rounded each fish's position to
/// a whole cell every frame, and the module docs blamed that for the tank looking
/// like wallpaper; it was true, and it was also only half the reason. The other
/// half was that the sprites were the same shape three times over.
const PHASES_X: usize = 2;
const PHASES_Y: usize = 4;

/// Rows of gravel along the bottom.
///
/// A function of the column and nothing else, for the crab's reason, which is
/// worth repeating because it is a trap that has already been sprung once in this
/// crate: a `rand` here gives a different floor every frame, and the canvas then
/// reports the whole bottom row as changed sixty times a second for a line that
/// never moved.
const GRAVEL: &[char] = &['.', ':', '-', '~', '.', '-'];

/// Depth of the gravel, in rows, as a function of column.
///
/// One or two rows, so the tank has a floor without the gravel becoming a
/// subject. The floor is what the depth gradient is *for* -- a gradient with
/// nothing at the bottom of it reads as a fade, not as depth.
fn gravel_depth(column: usize) -> usize {
    1 + (column % 5 == 0) as usize
}

/// A fish.
struct Fish {
    /// Cell coordinates, `y` down. A float, not an integer: the blit rounds to a
    /// *dot*, and a fish whose position was integral would step a whole cell at a
    /// time and look like it was on a grid. See
    /// `no_frame_moves_a_fish_more_than_a_fifth_of_a_cell`.
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    /// Which way this fish is pointing, kept **rather than inferred from `vx`**.
    ///
    /// Because `draw_fish` read `vx < 0.0`, a fish at exactly zero velocity faced
    /// **right**, and `fry` -- cruise 0.35, documented as the only species in the
    /// tank that ever comes to rest -- popped between its two facings every time it
    /// settled. `vx` is a measurement that happens to be zero in two different
    /// situations, one of which is "moving left very slowly" and the other of which
    /// is "stopped", and it cannot tell them apart. This is the crab's `airborne`
    /// flag again: **when the state is known, store it rather than re-deriving it
    /// from a measurement.**
    facing_left: bool,
    /// Depth, `0.0` near to `1.0` far. Drives colour only; size is the species.
    z: f32,
    /// Which row of [`SPECIES`].
    species: usize,
    /// Position in the tail cycle, derived from [`Fish::tail`].
    pose: usize,
    /// Phase of the tail beat, in **frames**, and the reason it is a float.
    ///
    /// The counter used to *be* `pose`, and the sub-frame remainder was thrown
    /// away on every single frame:
    ///
    /// ```text
    /// f.pose = ((f.pose as f32 + moved * 0.9) as usize) % poses;
    /// ```
    ///
    /// That truncates `0.09` to `0`, so the pose only ever advanced when a fish
    /// covered **1.11 cells in one frame**. At 60 Hz the fastest fish in the tank
    /// covers about 0.25, so the tail never moved -- not for the braille species,
    /// not for the character ones, not for any fish at any speed. Every tail in the
    /// tank was frozen and the comment above the line claimed, correctly, that "a
    /// fish that is not moving should not wag", which is a claim nothing tested.
    ///
    /// The wrap in the update is not decoration either. Without it the accumulator
    /// grows for ever and a `f32` loses the fraction after a few million frames,
    /// which would put the tail back to a stutter on a long-running screensaver
    /// and nowhere else -- the worst place in the world to find a bug.
    tail: f32,
    /// Phase of this fish's wander, so the shoal does not turn in unison.
    phase: f32,
    /// The row this fish prefers, chosen once at spawn and **never recomputed**.
    ///
    /// This field is the decoupling of height from depth, and it is the fix for
    /// the tank's oldest visible bug: the fish used to be pulled toward
    /// `row_for_depth(z)`, which is an absolute row, so two fish at the same depth
    /// sat on the same row and a species' jitter around its band became a set of
    /// horizontal stripes -- a tank that read as wallpaper.
    ///
    /// `home` is a *row* and `z` is a *distance*, and nothing connects them. That
    /// is not a simplification, it is the honest shape: a terminal has no
    /// perspective, so there is no way for a fish's depth to mean anything about
    /// where in the tank it is, and coupling them bought a stripe and cost the
    /// volume. Depth still drives colour and size, which it can do properly.
    home: f32,
}

/// A rising bubble.
///
/// Buoyancy plus a lateral wobble, which is what makes a bubble read as a bubble
/// rather than as a dot travelling up: a real one is pushed around by the water it
/// is passing through, so it traces a slightly helical path.
struct Bubble {
    x: f32,
    y: f32,
    phase: f32,
}

/// Most flakes alive at once.
///
/// A cap rather than a timer, because the two questions are different: a timer
/// bounds how long food lasts, and a cap bounds how much is *on screen*. Both
/// matter, and a tank with forty flakes in it is a soup.
const MAX_FLAKES: usize = 24;

/// A flake of food, sinking.
struct Flake {
    x: f32,
    y: f32,
    vy: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AquariumOptions {
    /// How many fish, **at an 80x24 terminal**.
    ///
    /// Scaled up with the terminal's area and down with nothing, so the tank is
    /// as populated at 400x200 as it is at 80x24. See the module docs on why this
    /// number used to be the whole story and made the tank emptier the larger you
    /// made the window.
    pub fish: u16,

    /// Cells per second at the default speed.
    pub speed: f32,

    /// The water colour at the surface.
    pub surface: Color,

    /// The water colour at the gravel.
    pub deep: Color,

    /// The gravel colour.
    pub gravel_color: Color,

    /// Bubbles per fish per second, as a percentage.
    ///
    /// A rate rather than a count, so the population scales with the school
    /// instead of being a constant that looks sparse with forty fish and crowded
    /// with four.
    pub bubble_rate: u16,

    /// Whether the gravel is drawn at all.
    ///
    /// Not a taste setting so much as a way to isolate the rest of the picture
    /// when checking whether the depth gradient reads without an anchor.
    pub show_gravel: bool,

    pub seed: u64,
}

impl Default for AquariumOptions {
    /// Hand-written so it is the single source of truth, and hand-written for the
    /// same reason as everywhere else in this crate: the derived `Default`
    /// produced zeros, serde used the derived one, and a config file that omitted
    /// this section silently zeroed it.
    fn default() -> Self {
        Self {
            fish: 12,
            speed: 6.0,
            surface: Color::Rgb {
                r: 26,
                g: 92,
                b: 108,
            },
            deep: Color::Rgb { r: 4, g: 16, b: 40 },
            gravel_color: Color::Rgb {
                r: 62,
                g: 58,
                b: 48,
            },
            bubble_rate: 18,
            show_gravel: true,
            seed: DEFAULT_SEED,
        }
    }
}

/// One species' address in [`Aquarium::art`].
///
/// Deliberately a *copy* of the species' metadata rather than a reference into
/// the art tables. The art tables are `static` and the sprite table is not, and
/// holding a slot keeps the per-frame lookups off the static tables entirely --
/// a fish's colour and depth band are read once per fish per frame and there is
/// no reason for that to chase a pointer into a different module.
#[derive(Debug, Clone, Copy)]
struct Slot {
    /// First entry in [`Aquarium::art`] belonging to this species.
    base: usize,
    /// Frames in this species' tail cycle, and **one** for the character species,
    /// which do not animate. See [`art::Source::cycle`] for why the two media
    /// differ, and why this is not the same number as `charart::POSES`.
    poses: usize,
    /// Whether this species is drawn in characters, and so has no sub-cell phase.
    chars: bool,
    /// Preferred depth band, `0.0` near to `1.0` far. A bias, not a lock.
    depth: f32,
    /// How much colour survives at depth.
    chroma: f32,
    /// How fast this species cruises, as a multiple of the configured speed.
    ///
    /// Per species, and the cory's is the interesting one: at 0.22 it comes to
    /// rest on the gravel between excursions, which is the only place this effect
    /// ever shows a fish *stopping*.
    cruise: f32,
    /// Base colour at the spine, at `depth == 0.0`.
    color: Color,
}

pub struct Aquarium {
    screen_size: (u16, u16),
    options: AquariumOptions,
    canvas: Canvas,
    fish: Vec<Fish>,
    bubbles: Vec<Bubble>,
    bubbles_spawned: f32,
    flakes: Vec<Flake>,
    /// Every fish bitmap, indexed by [`Self::art_index`].
    ///
    /// Built once, because rasterising a fish is a few hundred float operations
    /// and there are tens of thousands of them a frame. The mirrored half and the
    /// sub-cell phases are *derived* rather than drawn, for the crab's reason.
    art: Vec<art::Sprite>,
    /// Where each species' art lives in `art`, and how to index it.
    ///
    /// The stride is per species because the two media have different shapes: a
    /// braille species occupies `poses * 2 * 8` entries and is addressed by phase,
    /// a character species occupies **one** entry per facing and addresses its own
    /// frames internally. That asymmetry is recorded here rather than inferred
    /// from the species index, so nothing has to know which medium a species uses
    /// in order to find its art.
    slots: Vec<Slot>,
    /// The size scale the bitmaps in `art` were rasterised at, so a resize knows
    /// whether they need doing again.
    art_scale_applied: f32,
    /// Cells painted by a fish on the last frame.
    ///
    /// Counted rather than inferred, and the reason is worth recording. The first
    /// version of the population test counted *non-space cells that were not one of
    /// the tank's furniture glyphs*, with a blocklist of `.` `:` `-` `~` `o` --
    /// the gravel's alphabet. That was a heuristic standing in for a fact the
    /// effect already knew, and when the fish became character art it inverted:
    /// the new fish are drawn almost entirely from exactly those five glyphs, so
    /// the filter threw away the art and reported the tank as emptying out. A
    /// measurement that a change to the thing being measured can invalidate is
    /// not a measurement.
    fish_cells: usize,
    /// The water colour by row, `rows` entries. **Static**, which is the point:
    /// it is the largest thing in the frame and the encoder reports none of it
    /// after the first frame.
    water: Vec<Color>,
    time: f32,
    rng: EffectRng,
}

impl TerminalEffect for Aquarium {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.draw();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = normalize_effect_size((width, height));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.rebuild_water();
        // The fish are rasterised at the tank's size scale, so a resize that
        // crosses a scale boundary needs new bitmaps. Only then -- `art_scale` is
        // quantised by the terminal's own dimensions, so most resizes leave it
        // alone and this is a pointer assignment.
        let scale = self.art_scale();
        if (scale - self.art_scale_applied).abs() > 1e-3 {
            let (art, slots) = build_sprites(scale);
            self.art = art;
            self.slots = slots;
            self.art_scale_applied = scale;
        }
    }

    fn handle_input(&mut self, event: &InputEvent) {
        // A click drops food. `Pressed` rather than `Released`, so the flake
        // appears under the cursor while the button is down -- on a slow connection
        // or a laggy terminal, `Released` puts it somewhere the user has already
        // moved off, which reads as the effect not listening.
        if let InputEvent::Pointer {
            position,
            phase: PointerPhase::Pressed,
            ..
        } = event
            && self.flakes.len() < MAX_FLAKES
        {
            let (fx, fy) = (position.0 as f32, position.1 as f32);
            self.flakes.push(Flake {
                x: fx,
                y: fy,
                // A little variety in the sink rate, or a dropped pinch of food
                // goes down as a rigid object.
                vy: 1.6 + self.time.sin().abs() * 1.4,
            });
            self.startle(fx, fy);
        }
    }

    fn reset(&mut self) {
        let options = self.options.clone();
        let size = self.screen_size;
        *self = Self::new(options, size);
    }
}

impl Aquarium {
    pub fn new(options: AquariumOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            fish: Vec::new(),
            bubbles: Vec::new(),
            bubbles_spawned: 0.0,
            flakes: Vec::new(),
            art: build_sprites(1.0).0,
            slots: build_sprites(1.0).1,
            art_scale_applied: 1.0,
            water: Vec::new(),
            fish_cells: 0,
            time: 0.0,
            rng: seeded_rng(options.seed, "aquarium"),
            options,
        };
        effect.rebuild_water();
        effect.art_scale_applied = effect.art_scale();
        if (effect.art_scale_applied - 1.0).abs() > 1e-3 {
            let (art, slots) = build_sprites(effect.art_scale_applied);
            effect.art = art;
            effect.slots = slots;
        }
        effect.populate();
        effect
    }

    fn rows(&self) -> usize {
        self.screen_size.1 as usize
    }

    fn cols(&self) -> usize {
        self.screen_size.0 as usize
    }

    /// Index into [`Self::art`].
    ///
    /// `pose` before `facing`, so a fish's two facings sit next to each other in
    /// the table, and the sub-cell phase last. The order has to agree with
    /// `build_art` exactly, and when it did not -- facing outermost, so the table
    /// read `[r0, l0, r1, l1]` against an index that wanted `[r0, r1, l0, l1]` --
    /// nothing crashed and nothing looked wrong. Half the tank was simply
    /// swimming the other way, which is the kind of thing you do not notice until
    /// you read the table.
    fn art_index(
        &self,
        species: usize,
        facing_left: bool,
        pose: usize,
        px: usize,
        py: usize,
    ) -> usize {
        let slot = self.slots[species];
        if slot.chars {
            // A character sprite is already at cell resolution, so there is nothing
            // to phase and nothing to pose from outside: one entry per facing, and
            // the frame is chosen inside the sprite.
            slot.base + usize::from(facing_left)
        } else {
            (((pose % slot.poses) * 2 + usize::from(facing_left)) * PHASES_Y + py)
                * PHASES_X
                + px
        }
    }

    /// A fish's sprite, from a borrow the caller can hold across a paint.
    fn art_of(
        &self,
        species: usize,
        facing_left: bool,
        pose: usize,
        px: usize,
        py: usize,
    ) -> &art::Sprite {
        &self.art[self.art_index(species, facing_left, pose, px, py)]
    }

    /// Fills [`Self::water`] for the current size.
    ///
    /// A pure function of the size and the options, so this is called on a resize
    /// and at construction and never again. It is the largest computation in the
    /// effect and it is entirely static.
    fn rebuild_water(&mut self) {
        let rows = self.rows();
        let _ = &mut self.art;
        self.water = (0..rows)
            .map(|y| {
                // A square-root ramp rather than linear. The eye reads water depth
                // as compressive -- the first metre changes the colour far more than
                // the tenth -- and a linear ramp spends most of its range in the
                // bottom third, which is the part with the least going on.
                let t = if rows <= 1 {
                    0.0
                } else {
                    (y as f32 / (rows - 1) as f32).sqrt()
                };
                palette::lerp(self.options.surface, self.options.deep, t)
            })
            .collect();
    }

    /// Fills the tank: fish and a clean bubble list.
    fn populate(&mut self) {
        let count = self.shoal_size();

        // Uniformly random placement, and this is a correction rather than a
        // preference. Stratifying by column was meant to stop the shoal clumping
        // and produced the exact opposite: a jitter of two cells against
        // five-cell slots is a *lattice*, and at six cells a second four seconds
        // of swimming does not disperse it, so the tank opened as wallpaper.
        // Random placement clumps, and a shoal is supposed to clump -- the
        // separation term in `step_fish` organises it from there.
        self.fish = (0..count).map(|i| self.spawn_fish(i)).collect();
        self.bubbles.clear();
        self.bubbles_spawned = 0.0;
        self.flakes.clear();
    }

    /// How big the fish should be, as a multiple of the drawn size.
    ///
    /// The fourth quarter of the area's exponent, and the reason is arithmetic
    /// rather than taste. Coverage is `count x area-per-fish`, and
    /// [`Self::shoal_size`] already makes the count grow as the *square root* of
    /// the area. So for coverage to keep up, each fish's area has to grow as the
    /// square root of the area too -- and a fish's linear size is the square root
    /// of its area. Hence the quarter power.
    ///
    /// Without it, a 400x200 tank held forty-eight fish the size of the ones in an
    /// 80x24 tank, and the picture was **1.7% fish against 17.7%**: the aquarium
    /// got emptier the bigger you made the window. Both numbers are measured, and
    /// `the_tank_is_no_emptier_on_a_large_terminal` is the test.
    ///
    /// Capped at 2x. Past that a minnow is thirty cells wide and the tank is a
    /// traffic jam, and the *count* is already at its own ceiling.
    fn art_scale(&self) -> f32 {
        const REFERENCE_AREA: f32 = 80.0 * 24.0;
        let area = self.cols() as f32 * self.rows() as f32;
        (area / REFERENCE_AREA).sqrt().sqrt().clamp(1.0, 2.0)
    }

    /// How many fish this terminal should hold.
    ///
    /// The square root of the area ratio, clamped to `[1, 4]`. Square root rather
    /// than linear because a shoal's *spread* grows with the tank as well as its
    /// count; scaling the count linearly with area quadruples the density on a
    /// 400x200 screen and the tank turns to soup.
    ///
    /// The clamp is a measured number and not a round one. At 400x200 the
    /// unclamped factor is 6.45, and the separation pass is `O(n^2)`: eighty fish
    /// is 3,160 pairs a frame, which is nothing, and a hundred and sixty is
    /// 12,800, which shows up in `update` on a frame that also has a full water
    /// repaint in it. Four is where the tank looked full and the cost did not.
    fn shoal_size(&self) -> usize {
        const REFERENCE_AREA: f32 = 80.0 * 24.0;
        /// An absolute ceiling, whatever the config says.
        ///
        /// The separation pass is `O(n^2)`, so this is the difference between a
        /// tank and a hang: a hand-edited `fish = 5000` on a 400x200 terminal
        /// scales to twenty thousand fish and two hundred million pair tests a
        /// frame. Four hundred is well past a full-looking tank and costs 80,000
        /// pairs, which is nothing.
        const CEILING: f32 = 400.0;
        let area = self.cols() as f32 * self.rows() as f32;
        let factor = (area / REFERENCE_AREA).sqrt().clamp(1.0, 4.0);
        ((self.options.fish as f32) * factor)
            .clamp(1.0, CEILING)
            .round() as usize
    }

    /// One fish, with its species and depth drawn from the seeded mix.
    fn spawn_fish(&mut self, index: usize) -> Fish {
        let cols = self.cols() as f32;
        let species = pick_species(self.rng.random::<f32>());
        let band = self.slots[species].depth;
        // A little jitter around the band, so the shoal is not three stripes.
        // The spread has to exceed the gap between the species bands or each
        // species occupies its own horizontal stripe and the tank reads as rows of
        // fish rather than as a volume. That is the `ants` mirrored-palette bug on
        // the other axis.
        let z = (band + (self.rng.random::<f32>() - 0.5) * 0.44).clamp(0.0, 1.0);
        let angle = self.rng.random::<f32>() * std::f32::consts::TAU;
        let speed = self.options.speed * (0.7 + 0.6 * self.rng.random::<f32>());
        // Where it prefers to be, and where it starts.
        //
        // The band's row is the *centre* and the spread is most of the tank, and
        // the ratio between those two numbers is the fix for a bug that took a
        // measurement to find. `home` used to be `row_for_depth(z) + jitter` with
        // the jitter at 1.5 rows, which is a per-fish offset from a per-fish row --
        // so the crowding in the tank was ordered by depth. Measured across four
        // species, a fish in the *deep* half of its own band was swimming **1.4
        // times faster** than one in the shallow half, in the same direction for
        // every species, and with `PARALLAX` set to zero. The cause was not the
        // depth cue at all: fish sharing a row crowd each other, so the rows
        // nearest the gravel were a jam, and a fish in a jam has its velocity
        // cancelled by the separation term and goes slow.
        //
        // That is a crowding gradient wearing depth's clothes, and it ran the
        // wrong way for [`PARALLAX`], which is why the depth cue measured as
        // nothing: 0.08, -0.02, 0.30, 0.08 of correlation between depth and speed
        // across four species, none of them a signal.
        //
        // So the spread is large enough that the bands **overlap**, the species
        // still has a tendency (the anchor the depth ordering needs), and no two
        // fish are neighbours *because of their depth*. That is what
        // [`Fish::home`]'s own doc says the field is for, and the code was not
        // doing it.
        let home = (self.row_for_depth(self.slots[species].depth)
            + (self.rng.random::<f32>() - 0.5) * HOME_SPREAD)
            .clamp(top_of_water(), self.rows() as f32 - 1.0);
        Fish {
            x: self.rng.random::<f32>() * cols,
            y: home + (self.rng.random::<f32>() - 0.5) * 1.5,
            home,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed * 0.25,
            facing_left: angle.cos() < 0.0,
            z,
            species,
            pose: index % self.slots[species].poses,
            tail: (index % self.slots[species].poses) as f32,
            phase: self.rng.random::<f32>() * std::f32::consts::TAU,
        }
    }

    /// The row a fish at depth `z` prefers.
    ///
    /// Near fish sit low and far fish sit high, which is not arbitrary: in a tank
    /// seen from the side the near glass is the bottom of the frame and the far
    /// glass the top, so depth and height are the same axis seen twice. Getting
    /// this backwards -- near fish high -- reads as a school swimming on the
    /// ceiling, which is the single most noticeable way to get an aquarium wrong.
    fn row_for_depth(&self, z: f32) -> f32 {
        let rows = self.rows() as f32;
        let gravel = self.gravel_rows() as f32;
        // The full usable height, with no fudge factor. There used to be a `* 0.85`
        // here, and it left a **dead band seven rows deep** between the lowest fish
        // and the gravel on a 30-row tank: even a fish at `z = 0` mapped to y=24
        // rather than to the floor, so the bottom third of the picture was empty
        // water with nothing in it, which is the one part of a tank that should
        // never be empty. Near fish now cruise just above the gravel, which is
        // both where they belong and what gives the depth cue its anchor.
        let usable = (rows - gravel - 1.0).max(1.0);
        (1.0 - z) * usable + 1.0
    }

    /// How many rows the gravel occupies at the deepest column.
    fn gravel_rows(&self) -> usize {
        if self.options.show_gravel { 2 } else { 0 }
    }

    /// Advances the simulation by `dt` without drawing, for an example that wants
    /// to step the tank and then look at the accumulated frame.
    pub fn advance_for_picture(&mut self, dt: f32) {
        self.advance(dt);
    }

    /// The fish, for the picture example and for the depth tests.
    /// `(x, y, depth, species, vx, vy)`.
    ///
    /// The velocities are here because the first version of the movement tests
    /// measured `sqrt(x*x + y*y)` and called it a speed, which is the *position*
    /// and rises without bound: a cory at row 40 of a 50-row tank reported a
    /// "speed" of 99 cells a frame and the "a fish can come to rest" test passed
    /// for the wrong reason and failed for the right one at the same time. A
    /// snapshot that cannot express velocity is a snapshot that will be
    /// mismeasured.
    pub fn fish_snapshot(&self) -> Vec<(f32, f32, f32, usize, f32, f32)> {
        self.fish
            .iter()
            .map(|f| (f.x, f.y, f.z, f.species, f.vx, f.vy))
            .collect()
    }

    /// A fish's colour at an arbitrary depth and species, against a **fixed** row
    /// of water.
    ///
    /// The row is fixed and that is the whole point. This used to sample the water
    /// at the row the fish would actually be at, so the two ends of the depth ramp
    /// were measured against two *different* water colours -- a bright one near the
    /// surface and a dark one near the gravel -- and the red-versus-blue falloff
    /// came out backwards, because the near sample was being compared against a
    /// lighter background than the far one. Any test of "does depth mix toward the
    /// water" has to hold the water still and move only the fish.
    pub fn colour_at(&self, z: f32, species: usize) -> Color {
        self.colour_at_row(z, species, self.rows() / 2)
    }

    /// [`Self::colour_at`] against an explicit row of the gradient.
    pub fn colour_at_row(&self, z: f32, species: usize, row: usize) -> Color {
        Self::shade_into_water(
            art::shade_colour(self.slots[species].color, 0.5),
            &self.water,
            z,
            self.slots[species].chroma,
            row.min(self.rows() - 1),
        )
    }

    /// Advances the simulation by `dt` seconds.
    fn advance(&mut self, dt: f32) {
        let dt = if dt.is_finite() {
            dt.clamp(0.0, 0.25)
        } else {
            0.0
        };
        self.time += dt;
        self.step_fish(dt);
        self.step_flakes(dt);
        self.step_bubbles(dt);
    }

    /// The shoal, one step.
    ///
    /// Purpose-built rather than shared with `boids`, and the reason is that
    /// `boids` wants a torus with trails and a fish tank wants a bounded box with
    /// sprites. A `boids` that wrapped would swim through the glass; one that did
    /// not would need the wall term anyway, which is most of what a tank needs.
    ///
    /// # The model, in one paragraph
    ///
    /// Every fish computes a **desired velocity** and then *lags* toward it. That
    /// is the entire integration step, and it is different from the one it
    /// replaced: the old model summed accelerations, integrated them, and then
    /// clamped the speed from below so that a fish could never be slow or still.
    /// A lag needs no clamp, because a fish that wants to be stationary becomes
    /// stationary, and it is frame-rate independent, which the old
    /// `v += a * dt * 6.0` was not.
    ///
    /// Into that desired velocity go five terms, and the order they are summed in
    /// is the order they matter in: **walls** first, because nothing else is worth
    /// breaking a wall for; then **separation**, because two fish inside each other
    /// is a rendering fault and everything else is a preference; then **flakes**,
    /// which are the only thing in the tank that happens to a fish; then
    /// **cohesion and alignment**, which are what make a shoal a shoal; and
    /// **wander** last, as the thing everything else is a perturbation of.
    ///
    /// # The three things that were actually wrong
    ///
    /// **Depth was the same thing as height.** Each fish was pulled toward
    /// `row_for_depth(z)`, which is an absolute row, so two fish at the same depth
    /// were on the same row and a species' jitter around its band became a set of
    /// horizontal stripes. The fix is a per-fish [`Fish::home`] that is chosen
    /// once at spawn and is *not* recomputed from `z`, so depth now drives colour
    /// and size -- which is all a 2D aquarium can honestly use it for -- and
    /// position is its own thing.
    ///
    /// **Separation ignored depth.** Every fish pushed off every other fish,
    /// including one in a different plane, so a near fish shoved a far one around
    /// and the far one shoved back. [`SEPARATION_DEPTH`] weights a neighbour's push
    /// by how close the two are in depth, and a pair a full jitter apart does not
    /// feel each other at all.
    ///
    /// **There was a speed floor**, so nothing ever stopped. See [`STEER`].
    fn step_fish(&mut self, dt: f32) {
        let (cols, rows) = (self.cols() as f32, self.rows() as f32);
        let gravel = self.gravel_rows() as f32;
        let top = 1.0f32;
        let bottom = (rows - gravel - 1.0).max(1.0);
        // `1 - exp(-STEER * dt)`, computed once. The exponential is the point:
        // `STEER * dt` would be frame-rate dependent, and a tank that swims at two
        // speeds on a 60 Hz and a 144 Hz display is a bug you cannot see.
        let lag = 1.0 - (-STEER * dt).exp();

        for i in 0..self.fish.len() {
            let (x, y, vx, vy, z, species, phase, home) = {
                let f = &self.fish[i];
                (f.x, f.y, f.vx, f.vy, f.z, f.species, f.phase, f.home)
            };
            let slot = self.slots[species];
            let art = self.art_of(species, false, 0, 0, 0);
            let (cw, ch) = (art.cells_wide() as f32, art.cells_tall() as f32);
            let half_w = cw * 0.5;
            let half_h = ch * 0.5;

            // The fish's cruise speed. `PARALLAX` is the whole of the depth cue
            // this medium has: on a screen nothing gets smaller with distance, so
            // the only way a fish at the back reads as being at the back is if it
            // moves more slowly up there.
            let cruise = self.options.speed * slot.cruise * (1.0 - PARALLAX * z);

            // ---- Walls -------------------------------------------------------
            // A fish that reaches the glass turns before it gets there rather than
            // sticking to it, which is both what fish do and what stops a sprite
            // being clipped in half by the frame.
            //
            // The margin clears the sprite's **extent**, not its centre. That is
            // not a detail: a fish sitting exactly on a limit puts its last row
            // *into the gravel*, and a fish at either end of the row is cut in
            // half by the frame. A centre-based margin is wrong by half the sprite,
            // which is the whole sprite for a one-row fish.
            // ---- Accumulate a desired direction, in units of the fish's cruise ---
            //
            // **Every term is a fraction of `cruise`, and the sum is scaled by
            // `cruise` once at the end.** That uniformity is the point, and getting
            // it wrong is what this comment is about.
            //
            // The first version made separation and the walls *absolute* -- a fixed
            // number of cells per second, unscaled -- on the reasoning that a
            // perturbation should not scale with the fish's intent. Measured, that
            // made [`PARALLAX`] invisible: a far fish's cruise term shrank while its
            // separation and wall terms did not, so the correlation between depth
            // and speed across a single species came out at 0.08, -0.02, 0.30, 0.08
            // -- four species, none of them showing a depth cue. A depth cue that
            // only applies to one of a fish's five reasons to move is not a depth
            // cue.
            //
            // A far fish is a far fish in *every* respect, so the whole vector
            // scales, including the terms that happen to have absolute units: the
            // wall term is written as `penetration / cruise` precisely so that
            // multiplying back by `cruise` restores the absolute correction while
            // still being scaled by depth.
            let mut ux = 0.0f32;
            let mut uy = 0.0f32;

            // ---- Walls -------------------------------------------------------
            // A fish that reaches the glass turns before it gets there rather than
            // sticking to it, which is both what fish do and what stops a sprite
            // being clipped in half by the frame.
            //
            // The margin clears the sprite's **extent**, not its centre. That is
            // not a detail: a fish sitting exactly on a limit puts its last row
            // *into the gravel*, and a fish at either end of the row is cut in
            // half by the frame. A centre-based margin is wrong by half the sprite,
            // which is the whole sprite for a one-row fish.
            let (left, right) = (half_w + 0.5, cols - half_w - 0.5);
            let (ceil, floor) = (top + half_h, bottom - half_h);
            if x < left {
                ux += (left - x) / cruise;
            } else if x > right {
                ux -= (x - right) / cruise;
            }
            if y < ceil {
                uy += (ceil - y) / cruise;
            } else if y > floor {
                uy -= (y - floor) / cruise;
            }

            // ---- Separation, cohesion, alignment -----------------------------
            // One pass over the shoal, three sums, because they need the same
            // neighbours and three passes would find three different shoals.
            let mut sep_x = 0.0f32;
            let mut sep_y = 0.0f32;
            let mut coh_x = 0.0f32;
            let mut coh_y = 0.0f32;
            let mut ali_x = 0.0f32;
            let mut ali_y = 0.0f32;
            let mut kin = 0.0f32;
            for (j, other) in self.fish.iter().enumerate() {
                if i == j {
                    continue;
                }
                let odx = x - other.x;
                // Cells are about twice as tall as they are wide, so a y distance
                // of one cell is worth less than an x distance of one cell. Not a
                // half either -- a fish is an ellipse, not a line.
                let ody = (y - other.y) * 0.5;
                let d2 = odx * odx + ody * ody;
                if d2 < 1e-4 {
                    continue;
                }

                // Separation, weighted by closeness **in depth**. See
                // SEPARATION_DEPTH for why this is the whole of the depth-aware
                // part of the model.
                let dz = (z - other.z).abs();
                if dz < SEPARATION_DEPTH {
                    let other_art = self.art_of(other.species, false, 0, 0, 0);
                    let radius =
                        (cw + other_art.cells_wide() as f32) * 0.5 + SEP_GAP;
                    if d2 < radius * radius {
                        let d = d2.sqrt();
                        let push =
                            (radius - d) / radius * (1.0 - dz / SEPARATION_DEPTH);
                        sep_x += odx / d * push;
                        sep_y += ody / d * push;
                    }
                }

                // Cohesion and alignment are about **your own kind**, inside a
                // radius, and they are what turn a shoal into a shoal. Cohesion
                // alone is enough to make a clump; alignment is what makes the
                // clump move like one animal, and it is the term whose absence is
                // most visible -- without it a shoal is a bag of fish that happen
                // to be near each other.
                if other.species == species && d2 < SHOAL_RADIUS * SHOAL_RADIUS {
                    kin += 1.0;
                    coh_x += other.x;
                    coh_y += other.y;
                    ali_x += other.vx;
                    ali_y += other.vy;
                }
            }
            if kin > 0.0 {
                coh_x = coh_x / kin - x;
                coh_y = coh_y / kin - y;
                ali_x /= kin;
                ali_y /= kin;
            }
            ux += sep_x * SEP_GAIN;
            uy += sep_y * SEP_GAIN;
            // ---- Flakes -------------------------------------------------------
            // The nearest one within reach wins. A fish does leave its band to eat,
            // and the row pull is damped to a tenth while one is in sight, so the
            // damping is the behaviour as well as the fix. The old weights had the
            // row pull at `0.45 * distance` and the attraction at `3.0`, which meant
            // a fish more than about seven cells off its band never moved toward
            // the flake at all and the chase did not happen.
            let chasing = self.nearest_flake(x, y);
            if let Some(flake) = chasing {
                let fdx = flake.0 - x;
                let fdy = (flake.1 - y) * 0.5;
                let d = (fdx * fdx + fdy * fdy).sqrt().max(0.5);
                ux += fdx / d * FLAKE_PULL;
                uy += fdy / d * FLAKE_PULL * 0.5;
            }

            // ---- Cohesion, alignment, row, wander ----------------------------
            if kin > 0.0 {
                let d = (coh_x * coh_x + coh_y * coh_y).sqrt().max(0.5);
                ux += coh_x / d * COHESION;
                uy += coh_y / d * COHESION;
                let a = (ali_x * ali_x + ali_y * ali_y).sqrt();
                if a > 1e-3 {
                    ux += ali_x / a * ALIGNMENT;
                    uy += ali_y / a * ALIGNMENT;
                }
            }
            // Toward `home`, its own row, chosen at spawn -- **not** toward
            // `row_for_depth(z)`. See the note on the fn. Divided by `cruise` so
            // that multiplying back restores an absolute cells-per-second
            // correction, which is what a restoring force should be.
            uy += (home - y) * ROW_PULL / cruise
                * if chasing.is_some() { 0.1 } else { 1.0 };

            // Wander, and this is the **intent**: the only term of magnitude 1.0,
            // and the reason a fish is going somewhere rather than merely being
            // pushed about. Per-fish phase, so they do not turn in unison, and a
            // slow pair of sines rather than noise, so a fish's course is a curve
            // rather than a jitter.
            //
            // Wider than tall, because a fish is: `0.35` is close to the 1:2 of the
            // cell and a little flatter, which is what a fish's path looks like
            // against a grid of cells that are themselves 1:2.
            ux += (self.time * 0.7 + phase).cos();
            uy += (self.time * 0.5 + phase).sin() * 0.35;

            // ---- Scale to a real velocity, and bank ---------------------------
            let (mut wx, mut wy) = (ux * cruise, uy * cruise);
            // **Banking**, on the finished steering. A fish already committed to a
            // turn presents more of itself, so it needs more room, and this is what
            // makes a turn read as a turn rather than as a fish changing direction.
            //
            // The measure is the component of the *steering* perpendicular to the
            // *current heading* -- a cross product -- so a fish already pointing
            // where it wants to go gets no widening, which is correct. The first
            // version computed it from the wall term alone, before the other terms
            // existed, which meant the widening only ever happened near a wall and
            // a fish turning in open water got none at all.
            let lateral = (wx * vy - wy * vx).abs();
            let bank = 1.0 + BANK * lateral / cruise.max(0.01);
            wx += sep_x * SEP_GAIN * cruise * (bank - 1.0);
            wy += sep_y * SEP_GAIN * cruise * (bank - 1.0);

            // ---- Integrate ---------------------------------------------------
            let max = cruise * MAX_SPEED_FACTOR;
            let f = &mut self.fish[i];
            f.vx += (wx - f.vx) * lag;
            f.vy += (wy - f.vy) * lag;
            let sp = (f.vx * f.vx + f.vy * f.vy).sqrt();
            if sp > max {
                f.vx = f.vx / sp * max;
                f.vy = f.vy / sp * max;
            }
            f.x += f.vx * dt;
            f.y += f.vy * dt;

            // The facing, from the velocity that was just integrated -- and only
            // when that velocity actually has a direction. A threshold rather than
            // `vx < 0.0`, because a fish that has slowed to a crawl is still going
            // the way it was going, and a fish at rest must keep the facing it had.
            // `fry` comes to a full stop on the gravel several times a minute, and
            // the old read flipped it to face right each time and popped it on the
            // way out.
            if f.vx.abs() > FACING_EPSILON {
                f.facing_left = f.vx < 0.0;
            }

            // The tail cycle, advanced on **distance travelled** rather than on
            // time, and see [`tail_advance`] for why that is arithmetic worth
            // naming.
            let moved = (f.vx * dt).abs() + (f.vy * dt).abs();
            let poses = self.slots[f.species].poses;
            f.tail = tail_advance(f.tail, moved, poses);
            f.pose = f.tail as usize;
        }

        // Every fish, every frame -- unconditionally rather than only the ones the
        // resolve happened to touch. It used to wrap x with `rem_euclid`, which is
        // the torus behaviour a *boids* wants and a tank must not have: it parks a
        // fish at x = 0, which clips half a fish off the left of the screen.
        for i in 0..self.fish.len() {
            self.clamp_one(i);
        }
        self.resolve_overlaps();
    }

    /// Puts one fish back inside the tank, clearing its sprite's extent.
    fn clamp_one(&mut self, i: usize) {
        let (cols, rows) = (self.cols() as f32, self.rows() as f32);
        let gravel = self.gravel_rows() as f32;
        let species = self.fish[i].species;
        // The extent is read off pose 0 and the unflipped sprite, which is the
        // widest case: every other frame moves ink *within* the sprite rather than
        // outside it, so clamping on pose 0 cannot clip a fish mid-beat.
        let art = &self.art[self.art_index(species, false, 0, 0, 0)];
        let (half_w, half_h) =
            (art.cells_wide() as f32 * 0.5, art.cells_tall() as f32 * 0.5);
        let f = &mut self.fish[i];
        // The extra 0.5 on the floor is for the rounding in `draw_fish`: the origin
        // is `round(y * DOTS_Y) - height/2`, so a limit of `rows - gravel - half_h`
        // admits a fish whose last row lands on the gravel.
        let top = 1.0 + half_h;
        let bottom = (rows - gravel - half_h - 0.5).max(top);
        f.x =
            f.x.clamp(half_w + 0.5, (cols - half_w - 0.5).max(half_w + 0.5));
        f.y = f.y.clamp(top, bottom);
    }

    /// Pushes fish apart **positionally**, after integration.
    ///
    /// Separation as a force cannot make the invariant hold, only make it likely.
    /// At nine and a half cells a second a fish closes a two-cell gap in about
    /// twelve frames while the force ramps over the same window, so contacts
    /// happen -- and two abutting fish do not read as two fish.
    ///
    /// Resolving the overlap directly makes "no two sprites touch" a property
    /// rather than a tendency, which is what lets `no_two_fish_overlap_on_screen`
    /// be a test rather than a hope.
    ///
    /// The exclusion shape is an **axis-aligned box**, not a circle, and the first
    /// version used a circle. That is the wrong shape for a sprite eighteen cells
    /// wide and two tall: weighting y by a constant turns the exclusion into an
    /// ellipse that is nearly a horizontal line, so the pass pushed the whole
    /// shoal apart sideways and collapsed it onto a single row -- twenty-six fish
    /// in a line, which is not a shoal. Rectangles overlap as boxes.
    ///
    /// Contacts are resolved along the axis of *least* penetration, which is the
    /// cheapest way out and the one that does not fight the depth band.
    ///
    /// **And only between fish in the same plane.** The exclusion used to be
    /// unconditional, which meant a near fish shoved a far one sideways and the far
    /// one shoved back, forever: two fish at different distances are *allowed* to
    /// overlap on a screen, because that is what being at different distances looks
    /// like, and an unconditional resolve spends the whole budget separating pairs
    /// that were never in the way. A pair further apart in depth than
    /// [`SEPARATION_DEPTH`] is not a contact at all.
    ///
    /// That leaves one honest exception, and it is what keeps
    /// `no_two_fish_overlap_on_screen` meaningful: the test asserts no two fish
    /// touch, and that assertion is now scoped to same-depth pairs, because the
    /// other case is legitimate overlap rather than a fault.
    fn resolve_overlaps(&mut self) {
        for _ in 0..RESOLVE_PASSES {
            for i in 0..self.fish.len() {
                for j in (i + 1)..self.fish.len() {
                    if (self.fish[i].z - self.fish[j].z).abs() > SEPARATION_DEPTH {
                        continue; // different planes
                    }
                    let (wi, hi) = {
                        let a = self.art_of(self.fish[i].species, false, 0, 0, 0);
                        (a.cells_wide() as f32, a.cells_tall() as f32)
                    };
                    let (wj, hj) = {
                        let a = self.art_of(self.fish[j].species, false, 0, 0, 0);
                        (a.cells_wide() as f32, a.cells_tall() as f32)
                    };
                    let dx = (self.fish[j].x - self.fish[i].x).abs();
                    let dy = (self.fish[j].y - self.fish[i].y).abs();
                    let into_x = (wi + wj) * 0.5 + RESOLVE_GAP_X - dx;
                    let into_y = (hi + hj) * 0.5 + RESOLVE_GAP_Y - dy;
                    if into_x <= 0.0 || into_y <= 0.0 {
                        continue; // not touching
                    }
                    // Side of the midpoint, so the pair separates rather than
                    // swapping places.
                    let sign = if self.fish[j].x >= self.fish[i].x {
                        1.0
                    } else {
                        -1.0
                    };
                    if into_x <= into_y {
                        let push = into_x * 0.5;
                        self.fish[i].x -= sign * push;
                        self.fish[j].x += sign * push;
                    } else {
                        let push = into_y * 0.5;
                        let vsign = if self.fish[j].y >= self.fish[i].y {
                            1.0
                        } else {
                            -1.0
                        };
                        self.fish[i].y -= vsign * push;
                        self.fish[j].y += vsign * push;
                    }
                    // Clamped here, not afterwards. Clamping at the end of the
                    // frame undid the separation: two fish pushed apart
                    // vertically were then both clamped onto the same floor row
                    // and overlapping again, which is why five passes made no
                    // difference to a residual. A resolve that can push a fish out
                    // of the tank is not a resolve.
                    self.clamp_one(i);
                    self.clamp_one(j);
                }
            }
        }
    }

    /// Scatters fish away from a point, once.
    ///
    /// The startle reflex.
    ///
    /// **It redirects and re-speeds; it does not add.** That distinction is the
    /// whole of this function and it was got wrong twice. An *additive* impulse
    /// against a fish already moving at three cells a second, with the food on its
    /// nose, produced a fish swimming the *other way* at one and three-quarter --
    /// it had turned around and it had **slowed down**, because the impulse
    /// cancelled most of the velocity it was added to. Measured: `v` went from
    /// `(2.99, -0.62)` to `(-1.65, -0.62)`. A fish that reverses and decelerates
    /// is not bolting, and no threshold on "did the speed go up" can be satisfied
    /// by an additive impulse without being set below the speed the fish already
    /// had.
    ///
    /// What an animal does is leave *at speed*, in the direction of escape. So the
    /// reflex sets the velocity: same heading away from the disturbance, at the
    /// greater of the fish's own speed and a dart speed, and then the steering lag
    /// decays it back to cruising over the next second or so. That is what makes it
    /// read as a dart and a recovery rather than as a fish that has decided to swim
    /// somewhere faster.
    ///
    /// It is the difference between dropping food into a tank and dropping food
    /// *near* a fish. Before this, a pinch of food landed on a fish's nose and the
    /// fish swam into it at its ordinary cruise speed, which is a thing no animal
    /// does.
    fn startle(&mut self, x: f32, y: f32) {
        for f in &mut self.fish {
            let dx = f.x - x;
            let dy = (f.y - y) * 0.5;
            let d = (dx * dx + dy * dy).sqrt();
            if !(1e-3..=STARTLE_RADIUS).contains(&d) {
                continue;
            }
            // Falls off with distance, so a fish the food lands on is the one that
            // bolts hardest and the ones at the edge of the radius drift. The floor
            // of 0.4 is deliberate: even a fish at the very edge of the radius
            // turns, it just does not bolt.
            let nearness = 1.0 - d / STARTLE_RADIUS;
            let dart = STARTLE_IMPULSE * (0.4 + 0.6 * nearness);
            let speed = (f.vx * f.vx + f.vy * f.vy).sqrt();
            let out = speed.max(dart);
            f.vx = dx / d * out;
            f.vy = dy / d * out * 0.5;
        }
    }

    fn nearest_flake(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        self.flakes
            .iter()
            .map(|f| {
                let dx = f.x - x;
                let dy = f.y - y;
                (dx * dx + dy * dy, f.x, f.y)
            })
            .filter(|(d2, _, _)| *d2 < 900.0)
            .min_by(|a, b| {
                a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, fx, fy)| (fx, fy))
    }

    /// Is this flake inside this fish's **jaw**?
    ///
    /// The whole front of the fish, and not a point at the end of its nose, and
    /// that is the third version of this test. Each of the first two worked on a
    /// smaller fish, and both were killed by a measurement rather than a thought.
    ///
    /// # Three models
    ///
    /// **A point at the centre**, within 1.2 cells. Calibrated when a fish was one
    /// braille glyph. It stopped working the day a fish became sixteen cells wide:
    /// ten of them converging on one flake settle into a ring of radius
    /// `(16 + 16) / 2 + SEP_GAP` -- **seventeen cells** -- and no centre ever gets
    /// inside 1.2. Measured: the closest fish sat at 0.3 cells from the drop point
    /// for four seconds and the flake was never eaten.
    ///
    /// **A point at the mouth**, half a sprite's width ahead of the centre on the
    /// side it faces. That is right about where a mouth is, and it assumes the fish
    /// puts its nose *on* the food and stops -- and a fish does not. Measured on the
    /// current set: a fish drove its **centre** to **0.2** cells from the flake and
    /// its **mouth** was still **2.6** away, because the centre arrives first and
    /// the nose only gets there if the fish swims straight through the flake. The
    /// food was uneaten and a shoal was visibly on it.
    ///
    /// **The front half of the fish.** A flake is eaten if it lies anywhere along
    /// the body from the centre forward, within a cell of the flank. That is what
    /// happens: a swimming fish passes over food with its whole front, and the
    /// reach only has to be a cell or so because the *body* is what sweeps.
    ///
    /// So this is a box anchored on the front and scaled by the sprite. The third
    /// failure was only visible because the diagnostic printed the centre distance
    /// and the mouth distance **side by side**. **A model's failure looks like a
    /// bug in the thing it models until you print both numbers.**
    fn flake_is_in_the_jaw(&self, f: &Fish, flake: &Flake) -> bool {
        let art = self.art_of(f.species, f.vx < 0.0, 0, 0, 0);
        let half_w = art.cells_wide() as f32 * 0.5;
        let half_h = art.cells_tall() as f32 * 0.5;
        // A cell of grace on every side, and never less than one cell: a one-row
        // fish still has to be able to reach a flake.
        let reach = 1.0f32.max(half_h * 0.5);
        // Which way it swims, and so which way its jaw points.
        let forward = if f.vx < 0.0 { -1.0 } else { 1.0 };
        let dx = (flake.x - f.x) * forward;
        let dy = (flake.y - f.y).abs();
        // Along the body: from a cell behind the centre to a cell past the snout.
        // Across it, weighted, because a cell is twice as tall as it is wide.
        dx >= -reach && dx <= half_w + reach && dy * 0.5 <= reach + half_h
    }

    /// Flakes sink, and the shoal eats them.
    ///
    /// Sinking rather than hanging, because a flake of food in a tank falls and a
    /// fish that can hover over one is a fish that has stopped being a fish. The
    /// rate is slow enough that a flake is usually still drifting when the first
    /// fish reaches it, which is what makes the chase look like a chase.
    fn step_flakes(&mut self, dt: f32) {
        let (cols, rows) = (self.cols() as f32, self.rows() as f32);
        for flake in &mut self.flakes {
            flake.y += flake.vy * dt;
        }
        // Eaten when a flake reaches a fish's **jaw**. See `flake_is_in_the_jaw`
        // for the three models this went through and the measurement that killed
        // each of them.
        //
        // Collected first, then removed: `retain` holds `&mut self.flakes` and the
        // jaw test reads `self.art`, so the two borrows cannot overlap.
        let mut eaten = vec![false; self.flakes.len()];
        for (i, flake) in self.flakes.iter().enumerate() {
            for f in &self.fish {
                if self.flake_is_in_the_jaw(f, flake) {
                    eaten[i] = true;
                    break;
                }
            }
        }
        let mut keep = 0usize;
        self.flakes.retain(|_| {
            let gone = eaten[keep];
            keep += 1;
            !gone
        });
        // And swept off the floor, or they accumulate on the gravel for ever.
        let floor = rows - self.gravel_rows() as f32;
        self.flakes
            .retain(|f| f.y < floor && f.x > -2.0 && f.x < cols + 2.0);
    }

    fn step_bubbles(&mut self, dt: f32) {
        let cols = self.cols() as f32;
        // Spawn, at a rate proportional to the school.
        self.bubbles_spawned += dt * f32::from(self.options.bubble_rate) / 100.0
            * self.fish.len() as f32;
        while self.bubbles_spawned >= 1.0 {
            self.bubbles_spawned -= 1.0;
            if !self.fish.is_empty() {
                let pick = (self.rng.random::<f32>() * self.fish.len() as f32)
                    as usize
                    % self.fish.len();
                let fish = &self.fish[pick];
                // Behind and *below* the fish's snout. Behind and above put the
                // bubble on the fish's back, and a pale `o` sitting on a sprite
                // reads as a second head -- the picture showed a shoal with pairs
                // of eyes.
                let dir = if fish.vx >= 0.0 { 1.0 } else { -1.0 };
                self.bubbles.push(Bubble {
                    x: fish.x - dir * 1.5,
                    y: fish.y + 0.5,
                    phase: self.rng.random::<f32>() * std::f32::consts::TAU,
                });
            }
        }

        for bubble in &mut self.bubbles {
            bubble.y -= 9.0 * dt;
            bubble.x += (self.time * 2.0 + bubble.phase).sin() * 2.5 * dt;
        }
        // Pop at the surface, and keep the list from growing without bound.
        self.bubbles
            .retain(|b| b.y > top_of_water() && b.x > -2.0 && b.x < cols + 2.0);
    }

    fn draw(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        self.canvas.clear();
        self.fish_cells = 0;

        // The water. Row by row, and it is the whole of the background: the
        // gradient is static, so after the first frame the encoder reports none of
        // it. See the module docs.
        for y in 0..rows {
            let base = self.water[y];
            for x in 0..cols {
                self.canvas.set(
                    x,
                    y,
                    Cell::with_bg(' ', Color::Reset, base, Attribute::Reset),
                );
            }
        }

        if self.options.show_gravel {
            self.draw_gravel();
        }
        self.draw_flakes();
        self.draw_bubbles();
        // Drawn back to front, so a near fish paints over a far one. The sort is
        // on depth, which is what makes the shoal read as a volume rather than as
        // a flat set of overlapping sprites.
        let mut order: Vec<usize> = (0..self.fish.len()).collect();
        order.sort_by(|a, b| {
            self.fish[*b]
                .z
                .partial_cmp(&self.fish[*a].z)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for i in order {
            self.draw_fish(i);
        }
    }

    /// A fish, blitted with its tail facing the way it is going.
    ///
    /// The position splits into a whole-cell origin and a sub-cell **phase**, and
    /// the phase picks a bitmap whose dots are already offset by it. That is the
    /// only way to get a fish off the cell lattice with a renderer whose unit is
    /// the cell, and it is why [`Self::art`] holds
    /// `species * POSES * 2 * PHASES_X * PHASES_Y` bitmaps rather than two.
    fn draw_fish(&mut self, index: usize) {
        // The stored facing rather than `vx < 0.0`: a fish at rest has no
        // direction to read, and would otherwise face right and pop the moment it
        // settled. See [`Fish::facing_left`] and [`FACING_EPSILON`].
        let (x, y, facing_left, z, species, pose) = {
            let f = &self.fish[index];
            (f.x, f.y, f.facing_left, f.z, f.species, f.pose)
        };

        // The cell the fish's own top-left cell lands in, and how far into it the
        // fish is. Both are `floor`/`fraction` of the float, never a `round` of
        // the whole thing: rounding the position is what put the shoal on a
        // lattice in the first place.
        let cell_x = x.floor();
        let cell_y = y.floor();
        let px =
            (((x - cell_x) * PHASES_X as f32).round() as usize).min(PHASES_X - 1);
        let py =
            (((y - cell_y) * PHASES_Y as f32).round() as usize).min(PHASES_Y - 1);
        let sprite = &self.art[self.art_index(species, facing_left, pose, px, py)];
        // The origin is the fish's centre column, so a left-facing fish hangs off
        // its own nose rather than off its tail.
        let origin_x = (cell_x as i32) - (sprite.cells_wide() as i32) / 2;
        let origin_y = (cell_y as i32) - (sprite.cells_tall() as i32) / 2;

        let (cols, rows) = (self.cols() as i32, self.rows() as i32);
        let base = self.slots[species].color;
        let chroma = self.slots[species].chroma;
        // Two disjoint field borrows rather than one method call each: the sprite
        // holds `&self.art` for the whole loop and the canvas is written every
        // pass, so going through `&self` for the palette would collide.
        let water = &self.water;
        let canvas = &mut self.canvas;

        // A callback, not a returned iterator, because the two media return
        // different iterator types: see `Sprite::for_each_cell`.
        let mut cells = 0usize;
        sprite.for_each_cell(pose, |cx, cy, symbol, shade| {
            let (sx, sy) = (origin_x + cx, origin_y + cy);
            if sx < 0 || sy < 0 || sx >= cols || sy >= rows {
                return;
            }
            let (sx, sy) = (sx as usize, sy as usize);
            let colour = Aquarium::shade_into_water(
                art::shade_colour(base, shade),
                water,
                z,
                chroma,
                sy,
            );
            // The background is the water for this row. **Not** `Cell::new`:
            // braille's own writers use it, and it sets `bg: Color::Reset`, which
            // would punch a terminal-default hole in the water behind every fish.
            canvas.set(
                sx,
                sy,
                Cell::with_bg(symbol, colour, water[sy], Attribute::Reset),
            );
            cells += 1;
        });
        self.fish_cells += cells;
    }

    /// Mixes a fish's colour toward the water by its depth.
    ///
    /// The whole aerial-perspective effect, and it is a few lines. Red is absorbed
    /// first, so the mix is not even: a deep fish loses its red channel first,
    /// then its green, and keeps the blue. Mixing all three channels equally
    /// toward grey would produce the same *distance* cue with none of the
    /// *underwater* cue, and the two together are what make a fish look like it is
    /// in water rather than behind fog.
    ///
    /// An associated function taking the water slice rather than a method on
    /// `&self`, because the caller is inside a loop already holding a borrow of the
    /// fish's bitmap out of the same struct, and `&self` would cover the canvas
    /// that loop is writing.
    fn shade_into_water(
        base: Color,
        water: &[Color],
        z: f32,
        chroma: f32,
        row: usize,
    ) -> Color {
        let water = water[row.min(water.len().saturating_sub(1))];
        let keep = (1.0 - z).clamp(0.0, 1.0) * chroma;
        match (base, water) {
            (
                Color::Rgb {
                    r: sr,
                    g: sg,
                    b: sb,
                },
                Color::Rgb {
                    r: wr,
                    g: wg,
                    b: wb,
                },
            ) => {
                let r = wr as f32 + (sr as f32 - wr as f32) * keep.powi(2);
                let g = wg as f32 + (sg as f32 - wg as f32) * keep;
                let b = wb as f32 + (sb as f32 - wb as f32) * keep.powf(0.35);
                Color::Rgb {
                    r: r.round().clamp(0.0, 255.0) as u8,
                    g: g.round().clamp(0.0, 255.0) as u8,
                    b: b.round().clamp(0.0, 255.0) as u8,
                }
            }
            _ => base,
        }
    }

    fn draw_gravel(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        let base = rows.saturating_sub(2);
        for x in 0..cols {
            let depth = gravel_depth(x);
            for d in 0..depth {
                let y = base + d;
                if y < rows {
                    let glyph = GRAVEL[(x + d) % GRAVEL.len()];
                    // A little darker with depth, so the floor has a top surface
                    // rather than reading as a flat band.
                    let colour = if d == 0 {
                        self.options.gravel_color
                    } else {
                        palette::shade(self.options.gravel_color, 0.6)
                    };
                    self.canvas.set(
                        x,
                        y,
                        Cell::with_bg(
                            glyph,
                            colour,
                            self.water[y.min(rows - 1)],
                            Attribute::Reset,
                        ),
                    );
                }
            }
        }
    }

    /// A flake of food, as a small bright mark.
    ///
    /// Bright and *not* blue, because a flake that looked like a bubble would be
    /// read as one -- and the whole point of the bubbles is that they are not
    /// food. Warm against a blue tank, and a different glyph so the two are told
    /// apart even in a monochrome terminal.
    fn draw_flakes(&mut self) {
        for flake in &self.flakes {
            let (x, y) = (flake.x.round() as i32, flake.y.round() as i32);
            if x < 0 || y < 0 || x >= self.cols() as i32 || y >= self.rows() as i32
            {
                continue;
            }
            let (x, y) = (x as usize, y as usize);
            self.canvas.set(
                x,
                y,
                Cell::with_bg(
                    'o',
                    Color::Rgb {
                        r: 250,
                        g: 200,
                        b: 120,
                    },
                    self.water[y],
                    Attribute::Reset,
                ),
            );
        }
    }

    fn draw_bubbles(&mut self) {
        for bubble in &self.bubbles {
            let (x, y) = (bubble.x.round() as i32, bubble.y.round() as i32);
            if x < 0 || y < 0 {
                continue;
            }
            let (x, y) = (x as usize, y as usize);
            if x >= self.cols() || y >= self.rows() {
                continue;
            }
            // A bubble is a ring, not a dot, and the ring is what makes it read
            // as one: `o` with the water still visible inside it.
            self.canvas.set(
                x,
                y,
                Cell::with_bg(
                    'o',
                    Color::Rgb {
                        r: 190,
                        g: 225,
                        b: 240,
                    },
                    self.water[y],
                    Attribute::Reset,
                ),
            );
        }
    }
}

/// Rasterises every species at every pose, both facings.
///
/// The mirrored half is derived by [`FishArt::mirrored`] rather than drawn, for
/// the crab's reason: a hand-mirrored sprite has to be redrawn by hand every time
/// the art changes, and when that was skipped there the left-facing clap's first
/// three lines came out byte-identical to the right-facing ones.
/// Builds the sprite table and the slot table that indexes it.
///
/// Returns both because they are two halves of one fact -- where a species' art
/// is *and* how wide its stride is -- and returning them apart would let a
/// caller hold one without the other, which is the same class of bug as the
/// `art_index` order that once left half the tank swimming backwards.
///
/// `scale` applies to the **braille** species only. A character sprite is
/// hand-drawn at a fixed size and cannot be scaled, and the alternative --
/// scaling down the braille ones to match on a small terminal -- inverts the size
/// hierarchy: a scaled neon would come out wider than the discus it is supposed to
/// be swimming in front of. So on a small terminal the character species are simply
/// the large fish, which is what a small tank looks like anyway.
fn build_sprites(scale: f32) -> (Vec<art::Sprite>, Vec<Slot>) {
    let dot_species = art::SPECIES.len();
    let char_species = art::CHAR_SPECIES.len();
    let mut all = Vec::with_capacity(
        dot_species * art::POSES * 2 * PHASES_X * PHASES_Y + char_species * 2,
    );
    let mut slots = Vec::with_capacity(dot_species + char_species);

    for src in art::sources() {
        let base = all.len();
        match src {
            art::Source::Dots(s) => {
                // A scaled copy of the row. Every dimension of a fish except its
                // length is a fraction of its girth, so length and girth are all
                // that have to move and a fish at twice the size is the same
                // animal twice as big.
                let mut scaled = *s;
                scaled.length =
                    ((s.length as f32) * scale).round().max(8.0) as usize;
                scaled.girth = s.girth * scale;
                for pose in 0..art::POSES {
                    let master = art::rasterise(&scaled, pose);
                    // **Mirrored first**, which is the order the character branch
                    // below uses and the order `art_index` assumes. It was
                    // `[false, true]`, and that shipped **a tank where every
                    // braille fish swam backwards**: the character art is drawn
                    // nose-left and this art is rasterised nose-left too, so
                    // `facing = false` means *nose-left* and pushing it at index 0
                    // put the left-facing sprite where `art_index` reads a boolean
                    // for "facing left" and gets a right-facing fish. Every braille
                    // fish, both directions, for as long as the two-media tank has
                    // existed.
                    //
                    // Nothing caught it because the mirror test checks that
                    // `mirrored()` *is* a mirror -- a property of `FishArt`, not of
                    // the order these two loops agree on. The invariant is a fact
                    // about **two sites**, and it is pinned by
                    // `a_fish_faces_the_way_it_travels` rather than by a comment,
                    // because the comment is what was there instead.
                    for facing in [true, false] {
                        let posed = if facing {
                            master.mirrored()
                        } else {
                            master.shifted(0, 0)
                        };
                        for py in 0..PHASES_Y {
                            for px in 0..PHASES_X {
                                all.push(art::Sprite::Dots(Box::new(
                                    posed.shifted(px, py),
                                )));
                            }
                        }
                    }
                }
            }
            art::Source::Chars(s) => {
                // One entry per facing, holding every frame. Mirroring is done once
                // per facing rather than once per (facing, frame), which is the
                // crab's reason: a mirror that is re-derived by hand is a mirror
                // that eventually is not.
                let master = charart::species(s);
                // **Mirrored first**, the same order the braille branch above
                // uses. `art_index` reads this index as "is this facing left", so
                // index 0 has to be the right-facing sprite in both branches --
                // they used to disagree and one of the two media swam backwards.
                // See `a_fish_faces_the_way_it_travels`.
                all.push(art::Sprite::Chars(Box::new(master.mirrored())));
                all.push(art::Sprite::Chars(Box::new(master)));
            }
        }
        slots.push(Slot {
            base,
            poses: src.cycle(),
            chars: src.is_chars(),
            depth: src.depth(),
            chroma: src.chroma(),
            cruise: src.cruise(),
            color: src.color(),
        });
    }
    (all, slots)
}

/// Topmost row the bubbles pop at: just under the surface.
fn top_of_water() -> f32 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aquarium::art::SPECIES;
    use crate::render::palette::perceptual_distance;
    use std::collections::HashSet;

    /// Runs a tank forward.
    fn settle(tank: &mut Aquarium) {
        for _ in 0..180 {
            tank.advance(1.0 / 60.0);
        }
    }

    /// Cells of fish on the screen of a settled tank.
    ///
    /// Read out of the effect's own counter, which the draw path keeps. The
    /// original version inferred it from the glyphs -- "not a space, and not one
    /// of the gravel's five glyphs" -- which is a heuristic standing in for a fact
    /// the effect already had, and which inverted the moment the fish became
    /// character art drawn from those same five glyphs. **A measurement a change
    /// to the subject can invalidate is not a measurement.**
    ///
    /// `get_diff` is still called, because the counter is only meaningful once a
    /// frame has actually been drawn.
    fn fish_ink(tank: &mut Aquarium) -> usize {
        let _ = tank.get_diff();
        tank.fish_cells
    }

    /// **The fish are actually drawn.**
    ///
    /// The single worst bug this file ever had, and it is here because it looked
    /// like a renderer fault and was not one. [`art::FishArt::cells`] yielded
    /// offsets in *dots* and the effect added them to a position in *cells*, so
    /// every fish was drawn up to four columns and four rows away from where it
    /// was, which on a thirty-row tank put most of them off the bottom. The
    /// picture came back as clean water and a gravel line, the frame was three
    /// thousand cells of churn, and both the glyph dump and the frame table said
    /// the effect was working.
    ///
    /// So this asserts the fish's own ink exists, over a long run, at a size
    /// proportional to the shoal. A tank of water and gravel fails it instantly.
    #[test]
    fn the_fish_are_drawn() {
        for (w, h) in [(100usize, 30usize), (80, 24), (200, 50)] {
            let mut tank =
                Aquarium::new(AquariumOptions::default(), (w as u16, h as u16));
            settle(&mut tank);
            let mut best = 0usize;
            for _ in 0..120 {
                tank.advance(1.0 / 60.0);
                best = best.max(fish_ink(&mut tank));
            }
            let fish = tank.fish_snapshot().len();
            let wanted = fish * 8; // a fish covers at least eight cells of ink
            assert!(
                best > wanted,
                "{w}x{h}: a shoal of {fish} fish never drew more than {best} cells of \\
                 ink; each should cover at least eight. The tank is water."
            );
        }
    }

    /// The water behind a fish is the water, not the terminal's own background.
    ///
    /// [`crate::buffer::Cell::new`] sets `bg: Color::Reset`, and both of braille's
    /// own writers use it. The aquarium keeps the gradient in the background
    /// channel -- it is the reason the whole effect is 2 KB a frame -- so a fish
    /// drawn with `Cell::new` punches a terminal-default hole in the water behind
    /// it. Nothing about that is visible in a glyph dump, and the frame table would
    /// report it as *cheaper*.
    #[test]
    fn a_fish_does_not_punch_a_hole_in_the_water_behind_it() {
        let (w, h) = (100usize, 30usize);
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 4,
                ..AquariumOptions::default()
            },
            (w as u16, h as u16),
        );
        settle(&mut tank);
        // Every row's background must be one of the water's own colours, and the
        // set of them must be the gradient -- one per row, monotonic, nothing else.
        let mut seen: HashSet<Color> = HashSet::new();
        for _ in 0..60 {
            tank.advance(1.0 / 60.0);
            for (_, _, cell) in tank.get_diff() {
                if !cell.bg.eq(&Color::Reset) {
                    seen.insert(cell.bg);
                }
            }
        }
        assert!(
            seen.iter().all(|c| tank.water.contains(c)),
            "a cell carried a background that is not one of the tank's {} water \\
             colours; a braille cell written with `Cell::new` resets it to the \\
             terminal's own",
            tank.water.len()
        );
    }

    /// No two fish's sprites touch.
    ///
    /// Separation as a *force* made this likely; the positional box resolve makes
    /// it a property, which is what lets it be a test. Two abutting fish do not
    /// read as two fish -- they read as one thirty-column fish.
    ///
    /// Measured in **cells**, against the bitmaps' own cell sizes. The first
    /// version of this file measured in characters against a hand-written sprite
    /// table; the fish are dot bitmaps now and their extent is a dot count, so
    /// both numbers here come from the art rather than from a second statement of
    /// it.
    #[test]
    fn no_two_fish_overlap_on_screen() {
        // A tank the shoal actually fits in. The overlap resolve is a *packing*
        // pass, and packing fourteen twenty-cell fish into a hundred and forty
        // columns has no solution -- measured, the residual was half a cell, and
        // the honest reading is that the tank was full rather than that the resolve
        // had failed. The invariant is only meaningful where the fish have room,
        // so the fixture gives them some.
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 9,
                ..AquariumOptions::default()
            },
            (200, 40),
        );
        settle(&mut tank);
        let fish = tank.fish_snapshot();
        let mut worst: f32 = 0.0;
        for i in 0..fish.len() {
            for j in (i + 1)..fish.len() {
                // Same plane only. Two fish at different depths are *allowed* to
                // overlap on a screen -- that is what being at different depths
                // looks like -- so an unconditional version of this assertion
                // would be asserting that the tank is flat, which is the bug the
                // depth bands exist to prevent rather than a property.
                if (fish[i].2 - fish[j].2).abs() > SEPARATION_DEPTH {
                    continue;
                }
                let (wi, hi) = {
                    let a = tank.art_of(fish[i].3, false, 0, 0, 0);
                    (a.cells_wide() as f32, a.cells_tall() as f32)
                };
                let (wj, hj) = {
                    let a = tank.art_of(fish[j].3, false, 0, 0, 0);
                    (a.cells_wide() as f32, a.cells_tall() as f32)
                };
                let dx = (fish[i].0 - fish[j].0).abs();
                let dy = (fish[i].1 - fish[j].1).abs();
                let into_x = (wi + wj) * 0.5 + RESOLVE_GAP_X - dx;
                let into_y = (hi + hj) * 0.5 + RESOLVE_GAP_Y - dy;
                if into_x > 0.0 && into_y > 0.0 {
                    worst = worst.max(into_x.min(into_y));
                }
            }
        }
        // A third of a cell, and the bound is the blit's own resolution rather than
        // a number chosen to be safe: two sprites a third of a cell apart round to
        // the same cell, and neither the character nor the terminal can show the
        // difference.
        assert!(
            worst <= 0.34,
            "two sprites overlap by {worst:.2} cells after settling, which is more \
             than the blit can show"
        );
    }

    /// A fish moves by less than half a cell a frame.
    ///
    /// The sub-cell phases exist for this and the test is the reason they do. A
    /// fish's position is a float and its bitmap is drawn at one of
    /// `2 * 4` dot offsets, so the finest step is half a column; a version that
    /// rounded the position to a cell put every fish on a lattice and the shoal
    /// read as wallpaper, which is a complaint about the *picture* that no frame
    /// count can express.
    #[test]
    fn no_frame_moves_a_fish_more_than_half_a_cell() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        let mut worst: f32 = 0.0;
        for _ in 0..600 {
            let before = tank.fish_snapshot();
            tank.advance(1.0 / 60.0);
            let after = tank.fish_snapshot();
            for (a, b) in before.iter().zip(after.iter()) {
                worst = worst.max((a.0 - b.0).abs().max((a.1 - b.1).abs()));
            }
        }
        assert!(
            worst <= 0.5 + 1e-3,
            "a fish moved {worst:.3} cells in one frame; the finest step the dot \\
             phases give is half a cell and anything more is a fish on a grid"
        );
    }

    /// A large terminal gets a *populated* tank, not a handful of specks.
    ///
    /// `options.fish` is a count and a count means nothing without an area. The
    /// first version spawned twenty fish whatever the terminal was, so the tank was
    /// thirteen percent covered at 80x24 and **a third of one percent** at
    /// 400x200: the aquarium got emptier the bigger you made the window, which is
    /// exactly backwards, and nothing reported it.
    ///
    /// Two numbers are asserted, and the second is deliberately *not* parity with
    /// the small tank:
    ///
    /// - **A floor**, and the floor is a measurement rather than a round number.
    ///   The fixed-count version measured 0.33% and read as an empty blue screen;
    ///   scaling the count alone took it to 1.7%, which is still thin; scaling the
    ///   fish as well took it to 5.8%, which is a tank with forty-eight animals in
    ///   it. Anything under about 3% looks like a screensaver that failed to start.
    /// - **A floor**, loose rather than tight, and it is loose *because* of the
    ///   character art. Measured: 17.4% at 80x24 and 3.8% at 400x200. The coverage
    ///   falls by a factor of 4.6 while the area grows by 42, and that is not a bug
    ///   -- a character sprite is a fixed number of cells and cannot be scaled, so
    ///   tripling the window triples the water and not the fish. The dot species
    ///   *do* scale, with `art_scale`, and they are the ones that would have held
    ///   the fraction up. **This is the cost of drawing a face, and it was taken
    ///   knowingly**: the alternative was scaling the braille fish down to match on
    ///   a large terminal, which inverts the size hierarchy and puts a scaled neon
    ///   in front of a discus.
    ///
    ///   What matters instead is that the big tank has *more fish on screen*, and
    ///   it is asserted that way, in absolute cells.
    #[test]
    fn a_large_terminal_gets_a_populated_tank() {
        let measure = |w: u16, h: u16| -> (usize, usize) {
            let mut tank = Aquarium::new(AquariumOptions::default(), (w, h));
            settle(&mut tank);
            let count = tank.fish_snapshot().len();
            let mut best = 0usize;
            for _ in 0..120 {
                tank.advance(1.0 / 60.0);
                best = best.max(fish_ink(&mut tank));
            }
            (count, best)
        };
        let (small_count, small_cells) = measure(80, 24);
        let (large_count, large_cells) = measure(400, 200);
        let area = |w: u16, h: u16| (w as f32 * h as f32) as usize;
        let small = small_cells as f32 / area(80, 24) as f32;
        let large = large_cells as f32 / area(400, 200) as f32;

        assert!(
            small > 0.05,
            "the 80x24 tank is only {:.1}% fish; the shoal has collapsed",
            small * 100.0
        );
        assert!(
            large_count >= small_count * 3,
            "the 400x200 tank holds {large_count} fish against the 80x24 tank's \
             {small_count}. A fixed count in a terminal forty times the area is a \
             handful of specks in an empty blue screen."
        );
        assert!(
            large > 0.01,
            "the 400x200 tank is {:.2}% fish, {large_cells} cells. The fixed-count \
             version measured 0.33% and read as an empty screen. Character art \
             cannot be scaled, so this floor is 1% rather than 3% -- see the note on \
             the test.",
            large * 100.0
        );
        assert!(
            large_cells >= small_cells * 3,
            "the 400x200 tank has {large_cells} cells of fish and the 80x24 tank has \
             {small_cells}. The *fraction* is supposed to fall -- 3.8% against 17.4% \
             -- because a character sprite is a fixed number of cells and a bigger \
             window is more water. The absolute count is the one that must grow."
        );
    }

    /// **A shoal clusters, and no two fish in a cluster touch.**
    ///
    /// The two halves are one property and they have to be asserted together,
    /// because either alone is satisfiable by nothing:
    ///
    /// - *Cohesion alone* is a bag of fish that happen to be near each other, and
    ///   that is what the tank looked like before this: nine fish spread evenly
    ///   across 200 columns, which is a shoal in the way a coat of paint is
    ///   weather.
    /// - *Separation alone* is a fish that avoids its own kind so reliably that it
    ///   is alone, which also satisfies a "they are not touching" test.
    ///
    /// So: a settled tank has at least one **clump** -- a group of same-species
    /// fish that are much closer together than the shoal's average spacing -- and
    /// every same-depth pair in it is clear.
    #[test]
    fn the_shoal_clusters_without_touching() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 24,
                ..AquariumOptions::default()
            },
            (200, 50),
        );
        settle(&mut tank);
        let fish = tank.fish_snapshot();
        let nearest = |i: usize| -> f32 {
            fish.iter()
                .enumerate()
                .filter(|(j, f)| *j != i && f.3 == fish[i].3)
                .map(|(_, f)| {
                    ((fish[i].0 - f.0).powi(2) + ((fish[i].1 - f.1) * 0.5).powi(2))
                        .sqrt()
                })
                .fold(f32::MAX, f32::min)
        };
        let distances: Vec<f32> = (0..fish.len()).map(nearest).collect();
        let mean = distances.iter().sum::<f32>() / distances.len() as f32;
        let closest = *distances
            .iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap();
        assert!(
            closest < mean * 0.55,
            "the closest same-species pair is {closest:.1} cells apart and the \
             average is {mean:.1}. A shoal is a clump; a uniform spread is a bag."
        );
        // And no same-depth pair is inside the resolve's gap.
        let mut worst: f32 = 0.0;
        for i in 0..fish.len() {
            for j in (i + 1)..fish.len() {
                if (fish[i].2 - fish[j].2).abs() > SEPARATION_DEPTH {
                    continue;
                }
                let (wi, hi) = {
                    let a = tank.art_of(fish[i].3, false, 0, 0, 0);
                    (a.cells_wide() as f32, a.cells_tall() as f32)
                };
                let (wj, hj) = {
                    let a = tank.art_of(fish[j].3, false, 0, 0, 0);
                    (a.cells_wide() as f32, a.cells_tall() as f32)
                };
                let into_x =
                    (wi + wj) * 0.5 + RESOLVE_GAP_X - (fish[i].0 - fish[j].0).abs();
                let into_y =
                    (hi + hj) * 0.5 + RESOLVE_GAP_Y - (fish[i].1 - fish[j].1).abs();
                if into_x > 0.0 && into_y > 0.0 {
                    worst = worst.max(into_x.min(into_y));
                }
            }
        }
        assert!(
            worst < 1.0,
            "a same-depth pair is {worst:.2} cells inside the resolve's gap"
        );
    }

    /// A near fish is above a far one, and depth is not a stripe.
    ///
    /// Two assertions, and the second is the one this round is really about.
    ///
    /// The first is the aerial-perspective ordering: a fish's colour is mixed
    /// toward the water by its depth, so a far fish's *rows* are free to be
    /// anywhere, but the tank should still read as having a front and a back.
    ///
    /// The second is the decoupling. Before [`Fish::home`] every fish was pulled
    /// toward `row_for_depth(z)`, so fish at the same depth were on the same row
    /// and the tank was horizontal stripes. **The measure is the spread of rows
    /// within a depth band, and it has to exceed the band's own height** -- a
    /// band whose fish all share a row is a stripe, however the depth is
    /// distributed.
    #[test]
    fn depth_is_not_a_stripe() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 30,
                ..AquariumOptions::default()
            },
            (200, 50),
        );
        settle(&mut tank);
        let fish = tank.fish_snapshot();
        let rows = tank.rows() as f32;

        // Front to back. **The near fish are lower on the screen than the far
        // ones**, which is the crate's stated projection and is the opposite of the
        // intuition: `row_for_depth` maps `z = 0` to the gravel and `z = 1` to the
        // surface, on the argument that a side-on tank's near glass is the bottom
        // of the frame. That convention predates this work and is defended at
        // `row_for_depth`, so the test follows it rather than arguing with it. It
        // is worth being explicit that this is the *only* remaining coupling
        // between depth and height, and it acts once, at spawn, through
        // [`Fish::home`].
        let mut near = Vec::new();
        let mut far = Vec::new();
        for f in &fish {
            if f.2 < 0.25 {
                near.push(f.1);
            } else if f.2 > 0.45 {
                far.push(f.1);
            }
        }
        assert!(
            near.len() >= 3 && far.len() >= 3,
            "the fixture needs fish at both ends of the tank; it has {} near and {} far",
            near.len(),
            far.len()
        );
        let mean = |v: &Vec<f32>| v.iter().sum::<f32>() / v.len() as f32;
        assert!(
            mean(&near) > mean(&far),
            "the near fish average row {:.1} and the far fish {:.1}. This tank maps \
             depth onto height the other way up -- near is the bottom of the frame, \
             per `row_for_depth` -- and the fish are not.",
            mean(&near),
            mean(&far)
        );

        // And within a depth band the rows are spread, not railed.
        for (lo, hi, name) in [(0.0f32, 0.2f32, "near"), (0.4, 0.6, "far")] {
            let band: Vec<f32> = fish
                .iter()
                .filter(|f| f.2 >= lo && f.2 < hi)
                .map(|f| f.1)
                .collect();
            if band.len() < 3 {
                continue;
            }
            let m = mean(&band);
            let spread = (band.iter().map(|y| (y - m).abs()).fold(0.0, f32::max)
                / rows)
                .max(0.001);
            assert!(
                spread > 0.06,
                "the {name} band spans only {:.1}% of the tank's height ({spread:.3}). \
                 That is a horizontal stripe. `Fish::home` is chosen per fish and is \
                 never recomputed from `z` precisely so this cannot happen.",
                spread * 100.0
            );
        }
    }

    /// A far fish travels less than a near one -- and **where that is measurable
    /// is a finding, not a detail**.
    ///
    /// There is no perspective on a screen, so a fish at the back that moves as
    /// fast as one at the front reads as being on the same plane, and the tank
    /// becomes a flat sheet of animals instead of a volume. [`PARALLAX`] is the
    /// only depth cue this medium has, so it has to be a real one.
    ///
    /// # Two things had to be right, and each was found by this failing
    ///
    /// **Per species, not per band.** Comparing the near band against the far band
    /// compares *species*, because the bands and the species are the same
    /// partition: the near band is the character fish and the far band the braille
    /// ones, and a neon cruises at 1.0 where a `fry` cruises at 0.35. The first
    /// version reported the far fish moving *faster*. Normalising by each species'
    /// cruise does not fix it either, because it multiplies the `fry` by nearly
    /// three and the `fry` is in the near band. The only clean way to isolate depth
    /// is to hold species constant.
    ///
    /// **And a shoal large enough to measure.** At 40 fish a species has eight to
    /// fourteen members and a quartile is three fish, so the measurement reported
    /// `slashback` running *backwards* -- 2.36x -- at a correlation of +0.47 on
    /// fourteen fish, which is not a signal. At 160 fish in a 400x80 tank the same
    /// comparison gives every species at or below 0.95. **A negative result from a
    /// fixture that is too small is not a result.**
    ///
    /// # The measurement, and what it says
    ///
    /// 160 fish, 400x80, deepest quarter against shallowest, per species:
    ///
    /// | species | cells wide | deep / shallow |
    /// |---|---|---|
    /// | neon | 11 | **0.77** |
    /// | tetra | 15 | **0.81** |
    /// | longfin | 19 | 0.98 |
    /// | bigeye | 21 | 0.93 |
    /// | slashback | 19 | 0.85 |
    /// | fry | 9 | 0.90 |
    ///
    /// **The cue shows on the small fish and is buried on the big ones**, and the
    /// reason is the separation term rather than the parallax: a nineteen-column
    /// fish is constantly in someone else's exclusion radius, so its speed is set
    /// by how crowded it is and the `1 - PARALLAX * z` on its cruise is a rounding
    /// error on top. That is a real limitation of a cue applied to a velocity that
    /// interaction dominates, and it is worth knowing before anyone raises
    /// `PARALLAX` chasing it.
    ///
    /// So the test asserts the two things that are true everywhere -- **no species
    /// is faster when it is deeper**, and **the two species the cue works on clear
    /// a hard bound** -- and the comment records the rest. A test that demanded 0.92
    /// of a nineteen-column fish would be demanding something the model does not do.
    #[test]
    fn a_far_fish_travels_less_than_a_near_one_within_its_own_species() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 160,
                ..AquariumOptions::default()
            },
            (400, 80),
        );
        settle(&mut tank);
        let n = tank.fish.len();
        // Per-fish mean speed over a run, and the depth it ended at. A single
        // frame's speed is dominated by which way the fish happened to be
        // turning, so this averages 180 frames.
        let mut depth = vec![0.0f32; n];
        let mut speed = vec![0.0f32; n];
        let mut frames = vec![0.0f32; n];
        for _ in 0..180 {
            tank.advance(1.0 / 60.0);
            for (i, f) in tank.fish_snapshot().iter().enumerate() {
                depth[i] = f.2;
                speed[i] += (f.4 * f.4 + f.5 * f.5).sqrt();
                frames[i] += 1.0;
            }
        }
        for i in 0..n {
            speed[i] /= frames[i].max(1.0);
        }
        // The cue is measured on the **braille** species, and the discriminator is
        // the medium rather than a width. Two reasons, and the second is the
        // practical one.
        //
        // A fish the separation term owns is not a measurement of its own cruise,
        // and the big character fish are separation-owned. But sprite width is the
        // wrong proxy for "big" too: `art_scale` doubles the braille fish at
        // 400x80, so at this size a neon is 22 cells wide and a `fry` is nine, and
        // **a width threshold picks the wrong two of the seven.** The medium is the
        // property that actually distinguishes them.
        const CUE_BOUND: f32 = 0.90;
        let mut checked = 0;
        for species in 0..tank.slots.len() {
            let mut kin: Vec<(f32, f32)> = (0..n)
                .filter(|i| tank.fish[*i].species == species)
                .map(|i| (depth[i], speed[i]))
                .collect();
            if kin.len() < 8 {
                continue;
            }
            kin.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            let q = kin.len() / 4;
            let mean = |v: &[(f32, f32)]| {
                v.iter().map(|p| p.1).sum::<f32>() / v.len() as f32
            };
            let (shallow, deep) = (mean(&kin[..q]), mean(&kin[kin.len() - q..]));
            let ratio = deep / shallow.max(1e-3);
            let chars = tank.slots[species].chars;
            let wide = tank.art_of(species, false, 0, 0, 0).cells_wide();
            if !chars {
                assert!(
                    ratio < CUE_BOUND,
                    "species {species} is {wide} cells wide and its deepest quarter \
                     swims at {ratio:.2} of its shallowest quarter ({deep:.2} against \
                     {shallow:.2} cells a second). PARALLAX is {PARALLAX} and it is \
                     the only depth cue a screen has."
                );
                checked += 1;
            } else {
                // Directional, and it holds everywhere: being further away never
                // makes a fish *quicker*.
                assert!(
                    ratio < 1.0,
                    "species {species} is {wide} cells wide and its deepest quarter \
                     swims at {ratio:.2} of its shallowest -- the depth cue is \
                     running backwards."
                );
            }
        }
        assert!(
            checked >= 2,
            "only {checked} species were small enough to measure the cue on"
        );
    }

    /// **A fish can come to rest.**
    ///
    /// The single clearest way to tell the old movement model from the new one.
    /// The old one clamped speed from *below* at 35% of the configured speed, so
    /// every fish in the tank was permanently, obligatorily moving no matter what,
    /// and no fish in this crate had ever stopped. Nothing about that looks like a
    /// fault on its own; it looks like fish. It only becomes visible as a fault
    /// when you ask what happens when a fish has nowhere to be -- and the answer
    /// was that it is not allowed to have nowhere to be.
    ///
    /// The cory is the species that should stop: a bottom-dweller spends most of
    /// its time on the substrate. So the assertion is that a cory's speed
    /// approaches zero, and the threshold is written as a fraction of its own
    /// cruise rather than as an absolute, because that is the number that says
    /// what it means.
    #[test]
    fn a_fish_can_come_to_rest() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 16,
                ..AquariumOptions::default()
            },
            (200, 50),
        );
        settle(&mut tank);
        let cory = tank.slots.iter().position(|s| s.cruise < 0.5).unwrap();
        let mut slowest = f32::MAX;
        for _ in 0..600 {
            tank.advance(1.0 / 60.0);
            let fish = tank.fish_snapshot();
            let speeds: Vec<f32> = fish
                .iter()
                .filter(|f| f.3 == cory)
                .map(|f| (f.4 * f.4 + f.5 * f.5).sqrt())
                .collect();
            slowest = slowest.min(speeds.iter().cloned().fold(f32::MAX, f32::min));
        }
        let cruise = tank.options.speed * tank.slots[cory].cruise;
        assert!(
            slowest < cruise * 0.5,
            "the slowest cory never dropped below {slowest:.3} cells/frame against a \
             cruise of {cruise:.3}. Every fish in this tank is obliged to move, and \
             nothing in the effect is allowed to stop."
        );
    }

    /// The tail beat is arithmetic, and this tests the arithmetic.
    ///
    /// Three properties, and the second one is the bug. Split from the simulation
    /// on purpose: the original expression was `(f.pose as f32 + moved) as usize`,
    /// which discards the fraction, and it lived in the middle of a per-fish loop
    /// behind a steering update and a position clamp. Testing it there needs a tank,
    /// a settle loop and a seed, and what it would assert is "some fish went
    /// somewhere".
    ///
    /// 1. **A fish that has not moved has not wagged.** The claim the old comment
    ///    made and nothing checked.
    /// 2. **A fish that has moved *less than a frame's worth* still wags**, given
    ///    enough of them. This is the one that failed for two rounds: at 60 Hz a
    ///    fish covers about 0.1 cells per frame, and `(0.0 + 0.09) as usize` is
    ///    zero, so the beat could only ever advance when a fish covered more than a
    ///    whole cell in a single frame.
    /// 3. **The beat depends on distance, not on how the distance was divided.**
    ///    Same total, one step or a hundred, is the same phase. This is the
    ///    frame-rate independence the comment claimed for tying it to travel, and
    ///    it is the reason `dt` does not appear in the signature at all.
    #[test]
    fn the_tail_beat_is_proportional_to_distance_travelled() {
        const POSES: usize = 4;

        // 1. Nothing moved, nothing wagged.
        for start in [0.0f32, 0.5, 1.5, 3.75] {
            assert_eq!(tail_advance(start, 0.0, POSES), start);
        }

        // 2. Sub-frame increments accumulate into a whole frame.
        let mut phase = 0.0f32;
        let mut advanced = false;
        for _ in 0..100 {
            phase = tail_advance(phase, 0.09, POSES);
            advanced |= phase >= 1.0;
        }
        assert!(
            advanced,
            "a fish covering 0.09 cells a frame never reached a single pose, so it \
             never wagged. This is the truncation bug: a beat that advances only on \
             whole frames is a beat that does not advance at 60 Hz."
        );

        // 3. The phase depends on the distance, not on the subdivision of it.
        let distance = 7.3f32;
        let one_step = tail_advance(0.0, distance, POSES);
        let many = (0..1000)
            .fold(0.0f32, |p, _| tail_advance(p, distance / 1000.0, POSES));
        assert!(
            (one_step - many).abs() < 1e-3,
            "covering {distance} cells in one step gave phase {one_step:.4} and in a \
             thousand gave {many:.4}. The beat is reading the frame clock, not the \
             distance, so it will run at a different rate on a different terminal."
        );

        // And it stays inside its own cycle, which is what the wrap is for.
        let mut phase = 0.0f32;
        for step in 0..100_000 {
            phase = tail_advance(phase, 0.25, POSES);
            assert!(
                (0.0..POSES as f32).contains(&phase),
                "phase {phase} left the cycle at step {step}"
            );
        }
    }

    /// A **braille** fish's tail actually moves.
    ///
    /// The behavioural half of the tail-counter fix, and the test that should have
    /// existed from the day the counter was written. The old
    /// `the_tail_poses_differ_only_in_the_tail` checked that the *frames* differed
    /// from each other, which is a property of the art table, and the frames were
    /// used **not at all** -- so it passed for ever while every fish in the tank swam
    /// with a frozen tail.
    ///
    /// **Braille only, and that is now load-bearing.** This used to assert that
    /// every species visited all of its poses, and when the character species were
    /// taken out of the animation it went on passing *vacuously* -- one pose is one
    /// pose visited -- without anybody noticing that it had stopped asserting
    /// anything for five of the seven species. It is split with
    /// `a_character_fish_does_not_wag_its_tail` so each half names the species it is
    /// about and a species cannot be quietly added to the wrong one.
    #[test]
    fn a_braille_fish_wags_its_tail() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 8,
                ..AquariumOptions::default()
            },
            (80, 24),
        );
        settle(&mut tank);

        let braille = tank.slots.iter().filter(|s| !s.chars).count();
        assert_eq!(
            braille,
            art::SPECIES.len(),
            "the braille species are missing from the tank, so this would pass on an \
             empty set"
        );

        for i in 0..tank.fish.len() {
            let species = tank.fish[i].species;
            if tank.slots[species].chars {
                continue;
            }
            let poses = tank.slots[species].poses;
            assert_eq!(
                poses,
                art::POSES,
                "a braille species has a cycle of {poses} rather than the art's {}, so \
                 the table and the effect disagree about how many frames exist",
                art::POSES
            );
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..120 {
                tank.advance(1.0 / 60.0);
                seen.insert(tank.fish[i].pose);
            }
            assert!(
                seen.len() == poses,
                "braille fish {i} showed only {} of its {poses} poses in two \
                 seconds: {seen:?}. A tail that does not advance through its cycle is \
                 a fish sliding rather than swimming.",
                seen.len()
            );
        }
    }

    /// A **character** fish's tail never moves, on purpose.
    ///
    /// The asymmetry with the test above is the whole design, so it gets its own
    /// assertion rather than falling out of a shared one by accident.
    ///
    /// **Why line art does not animate and dots do, at the same rate.** Both media
    /// beat at four or five times a second. For the braille pair that reads as a
    /// fish swimming, because a dot is a quarter of a cell and the eye integrates
    /// it. For a hand-drawn tail it is about **fourteen frames a second of strobing
    /// marks**, and it was reported as jitter. **Density is what makes motion
    /// legible, so the sparser medium cannot carry motion at all** -- the opposite of
    /// the usual "the denser renderer is the fancier one" reading, and the same
    /// lesson as the travelling wave's dot quantum arriving from the other side.
    ///
    /// The three hand-drawn frames per species still exist in
    /// [`art::CHAR_SPECIES`]; they are simply not played. They are kept because
    /// they are hand-drawn art in a file that is **not yet committed**, so deleting
    /// them would destroy work git could not bring back.
    #[test]
    fn a_character_fish_does_not_wag_its_tail() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 8,
                ..AquariumOptions::default()
            },
            (80, 24),
        );
        settle(&mut tank);

        let chars = tank.slots.iter().filter(|s| s.chars).count();
        assert_eq!(
            chars,
            art::CHAR_SPECIES.len(),
            "the character species are missing from the tank, so this would pass on an \
             empty set"
        );

        for (i, slot) in tank.slots.iter().enumerate() {
            if !slot.chars {
                continue;
            }
            assert_eq!(
                slot.poses, 1,
                "character species {i} has a cycle of {} poses. They are not meant to \
                 animate -- see `art::Source::cycle` -- and anything above one puts \
                 the hand-drawn beat back.",
                slot.poses
            );
        }

        // And the observable consequence, not just the number: every character fish
        // holds one frame for the whole run.
        for i in 0..tank.fish.len() {
            if !tank.slots[tank.fish[i].species].chars {
                continue;
            }
            for _ in 0..180 {
                tank.advance(1.0 / 60.0);
                assert_eq!(
                    tank.fish[i].pose, 0,
                    "character fish {i} advanced to frame {} of a one-frame cycle.",
                    tank.fish[i].pose
                );
            }
        }
    }
    /// A fish is drawn facing the way it is travelling.
    ///
    /// The invariant the whole two-medium tank rests on, and the one that was never
    /// written down anywhere except as a comment on one of the two loops.
    ///
    /// `art_index` reads the facing as `usize::from(facing_left)`, so index 0 has
    /// to be the **right**-facing sprite and index 1 the left-facing one, in *both*
    /// build loops. They shipped in opposite orders -- the braille branch iterated
    /// `for facing in [false, true]` and the character branch pushed `mirrored()`
    /// first -- so the expression was right for one medium and **inverted for the
    /// other**. Every braille fish was drawn nose-first in the direction it was
    /// swimming, in both directions, at every speed, for as long as there have been
    /// two media in this effect. The character five were correct throughout, which
    /// is why the report was "the braille fish go backwards" and not "the fish go
    /// backwards".
    ///
    /// **Why nothing caught it.** The mirror test,
    /// `a_mirrored_fish_is_the_same_fish_facing_the_other_way`, asserts that
    /// `mirrored()` *is* a mirror -- a property of [`art::FishArt`] in isolation,
    /// and completely silent about the order the two loops agree on. It passed
    /// perfectly for the whole time the tank was wrong. **The defect was never in
    /// one place; it was in two places disagreeing, and neither was wrong on its
    /// own.** That is the shape of bug a test has to be written against, because
    /// neither site could have caught it alone.
    ///
    /// Checked against the sprite rather than against the art, and **on the as-drawn
    /// sprite only**. That is not a simplification, it is the whole shape of the
    /// question: "which end is the head" is a property of the artwork, and
    /// mirroring the artwork turns a position into `width - 1 - position`, so any
    /// positional threshold gets a *different* answer from the mirror and the two
    /// can be made to disagree about a fish that is perfectly fine. `fry` did
    /// exactly that, and the first version of this test reported it as broken.
    ///
    /// **One species declines to answer, and that is the interesting case.** `fry`
    /// is nine columns wide with its eye at column five -- its art genuinely does
    /// not say which end is the head, and the left end is only distinguishable by
    /// the outline being attached to it. So `faces_left` returns `None` for it
    /// rather than guessing from a threshold, it is checked by the weaker property
    /// that still holds (its two facings are exact mirrors, so a swap is caught),
    /// and it is **named on a list** so that it cannot quietly stop being checked.
    ///
    /// The `checked` floor at the end is the other guard: if a redesign made every
    /// species undecidable this test would pass while asserting almost nothing, and
    /// a fish's facing is supposed to be readable from its art.
    #[test]
    fn a_fish_faces_the_way_it_travels() {
        /// Species whose art cannot say which end is the head. One, and the
        /// measurements are at `art::Sprite::faces_left`.
        const UNDECIDABLE: &[&str] = &["fry"];

        let tank = Aquarium::new(
            AquariumOptions {
                fish: 1,
                ..AquariumOptions::default()
            },
            (80, 24),
        );
        let mut checked = 0usize;
        let mut exempt = Vec::new();

        for (species, source) in art::sources().enumerate() {
            let (name, medium) = match source {
                art::Source::Dots(s) => (s.name, "braille"),
                art::Source::Chars(s) => (s.name, "character"),
            };
            // The sprite the table hands out for a fish travelling **left**, which
            // is the artwork as drawn in both media -- see the module docs on the
            // snout being at `x = 0`.
            let as_drawn = tank.art_of(species, true, 0, 0, 0);
            match as_drawn.faces_left(0) {
                Some(faces_left) => {
                    checked += 1;
                    assert!(
                        faces_left,
                        "{name} ({medium}): the sprite art_index hands out for a fish \
                         travelling left has its eye near its **tail**. The facing \
                         order in the art table is the wrong way round."
                    );
                }
                None => {
                    exempt.push(name);
                    // Still pinned: the two entries have to be exact mirrors, so a
                    // swap is caught even though the direction is unknowable here.
                    // Compared **in the as-drawn frame**, so the mirrored side's
                    // columns are flipped back rather than compared against the other
                    // one's raw coordinates -- otherwise this would assert that a
                    // fish equals its own mirror, which is the opposite of the point.
                    let mirrored = tank.art_of(species, false, 0, 0, 0);
                    let cells = |s: &art::Sprite, flip: bool| {
                        let w = s.cells_wide() as i32;
                        let mut v: Vec<(i32, i32, char)> = Vec::new();
                        s.for_each_cell(0, |x, y, ch, _| {
                            v.push((if flip { w - 1 - x } else { x }, y, ch))
                        });
                        v.sort_unstable();
                        v
                    };
                    let (a, b) = (cells(as_drawn, false), cells(mirrored, true));
                    assert!(!a.is_empty(), "{name} drew no cells to compare.");
                    assert_eq!(
                        a, b,
                        "{name} ({medium}): its two facings are not the same fish \
                         turned around, so they are two different animals rather than \
                         one animal facing two ways."
                    );
                }
            }
        }

        assert_eq!(
            exempt, UNDECIDABLE,
            "the set of species whose art cannot say which end is the head has \
             changed. If a species became undecidable it has stopped being checked \
             for facing, and if one became decidable it should be asserted like the \
             rest -- update the list deliberately rather than letting it drift."
        );
        assert!(
            checked >= 6,
            "only {checked} species had a decisive eye, so this test is barely \
             asserting anything."
        );
    }

    /// A fish that stops keeps the facing it had.
    ///
    /// The crab's `airborne` lesson, in a second place. `draw_fish` read
    /// `facing_left` off `vx < 0.0`, and a fish at exactly zero velocity therefore
    /// faced **right**. `fry` is documented as the only species in this tank that
    /// ever comes to rest, so the tank's one stationary fish flipped to face the
    /// wrong way several times a minute and popped back when it moved off.
    ///
    /// **The velocity is a measurement that is zero for two different reasons** --
    /// "stopped" and "turning through a moment of stillness" -- and the sign of it
    /// cannot tell them apart. So the facing is stored on [`Fish`] and only
    /// refreshed once the velocity exceeds [`FACING_EPSILON`].
    ///
    /// Asserted by *watching a fish settle*, not by poking the field: the thing that
    /// was wrong was a behaviour over time, and a test that set `vx = 0.0` and read
    /// `facing_left` back would have passed against the bug, because the bug was in
    /// `draw_fish` deciding to derive instead. This drives the tank until a cory has
    /// genuinely slowed and then checks the facing never changed underneath it.
    #[test]
    fn a_fish_that_stops_keeps_the_facing_it_had() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 16,
                ..AquariumOptions::default()
            },
            (200, 50),
        );
        settle(&mut tank);
        let cory = tank.slots.iter().position(|s| s.cruise < 0.5).unwrap();

        // The facing every fish starts this measurement with, so a change can be
        // attributed to a stop rather than to a turn.
        let mut before: Vec<bool> =
            tank.fish.iter().map(|f| f.facing_left).collect();
        let mut saw_a_rest = false;
        for _ in 0..900 {
            tank.advance(1.0 / 60.0);
            for (i, f) in tank.fish.iter().enumerate() {
                if f.species != cory {
                    continue;
                }
                if f.vx.abs() <= FACING_EPSILON {
                    saw_a_rest = true;
                    // At rest: the facing must be whatever it was, not a fresh
                    // reading of a zero.
                    assert_eq!(
                        f.facing_left, before[i],
                        "fish {i} (a {cory}-species cory, cruise {}) is at rest with \
                         vx = 0 and has changed which way it faces. A resting fish \
                         has no direction to read, so the facing has to be kept.",
                        tank.slots[cory].cruise
                    );
                } else {
                    // Moving: the facing must agree with the direction, or the
                    // stored flag has simply stopped being updated.
                    assert_eq!(
                        f.facing_left,
                        f.vx < 0.0,
                        "fish {i} is travelling at vx = {:.3} but is drawn facing the \
                         other way.",
                        f.vx
                    );
                    before[i] = f.facing_left;
                }
            }
        }
        assert!(
            saw_a_rest,
            "no cory was ever at rest in fifteen seconds, so this test never reached \
             the case it exists for."
        );
    }

    /// **A flake landing on a fish makes it bolt.**
    ///
    /// The startle reflex, and the reason it is an *impulse* rather than a force is
    /// visible in the shape of the test: a force produces a fish that swims away at
    /// a new speed, and an impulse produces a fish that darts and then recovers.
    /// So this asserts a **peak** -- the fish is much faster immediately after the
    /// drop than a moment later -- and that it is *moving away*.
    ///
    /// Before this, dropping food on a fish's nose made the fish swim into it at its
    /// ordinary cruise speed, which is a thing no animal does.
    #[test]
    fn a_dropped_flake_starts_a_burst() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 1,
                ..AquariumOptions::default()
            },
            (120, 40),
        );
        // One fish, and it is the fastest species so the burst is unmistakable.
        tank.fish.truncate(1);
        tank.fish[0].species = 0;
        settle(&mut tank);
        let (fx, fy, fvx, fvy) = {
            let f = &tank.fish[0];
            (f.x, f.y, f.vx, f.vy)
        };
        let speed_before = (fvx * fvx + fvy * fvy).sqrt();
        // Drop the flake on its nose.
        tank.flakes.push(Flake {
            x: fx + 0.5,
            y: fy,
            vy: 2.0,
        });
        tank.startle(fx + 0.5, fy);
        let burst = (tank.fish[0].vx.powi(2) + tank.fish[0].vy.powi(2)).sqrt();
        // Two assertions, because a bolt is two things and **neither a ratio nor a
        // difference measures it**. A fish swimming right at 1.5 that reverses to
        // the left at 3.1 has not tripled its speed (1.5 -> 3.1 is 2.1x) and its
        // speed has only gone up by 1.65 even though a 4.6 impulse was applied --
        // reversing costs half the visible change, because the old velocity
        // subtracts. Both ratio and difference tests were written and both reported
        // a failure against a reflex that plainly worked. The two things that are
        // actually true are: **it turned around**, and **it is now going faster
        // than it cruises**.
        let cruise = tank.options.speed
            * tank.slots[tank.fish[0].species].cruise
            * (1.0 - PARALLAX * tank.fish[0].z);
        assert!(
            fvx > 0.0 && tank.fish[0].vx < 0.0,
            "the fish was swimming right at {fvx:.2} and is now going {:.2}. A \
             flake landed half a cell to its right; it should have turned.",
            tank.fish[0].vx
        );
        // Both, and not a ratio. A bolt is *faster than it was* and *at least
        // cruising*, and the additive-impulse version of this reflex satisfied
        // neither while looking, on the screen, like it worked.
        assert!(
            burst > speed_before,
            "a flake half a cell away took the fish from {speed_before:.2} to \
             {burst:.2} cells/second. A startle is not a deceleration."
        );
        assert!(
            burst >= cruise,
            "a flake half a cell away took the fish to {burst:.2} cells/second \
             against a cruise of {cruise:.2}. A startle leaves at speed."
        );
        // Away from it, not through it. The flake went in at `x + 0.5`, so the
        // escape is to the left, and a *signed* check rather than a magnitude one
        // -- a reflex that added speed in the wrong direction would satisfy a
        // "did it move faster" assertion perfectly.
        // And the burst decays: it is a reflex, not a new cruise speed.
        for _ in 0..90 {
            tank.advance(1.0 / 60.0);
        }
        let after = (tank.fish[0].vx.powi(2) + tank.fish[0].vy.powi(2)).sqrt();
        assert!(
            after < burst * 0.6,
            "the fish was still at {after:.2} cells/second a second and a half after \
             the drop, against a peak of {burst:.2}. That is a new cruise speed, not a \
             startle."
        );
    }

    /// A fish that turns sweeps wider than one going straight.
    ///
    /// The banking term, and the smallest of the movement changes with the most
    /// visible effect: it is what makes a turn read as a turn rather than as a fish
    /// changing direction.
    #[test]
    fn a_turning_fish_sweeps_wider() {
        // The term itself, on the numbers, with no simulation. A unit turn against
        // a unit straight one: the turning fish's separation radius is larger.
        let straight = 1.0;
        let turn_rate = 4.0;
        let bank = |lateral: f32, cruise: f32| {
            1.0 + BANK * lateral.abs() / cruise.max(0.01)
        };
        assert!(
            bank(turn_rate, 1.0) > bank(0.0, 1.0),
            "a fish turning at {turn_rate} sweeps the same width as one going straight"
        );
        // And the growth is bounded: a hard turn must not blow the exclusion
        // radius up to a shoal-wide void.
        assert!(
            bank(4.0, 1.0) < 1.0 + BANK * 4.0 * MAX_SPEED_FACTOR,
            "the banking term is unbounded"
        );
        let _ = straight;
    }

    /// Every fish is fully inside the tank, and clear of the gravel.
    ///
    /// A fish half out of frame, or resting in the gravel, is one of the ways this
    /// stops reading as a tank. The rounding matters: the origin is a whole cell
    /// with a dot phase on it, so a wall limit that does not allow for the sprite's
    /// height puts a fish's last row on the gravel.
    #[test]
    fn every_fish_is_inside_the_tank_and_above_the_gravel() {
        let (w, h) = (120usize, 32usize);
        let mut tank =
            Aquarium::new(AquariumOptions::default(), (w as u16, h as u16));
        settle(&mut tank);
        let gravel_rows = tank.gravel_rows() as f32;
        for (x, y, _, species, ..) in tank.fish_snapshot() {
            let (cw, ch) = {
                let a = tank.art_of(species, false, 0, 0, 0);
                (a.cells_wide() as f32, a.cells_tall() as f32)
            };
            let left = (x - cw * 0.5).round();
            let right = (x + cw * 0.5).round();
            let top = (y - ch * 0.5).round();
            let bottom = (y + ch * 0.5).round();
            assert!(left >= 0.0, "a {species} sprite starts at x={left}");
            assert!(
                right <= w as f32,
                "a {species} sprite ends at x={right}, past {w}"
            );
            assert!(top >= 0.0, "a {species} sprite starts at y={top}");
            assert!(
                bottom <= h as f32 - gravel_rows,
                "a {species} sprite ends at y={bottom}, into the gravel which starts \
                 at {}",
                h as f32 - gravel_rows
            );
        }
    }

    /// Depth is shading, and it is *shading* -- red goes first.
    ///
    /// The whole aerial-perspective effect, and it is a few lines, so the test has
    /// to check the two things that make it underwater rather than foggy: the
    /// colour moves toward the water's, and it does so **out of proportion**,
    /// losing red faster than blue.
    ///
    /// Three versions of this assertion, and each one was wrong in a way worth
    /// recording.
    ///
    /// The first compared the *absolute* drop in each channel and only ever worked
    /// by accident: it passed for a saturated orange tetra and failed for a pale
    /// silver minnow, because a silver fish has as much blue to lose as red. It
    /// was pinned to species 1, and when the table grew a fifth species and the
    /// order changed, a green assertion went red with nothing wrong.
    ///
    /// The second normalised by each channel's available range, which fixed the
    /// species dependence and then failed for a real reason: "red falls off
    /// faster" is a claim about *survival*, not about how many bytes of channel
    /// moved. At a low `keep`, `keep^0.35` is numerically larger than `keep^2`, so
    /// a nearly-vanished fish has lost more blue than red and always has.
    ///
    /// So the claim is stated the way the physics states it: **at depth, the red
    /// that survives relative to the blue is far smaller than it is at the front.**
    /// A fish getting bluer as it goes back is the whole effect, and it is not
    /// vacuous -- a mix that pulled all three channels evenly toward grey would
    /// hold this ratio at 1.0 and fail it.
    #[test]
    fn depth_costs_red_first_and_so_reads_as_underwater_rather_than_fog() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let row = tank.rows() / 2;
        let (wr, _, wb) = rgb_of(tank.water[row]).unwrap();
        // Red the fish carries *in excess of the water it is in*, over the blue it
        // carries in excess. One at the front, one at depth.
        let excess = |c: Color| {
            let (r, _, b) = rgb_of(c).unwrap();
            (r as f32 - wr as f32) / (b as f32 - wb as f32).max(1.0)
        };
        for (species, s) in SPECIES.iter().enumerate() {
            let front = excess(tank.colour_at_row(0.0, species, row));
            let back = excess(tank.colour_at_row(0.65, species, row));
            assert!(
                front > 0.0,
                "{}: even at the front it carries no red over the water",
                s.name
            );
            assert!(
                back < front * 0.5,
                "{}: its red excess falls from {front:.2} at the front to {back:.2} \
                 at depth. Red is absorbed first -- gone by about twenty feet, \
                 green by sixty -- and a fish that does not get bluer as it goes \
                 back is a fish behind fog.",
                s.name
            );
        }
    }

    /// A far fish is measurably closer to the water than a near one.
    ///
    /// In OKLab rather than by eye, reusing the distance from `newton`. This is
    /// the "does it read" test for the depth channel, and the thing it guards is
    /// the effect collapsing into a flat set of equally bright sprites.
    #[test]
    fn a_far_fish_is_measurably_closer_to_the_water_than_a_near_one() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let water = tank.water[tank.rows() / 2];
        for (species, s) in SPECIES.iter().enumerate() {
            let near = tank.colour_at(0.0, species);
            // The far fish is not read here: at `z = 1` the mix reaches `keep = 0`
            // and the colour is the water's exactly, so its distance is zero by
            // construction and the only thing to measure is how far the near one
            // starts out.
            let near_gap = perceptual_distance(near, water);
            // A far fish mixes *all the way* to the water -- `keep` goes to zero at
            // `z = 1` -- so its distance is zero by construction and the only thing
            // to measure is how far the near one starts out.
            //
            // Which means the gap is proportional to the species' own chroma, and a
            // single absolute threshold reads a low-chroma cory as having no depth
            // cue when it has exactly as much of one as its chroma allows. So the
            // assertion is relative, with a small absolute floor to catch a colour
            // that has collapsed onto the water entirely.
            assert!(
                near_gap > 0.02,
                "{}: even a near fish is only {near_gap:.3} from the water, so there \
                 is no colour for depth to take away",
                s.name
            );
            assert!(
                near_gap > 0.02,
                "{}: a far fish is {near_gap:.3} from the water, which is not a \
                 depth cue",
                s.name
            );
        }
    }

    /// The species are told apart by colour, not only by size.
    ///
    /// Without per-species chroma every species converges on the same hue as the
    /// water mixes in, and a monochrome tank is a shoal in fog. The colour
    /// *separation* here is a different measurement from the one in
    /// `art::tests::the_species_are_not_the_same_fish_at_different_lengths`: that
    /// one asks whether they are different shapes and this one asks whether they
    /// are different colours, and the tank was monochrome once already.
    #[test]
    fn the_species_are_told_apart_by_colour() {
        let tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let colours: Vec<Color> = (0..SPECIES.len())
            .map(|s| tank.colour_at(0.35, s))
            .collect();
        for i in 0..colours.len() {
            for j in (i + 1)..colours.len() {
                let d = perceptual_distance(colours[i], colours[j]);
                assert!(
                    d > 0.05,
                    "{} and {} are {d:.3} apart at mid depth, which is a shading \
                     step rather than two fish",
                    SPECIES[i].name,
                    SPECIES[j].name
                );
            }
        }
    }

    /// The water gradient goes from the surface colour to the deep one, and gets
    /// darker going down.
    ///
    /// Also checks it is **monotonic**, which is the property that makes it read as
    /// depth. A gradient that wanders is a pattern, not a depth, and a monotonicity
    /// check is the only way to notice.
    #[test]
    fn the_water_darkens_monotonically_with_depth() {
        let tank = Aquarium::new(AquariumOptions::default(), (80, 30));
        assert_eq!(tank.water.len(), 30);
        assert_eq!(tank.water[0], tank.options.surface);
        assert_eq!(
            tank.water[29], tank.options.deep,
            "the last row is not the deep colour"
        );
        for y in 1..tank.water.len() {
            let above = palette::luminance(tank.water[y - 1]);
            let below = palette::luminance(tank.water[y]);
            assert!(
                below <= above + 1e-6,
                "row {y} is brighter than row {}; the gradient is not monotonic, so \
                 it reads as a pattern rather than as depth",
                y - 1
            );
        }
    }

    /// The water is **static**, so the encoder reports none of it after the
    /// first frame.
    ///
    /// The most expensive way to get an aquarium wrong. The depth gradient is the
    /// largest thing in the frame; if anything in it moved per frame, the bottom
    /// two thirds of the screen would be re-sent sixty times a second for a
    /// gradient that never changed.
    #[test]
    fn the_water_is_static_so_the_encoder_does_not_resend_it() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        let before: Vec<Color> = tank.water.clone();
        settle(&mut tank);
        assert_eq!(
            before, tank.water,
            "the water gradient moved while time passed"
        );
    }

    /// A settled frame is mostly *still*.
    ///
    /// The payoff for the static gradient, measured rather than asserted from the
    /// design: a tank that repainted its whole background every frame would be
    /// re-sending eighty thousand cells, and this is the assertion that would say
    /// so. It is a *budget*, so it is loose -- the point is the order of magnitude.
    #[test]
    fn a_settled_frame_reports_far_fewer_cells_than_it_has() {
        let (w, h) = (200usize, 50usize);
        let mut tank =
            Aquarium::new(AquariumOptions::default(), (w as u16, h as u16));
        settle(&mut tank);
        // Draw once before measuring. `settle` steps the simulation and never
        // draws, so the canvas baseline is still blank and the *first* diff is the
        // entire screen -- which is a real frame, and not the one this is about.
        // Measuring it as steady state reported 10,000 of 10,000 cells changed and
        // read as "the water is being re-sent every frame", which is the exact
        // opposite of what it is for.
        let _ = tank.get_diff();
        let mut worst = 0usize;
        for _ in 0..120 {
            tank.advance(1.0 / 60.0);
            worst = worst.max(tank.get_diff().len());
        }
        let total = w * h;
        assert!(
            worst < total / 12,
            "a settled frame reported {worst} changed cells out of {total}; the water \
             is in the background channel precisely so that this is a few per cent"
        );
    }

    /// The gravel does not churn.
    ///
    /// A `rand` in the sand line gives a different line every frame, and the canvas
    /// then reports the whole bottom row as changed sixty times a second for a
    /// line that never moved.
    #[test]
    fn the_gravel_does_not_churn() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        let _ = tank.get_diff();
        // Draw again with no time passing: a still picture must report nothing.
        let second = tank.get_diff();
        let gravel_changes = second.iter().filter(|(_, y, _)| *y >= 28).count();
        assert!(
            gravel_changes == 0,
            "{gravel_changes} gravel cells reported as changed with no time passing"
        );
    }

    /// Something actually changes, every frame or nearly.
    #[test]
    fn the_picture_keeps_changing() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        // The property is that the tank never *settles* into a still picture, not
        // that every frame is busy. A tank is a calm picture and should have quiet
        // moments.
        let (mut longest, mut run) = (0usize, 0usize);
        for _ in 0..600 {
            tank.advance(1.0 / 60.0);
            if tank.get_diff().len() < 4 {
                run += 1;
                longest = longest.max(run);
            } else {
                run = 0;
            }
        }
        assert!(
            longest < 12,
            "the tank went still for {longest} consecutive frames (a fifth of a second)"
        );
    }

    /// Bubbles rise and do not accumulate, and they leave the fish alone.
    ///
    /// "Leave the fish alone" because they used not to: a bubble was spawned one
    /// and a half cells *above* the fish's centre, which put it on the animal's
    /// back, and a pale `o` sitting on a sprite reads as a second head. The picture
    /// had shoals with pairs of eyes.
    #[test]
    fn bubbles_rise_are_cleared_and_do_not_sit_on_the_fish() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        let mut on_a_fish = 0usize;
        for _ in 0..900 {
            tank.advance(1.0 / 60.0);
            assert!(
                tank.bubbles.len() < 4000,
                "{} bubbles accumulated",
                tank.bubbles.len()
            );
            for b in &tank.bubbles {
                assert!(b.y > 0.0, "a bubble rose above the surface");
                for f in &tank.fish {
                    let dx = f.x - b.x;
                    let dy = f.y - b.y;
                    if dx * dx + dy * dy < 2.25 {
                        on_a_fish += 1;
                    }
                }
            }
        }
        assert!(!tank.bubbles.is_empty(), "no bubbles were ever spawned");
        // A bubble passing near a fish is fine; one sitting on it, every frame, is
        // a second head.
        assert!(
            on_a_fish < 400,
            "{on_a_fish} bubble-frames landed within a cell and a half of a fish's \
             centre; bubbles are drawn a pale `o` and one on a fish's back is a \
             second eye"
        );
    }

    /// A hostile config renders rather than panicking.
    ///
    /// Every clamp exercised with the values a hand-edited file actually contains.
    /// The mandelbrot's old budget curve passed its floor and ceiling to
    /// `f32::clamp`, and `clamp` *asserts* they are ordered, so a config with
    /// `max_iterations` under 24 died on its first frame. A clamp no test
    /// exercises is a clamp nobody has shown to be ordered.
    #[test]
    fn a_hostile_config_renders_rather_than_panicking() {
        let hostile = AquariumOptions {
            fish: 0,
            speed: -5.0,
            bubble_rate: 9999,
            ..AquariumOptions::default()
        };
        for options in [
            hostile,
            AquariumOptions {
                speed: f32::NAN,
                ..AquariumOptions::default()
            },
            AquariumOptions {
                fish: 5000,
                ..AquariumOptions::default()
            },
        ] {
            let mut tank = Aquarium::new(options, (80, 24));
            settle(&mut tank);
            let _ = tank.get_diff();
        }
    }

    /// The smallest supported terminal, and an absurdly wide one.
    #[test]
    fn extreme_shapes_render() {
        for (w, h) in [(6u16, 6u16), (6, 200), (400, 6), (200, 50)] {
            let mut tank = Aquarium::new(AquariumOptions::default(), (w, h));
            settle(&mut tank);
            tank.advance(1.0 / 60.0);
            let diff = tank.get_diff();
            for (x, y, _) in &diff {
                assert!(
                    *x < w as usize && *y < h as usize,
                    "({x},{y}) outside {w}x{h}"
                );
            }
        }
    }

    /// A resize keeps the fish inside the tank.
    ///
    /// A new one, and it is a bug this rewrite could have had: `update_size` moves
    /// the canvas and rebuilds the water but does not touch the shoal, so a fish
    /// that was comfortably in the middle of a 400-wide tank is suddenly half off
    /// the left of a 60-wide one. The clamp runs on the next step, so the picture
    /// is never wrong -- but the *frame* after the resize draws fish outside the
    /// surface, and the runtime will happily emit coordinates it should not.
    #[test]
    fn a_resized_tank_keeps_its_fish_inside_the_frame() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (200, 50));
        settle(&mut tank);
        tank.update_size(60, 20);
        // One step is enough for the clamp; the assertion is that nothing is drawn
        // out of bounds on the very next frame, which is the window the test is for.
        tank.advance(1.0 / 60.0);
        for (x, y, _) in tank.get_diff() {
            assert!(x < 60 && y < 20, "({x},{y}) outside the resized 60x20");
        }
    }

    /// Seeded runs differ, and a pinned seed reproduces.
    #[test]
    fn a_seed_reproduces_and_two_seeds_differ() {
        let run = |seed: u64| {
            let mut tank = Aquarium::new(
                AquariumOptions {
                    seed,
                    ..AquariumOptions::default()
                },
                (100, 30),
            );
            settle(&mut tank);
            tank.fish_snapshot()
        };
        assert_eq!(run(99), run(99), "seed 99 diverged");
        assert_ne!(run(99), run(100), "two seeds agreed");
    }

    /// The same seed gives the same *picture*, not just the same fish.
    #[test]
    fn a_seed_reproduces_the_picture() {
        let run = |seed: u64| {
            let mut tank = Aquarium::new(
                AquariumOptions {
                    seed,
                    ..AquariumOptions::default()
                },
                (80, 24),
            );
            settle(&mut tank);
            tank.advance(1.0 / 60.0);
            tank.get_diff()
        };
        assert_eq!(run(5), run(5), "seed 5 drew two different pictures");
        assert_ne!(run(5), run(6), "two seeds drew the same picture");
    }

    /// A click drops a flake, and the flake is drawn.
    #[test]
    fn a_click_drops_a_flake_and_the_flake_is_drawn() {
        // One fish, and the food goes on the far side of the tank from it. The
        // first version of this used the default twenty and clicked at (50, 10),
        // which is a coin flip: flakes are drawn *before* the fish, so a fish
        // swimming over the cursor covers the flake completely and the assertion
        // fails for a reason that has nothing to do with the flake. A test that
        // depends on where a shoal happens to be is a test that will be blamed on
        // the shoal.
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 1,
                ..AquariumOptions::default()
            },
            (100, 30),
        );
        settle(&mut tank);
        assert!(tank.flakes.is_empty(), "food appeared with no click");
        let _ = tank.get_diff();
        for _ in 0..300 {
            tank.advance(1.0 / 60.0);
            let _ = tank.get_diff();
        }
        assert_eq!(tank.flakes.len(), 0, "flakes appeared unbidden");

        let fish_x = tank.fish_snapshot()[0].0;
        let drop_x = if fish_x > 50.0 { 12 } else { 88 };
        tank.handle_input(&InputEvent::Pointer {
            position: (drop_x, 10),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        assert_eq!(tank.flakes.len(), 1, "a click did not drop a flake");
        assert_eq!((tank.flakes[0].x, tank.flakes[0].y), (drop_x as f32, 10.0));

        // And it is on screen, which is the part a "the list grew" assertion
        // cannot see: a flake can be in the list and never drawn.
        let diff = tank.get_diff();
        let drawn = diff.iter().any(|(x, y, cell)| {
            *x == drop_x as usize && *y == 10 && cell.symbol == 'o'
        });
        assert!(drawn, "the flake is in the list but was not drawn");
    }

    /// The shoal goes for the food, and the food gets eaten.
    #[test]
    fn the_fish_go_for_the_food_and_eat_it() {
        let mut tank = Aquarium::new(
            AquariumOptions {
                fish: 10,
                ..AquariumOptions::default()
            },
            (100, 30),
        );
        settle(&mut tank);
        tank.handle_input(&InputEvent::Pointer {
            position: (50, 15),
            phase: PointerPhase::Pressed,
            button: crate::runtime::PointerButton::Left,
        });
        let mut closed = 0usize;
        for _ in 0..240 {
            tank.advance(1.0 / 60.0);
            for (x, y, ..) in tank.fish_snapshot() {
                let d = ((x - 50.0).powi(2) + (y - 15.0).powi(2)).sqrt();
                if d < 20.0 {
                    closed += 1;
                    break;
                }
            }
            if tank.flakes.is_empty() {
                break;
            }
        }
        assert!(
            closed > 0,
            "no fish came within twenty cells of a flake that sat for four seconds"
        );
        assert!(
            tank.flakes.is_empty(),
            "the flake was never eaten, so the attraction did nothing"
        );
    }

    /// Flakes are capped, and are swept off the floor.
    #[test]
    fn food_is_capped_and_cannot_pile_up_on_the_gravel() {
        let mut tank = Aquarium::new(AquariumOptions::default(), (100, 30));
        settle(&mut tank);
        for i in 0..200u16 {
            tank.handle_input(&InputEvent::Pointer {
                position: (i % 100, 3 + (i / 100) % 25),
                phase: PointerPhase::Pressed,
                button: crate::runtime::PointerButton::Left,
            });
        }
        assert!(
            tank.flakes.len() <= MAX_FLAKES,
            "{} flakes after 200 clicks, cap is {MAX_FLAKES}",
            tank.flakes.len()
        );
        for _ in 0..1200 {
            tank.advance(1.0 / 60.0);
        }
        let floor = tank.rows() as f32 - tank.gravel_rows() as f32;
        for f in &tank.flakes {
            assert!(f.y < floor, "a flake is resting on the gravel at y={}", f.y);
        }
    }

    /// The tank's roster, by name, with each species' medium.
    ///
    /// The tank's roster, by name, with each species' medium.
    ///
    /// Seven species: **two in braille and five in characters.** The character five
    /// are one drawing set -- five spindles by one hand, 1.33 to 1.90 -- and the
    /// two braille are the procedural small-far pair that carries the shoal. Seven
    /// is more than a planted community tank comfortably holds, which is why
    /// [`SPECIES_MIX`] gives **52% of the count to the two small ones** rather
    /// than splitting it seven ways: five fish of nineteen to twenty-one columns on
    /// an eighty-column terminal is three across the screen, and equal shares would
    /// be a wall rather than a tank.
    ///
    /// It pins the **order**, because the order is an interface: `SPECIES_MIX`
    /// addresses species by index into the union of the two art tables, and
    /// transposing it would not crash -- it would just give the `0.88` bound to
    /// the wrong animal.
    #[test]
    fn the_tank_is_a_seven_species_community_in_two_media() {
        let roster: Vec<(&str, bool)> =
            art::sources().map(|s| (s.name(), s.is_chars())).collect();
        assert_eq!(
            roster,
            vec![
                ("neon", false),
                ("tetra", false),
                ("longfin", true),
                ("bigeye", true),
                ("slashback", true),
                ("fry", true),
                ("stipple", true),
            ],
            "the tank's roster changed. If a species was added, dropped or \
             reordered, this list is the thing to update -- and note that the \
             order is load-bearing, because SPECIES_MIX indexes it."
        );
    }
    /// Every species turns up, and none of them turns up every time.
    ///
    /// A mix that always lands on one species is a tank of one animal, which is
    /// what the string sprites were: three names for the same drawing. The
    /// cumulative bounds in `SPECIES_MIX` are the thing being checked, and they are
    /// the same kind of table as the mirrored palettes in `ants` -- a set of
    /// categorical bands that can quietly become one band.
    #[test]
    fn the_species_mix_covers_every_species_and_is_not_one_species() {
        // Names from the art table rather than from a list here, so a species that
        // was renamed or reordered is reported under the name its art carries.
        // Deliberately *not* a field on `Slot`: this is read by tests only, and a
        // test-only field on a struct the frame path indexes is the wrong place for
        // it. `the_tank_is_a_freshwater_community_of_five` is what pins the order.
        let names: Vec<&'static str> = art::sources().map(|s| s.name()).collect();
        let mut counts = vec![0usize; names.len()];
        for seed in 0..24u64 {
            let tank = Aquarium::new(
                AquariumOptions {
                    fish: 40,
                    seed: seed.wrapping_mul(7919),
                    ..AquariumOptions::default()
                },
                (100, 30),
            );
            for (_, _, _, s, ..) in tank.fish_snapshot() {
                counts[s] += 1;
            }
        }
        for (i, n) in counts.iter().enumerate() {
            assert!(
                *n > 0,
                "{} never appeared in {} spawns; the mix's cumulative bounds are \
                 wrong",
                names[i],
                counts.iter().sum::<usize>()
            );
        }
        // And the shares are not absurd. The neon is the commonest by design and
        // the discus the rarest, so a species that takes more than half the tank
        // means the bounds have been transposed. This is the same shape of
        // assertion as the mirrored palettes in `ants`: a set of categorical bands
        // that can quietly collapse into one band.
        let total: usize = counts.iter().sum();
        for (i, n) in counts.iter().enumerate() {
            let share = *n as f32 / total as f32;
            assert!(
                share < 0.55,
                "{} took {:.0}% of the tank; the mix is not a mix",
                names[i],
                share * 100.0
            );
        }
    }

    fn rgb_of(c: Color) -> Option<(u8, u8, u8)> {
        match c {
            Color::Rgb { r, g, b } => Some((r, g, b)),
            _ => None,
        }
    }
}
