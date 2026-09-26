use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

static PALETTE: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let mut vec = Vec::with_capacity(768);
    vec.extend_from_slice(&[
        0, 0, 0, 0, 0, 6, 0, 0, 6, 0, 0, 7, 0, 0, 8, 0, 0, 8, 0, 0, 9, 0, 0, 10, 2,
        0, 10, 4, 0, 9, 6, 0, 9, 8, 0, 8, 10, 0, 7, 12, 0, 7, 14, 0, 6, 16, 0, 5,
        18, 0, 5, 20, 0, 4, 22, 0, 4, 24, 0, 3, 26, 0, 2, 28, 0, 2, 30, 0, 1, 32,
        0, 0, 32, 0, 0, 33, 0, 0, 34, 0, 0, 35, 0, 0, 36, 0, 0, 36, 0, 0, 37, 0, 0,
        38, 0, 0, 39, 0, 0, 40, 0, 0, 40, 0, 0, 41, 0, 0, 42, 0, 0, 43, 0, 0, 44,
        0, 0, 45, 0, 0, 46, 1, 0, 47, 1, 0, 48, 2, 0, 49, 2, 0, 50, 3, 0, 51, 3, 0,
        52, 4, 0, 53, 4, 0, 54, 5, 0, 55, 5, 0, 56, 6, 0, 57, 6, 0, 58, 7, 0, 59,
        7, 0, 60, 8, 0, 61, 8, 0, 63, 9, 0, 63, 9, 0, 63, 10, 0, 63, 10, 0, 63, 11,
        0, 63, 11, 0, 63, 12, 0, 63, 12, 0, 63, 13, 0, 63, 13, 0, 63, 14, 0, 63,
        14, 0, 63, 15, 0, 63, 15, 0, 63, 16, 0, 63, 16, 0, 63, 17, 0, 63, 17, 0,
        63, 18, 0, 63, 18, 0, 63, 19, 0, 63, 19, 0, 63, 20, 0, 63, 20, 0, 63, 21,
        0, 63, 21, 0, 63, 22, 0, 63, 22, 0, 63, 23, 0, 63, 24, 0, 63, 24, 0, 63,
        25, 0, 63, 25, 0, 63, 26, 0, 63, 26, 0, 63, 27, 0, 63, 27, 0, 63, 28, 0,
        63, 28, 0, 63, 29, 0, 63, 29, 0, 63, 30, 0, 63, 30, 0, 63, 31, 0, 63, 31,
        0, 63, 32, 0, 63, 32, 0, 63, 33, 0, 63, 33, 0, 63, 34, 0, 63, 34, 0, 63,
        35, 0, 63, 35, 0, 63, 36, 0, 63, 36, 0, 63, 37, 0, 63, 38, 0, 63, 38, 0,
        63, 39, 0, 63, 39, 0, 63, 40, 0, 63, 40, 0, 63, 41, 0, 63, 41, 0, 63, 42,
        0, 63, 42, 0, 63, 43, 0, 63, 43, 0, 63, 44, 0, 63, 44, 0, 63, 45, 0, 63,
        45, 0, 63, 46, 0, 63, 46, 0, 63, 47, 0, 63, 47, 0, 63, 48, 0, 63, 48, 0,
        63, 49, 0, 63, 49, 0, 63, 50, 0, 63, 50, 0, 63, 51, 0, 63, 52, 0, 63, 52,
        0, 63, 52, 0, 63, 52, 0, 63, 52, 0, 63, 53, 0, 63, 53, 0, 63, 53, 0, 63,
        53, 0, 63, 54, 0, 63, 54, 0, 63, 54, 0, 63, 54, 0, 63, 54, 0, 63, 55, 0,
        63, 55, 0, 63, 55, 0, 63, 55, 0, 63, 56, 0, 63, 56, 0, 63, 56, 0, 63, 56,
        0, 63, 57, 0, 63, 57, 0, 63, 57, 0, 63, 57, 0, 63, 57, 0, 63, 58, 0, 63,
        58, 0, 63, 58, 0, 63, 58, 0, 63, 59, 0, 63, 59, 0, 63, 59, 0, 63, 59, 0,
        63, 60, 0, 63, 60, 0, 63, 60, 0, 63, 60, 0, 63, 60, 0, 63, 61, 0, 63, 61,
        0, 63, 61, 0, 63, 61, 0, 63, 62, 0, 63, 62, 0, 63, 62, 0, 63, 62, 0, 63,
        63, 0, 63, 63, 1, 63, 63, 2, 63, 63, 3, 63, 63, 4, 63, 63, 5, 63, 63, 6,
        63, 63, 7, 63, 63, 8, 63, 63, 9, 63, 63, 10, 63, 63, 10, 63, 63, 11, 63,
        63, 12, 63, 63, 13, 63, 63, 14, 63, 63, 15, 63, 63, 16, 63, 63, 17, 63, 63,
        18, 63, 63, 19, 63, 63, 20, 63, 63, 21, 63, 63, 21, 63, 63, 22, 63, 63, 23,
        63, 63, 24, 63, 63, 25, 63, 63, 26, 63, 63, 27, 63, 63, 28, 63, 63, 29, 63,
        63, 30, 63, 63, 31, 63, 63, 31, 63, 63, 32, 63, 63, 33, 63, 63, 34, 63, 63,
        35, 63, 63, 36, 63, 63, 37, 63, 63, 38, 63, 63, 39, 63, 63, 40, 63, 63, 41,
        63, 63, 42, 63, 63, 42, 63, 63, 43, 63, 63, 44, 63, 63, 45, 63, 63, 46, 63,
        63, 47, 63, 63, 48, 63, 63, 49, 63, 63, 50, 63, 63, 51, 63, 63, 52, 63, 63,
        52, 63, 63, 53, 63, 63, 54, 63, 63, 55, 63, 63, 56, 63, 63, 57, 63, 63, 58,
        63, 63, 59, 63, 63, 60, 63, 63, 61, 63, 63, 62, 63, 63, 63,
    ]);
    vec
});
const MAXTABLE: usize = 256 * 5;

