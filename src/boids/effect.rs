use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, TerminalEffect, seeded_rng};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum BoidCharset {
    #[default]
    Braille,
    Arrow,
    Simple,
    Dot,
}

impl BoidCharset {
    pub fn chars(&self) -> [char; 8] {
        match self {
            BoidCharset::Braille => ['⣤', '⢰', '⣰', '⡆', '⡇', '⠇', '⠛', '⠙'],
            BoidCharset::Arrow => ['→', '↘', '↓', '↙', '←', '↖', '↑', '↗'],
            BoidCharset::Simple => ['>', '>', 'v', 'v', '<', '<', '^', '^'],
            BoidCharset::Dot => ['•'; 8],
        }
    }
}

// Individual boid
#[derive(Clone)]
struct Boid {
    position: (f32, f32), // Floating point for smooth movement
    velocity: (f32, f32), // Direction vector
    character: char,      // Visual representation
    color: style::Color,  // Color based on velocity/state
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoidsOptions {
    #[serde(skip)]
    pub screen_size: (u16, u16),
    #[serde(skip)]
    pub boid_count: u16,

    pub boid_coeff: f32,

    // Separation parameters
    separation_weight: f32,
    separation_distance: f32,

    // Alignment parameters
    alignment_weight: f32,
    alignment_distance: f32,

    // Cohesion parameters
    cohesion_weight: f32,
    cohesion_distance: f32,

    // Additional parameters
    drive_factor: f32,  // Helps maintain momentum
    swirl_factor: f32,  // Adds some rotation to movement
    border_factor: f32, // How strongly to avoid borders

    max_speed: f32,
    min_speed: f32,

    charset: BoidCharset,

    /// Seed for the initial flock. Boids never draw again after `new`, so this
    /// fixes the whole run.
    pub seed: u64,
}

impl Default for BoidsOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            screen_size: Default::default(),
            boid_count: 100,
            boid_coeff: 1.0,
            separation_weight: 1.5,
            separation_distance: 3.0,
            alignment_weight: 2.0,
            alignment_distance: 15.0,
            cohesion_weight: 1.5,
            cohesion_distance: 15.0,
            drive_factor: 2.0,
            swirl_factor: 1.2,
            border_factor: 1.8,
            max_speed: 0.6,
            min_speed: 0.08,
            charset: BoidCharset::default(),
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Boids {
    options: BoidsOptions,
    canvas: Canvas,
    boids: Vec<Boid>,
    charset_chars: [char; 8],
}

impl Boid {
    fn new(position: (f32, f32), velocity: (f32, f32)) -> Self {
        Self {
            position,
            velocity,
            character: '•',
            color: style::Color::White,
        }
    }

    fn get_direction_char(&self, charset: &[char; 8]) -> char {
        let (vx, vy) = self.velocity;
        let angle = f32::atan2(vy, vx);

        let idx = ((angle / PI * 4.0).round() as i32 + 8) % 8;
        charset[idx as usize]
    }

    fn update_visual(&mut self, charset: &[char; 8]) {
        self.character = self.get_direction_char(charset);

        // Speed-based color (green to white)
        let speed = (self.velocity.0.powi(2) + self.velocity.1.powi(2)).sqrt();
        let intensity = ((speed * 128.0).clamp(0.0, 255.0)) as u8;
        self.color = style::Color::Rgb {
            r: intensity,
            g: 200,
            b: intensity.saturating_add(20),
        };
    }
}

impl TerminalEffect for Boids {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear();

        // Fill the canvas with boids
        for boid in &self.boids {
            let x = boid.position.0.round() as usize
                % self.options.screen_size.0 as usize;
            let y = boid.position.1.round() as usize
                % self.options.screen_size.1 as usize;

            self.canvas.set(
                x,
                y,
                Cell::new(boid.character, boid.color, style::Attribute::Bold),
            );
        }

        self.canvas.commit()
    }

    fn update(&mut self) {
        // `scale` is 1.0 at 60 fps, which is the rate the weights in the config
        // were tuned against, so the plain `update` path is unchanged.
        self.step(1.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not fling the flock off screen, and converted
        // to a multiple of the nominal frame so the weights keep their meaning.
        let seconds = context.delta.as_secs_f64().min(0.1);
        self.step((seconds * 60.0) as f32);
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.options.screen_size = (width.max(1), height.max(1));
        self.canvas
            .resize(self.options.screen_size.0, self.options.screen_size.1);
        let area =
            self.options.screen_size.0 as f32 * self.options.screen_size.1 as f32;
        self.options.boid_count =
            ((area * 0.5 * self.options.boid_coeff) as u16).clamp(50, 300);
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone());
    }
}

