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
        // Mixing an indexed or reset colour with a truecolor one has no
        // meaningful answer, so the endpoints win rather than inventing one.
        _ => {
            if t < 0.5 {
                a
            } else {
                b
            }
        }
    }
}

#[inline]
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
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
}
