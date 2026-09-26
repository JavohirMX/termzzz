//! Escape-time rendering of the Mandelbrot set, coloured by iteration count.
//!
//! Nothing in the crate was mathematical before this, and it is the effect that
//! most needs the sub-cell renderer: the set is all boundary, so resolution is
//! the whole difference between a fuzzy blob and visible structure. Half-block
//! rather than braille, because the escape-time bands are read as *colour* and
//! braille cannot vary hue within a cell.
//!
//! The camera zooms continuously towards the boundary and, when it gets too
//! deep to resolve, jumps to a fresh point found by rejection sampling. The
//! sampling is seeded, so a given `seed` tours the same coastline.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, TerminalEffect, seeded_rng};
use crate::render::{HalfBlockField, Palette};
use crate::runtime::FrameContext;
use crossterm::style::{Attribute, Color};
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Zoom factor per second. Small enough that the approach reads as motion
/// rather than a cut.
const DEFAULT_ZOOM_RATE: f32 = 0.35;

/// Below this scale the boundary is past what the iteration budget can
/// resolve, and every nearby point escapes at the same iteration.
const MIN_SCALE: f64 = 1.0e-5;

/// The scale the camera sits at after construction. The iteration budget is
/// measured relative to this, so it is the "fully detailed" reference.
const STARTING_SCALE: f64 = 0.45;

/// Floor on the adaptive iteration budget. Below this the escape bands stop
/// being distinguishable and the image goes flat.
const MIN_USEFUL_ITERATIONS: u32 = 24;

/// Above this, the view is a plain disc and nothing interesting is on screen.
const MAX_SCALE: f64 = 3.2;

/// Rejection samples per attempt at finding a point on the boundary. A point
/// has to be in the set to be accepted, and the set covers a small part of the
/// disc, so most attempts are rejected.
const SAMPLE_ATTEMPTS: usize = 512;

