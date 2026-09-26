//! A double-buffered drawing surface, and the only place the diff-and-commit
//! sequence is written.
//!
//! Every effect needs the same three things: a surface to draw into, a record of
//! what the terminal is already showing, and a way to hand back just the cells
//! that changed. Fifteen effects each open-coded that as
//!
//! ```ignore
//! let mut curr = Buffer::new(w, h);
//! /* draw into curr */
//! let diff = self.buffer.diff(&curr);
//! self.buffer = curr;
//! ```
//!
//! which is correct, repetitive, and easy to get subtly wrong. The copy at the
//! end also threw away the allocation and rebuilt a full-screen `Buffer` every
//! frame — nearly a megabyte per frame at 400x200, sixty times a second.
//!
//! This holds both surfaces, reuses the allocations, and makes the sequence a
//! method. An effect holds a `Canvas` instead of a `Buffer` and calls
//! [`Canvas::clear`], [`Canvas::set`], then [`Canvas::commit`].

use crate::buffer::{Buffer, Cell};

/// A reusable drawing surface paired with the frame the terminal is showing.
#[derive(Clone)]
pub struct Canvas {
    /// What the terminal is currently displaying. Kept at its own size so that a
    /// resize can still diff against it, which is what makes the first frame
    /// after a resize a full repaint instead of a partial one.
    previous: Buffer,
    /// The surface being drawn into.
    current: Buffer,
    width: u16,
    height: u16,
}