/// Scale that takes the table's 0..63 range to 0..255.
const PALETTE_GAIN: f32 = 4.047619;

/// Highest intensity the palette is used at.
///
/// The table's top entries walk out of yellow and into white, ending at
/// `(63, 63, 63)`, which `PALETTE_GAIN` turns into `(254, 254, 254)`. Every cell
/// above intensity 187 was therefore drawn as a near-white block -- and the core
/// of a large flame is almost entirely above 187, so the brightest part of the
/// effect was a slab of white rather than a hot colour.
///
/// 187 is the last table entry with no blue channel at all, i.e. pure yellow,
/// and yellow is what the hottest part of a flame actually looks like. Capping
/// the index there rather than editing the table keeps the table honest: the
/// white is still in it, the effect just never reaches it.
const HOTTEST: usize = 187;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FireOptions {
    pub use_colors: bool,
    /// Seed for the combustion noise that seeds new fires along the bottom row.
    pub seed: u64,
}

impl Default for FireOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            use_colors: true,
            seed: DEFAULT_SEED,
        }
    }
}

/// One combustion iteration is modelled as taking a fixed slice of wall clock.
/// Real elapsed time is accumulated and spent in whole quanta, so the fire
/// burns at the same rate on any refresh rate and honours the speed keys.
const STEP_QUANTUM: f32 = 1.0 / 60.0;
/// Upper bound on catch-up work per frame, so a long stall cannot lock up.
const MAX_STEPS_PER_FRAME: usize = 4;

pub struct Fire {
    pub screen_size: (u16, u16),
    options: FireOptions,
    canvas: Canvas,
    color_palette: Vec<style::Color>,
    fire_bitmap: Vec<u8>,     // fire bitmap
    intensity_table: Vec<u8>, // Fire intensity lookup table
    loop_counter: i32,        // Animation loop counter
    sloop_counter: i32,       // Secondary loop counter
    height_counter: u32,      // Height counter for fire growth
    step_accumulator: f32,    // Elapsed time not yet spent on a combustion step
    rng: EffectRng,
}

