//! Shared renderers for drawing at sub-cell resolution.
//!
//! A terminal cell is one character in one colour. That is a hard limit, and
//! everything here is a way to get more out of it:
//!
//! - [`braille`] packs eight dots into a cell, for 8x density in one colour.
//! - [`halfblock`] splits a cell into an upper and lower half drawn in the
//!   foreground and background colours, for 2x vertical resolution in two.
//! - [`dither`] turns a hard threshold into a gradient.
//! - [`palette`] holds colour ramps that several effects share.
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
pub mod halfblock;
pub mod palette;

pub use braille::BrailleGrid;
pub use dither::Dither;
pub use halfblock::HalfBlockField;
pub use palette::Palette;