impl Canvas {
    /// A canvas of the given size, with both surfaces blank.
    ///
    /// Dimensions are floored at 1 because a zero-sized `Buffer` has no cells
    /// and would make every subsequent `set` meaningless.
    pub fn new(width: u16, height: u16) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let cells = (width as usize, height as usize);
        Self {
            previous: Buffer::new(cells.0, cells.1),
            current: Buffer::new(cells.0, cells.1),
            width,
            height,
        }
    }

    /// The size as the frame loop thinks in it: columns, rows.
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn width(&self) -> usize {
        self.width as usize
    }

    pub fn height(&self) -> usize {
        self.height as usize
    }

    /// Resizes both surfaces and blanks them.
    ///
    /// Blanking `previous` is what forces the next [`Canvas::commit`] to report
    /// every cell, which is what a resize needs: the old frame described
    /// dimensions that no longer exist, so nothing about it can be trusted.
    pub fn resize(&mut self, width: u16, height: u16) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        let (w, h) = (width as usize, height as usize);
        self.previous = Buffer::new(w, h);
        self.current = Buffer::new(w, h);
        self.width = width;
        self.height = height;
    }

    /// Blanks the drawing surface, ready for a new frame.
    pub fn clear(&mut self) {
        self.current.fill_with(&Cell::default());
    }

    /// Fills the drawing surface with one repeated cell.
    ///
    /// For an effect whose whole frame is one value, such as a solid colour.
    pub fn clear_with(&mut self, cell: &Cell) {
        self.current.fill_with(cell);
    }

    /// Writes one cell, ignoring coordinates outside the surface.
    ///
    /// Out-of-range writes are dropped rather than panicking. `Buffer::set` only
    /// debug-asserts its bounds, so a release build would write past the end of
    /// the vector; an effect that computes a coordinate from a float position
    /// should not be able to corrupt memory by rounding the wrong way.
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, cell: Cell) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        self.current.set(x, y, cell);
    }

    /// Reads one cell. Out-of-range coordinates read as blank.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> Cell {
        if x >= self.width() || y >= self.height() {
            return Cell::default();
        }
        self.current.get(x, y)
    }

    /// Copies another buffer's cells over the drawing surface.
    ///
    /// For an effect whose frame is a template plus a few changes: the maze
    /// redraws its whole wall texture and then paints the carved path over it,
    /// and cloning the template per frame was the alternative.
    ///
    /// Cells beyond the template's own dimensions keep whatever the surface
    /// already held, so a template smaller than the surface is a partial
    /// overlay rather than a resize.
    pub fn blit(&mut self, source: &Buffer) {
        let width = source.width.min(self.width());
        let height = source.height.min(self.height());
        for y in 0..height {
            for x in 0..width {
                let cell = source.get(x, y);
                self.current.set(x, y, cell);
            }
        }
    }

    /// Borrows the drawing surface: the frame currently being drawn.
    ///
    /// Note that [`Canvas::commit`] swaps the two surfaces, so this is *not* the
    /// frame that was just committed. Use [`Canvas::on_screen`] for that.
    pub fn surface(&self) -> &Buffer {
        &self.current
    }

    /// Borrows the frame the terminal is showing.
    ///
    /// Before the first commit this is blank, since nothing has been sent yet.
    /// After one it is the frame that was just handed to the runtime.
    pub fn on_screen(&self) -> &Buffer {
        &self.previous
    }

    /// Mutably borrows the drawing surface, for an effect that fills it wholesale.
    pub fn surface_mut(&mut self) -> &mut Buffer {
        &mut self.current
    }

    /// Replaces the drawing surface wholesale.
    ///
    /// For an effect that composes the frame in a scratch buffer and then hands
    /// the result over -- the playlist applies its wipe to a copy so the
    /// undimmed frame underneath is not destroyed by the transition.
    pub fn replace_surface(&mut self, surface: Buffer) {
        self.current = surface;
    }

    /// Diffs the drawing surface against the frame on screen and commits it.
    ///
    /// Returns the cells that changed, in the `(x, y, cell)` form the runtime
    /// writes to the terminal. The two surfaces are swapped rather than copied,
    /// so the commit costs one comparison pass and no allocation.
    pub fn commit(&mut self) -> Vec<(usize, usize, Cell)> {
        let diff = self.previous.diff(&self.current);
        std::mem::swap(&mut self.previous, &mut self.current);
        diff
    }

    /// Declares the drawing surface to be what the terminal is already showing.
    ///
    /// For an effect whose constructor has already produced its first frame --
    /// a solid fill, a generated landscape -- that frame is on screen before the
    /// loop starts, so the first [`Canvas::commit`] must not report it. Without
    /// this the effect would emit its whole opening frame twice, once on a
    /// terminal that has just been cleared.
    ///
    /// The diff that commit produces is discarded, because nothing has been
    /// written to the terminal yet and there is nothing to report.
    pub fn establish_baseline(&mut self) {
        self.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::style;

    fn cell(symbol: char) -> Cell {
        Cell::new(symbol, style::Color::White, style::Attribute::Reset)
    }

    #[test]
    fn a_fresh_canvas_commits_everything_drawn() {
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();
        canvas.set(1, 2, cell('x'));

        let diff = canvas.commit();

        assert_eq!(diff, vec![(1, 2, cell('x'))]);
    }

    #[test]
    fn an_unchanged_frame_commits_nothing() {
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();
        canvas.set(1, 2, cell('x'));
        canvas.commit();

        canvas.clear();
        canvas.set(1, 2, cell('x'));

        assert!(
            canvas.commit().is_empty(),
            "redrawing the same frame reported cells as changed"
        );
    }

    #[test]
    fn a_cell_going_blank_is_reported() {
        // The case a naive "skip blanks" optimisation gets wrong: the terminal
        // needs to be told to erase, so a cell returning to blank is a change.
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();
        canvas.set(0, 0, cell('#'));
        canvas.commit();

        canvas.clear();
        let diff = canvas.commit();

        assert_eq!(diff, vec![(0, 0, Cell::default())]);
    }

    #[test]
    fn writes_outside_the_surface_are_dropped_not_written_past_the_end() {
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();

        // `Buffer::set` only debug-asserts, so without the guard in `Canvas::set`
        // this would be an out-of-bounds write in a release build.
        canvas.set(4, 0, cell('o'));
        canvas.set(0, 3, cell('o'));
        canvas.set(usize::MAX, usize::MAX, cell('o'));

        assert!(canvas.commit().is_empty());
    }

    #[test]
    fn reads_outside_the_surface_are_blank() {
        let canvas = Canvas::new(4, 3);
        assert_eq!(canvas.get(4, 0), Cell::default());
        assert_eq!(canvas.get(0, 3), Cell::default());
    }

    #[test]
    fn resizing_invalidates_the_previous_frame() {
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();
        canvas.set(0, 0, cell('a'));
        assert_eq!(canvas.commit(), vec![(0, 0, cell('a'))]);

        canvas.resize(6, 2);
        assert_eq!(canvas.size(), (6, 2));

        // Redrawing the identical cell after a resize must report it again. The
        // old frame described dimensions that no longer exist, so treating it as
        // a valid baseline would suppress cells the terminal still needs.
        canvas.clear();
        canvas.set(0, 0, cell('a'));
        assert_eq!(
            canvas.commit(),
            vec![(0, 0, cell('a'))],
            "a resize left the stale frame in place and suppressed a cell"
        );
    }

    #[test]
    fn a_resized_canvas_reports_only_what_was_drawn() {
        // The other half of the resize contract: blanking the baseline means the
        // commit stays proportional to the content, not to the screen. A full
        // repaint on every resize would be the alternative, and on a large
        // terminal it is the more expensive one.
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();
        canvas.set(0, 0, cell('a'));
        canvas.commit();

        canvas.resize(200, 60);
        canvas.clear();
        canvas.set(5, 1, cell('b'));
        let diff = canvas.commit();

        assert_eq!(diff, vec![(5, 1, cell('b'))]);
    }

    #[test]
    fn resizing_to_the_same_size_keeps_the_frame() {
        let mut canvas = Canvas::new(4, 3);
        canvas.clear();
        canvas.set(0, 0, cell('a'));
        canvas.commit();

        canvas.resize(4, 3);
        canvas.clear();
        canvas.set(0, 0, cell('a'));

        assert!(
            canvas.commit().is_empty(),
            "a no-op resize discarded the frame and forced a full repaint"
        );
    }

    #[test]
    fn zero_sized_dimensions_are_floored_at_one() {
        let canvas = Canvas::new(0, 0);
        assert_eq!(canvas.size(), (1, 1));
        assert_eq!(canvas.width(), 1);
        assert_eq!(canvas.height(), 1);
    }

    #[test]
    fn blitting_copies_the_source_over_the_surface() {
        let mut template = Buffer::new(3, 2);
        for y in 0..2 {
            for x in 0..3 {
                template.set(x, y, cell((b'a' + (y * 3 + x) as u8) as char));
            }
        }

        let mut canvas = Canvas::new(3, 2);
        canvas.clear();
        canvas.blit(&template);

        let drawn: String = (0..3).map(|x| canvas.get(x, 0).symbol).collect();
        assert_eq!(drawn, "abc");
    }

    #[test]
    fn a_smaller_blit_source_leaves_the_rest_of_the_surface_alone() {
        let mut canvas = Canvas::new(4, 2);
        canvas.clear();
        for x in 0..4 {
            canvas.set(x, 0, cell('.'));
        }

        // The source covers 0..2, and its second cell is deliberately distinct
        // from the surface's: a blit copies the source's cells verbatim, blanks
        // included, so only cells outside the source prove the bound.
        let mut small = Buffer::new(2, 1);
        small.set(0, 0, cell('#'));
        small.set(1, 0, cell('+'));

        canvas.blit(&small);

        assert_eq!(canvas.get(0, 0).symbol, '#');
        assert_eq!(canvas.get(1, 0).symbol, '+');
        assert_eq!(
            canvas.get(2, 0).symbol,
            '.',
            "a partial blit reached past the source's width"
        );
    }

    #[test]
    fn the_allocations_are_reused_across_frames() {
        // The reason this type exists: no full-screen allocation per frame.
        // `commit` swaps the two surfaces, so the address alternates between
        // them; what must not happen is a third address appearing.
        let mut canvas = Canvas::new(8, 4);
        let mut seen: Vec<usize> = Vec::new();

        for frame in 0..12 {
            canvas.clear();
            canvas.set(0, 0, cell('a'));
            let ptr = canvas.surface().buffer.as_ptr() as usize;
            if !seen.contains(&ptr) {
                seen.push(ptr);
            }
            canvas.commit();
            assert!(frame < 12);
        }

        assert!(
            seen.len() <= 2,
            "the drawing surface took {} distinct addresses over 12 frames; \
             expected the two swapped buffers and nothing else",
            seen.len()
        );
    }

    #[test]
    fn a_long_run_of_identical_frames_allocates_nothing_new() {
        // The case that actually matters at 60fps: a mostly-static screen, where
        // the old code still rebuilt a full buffer every frame to diff against.
        let mut canvas = Canvas::new(200, 50);
        let mut seen: Vec<usize> = Vec::new();

        for _ in 0..30 {
            canvas.clear();
            canvas.set(10, 10, cell('a'));
            let ptr = canvas.surface().buffer.as_ptr() as usize;
            if !seen.contains(&ptr) {
                seen.push(ptr);
            }
            canvas.commit();
        }

        assert!(
            seen.len() <= 2,
            "30 frames touched {} distinct surfaces",
            seen.len()
        );
    }
}
