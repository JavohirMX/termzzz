//! A two-tone bitmap at 2x2 resolution per cell, drawn with quadrant blocks.
//!
//! `▀` gives 2x vertically and `▌` gives 2x horizontally, but they cannot be
//! combined: a cell has one foreground and one background, so splitting it
//! vertically already spends both on the vertical split. The quadrant block
//! elements -- `▛` `▜` `▙` `▟` -- are the glyphs that split a cell both ways at
//! once, and they are what a logo moving diagonally needs: `▀` alone leaves the
//! horizontal axis quantised to whole cells, so a seventeen-cell-wide slab still
//! steps sideways once a cell at a time.
//!
//! # What this is for
//!
//! A *two-tone* bitmap: one ink colour and the background. That is a deliberate
//! limit rather than a missing feature. Four arbitrary colours cannot be shown in
//! two, so a general colour field at 2x2 is not expressible, and pretending
//! otherwise by averaging would be worse than the banding it fixed. What this
//! *can* do is place a shape with crisp edges on both axes, which is exactly a
//! logo, an icon, or a sprite.
//!
//! Use [`HalfBlockField`](super::HalfBlockField) for a smooth colour field, and
//! [`BrailleGrid`](super::BrailleGrid) when 4x vertical detail in one colour
//! matters more than edge crispness.
//!
//! # Font support
//!
//! `█` `▀` `▄` `▌` `▐` are universally available; the four quadrants are in
//! common use but are not in every font. That degrades locally rather than
//! globally: a solid interior is `█` and stays solid, and only the outline of the
//! shape is at risk. A renderer built on `▀` alone has no such failure mode,
//! which is the trade for the resolution.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crossterm::style::{Attribute, Color};

/// Field samples per cell, per axis.
pub const SAMPLES_PER_CELL: usize = 2;

/// All four quadrants ink. Also what a terminal shows for a missing glyph, which
/// is why the interior of a shape is safe.
pub const FULL: char = '█';
/// Top half.
pub const UPPER: char = '▀';
/// Bottom half.
pub const LOWER: char = '▄';
/// Left half.
pub const LEFT: char = '▌';
/// Right half.
pub const RIGHT: char = '▐';
/// Upper-left quadrant.
pub const UPPER_LEFT: char = '▛';
/// Upper-right quadrant.
pub const UPPER_RIGHT: char = '▜';
/// Lower-left quadrant.
pub const LOWER_LEFT: char = '▙';
/// Lower-right quadrant.
pub const LOWER_RIGHT: char = '▟';

/// Which quadrants of one cell are ink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quadrants {
    pub upper_left: bool,
    pub upper_right: bool,
    pub lower_left: bool,
    pub lower_right: bool,
}

impl Quadrants {
    /// The glyph for these quadrants, given a foreground ink and a background.
    ///
    /// The halves and quadrants all work the same way: the ink is the
    /// *foreground* and the empty part is the *background*, so `▀` with an ink
    /// foreground paints the top half in the ink and leaves the bottom half
    /// showing the background through.
    pub fn glyph(self) -> char {
        let Self {
            upper_left,
            upper_right,
            lower_left,
            lower_right,
        } = self;
        match (upper_left, upper_right, lower_left, lower_right) {
            (false, false, false, false) => ' ',
            (true, true, true, true) => FULL,
            (true, true, false, false) => UPPER,
            (false, false, true, true) => LOWER,
            (true, false, true, false) => LEFT,
            (false, true, false, true) => RIGHT,
            (true, false, false, false) => UPPER_LEFT,
            (false, true, false, false) => UPPER_RIGHT,
            (false, false, true, false) => LOWER_LEFT,
            (false, false, false, true) => LOWER_RIGHT,
            // The three-quadrant and two-diagonal cases have no glyph in the block
            // elements set. Drawn as a full block: it errs towards ink, which for
            // a logo means a slightly fatter outline rather than holes in it.
            _ => FULL,
        }
    }

    /// Whether this cell needs a background colour at all.
    ///
    /// True for a blank cell, which is written as a space, and false for a solid
    /// one, which is a single foreground. Only the genuinely split cells need both.
    pub fn needs_background(self) -> bool {
        self.glyph() != FULL && self.glyph() != ' '
    }

    /// The quadrant bits for one cell of a field, row-major within the cell.
    pub fn from_samples(
        samples: [[bool; SAMPLES_PER_CELL]; SAMPLES_PER_CELL],
    ) -> Self {
        Self {
            upper_left: samples[0][0],
            upper_right: samples[0][1],
            lower_left: samples[1][0],
            lower_right: samples[1][1],
        }
    }
}

