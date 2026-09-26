use crate::rain::gradient;
use crate::rain::rain_drop::RainDropStyle;
use crossterm::style;

/// Green at the head of a fading drop, and the floor its tail fades to.
///
/// Both ends are explicit because the old arithmetic was not. It computed
/// `255 - clamp(pos * step, 10, 256) as u8`, and `256u16 as u8` is **0**, not
/// 255: a float-to-int cast truncates rather than saturating. So from the cell
/// where the step first exceeded 255 the subtraction became `255 - 0 = 255`,
/// the *brightest* value in the ramp, and held it for the rest of the drop. On
/// a 50-row terminal `max_length` reaches `2h/3`, so roughly the bottom two
/// thirds of every `Front` drop and every `Gradient` drop from index 22 downward
/// was flat full-bright green instead of fading.
///
/// `TAIL_GREEN` is the value the old `clamp(10, ..)` lower bound was reaching
/// for, except that bound was applied to the amount *subtracted*, so it pinned
/// the dimmest green at 245 -- barely dimmer than the head. A drop that fades to
/// 245 has not faded at all.
const HEAD_GREEN: u8 = 255;
const TAIL_GREEN: u8 = 10;

/// The bright head, as a value in the ramp's own family.
///
/// A bright head is the effect, so it stays. Two things change:
///
/// * It is spring green rather than `style::Color::White`. The white head was
///   one of the sources of a screen full of white, and it was not a small one:
///   a drop is one head cell per twenty-odd of body, and a drop that never grows
///   past its initial length is *mostly* head. Spring green is still the
///   brightest thing on screen, and it does not read as a blown-out cell.
/// * It is a truecolor triple rather than one of crossterm's sixteen named
///   colours. A named colour encodes as an SGR index, which is the one shape
///   the output path has to treat differently from an RGB triple; the rest of
///   this ramp is truecolor, and the head has no reason to be the exception.
const HEAD: gradient::Color = gradient::Color {
    r: 120,
    g: 255,
    b: 120,
};

/// How quickly a drop's green falls off along its length.
#[derive(Clone, Copy)]
enum Fade {
    /// Evenly over the whole drop.
    Linear,
    /// Steeply at the head, so the bright part stays short and the long tail is
    /// dim. This is what `Front` drops used to compute, and it read as a solid
    /// green block because the quadratic ran out of range long before the tail
    /// and the truncating cast turned the rest back to full brightness.
    Quadratic,
}

/// Green channel at `pos` of a drop `len` cells long.
///
/// `pos` is measured against the drop's own length, not against a fixed step per
/// cell. A fixed step cannot be right at both 24 rows and 200, because
/// `max_length` grows with the height: at 12 per cell a 50-row terminal's drops
/// ran out of ramp after 20 cells and then sat at full brightness for the rest
/// of a body up to 33 cells long.
///
/// `pos` is clamped into the drop and the result is clamped into the green
/// range before it is cast, so neither an out-of-range index nor a rounding
/// overshoot can wrap or saturate. `len` of 0 or 1 is a drop with no tail to
/// fade, so it reads as the head.
fn fade_green(pos: usize, len: usize, fade: Fade) -> u8 {
    let last = len.saturating_sub(1);
    let t = if last == 0 {
        0.0
    } else {
        pos.min(last) as f32 / last as f32
    };
    let t = match fade {
        Fade::Linear => t,
        Fade::Quadratic => t * t,
    };

    let span = f32::from(HEAD_GREEN - TAIL_GREEN);
    (f32::from(HEAD_GREEN) - span * t)
        .round()
        .clamp(f32::from(TAIL_GREEN), f32::from(HEAD_GREEN)) as u8
}

