use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Direction the crab is facing
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Left,
    Right,
}

// Selected animation frames for the crab
// Each frame has (or no?) the same width/height for consistent rendering
static CRAB_FRAMES: LazyLock<Vec<&str>> = LazyLock::new(|| {
    vec![
        // Frame 0: Standard pose facing right
        r#"    _~^~^~_
\) /  o o  \ (/
  '_   ¬   _'
  \ '-----' /"#,
        // Frame 1: Walking pose facing right
        r#"    _~^~^~_
\) /  o o  \ (/
 '-,   -  _'\
  | '----' "#,
        // Frame 2: Special pose (claws open) facing right
        r#"    _~^~^~_
\/ /  o o  \ \/
  '_   u   _'
  \ '-----' /"#,
        // Frame 3: Standard pose facing left (mirrored)
        r#"    _~^~^~_
(\ /  o o  \ ()
  '_   ¬   _'
  / '-----' \"#,
        // Frame 4: Walking pose facing left (mirrored)
        r#"    _~^~^~_
(\ /  o o  \ ()
 /'_  -   ,-'
    '----' |"#,
        // Frame 5: Special pose (claws open) facing left
        r#"    _~^~^~_
\/ /  o o  \ \/
  '_   u   _'
  / '-----' \"#,
    ]
});

// Individual crab entity
#[derive(Clone)]
struct CrabEntity {
    position: (f32, f32), // Floating point for smooth movement
    velocity: (f32, f32), // Direction and speed
    direction: Direction, // Facing left or right
    current_frame: usize, // Current animation frame
    animation_timer: f32, // Timer for animation
    special_timer: f32,   // Timer for special animations
    is_special: bool,     // Whether doing special animation
    color: style::Color,  // Crab color
    frame_width: usize,   // Cached frame width
    frame_height: usize,  // Cached frame height
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrabOptions {
    #[serde(skip)]
    pub crab_count: u16,

    pub animation_speed: f32,

    pub clap_chance: f32, // Random chance for special animation

    pub movement_speed: f32,

    pub crab_coeff: f32,

    /// Seed for the initial colony and every movement, turn and clap after it.
    pub seed: u64,
}

impl Default for CrabOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            crab_count: 5,
            animation_speed: 0.2,
            clap_chance: 0.05,
            movement_speed: 3.0,
            crab_coeff: 1.0,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Crab {
    pub screen_size: (u16, u16),
    options: CrabOptions,
    canvas: Canvas,
    crabs: Vec<CrabEntity>,
    rng: EffectRng,
    frame_timer: f32,
}

impl CrabEntity {
    fn new(
        position: (f32, f32),
        velocity: (f32, f32),
        rng: &mut EffectRng,
    ) -> Self {
        // Determine initial direction based on velocity
        let direction = if velocity.0 >= 0.0 {
            Direction::Right
        } else {
            Direction::Left
        };

        // Random color with predominantly red tint for crabs
        let color = style::Color::Rgb {
            r: rng.random_range(200..=255),
            g: rng.random_range(50..=150),
            b: rng.random_range(50..=100),
        };

        // Calculate frame dimensions from the first frame
        let frame_lines: Vec<&str> = CRAB_FRAMES[0].lines().collect();
        let frame_height = frame_lines.len();
        let frame_width =
            frame_lines.iter().map(|line| line.len()).max().unwrap_or(0);

        Self {
            position,
            velocity,
            direction,
            current_frame: 0,
            animation_timer: 0.0,
            special_timer: 0.0,
            is_special: false,
            color,
            frame_width,
            frame_height,
        }
    }

    // Get the appropriate frame for the crab's current state
    fn get_frame_index(&self) -> usize {
        if self.is_special {
            // Special animation (open claws)
            if self.direction == Direction::Right {
                2
            } else {
                5
            }
        } else if self.direction == Direction::Right {
            // Walking animation right
            self.current_frame % 2 // Alternate between frames 0 and 1
        } else {
            // Walking animation left
            3 + (self.current_frame % 2) // Alternate between frames 3 and 4
        }
    }

    // Get the lines of the current frame for rendering
    fn get_frame_lines(&self) -> Vec<String> {
        let frame_index = self.get_frame_index();
        CRAB_FRAMES[frame_index]
            .lines()
            .map(|line| line.to_string())
            .collect()
    }

    // Update the crab's position and animation state
    fn update(
        &mut self,
        dt: f32,
        screen_size: (u16, u16),
        animation_speed: f32,
        movement_speed: f32,
        clap_chance: f32,
        rng: &mut EffectRng,
    ) {
        // Update position based on velocity
        self.position.0 += self.velocity.0 * movement_speed * dt;
        self.position.1 += self.velocity.1 * movement_speed * dt;

        // Screen boundary collision detection
        let width = screen_size.0 as f32;
        let height = screen_size.1 as f32;

        // Detect collision with screen edges and reverse direction
        if self.position.0 < 0.0 {
            self.position.0 = 0.0;
            self.velocity.0 = rng.random_range(0.5..1.5);
            self.direction = Direction::Right;
        } else if self.position.0 + self.frame_width as f32 > width {
            self.position.0 = width - self.frame_width as f32;
            self.velocity.0 = rng.random_range(-1.5..-0.5);
            self.direction = Direction::Left;
        }

        if self.position.1 < 0.0 {
            self.position.1 = 0.0;
            self.velocity.1 = rng.random_range(0.2..0.8);
        } else if self.position.1 + self.frame_height as f32 > height {
            self.position.1 = height - self.frame_height as f32;
            self.velocity.1 = rng.random_range(-0.8..-0.2);
        }

        // Update direction based on velocity
        if self.velocity.0 > 0.0 {
            self.direction = Direction::Right;
        } else if self.velocity.0 < 0.0 {
            self.direction = Direction::Left;
        }

        // Update animation timer
        let is_moving = self.velocity.0.abs() > 0.1 || self.velocity.1.abs() > 0.1;
        if is_moving {
            self.animation_timer += dt;
            if self.animation_timer >= animation_speed {
                self.animation_timer = 0.0;
                self.current_frame = (self.current_frame + 1) % 2;
            }
        } else {
            // Use standing frame when not moving
            self.animation_timer = 0.0;
            self.current_frame = 0;
        }

        // Update special animation
        if self.is_special {
            self.special_timer -= dt;
            if self.special_timer <= 0.0 {
                self.is_special = false;
            }
        } else if rng.random::<f32>() < clap_chance * dt {
            // Random chance to trigger special animation
            self.is_special = true;
            self.special_timer = animation_speed * 5.0; // Duration of special animation
        }

        // Occasionally add slight randomness to movement
        if rng.random::<f32>() < 0.02 {
            self.velocity.0 += rng.random_range(-0.2..0.2);
            self.velocity.1 += rng.random_range(-0.1..0.1);

            // Keep velocity in reasonable bounds
            if self.velocity.0.abs() > 2.0 {
                self.velocity.0 = self.velocity.0.signum() * 1.5;
            }

            if self.velocity.1.abs() > 1.0 {
                self.velocity.1 = self.velocity.1.signum() * 0.5;
            }
        }
    }
}

impl TerminalEffect for Crab {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear();

        // Draw each crab
        for crab in &self.crabs {
            let frame_lines = crab.get_frame_lines();
            let base_x = crab.position.0.round() as usize;
            let base_y = crab.position.1.round() as usize;

            // Draw each line of the crab frame
            for (y_offset, line) in frame_lines.iter().enumerate() {
                let y = base_y + y_offset;
                if y >= self.canvas.height() {
                    continue;
                }

                for (x_offset, ch) in line.chars().enumerate() {
                    let x = base_x + x_offset;
                    if x >= self.canvas.width() || ch == ' ' {
                        continue;
                    }

                    // Set the character in the canvas
                    self.canvas.set(
                        x,
                        y,
                        Cell::new(ch, crab.color, style::Attribute::Bold),
                    );
                }
            }
        }

        self.canvas.commit()
    }

    fn update(&mut self) {
        // The rate the defaults were tuned against. Driven through the plain
        // `update` path, the colony still walks at the speed it was designed
        // for; the frame loop supplies the real delta below.
        self.step(1.0 / 30.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not teleport the colony across the screen.
        self.step(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        let area = self.screen_size.0 as f32 * self.screen_size.1 as f32;
        self.options.crab_count =
            (area / 800.0 * self.options.crab_coeff).clamp(3.0, 15.0) as u16;
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Crab {
    fn step(&mut self, dt: f64) {
        self.frame_timer += dt as f32;

        for crab in &mut self.crabs {
            crab.update(
                dt as f32,
                self.screen_size,
                self.options.animation_speed,
                self.options.movement_speed,
                self.options.clap_chance,
                &mut self.rng,
            );
        }

        self.check_crab_collisions();
    }

    pub fn new(options: CrabOptions, screen_size: (u16, u16)) -> Self {
        // One generator for the whole colony, drawn from sequentially, so the
        // crabs diverge from each other the way separate draws would.
        let mut rng = seeded_rng(options.seed, "crab");
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        let width = screen_size.0 as f32;
        let height = screen_size.1 as f32;

        // Create initial crabs with random positions and velocities
        let mut crabs = Vec::with_capacity(options.crab_count as usize);
        for _ in 0..options.crab_count {
            let position = (
                rng.random_range(0.0..width * 0.8),
                rng.random_range(0.0..height * 0.8),
            );

            // Random velocity, but ensure it's not too slow
            let velocity = (
                rng.random_range(-1.0f32..1.0f32).signum()
                    * rng.random_range(0.5..1.5),
                rng.random_range(-0.5..0.5),
            );

            crabs.push(CrabEntity::new(position, velocity, &mut rng));
        }

        let min_distance_squared = 100.0; // Adjust based on crab size
        let mut i = 0;
        let mut attempts = 0;
        let max_attempts = crabs.len().saturating_mul(64).max(1);
        while i < crabs.len() && attempts < max_attempts {
            attempts += 1;
            let mut repositioned = false;

            for j in 0..i {
                let dx = crabs[i].position.0 - crabs[j].position.0;
                let dy = crabs[i].position.1 - crabs[j].position.1;
                let distance_squared = dx * dx + dy * dy;

                if distance_squared < min_distance_squared {
                    crabs[i].position.0 = rng.random_range(0.0..width * 0.8);
                    crabs[i].position.1 = rng.random_range(0.0..height * 0.8);
                    repositioned = true;
                    break;
                }
            }

            if !repositioned {
                i += 1;
            }
        }

        Self {
            screen_size,
            options,
            canvas,
            crabs,
            rng,
            frame_timer: 0.0,
        }
    }

    // Check for collisions between crabs and handle them
    fn check_crab_collisions(&mut self) {
        let crab_count = self.crabs.len();
        if crab_count < 2 {
            return;
        }

        // Simple collision detection based on proximity
        for i in 0..crab_count {
            for j in (i + 1)..crab_count {
                let dx = self.crabs[i].position.0 - self.crabs[j].position.0;
                let dy = self.crabs[i].position.1 - self.crabs[j].position.1;
                let distance_squared = dx * dx + dy * dy;

                // If crabs are close enough, consider it a collision
                if distance_squared < 36.0 {
                    // Trigger special animation for both crabs
                    self.crabs[i].is_special = true;
                    self.crabs[i].special_timer =
                        self.options.animation_speed * 5.0;

                    self.crabs[j].is_special = true;
                    self.crabs[j].special_timer =
                        self.options.animation_speed * 5.0;

                    // Reverse directions
                    self.crabs[i].velocity.0 = -self.crabs[i].velocity.0;
                    self.crabs[j].velocity.0 = -self.crabs[j].velocity.0;

                    // Add some vertical movement to avoid getting stuck
                    self.crabs[i].velocity.1 += self.rng.random_range(-0.5..0.5);
                    self.crabs[j].velocity.1 += self.rng.random_range(-0.5..0.5);

                    // Update directions based on new velocities
                    self.crabs[i].direction = if self.crabs[i].velocity.0 >= 0.0 {
                        Direction::Right
                    } else {
                        Direction::Left
                    };

                    self.crabs[j].direction = if self.crabs[j].velocity.0 >= 0.0 {
                        Direction::Right
                    } else {
                        Direction::Left
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_recomputes_crab_count() {
        let options = CrabOptions {
            crab_count: 10,
            crab_coeff: 1.0,
            ..Default::default()
        };
        let mut crab = Crab::new(options, (80, 40));

        crab.update_size(10, 10);

        assert_eq!(crab.screen_size, (10, 10));
        assert_eq!(crab.options.crab_count, 3);
    }

    #[test]
    fn new_terminates_at_minimum_size() {
        let options = CrabOptions {
            crab_count: 3,
            ..Default::default()
        };

        let _ = Crab::new(options, (6, 6));
    }
}
