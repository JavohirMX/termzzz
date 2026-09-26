//! Shared renderers for drawing at sub-cell resolution.
//!
//! A terminal cell is one character in one colour. That is a hard limit, and
//! everything here is a way to get more out of it:
//!
//! - [`braille`] packs eight dots into a cell, for 8x density in one colour.
//! - [`halfblock`] splits a cell into an upper and lower half drawn in the
//!   foreground and background colours, for 2x vertical resolution in two.
//! - [`dither`] turns a hard threshold into a gradient.
//! - [`quadrant`] places a two-tone shape at 2x2 per cell, with crisp edges on
//!   both axes -- which half-block cannot do, because a cell has only one
//!   foreground and one background to spend.
//! - [`palette`] holds colour ramps that several effects share.
//! - [`glyph_ramp`] holds character ramps, for effects that turn a value into
//!   how much ink to put on the page.
//!
//! The two renderers are complements, not alternatives. Braille is 4x the
//! vertical resolution of half-block but cannot vary hue within a cell;
//! half-block carries a real gradient but only two rows per cell. An effect
//! picks the one that matches what it is drawing.
//!
//! Nothing here allocates per cell. The grids own their storage and are resized
//! in place, because they are rebuilt every frame.

pub mod braille;
pub mod dither;
pub mod glyph_ramp;
pub mod halfblock;
pub mod palette;
pub mod quadrant;
pub mod wipe;

pub use braille::BrailleGrid;
pub use dither::Dither;
pub use glyph_ramp::{GlyphRamp, presets as glyph_presets};
pub use halfblock::HalfBlockField;
pub use palette::Palette;
pub use quadrant::QuadrantMask;
pub use wipe::{apply as apply_wipe, blank_cell};