/// The colour at `pos` of the back-drop ramp.
///
/// Clamped to the end of the ramp rather than indexed directly. A `Back` drop is
/// up to `2h/3` cells long and the ramp is built for the height the effect was
/// constructed at, so the two only line up while the terminal stays the size it
/// was at construction: after a grow, a drop created afterwards reaches past
/// the end of a ramp sized for the old height, which is an out-of-bounds panic
/// rather than a wrong colour.
///
/// Clamping is also the right *answer* for a longer drop, not just a safe one. A
/// fade has one direction; wrapping would run the ramp back to its bright head
/// halfway down a long drop, which is the same white-out the clamping cast
/// caused, reached by a different route.
fn ramp_at(ramp: &[gradient::Color], pos: usize) -> style::Color {
    if ramp.is_empty() {
        return style::Color::Reset;
    }
    let color = ramp[pos.min(ramp.len() - 1)];
    style::Color::Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn head_color() -> style::Color {
    style::Color::Rgb {
        r: HEAD.r,
        g: HEAD.g,
        b: HEAD.b,
    }
}

/// The attribute of cell `pos` of a drop.
///
/// Bold is a *head* effect, not a body effect. Many terminals treat bold as a
/// brightening hint, so a bold truecolor foreground is not reliably the colour
/// that was asked for -- which on a green ramp pushes the bright end toward
/// white. That is the same mechanism that made eleven of the sixteen effects look
/// washed out, and it is why only the first few cells of a drop are bold.
pub fn pick_style(vw_style: &RainDropStyle, pos: usize) -> style::Attribute {
    match vw_style {
        // `Front` only. `Middle` used to be in this group, and that was the
        // white flash: `Middle` and `Fading` are both `style::Color::DarkGrey`,
        // which crossterm encodes as `ESC[38;5;8m` -- a 256-colour palette index,
        // not a truecolor triple. Most terminals treat bold on a palette index as
        // a request for the bright variant, so the code asked for "bold dark
        // grey" and got white. `Fading` never flashed because it was never bold,
        // and `Middle` did, which is why it looked intermittent: `Middle` is 10%
        // of drops and the band travels, so it is a moving highlight.
        RainDropStyle::Front => match pos {
            0..HEAD_BOLD_CELLS => style::Attribute::Bold,
            _ => style::Attribute::NormalIntensity,
        },
        _ => style::Attribute::NormalIntensity,
    }
}

/// How many cells at the head of a drop are drawn bold.
const HEAD_BOLD_CELLS: usize = 4;

/// The colour of cell `pos` of a drop that is `len` cells long.
///
/// `len` is the drop's *visible* length. It is what makes the fade land at the
/// tail of a drop on any terminal height, rather than running out of ramp part
/// way down a long one and flatlining for the rest.
pub fn pick_color(
    vw_style: &RainDropStyle,
    pos: usize,
    len: usize,
    ramp: &[gradient::Color],
) -> style::Color {
    match vw_style {
        RainDropStyle::Gradient => {
            if pos == 0 {
                head_color()
            } else {
                style::Color::Rgb {
                    r: 0,
                    g: fade_green(pos, len, Fade::Linear),
                    b: 0,
                }
            }
        }
        RainDropStyle::Front => {
            if pos == 0 {
                head_color()
            } else {
                style::Color::Rgb {
                    r: 0,
                    g: fade_green(pos, len, Fade::Quadratic),
                    b: 0,
                }
            }
        }
        // A dim green, roughly half the brightness of a `Gradient` tail. `Middle`
        // used to fall through to the same `DarkGrey` as `Fading`, which made the
        // two indistinguishable and left the enum promising a three-layer
        // brightness structure that only two of the layers had.
        RainDropStyle::Middle => style::Color::Rgb {
            r: 0,
            g: (f32::from(fade_green(pos, len, Fade::Linear)) * 0.45) as u8,
            b: 0,
        },
        // The back layer gets the same spring-green head as the bright drops, so
        // every drop has a bright head and the layers differ in how far their
        // tails fall rather than in whether they have a head at all.
        RainDropStyle::Back => {
            if pos == 0 {
                head_color()
            } else {
                ramp_at(ramp, pos)
            }
        }
        _ => style::Color::DarkGrey,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rain::digital_rain::{DigitalRain, DigitalRainOptions};

    fn get_default_rain() -> DigitalRain {
        let rain_options = DigitalRainOptions {
            drops_range: (10, 20),
            speed_range: (2, 15),
            ..Default::default()
        };
        DigitalRain::new(rain_options, (30, 30))
    }

    fn green(color: style::Color) -> u8 {
        match color {
            style::Color::Rgb { g, .. } => g,
            other => panic!("expected a truecolor green, got {other:?}"),
        }
    }

    /// Bold belongs to the head of a drop, never its body.
    ///
    /// Many terminals treat bold as a brightening hint, so a bold truecolor
    /// foreground is not reliably the colour that was asked for. On a green ramp
    /// that pushes the bright end toward white, which is the same mechanism that
    /// made most of the catalogue look washed out. `Front` and `Back` used to be
    /// bold along their whole length.
    #[test]
    fn only_the_head_of_a_drop_is_bold() {
        for style in [
            RainDropStyle::Front,
            RainDropStyle::Middle,
            RainDropStyle::Back,
            RainDropStyle::Fading,
            RainDropStyle::Gradient,
        ] {
            for pos in (HEAD_BOLD_CELLS + 1)..40 {
                assert_ne!(
                    pick_style(&style, pos),
                    style::Attribute::Bold,
                    "{} is still bold at cell {pos} of its body, so the whole drop \
                     is brightened rather than just its head",
                    style_name(&style)
                );
            }
        }
    }

    /// The bright drops get a bold head; the dim ones stay dim throughout.
    ///
    /// `Back` is a *background* drop, layered behind a `Front`. Giving it a bold
    /// head would make it as bright as the one it is supposed to sit behind, so
    /// the layering is the thing being protected here.
    #[test]
    fn only_the_bright_drops_have_a_bold_head() {
        // `Middle` was in this list and is the white flash: it is
        // `style::Color::DarkGrey`, a 256-colour palette index, and bold on a
        // palette index is a brightening hint that most terminals answer by
        // rendering the bright variant. It is now a dim green of its own, so it
        // separates from the front drop by colour rather than by boldness.
        for style in [RainDropStyle::Front] {
            for pos in 0..HEAD_BOLD_CELLS {
                assert_eq!(
                    pick_style(&style, pos),
                    style::Attribute::Bold,
                    "{} should have a bright head, and cell {pos} is part of it",
                    style_name(&style)
                );
            }
        }

        for style in [
            RainDropStyle::Middle,
            RainDropStyle::Back,
            RainDropStyle::Fading,
            RainDropStyle::Gradient,
        ] {
            for pos in 0..40 {
                assert_eq!(
                    pick_style(&style, pos),
                    style::Attribute::NormalIntensity,
                    "{} is a dim drop but cell {pos} is bold, so it competes with \
                     the front drop it is meant to sit behind",
                    style_name(&style)
                );
            }
        }
    }

    /// The number of bold head cells has to be a small minority of a drop.
    ///
    /// A drop can be `2h/3` cells long, so on a tall terminal a generous head
    /// would still be a small share of the body. This is the check that catches
    /// the constant being raised rather than tuned.
    #[test]
    fn the_bold_head_is_a_small_share_of_the_longest_drop() {
        let tallest = 200usize;
        let longest_drop = tallest * 2 / 3;
        assert!(
            HEAD_BOLD_CELLS * 4 < longest_drop,
            "the bold head is {HEAD_BOLD_CELLS} cells of a drop that can be \
             {longest_drop} long, so most of the drop would be brightened"
        );
    }

    /// No cell may be bold on top of a 256-colour palette index.
    ///
    /// This is the white flash. `Middle` and `Fading` drops are drawn
    /// `style::Color::DarkGrey`, which crossterm encodes as `ESC[38;5;8m` -- a
    /// *palette index*, not a truecolor triple -- and `Middle` additionally drew its
    /// head cells `Attribute::Bold`.
    ///
    /// Most terminals treat bold on a palette index as a request for the bright
    /// variant of that index, so bold + index 8 arrives as light grey. The code
    /// asked for "bold dark grey" and the terminal delivered white. `Fading` did
    /// not flash because it was never bold, and `Middle` did, which is why it
    /// looked intermittent: `Middle` is 10% of drops, and the bold band is five
    /// cells travelling down at two to twenty cells a second, so it is a moving
    /// highlight rather than a static one.
    ///
    /// The rule, and the reason it is a rule rather than a preference: the
    /// encoder's whole job is to emit the colour that was asked for, and a
    /// terminal that overrides it makes the effect's output depend on the user's
    /// configuration. Plasma, fire, mandelbrot, maze and ink all had this same
    /// problem and all now use `Attribute::Reset`.
    #[test]
    fn no_cell_is_bold_on_top_of_a_palette_index() {
        let ramp = DigitalRain::build_ramp(50);
        for style in RainDropStyle::ALL {
            for pos in 0..40 {
                let attribute = pick_style(style, pos);
                if attribute == style::Attribute::Bold {
                    let color = pick_color(style, pos, 12, &ramp);
                    assert!(
                        matches!(color, style::Color::Rgb { .. }),
                        "{style:?} cell {pos} is bold on top of {color:?}, which is \
                         not a truecolor value. Bold on a 256-colour palette index \
                         is a brightening hint, and most terminals answer it by \
                         rendering the bright variant -- which is how a grey line \
                         flashes white."
                    );
                }
            }
        }
    }

    /// The back layer must not be pale at the top.
    ///
    /// `Back` drops are coloured by `ramp_at`, indexed by *absolute* position in
    /// the drop rather than by a fraction of it, and `build_ramp` started the ramp
    /// at `rgb(200, 200, 200)` -- a light grey at 78% luminance. So the first
    /// stretch of every back drop was near-white, and the stretch grew with the
    /// terminal: about 20 cells at 200 rows, against 2 at 50.
    ///
    /// That is a non-head cell rendering as near-white, which is the other half of
    /// "the gray lines flash white". A drop's head is allowed to be bright; its
    /// second cell is not.
    #[test]
    fn the_back_layer_is_green_rather_than_pale() {
        for height in [6u16, 24, 50, 100, 200, 400] {
            let ramp = DigitalRain::build_ramp(height);
            for pos in 1..12 {
                let style::Color::Rgb { r, g, b } =
                    pick_color(&RainDropStyle::Back, pos, 12, &ramp)
                else {
                    panic!("the back layer stopped being truecolor");
                };
                let luminance =
                    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
                assert!(
                    luminance < 0.25,
                    "at height {height}, back cell {pos} is rgb({r},{g},{b}) at \
                     luminance {luminance:.2}, which reads as pale rather than as \
                     a receding layer"
                );
                assert!(
                    g > r && g > b,
                    "at height {height}, back cell {pos} is rgb({r},{g},{b}), which \
                     is not green-dominant, so it does not read as part of the \
                     matrix"
                );
            }
        }
    }

    /// A bold band has to be the length it says it is.
    ///
    /// `HEAD_BOLD_CELLS` is 4 and the range was `0..=HEAD_BOLD_CELLS`, so the band
    /// was five cells. Neither bold test noticed, because both are written in terms
    /// of the constant rather than in cells.
    #[test]
    fn the_bold_band_is_exactly_as_long_as_it_says() {
        let bold_cells = (0..20)
            .filter(|pos| {
                pick_style(&RainDropStyle::Front, *pos) == style::Attribute::Bold
            })
            .count();
        assert_eq!(
            bold_cells, HEAD_BOLD_CELLS,
            "the constant says {HEAD_BOLD_CELLS} and the band is {bold_cells} cells"
        );
    }

    /// sRGB channel to relative luminance, per WCAG 2.x.
    fn linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn style_name(style: &RainDropStyle) -> &'static str {
        match style {
            RainDropStyle::Front => "Front",
            RainDropStyle::Middle => "Middle",
            RainDropStyle::Back => "Back",
            RainDropStyle::Fading => "Fading",
            RainDropStyle::Gradient => "Gradient",
        }
    }

    #[test]
    fn run_loop_10_iterations() {
        let mut stdout = Vec::new();
        let mut digital_rain = get_default_rain();
        let _ = crate::common::run_loop(&mut stdout, &mut digital_rain, Some(10));
    }

    /// The bug this file exists to not have again: a cast that truncates instead
    /// of saturating, so the tail of the ramp comes back brighter than the
    /// middle of it.
    #[test]
    fn a_drop_tail_fades_all_the_way_down_instead_of_flatlining_bright() {
        // Long enough to cover every drop a 50-row terminal can build: `2h/3`.
        for len in [2usize, 3, 8, 20, 33] {
            for style in [RainDropStyle::Gradient, RainDropStyle::Front] {
                let tail: Vec<u8> = (1..len)
                    .map(|pos| green(pick_color(&style, pos, len, &[])))
                    .collect();

                for pair in tail.windows(2) {
                    assert!(
                        pair[0] > pair[1],
                        "{style:?} at len {len}: green rose from {} to {} part way \
                         down the drop ({tail:?})",
                        pair[0],
                        pair[1],
                    );
                }

                // The defect in one assertion: the old arithmetic clamped the
                // subtracted amount at 256, and `256u16 as u8` is 0, so
                // `255 - 0` pinned the whole tail of a long drop at full
                // brightness. Before the fix this is 255.
                assert!(
                    *tail.last().unwrap() < 64,
                    "{style:?} at len {len} ends on green {} rather than fading out \
                     ({tail:?})",
                    tail.last().unwrap(),
                );
            }
        }
    }

    #[test]
    fn the_bright_head_is_the_brightest_cell_of_its_drop() {
        for len in [2usize, 3, 5, 17, 33] {
            for style in [RainDropStyle::Gradient, RainDropStyle::Front] {
                let head = pick_color(&style, 0, len, &[]);
                let tail = pick_color(&style, len - 1, len, &[]);

                // The head is the maximum, not necessarily *strictly* above
                // every cell: a `Front` drop's quadratic is flat across its first
                // couple of cells by design, so that the bright part stays short.
                for pos in 1..len {
                    let g = pick_color(&style, pos, len, &[]);
                    assert!(
                        green(g) <= green(head),
                        "{style:?} at len {len}: cell {pos} (green {}) is brighter \
                         than the head ({})",
                        green(g),
                        green(head),
                    );
                }
                assert!(
                    green(head) > green(tail),
                    "{style:?} at len {len}: the head ({}) is no brighter than the \
                     tail ({})",
                    green(head),
                    green(tail),
                );
            }
        }
    }

    #[test]
    fn the_head_is_bright_without_being_white() {
        // Kept deliberately, and checked deliberately: a bright head is the
        // effect, but `Color::White` is one of the sources of the white-out, and
        // it is a named ANSI colour rather than a truecolor triple like the
        // rest of the ramp.
        for style in [
            RainDropStyle::Gradient,
            RainDropStyle::Front,
            RainDropStyle::Back,
        ] {
            let head =
                pick_color(&style, 0, 8, &[gradient::Color { r: 9, g: 9, b: 9 }]);
            assert_ne!(
                head,
                style::Color::White,
                "{style:?} head is still plain white"
            );
            assert!(
                matches!(head, style::Color::Rgb { .. }),
                "{style:?} head is not truecolor: {head:?}"
            );
        }
    }

    #[test]
    fn a_ramp_index_past_the_end_of_the_table_stays_in_range() {
        let ramp = gradient::two_step_color_gradient(
            gradient::Color {
                r: 200,
                g: 200,
                b: 200,
            },
            gradient::Color { r: 0, g: 200, b: 0 },
            gradient::Color {
                r: 10,
                g: 10,
                b: 10,
            },
            4,
            12,
        );
        let tail = ramp[ramp.len() - 1];

        for pos in [ramp.len(), ramp.len() + 1, 1_000, 1_000_000, usize::MAX] {
            let got = pick_color(&RainDropStyle::Back, pos, 1, &ramp);
            assert_eq!(
                got,
                style::Color::Rgb {
                    r: tail.r,
                    g: tail.g,
                    b: tail.b,
                },
                "ramp index {pos} did not clamp to the end of the table"
            );
        }

        // And explicitly not full brightness, which is what a clamped-then-
        // truncated cast produced.
        assert!(green(pick_color(&RainDropStyle::Back, 1_000_000, 1, &ramp)) < 64);
    }

    #[test]
    fn an_empty_ramp_reads_as_the_terminal_default_rather_than_panicking() {
        // Reachable: `two_step_color_gradient` returns nothing for a zero
        // length, and a drop is drawn before the effect knows the height.
        // Cell 0 is the head, which is a fixed colour and never consults the
        // ramp. Every other cell does, and has to fall back rather than index an
        // empty slice.
        assert_eq!(pick_color(&RainDropStyle::Back, 0, 4, &[]), head_color());
        for pos in [1usize, 2, 999] {
            assert_eq!(
                pick_color(&RainDropStyle::Back, pos, 4, &[]),
                style::Color::Reset,
                "cell {pos} of a back drop with no ramp did not fall back"
            );
        }
    }

    #[test]
    fn a_drop_with_no_visible_length_still_produces_a_colour() {
        // `len` is derived from a visible point list, which is empty for a drop
        // that has not entered the screen. Nothing here may divide by it.
        for style in [
            RainDropStyle::Gradient,
            RainDropStyle::Front,
            RainDropStyle::Back,
        ] {
            for pos in 0..4 {
                let _ = pick_color(&style, pos, 0, &[]);
            }
        }
    }

    /* // NOTE: this test failed on github CI pipeline
    #[test]
    fn run_loop_fps_gte_0() {
        let mut stdout = Vec::new();
        let mut digital_rain = get_default_rain();
        // let mut fps: f64 = 0.0;
        let fps = crate::common::run_loop(&mut stdout, &mut digital_rain, Some(10))
            .unwrap();

        /*
        for _ in 0..10 {
            let fps_res =
                crate::common::run_loop(&mut stdout, &mut digital_rain, Some(10));
            if let Ok(f) = fps_res {
                fps = f;
                break;
            }
        }
        */
        assert_eq!(fps > 0.0, true);
    } */
}
