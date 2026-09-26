/// Interpolates between two colours. `t` is clamped, so a miscomputed ratio
/// saturates at an endpoint rather than extrapolating past it and wrapping.
fn lerp(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 * (1.0 - t) + b as f32 * t).round() as u8
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Builds a ramp of `length` colours: `start_color` at index 0, `middle_color`
/// at `middle_point`, and `end_color` at the final index.
///
/// The interpolation parameter has to be measured against the distance to the
/// *next* anchor, not against `middle_point` twice. Dividing the second half by
/// `middle_point` made `t` run past 1.0 — for the parameters the matrix effect
/// uses, `(length - middle_point) / middle_point` is exactly 2.0 — so the tail
/// extrapolated beyond `end_color` and saturated to black. Starting the loop at
/// 1 rather than 0 meant `t` never reached 0 either, so the first colour was
/// never emitted and the drop style that relies on a pure white head never got
/// one.
pub fn two_step_color_gradient(
    start_color: Color,
    middle_color: Color,
    end_color: Color,
    middle_point: usize,
    length: usize,
) -> Vec<Color> {
    if length == 0 {
        return Vec::new();
    }

    // A ramp of one colour cannot show a middle anchor, so use the end colour.
    let middle_point = middle_point.clamp(0, length - 1);
    let head_span = middle_point.max(1) as f32;
    let tail_span = (length - 1 - middle_point).max(1) as f32;

    let mut gradient = Vec::with_capacity(length);
    for i in 0..length {
        let (from, to, t) = if i <= middle_point {
            (start_color, middle_color, i as f32 / head_span)
        } else {
            (
                middle_color,
                end_color,
                (i - middle_point) as f32 / tail_span,
            )
        };

        gradient.push(Color {
            r: lerp(from.r, to.r, t),
            g: lerp(from.g, to.g, t),
            b: lerp(from.b, to.b, t),
        });
    }
    gradient
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    const RED: Color = Color { r: 255, g: 0, b: 0 };
    const GREEN: Color = Color { r: 0, g: 255, b: 0 };

    fn ramp(length: usize, middle: usize) -> Vec<Color> {
        two_step_color_gradient(BLACK, WHITE, RED, middle, length)
    }

    #[test]
    fn starts_exactly_at_the_start_colour() {
        // `t` has to reach 0 at index 0, otherwise the pure start colour is
        // never emitted and the drop style that wants a white head never gets
        // one.
        assert_eq!(ramp(12, 6)[0], BLACK);
    }

    #[test]
    fn ends_exactly_at_the_end_colour() {
        // The old denominator made `t` reach 2.0 here, extrapolating past the
        // end colour and saturating to black.
        assert_eq!(ramp(12, 6)[11], RED);
    }

    #[test]
    fn passes_through_the_middle_colour() {
        assert_eq!(ramp(12, 6)[6], WHITE);
    }

    #[test]
    fn every_channel_is_monotone_within_a_half() {
        let colours = ramp(12, 6);

        // Red rises across the whole ramp: black to white, then white to red.
        for pair in colours.windows(2) {
            assert!(
                pair[0].r <= pair[1].r,
                "red went backwards: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn no_colour_saturates_to_black() {
        // The extrapolation bug drove the tail of the ramp past the end colour
        // and then negative, which the cast to u8 turned into black. Built from
        // a start colour that is not itself black, so black can only mean the
        // bug.
        let colours = two_step_color_gradient(RED, WHITE, GREEN, 6, 12);
        for colour in &colours {
            assert_ne!(
                *colour,
                Color { r: 0, g: 0, b: 0 },
                "a ramp entry fell to black: {colours:?}"
            );
        }
    }

    #[test]
    fn handles_the_parameters_the_matrix_effect_actually_uses() {
        // DigitalRain builds its gradients with length = 3h/2 and middle = h/2,
        // which is the case where the old denominator reached exactly 2.0.
        for height in [6u16, 24, 50, 200] {
            let length = (3 * height / 2) as usize;
            let middle = (height / 2) as usize;
            let colours = ramp(length, middle);

            assert_eq!(colours.len(), length);
            assert_eq!(colours[0], BLACK, "height {height}");
            assert_eq!(colours[length - 1], RED, "height {height}");
        }
    }

    #[test]
    fn a_middle_point_past_the_end_does_not_panic() {
        let colours = ramp(4, 99);
        assert_eq!(colours.len(), 4);
        assert_eq!(colours[0], BLACK);
    }

    #[test]
    fn degenerate_lengths_do_not_panic() {
        assert!(two_step_color_gradient(BLACK, WHITE, RED, 0, 0).is_empty());
        assert_eq!(two_step_color_gradient(BLACK, WHITE, RED, 0, 1).len(), 1);
        // A middle anchor of zero puts the start colour at index 0 and ramps
        // straight to the end colour.
        let colours = two_step_color_gradient(BLACK, WHITE, RED, 0, 3);
        assert_eq!(colours[0], BLACK);
        assert_eq!(colours[2], RED);
    }

    #[test]
    fn interpolation_is_exact_at_the_midpoints() {
        let colours = two_step_color_gradient(BLACK, WHITE, GREEN, 4, 9);
        // Halfway from black to white is mid grey.
        assert_eq!(
            colours[2],
            Color {
                r: 128,
                g: 128,
                b: 128
            }
        );
        // Halfway from white to green.
        assert_eq!(
            colours[6],
            Color {
                r: 128,
                g: 255,
                b: 128
            }
        );
    }
}
