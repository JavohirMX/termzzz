//! Colour ramps shared between effects.
//!
//! A ramp is a small number of anchor colours plus the rule for getting between
//! them. Effects that each grow their own ramp end up with subtly different
//! greens and magentas for the same idea, and the fire effect's hand-rolled
//! 768-byte lookup table is the shape this generalises.

use crossterm::style::Color;

/// A gradient between anchor colours, sampled by a position in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    stops: Vec<Color>,
}

impl Palette {
    /// A ramp through the given colours, evenly spaced.
    ///
    /// A single stop is allowed and yields a flat colour; an empty ramp falls
    /// back to white rather than panicking, because a palette built from user
    /// configuration should not be able to crash a renderer.
    pub fn new(stops: Vec<Color>) -> Self {
        let stops = if stops.is_empty() {
            vec![Color::White]
        } else {
            stops
        };
        Self { stops }
    }

    /// A ramp from `Color::Rgb` triples in `0.0..=1.0`.
    pub fn from_rgb(stops: Vec<[f32; 3]>) -> Self {
        Self::new(
            stops
                .into_iter()
                .map(|rgb| Color::Rgb {
                    r: (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                    g: (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                    b: (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                })
                .collect(),
        )
    }

    /// The number of anchors.
    pub fn len(&self) -> usize {
        self.stops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stops.is_empty()
    }

    /// The colour at `t` in `0.0..=1.0`.
    ///
    /// `t` is clamped, so a computed value that overshoots saturates at an
    /// endpoint rather than wrapping around to the far end. The fire effect's
    /// ramp once ran its interpolation to 2.0 for exactly that reason and
    /// painted its tail black.
    pub fn sample(&self, t: f32) -> Color {
        // `f32::clamp` propagates NaN rather than saturating it, so a NaN would
        // sail through the bounds check and the index arithmetic below and come
        // out as whatever that produced. Treated as the bottom of the ramp.
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        if self.stops.len() == 1 {
            return self.stops[0];
        }

        let scaled = t * (self.stops.len() - 1) as f32;
        let index = scaled.floor() as usize;
        let index = index.min(self.stops.len() - 2);
        let fraction = scaled - index as f32;

        lerp(self.stops[index], self.stops[index + 1], fraction)
    }

    /// The colour at an integer index, wrapping.
    ///
    /// For effects that treat the ramp as a cyclic lookup, which is how the
    /// plasma effect cycles its colours over time.
    pub fn sample_index(&self, index: usize) -> Color {
        self.stops[index % self.stops.len()]
    }

    /// The colour at `t` in `0.0..=1.0`, as floats in `0.0..=1.0`.
    ///
    /// For an effect filling a [`super::HalfBlockField`], which stores floats so
    /// a slow gradient keeps its steps at the dark end. A ramp whose stops are
    /// not `Color::Rgb` has no channels to give, and yields white.
    pub fn sample_rgb(&self, t: f32) -> [f32; 3] {
        match self.sample(t) {
            Color::Rgb { r, g, b } => {
                [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
            }
            _ => [1.0, 1.0, 1.0],
        }
    }

    /// Like [`sample`](Self::sample), but `t` wraps at the top instead of
    /// saturating.
    ///
    /// For a ramp that is meant to *cycle* -- a colour wheel, a slowly rotating
    /// offset into a fixed palette. [`sample`](Self::sample) clamps, which is
    /// right for a brightness ramp and catastrophic for a cyclic one: a mandelbrot
    /// was adding an unbounded time offset to a value already near 1.0, so after
    /// about seven seconds every pixel was above the clamp and the whole frame was
    /// one colour until the camera recentred.
    ///
    /// Wrapping is only seamless if the ramp's last stop and first stop are close
    /// in colour. This crate's mandelbrot ramp satisfies that by accident worth
    /// keeping: it ends on near-black and starts on dark navy.
    pub fn sample_wrapped(&self, t: f32) -> Color {
        let t = if t.is_nan() { 0.0 } else { t.rem_euclid(1.0) };
        self.sample(t)
    }

    /// [`sample_wrapped`](Self::sample_wrapped), returning floats.
    pub fn sample_rgb_wrapped(&self, t: f32) -> [f32; 3] {
        match self.sample_wrapped(t) {
            Color::Rgb { r, g, b } => {
                [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
            }
            _ => [1.0, 1.0, 1.0],
        }
    }

    /// Looks up a named ramp.
    ///
    /// For effects that take `palette = "ember"` in their config rather than a
    /// list of colours. An inline list in TOML is unpleasant to write and worse to
    /// read back, and the useful ramps are shared: mandelbrot, donut, fire, ink
    /// and constellation all want "the same six colours, arranged differently".
    pub fn named(name: &str) -> Option<Palette> {
        presets::by_name(name).map(|stops| Palette::new(stops.to_vec()))
    }

    /// Every named ramp, for `--help` and the docs.
    pub fn preset_names() -> Vec<&'static str> {
        presets::ALL.iter().map(|(name, _)| *name).collect()
    }

    /// This ramp cut off at its brightest stop, for a *brightness* consumer.
    ///
    /// ## Why this exists
    ///
    /// Nearly every ramp in [`presets`] is **mirrored**: it climbs to a peak and
    /// comes back down again, so that [`sample_wrapped`](Self::sample_wrapped) has
    /// no visible seam where it wraps. That is right for a cycling consumer and
    /// catastrophic for a brightness one, and it fails in a way no amount of
    /// looking at the code catches.
    ///
    /// [`sample`](Self::sample) returns `stops[len - 1]` at `t = 1.0`. For a
    /// mirrored ramp that stop is the second-*darkest* one, because the mirroring
    /// is symmetric -- so the top of a brightness scale lands on near-black and the
    /// ramp's brightest colour sits unused in the middle, where the field value
    /// that reaches it is a coincidence rather than the maximum.
    ///
    /// Measured on `MAGMA` as a brightness ramp, which is what `ants` did with it:
    ///
    /// ```text
    /// t      0.125   0.25    0.375   0.5     0.625   0.75    0.875   1.0
    /// rgb    93,23   200,75  253,170 253,239 253,235 253,159 192,65   81,18
    /// ```
    ///
    /// It peaks at `t = 0.5` and falls back to dark at the top, so a value at the
    /// *maximum* of the scale was drawn in near-black and a value at a quarter of
    /// it was drawn in cream. Four effects were built on that.
    ///
    /// ## Peak choice
    ///
    /// Rec. 709 luma, not max-of-channels: a ramp that peaks in blue is dark to
    /// the eye, and `DEPTH` peaks in blue at index 2. First stop wins a tie, so a
    /// flat ramp is returned as itself rather than truncated to nothing.
    ///
    /// ## A flat result is a real answer
    ///
    /// If the brightest stop is the first, there is no ascending half and this
    /// returns a one-stop ramp -- which [`sample`](Self::sample) handles as a flat
    /// colour. A user-supplied ramp can get here, and a solid block is a better
    /// failure than the inverted scale this exists to prevent.
    pub fn truncated_at_peak(&self) -> Palette {
        let mut peak = 0usize;
        let mut best = f32::NEG_INFINITY;
        for (i, color) in self.stops.iter().enumerate() {
            let luma = luminance(*color);
            if luma > best {
                best = luma;
                peak = i;
            }
        }
        Palette::new(self.stops[..=peak].to_vec())
    }

    /// The anchors, for an effect that wants to reason about them.
    pub fn stops(&self) -> &[Color] {
        &self.stops
    }

    /// A ramp of `n` evenly spaced samples, for an effect that wants a table.
    pub fn expand(&self, n: usize) -> Vec<Color> {
        let n = n.max(2);
        (0..n)
            .map(|i| self.sample(i as f32 / (n - 1) as f32))
            .collect()
    }
}

/// Interpolates two colours, treating anything that is not `Color::Rgb` as
/// having no channels to blend.
pub fn lerp(a: Color, b: Color, t: f32) -> Color {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    match (a, b) {
        (
            Color::Rgb {
                r: r0,
                g: g0,
                b: b0,
            },
            Color::Rgb {
                r: r1,
                g: g1,
                b: b1,
            },
        ) => Color::Rgb {
            r: lerp_u8(r0, r1, t),
            g: lerp_u8(g0, g1, t),
            b: lerp_u8(b0, b1, t),
        },
        // Named colours have well-defined RGB values in the standard 16-colour
        // palette, so they blend with a truecolor like anything else. They have to:
        // `lerp(Color::Black, ink, t)` is the natural way to write "fade this in
        // from the background", and under the old policy it returned `Color::Black`
        // for every `t < 0.5` and the ink for every `t >= 0.5`. That is a hard step
        // at the halfway point, not a fade, and it looks like working code.
        // A named colour mixed with a truecolor. Resolved independently per side,
        // so `Black` against an `Rgb` blends -- which is the case that matters,
        // because "fade this in from the background" is written that way.
        (a, b) => match (named_rgb(a), named_rgb(b)) {
            (Some((r0, g0, b0)), Some((r1, g1, b1))) => Color::Rgb {
                r: lerp_u8(r0, r1, t),
                g: lerp_u8(g0, g1, t),
                b: lerp_u8(b0, b1, t),
            },
            // `Color::Reset` means "whatever the terminal's foreground is", which
            // this code has no way to know, and `AnsiValue` means "index into a
            // palette whose contents are the terminal's business". Neither has an
            // answer, so the nearer endpoint wins rather than inventing one.
            _ => {
                if t < 0.5 {
                    a
                } else {
                    b
                }
            }
        },
    }
}

/// The standard 16-colour palette as RGB, for the variants that have one.
///
/// `None` for `Reset` and `AnsiValue`, which are not resolvable without knowing
/// the terminal's configuration. Those are the only two that make
/// [`lerp`] fall back to picking an endpoint.
fn named_rgb(color: Color) -> Option<(u8, u8, u8)> {
    Some(match color {
        // A truecolor resolves to itself. The two-`Rgb` case is matched before
        // this is ever consulted, so it costs nothing on the hot path -- it is here
        // so that a *named* colour mixed with a truecolor resolves both sides.
        Color::Rgb { r, g, b } => (r, g, b),
        Color::Black => (0, 0, 0),
        Color::DarkGrey => (128, 128, 128),
        Color::Red => (170, 0, 0),
        Color::DarkRed => (170, 0, 0),
        Color::Green => (0, 170, 0),
        Color::DarkGreen => (0, 170, 0),
        Color::Yellow => (170, 170, 0),
        Color::DarkYellow => (170, 170, 0),
        Color::Blue => (0, 0, 170),
        Color::DarkBlue => (0, 0, 170),
        Color::Magenta => (170, 0, 170),
        Color::DarkMagenta => (170, 0, 170),
        Color::Cyan => (0, 170, 170),
        Color::DarkCyan => (0, 170, 170),
        Color::Grey => (170, 170, 170),
        Color::White => (255, 255, 255),
        _ => return None,
    })
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Rec. 709 relative luminance in `0.0..=1.0`.
///
/// The perceptual weights, not a channel average, and not max-of-channels. This is
/// what "bright" has to mean for a ramp to be ordered correctly: a saturated blue
/// has the highest channel value in a navy ramp and is one of the *darkest*
/// colours on the screen, and a ramp that peaks in blue peaks in the dark.
///
/// Named colours resolve through [`named_rgb`], so this answers for the whole
/// [`Color`] type rather than only the truecolor case. `Reset` and `AnsiValue`
/// have no answer without knowing the terminal, and return 0.0 -- the dark end,
/// which is the safe direction for every caller here.
pub fn luminance(color: Color) -> f32 {
    let (r, g, b) = named_rgb(color).unwrap_or((0, 0, 0));
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

/// A colour at a given OKLab hue, at fixed lightness and chroma.
///
/// This is what a *categorical* palette wants, and building one any other way is
/// a trap worth recording. The obvious alternative is to hold HSV's *saturation*
/// and *value* fixed and vary the hue, and HSV value is not perceptual lightness:
/// at `S = 0.85, V = 0.95` the six sector anchors come out with OKLab lightnesses
/// spread over **0.47**, which is roughly ten times a just-noticeable difference.
/// So a set of HSV colours told apart only
/// by hue is really being told apart mostly by *brightness* -- and if the effect
/// is also using brightness for something, which `newton` is, the two channels
/// fight over the same signal and the categories stop reading as categories.
///
/// Holding lightness and chroma fixed removes that by construction: the colours
/// differ in hue and in nothing else. Measured minimum pairwise
/// [`perceptual_distance`] across degrees 3, 4 and 5, at `L = 0.75, C = 0.20`:
///
/// ```text
/// degree 3   0.282
/// degree 4   0.211
/// degree 5   0.172
/// ```
///
/// against a threshold of 0.15, with the lightness spread down to 0.048. The same
/// three degrees on an HSV wheel of fixed saturation and value measure 0.475,
/// 0.121 and 0.181 -- so HSV is *better* at three and worse at four, which is worth
/// knowing before anyone concludes that a hue wheel is a hue wheel.
///
/// `hue` is in turns, `0.0..=1.0`, and wraps. Out-of-gamut results are clamped
/// per channel, which costs some chroma near the corners of the space rather than
/// producing a colour outside sRGB.
pub fn oklab_hue(lightness: f32, chroma: f32, hue: f32) -> Color {
    let lightness = if lightness.is_nan() {
        0.75
    } else {
        lightness.clamp(0.0, 1.0)
    };
    let chroma = if chroma.is_nan() {
        0.0
    } else {
        chroma.max(0.0)
    };
    let hue = if hue.is_nan() { 0.0 } else { hue } * std::f32::consts::TAU;

    let (r, g, b) =
        oklab_to_srgb(lightness, chroma * hue.cos(), chroma * hue.sin());
    Color::Rgb { r, g, b }
}

/// The inverse of [`oklab`], clamping out-of-gamut channels.
fn oklab_to_srgb(l: f32, a: f32, b: f32) -> (u8, u8, u8) {
    let l_ = l + 0.396_337_8 * a + 0.215_803_8 * b;
    let m_ = l - 0.105_561_3 * a - 0.063_854_2 * b;
    let s_ = l - 0.089_484_2 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    let r = 4.076_742 * l3 - 3.307_711_6 * m3 + 0.230_969_9 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
    let b = -0.004_196_1 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;

    let encode = |v: f32| -> u8 {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    };
    (encode(r), encode(g), encode(b))
}

/// Rescales a colour's value channel, keeping its hue and saturation.
///
/// The shading half of a two-channel colour scheme: `newton` picks the hue from
/// which root a sample reached and then uses this to encode how many iterations
/// that took, so one cell carries both facts.
///
/// Darkening rather than lightening, because a field drawn on a black terminal
/// has room below and none above -- and because a dark cell is one the glyph ramp
/// is already backing up with a sparser mark.
///
/// The shading half of a two-channel colour scheme: `newton` picks the hue from
/// which root a sample reached and then uses this to encode how many iterations
/// that took, so one cell carries both facts.
///
/// Darkening rather than lightening, because a field drawn on a black terminal
/// has room below and none above.
pub fn shade(color: Color, value: f32) -> Color {
    match color {
        Color::Rgb { r, g, b } => {
            let k = if value.is_nan() {
                1.0
            } else {
                value.clamp(0.0, 1.0)
            };
            Color::Rgb {
                r: (r as f32 * k).round() as u8,
                g: (g as f32 * k).round() as u8,
                b: (b as f32 * k).round() as u8,
            }
        }
        other => other,
    }
}

/// Distance between two colours in OKLab, the cheap perceptual space.
///
/// For "can a viewer tell these two apart", which is the question every choice
/// of categorical palette has to answer, and the one that eyeballing a hex code
/// cannot.
///
/// ## The scale, because it is not the one you would guess
///
/// **OKLab is not scaled to 0-100.** There is no multiply-by-100 convention here,
/// and assuming one is exactly how this function's first caller came to set a
/// threshold of `30` against a quantity whose entire useful range is about `0.6` --
/// so every assertion built on it failed while looking like it was testing
/// separation. The numbers to compare against:
///
/// - **about 0.02** -- a just-noticeable difference, the smallest gap a viewer
///   reliably sees;
/// - **about 0.1** -- comfortably different, obviously two things;
/// - **about 0.3** -- different categories;
/// - **about 0.5** -- about as far apart as two colours get without a third hue
///   between them. Saturated red against saturated green measures 0.52 here.
///
/// So a categorical palette wants its closest pair above roughly `0.15`, and a
/// pair under `0.05` is a shading step however different the hex codes look.
///
/// The transform is the standard sRGB-linear to LMS to OKLab chain, and the linear
/// step is not optional -- skipping it shifts lightness noticeably in the mid-tones,
/// which is exactly where a categorical set lives. Raw RGB is a poor perceptual
/// space for the same reason this function exists: pure blue and pure green are
/// 255 apart per channel and nearly the same brightness, while a colour and a slightly darker copy of
/// itself are close in every channel and can still be far apart to the eye.
pub fn perceptual_distance(a: Color, b: Color) -> f32 {
    let (ar, ag, ab) = named_rgb(a).unwrap_or((0, 0, 0));
    let (br, bg, bb) = named_rgb(b).unwrap_or((0, 0, 0));
    let (al, aa, ac) = oklab((ar, ag, ab));
    let (bl, ba, bc) = oklab((br, bg, bb));
    let (dl, da, dc) = (al - bl, aa - ba, ac - bc);
    (dl * dl + da * da + dc * dc).sqrt()
}

/// One colour into OKLab's `(L, a, b)`.
fn oklab((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    fn lin(v: u8) -> f32 {
        let v = v as f32 / 255.0;
        // sRGB is not linear, and skipping this step is the single most common
        // way to get a perceptual transform subtly wrong: it shifts lightness
        // noticeably in the mid-tones, which is exactly where a categorical set
        // lives.
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }

    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = 0.412_221_5 * r + 0.536_332_5 * g + 0.051_446 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_397 * b;
    let s = 0.088_302_5 * r + 0.281_718_8 * g + 0.629_978_7 * b;

    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    (
        0.210_454_3 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
        1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
        0.025_904 * l_ + 0.782_771_8 * m_ - 0.808_675_8 * s_,
    )
}

/// Black to white, the ramp a scalar field wants when nothing else is specified.
pub fn greyscale() -> Palette {
    Palette::from_rgb(vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]])
}

/// Named ramps, for effects that take `palette = "..."` in their config.
///
/// The escape-time colouring tradition is that the ramp's last stop and first stop
/// are close in colour, because the bands cycle. Every ramp here satisfies that,
/// which is what makes [`Palette::sample_wrapped`] seamless rather than a visible
/// seam once per lap.
pub mod presets {
    use crossterm::style::Color::{self, Rgb};

    /// Navy through blue, a bright band, amber, red, and back to near-black.
    ///
    /// The mandelbrot's original ramp, kept as the default because it is the one
    /// the effect was tuned against. The bright third is deliberate: it puts a
    /// hard edge where escape counts cross from fast to slow, which is where the
    /// boundary detail lives.
    pub const DEPTH: &[Color] = &[
        Rgb { r: 0, g: 7, b: 100 },
        Rgb {
            r: 32,
            g: 107,
            b: 203,
        },
        Rgb {
            r: 237,
            g: 255,
            b: 255,
        },
        Rgb {
            r: 255,
            g: 170,
            b: 0,
        },
        Rgb { r: 204, g: 0, b: 0 },
        Rgb { r: 16, g: 8, b: 40 },
    ];

    /// Black to red to orange to yellow to white, and back down again.
    ///
    /// Fire, for a set boundary. The descending half is not an accident: a ramp
    /// that goes dark to light cannot cycle, because the wrap from light back to
    /// dark is a seam. Mirroring makes it a triangle wave, so the bands run out
    /// and back and the cycle closes on a colour close to where it started. Every
    /// multi-stop ramp here is mirrored for that reason, and
    /// `every_preset_cycles_without_a_visible_seam` in the mandelbrot is the guard.
    pub const EMBER: &[Color] = &[
        Rgb { r: 0, g: 0, b: 0 },
        Rgb { r: 120, g: 0, b: 0 },
        Rgb {
            r: 200,
            g: 60,
            b: 0,
        },
        Rgb {
            r: 255,
            g: 160,
            b: 0,
        },
        Rgb {
            r: 255,
            g: 240,
            b: 160,
        },
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        Rgb {
            r: 255,
            g: 160,
            b: 0,
        },
        Rgb {
            r: 200,
            g: 60,
            b: 0,
        },
        Rgb { r: 120, g: 0, b: 0 },
    ];

    /// Black to deep blue to cyan to white, and back down again.
    ///
    /// Cold, and the most legible of the set on a dark profile because the
    /// mid-tones stay far apart in luminance.
    pub const OCEAN: &[Color] = &[
        Rgb { r: 0, g: 0, b: 20 },
        Rgb {
            r: 0,
            g: 40,
            b: 120,
        },
        Rgb {
            r: 0,
            g: 130,
            b: 190,
        },
        Rgb {
            r: 60,
            g: 210,
            b: 220,
        },
        Rgb {
            r: 220,
            g: 250,
            b: 255,
        },
        Rgb {
            r: 60,
            g: 210,
            b: 220,
        },
        Rgb {
            r: 0,
            g: 130,
            b: 190,
        },
        Rgb {
            r: 0,
            g: 40,
            b: 120,
        },
        Rgb { r: 0, g: 10, b: 50 },
    ];

    /// Magma: the standard perceptually-uniform ramp, and back down again.
    ///
    /// Dark purple through red and orange to cream, with roughly even *perceived*
    /// lightness steps, which is the property [`DEPTH`] trades away for the bright
    /// band in its middle. As given it is one of the best ramps here to look at and
    /// one of the worst to cycle, which is why the descending half is included.
    pub const MAGMA: &[Color] = &[
        Rgb { r: 0, g: 0, b: 4 },
        Rgb {
            r: 81,
            g: 18,
            b: 124,
        },
        Rgb {
            r: 183,
            g: 55,
            b: 121,
        },
        Rgb {
            r: 252,
            g: 137,
            b: 97,
        },
        Rgb {
            r: 254,
            g: 224,
            b: 181,
        },
        Rgb {
            r: 252,
            g: 253,
            b: 191,
        },
        Rgb {
            r: 254,
            g: 224,
            b: 181,
        },
        Rgb {
            r: 252,
            g: 137,
            b: 97,
        },
        Rgb {
            r: 183,
            g: 55,
            b: 121,
        },
        Rgb {
            r: 81,
            g: 18,
            b: 124,
        },
    ];

    /// Two stops only, so `Palette::sample` returns either black or white.
    ///
    /// Reads as a pure boundary map with no interior shading, which is
    /// occasionally exactly what you want and otherwise a mistake. The one ramp
    /// here that is exempt from the seamless-wrap rule, and legitimately so: with
    /// two stops every band boundary is already a hard black-to-white edge, so
    /// there is no seam distinguishable from the rest of the pattern.
    pub const CONTRAST: &[Color] = &[
        Rgb { r: 0, g: 0, b: 0 },
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
    ];

    /// Every ramp above, in the order they should be offered to a user.
    pub const ALL: &[(&str, &[Color])] = &[
        ("depth", DEPTH),
        ("ember", EMBER),
        ("ocean", OCEAN),
        ("magma", MAGMA),
        ("contrast", CONTRAST),
    ];

    /// Looks up a ramp by name, case-insensitively.
    pub fn by_name(name: &str) -> Option<&'static [Color]> {
        ALL.iter()
            .find(|(preset, _)| preset.eq_ignore_ascii_case(name))
            .map(|(_, stops)| *stops)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb { r, g, b }
    }

    #[test]
    fn the_endpoints_are_exact() {
        let palette = Palette::new(vec![rgb(0, 0, 0), rgb(255, 255, 255)]);
        assert_eq!(palette.sample(0.0), rgb(0, 0, 0));
        assert_eq!(palette.sample(1.0), rgb(255, 255, 255));
    }

    #[test]
    fn the_middle_is_the_middle() {
        let palette = Palette::new(vec![rgb(0, 0, 0), rgb(200, 100, 50)]);
        assert_eq!(palette.sample(0.5), rgb(100, 50, 25));
    }

    #[test]
    fn an_overshoot_saturates_rather_than_wrapping() {
        // The bug the fire ramp had: an interpolation parameter above 1.0 ran
        // past the end colour and the tail came out black.
        let palette = Palette::new(vec![rgb(255, 0, 0), rgb(0, 0, 255)]);
        assert_eq!(palette.sample(1.5), rgb(0, 0, 255));
        assert_eq!(palette.sample(-0.5), rgb(255, 0, 0));
        assert_eq!(palette.sample(f32::NAN), rgb(255, 0, 0));
    }

    #[test]
    fn a_single_stop_is_flat() {
        let palette = Palette::new(vec![rgb(10, 20, 30)]);
        assert_eq!(palette.sample(0.0), rgb(10, 20, 30));
        assert_eq!(palette.sample(0.7), rgb(10, 20, 30));
        assert_eq!(palette.sample(1.0), rgb(10, 20, 30));
    }

    #[test]
    fn an_empty_ramp_is_white_rather_than_a_panic() {
        let palette = Palette::new(Vec::new());
        assert_eq!(palette.len(), 1);
        assert_eq!(palette.sample(0.5), Color::White);
    }

    #[test]
    fn intermediate_stops_are_evenly_spaced() {
        // Three stops: 0.0 black, 0.5 red, 1.0 white.
        let palette =
            Palette::new(vec![rgb(0, 0, 0), rgb(255, 0, 0), rgb(255, 255, 255)]);
        assert_eq!(palette.sample(0.5), rgb(255, 0, 0));
        assert_eq!(palette.sample(0.25), rgb(128, 0, 0));
        assert_eq!(palette.sample(0.75), rgb(255, 128, 128));
    }

    #[test]
    fn index_sampling_wraps_but_does_not_panic() {
        let palette = Palette::new(vec![rgb(1, 1, 1), rgb(2, 2, 2)]);
        assert_eq!(palette.sample_index(0), rgb(1, 1, 1));
        assert_eq!(palette.sample_index(1), rgb(2, 2, 2));
        assert_eq!(palette.sample_index(2), rgb(1, 1, 1));
        assert_eq!(palette.sample_index(99), rgb(2, 2, 2));
    }

    #[test]
    fn expand_produces_the_requested_count_with_exact_endpoints() {
        let palette = Palette::new(vec![rgb(0, 0, 0), rgb(255, 0, 0)]);
        let table = palette.expand(5);
        assert_eq!(table.len(), 5);
        assert_eq!(table[0], rgb(0, 0, 0));
        assert_eq!(table[4], rgb(255, 0, 0));
    }

    #[test]
    fn expand_never_returns_a_single_entry() {
        // A one-entry table would divide by zero below.
        let table = Palette::new(vec![rgb(0, 0, 0), rgb(1, 1, 1)]).expand(1);
        assert_eq!(table.len(), 2);
    }

    /// Fading to black has to be a fade.
    ///
    /// This is a real bug that was found by an effect, not by reading the code:
    /// `lerp` had a policy of "endpoints win" for any pair it could not blend,
    /// and `Color::Black` fell into it because the match only handled two `Rgb`
    /// values. So `lerp(Color::Black, ink, 0.4)` returned `Color::Black` and
    /// `lerp(Color::Black, ink, 0.6)` returned `ink` -- a hard step at the halfway
    /// point, from code that reads exactly like a working fade. Every one of
    /// those effects was drawing full-brightness trail cells and calling it a
    /// gradient.
    #[test]
    fn fading_from_black_is_a_fade_and_not_a_step() {
        let ink = Color::Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        let mut previous = lerp(Color::Black, ink, 0.9);
        for step in 1..=20 {
            let t = step as f32 / 20.0;
            let blended = lerp(Color::Black, ink, t);
            assert_ne!(
                blended, previous,
                "at t = {t} the colour did not change, so this is a step"
            );
            previous = blended;
        }

        // And it lands exactly on the endpoints -- as *resolved* colours, not as
        // the named variants they were given. `Color::Black` becomes
        // `rgb(0,0,0)`, which renders identically and costs a few more bytes on
        // the wire. That is the price of resolving, and it is worth paying: a
        // caller that mixes a named colour with a truecolor now gets a truecolor
        // back rather than a colour that silently changes meaning depending on
        // which side of the blend it was on.
        assert_eq!(
            lerp(Color::Black, ink, 0.0),
            Color::Rgb { r: 0, g: 0, b: 0 }
        );
        assert_eq!(lerp(Color::Black, ink, 1.0), ink);
        // Order does not matter.
        assert_eq!(lerp(ink, Color::Black, 0.0), ink);
        assert_eq!(
            lerp(ink, Color::Black, 1.0),
            Color::Rgb { r: 0, g: 0, b: 0 }
        );
    }

    /// Every named colour blends with a truecolor, in both directions.
    ///
    /// `Color::Black` is the endpoint anyone actually reaches for, but the same
    /// trap applies to all sixteen, and `Color::White` is the second most likely.
    #[test]
    fn every_named_colour_blends_with_a_truecolor() {
        let ink = Color::Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        let named = [
            Color::Black,
            Color::DarkGrey,
            Color::Red,
            Color::Green,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Yellow,
            Color::Grey,
            Color::White,
        ];
        for color in named {
            for (a, b) in [(color, ink), (ink, color)] {
                let middle = lerp(a, b, 0.5);
                assert_ne!(
                    middle, a,
                    "{color:?} did not blend towards {ink:?} at the halfway point"
                );
                assert_ne!(
                    middle, b,
                    "{ink:?} did not blend towards {color:?} at the halfway point"
                );
            }
        }
    }

    /// `Reset` and `AnsiValue` genuinely have no answer, and must not be faked.
    ///
    /// `Reset` is "whatever the terminal's foreground is", which this code cannot
    /// know, and inventing a value for it would produce a colour the user did not
    /// ask for. Picking the nearer endpoint is the honest failure.
    #[test]
    fn unresolvable_endpoints_still_pick_an_endpoint() {
        let ink = Color::Rgb {
            r: 200,
            g: 100,
            b: 50,
        };
        for unresolvable in [Color::Reset, Color::AnsiValue(42)] {
            assert_eq!(lerp(unresolvable, ink, 0.0), unresolvable);
            assert_eq!(lerp(unresolvable, ink, 0.49), unresolvable);
            assert_eq!(lerp(unresolvable, ink, 0.5), ink);
            assert_eq!(lerp(unresolvable, ink, 1.0), ink);
        }
    }

    /// A ramp that starts at `Color::Black` has to actually ramp.
    ///
    /// This is the case the bug produced in the wild, expressed as a test on the
    /// type an effect would build rather than on the function directly.
    #[test]
    fn a_ramp_starting_at_black_is_monotonic() {
        let ramp = Palette::new(vec![Color::Black, ink_colour()]);
        assert_eq!(ramp.sample(0.0), Color::Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(ramp.sample(1.0), ink_colour());

        let mut previous = 0u8;
        for step in 1..=20 {
            let Color::Rgb { r, .. } = ramp.sample(step as f32 / 20.0) else {
                panic!("a two-stop black ramp stopped being truecolor");
            };
            assert!(r >= previous, "the ramp went backwards at step {step}");
            previous = r;
        }
    }

    fn ink_colour() -> Color {
        Color::Rgb {
            r: 200,
            g: 100,
            b: 50,
        }
    }

    #[test]
    fn from_rgb_clamps_and_rounds() {
        let palette = Palette::from_rgb(vec![[-1.0, 0.5, 2.0]]);
        assert_eq!(palette.stops()[0], rgb(0, 128, 255));
    }

    #[test]
    fn lerp_across_colour_kinds_picks_an_endpoint() {
        assert_eq!(lerp(Color::Reset, rgb(9, 9, 9), 0.2), Color::Reset);
        assert_eq!(lerp(Color::Reset, rgb(9, 9, 9), 0.8), rgb(9, 9, 9));
    }

    #[test]
    fn greyscale_runs_black_to_white() {
        let palette = greyscale();
        assert_eq!(palette.sample(0.0), rgb(0, 0, 0));
        assert_eq!(palette.sample(1.0), rgb(255, 255, 255));
        assert_eq!(palette.sample(0.5), rgb(128, 128, 128));
    }

    /// A brightness ramp's top of scale is its brightest colour.
    ///
    /// This is the test the four affected effects were missing, and it is the
    /// whole of the defect: `sample(1.0)` returns `stops[len - 1]`, which on a
    /// mirrored ramp is the second-darkest stop. Every preset here is mirrored
    /// except `CONTRAST`, so every one of them fails this before
    /// `truncated_at_peak` exists -- checked by running it against `palette`
    /// untruncated, which is what the second half of the loop does.
    #[test]
    fn a_brightness_ramp_puts_its_brightest_colour_at_the_top() {
        for (name, stops) in presets::ALL {
            let brightest = stops
                .iter()
                .copied()
                .max_by(|a, b| {
                    luminance(*a)
                        .partial_cmp(&luminance(*b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .expect("a preset with no stops");

            let truncated = Palette::new(stops.to_vec()).truncated_at_peak();
            assert_eq!(
                truncated.sample(1.0),
                brightest,
                "{name}: the truncated ramp does not end on its brightest stop"
            );

            // And the defect itself, asserted rather than assumed: an untruncated
            // mirror does not. If a preset ever stops being mirrored this line
            // starts failing, and that is the point -- it is the guard that says
            // "this preset no longer needs truncating", not a tautology.
            let untruncated = Palette::new(stops.to_vec());
            if stops.len() > 2 {
                assert_ne!(
                    untruncated.sample(1.0),
                    brightest,
                    "{name} is no longer mirrored, so truncated_at_peak is a \
                     no-op for it and the call sites should stop calling it"
                );
            }
        }
    }

    /// Truncation must not merely move the peak to the end -- the ramp has to
    /// still be *ordered*, or a value higher than another would still draw dimmer.
    ///
    /// Sampled across the whole truncated range rather than at the stops, because
    /// the interpolation between two stops can dip even when the stops are ordered.
    #[test]
    fn a_truncated_ramp_is_monotonic_in_luminance() {
        for (name, stops) in presets::ALL {
            let ramp = Palette::new(stops.to_vec()).truncated_at_peak();
            let mut previous = f32::NEG_INFINITY;
            for step in 0..=32 {
                let t = step as f32 / 32.0;
                let luma = luminance(ramp.sample(t));
                assert!(
                    luma >= previous - 1e-4,
                    "{name} went backwards in luminance at t = {t}: \
                     {luma} after {previous}"
                );
                previous = luma;
            }
        }
    }

    /// A peak found by channel maximum would be in the wrong place.
    ///
    /// `DEPTH`'s brightest channel is red at its last stop, `(204, 0, 0)`, which
    /// is a dark brick; its actual brightest stop is the near-white cyan at index
    /// 2. So a channel-max peak would truncate to five stops and leave the scale
    /// running bright-to-dark, which is the same defect under a new name.
    #[test]
    fn the_peak_is_found_by_luminance_and_not_by_channel_maximum() {
        let ramp = Palette::new(presets::DEPTH.to_vec()).truncated_at_peak();
        assert_eq!(ramp.len(), 3, "DEPTH peaks at index 2, so three stops");
        assert_eq!(ramp.sample(1.0), rgb(237, 255, 255));
        // The brick red is outside the truncated ramp entirely.
        assert!(!ramp.stops().contains(&rgb(204, 0, 0)));
    }

    /// A ramp whose brightest stop is its first has no ascending half.
    ///
    /// The honest answer is a flat ramp, and the requirement is that it is flat
    /// rather than inverted or empty. A user-supplied ramp reaches this, and
    /// `sample` has to answer without panicking.
    #[test]
    fn a_ramp_that_never_climbs_truncates_to_itself() {
        let flat = Palette::new(vec![rgb(10, 20, 30), rgb(9, 19, 29)]);
        let truncated = flat.truncated_at_peak();
        assert_eq!(truncated.len(), 1);
        assert_eq!(truncated.sample(0.0), rgb(10, 20, 30));
        assert_eq!(truncated.sample(1.0), rgb(10, 20, 30));
    }

    /// Blue is dark. This is the fact `truncated_at_peak` is built on.
    #[test]
    fn luminance_says_a_saturated_blue_is_dark() {
        let blue = rgb(0, 0, 255);
        let red = rgb(255, 0, 0);
        assert!(
            luminance(blue) < luminance(red),
            "a channel maximum would have called blue the brighter colour"
        );
        assert!((luminance(rgb(255, 255, 255)) - 1.0).abs() < 1e-6);
        assert!((luminance(rgb(0, 0, 0))).abs() < 1e-6);
        // Named colours resolve, so a ramp built from them is ordered too.
        assert!(luminance(Color::White) > luminance(Color::DarkGrey));
        // The unresolvable pair answers at the dark end rather than panicking.
        assert_eq!(luminance(Color::Reset), 0.0);
    }
}