impl Boids {
    /// Advances the flock by `scale`, a multiple of the nominal frame.
    fn step(&mut self, scale: f32) {
        self.apply_rules(scale);
        self.update_positions(scale);
    }

    pub fn new(options: BoidsOptions) -> Self {
        let mut rng = seeded_rng(options.seed, "boids");
        let canvas = Canvas::new(options.screen_size.0, options.screen_size.1);

        let width = options.screen_size.0 as f32;
        let height = options.screen_size.1 as f32;

        // Create initial boids with random positions and velocities
        let charset_chars = options.charset.chars();

        let mut boids = Vec::with_capacity(options.boid_count as usize);
        for _ in 0..options.boid_count {
            let position =
                (rng.random_range(0.0..width), rng.random_range(0.0..height));

            let velocity =
                (rng.random_range(-1.0..1.0), rng.random_range(-1.0..1.0));

            let mut boid = Boid::new(position, velocity);
            boid.update_visual(&charset_chars);
            boids.push(boid);
        }

        Self {
            options,
            canvas,
            boids,
            charset_chars,
        }
    }

    // Calculate toroidal difference between two positions
    fn toroidal_diff(&self, a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
        let width = self.options.screen_size.0 as f32;
        let height = self.options.screen_size.1 as f32;

        let mut dx = a.0 - b.0;
        let mut dy = a.1 - b.1;

        if dx > width / 2.0 {
            dx -= width;
        } else if dx < -width / 2.0 {
            dx += width;
        }

        if dy > height / 2.0 {
            dy -= height;
        } else if dy < -height / 2.0 {
            dy += height;
        }

        (dx, dy)
    }

