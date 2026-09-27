use crate::buffer::{Buffer, Cell};
use crate::render::palette::Palette;
use crossterm::style::{self, Color};

/// The phosphor ramp the field is painted with by default, dimmest first.
///
/// A CRT phosphor is green because that is the colour a long-persistence screen
/// could actually be made to hold; here it is a look, and it is the look this
/// effect has always had.
///
/// The **top stop is pure white**, `rgb(255, 255, 255)`, and it is worth saying
/// why that is a fix rather than a preference. It was `rgb(230, 243, 227)` --
/// a 6.6% saturated green, which is not white by any measure and reads as a
/// green cast over the brightest parts of the field. The user reported it twice,
/// once as "the white blobs are not fully white; they have a green tint", and
/// the first time it was reported it was not actually changed either.
///
/// The value is also *fully* 255 rather than merely neutral. A near-white top
/// (`rgb(240, 240, 240)`) is neutral and still not white: on a terminal whose
/// own background is not black the brightest cell reads as "dimmed" rather than
/// as the top of the ramp, and this field's brightest cells are the ones the
/// pointer paints.
///
/// The rest of the ramp is a phosphor green, which is the point of it: a
/// monochrome white top against a green ramp is a phosphor screen, and a
/// monochrome *field* is one option away, through [`AsciiFieldOptions::colors`].
///
/// Pinned by `the_brightest_stop_is_white_and_neutral` in `effect.rs`.
pub const PHOSPHOR_RAMP: &[Color] = &[
    Color::Rgb { r: 8, g: 32, b: 24 },
    Color::Rgb {
        r: 16,
        g: 88,
        b: 54,
    },
    Color::Rgb {
        r: 32,
        g: 156,
        b: 86,
    },
    Color::Rgb {
        r: 83,
        g: 212,
        b: 144,
    },
    Color::Rgb {
        r: 165,
        g: 235,
        b: 188,
    },
    Color::Rgb {
        r: 255,
        g: 255,
        b: 255,
    },
];

/// The glyph ramp, dimmest first.
pub const DEFAULT_GLYPHS: &str = " .:-=+*#%@";

#[derive(Debug, Clone)]
pub struct GlyphPalette {
    glyphs: Vec<char>,
    /// A [`Palette`] rather than a bare `Vec`, so the "never empty" rule and the
    /// named-colour handling live in one place instead of being restated here.
    colors: Palette,
}

impl Default for GlyphPalette {
    fn default() -> Self {
        Self {
            glyphs: DEFAULT_GLYPHS.chars().collect(),
            colors: Palette::new(PHOSPHOR_RAMP.to_vec()),
        }
    }
}

impl GlyphPalette {
    /// Builds a palette from a glyph string and a list of colour stops.
    ///
    /// Either argument may be unusable, and an unusable one becomes the default
    /// rather than nothing:
    ///
    /// - a glyph string that filters down to empty falls back to
    ///   [`DEFAULT_GLYPHS`], because a palette with no glyphs would index
    ///   `len() - 1` on an empty vector;
    /// - an empty colour list falls back to [`PHOSPHOR_RAMP`], which is the case
    ///   a hand-edited config reaches most easily. The alternative -- a palette
    ///   that paints nothing -- is a screensaver that shows a blank screen and
    ///   no reason why.
    ///
    /// Both fallbacks are about *usable*, not about *valid*: a colour the
    /// renderer cannot resolve to channels (`reset`, `ansi(3)`) is kept, because
    /// the terminal can draw it and the terminal is the authority on what its own
    /// foreground is. Nothing is blended, so there is nothing here that needs a
    /// channel value.
    pub fn new(glyphs: &str, colors: Vec<Color>) -> Self {
        let glyphs: Vec<char> = glyphs
            .chars()
            .filter(|glyph| glyph.is_ascii() && !glyph.is_control())
            .collect();
        let glyphs = if glyphs.is_empty() {
            DEFAULT_GLYPHS.chars().collect()
        } else {
            glyphs
        };
        let colors = if colors.is_empty() {
            Palette::new(PHOSPHOR_RAMP.to_vec())
        } else {
            Palette::new(colors)
        };
        Self { glyphs, colors }
    }

