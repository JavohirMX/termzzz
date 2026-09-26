use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crossterm::style;
use serde::{Deserialize, Serialize};

/// The one cell this effect ever draws. A function rather than a constant
/// because `Cell::new` is not `const`, and naming it keeps the constructor and
/// the frame loop from disagreeing about it.
fn fill() -> Cell {
    Cell::new('#', style::Color::Green, style::Attribute::Reset)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BlankOptions {}

impl Default for BlankOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {}
    }
}

pub struct Blank {
    screen_size: (u16, u16),
    options: BlankOptions,
    canvas: Canvas,
}

impl TerminalEffect for Blank {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear_with(&fill());
        self.canvas.commit()
    }

    fn update(&mut self) {}

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(width, height);
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Blank {
    pub fn new(options: BlankOptions, screen_size: (u16, u16)) -> Self {
        let mut canvas = Canvas::new(screen_size.0, screen_size.1);
        canvas.clear_with(&fill());
        // The fill is what the terminal will show, so it is the baseline rather
        // than a change to report.
        canvas.establish_baseline();

        Self {
            screen_size: (screen_size.0.max(1), screen_size.1.max(1)),
            options,
            canvas,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_frame_is_reported_once_then_settles() {
        let mut blank = Blank::new(BlankOptions::default(), (4, 2));

        // The constructor primes the surface, so the first frame after it is
        // already the steady state and reports nothing.
        assert!(
            blank.get_diff().is_empty(),
            "the first frame was not already up to date"
        );

        // And it stays that way: this effect never changes.
        assert!(blank.get_diff().is_empty());
    }

    #[test]
    fn resizing_reports_the_new_frame_in_full() {
        let mut blank = Blank::new(BlankOptions::default(), (4, 2));
        blank.update_size(6, 3);

        let diff = blank.get_diff();

        assert_eq!(
            diff.len(),
            18,
            "a resize to 6x3 should report all 18 cells, got {}",
            diff.len()
        );
        assert!(
            diff.iter().all(|(x, y, cell)| {
                *x < 6
                    && *y < 3
                    && cell.symbol == '#'
                    && cell.color == style::Color::Green
            }),
            "the frame after a resize was not uniformly filled"
        );
    }
}
