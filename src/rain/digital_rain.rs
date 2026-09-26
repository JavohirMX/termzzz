use super::draw::{pick_color, pick_style};
use super::gradient;
use super::rain_drop::RainDrop;
use crate::buffer::{Buffer, Cell};
use crate::common::{DefaultOptions, TerminalEffect};

use rand::{self, RngExt};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct DigitalRainOptions {
    /// Derived from the terminal size by the config layer, so not persisted.
    #[serde(skip)]
    pub drops_range: (u16, u16),
    /// Derived from the terminal size by the config layer, so not persisted.
    #[serde(skip)]
    pub speed_range: (u16, u16),
    pub drops_coeff: f32,
    pub speed_coeff: f32,
}

impl Default for DigitalRainOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file that
    /// omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            drops_range: (10, 20),
            speed_range: (2, 16),
            drops_coeff: 1.0,
            speed_coeff: 1.0,
        }
    }
}

pub struct DigitalRain {
    pub screen_size: (u16, u16),
    options: DigitalRainOptions,
    gradients: Vec<Vec<gradient::Color>>,
    rain_drops: Vec<RainDrop>,
    buffer: Buffer,
    rng: rand::prelude::ThreadRng,
}

impl TerminalEffect for DigitalRain {
    /// Calculate difference between current frame and previous frame
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let mut curr_buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);

        // fill current buffer
        // first draw drops with bigger fy
        Self::fill_buffer(&mut self.rain_drops, &mut curr_buffer, &self.gradients);

        let diff = self.buffer.diff(&curr_buffer);
        self.buffer = curr_buffer;
        diff
    }

    /// Update each rain drop position
    fn update(&mut self) {
        self.update_rain(Duration::from_secs_f64(1.0 / 60.0));
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.update_rain(context.delta);
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        let area = self.screen_size.0 as f32 * self.screen_size.1 as f32;
        self.options.drops_range = (
            ((area / 160.0 * self.options.drops_coeff) as u16).max(10),
            ((area / 80.0 * self.options.drops_coeff) as u16).max(20),
        );
        self.options.speed_range = (
            ((self.screen_size.1 as f32 / 20.0 * self.options.speed_coeff) as u16)
                .max(2),
            ((self.screen_size.1 as f32 / 10.0 * self.options.speed_coeff) as u16)
                .max(16),
        );
    }

    fn reset(&mut self) {
        let new_effect = DigitalRain::new(self.options.clone(), self.screen_size);
        *self = new_effect;
    }
}

/// Process digital rain effect.
/// Note that all processing done implying coordinates started from 0, 0
/// and width / height is actual number of columns and rows
impl DigitalRain {
    // Initialize screensaver
    pub fn new(options: DigitalRainOptions, screen_size: (u16, u16)) -> Self {
        let mut rng = rand::rng();
        let mut rain_drops: Vec<RainDrop> = vec![];
        let mut buffer: Buffer =
            Buffer::new(screen_size.0 as usize, screen_size.1 as usize);
        for rain_drop_id in 1..=options.get_min_drops_number() {
            rain_drops.push(RainDrop::new(
                screen_size,
                &options,
                rain_drop_id as usize,
                &mut rng,
            ));
        }

        // fill gradients
        let gradients = vec![
            gradient::two_step_color_gradient(
                gradient::Color {
                    r: 255,
                    g: 255,
                    b: 255,
                },
                gradient::Color { r: 0, g: 255, b: 0 },
                gradient::Color {
                    r: 10,
                    g: 10,
                    b: 10,
                },
                4,
                3 * screen_size.1 as usize / 2,
            ),
            gradient::two_step_color_gradient(
                gradient::Color {
                    r: 200,
                    g: 200,
                    b: 200,
                },
                gradient::Color { r: 0, g: 250, b: 0 },
                gradient::Color {
                    r: 10,
                    g: 10,
                    b: 10,
                },
                6,
                3 * screen_size.1 as usize / 2,
            ),
            gradient::two_step_color_gradient(
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
                screen_size.1 as usize / 2,
                3 * screen_size.1 as usize / 2,
            ),
        ];

        Self::fill_buffer(&mut rain_drops, &mut buffer, &gradients);

        Self {
            screen_size,
            options,
            gradients,
            rain_drops,
            buffer,
            rng,
        }
    }

    fn update_rain(&mut self, delta: Duration) {
        for rain_drop in self.rain_drops.iter_mut() {
            rain_drop.update(self.screen_size, &self.options, delta, &mut self.rng);
        }
        self.add_one();
    }

