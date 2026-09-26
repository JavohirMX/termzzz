use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

const STAR_GLYPHS: [char; 4] = ['○', '◦', '*', '✦'];

const PALETTE: [(u8, u8, u8); 4] = [
    (110, 150, 240),
    (170, 110, 230),
    (90, 210, 230),
    (190, 150, 255),
];

const DIM_PALETTE: [(u8, u8, u8); 4] =
    [(33, 43, 78), (48, 30, 68), (26, 58, 68), (53, 38, 78)];

const BRIGHT: (u8, u8, u8) = (238, 243, 255);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConstellationOptions {
    pub star_count: usize,
    pub connect_radius: f64,
    pub max_connections: usize,
    pub twinkle: bool,
    pub min_speed: f64,
    pub max_speed: f64,
    /// Seed for the star field. Fixes positions, speeds, glyphs and twinkle
    /// phases, so the whole sky is reproducible.
    pub seed: u64,
}

impl Default for ConstellationOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            star_count: 65,
            connect_radius: 0.18,
            max_connections: 4,
            twinkle: true,
            min_speed: 0.3,
            max_speed: 1.5,
            seed: DEFAULT_SEED,
        }
    }
}

#[derive(Clone, Debug)]
struct Star {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    twinkle: f64,
    twinkle_freq: f64,
    glyph_idx: usize,
    palette_idx: usize,
}

pub struct Constellation {
    screen_size: (u16, u16),
    options: ConstellationOptions,
    canvas: Canvas,
    stars: Vec<Star>,
    connect_dist: f64,
}

impl TerminalEffect for Constellation {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // Destructured so the canvas can be borrowed while the rest of the
        // effect is read; a `&self` draw method could not also hold
        // `&mut self.canvas`.
        let Self {
            stars,
            connect_dist,
            options,
            screen_size,
            canvas,
            ..
        } = self;

