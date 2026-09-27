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

/// Named single-hue ramps, for `palette = "orange"` in an `[ink]` config.
///
/// The user asked for this by name: "can we have different settings, like an
/// orange palette". There was already a `colors` option, and it was the wrong
/// shape for the request -- it means typing `rgb_(255, 126, 42)` to get what
/// `palette = "orange"` says. Both are kept; see
/// [`AsciiFieldOptions::palette`](super::AsciiFieldOptions) for which one wins.
///
/// # Why these are here and not in `render::palette`
///
/// [`crate::render::palette::presets`] holds *multi-hue* ramps that
/// [`crate::Palette::sample_wrapped`] cycles: `depth`, `ember`, `ocean`, `magma`
/// and `contrast`. Two other effects resolve their `palette` option against that
/// table, and adding single-hue ramps to it would make "an orange palette" show
/// up in mandelbrot's and donut's option lists too, where it would be a
/// different thing wearing the same name. This effect cycles nothing -- the
/// glyph ramp is indexed by value and stops at the top -- so the ramp it wants is
/// a brightness ramp, not a wheel. Keeping the two apart is also why these
/// ramps are listed darkest-first: that is the order
/// [`GlyphPalette::sample`] reads them in.
///
/// # What every one of these has in common
///
/// **Each one ends on `rgb(255, 255, 255)`, and that is a fix rather than a
/// preference.** [`PHOSPHOR_RAMP`] above documents the incident: its top stop
/// was `rgb(230, 243, 227)`, a 6.6% saturated green that the user reported
/// twice, once as "the white blobs are not fully white; they have a green tint",
/// and the first time it was reported it was not actually changed either. A
/// tinted top stop is the natural thing to write for a coloured ramp and it is
/// wrong every time, so every entry here is checked against it by
/// `every_named_palette_ends_on_exactly_white` rather than trusted.
///
/// The second stop from the top is the one that *is* tinted, and that is
/// deliberate: it is where a ramp has to hand over from "the colour" to "the
/// highlight", and a ramp that goes from a mid-tone straight to white has no
/// highlight to hand over from.
pub const FIELD_PALETTES: &[(&str, &[Color])] = &[
    ("green", PHOSPHOR_RAMP),
    (
        "orange",
        &[
            Color::Rgb { r: 26, g: 10, b: 4 },
            Color::Rgb { r: 92, g: 32, b: 8 },
            Color::Rgb {
                r: 196,
                g: 84,
                b: 12,
            },
            Color::Rgb {
                r: 255,
                g: 140,
                b: 32,
            },
            Color::Rgb {
                r: 255,
                g: 210,
                b: 140,
            },
            Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
        ],
    ),
    (
        "blue",
        &[
            Color::Rgb { r: 4, g: 10, b: 28 },
            Color::Rgb {
                r: 10,
                g: 32,
                b: 96,
            },
            Color::Rgb {
                r: 20,
                g: 86,
                b: 198,
            },
            Color::Rgb {
                r: 60,
                g: 150,
                b: 255,
            },
            Color::Rgb {
                r: 150,
                g: 210,
                b: 255,
            },
            Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
        ],
    ),
    (
        "magenta",
        &[
            Color::Rgb { r: 28, g: 4, b: 26 },
            Color::Rgb {
                r: 96,
                g: 12,
                b: 88,
            },
            Color::Rgb {
                r: 198,
                g: 30,
                b: 180,
            },
            Color::Rgb {
                r: 255,
                g: 96,
                b: 226,
            },
            Color::Rgb {
                r: 255,
                g: 186,
                b: 246,
            },
            Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
        ],
    ),
    (
        "ice",
        // Deliberately low-chroma, and the only ramp here whose first two stops
        // are close together. "Ice" read as a cold blue is a hue ramp, which
        // `blue` already is; what makes this one different is that the dark end
        // is nearly neutral, so a cell at the bottom of the field is dark rather
        // than blue, and a large dim region reads as shadow instead of as
        // colour. The top two stops are correspondingly close to white, and the
        // handover to pure white is a short one -- which is the same reason the
        // top stop is exactly white rather than nearly so.
        &[
            Color::Rgb { r: 8, g: 12, b: 16 },
            Color::Rgb {
                r: 40,
                g: 58,
                b: 72,
            },
            Color::Rgb {
                r: 110,
                g: 148,
                b: 170,
            },
            Color::Rgb {
                r: 180,
                g: 214,
                b: 230,
            },
            Color::Rgb {
                r: 232,
                g: 244,
                b: 250,
            },
            Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
        ],
    ),
    (
        "amber",
        // Close to `orange` on purpose, and the pair is worth having: `amber`
        // is the phosphor colour, `orange` is the dye. Amber's dark end is
        // brown rather than near-black and its top is yellower, so the two
        // differ at both ends of the range and not only in the middle. A user
        // who tried both and could not tell them apart would be right to.
        &[
            Color::Rgb { r: 24, g: 14, b: 2 },
            Color::Rgb { r: 90, g: 52, b: 4 },
            Color::Rgb {
                r: 196,
                g: 132,
                b: 10,
            },
            Color::Rgb {
                r: 255,
                g: 192,
                b: 44,
            },
            Color::Rgb {
                r: 255,
                g: 232,
                b: 168,
            },
            Color::Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
        ],
    ),
];

