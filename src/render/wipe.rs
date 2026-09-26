//! The diagonal wipe used to move between effects.
//!
//! Extracted from the playlist, which had it inline, so that switching the
//! running effect from a keypress looks the same as the playlist advancing. Two
//! implementations of "the transition" is how they drift apart.
//!
//! The front runs diagonally, so a wipe reads as a wipe rather than as a
//! dissolve. It is sampled at cell centres, which makes `progress` 0.0 hide
//! nothing and 1.0 hide everything -- without that, the first and last frames
//! of a transition are one row or column off.

use crate::buffer::{Buffer, Cell};
use crossterm::style;

/// The cell a wipe leaves behind.
///
/// `Color::Reset` rather than the `Cell::default()` black: this is meant to
/// match whatever the terminal's own background is, and black-on-black is only
/// right by coincidence.
pub fn blank_cell() -> Cell {
    Cell::new(' ', style::Color::Reset, style::Attribute::Reset)
}

/// Hides the part of `buffer` the wipe front has already passed.
///
/// `progress` is clamped, so a computed value that overshoots hides everything
/// rather than running off the end of the diagonal.
pub fn apply(buffer: &mut Buffer, width: usize, height: usize, progress: f32) {
    let progress = progress.clamp(0.0, 1.0);
    if width == 0 || height == 0 {
        return;
    }
    let blank = blank_cell();
    for y in 0..height {
        for x in 0..width {
            // Normalised cell centre, so the front passes through the middle of
            // each cell rather than its corner.
            let front = ((x as f32 + 0.5) / width as f32
                + (y as f32 + 0.5) / height as f32)
                / 2.0;
            if front <= progress {
                buffer.set(x, y, blank);
            }
        }
    }
}

/// Blanks the cells of a *diff* that the wipe front has already passed.
///
/// Takes the changed cells rather than a whole buffer, because the frame loop
/// only ever holds the diff. Rebuilding a full-screen buffer to wipe it would
/// undo the point of diffing.
pub fn apply_to_cells(
    cells: &mut [(usize, usize, Cell)],
    width: usize,
    height: usize,
    progress: f32,
) {
    let progress = progress.clamp(0.0, 1.0);
    if width == 0 || height == 0 {
        return;
    }
    let blank = blank_cell();
    for (x, y, cell) in cells.iter_mut() {
        let front = ((*x as f32 + 0.5) / width as f32
            + (*y as f32 + 0.5) / height as f32)
            / 2.0;
        if front <= progress {
            *cell = blank;
        }
    }
}

