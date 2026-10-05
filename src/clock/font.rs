//! The digit art, and why it is authored in *source cells* rather than in dots.
//!
//! # A braille dot is square, so a square source cell is the unit
//!
//! A terminal cell is one unit wide and about two tall. Braille packs two dots
//! across and four down, so each dot is 0.5 by 0.5 -- **square**. A glyph drawn
//! on a square grid therefore comes out undistorted, and the aspect-ratio caveat
//! at the top of [`crate::render`] does not apply to it.
//!
//! The art below is authored on a 5x7 grid of *source cells*, and each source
//! cell is expanded to a [`CELL_DOTS`]-by-[`CELL_DOTS`] block of dots. A digit is
//! 10 by 14 dots, which is 5 by 7 cells on screen: five units wide and fourteen
//! tall, which is the correct rendering of a 5x7 character-cell digit. Authoring
//! at the cell grid rather than the dot grid keeps the art readable in a diff --
//! five characters per row instead of ten -- and the expansion is one line of
//! [`bitmap`].
//!
//! # The colon is one source cell wide, which is a constraint and a choice
//!
//! Every glyph's width must be a whole number of source cells, because the
//! expansion is a fixed 2x2 block. A three-wide colon -- the usual shape -- is
//! not, so the colon is authored at one cell (four dots) with its ink on rows 2
//! and 5.
//!
//! **Rows 2 and 5 rather than 0 and 6**, and that is the whole difference between
//! a colon and two full stops stacked at the extremes. Each source row becomes two
//! dot rows, so rows 2 and 5 land on dot rows 4 and 10 of fourteen: upper-middle
//! and lower-middle. A colon whose dots sit at the very top and bottom reads as
//! a full stop twice, and on a seven-row glyph there is barely a third of the
//! height between them for the eye to do that reading with.
//!
//! # The font is eleven glyphs, and that is a decision
//!
//! Ten digits and a colon. A date line would want letters, a slash and a comma
//! -- roughly forty more hand-authored glyphs -- and hand-drawn art is the part
//! of this crate that has most often been wrong in ways no test could see. See
//! [`crate::aquarium::charart`]: five hand-drawn species whose art was reviewed
//! character by character, and a wave amplitude shipped at 3.1x the design
//! because a comment described the bug accurately enough to read as a spec.
//!
//! Eleven glyphs can be checked by eye against the table below, in one sitting.

/// Width of the design grid, in source cells. Every digit is exactly this wide.
pub const GLYPH_W: usize = 5;

/// Height of the design grid, in source cells. Every glyph is exactly this tall.
pub const GLYPH_H: usize = 7;

/// Source cells expanded to dots, on both axes.
///
/// Equal to [`crate::render::braille::DOTS_X`], and that equality is load-bearing
/// rather than coincidental: it is what makes a glyph an even number of dots
/// across, so a glyph never straddles a cell boundary and no drawing has to carry
/// a spare column for the skew. [`crate::dvd`] does need that column, because its
/// logo moves to an arbitrary dot offset; this art never does.
pub const CELL_DOTS: usize = 2;

/// A glyph's ink, rasterised into braille dot space at scale 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    /// Width in dots. `GLYPH_W * CELL_DOTS` for a digit.
    pub width: usize,
    /// Height in dots. `GLYPH_H * CELL_DOTS` for every glyph.
    pub height: usize,
    /// Row-major, `width * height`, `true` where the dot is raised.
    pub dots: Vec<bool>,
}

impl Bitmap {
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.width && y < self.height && self.dots[y * self.width + x]
    }

    /// How many dots are raised.
    pub fn ink(&self) -> usize {
        self.dots.iter().filter(|d| **d).count()
    }

    /// The smallest box containing every raised dot, as `(x0, y0, x1, y1)`, or
    /// `None` for a glyph with no ink at all.
    ///
    /// Inclusive on both ends. For the silhouette comparison, which is a
    /// comparison of *where the ink is* rather than of how much of it there is.
    pub fn ink_extent(&self) -> Option<(usize, usize, usize, usize)> {
        let mut bounds: Option<(usize, usize, usize, usize)> = None;
        for y in 0..self.height {
            for x in 0..self.width {
                if !self.get(x, y) {
                    continue;
                }
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => {
                        (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                    }
                });
            }
        }
        bounds
    }
}

/// The character a source cell uses for ink. Everything else is background.
const ON: char = '#';

/// The character a source cell uses for background.
///
/// Its counterpart to [`ON`] rather than a fourth spelling of it: the tests use
/// this one and the art uses [`OFF_STR`], and having two names for background
/// would let one of them be wrong without anything noticing.
#[cfg(test)]
const OFF: char = '.';