impl TerminalEffect for Fire {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.draw_fire();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(STEP_QUANTUM as f64);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.advance(context.delta.as_secs_f64());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.reset();
    }

    fn reset(&mut self) {
        let width = self.screen_size.0 as usize;
        let height = self.screen_size.1 as usize;

        // A resize invalidates the frame on screen, so the canvas blanks its
        // baseline and the next commit repaints in full.
        self.canvas.resize(width as u16, height as u16);

        // Initialize fire bitmap (double the width and height for better resolution)
        let bitmap_width = width * 2;
        let bitmap_height = height * 2;
        self.fire_bitmap = vec![0; bitmap_width * bitmap_height];

        // Generate the intensity table
        self.generate_intensity_table();

        // Reseeded with the rest of the state this function already rebuilds,
        // so a resize restarts the combustion noise rather than continuing the
        // old sequence against a blank bitmap.
        self.rng = seeded_rng(self.options.seed, "fire");

        // Reset animation counters
        self.loop_counter = 0;
        self.sloop_counter = 0;
        self.height_counter = 0;
    }
}

impl Fire {
    pub fn new(options: FireOptions, screen_size: (u16, u16)) -> Self {
        let width = screen_size.0 as usize;
        let height = screen_size.1 as usize;
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        // Create color palette from RGB values
        let mut color_palette = Vec::with_capacity(256);
        for i in 0..256 {
            let r = (PALETTE[i * 3] as f32 * PALETTE_GAIN) as u8;
            let g = (PALETTE[i * 3 + 1] as f32 * PALETTE_GAIN) as u8;
            let b = (PALETTE[i * 3 + 2] as f32 * PALETTE_GAIN) as u8;
            color_palette.push(style::Color::Rgb { r, g, b });
        }

        // Initialize fire bitmap (double the width and height)
        let bitmap_width = width * 2;
        let bitmap_height = height * 2;
        let fire_bitmap = vec![0; bitmap_width * bitmap_height];

        // Initialize intensity table
        let intensity_table = vec![0; MAXTABLE];

        let mut fire = Fire {
            screen_size,
            rng: seeded_rng(options.seed, "fire"),
            options,
            canvas,
            color_palette,
            fire_bitmap,
            intensity_table,
            loop_counter: 0,
            sloop_counter: 0,
            height_counter: 0,
            step_accumulator: 0.0,
        };

        // Generate the intensity table
        fire.generate_intensity_table();

        fire
    }

    /// Spends elapsed time on whole combustion steps.
    fn advance(&mut self, delta: f64) {
        self.step_accumulator += delta as f32;
        let mut steps = 0;
        while self.step_accumulator >= STEP_QUANTUM && steps < MAX_STEPS_PER_FRAME {
            self.step_accumulator -= STEP_QUANTUM;
            self.combust();
            steps += 1;
        }
        if steps == MAX_STEPS_PER_FRAME {
            self.step_accumulator = 0.0;
        }
    }

    /// A single combustion iteration.
    fn combust(&mut self) {
        self.height_counter += 1;
        self.loop_counter -= 1;

        if self.loop_counter < 0 {
            self.loop_counter = self.rng.random_range(0..3);
            self.sloop_counter += 1;
        }

        self.generate_fire_base();
        self.propagate_fire();
    }

    fn generate_intensity_table(&mut self) {
        let height = self.screen_size.1 as usize * 2;
        let minus = if height > 0 { 800 / height } else { 1 };

        for i in 0..MAXTABLE {
            if i > minus {
                let p2 = (i - minus) / 5;
                self.intensity_table[i] = p2 as u8;
            } else {
                self.intensity_table[i] = 0;
            }
        }
    }

