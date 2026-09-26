//! A dot grid at 2x4 sub-cell resolution, encoded into braille characters.
//!
//! One terminal cell holds eight braille dots, so a full-screen braille grid
//! carries 8x the density of a character grid: 2 across, 4 down. That is the
//! cheapest resolution win available, and it needs no change to
//! [`crate::buffer::Cell`] -- a braille cell is one glyph in one colour, which
//! is exactly what a cell already is.
//!
//! The trade is colour. All eight dots in a cell share the cell's foreground
//! colour, so braille carries *density* and not *hue*. An effect that wants
//! smooth colour wants [`super::halfblock`] instead, which gives up half the
//! vertical resolution for two colours per cell.
//!
//! # Coordinate systems
//!
//! Two, and confusing them is the usual bug:
//!
//! - *dot* coordinates run `0..2*width` by `0..4*height`, and are what
//!   [`BrailleGrid::set_dot`] takes.
//! - *cell* coordinates run `0..width` by `0..height`, and are what
//!   [`Canvas::set`] takes.
//!
//! [`BrailleGrid::write_to`] is the only place the two meet.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crossterm::style::{self, Attribute, Color};

/// Dots across per cell.
pub const DOTS_X: usize = 2;
/// Dots down per cell.
pub const DOTS_Y: usize = 4;
/// Dots per cell, and so the number of bits in one cell's pattern.
pub const DOTS_PER_CELL: usize = DOTS_X * DOTS_Y;

/// First codepoint of the Unicode braille block. Every pattern is this plus the
/// cell's bit mask.
const BRAILLE_BASE: u32 = 0x2800;

/// A grid of braille patterns, addressable by dot.
#[derive(Clone)]
pub struct BrailleGrid {
    width: usize,
    height: usize,
    /// One byte of dot bits per cell, row-major.
    dots: Vec<u8>,
}

impl BrailleGrid {
    /// An all-blank grid of `width` by `height` cells.
    ///
    /// Both dimensions are floored at 1, because a cell with no dots has no
    /// coordinates to address.
    pub fn new(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            dots: vec![0; width * height],
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

    /// Width in dots.
    pub fn dot_width(&self) -> usize {
        self.width * DOTS_X
    }

    /// Height in dots.
    pub fn dot_height(&self) -> usize {
        self.height * DOTS_Y
    }

    /// Blanks every dot.
    pub fn clear(&mut self) {
        self.dots.fill(0);
    }

    /// Resizes, discarding the contents.
    pub fn resize(&mut self, width: usize, height: usize) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        *self = Self::new(width, height);
    }

    /// Raises or lowers a single dot.
    ///
    /// Coordinates outside the grid are ignored. A line rasteriser walking
    /// diagonally will step outside briefly at the edges, and dropping those
    /// dots is the correct result rather than an error.
    #[inline]
    pub fn set_dot(&mut self, dot_x: usize, dot_y: usize, raised: bool) {
        let Some(index) = self.cell_index(dot_x, dot_y) else {
            return;
        };
        let bit = 1u8 << Self::dot_index(dot_x, dot_y);
        if raised {
            self.dots[index] |= bit;
        } else {
            self.dots[index] &= !bit;
        }
    }

    #[inline]
    pub fn raise_dot(&mut self, dot_x: usize, dot_y: usize) {
        self.set_dot(dot_x, dot_y, true);
    }

    #[inline]
    pub fn lower_dot(&mut self, dot_x: usize, dot_y: usize) {
        self.set_dot(dot_x, dot_y, false);
    }

    /// Whether a dot is raised. Out-of-grid reads as lowered.
    #[inline]
    pub fn dot(&self, dot_x: usize, dot_y: usize) -> bool {
        let Some(index) = self.cell_index(dot_x, dot_y) else {
            return false;
        };
        self.dots[index] & (1u8 << Self::dot_index(dot_x, dot_y)) != 0
    }

