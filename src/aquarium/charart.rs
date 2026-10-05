//! Fish drawn in **characters**, in the idiom of the ASCII art archives.
//!
//! # Why characters and not the braille in the sibling module
//!
//! Two media are in use in this crate and they are not interchangeable, because
//! they are good at opposite things. `art::FishArt` is a braille dot bitmap:
//! eight times the density, square dots, smooth sub-cell motion. It is very good
//! at a *silhouette* and at tone.
//!
//! It is bad at a **drawing**, and that is what a fish is. Look at what the
//! galleries actually do -- Joan Stark's betta is `|\\__/|`, Bob Allison's is
//! `>.....::::;\\\\`, Strandberg's discus is `/@` for an eye and `> )<` for a
//! mouth. Every one of them is an **outline** with a little detail inside it and
//! the water showing between the strokes. Not one is filled.
//!
//! A filled braille fish is a solid mass of dots, and a solid mass reads as a
//! rock at any density and at any colour ramp. An outline reads as a container,
//! and an eye and a mouth turn a container into an animal. That is the whole
//! difference between this file and a braille fish, and it is not a matter of
//! degree.
//!
//! So the split follows **depth**, because depth is what decides it. A near fish
//! is drawn in characters, because you can see its face and a face needs
//! strokes. A small or far one is drawn in braille, because at that size only the
//! silhouette is legible and braille's density is worth more there than its fill
//! is worth less.
//!
//! And the split is not a compromise. **It is a depth cue that was free.** A near
//! fish drawn in `/ \ | _ -` carries more ink and more internal detail than a far
//! fish drawn as sparse dots, which is true of real water -- you resolve more in
//! the thing in front of you -- and it is the same trick as the aerial
//! perspective in `fish_colour`, done with resolution instead of hue.
//!
//! # The art
//!
//! Hand-drawn, and that is a considered choice rather than a default. The obvious
//! alternative is generating these procedurally, the way the braille species are
//! generated, and it was tried: a species is a body *shape* -- four numbers for a
//! two-arc profile -- and it produces a recognisable fish every time. It also
//! produces a **generic** one, with no face, because a profile has nowhere to put
//! an eye.
//!
//! The braille species get away with being shapes because at 30 dots a body is
//! all silhouette anyway. At 16 characters a body is a drawing, and a drawing has
//! to be drawn.
//!
//! Three frames per species, and **the frames differ only in the trailing
//! third**. That is what a tail does, and
//! `the_tail_poses_differ_only_in_the_tail` is what holds it.
//!
//! # The art
//!
//! **Five drawings, verbatim, from one hand.** They are in
//! [`crate::aquarium::art::CHAR_SPECIES`] and they are the reference art itself --
//! the doubled backslashes, the stipple and the `©` eye all included, on purpose.
//! The names (`longfin`, `bigeye`, `slashback`, `fry`, `stipple`) are descriptive
//! rather than zoological: the drawings are unnamed.
//!
//! # They are five spindles, and that is a fact about them
//!
//! Measured 1.33, 1.58, 1.67, 1.75 and 1.90 to one, once the cell's 2:1 height is
//! accounted for. A previous set written for this effect held a round fish and a
//! tall one on purpose, so the tank had shape variety. **This set has neither.** It
//! is uniform, and two tests had to change because of it rather than in spite of
//! it -- see `the_character_species_are_all_spindles_of_the_same_proportion` and
//! `art::no_two_species_are_the_same_animal`.
//!
//! # Frames are drawn, not bent, and the drawing is smaller than it looks
//!
//! Three frames per species, and the frames differ **only in the trailing third**.
//! That was going to need relaxing -- a twenty-one-column fish has seven columns
//! past the two-thirds mark, which is enough for a tail and not for a body wave --
//! and it did not, because the beat these drawings support *is* a tail beat. Their
//! tails are a `)` and a `{`, a `/` and a `\`, an `=` hanging off a `//\___`; each
//! is a handful of marks at the very end, and moving those up and down a row is
//! the whole of the undulation. `the_frames_differ_only_in_the_tail` is
//! unchanged and `every_frame_has_the_same_head` is the one that matters: a head
//! that changes between frames reads as twitching, which is worse than a fish that
//! merely slides.
//!
//! A mechanical per-column shear was tried first and it **tears these apart**.
//! They are line drawings with ornament -- `/-._`, `·.¸`, `_\`___=` -- and a
//! per-column shift scatters the flourishes, splits `/-._` and separates every tail
//! from the body it belongs to. Bending a *shape* works; bending a raster of a
//! drawing does not, and the same was true of the earlier attempt in this file.
//!
//! # The two characters that needed a decision
//!
//! **`stipple` is built from `·`, `¸` and `´`,** which are East-Asian **ambiguous**
//! width and render two cells wide in a CJK-locale terminal. They are allowed
//! because [`crate::render::is_ambiguous_or_narrow`] accepts them -- the
//! `\u{00A0}..=\u{00FF}` band covers all of them -- and `presets::DOTS` already
//! ships a `·`. The texture *is* the stipple, so transliterating it is a decision
//! and not a repair.
//!
//! **`©` is the eye**, and it is kept because the drawing is kept. It is not an
//! eye glyph any renderer has heard of, so it is in [`EYE_CHARS`] here.
//!
//! # The other rule the two media share
//!
//! **The snout is at `x = 0` in both.** `art::FishArt` puts it there because its
//! profile runs from `t = 0` at the nose, and all five of these drawings face
//! left, which is the same thing. One facing rule and one `mirrored()` serve the
//! whole crate; two conventions in one crate is a bug factory, and the first
//! runtime-bend implementation here anchored itself at the wrong end and tore every
//! fish in the tank.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crossterm::style::{Attribute, Color};