    fn propagate_fire(&mut self) {
        let width = self.screen_size.0 as usize * 2;
        let height = self.screen_size.1 as usize * 2;

        // Skip last row since bitmap has look-ahead
        for y in 0..height - 1 {
            for x in 1..width - 1 {
                let idx = y * width + x;

                if idx >= self.fire_bitmap.len()
                    || idx + width >= self.fire_bitmap.len()
                {
                    continue;
                }

                // Calculate the new value based on surrounding pixels
                let sum = self.fire_bitmap[idx + width - 1] as usize
                    + self.fire_bitmap[idx + width + 1] as usize
                    + self.fire_bitmap[idx + width] as usize;

                // Add pixels from two rows below
                let two_rows = idx + 2 * width;
                let additional = if two_rows < self.fire_bitmap.len() {
                    self.fire_bitmap[two_rows - 1] as usize
                        + self.fire_bitmap[two_rows + 1] as usize
                } else {
                    0
                };

                let total = sum + additional;
                if total < MAXTABLE {
                    self.fire_bitmap[idx] = self.intensity_table[total];
                }
            }
        }
    }

    fn generate_fire_base(&mut self) {
        // Reuse the effect's own generator rather than constructing a fresh
        // thread-local one on every combustion step.
        let rng = &mut self.rng;
        let width = self.screen_size.0 as usize * 2;
        let height = self.screen_size.1 as usize * 2;

        // Start with base row
        let base_row = height - 1;
        let mut x = 0;

        while x < width {
            let i1 = 1 + x * 4;
            let i2 = 4 * width + 1 - x * 4;
            let min_val = i1.min(i2).min(self.height_counter as usize);
            let last1 = rng.random_range(0..min_val) as u8;

            let count = rng.random_range(0..6);

            for i in 0..count {
                if x + i >= width {
                    break;
                }

                // Set current pixel
                let idx = base_row * width + x + i;
                if idx < self.fire_bitmap.len() {
                    self.fire_bitmap[idx] = last1;
                }

                // Set pixel one row up with slightly varied intensity
                let one_up = (base_row - 1) * width + x + i;
                if one_up < self.fire_bitmap.len() {
                    let intensity = last1 as i16 + rng.random_range(-2..4) as i16;
                    self.fire_bitmap[one_up] = intensity.clamp(0, 255) as u8;
                }

                // Set pixel two rows up with more varied intensity
                let two_up = (base_row - 2) * width + x + i;
                if two_up < self.fire_bitmap.len() {
                    let intensity = last1 as i16 + rng.random_range(-2..4) as i16;
                    self.fire_bitmap[two_up] = intensity.clamp(0, 255) as u8;
                }
            }

            x += count;
        }
    }