    /// The colour stops, dimmest first. For a test, and for `--print-config`.
    pub fn colors(&self) -> &[Color] {
        self.colors.stops()
    }

    /// The glyphs, dimmest first.
    pub fn glyphs(&self) -> &[char] {
        &self.glyphs
    }

    /// The cell for a field value in `0.0..=1.0`.
    ///
    /// Both channels read the same value, which is the point: the glyph carries
    /// brightness and the colour carries the same brightness, so a monochrome
    /// configuration is a legitimate answer rather than a degraded one.
    ///
    /// `Attribute::Reset` and not `Attribute::Bold`, for the reason every other
    /// effect in this crate gives. Bold on a truecolor foreground is a
    /// *brightening hint* that many terminals act on by brightening the colour,
    /// so the top of a ramp does not land where the ramp says it does. This
    /// effect was the last one still asking for it, and asking for it exactly
    /// where it did the most damage -- the top sixth of the value range, which is
    /// the white the user was looking at. See `no_cell_is_asked_to_be_bright`
    /// in `effect.rs`.
    pub fn sample(&self, value: f32) -> Cell {
        let value = if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, 1.0)
        };

        let glyphs = self.glyphs.len();
        let colors = self.colors.len();
        let glyph_index =
            ((value * (glyphs - 1) as f32).round() as usize).min(glyphs - 1);
        let color_index =
            ((value * (colors - 1) as f32).round() as usize).min(colors - 1);

        Cell::new(
            self.glyphs[glyph_index],
            self.colors.sample_index(color_index),
            style::Attribute::Reset,
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct AsciiRenderer {
    palette: GlyphPalette,
}

impl AsciiRenderer {
    pub fn new(palette: GlyphPalette) -> Self {
        Self { palette }
    }

    /// The palette the renderer draws from. For a test, and so a caller can
    /// read back what a configuration actually resolved to.
    pub fn palette(&self) -> &GlyphPalette {
        &self.palette
    }

    pub fn render_field(
        &self,
        values: &[f32],
        width: usize,
        height: usize,
        buffer: &mut Buffer,
    ) {
        let width = width.min(buffer.width);
        let height = height.min(buffer.height);

        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let value = values.get(index).copied().unwrap_or_default();
                buffer.set(x, y, self.palette.sample(value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top of the ramp has to be white, and *neutral* white.
    ///
    /// Two separate claims, because the bug was only half of one of them. The
    /// old top stop was `rgb(230, 243, 227)`: the green channel is 13 above the
    /// red and the blue is 16 below the red, so it is a 6.6% saturated green and
    /// a viewer reads the brightest part of the field as green-cast no matter how
    /// bright it is. And it is also not *white*: 243 is 95% of full, so even with
    /// the tint removed it would be a very light grey, and against a background
    /// that is not black it reads as dimmed rather than as the top of a ramp.
    ///
    /// Asserted as neutrality plus a level rather than as a literal, so the test
    /// is about the property and not about one chosen shade of white.
    #[test]
    fn the_brightest_stop_is_white_and_neutral() {
        let top = match PHOSPHOR_RAMP.last() {
            Some(Color::Rgb { r, g, b }) => (*r, *g, *b),
            other => {
                panic!("the top of the phosphor ramp is not a truecolor: {other:?}")
            }
        };
        let (r, g, b) = top;

        let spread = r.max(g).max(b) - r.min(g).min(b);
        assert_eq!(
            spread, 0,
            "the brightest stop is rgb({r}, {g}, {b}): the channels differ by \
             {spread}, which is a {spread:.1}% tint on the whitest colour in the \
             effect and is exactly what the user reported",
        );
        assert!(
            r >= 250,
            "the brightest stop is rgb({r}, {g}, {b}) at {r}/255, which is not \
             white; the field's brightest cells have to reach the top"
        );
    }

    /// A palette with a green top is a bug even if the *default* is fixed, so
    /// the sampled output is checked as well as the constant.
    ///
    /// A constant can be correct and the sampler can still be wrong about which
    /// stop is the top -- an inverted index would draw the ramp upside down and
    /// every constant would look right. Pinned on the drawn cell.
    #[test]
    fn the_brightest_value_draws_the_brightest_stop() {
        let palette = GlyphPalette::default();
        let top = palette
            .colors()
            .last()
            .copied()
            .expect("the ramp has a top");

        let dim = palette.sample(0.0);
        assert_eq!(
            dim.color,
            palette.colors()[0],
            "the bottom of the value \
             range did not draw the bottom of the ramp"
        );

        let bright = palette.sample(1.0);
        assert_eq!(bright.color, top, "value 1.0 did not draw the top stop");
        assert_eq!(
            bright.color,
            Color::Rgb {
                r: 255,
                g: 255,
                b: 255
            },
            "the brightest cell is not white"
        );
    }

    /// An empty colour list is a blank screen, which is not a fallback.
    #[test]
    fn an_empty_colour_list_falls_back_to_the_phosphor_ramp() {
        let palette = GlyphPalette::new(DEFAULT_GLYPHS, Vec::new());
        assert_eq!(palette.colors(), PHOSPHOR_RAMP);
        assert!(
            !palette.colors().is_empty(),
            "an empty colour list produced a palette that paints nothing"
        );
    }

    /// A single colour is a legitimate configuration, and paints one colour.
    ///
    /// "They should be fully white or a single color" -- the second half of that
    /// is this, and it has to be reachable from a config file rather than only
    /// by editing source.
    #[test]
    fn a_single_colour_paints_the_whole_range() {
        let palette = GlyphPalette::new(
            DEFAULT_GLYPHS,
            vec![Color::Rgb {
                r: 10,
                g: 20,
                b: 30,
            }],
        );
        for value in [0.0f32, 0.25, 0.5, 0.74, 1.0] {
            assert_eq!(
                palette.sample(value).color,
                Color::Rgb {
                    r: 10,
                    g: 20,
                    b: 30
                },
                "value {value} did not draw the single configured colour"
            );
        }
    }

    /// The glyphs still vary when the colours do not, or a monochrome field is
    /// a monochrome *blank*.
    ///
    /// The two channels read the same field value independently. Pinning that
    /// they still differ is what stops a "monochrome" fix from being implemented
    /// by flattening both.
    #[test]
    fn a_monochrome_palette_still_shades_by_glyph() {
        let palette = GlyphPalette::new(
            DEFAULT_GLYPHS,
            vec![Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            }],
        );
        let dim = palette.sample(0.0).symbol;
        let bright = palette.sample(1.0).symbol;
        assert_ne!(dim, bright, "the glyph does not vary with the value");
    }

    /// Bold is a brightening hint, so the ramp's top is not where it says.
    #[test]
    fn no_value_is_asked_to_be_bold() {
        let palette = GlyphPalette::default();
        for step in 0..=100 {
            let value = step as f32 / 100.0;
            let cell = palette.sample(value);
            assert_eq!(
                cell.attr,
                style::Attribute::Reset,
                "value {value:.2} drew {cell:?}, asking the terminal to brighten \
                 a colour the ramp has already placed"
            );
        }
    }

    /// An out-of-range or NaN value must not index outside the palette.
    #[test]
    fn an_unusable_value_saturates_rather_than_panicking() {
        let palette = GlyphPalette::default();
        let dim = palette.sample(0.0);
        let bright = palette.sample(1.0);
        for value in [-1.0f32, 2.0, 1.0e9, f32::INFINITY, f32::NAN] {
            let cell = palette.sample(value);
            assert!(
                cell.color == dim.color || cell.color == bright.color,
                "value {value} produced {:?}, which is not an endpoint of the ramp",
                cell.color
            );
        }
    }
}