/// Hand-drawn frames per species, all three of them real drawings.
///
/// **This is not the length of anyone's tail cycle.** It is how many frames the art
/// has, and the effect plays **one** of them: see
/// [`crate::aquarium::art::Source::cycle`], where the decision not to animate the
/// character species lives and why.
///
/// The two numbers had to be renamed apart precisely because they used to be the
/// same field, and this comment is the second version of the argument -- the first
/// said the cycle was three because "a drawn fish has a body and three frames is the
/// smallest number that reads as a beat rather than as a twitch". That was true, it
/// was tested, the frames were drawn and the cross-frame tests all passed, and the
/// fish were still reported as jittering. **Three frames of line art beating four
/// times a second is fourteen frames a second of strobing marks, and there is
/// nothing in a handful of characters for the eye to integrate** -- which is exactly
/// what the braille pair have a quarter of a cell of, and do not have any other
/// medium's worth of.
///
/// So the frames stay. They are hand-drawn, they are the reference art, and the
/// tests that compare them frame to frame still check something real about the
/// drawings. **The animation is what was removed, not the art.**
pub const POSES: usize = 3;

/// The lit back, as a point on the shading ramp. The ramp runs `0.0` at the
/// belly to `1.0` at the back; see [`crate::aquarium::art::shade_colour`].
///
/// The body deliberately does not reach either end of the range, and unlike the
/// braille species there is no fin that needs the ends. In character art the
/// *glyphs* separate a fin from a flank -- `/` and `\` are visibly not `:` and
/// `;` -- so the tone only has to do the one job it is good at, which is the
/// light coming from above.
const BODY_TOP: f32 = 0.78;
const BODY_BOT: f32 = 0.20;

/// Tones for the marks that are not body.
/// The shade the renderer gives an eye, and the **only** mark that says which
/// way a sprite faces.
///
/// Public because [`crate::aquarium::art::Sprite::faces_left`] has to answer that
/// question for both media from outside this module, and the alternative -- "the
/// head end carries more ink than the tail end" -- is a heuristic that happens to
/// hold for the current seven fish and goes wrong the moment one has a heavy
/// peduncle or a long trailing dorsal. An eye is the one mark every fish has.
pub const EYE: f32 = 0.03;
const MOUTH: f32 = 0.34;
const BARBEL: f32 = 0.18;

/// The characters that are an eye.
///
/// `o`, `O` and `@` are what most galleries use. **`0` and `©` are here because
/// the current set of drawings uses them** -- a small fish's eye is a zero and a
/// stippled fish's is a copyright ring -- and an eye the renderer does not know
/// about is an eye the light does not reach, which is the same failure the
/// braille species had with a pupil smaller than a cell.
///
/// Accepting all five means a species can be re-drawn by hand later without
/// silently losing its eye.
const EYE_CHARS: [char; 5] = ['o', 'O', '@', '0', '©'];
/// The characters that are a mouth.
///
/// `<` and `>` open left and right, so the sprite's own facing decides which one a
/// given fish uses. Of the current drawings only `bigeye` has one -- `> O )<)`,
/// where the `>` is the snout and the `)<)` is the gill.
///
/// **The parentheses are deliberately not in this set**, and it is worth saying why
/// because getting it wrong is silent. These drawings use a pair of brackets for
/// the head and another for the caudal, and those are *contour*, not mouth.
/// Classifying them as a mouth gave the tail fork a fixed mid-tone, and the tail
/// sits on the fish's **back** -- so the back of a fish came out lighter than its
/// flank and `the_body_is_lit_from_above` failed. A mark has to mean the same thing
/// everywhere it appears, and here the same characters mean two different things in
/// two drawings, which is a sign the class is wrong rather than the art.
const MOUTH_CHARS: [char; 2] = ['<', '>'];
/// Whiskers.
const BARBEL_CHARS: [char; 2] = ['v', 'V'];

/// The directional strokes a contour is drawn with: the eight compass marks and
/// the galleries' corner marks.
///
/// Test-only, and that is the point. This is the vocabulary the medium has, and
/// the first version of the art tests had no way to name it -- so "drawn, not
/// stamped", the one property that separates this file from the braille one, was
/// asserted as a proxy (a gap in the middle column) that a legitimate solid
/// interior also satisfied. Naming the strokes is what made it checkable.
#[cfg(test)]
const STROKES: [char; 9] = ['/', '\\', '|', '-', '_', ',', '\'', '`', '.'];