    /// Maps the fire bitmap onto the canvas, one cell per 2x2 bitmap block.
    fn draw_fire(&mut self) {
        self.canvas.clear();

        // Map fire bitmap to terminal cells
        for y in 0..self.screen_size.1 as usize {
            for x in 0..self.screen_size.0 as usize {
                // Sample from the fire bitmap
                let fire_x = x * 2;
                let fire_y = y * 2;
                let width = self.screen_size.0 as usize * 2;

                if fire_y * width + fire_x < self.fire_bitmap.len() {
                    let intensity =
                        self.fire_bitmap[fire_y * width + fire_x] as usize;

                    // Choose character based on intensity
                    let character = match intensity {
                        0 => ' ',
                        1..=20 => '.',
                        21..=40 => ':',
                        41..=60 => '*',
                        61..=80 => 'o',
                        81..=100 => 'O',
                        101..=120 => '#',
                        121..=140 => '@',
                        _ => '%',
                    };

                    // A blank is a blank, in either colour mode. The
                    // `use_colors = false` branch used to hand back
                    // `Color::White` for these as well as for the glyphs, so the
                    // cold part of the frame was a field of white spaces: a slab
                    // of white on a light profile and nothing at all on a dark
                    // one, which is the opposite of the plain ASCII fire the
                    // option is meant to give.
                    let cell = if character == ' ' {
                        Cell::default()
                    } else {
                        // `Color::Reset` rather than white when colour is off:
                        // "no colours" means the terminal's own foreground, not
                        // the one that happens to be in the palette.
                        let fg_color = if self.options.use_colors
                            && intensity < self.color_palette.len()
                        {
                            self.color_palette[intensity.min(HOTTEST)]
                        } else {
                            style::Color::Reset
                        };

                        // `Attribute::Reset`, not `Attribute::Bold`. Every cell
                        // used to be bold, and bold on a truecolor foreground is
                        // a brightening hint that many terminals act on, which
                        // pushes the hot end of a ramp toward white.
                        Cell::new(character, fg_color, style::Attribute::Reset)
                    };

                    self.canvas.set(x, y, cell);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hottest_colour_the_effect_can_draw_is_yellow_not_white() {
        let fire = Fire::new(FireOptions::default(), (20, 10));

        // Scanned over the range the effect indexes, not the whole table: the
        // table's top entries are white by design and the cap is what stops the
        // effect reaching them. If the cap goes, this fails.
        let reachable = fire.color_palette.len().min(HOTTEST + 1);
        for (index, color) in fire.color_palette.iter().take(reachable).enumerate()
        {
            let style::Color::Rgb { r, g, b } = *color else {
                panic!("palette entry {index} is not truecolor: {color:?}");
            };
            assert!(
                r.min(g).min(b) < 200,
                "palette entry {index} is ({r}, {g}, {b}): close enough to white \
                 that the core of a flame reads as a white block"
            );
        }

        // And the cap is at the top of the table's yellow, not merely somewhere
        // short of the white.
        let style::Color::Rgb { r, g, b } = fire.color_palette[HOTTEST] else {
            unreachable!("the palette is built as truecolor");
        };
        assert!(
            r > 200 && g > 200 && b < 32,
            "the hottest entry is ({r}, {g}, {b}), which is not hot yellow"
        );
    }

    #[test]
    fn a_settled_fire_never_puts_a_near_white_or_a_bold_cell_on_screen() {
        let mut fire = Fire::new(FireOptions::default(), (40, 12));
        for _ in 0..180 {
            fire.update();
            fire.get_diff();
        }

        let mut hottest: Option<u8> = None;
        for cell in fire.canvas.on_screen().buffer.iter() {
            assert_eq!(
                cell.attr,
                style::Attribute::Reset,
                "cell {:?} is still bold, which brightens the hot end of the ramp",
                cell.symbol
            );
            let style::Color::Rgb { r, g, b } = cell.color else {
                continue;
            };
            let lo = r.min(g).min(b);
            if hottest.is_none_or(|best| lo > best) {
                hottest = Some(lo);
            }
        }

        // The brightest thing in the effect is a hot yellow, not a white block.
        // Before the cap the core was `(254, 254, 254)` and this is 254.
        let hottest = hottest.expect("nothing was drawn at all");
        assert!(
            hottest < 64,
            "the brightest cell on screen has every channel at {hottest} or above, \
             which is white"
        );
    }

    #[test]
    fn without_colours_a_dead_cell_is_a_blank_rather_than_a_white_space() {
        let mut fire = Fire::new(
            FireOptions {
                use_colors: false,
                seed: 7,
            },
            (40, 12),
        );
        for _ in 0..120 {
            fire.update();
            fire.get_diff();
        }

        let blanks: Vec<Cell> = fire
            .canvas
            .on_screen()
            .buffer
            .iter()
            .copied()
            .filter(|cell| cell.symbol == ' ')
            .collect();

        assert!(
            !blanks.is_empty(),
            "the fixture produced no dead cells, so there was nothing to check"
        );
        for cell in blanks {
            assert_ne!(
                cell.color,
                style::Color::White,
                "a dead cell was painted as a white space"
            );
            assert_eq!(
                cell,
                Cell::default(),
                "a dead cell is not the blank the rest of the crate clears to"
            );
        }
    }
}