        canvas.clear();
        Self::draw_connections(
            stars,
            *connect_dist,
            options,
            *screen_size,
            canvas.surface_mut(),
        );
        Self::draw_stars(stars, options, *screen_size, canvas.surface_mut());
        canvas.commit()
    }

    fn update(&mut self) {
        // Nominal frame time, so an effect driven through the plain `update`
        // path still moves at the rate it was tuned for.
        self.step(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not teleport every star across the screen.
        self.step(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas
            .resize(self.screen_size.0.max(1), self.screen_size.1.max(1));
        self.connect_dist = Self::calc_connect_dist(
            self.screen_size.0,
            self.screen_size.1,
            self.options.connect_radius,
        );

        self.stars.clear();
        self.stars.reserve(self.options.star_count);
        let mut rng = seeded_rng(self.options.seed, "constellation");
        for _ in 0..self.options.star_count {
            self.stars.push(Self::random_star(
                &self.screen_size,
                &self.options,
                &mut rng,
                true,
            ));
        }
    }
}

impl Constellation {
    pub fn new(options: ConstellationOptions, screen_size: (u16, u16)) -> Self {
        let mut effect = Self {
            screen_size,
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            stars: Vec::new(),
            connect_dist: 0.0,
        };

        effect.reset();
        effect
    }

    fn step(&mut self, dt: f64) {
        let width = self.screen_size.0 as f64;
        let height = self.screen_size.1 as f64;

        for star in &mut self.stars {
            star.x += star.vx * dt;
            star.y += star.vy * dt;

            if self.options.twinkle {
                star.twinkle += star.twinkle_freq * dt;
            }

            if star.x < 0.0 {
                star.x = -star.x;
                star.vx = -star.vx;
            } else if star.x >= width {
                star.x = 2.0 * width - star.x - 0.01;
                star.vx = -star.vx;
            }

            if star.y < 0.0 {
                star.y = -star.y;
                star.vy = -star.vy;
            } else if star.y >= height {
                star.y = 2.0 * height - star.y - 0.01;
                star.vy = -star.vy;
            }
        }
    }

    fn calc_connect_dist(width: u16, height: u16, radius_factor: f64) -> f64 {
        ((width as f64).powi(2) + (height as f64).powi(2)).sqrt() * radius_factor
    }

    fn random_star(
        screen_size: &(u16, u16),
        options: &ConstellationOptions,
        rng: &mut EffectRng,
        scattered: bool,
    ) -> Star {
        let speed = rng.random_range(options.min_speed..options.max_speed);
        let angle = rng.random_range(0.0..(std::f64::consts::PI * 2.0));

        let (x, y) = if scattered {
            (
                rng.random_range(0.0..screen_size.0 as f64),
                rng.random_range(0.0..screen_size.1 as f64),
            )
        } else {
            match rng.random_range(0..4) {
                0 => (rng.random_range(0.0..screen_size.0 as f64), 0.0),
                1 => (
                    rng.random_range(0.0..screen_size.0 as f64),
                    screen_size.1.saturating_sub(1) as f64,
                ),
                2 => (0.0, rng.random_range(0.0..screen_size.1 as f64)),
                _ => (
                    screen_size.0.saturating_sub(1) as f64,
                    rng.random_range(0.0..screen_size.1 as f64),
                ),
            }
        };

        Star {
            x,
            y,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed,
            twinkle: rng.random_range(0.0..(std::f64::consts::PI * 2.0)),
            twinkle_freq: rng.random_range(0.4..1.2),
            glyph_idx: rng.random_range(0..STAR_GLYPHS.len()),
            palette_idx: rng.random_range(0..PALETTE.len()),
        }
    }

    fn draw_connections(
        stars: &[Star],
        connect_dist: f64,
        options: &ConstellationOptions,
        size: (u16, u16),
        buffer: &mut Buffer,
    ) {
        let mut conn_count = vec![0usize; stars.len()];

        for i in 0..stars.len() {
            let mut neighbors: Vec<(usize, f64)> = Vec::new();
            for j in (i + 1)..stars.len() {
                let dx = stars[j].x - stars[i].x;
                let dy = stars[j].y - stars[i].y;
                let distance = (dx * dx + dy * dy).sqrt();

                if distance <= connect_dist {
                    neighbors.push((j, distance));
                }
            }

            neighbors.sort_by(|a, b| a.1.total_cmp(&b.1));

            for (j, distance) in neighbors {
                if conn_count[i] >= options.max_connections
                    || conn_count[j] >= options.max_connections
                {
                    continue;
                }

                conn_count[i] += 1;
                conn_count[j] += 1;

                let alpha = (1.0 - distance / connect_dist) * 0.55;
                let color = Self::connection_color(stars[i].palette_idx, alpha);

                Self::draw_dotted_line(
                    size,
                    buffer,
                    stars[i].x.round() as i32,
                    stars[i].y.round() as i32,
                    stars[j].x.round() as i32,
                    stars[j].y.round() as i32,
                    color,
                );
            }
        }
    }

    fn draw_stars(
        stars: &[Star],
        options: &ConstellationOptions,
        size: (u16, u16),
        buffer: &mut Buffer,
    ) {
        for star in stars {
            let brightness = if options.twinkle {
                0.55 + 0.45 * star.twinkle.sin()
            } else {
                0.85
            };

            let mut color = Self::star_color(star.palette_idx, brightness);
            if brightness > 0.9 {
                let pal = PALETTE[star.palette_idx % PALETTE.len()];
                color = lerp_color(
                    style::Color::Rgb {
                        r: pal.0,
                        g: pal.1,
                        b: pal.2,
                    },
                    style::Color::Rgb {
                        r: BRIGHT.0,
                        g: BRIGHT.1,
                        b: BRIGHT.2,
                    },
                    (brightness - 0.9) * 10.0,
                );
            }

            let x = star.x.round() as i32;
            let y = star.y.round() as i32;
            if Self::in_bounds(size, x, y) {
                buffer.set(
                    x as usize,
                    y as usize,
                    Cell::new(
                        STAR_GLYPHS[star.glyph_idx],
                        color,
                        style::Attribute::Bold,
                    ),
                );
            }
        }
    }

    fn draw_dotted_line(
        size: (u16, u16),
        buffer: &mut Buffer,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: style::Color,
    ) {
        let dx = x1 - x0;
        let dy = y1 - y0;

        let steps = dx.abs().max(dy.abs());
        if steps < 2 {
            return;
        }

        for i in 1..steps {
            let t = i as f64 / steps as f64;
            let x = x0 + (dx as f64 * t + 0.5) as i32;
            let y = y0 + (dy as f64 * t + 0.5) as i32;

            if Self::in_bounds(size, x, y) {
                buffer.set(
                    x as usize,
                    y as usize,
                    Cell::new('·', color, style::Attribute::NormalIntensity),
                );
            }
        }
    }

    fn connection_color(palette_idx: usize, alpha: f64) -> style::Color {
        let dim = DIM_PALETTE[palette_idx % DIM_PALETTE.len()];
        let pal = PALETTE[palette_idx % PALETTE.len()];
        lerp_color(
            style::Color::Rgb {
                r: dim.0,
                g: dim.1,
                b: dim.2,
            },
            style::Color::Rgb {
                r: pal.0,
                g: pal.1,
                b: pal.2,
            },
            alpha,
        )
    }

    fn star_color(palette_idx: usize, brightness: f64) -> style::Color {
        let dim = DIM_PALETTE[palette_idx % DIM_PALETTE.len()];
        let pal = PALETTE[palette_idx % PALETTE.len()];
        lerp_color(
            style::Color::Rgb {
                r: dim.0,
                g: dim.1,
                b: dim.2,
            },
            style::Color::Rgb {
                r: pal.0,
                g: pal.1,
                b: pal.2,
            },
            brightness,
        )
    }

    fn in_bounds(size: (u16, u16), x: i32, y: i32) -> bool {
        x >= 0 && x < size.0 as i32 && y >= 0 && y < size.1 as i32
    }
}

fn lerp_color(a: style::Color, b: style::Color, t: f64) -> style::Color {
    let (ar, ag, ab) = as_rgb(a);
    let (br, bg, bb) = as_rgb(b);

    let clamped_t = t.clamp(0.0, 1.0);
    style::Color::Rgb {
        r: (ar as f64 + (br as f64 - ar as f64) * clamped_t) as u8,
        g: (ag as f64 + (bg as f64 - ag as f64) * clamped_t) as u8,
        b: (ab as f64 + (bb as f64 - ab as f64) * clamped_t) as u8,
    }
}

fn as_rgb(color: style::Color) -> (u8, u8, u8) {
    match color {
        style::Color::Rgb { r, g, b } => (r, g, b),
        _ => (255, 255, 255),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_constellation_effect() {
        let options = ConstellationOptions::default();
        let effect = Constellation::new(options, (80, 24));
        assert_eq!(effect.screen_size, (80, 24));
    }
}