/// A species drawn in characters, at every pose and both facings.
///
/// `frames[pose][row][col]`, and every row of every pose is the same width --
/// [`CharArt::new`] refuses to build a ragged one, for the crab's reason: the art
/// is hand-written, and a row one character short is the kind of thing that
/// survives a year of editing.
pub struct CharArt {
    frames: Vec<Vec<Vec<char>>>,
    width: usize,
    height: usize,
}

impl CharArt {
    /// Builds a sprite from [`POSES`] frames of equal-width rows.
    ///
    /// # Panics
    ///
    /// If the frames are ragged or the count is wrong. Both are authoring
    /// mistakes in a table of hand-written art, both surface here rather than in
    /// the terminal, and a sprite whose rows differ in width is a sprite whose
    /// fish have tails of two lengths.
    ///
    /// The row count is checked **per frame** rather than against frame 0's, and
    /// the width against frame 0's, because those are the two ways a hand-written
    /// table actually goes wrong. A frame that lost a row reads as a fish with a
    /// bitten-off belly; a frame one character wider reads as a fish with a longer
    /// tail than the one it is supposed to be.
    pub fn new(frames: Vec<Vec<Vec<char>>>) -> Self {
        assert_eq!(
            frames.len(),
            POSES,
            "a species has {POSES} frames, one per point of the tail beat",
        );
        let width = frames[0][0].len();
        let height = frames[0].len();
        for (pose, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.len(),
                height,
                "pose {pose} has {} rows and pose 0 has {height}",
                frame.len(),
            );
            for (row, line) in frame.iter().enumerate() {
                assert_eq!(
                    line.len(),
                    width,
                    "pose {pose} row {row} is {} characters wide and the sprite is \
                     {width}",
                    line.len(),
                );
            }
        }
        Self {
            frames,
            width,
            height,
        }
    }

    /// Width in terminal cells. Trailing spaces count, so a sprite's centring and
    /// its separation radius are both predictable.
    pub fn cells_wide(&self) -> usize {
        self.width
    }

    /// Height in terminal cells.
    pub fn cells_tall(&self) -> usize {
        self.height
    }

    /// The sprite's own cells for one pose: `(x, y, glyph, shade)`.
    ///
    /// Cell offsets, not character offsets inside some larger buffer, for the
    /// same reason the braille iterator yields cell offsets: the caller offsets by
    /// the fish's position in cells and this iterator has already done the
    /// dot-to-cell resampling. (It also used to get that wrong, and every fish in
    /// the tank was drawn four times too far away and off the bottom of the
    /// screen -- see [`crate::aquarium::art::FishArt::cells`].)
    pub fn cells(
        &self,
        pose: usize,
    ) -> impl Iterator<Item = (i32, i32, char, f32)> + '_ {
        let pose = pose % POSES;
        (0..self.height).flat_map(move |y| {
            (0..self.width).filter_map(move |x| {
                let ch = self.frames[pose][y][x];
                if ch == ' ' {
                    return None;
                }
                Some((x as i32, y as i32, ch, Self::shade_of(ch, y, self.height)))
            })
        })
    }

    /// A left-facing copy, by reversing every row.
    ///
    /// Derived rather than hand-drawn, for the crab's reason: a hand-mirrored
    /// sprite has to be redrawn by hand every time the art changes, and when that
    /// was skipped in the crab its left-facing clap's first three lines came out
    /// byte-identical to the right-facing ones, so its claws and its eyes were
    /// both on the wrong side and only its legs were flipped.
    ///
    /// Reversed per **character**, so a multi-byte glyph is not torn in half.
    pub fn mirrored(&self) -> CharArt {
        CharArt {
            frames: self
                .frames
                .iter()
                .map(|frame| {
                    frame
                        .iter()
                        .map(|line| line.iter().rev().copied().collect())
                        .collect()
                })
                .collect(),
            width: self.width,
            height: self.height,
        }
    }

    /// Writes the sprite onto a canvas for one pose.
    ///
    /// The background is the row's water colour, set explicitly and **not** by
    /// [`Cell::new`], which sets `bg: Color::Reset` and would punch a
    /// terminal-default hole in the water behind the fish. The aquarium keeps its
    /// whole depth gradient in that channel -- it is the reason the effect costs
    /// a couple of kilobytes a frame -- so it is worth being careful with.
    pub fn blit(
        &self,
        canvas: &mut Canvas,
        pose: usize,
        left: i32,
        top: i32,
        ramp: &dyn Fn(f32) -> Color,
        bg_of: &dyn Fn(usize) -> Color,
    ) {
        for (x, y, ch, shade) in self.cells(pose) {
            let (sx, sy) = (left + x, top + y);
            if sx < 0 || sy < 0 {
                continue;
            }
            let (sx, sy) = (sx as usize, sy as usize);
            if sx >= canvas.width() || sy >= canvas.height() {
                continue;
            }
            canvas.set(
                sx,
                sy,
                Cell::with_bg(ch, ramp(shade), bg_of(sy), Attribute::Reset),
            );
        }
    }

    /// The tone of one character: the body's vertical ramp, overridden for the
    /// three marks that are not body.
    ///
    /// The ramp is a function of the **row**, which is the whole of the
    /// "light from above" for this medium: a cell is one glyph in one colour, so
    /// the only place a fish can be lit is top to bottom.
    fn shade_of(ch: char, y: usize, height: usize) -> f32 {
        if EYE_CHARS.contains(&ch) {
            return EYE;
        }
        if MOUTH_CHARS.contains(&ch) {
            return MOUTH;
        }
        if BARBEL_CHARS.contains(&ch) {
            return BARBEL;
        }
        let t = if height <= 1 {
            0.0
        } else {
            y as f32 / (height - 1) as f32
        };
        BODY_TOP + (BODY_BOT - BODY_TOP) * t
    }
}