    /// The character for one cell, or a space when the cell is blank.
    pub fn cell_char(&self, cell_x: usize, cell_y: usize) -> char {
        if cell_x >= self.width || cell_y >= self.height {
            return ' ';
        }
        let bits = self.dots[cell_y * self.width + cell_x];
        if bits == 0 {
            // U+2800 is the blank pattern, but a space is one byte narrower on
            // the wire and every terminal renders it identically.
            return ' ';
        }
        char::from_u32(BRAILLE_BASE + bits as u32).unwrap_or('?')
    }

    /// The raw bit pattern for one cell, for callers assembling their own glyph.
    pub fn cell_bits(&self, cell_x: usize, cell_y: usize) -> u8 {
        if cell_x >= self.width || cell_y >= self.height {
            return 0;
        }
        self.dots[cell_y * self.width + cell_x]
    }

    /// Calls `f(dot_x, dot_y, raised)` for every dot, row-major.
    pub fn for_each_dot(&self, mut f: impl FnMut(usize, usize, bool)) {
        for cell_y in 0..self.height {
            for cell_x in 0..self.width {
                let bits = self.dots[cell_y * self.width + cell_x];
                for sub in 0..DOTS_PER_CELL {
                    let (dx, dy) = Self::dot_position(sub);
                    f(
                        cell_x * DOTS_X + dx,
                        cell_y * DOTS_Y + dy,
                        bits & (1u8 << sub) != 0,
                    );
                }
            }
        }
    }

    /// Writes the grid into a canvas, one cell per braille glyph.
    ///
    /// Every cell gets the same colour, because a braille cell cannot carry
    /// two: the dots are a single glyph.
    pub fn write_to(&self, canvas: &mut Canvas, color: Color, attr: Attribute) {
        for cell_y in 0..self.height.min(canvas.height()) {
            for cell_x in 0..self.width.min(canvas.width()) {
                let symbol = self.cell_char(cell_x, cell_y);
                canvas.set(cell_x, cell_y, Cell::new(symbol, color, attr));
            }
        }
    }

    /// Writes the grid, leaving cells that are blank untouched.
    ///
    /// For an effect that composites braille over something else already on the
    /// canvas. A blank braille cell is a space, and painting it would erase
    /// whatever is underneath.
    pub fn overlay_onto(&self, canvas: &mut Canvas, color: Color, attr: Attribute) {
        for cell_y in 0..self.height.min(canvas.height()) {
            for cell_x in 0..self.width.min(canvas.width()) {
                let symbol = self.cell_char(cell_x, cell_y);
                if symbol == ' ' {
                    continue;
                }
                canvas.set(cell_x, cell_y, Cell::new(symbol, color, attr));
            }
        }
    }

    /// Lifts dots from a scalar field, comparing against a threshold that
    /// varies per dot so a gradient survives as intermediate density.
    ///
    /// `values` is row-major over the *dot* grid and is read with clamping, so
    /// a field sized for cells rather than dots still produces a picture rather
    /// than a panic.
    pub fn draw_field(
        &mut self,
        values: &[f32],
        threshold: f32,
        spread: f32,
        dither: super::dither::Dither,
    ) {
        let (dw, dh) = (self.dot_width(), self.dot_height());
        for dot_y in 0..dh {
            for dot_x in 0..dw {
                let index = dot_y * dw + dot_x;
                let value = values.get(index).copied().unwrap_or(0.0);
                // The dither offset turns one hard threshold into a moving
                // boundary, so values within `spread` of the threshold land on
                // a stable fraction of the dots instead of all-or-nothing.
                let jitter = (dither.at(dot_x, dot_y) - 0.5) * spread;
                self.set_dot(dot_x, dot_y, value > threshold + jitter);
            }
        }
    }