    fn apply_rules(&mut self, scale: f32) {
        let num_boids = self.boids.len();
        let mut separation_adjustments = vec![(0.0, 0.0); num_boids];
        let mut alignment_adjustments = vec![(0.0, 0.0); num_boids];
        let mut cohesion_adjustments = vec![(0.0, 0.0); num_boids];
        let mut border_adjustments = vec![(0.0, 0.0); num_boids];

        // Pre-calculate all adjustments
        for i in 0..num_boids {
            // Apply separation rule
            let mut separation = (0.0, 0.0);
            let mut sep_count = 0;

            // Apply alignment rule
            let mut avg_velocity = (0.0, 0.0);
            let mut align_count = 0;

            // Apply cohesion rule
            let mut center = (0.0, 0.0);
            let mut cohesion_count = 0;

            for j in 0..num_boids {
                if i == j {
                    continue;
                }

                let diff = self
                    .toroidal_diff(self.boids[j].position, self.boids[i].position);
                let distance = (diff.0.powi(2) + diff.1.powi(2)).sqrt();

                // Separation
                #[allow(clippy::collapsible_if)]
                if distance < self.options.separation_distance {
                    if distance > 0.0 {
                        let factor = 1.0 / distance;
                        separation.0 -= diff.0 * factor;
                        separation.1 -= diff.1 * factor;
                        sep_count += 1;
                    }
                }

                // Alignment
                if distance < self.options.alignment_distance {
                    avg_velocity.0 += self.boids[j].velocity.0;
                    avg_velocity.1 += self.boids[j].velocity.1;
                    align_count += 1;
                }

                // Cohesion
                if distance < self.options.cohesion_distance {
                    center.0 += self.boids[j].position.0;
                    center.1 += self.boids[j].position.1;
                    cohesion_count += 1;
                }
            }

            // Finalize separation
            if sep_count > 0 {
                separation_adjustments[i] = (
                    separation.0 * self.options.separation_weight,
                    separation.1 * self.options.separation_weight,
                );
            }

            // Finalize alignment
            if align_count > 0 {
                let avg_vel = (
                    avg_velocity.0 / align_count as f32,
                    avg_velocity.1 / align_count as f32,
                );

                alignment_adjustments[i] = (
                    (avg_vel.0 - self.boids[i].velocity.0)
                        * self.options.alignment_weight
                        * 0.05,
                    (avg_vel.1 - self.boids[i].velocity.1)
                        * self.options.alignment_weight
                        * 0.05,
                );
            }

            // Finalize cohesion
            if cohesion_count > 0 {
                let perceived_center = (
                    center.0 / cohesion_count as f32,
                    center.1 / cohesion_count as f32,
                );

                let toward_center =
                    self.toroidal_diff(perceived_center, self.boids[i].position);

                // Calculate perpendicular (swirl) vector
                let swirl = (-toward_center.1, toward_center.0);
                let swirl_len = (swirl.0.powi(2) + swirl.1.powi(2)).sqrt();
                let swirl_normalized = if swirl_len > 0.0 {
                    (swirl.0 / swirl_len, swirl.1 / swirl_len)
                } else {
                    (0.0, 0.0)
                };

                cohesion_adjustments[i] = (
                    toward_center.0 * self.options.cohesion_weight * 0.03
                        + swirl_normalized.0 * self.options.swirl_factor * 0.02,
                    toward_center.1 * self.options.cohesion_weight * 0.03
                        + swirl_normalized.1 * self.options.swirl_factor * 0.02,
                );
            }

            // Apply border avoidance
            let width = self.options.screen_size.0 as f32;
            let height = self.options.screen_size.1 as f32;
            let border_margin = 5.0;
            let border_strength = self.options.border_factor;

            let mut border_force = (0.0, 0.0);
            let pos = self.boids[i].position;

            // Left edge
            if pos.0 < border_margin {
                border_force.0 += border_strength * (1.0 - pos.0 / border_margin);
            }
            // Right edge
            else if pos.0 > width - border_margin {
                border_force.0 -=
                    border_strength * (1.0 - (width - pos.0) / border_margin);
            }

            // Top edge
            if pos.1 < border_margin {
                border_force.1 += border_strength * (1.0 - pos.1 / border_margin);
            }
            // Bottom edge
            else if pos.1 > height - border_margin {
                border_force.1 -=
                    border_strength * (1.0 - (height - pos.1) / border_margin);
            }

            border_adjustments[i] = border_force;
        }

        // Apply all forces to boids
        for i in 0..num_boids {
            // Get current velocity
            let mut new_vx = self.boids[i].velocity.0;
            let mut new_vy = self.boids[i].velocity.1;

            // Apply rules. Scaled so the flock turns at the same rate whatever
            // the frame rate; the damping below is left unscaled because it is a
            // fixed fraction of the previous velocity, not a force.
            new_vx += separation_adjustments[i].0 * scale;
            new_vy += separation_adjustments[i].1 * scale;

            new_vx += alignment_adjustments[i].0 * scale;
            new_vy += alignment_adjustments[i].1 * scale;

            new_vx += cohesion_adjustments[i].0 * scale;
            new_vy += cohesion_adjustments[i].1 * scale;

            new_vx += border_adjustments[i].0 * scale;
            new_vy += border_adjustments[i].1 * scale;

            // Apply drive factor
            let speed = (new_vx * new_vx + new_vy * new_vy).sqrt();
            if speed > 0.0 {
                let normalized_vx = new_vx / speed;
                let normalized_vy = new_vy / speed;
                new_vx += normalized_vx * self.options.drive_factor * 0.1;
                new_vy += normalized_vy * self.options.drive_factor * 0.1;
            }

            // Apply damping for smoother movement
            new_vx = self.boids[i].velocity.0 * 0.7 + new_vx * 0.3;
            new_vy = self.boids[i].velocity.1 * 0.7 + new_vy * 0.3;

            // Apply speed limits
            let speed = (new_vx * new_vx + new_vy * new_vy).sqrt();
            if speed > self.options.max_speed {
                let scale = self.options.max_speed / speed;
                new_vx *= scale;
                new_vy *= scale;
            } else if speed < self.options.min_speed && speed > 0.0 {
                let scale = self.options.min_speed / speed;
                new_vx *= scale;
                new_vy *= scale;
            }

            // Update velocity
            self.boids[i].velocity = (new_vx, new_vy);
        }
    }

    fn update_positions(&mut self, scale: f32) {
        let width = self.options.screen_size.0 as f32;
        let height = self.options.screen_size.1 as f32;

        for boid in &mut self.boids {
            // Update position
            boid.position.0 += boid.velocity.0 * scale;
            boid.position.1 += boid.velocity.1 * scale;

            // Wrap around screen boundaries
            if boid.position.0 < 0.0 {
                boid.position.0 += width;
            } else if boid.position.0 >= width {
                boid.position.0 -= width;
            }

            if boid.position.1 < 0.0 {
                boid.position.1 += height;
            } else if boid.position.1 >= height {
                boid.position.1 -= height;
            }

            // Update visual representation
            boid.update_visual(&self.charset_chars);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_recomputes_boid_count() {
        let options = BoidsOptions {
            screen_size: (80, 40),
            boid_count: 10,
            boid_coeff: 1.0,
            ..Default::default()
        };
        let mut boids = Boids::new(options);

        boids.update_size(10, 10);

        assert_eq!(boids.options.screen_size, (10, 10));
        assert_eq!(boids.options.boid_count, 50);
    }
}