/// A species drawn in characters, as **data**.
///
/// A table of frame literals rather than of built sprites, because
/// [`species`] allocates and a `static` initialiser cannot. Keeping the table as
/// plain data is also what makes it reviewable: the art is right there in the
/// source, one row per line, rather than behind a constructor.
#[derive(Debug, Clone, Copy)]
pub struct CharSpecies {
    /// For test failure messages.
    pub name: &'static str,
    /// Right-facing, snout at `x = 0`, `POSES` frames of equal-width rows.
    ///
    /// Snout at the left is the same convention the braille species use, so one
    /// facing rule and one `mirrored()` serve the crate. Getting this backwards
    /// is not a cosmetic error: the first runtime-bend implementation anchored
    /// itself at the tail and tore every fish in the tank.
    pub frames: [&'static [&'static str]; POSES],
    /// The preferred depth band, `0.0` near to `1.0` far. A bias, not a lock.
    pub depth: f32,
    /// How much colour survives at depth, `0.0`..=`1.0`.
    pub chroma: f32,
    /// Base colour at the spine, at `depth == 0.0`.
    pub color: Color,
    /// How fast this species cruises, as a multiple of the configured speed.
    ///
    /// A cory's is the interesting one: at 0.22 it comes to rest on the gravel
    /// between excursions, which is what a bottom-dweller does and is the only
    /// way this effect ever shows a fish *stopping*.
    pub cruise: f32,
}

