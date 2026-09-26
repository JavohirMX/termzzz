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