    pub fn fill_buffer(
        rain_drops: &mut [RainDrop],
        buffer: &mut Buffer,
        gradients: &[Vec<gradient::Color>],
    ) {
        rain_drops.sort_by(|a, b| a.speed.partial_cmp(&b.speed).unwrap());
        for rain_drop in rain_drops.iter().rev() {
            let points = rain_drop.to_points_vec();
            for (index, (x, y, character)) in points.iter().enumerate() {
                let (width, height) = buffer.get_size();
                if *x < width as u16 && *y < height as u16 {
                    buffer.set(
                        *x as usize,
                        *y as usize,
                        Cell::new(
                            *character,
                            pick_color(&rain_drop.style, index, gradients),
                            pick_style(&rain_drop.style, index),
                        ),
                    );
                };
            }
        }
    }

    /// Add one more worm with decent chance
    pub fn add_one(&mut self) {
        if self.rain_drops.len() >= self.options.get_max_drops_number() as usize {
            return;
        };
        let mut rng = rand::rng();
        if rng.random_range(0.0..=1.0) <= 0.3 {
            self.rain_drops.push(RainDrop::new(
                self.screen_size,
                &self.options,
                self.rain_drops.len() + 1,
                &mut rng,
            ));
        };
    }
}

impl DigitalRainOptions {
    #[inline]
    pub fn get_min_drops_number(&self) -> u16 {
        self.drops_range.0
    }

    #[inline]
    pub fn get_max_drops_number(&self) -> u16 {
        self.drops_range.1
    }

    #[inline]
    pub fn get_min_speed(&self) -> u16 {
        self.speed_range.0
    }

    #[inline]
    pub fn get_max_speed(&self) -> u16 {
        self.speed_range.1
    }
}

impl DefaultOptions for DigitalRain {
    type Options = DigitalRainOptions;

    fn default_options(width: u16, height: u16) -> Self::Options {
        // Widened before multiplying. `width * height` as `u16` wraps on any
        // terminal past 256x256, which silently produced a small drop count on
        // exactly the large screens that need the most of them.
        let area = width as f32 * height as f32;

        let drops_range = {
            // Approximately 0.6% to 1.2% of screen space.
            let min_drops = (area / 160.0) as u16;
            let max_drops = (area / 80.0) as u16;
            (min_drops.max(10), max_drops.max(20)) // Ensure minimum values
        };

        let speed_range = {
            let min_speed = (height / 20).max(2); // Faster for larger screens
            let max_speed = (height / 10).max(16); // But not too fast
            (min_speed, max_speed)
        };

        DigitalRainOptions {
            drops_range,
            speed_range,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_sane_default_options() -> DigitalRainOptions {
        DigitalRainOptions {
            drops_range: (20, 30),
            speed_range: (10, 20),
            ..Default::default()
        }
    }

    #[test]
    fn create_new() {
        let foo = DigitalRain::new(get_sane_default_options(), (100, 100));
        assert_eq!(foo.rain_drops.len(), 20);
    }

    #[test]
    fn resize_recomputes_drop_and_speed_ranges() {
        let options = DigitalRainOptions {
            drops_range: (50, 60),
            speed_range: (3, 4),
            drops_coeff: 1.0,
            speed_coeff: 1.0,
        };
        let mut rain = DigitalRain::new(options, (80, 40));

        rain.update_size(10, 10);

        assert_eq!(rain.options.drops_range, (10, 20));
        assert_eq!(rain.options.speed_range, (2, 16));
    }

    #[test]
    fn no_diff() {
        let mut foo = DigitalRain::new(get_sane_default_options(), (100, 100));
        let q = foo.get_diff();
        assert!(q.is_empty());
    }

    #[test]
    fn same_diff_and_update() {
        let mut foo = DigitalRain::new(get_sane_default_options(), (100, 100));
        let mut q = Vec::new();
        for _ in 0..60 {
            foo.update();
            q = foo.get_diff();
            if !q.is_empty() {
                break;
            }
        }
        assert!(!q.is_empty());
    }

    #[test]
    fn quantum_update_moves_less_than_the_legacy_step() {
        let options = get_sane_default_options();
        let mut quick = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c'],
            crate::rain::rain_drop::RainDropStyle::Back,
            10,
            10.0,
            20,
            10,
        );
        let mut tick = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c'],
            crate::rain::rain_drop::RainDropStyle::Back,
            10,
            10.0,
            20,
            10,
        );

        quick.update(
            (100, 100),
            &options,
            Duration::from_millis(50),
            &mut rand::rng(),
        );
        tick.update(
            (100, 100),
            &options,
            Duration::from_secs_f64(1.0 / 60.0),
            &mut rand::rng(),
        );

        assert!(tick.fy < quick.fy);
    }
}