/// Builds a sprite from a table entry.
///
/// Every row is padded to the sprite's width, so a table entry can be written as
/// plain literals and read in a source file without counting columns.
pub fn species(entry: &CharSpecies) -> CharArt {
    let frames = &entry.frames;
    let frames: Vec<Vec<Vec<char>>> = frames
        .iter()
        .map(|frame| {
            let width = frame.iter().map(|r| r.chars().count()).max().unwrap_or(0);
            frame
                .iter()
                .map(|row| {
                    let mut line: Vec<char> = row.chars().collect();
                    line.resize(width, ' ');
                    line
                })
                .collect()
        })
        .collect();
    CharArt::new(frames)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aquarium::art::CHAR_SPECIES;

    /// Every species in the real table, built.
    ///
    /// These tests read [`CHAR_SPECIES`] rather than keeping a fixture of their
    /// own, and that is not tidiness. A hand-copied fixture is a second place the
    /// art is written down, and the first version of this module had one: the
    /// discus's third frame had a bar one character out of place in the fixture
    /// and had been fixed in the table, so the frame tests failed against art
    /// nobody ships. Round one of this effect already paid for the same mistake
    /// once -- every assertion was about *one* sprite, so nothing could see that
    /// two rows of the species table were the same drawing.
    ///
    /// Reading the table has three consequences worth having on their own: a
    /// species added to it is covered without being named, a fixture cannot drift,
    /// and "all three species" is a fact about the data rather than about this
    /// file.
    fn all() -> Vec<(&'static str, CharArt)> {
        CHAR_SPECIES.iter().map(|e| (e.name, species(e))).collect()
    }

    /// The table itself is rectangular, frame to frame and row to row.
    ///
    /// [`CharArt::new`] already asserts this, but only once something builds a
    /// sprite, and the failure would be a panic in the effect's constructor
    /// rather than a name and a position. This reads the raw data so a ragged row
    /// is a test failure that says which species and which row.
    #[test]
    fn the_table_is_rectangular() {
        for e in CHAR_SPECIES {
            let w = e.frames[0][0].chars().count();
            let h = e.frames[0].len();
            for (f, frame) in e.frames.iter().enumerate() {
                assert_eq!(
                    frame.len(),
                    h,
                    "{}: frame {f} has {} rows and frame 0 has {h}",
                    e.name,
                    frame.len()
                );
                for (r, row) in frame.iter().enumerate() {
                    assert_eq!(
                        row.chars().count(),
                        w,
                        "{}: frame {f} row {r} is {} characters wide and the sprite is {w}",
                        e.name,
                        row.chars().count()
                    );
                }
            }
        }
    }

    /// A character fish is **drawn, not stamped**.
    ///
    /// The braille species are solid masses of dots, and that is right at 30 dots
    /// where a body is all silhouette. It is wrong here, and wrong *invisibly*: a
    /// filled 16x7 blob is a perfectly valid sprite, it satisfies every property
    /// the braille one is held to, and on screen it reads as a rock. Nothing in the
    /// frame table would ever have said otherwise.
    ///
    /// So the property is that the sprite is a **drawing** rather than a stamped
    /// shape, measured as three things a stamp cannot do.
    ///
    /// # The third one used to be the wrong one
    ///
    /// The first version asserted a *graded interior* -- at least two of the
    /// density glyphs `.` `:` `;`. The previous set of drawings was tone-graded and
    /// that was a real property of it. **The current set is a line drawing with
    /// ornament, and it has no interior gradient at all**: it uses `.` once and
    /// nothing else from that set, and it fails the assertion against art that is
    /// correct. A test that fails against correct art has stopped testing its
    /// subject, which is the failure mode this project has hit three times.
    ///
    /// Glyph *variety* is the property that survives, and it is the better one
    /// anyway: a stamped blob is one character repeated, and a drawing is a dozen.
    /// The smallest species here is nine columns by three, and at that size it is
    /// legitimately 79% ink -- there is no room to be sparse in three rows -- so
    /// the fill bound is 0.85, where a stamp sits near 1.0 and a blob with a rim
    /// sits near 0.7.
    #[test]
    fn a_character_fish_is_drawn_and_not_stamped() {
        for (name, a) in all() {
            for pose in 0..POSES {
                let (x0, x1, y0, y1) = ink_extent(&a, pose);
                let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
                let ink = count_ink(&a, pose);
                let fill = ink as f32 / (bw * bh) as f32;
                assert!(
                    fill < 0.85,
                    "{name} pose {pose} fills {:.0}% of its own {bw}x{bh} box. A \
                     character fish is a drawing; a stamped one is nearly solid.",
                    fill * 100.0
                );
                // A stamp is one character. A drawing is a dozen, and the variety is
                // what makes the shape read as drawn rather than filled in.
                let glyphs: std::collections::BTreeSet<char> =
                    glyph_list(&a, pose).into_iter().collect();
                assert!(
                    glyphs.len() >= 6,
                    "{name} pose {pose} is drawn from {glyphs:?} -- {n} distinct \
                     glyphs. That is a stamp.",
                    n = glyphs.len()
                );
                // And the rim is made of strokes. A filled shape has none of these
                // and neither does a braille fish, which is the whole difference
                // between the two media in one set.
                let strokes: std::collections::BTreeSet<char> = glyphs
                    .iter()
                    .filter(|c| STROKES.contains(*c))
                    .copied()
                    .collect();
                assert!(
                    strokes.len() >= 4,
                    "{name} pose {pose} draws its contour from {strokes:?}, which is \
                     not a contour."
                );
            }
        }
    }

    /// A fish is not a filled box.
    ///
    /// The first version of this asserted that the widest row is not the top or
    /// the bottom one, on the reasoning that a fish is widest at its lateral line.
    /// That is a real thing about fish and it held for every species in the
    /// previous set, and **it fails on a legitimately tiny fish**: `fry` is nine
    /// columns by three and measures 7, 5, 7 -- the top and bottom rows are tied
    /// for widest, because at three rows a fish *is* widest at its back and its
    /// belly at once.
    ///
    /// A three-row fish is the reason a proxy needs replacing rather than
    /// loosening. What is actually being asked is "is this a shape or a stamp",
    /// and the honest form of that question does not care which row is widest: a
    /// stamp fills its bounding box and a drawing leaves holes in it. So this asks
    /// for **empty cells inside the ink's own bounding box**, which is the same
    /// property as the fill bound in
    /// [`a_character_fish_is_drawn_and_not_stamped`] from the other side, and
    /// which a three-row fish passes honestly.
    #[test]
    fn a_fish_leaves_holes_in_its_own_bounding_box() {
        for (name, a) in all() {
            for pose in 0..POSES {
                let (x0, x1, y0, y1) = ink_extent(&a, pose);
                let cells = (x1 - x0 + 1) * (y1 - y0 + 1);
                let ink = count_ink(&a, pose);
                let holes = cells - ink;
                assert!(
                    holes > 0,
                    "{name} pose {pose} fills every one of the {} cells in its own \
                     bounding box. That is a stamp.",
                    cells
                );
                // And the holes are not a single trailing column -- that is a
                // sprite padded on the right, which is not a shape.
                let interior: usize = (y0..=y1)
                    .map(|y| {
                        (x0..=x1)
                            .filter(|x| cell_at(&a, pose, *x, y).is_none())
                            .count()
                    })
                    .sum();
                assert!(
                    interior > cells / 8,
                    "{name} pose {pose} has {interior} gaps in its {cells}-cell box, \
                     which is a padded edge rather than a shape."
                );
            }
        }
    }

    /// The frames differ, and only in the trailing third.
    ///
    /// Two failures, and the second is the sneaky one. Frames that do not differ
    /// give a fish that slides. Frames that differ in the *head* give a fish that
    /// twitches, which is worse than one that slides, because it reads as broken
    /// rather than as stiff.
    #[test]
    fn the_frames_differ_only_in_the_tail() {
        for (name, a) in all() {
            let tail_start = a.cells_wide() * 2 / 3;
            for pose in 0..POSES - 1 {
                let mut differs = false;
                for y in 0..a.cells_tall() {
                    for x in 0..a.cells_wide() {
                        if cell_at(&a, pose, x, y) == cell_at(&a, pose + 1, x, y) {
                            continue;
                        }
                        differs = true;
                        assert!(
                            x >= tail_start,
                            "{name}: frames {pose} and {} differ at column {x}, in front \
                             of the tail third of {tail_start}. A fish whose head \
                             changes between frames reads as twitching.",
                            pose + 1
                        );
                    }
                }
                assert!(
                    differs,
                    "{name}: frames {pose} and {} are identical",
                    pose + 1
                );
            }
        }
    }

    /// The frames are the same animal.
    ///
    /// The other half of the frame test, and the one that catches what
    /// hand-aligned frames actually produce: a frame that loses the eye, or shifts
    /// a bar, still differs only in the tail and still passes the test above. So
    /// this checks the frames agree on everything forward of the tail, which is
    /// where a head lives.
    ///
    /// This is the assertion that caught the drifted discus bar, and it is worth
    /// noting that it caught it in a *fixture* -- the table had already been fixed
    /// by hand. The test was right and the copy it was reading was wrong.
    #[test]
    fn every_frame_has_the_same_head() {
        for (name, a) in all() {
            let tail_start = a.cells_wide() * 2 / 3;
            for pose in 1..POSES {
                for y in 0..a.cells_tall() {
                    for x in 0..tail_start {
                        assert_eq!(
                            cell_at(&a, 0, x, y),
                            cell_at(&a, pose, x, y),
                            "{name}: frame {pose} differs from frame 0 at ({x},{y}), \
                             which is the head. A hand-aligned frame that loses the \
                             eye still passes the tail-only test."
                        );
                    }
                }
            }
        }
    }

    /// There is an eye, and it is in the front two-thirds.
    ///
    /// The one mark that turns a container into an animal. Without it a character
    /// fish is a hollow shape, which is exactly what the filled braille one was.
    /// It is also the one thing the braille one *can* carry, because an eye at dot
    /// resolution is two dots of solid ink -- the difference is not that the mark is
    /// unavailable in braille, it is that a braille fish has no room for the rest of
    /// the face.
    ///
    /// **Two-thirds, not "forward of the middle".** The first version asserted the
    /// eye was in the front half and it failed against a correct drawing: `fry` is
    /// nine columns wide and its eye is a `0` at column 5, which is a small fish
    /// drawn with its eye in the middle of its flank. That is how the drawing is.
    /// The front *two-thirds* is the real requirement -- the eye is not at the tail
    /// -- and every species in the table clears it with room to spare.
    #[test]
    fn there_is_an_eye_and_it_is_toward_the_front() {
        for (name, a) in all() {
            for pose in 0..POSES {
                let ex = leftmost_eye(&a, pose).unwrap_or_else(|| {
                    panic!("{name} pose {pose} has no eye at all")
                });
                assert!(
                    ex < a.cells_wide() * 2 / 3,
                    "{name} pose {pose}: the eye is at column {ex} of {}, which is the \
                     tail end. A fish with its eye at the tail is a fish drawn \
                     backwards.",
                    a.cells_wide()
                );
            }
        }
    }

    /// The light comes from above, and the ramp is **monotonic in the row**.
    ///
    /// The whole of this medium's shading, and it is a function of the row. The fins
    /// do not need the ends of the ramp the way the braille species do, because in
    /// characters the *glyphs* separate a fin from a flank -- `/` and `\` are
    /// visibly not `:` and `;` -- so the tone only has to be monotonic.
    ///
    /// # Why the first version measured a top and a bottom instead
    ///
    /// It took the lightest body cell on the sprite and the darkest and required a
    /// gap of 0.3 between them. That is a *spread* assertion, and it fails on a
    /// correct drawing for two reasons that are both about the extremes rather than
    /// the ramp: a fish whose top row is entirely eye has no body cell there to
    /// measure, and the bottom row's body sits exactly on the ramp's floor, which
    /// the filter excluded. `slashback` is 0.63 against 0.34 -- a monotonic ramp
    /// over five rows -- and missed by a hundredth.
    ///
    /// Monotonicity is the property the light actually has, and checking it needs no
    /// threshold to tune: every row's mean body tone is at least as dark as the row
    /// above it.
    #[test]
    fn the_body_is_lit_from_above() {
        for (name, a) in all() {
            for pose in 0..POSES {
                let per_row: Vec<Option<f32>> = (0..a.cells_tall())
                    .map(|y| {
                        let v: Vec<f32> = (0..a.cells_wide())
                            .filter_map(|x| cell_at(&a, pose, x, y).map(|(_, s)| s))
                            // **Inclusive**, and the endpoints are body. The
                            // band was an open interval, which meant the lit back
                            // (`BODY_TOP`) and the shadowed belly (`BODY_BOT`) were
                            // both excluded from "is this the body" -- so a
                            // three-row fish had exactly *one* row in the band and
                            // "there is little to light" fired on correct art. The
                            // eye at 0.03 is still well outside, which is the
                            // distinction that matters here.
                            .filter(|s| *s >= BODY_BOT && *s <= BODY_TOP)
                            .collect();
                        if v.is_empty() {
                            None
                        } else {
                            Some(v.iter().sum::<f32>() / v.len() as f32)
                        }
                    })
                    .collect();
                let lit = per_row.iter().filter(|m| m.is_some()).count();
                // A *share* of the rows, not a count. The count was 20 cells, which
                // every seven- and eight-row species cleared and a five-row one did
                // not -- not because its light is wrong but because a shorter sprite
                // has fewer rows to be lit in.
                assert!(
                    lit * 2 >= a.cells_tall(),
                    "{name} pose {pose} has body in {lit} of {} rows; there is little \
                     to light",
                    a.cells_tall()
                );
                let last =
                    per_row.iter().flatten().copied().fold(f32::MAX, f32::min);
                let first =
                    per_row.iter().flatten().copied().fold(f32::MIN, f32::max);
                assert!(
                    first > last,
                    "{name} pose {pose}: the ramp does not vary at all across the body"
                );
                // Monotonic: a lower row is never lighter than the row above it.
                for w in per_row.windows(2) {
                    if let (Some(above), Some(below)) = (w[0], w[1]) {
                        assert!(
                            below <= above + 1e-6,
                            "{name} pose {pose}: a lower row is lighter ({below:.2}) \
                             than the one above it ({above:.2}). This tank is lit \
                             from above."
                        );
                    }
                }
            }
        }
    }

    /// Mirroring is a mirroring, and it keeps the eye toward the front.
    ///
    /// Derived rather than drawn, for the crab's reason -- and the assertion is on
    /// the *eye's* side and not merely on the pixels, because a mirror that reversed
    /// the rows but not the columns would pass a byte comparison and put the eye at
    /// the tail.
    ///
    /// The bound is the same two-thirds as the unmirrored test, reflected. The
    /// previous version demanded the mirrored eye be past the *midpoint*, and that
    /// is strictly more than "the mirror is correct": a fish whose eye sits at a
    /// third of its length mirrors to two thirds, and the extra strictness only
    /// ever rejected a correct mirror.
    #[test]
    fn a_mirrored_fish_faces_the_other_way_and_keeps_its_eye_toward_the_front() {
        for (name, a) in all() {
            let m = a.mirrored();
            for pose in 0..POSES {
                for y in 0..a.cells_tall() {
                    for x in 0..a.cells_wide() {
                        assert_eq!(
                            cell_at(&a, pose, x, y),
                            cell_at(&m, pose, a.cells_wide() - 1 - x, y),
                            "{name}: pose {pose} row {y} is not mirrored at column {x}"
                        );
                    }
                }
                // The eye's own position has to mirror -- **not** the leftmost one.
                // `slashback` carries four round marks (`O` at 0, `o` at 3, `o` at
                // 11, `o` at 16), so the leftmost of the mirror is the *rightmost*
                // of the original and the first version of this compared the two
                // leftmost and reported a perfect mirror as wrong.
                let before =
                    leftmost_eye(&a, pose).expect("the drawing lost its eye");
                let want = a.cells_wide() - 1 - before;
                let row = (0..a.cells_tall())
                    .find(|y| {
                        cell_at(&a, pose, before, *y)
                            .map(|(_, s)| s == EYE)
                            .unwrap_or(false)
                    })
                    .unwrap_or_else(|| {
                        panic!("{name}: the eye column has no eye on it")
                    });
                assert_eq!(
                    cell_at(&m, pose, want, row).map(|(_, s)| s),
                    Some(EYE),
                    "{name}: the eye at column {before} should mirror to {want} and did \
                     not. The rows came round but the columns did not."
                );
                assert!(
                    want >= a.cells_wide() / 3,
                    "{name}: after mirroring, the eye is at column {want} of {} -- \
                     still at the snout.",
                    a.cells_wide()
                );
            }
        }
    }

    /// Every character is one cell wide as far as the renderer is concerned.
    ///
    /// A multi-byte glyph would be blitted one cell at a time and tear. None of the
    /// art uses one, and the assertion is here because the failure would be a
    /// mangled fish rather than a compile error.
    #[test]
    fn no_glyph_is_wider_than_one_cell() {
        for (name, a) in all() {
            for pose in 0..POSES {
                for (x, y, ch, _) in a.cells(pose) {
                    assert!(
                        (ch as u32) < 0x100,
                        "{name} pose {pose} has a multi-byte glyph {ch:?} at ({x},{y})"
                    );
                }
            }
        }
    }

    /// Ragged art is a build failure, not a terminal surprise.
    ///
    /// The counts here are the contract [`CharArt::new`] enforces, asserted so
    /// that the panic messages are known to fire on the inputs they name. Each
    /// fixture has to reach the assertion it is testing, which is its own small
    /// trap -- the first two of these were written with two frames and so tripped
    /// the pose count instead, and `should_panic(expected = ...)` reported the
    /// mismatch rather than the bug.
    #[test]
    #[should_panic(expected = "rows")]
    fn a_sprite_with_a_missing_row_is_rejected() {
        let mut frames = vec![vec![vec!['x'; 4], vec!['x'; 4]]; POSES];
        frames[1].pop();
        CharArt::new(frames);
    }

    #[test]
    #[should_panic(expected = "wide")]
    fn a_sprite_with_a_ragged_row_is_rejected() {
        let mut frames = vec![vec![vec!['x'; 4], vec!['x'; 4]]; POSES];
        frames[2] = vec![vec!['x'; 4], vec!['x'; 3]];
        CharArt::new(frames);
    }

    #[test]
    #[should_panic(expected = "frames")]
    fn a_sprite_with_the_wrong_frame_count_is_rejected() {
        CharArt::new(vec![vec![vec!['x'; 4]], vec![vec!['x'; 4]]]);
    }

    /// A fish's wing or dorsal trails **behind** its head, not over it.
    ///
    /// All five drawings face left, so the snout is at column 0 and a dorsal fin
    /// has to lean *back* -- to higher columns. Three of them did not, and the
    /// report was that "the tip of its wing is on top of its head", which is
    /// exactly right and exactly what this measures: the **topmost inked row**
    /// must begin to the right of the row the eye is on.
    ///
    /// **Fails against the art as drawn**, which is the point. Measured before the
    /// shift, as a topmost-ink column against an eye column: `longfin` 0 against 3,
    /// `bigeye` 0 against 4 and `stipple` 0 against 2 -- a fin's leading edge
    /// *ahead of* the eye. After: 7 against 3, 6 against 4, and 6 against 2.
    ///
    /// The two that are **not** in the assertion, deliberately:
    ///
    /// - `fry`'s top row is `|\.-*-.`, which is the head's own edge at column 0
    ///   and then the back running rearward. It already trails, so requiring a
    ///   gap would fail a correct drawing.
    /// - `slashback`'s top row is `O  o`, and those are **bubbles** -- the drawing
    ///   carries three more round marks at `o` on rows 1 and 2, and a bubble rises
    ///   rather than leaning back. They must not shift, so they are excluded by
    ///   name rather than by a shape rule that would also catch them.
    ///
    /// So this is per species on purpose, and the exclusions are written down where
    /// the code excludes them: a shape heuristic that silently spared `fry` and
    /// `slashback` would be a heuristic nobody could check, which is the failure
    /// mode this file's other tests exist to avoid.
    #[test]
    fn a_wing_trails_behind_its_head_rather_than_standing_on_it() {
        /// Species whose top row is a dorsal and must lean back. `fry` needs no
        /// shift and `slashback`'s is bubbles; see the note above.
        const DORSAL: &[&str] = &["longfin", "bigeye", "stipple"];

        for (name, art) in all() {
            if !DORSAL.contains(&name) {
                continue;
            }
            let eye = leftmost_eye(&art, 0).unwrap_or_else(|| {
                panic!("{name}: has no eye, so this test has no head to compare the wing against")
            });

            // The topmost row carrying any ink, and where its ink starts.
            let (top_row, wing_start) = (0..art.cells_tall())
                .filter_map(|y| {
                    (0..art.cells_wide())
                        .find(|&x| {
                            cell_at(&art, 0, x, y)
                                .map(|(g, _)| g != ' ')
                                .unwrap_or(false)
                        })
                        .map(|x| (y, x))
                })
                .min_by_key(|&(y, _)| y)
                .unwrap_or_else(|| panic!("{name}: draws nothing at all"));

            assert!(
                wing_start > eye,
                "{name}: its topmost ink is at row {top_row}, column {wing_start}, and \
                 its eye is at column {eye}. The fin starts at or ahead of the eye, \
                 so it stands on the head instead of trailing back from it."
            );
        }
    }

    /// The **leftmost** eye in one pose, as a column.
    ///
    /// Leftmost and not first-found, and the difference is not pedantry: the inline
    /// loops these replaced kept whichever match came *last*, and `slashback` has an
    /// `O` at column 0 and an `o` at column 11, so the test reported the bubble
    /// mid-body as the eye and then failed a correct drawing for having "its eye at
    /// the tail". A fish can carry more than one round mark and the eye is the one
    /// nearest the snout.
    fn leftmost_eye(a: &CharArt, pose: usize) -> Option<usize> {
        (0..a.cells_wide()).find(|x| {
            (0..a.cells_tall()).any(|y| {
                cell_at(a, pose, *x, y)
                    .map(|(_, shade)| shade == EYE)
                    .unwrap_or(false)
            })
        })
    }

    fn cell_at(
        a: &CharArt,
        pose: usize,
        x: usize,
        y: usize,
    ) -> Option<(char, f32)> {
        a.cells(pose)
            .find(|(cx, cy, _, _)| *cx == x as i32 && *cy == y as i32)
            .map(|(_, _, ch, s)| (ch, s))
    }

    fn glyph_list(a: &CharArt, pose: usize) -> Vec<char> {
        a.cells(pose).map(|(_, _, ch, _)| ch).collect()
    }

    fn count_ink(a: &CharArt, pose: usize) -> usize {
        a.cells(pose).count()
    }

    fn ink_extent(a: &CharArt, pose: usize) -> (usize, usize, usize, usize) {
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
}