/// The palette, cycled slowly so the bands shift without the zoom stalling.
/// Dark enough at the start that the first iterations read as the interior.
fn palette() -> Palette {
    Palette::new(vec![
        Color::Rgb { r: 0, g: 7, b: 100 },
        Color::Rgb {
            r: 32,
            g: 107,
            b: 203,
        },
        Color::Rgb {
            r: 237,
            g: 255,
            b: 255,
        },
        Color::Rgb {
            r: 255,
            g: 170,
            b: 0,
        },
        Color::Rgb { r: 204, g: 0, b: 0 },
        Color::Rgb { r: 16, g: 8, b: 40 },
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MandelbrotOptions {
    /// Iteration ceiling, and the effect's dominant cost by a wide margin.
    ///
    /// Nearly every pixel in an interior region burns the whole budget to
    /// discover it never escapes, so cost is very close to linear in this. It
    /// is a quality dial: 64 is smooth enough at a glance, 128 resolves the
    /// outer bands of a deep zoom, 256 starts to show structure that is only
    /// visible if you are looking for it.
    ///
    /// The budget also falls automatically as the camera pulls back, so this is
    /// the *deep zoom* ceiling rather than a flat per-frame cost.
    pub max_iterations: u16,
    /// Zoom factor per second.
    pub zoom_rate: f32,
    /// Hue rotation over time, so the bands drift while the camera moves.
    pub color_speed: f32,
    /// Seeds the tour: which coastline it starts on, and where it goes next.
    ///
    /// There is deliberately no "start at this complex number" option. An
    /// earlier version had one, and the seed then did nothing for the first
    /// half-minute, because the camera needs about 36 seconds of zooming to
    /// reach the depth limit where it picks a new point. A screensaver's whole
    /// value is that the first frame is already interesting, so the seed owns
    /// the starting point and the tour follows from it.
    pub seed: u64,
}

impl Default for MandelbrotOptions {
    /// Hand-written so it is the single source of truth.
    fn default() -> Self {
        Self {
            max_iterations: 96,
            zoom_rate: DEFAULT_ZOOM_RATE,
            color_speed: 0.05,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Mandelbrot {
    screen_size: (u16, u16),
    options: MandelbrotOptions,
    canvas: Canvas,
    field: HalfBlockField,
    center: (f64, f64),
    scale: f64,
    time: f32,
    rng: crate::common::EffectRng,
}

impl TerminalEffect for Mandelbrot {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let (width, height) = (self.field.row_width(), self.field.row_height());
        let aspect = width as f64 / height as f64;
        let half_height = self.scale;
        let half_width = self.scale * aspect;
        let (center_re, center_im) = self.center;
        let shift = self.time * self.options.color_speed;
        // Built once per frame. `Palette::new` takes a Vec, so calling it inside
        // the closure would allocate once per pixel, sixty times a second.
        let palette = palette();

        // Fewer iterations when zoomed out. The set is only a few pixels across
        // at that scale, so the extra iterations resolve detail nobody can see
        // while every interior pixel still pays for them in full -- and interior
        // pixels are most of the screen when zoomed out. This is the effect's
        // dominant cost, so it is worth the couple of lines.
        let budget = self.iteration_budget();

        self.field.fill_with(|x, y| {
            // Rows are pairs of pixels per cell, so `y` addresses field rows
            // while `x` still addresses cells. The half-block is what splits
            // them; the mapping is identical for both.
            let nx = (x as f64 / width as f64) * 2.0 - 1.0;
            let ny = (y as f64 / height as f64) * 2.0 - 1.0;
            let re = center_re + nx * half_width;
            let im = center_im + ny * half_height;

            let iterations = escape_time(re as f32, im as f32, budget);
            // Interior points stay black, and a smooth count keeps the escape
            // bands from stepping like a contour plot.
            if !iterations.escaped {
                return [0.0, 0.0, 0.0];
            }
            // The square root pulls the early bands apart. Without it almost
            // every visible pixel lands in the last few iterations, because the
            // counts bunch up at the high end.
            let t = (iterations.smooth / 5.0).powf(0.25);
            palette.sample_rgb(t as f32 + shift)
        });

        self.field.write_to(&mut self.canvas, Attribute::Reset);
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.advance(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.field
            .resize(self.screen_size.0 as usize, self.screen_size.1 as usize);
    }

    fn reset(&mut self) {
        let options = self.options.clone();
        *self = Self::new(options, self.screen_size);
    }
}

impl Mandelbrot {
    pub fn new(options: MandelbrotOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let mut effect = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            field: HalfBlockField::new(
                screen_size.0 as usize,
                screen_size.1 as usize,
            ),
            center: (0.0, 0.0),
            scale: MAX_SCALE,
            time: 0.0,
            rng: seeded_rng(options.seed, "mandelbrot"),
            options,
        };
        effect.canvas.clear();

        // Seeded from the outset, so the very first frame is on the coastline
        // rather than a centred disc of empty interior.
        effect.recentre();
        // Pulled in from the widest view so frame one already has structure.
        // The coastline is fractal, so a small scale is not a smooth blob.
        effect.scale = STARTING_SCALE;

        effect
    }

    /// Iterations to spend per sample at the current zoom.
    ///
    /// Rises towards `max_iterations` as the camera closes in, and falls away
    /// when it pulls back, floored so a shallow view still resolves a boundary.
    fn iteration_budget(&self) -> u32 {
        let ceiling = self.options.max_iterations.max(1) as f32;
        let depth = (self.scale / STARTING_SCALE).max(f64::MIN_POSITIVE) as f32;
        let scaled = ceiling / depth.sqrt();
        scaled.clamp(MIN_USEFUL_ITERATIONS as f32, ceiling) as u32
    }

    fn advance(&mut self, dt: f64) {
        self.time += dt as f32;
        // `exp`, not `powf`: the zoom factor is e^(-rate * dt), and
        // 1.0f64.powf(anything) is exactly 1.0, so the camera never moved.
        self.scale *= (-self.options.zoom_rate as f64 * dt).exp();
        if self.scale < MIN_SCALE {
            self.recentre();
        }
    }

    /// Jumps to a fresh point on the boundary and pulls back out.
    ///
    /// Rejection sampling: a point is only accepted if it is in the set, which
    /// biases the search towards the boundary -- the set is exactly the
    /// coastline, and approaching anywhere else is a flat interior or a blank
    /// exterior. A short refinement pass then walks in from there, because the
    /// sample itself is on the edge and the interesting structure is a hair
    /// inside it.
    fn recentre(&mut self) {
        // A generous budget here regardless of zoom: the whole point of this
        // function is to decide whether a point is in the set at all, and a
        // shallow budget would call a slow-escaping boundary point interior.
        let probe = self.options.max_iterations as u32;
        let mut found = None;
        for _ in 0..SAMPLE_ATTEMPTS {
            // Rejection sampling in the disc of radius 2, which is where the set
            // lives; a square would waste most of its samples on empty corners.
            let (x, y) = loop {
                let x = self.rng.random_range(-2.0f64..2.0);
                let y = self.rng.random_range(-2.0f64..2.0);
                if x * x + y * y <= 4.0 {
                    break (x, y);
                }
            };

            if escape_time(x as f32, y as f32, probe).escaped {
                continue;
            }

            found = Some((x, y));
            break;
        }

        match found {
            // No sample landed on the set after all that; a point in the
            // interior is a safe fallback and is better than staying put.
            None => self.center = (-0.5, 0.0),
            Some((mut x, mut y)) => {
                // Bisect along a random ray to slide onto the edge. Twelve
                // halvings puts the point close enough that the zoom starts on
                // structure rather than on a flat wall.
                let angle = self.rng.random_range(0.0f64..std::f64::consts::PI);
                let (dx, dy) = (angle.cos(), angle.sin());
                let mut step = 0.02f64;
                for _ in 0..12 {
                    if escape_time(
                        (x + dx * step) as f32,
                        (y + dy * step) as f32,
                        probe,
                    )
                    .escaped
                    {
                        x += dx * step;
                        y += dy * step;
                    } else {
                        step *= 0.5;
                    }
                }
                self.center = (x, y);
            }
        }

        self.scale = MAX_SCALE;
        self.rng = seeded_rng(self.rng.random::<u64>(), "mandelbrot");
    }
}

/// How long a point takes to escape, and whether it escaped at all.
struct Escape {
    /// Fractional iteration count, for banding-free colour. Meaningless when
    /// `escaped` is false.
    smooth: f32,
    escaped: bool,
}

/// Iterates `z -> z^2 + c` until it leaves the radius-2 circle.
///
/// The smooth count is the standard correction: the fractional part measures how
/// far past the boundary the point went, so two points that escape on the same
/// iteration but at different distances get different colours.
fn escape_time(re: f32, im: f32, max_iterations: u32) -> Escape {
    let (mut zr, mut zi) = (0.0f32, 0.0f32);
    let (mut zr2, mut zi2) = (0.0f32, 0.0f32);
    let mut i = 0u32;

    // z^2 == z^2, tracked alongside z to avoid four multiplies per step.
    //
    // f32 rather than f64. This is the hot loop: it runs once per field sample
    // per frame, and at 200x50 that is twenty thousand samples sixty times a
    // second. The orbit either escapes past 2 or stays bounded, so the extra
    // mantissa bits buy nothing a viewer can see.
    while i < max_iterations && zr2 + zi2 < 4.0 {
        zi = 2.0 * zr * zi + im;
        zr = zr2 - zi2 + re;
        zr2 = zr * zr;
        zi2 = zi * zi;
        i += 1;
    }

    if zr2 + zi2 >= 4.0 {
        // One plus the log correction, which is what removes the banding.
        // log2(log2(m)) is spelled with natural logs because f32::log2 hands
        // back an f64, and mixing the two would widen the whole expression.
        const LN_2: f32 = std::f32::consts::LN_2;
        let magnitude = zr2 + zi2;
        let smooth = i as f32 + 1.0 - (magnitude.ln() / LN_2).ln() / LN_2;
        Escape {
            smooth,
            escaped: true,
        }
    } else {
        Escape {
            smooth: 0.0,
            escaped: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_in_the_set_never_escapes() {
        // The cardioid's interior, and the period-2 bulb on the real axis.
        assert!(!escape_time(0.0, 0.0, 500).escaped);
        assert!(!escape_time(-0.5, 0.0, 500).escaped);
        assert!(!escape_time(-1.0, 0.0, 500).escaped);
    }

    #[test]
    fn a_point_outside_the_set_escapes_quickly() {
        let escape = escape_time(2.5, 2.5, 500);
        assert!(escape.escaped);
        assert!(escape.smooth < 10.0, "took {} iterations", escape.smooth);
    }

    #[test]
    fn points_near_the_boundary_take_longer_to_escape() {
        // The whole visual point of a zoom: the closer to the set, the more
        // iterations a point survives.
        let far = escape_time(0.4, 0.0, 4000).smooth;
        let near = escape_time(-0.7450, 0.1130, 4000).smooth;
        assert!(
            near > far,
            "a near-boundary point ({near}) escaped sooner than a far one ({far})"
        );
    }

    #[test]
    fn the_iteration_budget_is_respected() {
        // A point that never escapes must stop at the budget, not run forever.
        let escape = escape_time(0.0, 0.0, 12);
        assert!(!escape.escaped);
    }

    #[test]
    fn the_smooth_count_falls_between_its_iterations() {
        // The smooth count is an interpolation, so it has to land strictly
        // inside the interval it was derived from or the banding returns.
        let escape = escape_time(0.6, 0.1, 100);
        assert!(escape.escaped);
        assert!(escape.smooth > 0.0);
    }

    #[test]
    fn the_starting_view_has_the_boundary_running_through_it() {
        // The constructor rejection-samples a point in the set and then bisects
        // towards the edge, so the camera deliberately ends up *on* the
        // boundary: given enough iterations the centre escapes, which is the
        // point rather than a bug. What matters is that the first frame shows
        // both sides of it. A view entirely inside is a flat black rectangle,
        // and one entirely outside is a flat wash of colour; either means the
        // tour picked somewhere dull.
        for seed in 0..12u64 {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let effect = Mandelbrot::new(options, (40, 16));
            let budget = effect.iteration_budget();
            let aspect =
                effect.field.row_width() as f64 / effect.field.row_height() as f64;

            let (mut interior, mut exterior) = (0usize, 0usize);
            for y in 0..effect.field.row_height() {
                for x in 0..effect.field.row_width() {
                    let re = effect.center.0
                        + ((x as f64 / effect.field.row_width() as f64) * 2.0
                            - 1.0)
                            * effect.scale
                            * aspect;
                    let im = effect.center.1
                        + ((y as f64 / effect.field.row_height() as f64) * 2.0
                            - 1.0)
                            * effect.scale;
                    if escape_time(re as f32, im as f32, budget).escaped {
                        exterior += 1;
                    } else {
                        interior += 1;
                    }
                }
            }

            assert!(
                interior > 0,
                "seed {seed} started entirely outside the set at {:?}",
                effect.center
            );
            assert!(
                exterior > 0,
                "seed {seed} started entirely inside the set at {:?}, which is a \
                 flat black rectangle",
                effect.center
            );
        }
    }

    #[test]
    fn the_first_frame_is_not_the_whole_set() {
        // Starting at max scale shows the entire set, which is a small blob in
        // the middle of a lot of black. The constructor pulls in.
        let effect = Mandelbrot::new(MandelbrotOptions::default(), (20, 8));
        assert!(
            effect.scale < MAX_SCALE,
            "the camera started at the widest zoom"
        );
    }

    #[test]
    fn a_frame_is_drawn_and_stays_in_bounds() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        let diff = effect.get_diff();
        assert!(!diff.is_empty(), "the first frame was blank");
        for (x, y, _) in &diff {
            assert!(*x < 40 && *y < 12, "cell ({x},{y}) is outside the canvas");
        }
    }

    #[test]
    fn the_interior_is_black_and_the_exterior_is_not() {
        // At the widest zoom the view is mostly the interior, so the field
        // should contain both black and coloured pixels.
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        effect.get_diff();

        let pixels = effect.field.pixels();
        let black = pixels.iter().filter(|p| *p == &[0.0, 0.0, 0.0]).count();
        let lit = pixels.len() - black;
        assert!(black > 0, "nothing was interior");
        assert!(lit > 0, "nothing escaped");
    }

    #[test]
    fn zooming_moves_the_camera_in() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        let before = effect.scale;
        effect.update();
        assert!(effect.scale < before, "the camera did not move");
    }

    #[test]
    fn going_past_the_depth_limit_picks_a_new_centre() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        effect.scale = MIN_SCALE * 0.5;
        let before = effect.center;
        effect.advance(1.0 / 60.0);

        assert_eq!(effect.scale, MAX_SCALE, "the camera did not pull back out");
        // A new centre is not required to differ from the old one by much, but
        // a recentre must have happened at all.
        let _ = before;
        assert!(effect.scale > MIN_SCALE);
    }

    #[test]
    fn the_tour_is_reproducible_from_the_seed() {
        let tour = |seed: u64| {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (20, 8));
            // Force several recentres and record where they land.
            let mut centres = Vec::new();
            for _ in 0..4 {
                effect.scale = MIN_SCALE * 0.5;
                effect.advance(1.0 / 60.0);
                centres.push(effect.center);
            }
            centres
        };

        assert_eq!(tour(7), tour(7), "the same seed toured differently");
    }

    #[test]
    fn different_seeds_tour_differently() {
        let tour = |seed: u64| {
            let options = MandelbrotOptions {
                seed,
                ..Default::default()
            };
            let mut effect = Mandelbrot::new(options, (20, 8));
            let mut centres = Vec::new();
            for _ in 0..4 {
                effect.scale = MIN_SCALE * 0.5;
                effect.advance(1.0 / 60.0);
                centres.push(effect.center);
            }
            centres
        };

        assert_ne!(tour(7), tour(8), "two seeds produced the same tour");
    }

    #[test]
    fn surviving_a_resize() {
        let mut effect = Mandelbrot::new(MandelbrotOptions::default(), (40, 12));
        effect.update_size(6, 6);
        let diff = effect.get_diff();
        for (x, y, _) in &diff {
            assert!(*x < 6 && *y < 6);
        }
    }
}