/// How much of the screen `progress` has hidden, in `0.0..=1.0`.
///
/// The inverse of the front calculation, for sizing a transition: the time to
/// go from nothing hidden to everything hidden is the same either way, but this
/// is what a caller needs to pick a duration.
pub fn hidden_fraction(progress: f32) -> f32 {
    progress.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts non-blank cells, which is what a wipe changes.
    fn lit(buffer: &Buffer) -> usize {
        (0..buffer.height)
            .flat_map(|y| (0..buffer.width).map(move |x| (x, y)))
            .filter(|(x, y)| buffer.get(*x, *y).symbol != ' ')
            .count()
    }

    fn filled(width: usize, height: usize) -> Buffer {
        let mut buffer = Buffer::new(width, height);
        for y in 0..height {
            for x in 0..width {
                buffer.set(
                    x,
                    y,
                    Cell::new('#', style::Color::White, style::Attribute::Reset),
                );
            }
        }
        buffer
    }

    #[test]
    fn zero_progress_hides_nothing() {
        let mut buffer = filled(8, 4);
        apply(&mut buffer, 8, 4, 0.0);
        assert_eq!(lit(&buffer), 32, "the first frame of a wipe hid something");
    }

    #[test]
    fn full_progress_hides_everything() {
        let mut buffer = filled(8, 4);
        apply(&mut buffer, 8, 4, 1.0);
        assert_eq!(lit(&buffer), 0, "the last frame of a wipe left something");
    }

    #[test]
    fn the_wipe_is_monotonic() {
        // A wipe that un-hid cells as it advanced would strobe.
        let mut buffer = filled(16, 8);
        let mut previous = lit(&buffer);
        for step in 0..=20 {
            let mut frame = filled(16, 8);
            apply(&mut frame, 16, 8, step as f32 / 20.0);
            let now = lit(&frame);
            assert!(
                now <= previous,
                "step {step} un-hid cells ({previous} -> {now})"
            );
            previous = now;
        }
        let _ = &mut buffer;
    }

    #[test]
    fn the_front_runs_diagonally() {
        // Top-left goes before bottom-right. A wipe that ran the other way, or
        // straight across, would not read as a diagonal.
        let mut buffer = filled(8, 8);
        apply(&mut buffer, 8, 8, 0.5);

        let top_left = buffer.get(0, 0).symbol;
        let bottom_right = buffer.get(7, 7).symbol;
        assert_eq!(top_left, ' ', "the top-left corner outlived the wipe");
        assert_eq!(
            bottom_right, '#',
            "the bottom-right corner was hidden too early"
        );
    }

    #[test]
    fn an_overshoot_hides_everything_rather_than_running_off_the_end() {
        let mut buffer = filled(4, 4);
        apply(&mut buffer, 4, 4, 1.5);
        assert_eq!(lit(&buffer), 0);
    }

    #[test]
    fn a_zero_sized_buffer_is_a_no_op_not_a_division_by_zero() {
        let mut buffer = Buffer::new(1, 1);
        apply(&mut buffer, 0, 0, 0.5);
        apply(&mut buffer, 4, 0, 0.5);
        assert_eq!(hidden_fraction(0.5), 0.5);
    }

    #[test]
    fn a_diff_is_wiped_in_place() {
        let mut cells: Vec<(usize, usize, Cell)> = (0..4)
            .flat_map(|x| (0..4).map(move |y| (x, y)))
            .map(|(x, y)| {
                (
                    x,
                    y,
                    Cell::new('#', style::Color::White, style::Attribute::Reset),
                )
            })
            .collect();

        apply_to_cells(&mut cells, 4, 4, 1.0);

        assert!(
            cells.iter().all(|(_, _, cell)| cell.symbol == ' '),
            "a full-progress wipe left cells in the diff"
        );
    }

    #[test]
    fn a_diff_wipe_agrees_with_the_buffer_wipe() {
        // Two implementations of the same front would drift. This is the check.
        let (width, height) = (9, 5);
        let mut buffer = filled(width, height);
        apply(&mut buffer, width, height, 0.45);

        let mut cells: Vec<(usize, usize, Cell)> = (0..width)
            .flat_map(|x| (0..height).map(move |y| (x, y)))
            .map(|(x, y)| {
                (
                    x,
                    y,
                    Cell::new('#', style::Color::White, style::Attribute::Reset),
                )
            })
            .collect();
        apply_to_cells(&mut cells, width, height, 0.45);

        let mut from_cells = filled(width, height);
        for (x, y, cell) in &cells {
            from_cells.set(*x, *y, *cell);
        }

        for y in 0..height {
            for x in 0..width {
                assert_eq!(
                    buffer.get(x, y),
                    from_cells.get(x, y),
                    "the two wipes disagree at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn a_diff_wipe_at_zero_changes_nothing() {
        let mut cells: Vec<(usize, usize, Cell)> = vec![(
            0,
            0,
            Cell::new('#', style::Color::White, style::Attribute::Reset),
        )];
        let before = cells.clone();
        apply_to_cells(&mut cells, 4, 4, 0.0);
        assert_eq!(cells, before);
    }

    #[test]
    fn the_blank_cell_uses_the_terminals_own_background() {
        // Black would be right only by coincidence; the terminal may be light.
        assert_eq!(blank_cell().bg, style::Color::Reset);
        assert_eq!(blank_cell().symbol, ' ');
    }
}