    #[inline]
    fn cell_index(&self, dot_x: usize, dot_y: usize) -> Option<usize> {
        if dot_x >= self.dot_width() || dot_y >= self.dot_height() {
            return None;
        }
        let cell_x = dot_x / DOTS_X;
        let cell_y = dot_y / DOTS_Y;
        Some(cell_y * self.width + cell_x)
    }

    /// Which bit a dot occupies within its cell.
    ///
    /// The dot numbering follows the Unicode braille block, which runs
    /// left-to-right across each row of four: bits 0 and 1 are the top row, 2
    /// and 3 the next, and so on. Getting this wrong produces a pattern that
    /// is the right density but the wrong shape.
    #[inline]
    fn dot_index(dot_x: usize, dot_y: usize) -> usize {
        (dot_y % DOTS_Y) * DOTS_X + (dot_x % DOTS_X)
    }

    /// The inverse of [`BrailleGrid::dot_index`].
    #[inline]
    fn dot_position(index: usize) -> (usize, usize) {
        (index % DOTS_X, index / DOTS_X)
    }
}

/// The colour a braille grid is written in when nothing else is specified.
pub fn default_color() -> Color {
    style::Color::White
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_grid_is_spaces() {
        let grid = BrailleGrid::new(4, 3);
        for y in 0..3 {
            for x in 0..4 {
                assert_eq!(grid.cell_char(x, y), ' ');
            }
        }
    }

    #[test]
    fn dot_dimensions_are_eight_times_the_cells() {
        let grid = BrailleGrid::new(10, 5);
        assert_eq!(grid.dot_width(), 20);
        assert_eq!(grid.dot_height(), 20);
        assert_eq!(grid.dot_width() * grid.dot_height(), 10 * 5 * 8);
    }

    #[test]
    fn the_first_dot_is_the_top_left_of_the_pattern() {
        // U+2801 is dot 1 alone: the top-left dot of the cell.
        let mut grid = BrailleGrid::new(1, 1);
        grid.raise_dot(0, 0);
        assert_eq!(grid.cell_char(0, 0), '\u{2801}');

        // U+2880 is dot 8 alone: the bottom-right, which is bit 7 rather than
        // bit 3. The bit index is `row * 2 + column`, so the last dot of a
        // four-row cell lands high in the byte, not next to U+2801. (U+2888 is
        // dots 4 and 8 together, which is an easy thing to reach for by mistake.)
        let mut grid = BrailleGrid::new(1, 1);
        grid.raise_dot(1, 3);
        assert_eq!(grid.cell_char(0, 0), '\u{2880}');
    }

    #[test]
    fn every_dot_maps_to_its_own_unicode_pattern() {
        // The whole point of the bit layout: each of the eight dots alone is a
        // distinct codepoint, U+2801 through U+2808.
        for sub in 0..DOTS_PER_CELL {
            let (dx, dy) = BrailleGrid::dot_position(sub);
            let mut grid = BrailleGrid::new(1, 1);
            grid.raise_dot(dx, dy);
            let expected = char::from_u32(0x2800 + (1u32 << sub)).unwrap();
            assert_eq!(
                grid.cell_char(0, 0),
                expected,
                "dot {sub} at ({dx},{dy}) encoded wrongly"
            );
        }
    }

    #[test]
    fn all_eight_dots_make_the_full_block() {
        // U+28FF is every dot raised.
        let mut grid = BrailleGrid::new(1, 1);
        for dy in 0..DOTS_Y {
            for dx in 0..DOTS_X {
                grid.raise_dot(dx, dy);
            }
        }
        assert_eq!(grid.cell_char(0, 0), '\u{28ff}');
    }

    #[test]
    fn dots_in_different_cells_do_not_merge() {
        let mut grid = BrailleGrid::new(2, 1);
        grid.raise_dot(0, 0);
        grid.raise_dot(2, 0); // cell 1, its left dot

        assert_eq!(grid.cell_char(0, 0), '\u{2801}');
        assert_eq!(grid.cell_char(1, 0), '\u{2801}');
    }

    #[test]
    fn lowering_a_dot_clears_only_that_bit() {
        let mut grid = BrailleGrid::new(1, 1);
        grid.raise_dot(0, 0);
        grid.raise_dot(1, 0);
        grid.lower_dot(0, 0);
        assert_eq!(grid.cell_char(0, 0), '\u{2802}');
    }

    #[test]
    fn dots_outside_the_grid_are_ignored_rather_than_panicking() {
        let mut grid = BrailleGrid::new(2, 2); // 4x8 dots
        grid.raise_dot(4, 0);
        grid.raise_dot(0, 8);
        grid.raise_dot(usize::MAX, usize::MAX);
        assert!(!grid.dot(4, 0));
        assert!(!grid.dot(usize::MAX, 0));
    }

    #[test]
    fn for_each_dot_visits_every_dot_once() {
        let grid = BrailleGrid::new(3, 2);
        let mut seen = Vec::new();
        grid.for_each_dot(|x, y, _| seen.push((x, y)));
        assert_eq!(seen.len(), 3 * 2 * 8);
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "a dot was visited twice");
    }

    #[test]
    fn writing_produces_one_cell_per_braille_glyph() {
        let mut grid = BrailleGrid::new(3, 2);
        grid.raise_dot(5, 7);

        let mut canvas = Canvas::new(3, 2);
        canvas.clear();
        grid.write_to(&mut canvas, Color::Red, Attribute::Bold);

        assert_eq!(canvas.get(2, 1).symbol, '\u{2880}');
        assert_eq!(canvas.get(2, 1).color, Color::Red);
        assert_eq!(canvas.get(0, 0).symbol, ' ');
    }

    #[test]
    fn overlaying_leaves_blank_cells_alone() {
        let mut grid = BrailleGrid::new(2, 1);
        grid.raise_dot(0, 0);

        let mut canvas = Canvas::new(2, 1);
        canvas.clear();
        canvas.set(1, 0, Cell::new('X', Color::Blue, Attribute::Reset));

        grid.overlay_onto(&mut canvas, Color::Red, Attribute::Reset);

        assert_eq!(canvas.get(0, 0).symbol, '\u{2801}');
        assert_eq!(
            canvas.get(1, 0).symbol,
            'X',
            "a blank braille cell painted over what was underneath"
        );
    }

    #[test]
    fn a_field_above_the_threshold_lifts_the_dot() {
        let mut grid = BrailleGrid::new(1, 1);
        let values = vec![1.0; 8];
        grid.draw_field(&values, 0.5, 0.0, super::super::dither::Dither::None);
        assert_eq!(grid.cell_char(0, 0), '\u{28ff}');
    }

    #[test]
    fn a_field_below_the_threshold_lifts_nothing() {
        let mut grid = BrailleGrid::new(1, 1);
        let values = vec![0.0; 8];
        grid.draw_field(&values, 0.5, 0.0, super::super::dither::Dither::None);
        assert_eq!(grid.cell_char(0, 0), ' ');
    }

    #[test]
    fn a_short_field_is_read_with_clamping_not_a_panic() {
        // A field sized for cells rather than dots is a plausible mistake, and
        // it should still draw rather than index past the end.
        let mut grid = BrailleGrid::new(4, 4); // 8x16 dots = 128 entries
        grid.draw_field(&[1.0; 16], 0.5, 0.0, super::super::dither::Dither::None);
        // The first 16 dots came from the field, the rest defaulted to 0.
        assert!(grid.dot(0, 0));
        assert!(!grid.dot(7, 15));
    }

    #[test]
    fn resizing_to_the_same_size_keeps_the_dots() {
        let mut grid = BrailleGrid::new(2, 2);
        grid.raise_dot(1, 1);
        grid.resize(2, 2);
        assert!(grid.dot(1, 1));
    }
}