/// Looks a named ramp up, case-insensitively, falling back to the default.
///
/// Returns `None` for a name that is not in [`FIELD_PALETTES`], and the caller
/// decides what that means -- which for this effect is "use the default", not
/// "draw nothing". Kept as a lookup returning `Option` rather than one that
/// falls back itself so a test can assert both halves: that a good name resolves,
/// and that a bad one is distinguishable from a good one.
pub fn palette_by_name(name: &str) -> Option<&'static [Color]> {
    FIELD_PALETTES
        .iter()
        .find(|(preset, _)| preset.eq_ignore_ascii_case(name))
        .map(|(_, stops)| *stops)
}

/// The ramp `palette = ""` and an unknown name both resolve to.
///
/// Spelled as a function rather than a constant so the fallback path in
/// [`palette_by_name`]'s caller has one place to point at, and so a test
/// asserting "the default is the phosphor ramp" is asserting against the same
/// thing the field uses rather than against a second copy of the list.
pub const DEFAULT_PALETTE_NAME: &str = "green";

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
    use crate::common::TerminalEffect;
    use crate::ink::{AsciiField, AsciiFieldOptions};
    use std::collections::HashSet;

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

    // --- named field palettes ---------------------------------------------

    /// Every named ramp has to reach pure white, and that is a fix rather than a
    /// preference.
    ///
    /// The top stop of [`PHOSPHOR_RAMP`] was `rgb(230, 243, 227)` -- a 6.6%
    /// saturated green -- and the user reported it twice before it was actually
    /// changed. A tinted top is the natural thing to write for a coloured ramp:
    /// a green ramp "should" end on a very pale green. It is wrong every time,
    /// because the brightest cells of this field are the ones the *pointer*
    /// paints, and those are exactly the cells a viewer notices and exactly the
    /// ones a tint makes read as coloured.
    ///
    /// Asserted on every entry rather than on the default alone. The bug this
    /// guards against is specific to adding a *new* ramp, and a test that only
    /// checked `green` would pass over the orange one written wrong on the same
    /// afternoon.
    #[test]
    fn every_named_palette_ends_on_exactly_white() {
        assert!(
            FIELD_PALETTES.len() >= 2,
            "the named palettes are a table with {} entries, so this test is \
             checking the default and nothing else",
            FIELD_PALETTES.len()
        );

        for (name, stops) in FIELD_PALETTES {
            let top = stops.last().unwrap_or_else(|| {
                panic!("the {name} palette is empty, so it paints nothing")
            });
            assert_eq!(
                top,
                &Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255
                },
                "the {name} palette's brightest stop is {top:?}, which is not pure \
                 white. This is the bug the user reported twice: a tinted top \
                 stop reads as a colour cast over the brightest part of the field."
            );
        }
    }

    /// Every named ramp has to get brighter, or the value is not reaching the
    /// colour channel.
    ///
    /// Rec. 601 luma, so the check does not depend on a ramp happening to be one
    /// where the green channel is highest. Non-*decreasing* rather than
    /// strictly increasing: a ramp with two stops at the same luma is a
    /// deliberate choice about where to spend a step, not a fault, and the
    /// `ice` ramp is built that way on purpose -- its dark end is nearly neutral
    /// so that a dim region reads as shadow rather than as colour.
    ///
    /// A *descending* step is a fault, and it is a silent one: the field still
    /// paints, the glyphs still vary, and the only symptom is that the colour
    /// fights the character.
    #[test]
    fn every_named_palette_is_monotone_in_luminance() {
        for (name, stops) in FIELD_PALETTES {
            let luma = |color: &Color| -> f64 {
                let Color::Rgb { r, g, b } = *color else {
                    panic!(
                        "the {name} palette contains {color:?}, which has no \
                            channels to measure"
                    )
                };
                0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
            };

            assert!(
                stops.len() >= 3,
                "the {name} palette has {} stops, which is a boundary rather than \
                 a ramp",
                stops.len()
            );

            let mut previous = -1.0;
            for (index, stop) in stops.iter().enumerate() {
                let here = luma(stop);
                assert!(
                    here >= previous,
                    "the {name} palette goes dark again at stop {index}: \
                     {previous:.1} then {here:.1} in luma, so a brighter value \
                     paints a darker cell"
                );
                previous = here;
            }
        }
    }

    /// The name has to reach the screen, and it has to be the *only* thing that
    /// changed.
    ///
    /// The user asked for "an orange palette", and the way that request fails is
    /// not by being wrong -- it is by parsing, sitting in the options struct, and
    /// never being read. So this measures the painted output rather than the
    /// table, and it checks that the glyph ramp is untouched while it is at it:
    /// a change of colour must not quietly become a change of character, because
    /// that is a much bigger change and it would look like a fix.
    #[test]
    fn a_named_palette_paints_the_screen_and_leaves_the_glyphs_alone() {
        let mut green = AsciiField::new(AsciiFieldOptions::default(), (120, 40));
        let mut orange_field = AsciiField::new(
            AsciiFieldOptions {
                palette: "orange".to_string(),
                ..Default::default()
            },
            (120, 40),
        );
        let orange = palette_by_name("orange").expect("orange is in the table");

        // One `get_diff` each, and the frames held rather than called twice: a
        // second call on an unchanged field returns an *empty* diff, which is
        // this crate's whole design and a quiet way to write a test that
        // compares nothing against something. A green frame collected as an
        // empty vector compares equal to no glyphs at all.
        let green_diff = green.get_diff();
        let orange_diff = orange_field.get_diff();
        assert!(
            green_diff.len() == orange_diff.len(),
            "the two fields drew {} and {} cells of a 120x40 screen",
            green_diff.len(),
            orange_diff.len()
        );
        let green_colors: Vec<Color> =
            green_diff.iter().map(|(_, _, cell)| cell.color).collect();
        let orange_colors: Vec<Color> =
            orange_diff.iter().map(|(_, _, cell)| cell.color).collect();
        let green_glyphs: Vec<char> =
            green_diff.iter().map(|(_, _, cell)| cell.symbol).collect();
        let orange_glyphs: Vec<char> =
            orange_diff.iter().map(|(_, _, cell)| cell.symbol).collect();

        assert_eq!(
            green_glyphs, orange_glyphs,
            "changing the palette changed the characters as well"
        );
        assert_ne!(
            green_colors, orange_colors,
            "palette = \"orange\" painted the same colours as the default, so the \
             name is being parsed and not used"
        );
        // Warm, checked on the ramp rather than on every painted cell. The top
        // stop is white by design and white is not warm, so the claim is about
        // the coloured stops: red above green above blue, which is what "orange"
        // means as a hue and is the property a wrong ramp would lose first.
        //
        // Deliberately not a fixed margin between red and blue. There is no
        // honest one: the stops run from rgb(26, 10, 4), where red is 22 ahead of
        // blue, to rgb(255, 140, 32) where it is 223 ahead, and a threshold wide
        // enough for the first would pass a grey ramp. Ordering is the claim that
        // survives the whole range.
        for (index, color) in orange
            .iter()
            .enumerate()
            .take(orange.len().saturating_sub(1))
        {
            let Color::Rgb { r, g, b } = *color else {
                panic!("the orange palette contains {color:?}")
            };
            assert!(
                r > g && g > b,
                "orange stop {index} is rgb({r}, {g}, {b}), which is not a warm \
                 hue: orange is red above green above blue"
            );
        }
    }

    /// The top stop has to be reachable *on screen*, not just in the table.
    ///
    /// A constant can be correct and the sampler can still be painting the wrong
    /// end of it, which is the mistake
    /// `the_brightest_value_draws_the_brightest_stop` already makes for the
    /// default. This is the same check for every name, because a new ramp and a
    /// new sampler are exactly as likely to disagree as an old one and a new one.
    #[test]
    fn every_named_palettes_brightest_value_paints_its_white() {
        for (name, stops) in FIELD_PALETTES {
            let palette = GlyphPalette::new(DEFAULT_GLYPHS, stops.to_vec());
            assert_eq!(
                palette.sample(1.0).color,
                Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255
                },
                "the {name} palette's value 1.0 did not paint white, so either the \
                 table's top stop is not what this test thinks it is or the \
                 sampler is reading the ramp upside down"
            );
            assert_eq!(
                palette.sample(0.0).color,
                stops[0],
                "the {name} palette's value 0.0 did not paint its darkest stop"
            );
        }
    }

    /// A name the table does not have has to leave a drawable field.
    ///
    /// `palette = "ornage"` is a typo someone will make, and it is a typo
    /// someone will make in a *config file*, which is to say at three in the
    /// morning with no error message. The alternatives are an unknown-ramp
    /// failure, which means every effect's config parser has to learn about
    /// them, or a blank screen, which means the effect is broken and says
    /// nothing. Falling back to the default is the same choice
    /// `an_unusable_colour_list_falls_back_rather_than_drawing_nothing` makes
    /// for an empty `colors` list, and for the same reason.
    #[test]
    fn an_unknown_palette_name_falls_back_rather_than_drawing_nothing() {
        for name in ["", "ornage", "grn", "puce", "  ", "green ", "ORANGE!"] {
            let options = AsciiFieldOptions {
                palette: name.to_string(),
                ..Default::default()
            };
            let mut field = AsciiField::new(options.clone(), (60, 20));
            let diff = field.get_diff();
            assert_eq!(
                diff.len(),
                60 * 20,
                "palette = {name:?} drew {} cells of a 60x20 field",
                diff.len()
            );

            let colors: HashSet<Color> =
                diff.iter().map(|(_, _, cell)| cell.color).collect();
            assert!(
                colors.contains(&Color::Rgb {
                    r: 255,
                    g: 255,
                    b: 255
                }),
                "palette = {name:?} painted {colors:?} and not one of them is \
                 white, so it did not fall back to the default ramp"
            );
        }

        // A good name that differs only in case is a *different* case: not a
        // fallback, the thing the user asked for. Orange is not a subset of
        // green, so this cannot pass by accident.
        let mut field = AsciiField::new(
            AsciiFieldOptions {
                palette: "OrAnGe".to_string(),
                ..Default::default()
            },
            (60, 20),
        );
        let colors: HashSet<Color> = field
            .get_diff()
            .iter()
            .map(|(_, _, cell)| cell.color)
            .collect();
        let orange = palette_by_name("orange").expect("orange is in the table");
        assert!(
            colors.iter().all(|color| orange.contains(color)),
            "palette = \"OrAnGe\" painted {colors:?}, which is not the orange \
             ramp, so the name lookup is case-sensitive"
        );
    }

    /// The name has to survive a config file, which is the only way a user will
    /// ever set it.
    ///
    /// Two things to check and one trap. The trap is that `--print-config`
    /// writes every key to disk, so the *default* is now pinned in every
    /// existing config file. That is not a reason to leave the key out -- the
    /// contract suite deletes one key at a time and re-parses, which is how this
    /// gets caught -- but it is the reason `palette` defaults to the empty string
    /// and not to `"green"`: an empty string means "no name set, use `colors`",
    /// and it keeps the two options' defaults from silently disagreeing with each
    /// other in every file anyone has already generated.
    #[test]
    fn the_named_palette_round_trips_through_toml() {
        let config: AsciiFieldOptions =
            toml::from_str("palette = \"orange\"\n").expect("a lone key parses");
        assert_eq!(
            config.palette, "orange",
            "the key was not read back, so a config file could not ask for a \
             palette by name at all"
        );
        assert_eq!(
            config.colors,
            AsciiFieldOptions::default().colors,
            "one key in the section silently reset the others"
        );

        let serialised = toml::to_string(&config).expect("the section serialises");
        assert!(
            serialised.contains("palette"),
            "the key is missing from the serialised form, so --print-config would \
             never write it: {serialised}"
        );
    }

    /// The default has to be what it always was, or every existing install
    /// changes appearance on upgrade.
    ///
    /// This is the "nothing changes for anyone who has not asked for this" claim
    /// as a test, and it is asserted against [`PHOSPHOR_RAMP`] rather than
    /// against a copy of the list, so it fails if either side moves.
    #[test]
    fn the_default_named_palette_is_the_phosphor_ramp() {
        let named = palette_by_name(DEFAULT_PALETTE_NAME)
            .expect("the default palette is in the table");
        assert_eq!(
            named, PHOSPHOR_RAMP,
            "palette = {DEFAULT_PALETTE_NAME:?} is not the ramp this effect has \
             always used, so every existing config changes appearance"
        );
        assert_eq!(
            AsciiFieldOptions::default().colors,
            PHOSPHOR_RAMP.to_vec(),
            "the default `colors` is not the phosphor ramp either, so the two \
             knobs disagree about what the default is"
        );
    }

    /// The two knobs have to pick one, and the one they pick has to be the one
    /// documented.
    ///
    /// The interaction is `palette` winning, and the reason it has to be is
    /// uncomfortable enough to be worth a test: the default `colors` is
    /// *always* populated, so "the list wins when it is set" is not a rule, it
    /// is "the list never wins". A user with a generated config who adds
    /// `palette = "orange"` and still gets green has no other way to interpret
    /// it.
    ///
    /// The reverse direction is asserted too, because it is the one that breaks
    /// silently: `palette = ""` has to go back to the list, and it is the escape
    /// hatch the note on `colors` offers, so an escape hatch that does not work
    /// is worse than not documenting it.
    #[test]
    fn the_palette_name_wins_over_a_configured_colour_list() {
        let listed = Color::Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        let options = AsciiFieldOptions {
            palette: "blue".to_string(),
            colors: vec![listed],
            ..Default::default()
        };
        let mut field = AsciiField::new(options.clone(), (40, 12));
        let colors: HashSet<Color> = field
            .get_diff()
            .iter()
            .map(|(_, _, cell)| cell.color)
            .collect();
        let blue = palette_by_name("blue").expect("blue is in the table");
        assert!(
            colors.iter().all(|color| blue.contains(color)) && colors.len() > 1,
            "palette = \"blue\" alongside a `colors` list painted {colors:?}, so \
             the two knobs are not resolving to a single answer"
        );

        // And the escape hatch. Empty name, so the list, so exactly one colour.
        let mut listed_only = AsciiField::new(
            AsciiFieldOptions {
                palette: String::new(),
                ..options
            },
            (40, 12),
        );
        let colors: HashSet<Color> = listed_only
            .get_diff()
            .iter()
            .map(|(_, _, cell)| cell.color)
            .collect();
        assert_eq!(
            colors,
            HashSet::from([listed]),
            "setting palette = \"\" did not go back to the `colors` list, so the \
             documented escape hatch does not work"
        );
    }

    /// The name has to survive `[` and `]`.
    ///
    /// The bug this is the second half of: `set_palette` rebuilds the renderer
    /// from scratch, and a version that read `self.options.colors` there would
    /// make a named palette work on the first frame and then revert the moment
    /// anyone pressed a key. The existing
    /// `cycling_the_glyph_set_keeps_the_configured_colours` test covers the
    /// `colors` half of the same seam.
    #[test]
    fn cycling_the_glyph_set_keeps_the_named_palette() {
        let mut field = AsciiField::new(
            AsciiFieldOptions {
                palette: "magenta".to_string(),
                ..Default::default()
            },
            (40, 12),
        );
        let magenta = palette_by_name("magenta").expect("magenta is in the table");

        for _ in 0..6 {
            field.handle_input(&crate::runtime::InputEvent::Key {
                key: crate::runtime::Key::Char(']'),
                phase: crate::runtime::KeyPhase::Pressed,
            });
            let colors: HashSet<Color> = field
                .get_diff()
                .iter()
                .map(|(_, _, cell)| cell.color)
                .collect();
            assert!(
                colors.iter().all(|color| magenta.contains(color)),
                "after cycling to glyph set {} the field painted {colors:?}, so the \
                 named palette was dropped by the palette switch",
                field.glyph_set_index()
            );
        }
    }

    /// The names are the feature, so they have to be findable and distinct.
    #[test]
    fn every_named_palette_is_findable_and_distinct() {
        let names: Vec<&str> =
            FIELD_PALETTES.iter().map(|(name, _)| *name).collect();
        for expected in ["green", "orange", "blue", "magenta", "ice", "amber"] {
            assert!(
                names.contains(&expected),
                "the {expected:?} palette is missing; the names are {names:?}"
            );
        }
        assert_eq!(
            names.len(),
            FIELD_PALETTES.len(),
            "the same name is in the table twice: {names:?}"
        );

        for (index, (name, stops)) in FIELD_PALETTES.iter().enumerate() {
            for (other_name, other) in &FIELD_PALETTES[index + 1..] {
                assert_ne!(
                    stops, other,
                    "{name} and {other_name} are the same ramp, so one of them is \
                     a duplicate"
                );
            }
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
