//! A field at 2x vertical resolution, drawn with half-block characters.
//!
//! `▀` (upper half block) paints the cell's foreground colour over the top half
//! and its background colour over the bottom half. That buys 2x the vertical
//! resolution of a character grid *and* two independent colours per cell, which
//! is what a smooth gradient needs.
//!
//! The cost is horizontal resolution, which stays at one sample per cell, and
//! the aspect ratio: a half-block only lines up if a cell is about twice as tall
//! as it is wide. DejaVu Sans Mono, the default on much of Linux, is closer to
//! 1:1.2, so output looks vertically squashed there. Braille has the same
//! constraint and the same caveat.
//!
//! This is the one renderer that needs more than a glyph and a colour, which is
//! why [`crate::buffer::Cell`] carries a background.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crossterm::style::{Attribute, Color};

/// Rows of the field per cell.
pub const ROWS_PER_CELL: usize = 2;

/// Upper half block: foreground over background.
pub const UPPER: char = '▀';
/// Lower half block: background over foreground.
pub const LOWER: char = '▄';
/// Full block: foreground over foreground.
pub const FULL: char = '█';

/// A colour field of `width` by `2 * height`, drawn one half-block per cell.
///
/// Colours are stored as `f32` triples rather than bytes so a field can be
/// accumulated over several frames without banding at the low end, which is
/// where a slowly rising gradient spends most of its time.
#[derive(Clone)]
pub struct HalfBlockField {
    width: usize,
    height: usize,
    /// Row-major, `width * height * ROWS_PER_CELL` entries of `[r, g, b]`.
    pixels: Vec<[f32; 3]>,
}

impl HalfBlockField {
    /// A black field of `width` by `height` cells.
    pub fn new(width: usize, height: usize) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        Self {
            width,
            height,
            pixels: vec![[0.0; 3]; width * height * ROWS_PER_CELL],
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

    /// Width in field rows.
    pub fn row_width(&self) -> usize {
        self.width
    }

    /// Height in field rows: twice the cell height.
    pub fn row_height(&self) -> usize {
        self.height * ROWS_PER_CELL
    }

    /// Fills every row with one colour.
    pub fn fill(&mut self, rgb: [f32; 3]) {
        self.pixels.fill(rgb);
    }

    /// Resizes, discarding the contents.
    pub fn resize(&mut self, width: usize, height: usize) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        *self = Self::new(width, height);
    }

    /// Writes one field row, clamping out-of-range coordinates.
    #[inline]
    pub fn set_row(&mut self, x: usize, y: usize, rgb: [f32; 3]) {
        if x >= self.row_width() || y >= self.row_height() {
            return;
        }
        let index = y * self.width + x;
        self.pixels[index] = clamp_rgb(rgb);
    }

    /// Reads one field row, out of range reads as black.
    #[inline]
    pub fn row(&self, x: usize, y: usize) -> [f32; 3] {
        if x >= self.width || y >= self.row_height() {
            return [0.0; 3];
        }
        self.pixels[y * self.width + x]
    }

    /// Borrows the whole field, row-major, for an effect that computes into it.
    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.pixels
    }

    /// Mutably borrows the whole field, row-major.
    pub fn pixels_mut(&mut self) -> &mut [[f32; 3]] {
        &mut self.pixels
    }

    /// Fills the field from a scalar callback, one field row at a time.
    ///
    /// The callback receives field-row coordinates, so an effect writes a
    /// function of `(x, y)` without caring that rows are paired into cells.
    pub fn fill_with(&mut self, mut f: impl FnMut(usize, usize) -> [f32; 3]) {
        let (w, h) = (self.width, self.row_height());
        for y in 0..h {
            for x in 0..w {
                self.pixels[y * w + x] = clamp_rgb(f(x, y));
            }
        }
    }

    /// Writes the field into a canvas as half-block glyphs.
    ///
    /// A cell whose two rows are the same colour is written as a full block in
    /// that colour, which is one byte narrower than a half-block pair and does
    /// not depend on the terminal honouring a background colour.
    pub fn write_to(&self, canvas: &mut Canvas, attr: Attribute) {
        for cell_y in 0..self.height.min(canvas.height()) {
            for cell_x in 0..self.width.min(canvas.width()) {
                let top = self.to_color(self.row(cell_x, cell_y * 2));
                let bottom = self.to_color(self.row(cell_x, cell_y * 2 + 1));

                let cell = if top == bottom {
                    Cell::new(FULL, top, attr)
                } else {
                    Cell::with_bg(UPPER, top, bottom, attr)
                };
                canvas.set(cell_x, cell_y, cell);
            }
        }
    }

    /// The two rows of one cell as foreground and background.
    pub fn cell_colors(&self, cell_x: usize, cell_y: usize) -> (Color, Color) {
        if cell_x >= self.width || cell_y >= self.height {
            return (Color::Reset, Color::Reset);
        }
        (
            self.to_color(self.row(cell_x, cell_y * 2)),
            self.to_color(self.row(cell_x, cell_y * 2 + 1)),
        )
    }

    #[inline]
    fn to_color(&self, rgb: [f32; 3]) -> Color {
        Color::Rgb {
            r: to_u8(rgb[0]),
            g: to_u8(rgb[1]),
            b: to_u8(rgb[2]),
        }
    }
}