/// An occupancy mask at 2x2 per cell, drawn with quadrant block glyphs.
#[derive(Clone)]
pub struct QuadrantMask {
    width: usize,
    height: usize,
    /// Row-major, `2 * width` by `2 * height`.
    occupied: Vec<bool>,
}

impl QuadrantMask {
    /// An empty mask of `width` by `height` cells.
    pub fn new(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            occupied: vec![
                false;
                width * SAMPLES_PER_CELL * height * SAMPLES_PER_CELL
            ],
        }
    }

    /// Width in cells.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in cells.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Width in field samples: twice the cell width.
    pub fn sample_width(&self) -> usize {
        self.width * SAMPLES_PER_CELL
    }

    /// Height in field samples: twice the cell height.
    pub fn sample_height(&self) -> usize {
        self.height * SAMPLES_PER_CELL
    }

    /// Clears every sample.
    pub fn clear(&mut self) {
        self.occupied.fill(false);
    }

    /// Resizes, discarding the contents.
    pub fn resize(&mut self, width: usize, height: usize) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        *self = Self::new(width, height);
    }

    /// Sets one field sample. Out-of-range coordinates are dropped.
    ///
    /// Dropped rather than clamped, because a logo drifting off the edge should
    /// be clipped by the edge and not smeared along it.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        if x >= self.sample_width() || y >= self.sample_height() {
            return;
        }
        let stride = self.sample_width();
        self.occupied[y * stride + x] = on;
    }

    /// Reads one field sample. Out-of-range reads as empty.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        if x >= self.sample_width() || y >= self.sample_height() {
            return false;
        }
        self.occupied[y * self.sample_width() + x]
    }

    /// The quadrants of one cell.
    pub fn cell(&self, cell_x: usize, cell_y: usize) -> Quadrants {
        let x = cell_x * SAMPLES_PER_CELL;
        let y = cell_y * SAMPLES_PER_CELL;
        Quadrants::from_samples([
            [self.get(x, y), self.get(x + 1, y)],
            [self.get(x, y + 1), self.get(x + 1, y + 1)],
        ])
    }

    /// How many field samples are ink.
    pub fn count(&self) -> usize {
        self.occupied.iter().filter(|on| **on).count()
    }

    /// Writes the mask into a canvas.
    ///
    /// Only the cells that have ink are written, so a mostly-empty mask costs
    /// almost nothing on the wire. A blank cell is left as whatever the canvas
    /// already held, which is why the caller clears or fills the canvas first.
    pub fn write_to(
        &self,
        canvas: &mut Canvas,
        ink: Color,
        background: Color,
        attr: Attribute,
    ) {
        for cell_y in 0..self.height.min(canvas.height()) {
            for cell_x in 0..self.width.min(canvas.width()) {
                let quadrants = self.cell(cell_x, cell_y);
                if quadrants == Quadrants::from_samples([[false; 2]; 2]) {
                    continue;
                }
                let cell = if quadrants.needs_background() {
                    Cell::with_bg(quadrants.glyph(), ink, background, attr)
                } else {
                    Cell::new(quadrants.glyph(), ink, attr)
                };
                canvas.set(cell_x, cell_y, cell);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quadrants(ul: bool, ur: bool, ll: bool, lr: bool) -> Quadrants {
        Quadrants {
            upper_left: ul,
            upper_right: ur,
            lower_left: ll,
            lower_right: lr,
        }
    }

    /// Every one of the sixteen arrangements has to name a glyph, and the four
    /// that do not exist in the block elements set have to say so by falling back
    /// to a full block rather than by producing nothing.
    #[test]
    fn every_arrangement_produces_a_glyph() {
        for bits in 0..16u8 {
            let q = quadrants(
                bits & 1 != 0,
                bits & 2 != 0,
                bits & 4 != 0,
                bits & 8 != 0,
            );
            let glyph = q.glyph();
            assert!(!glyph.is_control(), "{bits:04b} produced {glyph:?}");
        }
    }

    /// The four quadrants, the four halves and the full block have to be the
    /// glyphs a viewer expects, because the whole point is crisp edges.
    #[test]
    fn the_single_quadrants_use_the_quadrant_glyphs() {
        assert_eq!(quadrants(true, false, false, false).glyph(), '▛');
        assert_eq!(quadrants(false, true, false, false).glyph(), '▜');
        assert_eq!(quadrants(false, false, true, false).glyph(), '▙');
        assert_eq!(quadrants(false, false, false, true).glyph(), '▟');
    }

    /// The halves and the full block.
    #[test]
    fn the_halves_and_the_full_block_use_the_half_block_glyphs() {
        assert_eq!(quadrants(true, true, false, false).glyph(), '▀');
        assert_eq!(quadrants(false, false, true, true).glyph(), '▄');
        assert_eq!(quadrants(true, false, true, false).glyph(), '▌');
        assert_eq!(quadrants(false, true, false, true).glyph(), '▐');
        assert_eq!(quadrants(true, true, true, true).glyph(), '█');
        assert_eq!(quadrants(false, false, false, false).glyph(), ' ');
    }

    /// Only a genuinely split cell needs two colours on the wire.
    #[test]
    fn only_split_cells_need_a_background() {
        assert!(!quadrants(true, true, true, true).needs_background());
        assert!(!quadrants(false, false, false, false).needs_background());
        assert!(quadrants(true, true, false, false).needs_background());
        assert!(quadrants(true, false, false, false).needs_background());
    }

    /// Resolution has to be 2x on both axes, or this renderer is half-block with
    /// extra steps.
    #[test]
    fn the_field_is_two_samples_per_cell_on_both_axes() {
        let mask = QuadrantMask::new(10, 5);
        assert_eq!(mask.width(), 10);
        assert_eq!(mask.height(), 5);
        assert_eq!(mask.sample_width(), 20);
        assert_eq!(mask.sample_height(), 10);
    }

    /// A single sample has to be visible in exactly one quadrant of one cell.
    ///
    /// This is what a sub-cell offset actually buys: a shape edge that can sit
    /// between two columns rather than jumping a whole column at a time.
    #[test]
    fn one_sample_lights_exactly_one_quadrant() {
        for (x, y, expected) in
            [(0usize, 0usize, '▛'), (1, 0, '▜'), (0, 1, '▙'), (1, 1, '▟')]
        {
            let mut mask = QuadrantMask::new(4, 4);
            mask.set(x, y, true);
            assert_eq!(mask.cell(0, 0).glyph(), expected, "sample ({x},{y})");
            assert_eq!(mask.count(), 1);
        }
    }

    /// Out of range is dropped, not clamped. A shape drifting off the edge should
    /// be clipped by it rather than smeared along it.
    #[test]
    fn out_of_range_samples_are_dropped_rather_than_clamped() {
        let mut mask = QuadrantMask::new(4, 4);
        mask.set(99, 99, true);
        assert_eq!(
            mask.count(),
            0,
            "an out-of-range write leaked into the mask"
        );
        assert!(!mask.get(99, 99));
    }

    /// A solid rectangle has to come out solid, with no seams down the middle.
    ///
    /// This is the property that makes the renderer safe for a block-letter logo:
    /// if a solid region ever rendered as anything but `█`, the letter would be
    /// visibly striped.
    #[test]
    fn a_solid_region_renders_as_one_full_block_glyph() {
        let mut mask = QuadrantMask::new(6, 4);
        for y in 0..mask.sample_height() {
            for x in 0..mask.sample_width() {
                mask.set(x, y, true);
            }
        }
        for cell_y in 0..4 {
            for cell_x in 0..6 {
                assert_eq!(
                    mask.cell(cell_x, cell_y).glyph(),
                    FULL,
                    "cell ({cell_x},{cell_y}) of a solid region is not a full block"
                );
            }
        }
    }

    /// Writing has to stay in bounds and land where it should.
    #[test]
    fn writing_lands_in_the_right_cells() {
        let mut mask = QuadrantMask::new(4, 2);
        mask.set(0, 0, true); // Upper-left of cell (0,0).
        let mut canvas = Canvas::new(4, 2);
        let ink = Color::Rgb {
            r: 10,
            g: 20,
            b: 30,
        };
        let black = Color::Rgb { r: 0, g: 0, b: 0 };
        mask.write_to(&mut canvas, ink, black, Attribute::Reset);

        assert_eq!(canvas.get(0, 0).symbol, '▛');
        assert_eq!(canvas.get(0, 0).color, ink);
        // And nothing outside the written cell was touched.
        assert_eq!(canvas.get(1, 0).symbol, ' ');
        assert_eq!(canvas.get(0, 1).symbol, ' ');
    }

    /// A mask smaller than the canvas must not write past its own edge.
    #[test]
    fn writing_is_clipped_to_the_canvas() {
        let mut mask = QuadrantMask::new(10, 6);
        for y in 0..mask.sample_height() {
            for x in 0..mask.sample_width() {
                mask.set(x, y, true);
            }
        }
        let mut canvas = Canvas::new(4, 3);
        mask.write_to(&mut canvas, Color::White, Color::Black, Attribute::Reset);
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(canvas.get(x, y).symbol, FULL, "cell ({x},{y})");
            }
        }
    }
}
