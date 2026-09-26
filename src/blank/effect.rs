use crate::buffer::{Buffer, Cell};
use crate::common::TerminalEffect;
use crossterm::style;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[allow(dead_code)]
pub struct Blank {
    screen_size: (u16, u16),
    options: BlankOptions,
    buffer: Buffer,
}

impl TerminalEffect for Blank {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let mut curr_buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);

        curr_buffer.fill_with(&Cell {
            symbol: '#',
            color: style::Color::Green,
            attr: style::Attribute::Reset,
        });

        let diff = self.buffer.diff(&curr_buffer);
        self.buffer = curr_buffer;
        diff
    }

    fn update(&mut self) {}

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height)
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Blank {
    pub fn new(options: BlankOptions, screen_size: (u16, u16)) -> Self {
        let mut buffer =
            Buffer::new(screen_size.0 as usize, screen_size.1 as usize);

        buffer.fill_with(&Cell {
            symbol: '#',
            color: style::Color::Green,
            attr: style::Attribute::Reset,
        });

        Self {
            screen_size,
            options,
            buffer,
        }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn blank_test() {}
}