#[inline]
fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[inline]
fn clamp_rgb(rgb: [f32; 3]) -> [f32; 3] {
    [
        rgb[0].clamp(0.0, 1.0),
        rgb[1].clamp(0.0, 1.0),
        rgb[2].clamp(0.0, 1.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_of(field: &HalfBlockField) -> Canvas {
        let mut canvas = Canvas::new(field.width() as u16, field.height() as u16);
        canvas.clear();
        field.write_to(&mut canvas, Attribute::Reset);
        canvas
    }

    #[test]
    fn the_field_is_twice_as_tall_as_the_cells() {
        let field = HalfBlockField::new(10, 5);
        assert_eq!(field.row_height(), 10);
        assert_eq!(field.row_width(), 10);
    }

    #[test]
    fn matching_rows_become_a_full_block() {
        let mut field = HalfBlockField::new(1, 1);
        field.set_row(0, 0, [1.0, 0.0, 0.0]);
        field.set_row(0, 1, [1.0, 0.0, 0.0]);

        let canvas = canvas_of(&field);
        let cell = canvas.get(0, 0);
        assert_eq!(cell.symbol, FULL);
        assert_eq!(cell.color, Color::Rgb { r: 255, g: 0, b: 0 });
    }

    #[test]
    fn differing_rows_become_a_half_block_with_two_colors() {
        // Top red, bottom blue. The glyph paints the foreground over the top
        // half, so the top row has to be the foreground.
        let mut field = HalfBlockField::new(1, 1);
        field.set_row(0, 0, [1.0, 0.0, 0.0]);
        field.set_row(0, 1, [0.0, 0.0, 1.0]);

        let canvas = canvas_of(&field);
        let cell = canvas.get(0, 0);
        assert_eq!(cell.symbol, UPPER);
        assert_eq!(cell.color, Color::Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(cell.bg, Color::Rgb { r: 0, g: 0, b: 255 });
    }

    #[test]
    fn every_row_of_a_gradient_lands_somewhere_distinct() {
        // The reason for f32 storage: an 8-bit channel cannot show the bottom
        // of a slow ramp.
        let mut field = HalfBlockField::new(1, 8);
        for y in 0..16 {
            field.set_row(0, y, [y as f32 / 15.0, 0.0, 0.0]);
        }
        let mut reds: Vec<u8> = (0..16).map(|y| to_u8(y as f32 / 15.0)).collect();
        let before = reds.len();
        reds.dedup();
        assert_eq!(reds.len(), before, "a 16-step ramp collapsed");
    }

    #[test]
    fn out_of_range_rows_are_ignored_and_read_as_black() {
        let mut field = HalfBlockField::new(2, 2);
        field.set_row(5, 0, [1.0, 1.0, 1.0]);
        field.set_row(0, 9, [1.0, 1.0, 1.0]);
        assert_eq!(field.row(5, 0), [0.0; 3]);
        assert_eq!(field.row(0, 9), [0.0; 3]);
    }

    #[test]
    fn values_are_clamped_on_the_way_in() {
        let mut field = HalfBlockField::new(1, 1);
        field.set_row(0, 0, [2.0, -1.0, 0.5]);
        assert_eq!(field.row(0, 0), [1.0, 0.0, 0.5]);
    }

    #[test]
    fn cell_colors_reads_both_halves() {
        let mut field = HalfBlockField::new(2, 2);
        field.set_row(1, 2, [0.0, 1.0, 0.0]);
        field.set_row(1, 3, [1.0, 1.0, 0.0]);

        let (fg, bg) = field.cell_colors(1, 1);
        assert_eq!(fg, Color::Rgb { r: 0, g: 255, b: 0 });
        assert_eq!(
            bg,
            Color::Rgb {
                r: 255,
                g: 255,
                b: 0
            }
        );
    }

    #[test]
    fn writing_to_a_smaller_canvas_is_clipped_not_a_panic() {
        let field = HalfBlockField::new(10, 10);
        let mut canvas = Canvas::new(4, 4);
        canvas.clear();
        field.write_to(&mut canvas, Attribute::Reset);
        assert_eq!(canvas.get(3, 3).symbol, FULL);
    }

    #[test]
    fn fill_with_visits_field_rows_not_cells() {
        let mut field = HalfBlockField::new(2, 2);
        let mut seen = Vec::new();
        field.fill_with(|x, y| {
            seen.push((x, y));
            [0.0, 0.0, 0.0]
        });
        assert_eq!(seen.len(), 2 * 4);
        assert!(seen.contains(&(1, 3)), "field row 3 was never visited");
    }
}