/// The design, as art rather than as numbers.
///
/// Widths are **read off the rows** rather than declared in a second column,
/// which is the `plasma` lesson: a derived table with no test against its source
/// is a second place the same thing is written down. A glyph whose rows disagree
/// is caught by `every_row_of_every_glyph_is_the_same_width` instead of being
/// silently reconciled here.
///
/// The zero is slashed. There are no letters in this font, so a slashed zero is
/// not buying a distinction from `O` -- it is there because a big circle with a
/// hollow middle reads as a ring at a glance, and the slash gives the eye
/// something to land on at 78 dots across.
const INK: &[(char, [&str; GLYPH_H])] = &[
    (
        '0',
        [
            ".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###.",
        ],
    ),
    (
        '1',
        [
            "..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###.",
        ],
    ),
    (
        '2',
        [
            ".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####",
        ],
    ),
    (
        '3',
        [
            "#####", "...#.", "..#..", "...#.", "....#", "#...#", ".###.",
        ],
    ),
    (
        '4',
        [
            "...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#.",
        ],
    ),
    (
        '5',
        [
            "#####", "#....", "####.", "....#", "....#", "#...#", ".###.",
        ],
    ),
    (
        '6',
        [
            "..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '7',
        [
            "#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#...",
        ],
    ),
    (
        '8',
        [
            ".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###.",
        ],
    ),
    (
        '9',
        [
            ".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##..",
        ],
    ),
    // One source cell wide, and rows 2 and 5 rather than 0 and 6. See the module
    // docs: each source row becomes two dot rows, so these land on dot rows 4 and
    // 10 of fourteen -- a colon in the middle rather than two full stops at the
    // extremes.
    (
        SEPARATOR,
        [OFF_STR, OFF_STR, ON_STR, OFF_STR, OFF_STR, ON_STR, OFF_STR],
    ),
];

/// [`ON`] as a `&'static str`, because a `const` array of `str` cannot hold a
/// `char`. The two spell the same thing and the art above is the only place they
/// are both needed.
const ON_STR: &str = "#";
/// [`ON_STR`]'s counterpart. See its note.
const OFF_STR: &str = ".";

/// Every character this font can draw, digits first.
pub const DIGITS: [char; 10] = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

/// The separator between the fields of a time.
pub const SEPARATOR: char = ':';

/// The design grid size of `c`, in source cells, or `None` if it is not in the
/// font.
///
/// The widest row is the width, so a ragged glyph reports its own inconsistency
/// rather than being quietly trimmed. See [`INK`].
pub fn cell_width(c: char) -> Option<usize> {
    art(c).map(|rows| {
        rows.iter()
            .map(|row| row.chars().count())
            .max()
            .unwrap_or(0)
    })
}

/// A glyph's ink in dot space at scale 1.
///
/// Characters outside the font are `None`, not a blank glyph: a clock that
/// cannot draw a character should fail a test rather than print a hole.
pub fn bitmap(c: char) -> Option<Bitmap> {
    let rows = art(c)?;
    let width = rows
        .iter()
        .map(|row| row.chars().count())
        .max()
        .unwrap_or(0)
        * CELL_DOTS;
    let height = GLYPH_H * CELL_DOTS;

    let mut dots = vec![false; width * height];
    for (cell_y, row) in rows.iter().enumerate() {
        for (cell_x, ch) in row.chars().enumerate() {
            if ch != ON {
                continue;
            }
            // The 2x2 block. `cell_x * CELL_DOTS` rather than a nested loop over
            // the block, so the expansion is one expression and the scale factor
            // has somewhere obvious to go if this is ever generalised.
            let (x0, y0) = (cell_x * CELL_DOTS, cell_y * CELL_DOTS);
            for dy in 0..CELL_DOTS {
                for dx in 0..CELL_DOTS {
                    dots[(y0 + dy) * width + x0 + dx] = true;
                }
            }
        }
    }

    Some(Bitmap {
        width,
        height,
        dots,
    })
}

fn art(c: char) -> Option<&'static [&'static str; GLYPH_H]> {
    INK.iter()
        .find(|(glyph, _)| *glyph == c)
        .map(|(_, rows)| rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The font is exactly the ten digits and a separator, and nothing else.
    ///
    /// Written out rather than derived from [`DIGITS`], so adding a letter to
    /// [`INK`] without thinking about this fails here. It is the change detector
    /// for the one thing that grew the font's review cost in the first place.
    #[test]
    fn the_font_is_the_digits_and_a_separator_and_nothing_else() {
        let listed: Vec<char> = INK.iter().map(|(c, _)| *c).collect();
        let mut expected: Vec<char> = DIGITS.to_vec();
        expected.push(SEPARATOR);
        expected.sort_unstable();
        assert_eq!(
            listed.len(),
            expected.len(),
            "the font has {} glyphs, expected {}",
            listed.len(),
            expected.len()
        );
        for c in expected {
            assert!(
                listed.contains(&c),
                "{c:?} is missing from the font, or spelled differently"
            );
        }
        assert_eq!(
            listed.len(),
            11,
            "the font grew. It is eleven hand-authored glyphs on purpose, and a \
             larger table needs a deliberate decision about reviewing it -- not a \
             character added while fixing something else."
        );
    }

    /// Every glyph is the same box, except the separator, which is narrower.
    ///
    /// The width is *read* off the rows rather than declared, so this is the only
    /// thing standing between a ragged row and a glyph whose right-hand column
    /// silently vanishes. `bitmap` clamps to the widest row for the same reason.
    ///
    /// **The separator is exempt, and the exemption is the reason the rest is
    /// asserted at all.** It has to be narrower -- three source cells cannot be
    /// expanded to a whole number of dots at [`CELL_DOTS`] each -- so "every
    /// glyph is [`GLYPH_W`] wide" is false and would be a threshold nobody could
    /// satisfy. The list is written out rather than derived from the art so that
    /// a *second* narrow glyph has to be added here, where the sentence says why.
    #[test]
    fn every_row_of_every_glyph_is_the_same_width() {
        /// Glyphs narrower than the design grid, and why each one is.
        const NARROWER: &[(char, usize)] = &[(SEPARATOR, 1)];

        for (c, rows) in INK {
            let widths: Vec<usize> =
                rows.iter().map(|row| row.chars().count()).collect();
            let first = widths[0];
            for (y, width) in widths.iter().enumerate() {
                assert_eq!(
                    *width, first,
                    "the {c:?} glyph's row {y} is {width} wide and its row 0 is \
                     {first}. `bitmap` takes the widest row, so the short row's \
                     missing cells are drawn as background and the glyph is \
                     quietly the wrong shape."
                );
            }

            let expected = NARROWER
                .iter()
                .find(|(glyph, _)| *glyph == *c)
                .map(|(_, width)| *width)
                .unwrap_or(GLYPH_W);
            assert_eq!(
                first, expected,
                "{c:?} is {first} source cells wide and the design grid is {GLYPH_W}"
            );
            // And the exemption has to be total: a narrow glyph the list does not
            // know about fails above, and a listed glyph that has quietly been
            // widened to the grid would fail the `unwrap_or` arm.
            assert!(
                expected <= GLYPH_W,
                "{c:?} is listed as narrower than the grid at {expected}, which is \
                 not narrower"
            );
        }
    }

    /// Only the two characters that mean ink appear in the art.
    ///
    /// Catches the failure that is invisible on screen: a space where `#` was
    /// meant is a digit with a hole in it, and a digit with a hole in it still
    /// looks like a digit at a glance.
    #[test]
    fn the_art_uses_only_ink_and_background() {
        for (c, rows) in INK {
            for (y, row) in rows.iter().enumerate() {
                for (x, ch) in row.chars().enumerate() {
                    assert!(
                        ch == ON || ch == OFF,
                        "the {c:?} glyph has {ch:?} at ({x}, {y}); only {ON:?} and \
                         {OFF:?} are meaningful"
                    );
                }
            }
        }
    }

    /// A digit rasterises to the box the design grid implies, with the ink in
    /// the right place.
    ///
    /// The specific claim is the expansion: `bitmap` turns each source cell into
    /// a `CELL_DOTS` square, so a dot is raised exactly where its 2x2 block is,
    /// and a digit is `GLYPH_W * CELL_DOTS` by `GLYPH_H * CELL_DOTS`.
    #[test]
    fn a_source_cell_expands_to_a_square_block_of_dots() {
        let zero = bitmap('0').expect("0 is in the font");
        assert_eq!(zero.width, GLYPH_W * CELL_DOTS);
        assert_eq!(zero.height, GLYPH_H * CELL_DOTS);

        // Row 0 of `0` is `.###.`, so source cells 1, 2 and 3 are ink and 0 is
        // not. Each of those three expands to a 2x2 block, so the raised dots on
        // dot rows 0 and 1 are columns 2 through 7 and nothing else -- which is
        // the expansion working rather than the art merely being readable.
        for dot_x in 0..zero.width {
            let in_source_block = (2..8).contains(&dot_x);
            assert_eq!(
                zero.get(dot_x, 0),
                in_source_block,
                "dot ({dot_x}, 0) is {} but three 2x2 blocks from column 2 say it \
                 should be {}",
                zero.get(dot_x, 0),
                in_source_block
            );
            // Row 1 is the *same* source row, so it must be identical. If the
            // expansion wrote a 2-wide block rather than a 2x2 one, this is where
            // it would show.
            assert_eq!(
                zero.get(dot_x, 1),
                in_source_block,
                "dot ({dot_x}, 1) disagrees with ({dot_x}, 0): the block is not 2x2"
            );
        }
        // And dot row 2 belongs to source row 1, which is `#...#`.
        assert!(zero.get(0, 2), "source row 1 starts with ink");
        assert!(!zero.get(2, 2), "source row 1 has a gap in the middle");

        // And every raised dot is accounted for by exactly one inked source
        // cell, so the expansion neither drops nor duplicates a dot. Counted
        // rather than asserted against a literal: the number is `inked_cells *
        // CELL_DOTS^2` and saying so is the point.
        let inked_cells = INK
            .iter()
            .find(|(c, _)| *c == '0')
            .map(|(_, rows)| {
                rows.iter()
                    .map(|row| row.chars().filter(|ch| *ch == ON).count())
                    .sum::<usize>()
            })
            .expect("0 is in the font");
        assert_eq!(
            zero.ink(),
            inked_cells * CELL_DOTS * CELL_DOTS,
            "the rasterised zero has {} dots but its art has {inked_cells} inked \
             source cells",
            zero.ink()
        );
    }

    /// The colon's two dots sit inside the glyph, not at its extremes.
    ///
    /// **This is the test for a full stop drawn twice.** A colon is defined by
    /// where its dots are *not*: ink on row 0 or row `GLYPH_H - 1` reads as two
    /// full stops stacked at the top and bottom of the cell rather than as one
    /// colon in the middle of it, and the difference is obvious on screen and
    /// invisible in a diff of this file.
    ///
    /// **Counted in blocks, not dots.** One inked source row expands to a
    /// [`CELL_DOTS`]-dot-tall block, so a correctly drawn colon has *four*
    /// raised dot rows, not two. The first version of this test asserted
    /// `raised.len() == 2` and would have failed against correct art while
    /// passing against an off-by-one that drew half a dot -- so the count is
    /// against blocks, and the blocks are checked for being whole.
    #[test]
    fn the_colon_is_centred_rather_than_sitting_at_the_extremes() {
        let colon = bitmap(SEPARATOR).expect("the separator is in the font");
        assert_eq!(
            colon.width, CELL_DOTS,
            "the colon is one source cell wide, so {} dots across",
            CELL_DOTS
        );

        let raised: Vec<usize> = (0..colon.height)
            .filter(|y| (0..colon.width).any(|x| colon.get(x, *y)))
            .collect();
        assert_eq!(
            raised.len(),
            2 * CELL_DOTS,
            "a colon is two {CELL_DOTS}-dot blocks, got dot rows {raised:?}"
        );
        // Whole blocks, each `CELL_DOTS` rows: a partial one is a half-drawn dot
        // and reads as a smudge rather than a separator.
        let blocks: Vec<&[usize]> = raised.chunks(CELL_DOTS).collect();
        assert_eq!(blocks.len(), 2, "the colon is two blocks, got {blocks:?}");
        for block in &blocks {
            assert_eq!(
                block.len(),
                CELL_DOTS,
                "the colon has a partial block at {block:?}"
            );
            // And every dot across the block, so it is a square and not a stripe.
            let top = block[0];
            assert!(
                (0..colon.width).all(|x| colon.get(x, top)),
                "the colon's block at dot row {top} is not {CELL_DOTS} dots wide"
            );
        }

        let (first, second) = (blocks[0][0], blocks[1][0]);
        // Neither block touches an extreme row.
        assert!(first > 0, "the colon's first block starts on dot row 0");
        assert!(
            second + CELL_DOTS <= colon.height,
            "the colon's second block runs past the last dot row"
        );
        // One in each half of the glyph.
        assert!(
            first < colon.height / 2 && second >= colon.height / 2,
            "the colon's blocks start at dot rows {first} and {second}, which are \
             both in one half of a {} row glyph",
            colon.height
        );
        // And a real gap between them, so they read as two marks rather than one
        // thick one. `3 * CELL_DOTS` is the shipped spacing; the bound is two
        // blocks, which is the most a colon can be while still being a colon.
        assert!(
            second >= first + 2 * CELL_DOTS,
            "the colon's blocks start at dot rows {first} and {second}, with \
             nothing between them"
        );
    }

    /// Every glyph has ink. A blank glyph is a space on screen and a test that
    /// cannot see the difference between `8` and an empty box.
    #[test]
    fn no_glyph_is_empty() {
        for c in DIGITS.iter().copied().chain([SEPARATOR]) {
            let glyph = bitmap(c).expect("glyph is in the font");
            assert!(glyph.ink() > 0, "{c:?} rasterises to nothing");
        }
        assert!(bitmap('x').is_none(), "'x' is not in the font");
        assert!(bitmap(' ').is_none(), "a space is not in the font");
        assert!(cell_width('x').is_none());
    }
}
