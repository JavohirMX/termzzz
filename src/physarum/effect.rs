use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::halfblock::{HalfBlockField, ROWS_PER_CELL};
use crate::render::palette::{Palette, presets as palette_presets};
use crate::runtime::FrameContext;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Weight of the trail each agent lays down per step.
///
/// Large relative to the field's dynamic range so a fresh trail is unambiguously
/// the top of the ramp, and the network that builds out of it is the only thing
/// at the top for the first few thousand steps.
const DEPOSIT: f32 = 5.0;

/// How much of the trail survives one diffusion step, at the shipped settings.
///
/// Just under one, and that is the whole of the model's memory. At 1.0 the trail
/// never fades and a field that has been walked once is walked for ever; too low
/// and a trail is gone before an agent can find it again, so the agents are
/// choosing between three cells that are all equally empty. See
/// [`PhysarumOptions::decay`] for the measurement.
const DECAY: f32 = 0.90;

/// Sideways spread at the shipped settings.
///
/// Small, and much smaller than the first version of this used, which is the
/// finding the whole parameter set turned on. See [`PhysarumOptions::spread`].
const SPREAD: f32 = 0.10;

/// The shipped colour depth of the ramp. See [`PhysarumOptions::levels`].
const DEFAULT_LEVELS: usize = 16;

/// The most simulation steps one frame may run, per unit of
/// [`PhysarumOptions::substeps`].
///
/// A bound on catch-up, not on the rate. See [`Physarum::advance`].
const MAX_CATCH_UP_STEPS: usize = 4;

/// The most trail a cell can hold.
///
/// Not a performance bound and not a display tweak: it is what makes the model
/// legible. An agent deposits on every step, so a cell it keeps returning to
/// accumulates without limit, and after a couple of minutes those few pile-ups are
/// twenty times the value of the network around them. Normalise the ramp by the
/// maximum and the network is drawn in the bottom fifth of the palette while a
/// handful of cells are white; normalise by anything else and the picture depends
/// on which statistic was chosen.
///
/// Measured over 2,400 steps on an 80x24 terminal at the shipped density, an
/// uncapped field peaks above 300 while the network itself sits between 5 and 30.
/// A ceiling of 64 puts the peak at the top of the ramp and the network across
/// the middle of it, which is the whole picture.
const TRAIL_CEILING: f32 = 64.0;

/// One agent, on the trail map.
#[derive(Debug, Clone, Copy)]
struct Agent {
    x: f32,
    y: f32,
    /// Radians.
    heading: f32,
}

impl Agent {
    /// Samples the trail at a point relative to the agent, wrapping.
    fn sense(
        &self,
        trail: &[f32],
        width: usize,
        height: usize,
        dx: f32,
        dy: f32,
    ) -> f32 {
        sample(trail, width, height, self.x + dx, self.y + dy)
    }

    fn advance(&mut self, step: f32, width: usize, height: usize) {
        self.x = wrap(self.x + step * self.heading.cos(), width);
        self.y = wrap(self.y + step * self.heading.sin(), height);
    }
}

/// Wraps a coordinate into `0..n`.
///
/// Written out rather than left to `rem_euclid` because that is the bug this
/// exists to prevent. `rem_euclid` can return *exactly* the modulus: for a value a
/// hair below zero, `(-1e-7).rem_euclid(48.0)` rounds to `48.0` rather than
/// `47.9999999`, and the `floor` that turns a wrapped coordinate into a cell index
/// then produces 48 -- one row past the end of a 48-row map. That is an index
/// panic, in release as well as debug, from a coordinate that was in range.
///
/// The sign fix is the usual one and the two clamps are the new part: they are
/// applied after the wrap, which is the only point at which the value is known to
/// be in range, so they are the only place it can be checked. The NaN guard is
/// there because a NaN falls through both comparisons and would otherwise be cast
/// to an arbitrary index.
#[inline]
fn wrap(value: f32, n: usize) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    let limit = n as f32;
    let wrapped = value % limit;
    let wrapped = if wrapped < 0.0 {
        wrapped + limit
    } else {
        wrapped
    };
    if wrapped >= limit || wrapped < 0.0 {
        0.0
    } else {
        wrapped
    }
}

/// Bilinear sample of the trail map, wrapping.
///
/// Bilinear rather than nearest because the sensor distance is a couple of cells at
/// the shipped settings, so a nearest sample makes two of the three sensors read
/// the *same* cell whenever the offset rounds to zero -- and an agent that cannot
/// distinguish its three sensors has nothing to choose between, and wanders. This
/// is the single detail that separates a network from noise in this model.
fn sample(trail: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    let fx = wrap(x, width);
    let fy = wrap(y, height);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let x1 = (x0 + 1) % width;
    let y1 = (y0 + 1) % height;

    let top = trail[y0 * width + x0] * (1.0 - tx) + trail[y0 * width + x1] * tx;
    let bottom = trail[y1 * width + x0] * (1.0 - tx) + trail[y1 * width + x1] * tx;
    top * (1.0 - ty) + bottom * ty
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PhysarumOptions {
    /// Agents on the map. Derived from the field size, so it is not read from
    /// the config file.
    #[serde(skip)]
    pub agents: u32,

    /// Multiplies the built-in agent density.
    pub agent_coeff: f32,

    /// How far apart the three sensors are, in cells.
    ///
    /// **The parameter that decides whether this effect works at all**, which is
    /// not what a person tuning it would guess -- they would reach for the angle.
    /// Measured on a 400x200 terminal after 4,000 steps, holding everything else
    /// fixed: at 1.5 the trail comes out as a few hundred separate worms, at 6 it
    /// is mostly connected, and from about 9 up it is a network.
    ///
    /// The reason is scale. The agents move one cell per step, so this is how many
    /// cells ahead they can see; too small and all three sensors sit inside the
    /// same trail cell, they read the same value, and the turn is a coin flip. The
    /// angle cannot rescue that, which is what the note on it says.
    pub sensor_distance: f32,

    /// The angle the three sensors are spread across, in degrees.
    ///
    /// The three sensors are symmetric, so this is the angle from the agent's
    /// heading to each *outer* sensor and the middle one sits on the heading.
    ///
    /// The *weak* parameter, and the docs here used to claim otherwise. Measured
    /// across 9 to 45 degrees at the shipped distance, the result moves between
    /// about a third and all of the marked trail being in one connected piece, with
    /// no trend -- while the same sweep on [`sensor_distance`](Self::sensor_distance)
    /// runs from a quarter to all of it with a clear direction. 22.5 is Jones'
    /// value and is kept because it is what the model is usually written with, not
    /// because the measurement here singles it out.
    pub sensor_angle: f32,

    /// How far an agent turns per step, in degrees.
    pub turn_angle: f32,

    /// How far an agent moves per step, in cells.
    pub move_step: f32,

    /// How much of the trail survives one diffusion step.
    ///
    /// Just under one, and that is the whole of the model's behaviour. At 1.0 the
    /// trail never fades and a field that has been walked once is walked for ever;
    /// at 0.90 and below a trail is gone before an agent can find it again, so no
    /// network ever forms, because the agents are choosing between three cells
    /// that are all equally empty. The window between those is narrow and it is
    /// the reason this effect looks the way it does.
    ///
    /// This was a constant and is a knob because it is the one number in the model
    /// worth arguing about: it sets how far an agent remembers, and therefore
    /// whether the answer is a network, a fog, or a slowly spreading stain.
    /// Measured over 4,000 steps on a 400x200 terminal, 0.90 leaves a filament
    /// network covering under one percent of the field, and 0.94 thickens it into
    /// long walls that sweep the screen rather than branching.
    pub decay: f32,

    /// How much the trail is spread sideways, from 0 (only fades) to 1 (fully
    /// blurred into its neighbours).
    ///
    /// This turned out to be the parameter that decides whether the picture reads
    /// as a network at all, which is not what the name suggests and was not what I
    /// expected. At 1.0 a trail spreads about a cell per step, and since it lives
    /// for some tens of steps, the diffusion length is several cells -- so veins
    /// merge into each other and the field fills in. Measured on a 400x200
    /// terminal at 4,000 steps: a full blur marks four percent of the field as one
    /// solid component, which is a slab with a network inside it rather than a
    /// network. At 0.1 the marked cells are under one percent and the largest piece
    /// has a boundary-to-area ratio near 0.9, which is what a vein looks like.
    pub spread: f32,

    /// Diffusion and movement passes per 1/60th of a second.
    ///
    /// A multiplier on the simulation's rate, not a quota of steps per *frame*.
    /// The first version capped the loop at `substeps` and reset the accumulator
    /// when it hit the cap, so a 20 fps terminal ran the same number of steps as a
    /// 60 fps one and the effect moved at the refresh rate -- which
    /// `effects_advance_using_the_frame_delta` in `tests/effect_contracts.rs`
    /// caught, and which is the same defect the crate's own docs describe for
    /// every effect that took it.
    pub substeps: u16,

    /// A named ramp from [`crate::render::palette`].
    pub palette: String,

    /// How many distinct colours the ramp is quantised to.
    ///
    /// **A performance lever, not a quality setting**, and for the same reason as
    /// `ripple`'s: the output path emits a colour only when it differs from the
    /// last one written, so a continuously interpolated field is a colour change
    /// in every cell. This effect's trail spreads a long way at a shallow value
    /// across much of the frame, and at the interpolated ramp it emitted 410 KB a
    /// frame and spent 0.83 ms encoding it -- the most expensive encode in the
    /// crate. Quantising makes the long faint tail of the trail one colour, which
    /// is both cheaper and more truthful: that region carries no structure.
    pub levels: usize,

    pub seed: u64,
}

impl Default for PhysarumOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default` produced
    /// zeros, and serde used the derived one, so a config file that omitted a
    /// section silently zeroed it.
    fn default() -> Self {
        Self {
            agents: DEFAULT_AGENT_COUNT,
            agent_coeff: 1.0,
            sensor_distance: 9.0,
            sensor_angle: 22.5,
            turn_angle: 45.0,
            move_step: 1.0,
            decay: DECAY,
            spread: SPREAD,
            substeps: 1,
            palette: "ocean".to_string(),
            levels: DEFAULT_LEVELS,
            seed: DEFAULT_SEED,
        }
    }
}

/// Agents per field cell before the coefficient.
///
/// Derived rather than configured because a constant is wrong at both ends: on a
/// 6x6 terminal a few hundred agents is a solid block, and one agent per hundred
/// cells is a single worm. It is also the number that decides the *character* of
/// the result and not only its density -- measured on a 400x200 terminal at 4,000
/// steps, 0.002 leaves a branching filament network, and 0.005 thickens the same
/// thing into long walls that sweep the screen, because at that density the agents
/// find each other's trails and reinforce them instead of branching away.
///
/// Per *field row*, not per cell: the trail map is at half-block resolution. See
/// [`Physarum::size`].
pub const PHYSARUM_AGENT_DENSITY: f32 = 0.002;

/// The floor and ceiling on [`PhysarumOptions::agents`].
///
/// The floor is what a small terminal gets. At the shipped density a 6x6 terminal
/// works out at under one agent, and a single agent grows one filament, which is a
/// much smaller picture than the same effect on a large one.
pub const MIN_AGENT_COUNT: u32 = 40;
pub const MAX_AGENT_COUNT: u32 = 30_000;

/// Used as the `agents` default so a struct built without a field size still has
/// a sensible number.
const DEFAULT_AGENT_COUNT: u32 = 320;

pub struct Physarum {
    screen_size: (u16, u16),
    options: PhysarumOptions,
    canvas: Canvas,
    field: HalfBlockField,
    /// The trail, on its own grid. Row-major, `width * height`.
    trail: Vec<f32>,
    /// Double buffer for the diffusion pass, which cannot read and write the same
    /// array: a stencil that updates in place propagates along the sweep order
    /// instead of blurring, which smears the trail in one direction and reads as a
    /// rendering fault rather than as a model.
    scratch: Vec<f32>,
    colony: Vec<Agent>,
    /// Elapsed time not yet spent on a diffusion step.
    accumulator: f32,
    /// Smoothed peak of the trail field, and what the ramp is stretched to.
    ///
    /// Self-calibrating rather than a constant, because the field's absolute
    /// scale is a product of the deposit, the decay and the substep count, all
    /// three of which are configurable. A fixed divisor would leave a reconfigured
    /// field either black or saturated, and neither looks like a mistake in the
    /// config -- it looks like a broken effect.
    peak: f32,
    rng: EffectRng,
    palette: Palette,
}

impl TerminalEffect for Physarum {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.draw();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.field
            .resize(self.screen_size.0 as usize, self.screen_size.1 as usize);
        let (width, height) = self.size();
        self.trail = vec![0.0; width * height];
        self.scratch = vec![0.0; width * height];
        self.accumulator = 0.0;
        self.peak = 1.0;
        self.rng = seeded_rng(self.options.seed, "physarum");
        self.spawn();
        self.seed_trail();
    }
}

impl Physarum {
    pub fn new(options: PhysarumOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let field =
            HalfBlockField::new(screen_size.0 as usize, screen_size.1 as usize);
        let (width, height) = (field.row_width(), field.row_height());

        // Expanded to `levels` discrete stops so the draw can *index* the ramp
        // rather than interpolate it. See `PhysarumOptions::levels`.
        let palette = Palette::new(
            Palette::new(
                palette_presets::by_name(&options.palette)
                    .unwrap_or(palette_presets::OCEAN)
                    .to_vec(),
            )
            .expand(options.levels.max(2)),
        );

        let mut physarum = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            trail: vec![0.0; width * height],
            scratch: vec![0.0; width * height],
            colony: Vec::new(),
            accumulator: 0.0,
            peak: 1.0,
            field,
            rng: seeded_rng(DEFAULT_SEED, "physarum"),
            palette,
            options,
        };
        physarum.rng = seeded_rng(physarum.options.seed, "physarum");
        physarum.spawn();
        physarum.seed_trail();
        physarum
    }

    /// The trail map's dimensions, as `(width, height)` in field rows.
    pub fn field_dimensions(&self) -> (usize, usize) {
        self.size()
    }

    /// The trail field, for inspection by a caller measuring the model.
    pub fn trail_view(&self) -> &[f32] {
        &self.trail
    }

    /// The trail map's size, which is the half-block row grid rather than the
    /// cell grid.
    ///
    /// One entry per field row, so the trail is at the full vertical resolution
    /// the renderer will use. Deliberately *not* the braille dot grid: a
    /// diffusion pass over 800x800 is 640,000 cells of stencil, which is the
    /// entire 2 ms frame budget for the crate, spent on a model whose structure is
    /// measured in tens of cells rather than in single dots. Halving the linear
    /// resolution in each axis would be four times cheaper again and lose the
    /// thin trails, so this is the compromise: half-block resolution rather than
    /// braille's, and one stencil per cell rather than per dot.
    fn size(&self) -> (usize, usize) {
        (
            self.field.row_width(),
            self.field.row_height().max(ROWS_PER_CELL),
        )
    }

    fn spawn(&mut self) {
        let (width, height) = self.size();
        self.colony = (0..self.options.agents.max(1))
            .map(|_| Agent {
                x: self.rng.random_range(0.0..width as f32),
                y: self.rng.random_range(0.0..height as f32),
                heading: self.rng.random_range(0.0..std::f32::consts::TAU),
            })
            .collect();
    }

    /// Lays a first trail, so there is something to follow on frame one.
    ///
    /// Without this the field is empty, every agent's three sensors read the same
    /// zero, and the opening of the effect is a few hundred agents turning at
    /// random on a blank screen for as long as it takes the first one to lay a
    /// trail worth following. A blob is what the model is normally seeded with, and
    /// the position is seeded so two seeds do not both grow from dead centre.
    fn seed_trail(&mut self) {
        let (width, height) = self.size();
        for _ in 0..5 {
            let cx = self.rng.random_range(0.0..width as f32);
            let cy = self.rng.random_range(0.0..height as f32);
            let radius: f32 = self.rng.random_range(6.0..18.0);
            let span = (radius * 2.0).ceil() as usize;
            for dy in 0..span {
                for dx in 0..span {
                    let px =
                        (cx - radius + dx as f32).rem_euclid(width as f32) as usize;
                    let py = (cy - radius + dy as f32).rem_euclid(height as f32)
                        as usize;
                    let distance = (px as f32 - cx).hypot(py as f32 - cy);
                    if distance <= radius {
                        self.trail[py * width + px] = DEPOSIT;
                    }
                }
            }
        }
    }

    /// Spends elapsed time on whole simulation steps.
    ///
    /// The accumulator is in **steps**, not seconds, and that is the whole of the
    /// timing. A frame that lasted three 1/60ths owes three steps; a frame that
    /// lasted one owes one. Capping the loop at a fixed number of steps per frame
    /// instead -- which is the obvious way to write it, and the first way here --
    /// makes the simulation rate a function of the frame rate, because the cap is
    /// reached at the same point in both cases and the leftover time is thrown
    /// away rather than carried.
    fn advance(&mut self, delta: f32) {
        let substeps = self.options.substeps.max(1) as f32;
        self.accumulator += delta * 60.0 * substeps;

        // A stall must not hand this one a backlog, and a bound of a few steps
        // turns a hitch into a brief pause rather than a jump. Four of them, so a
        // `substeps` of 2 can still do 8 in one frame.
        let cap =
            (MAX_CATCH_UP_STEPS * self.options.substeps.max(1) as usize).max(1);
        let mut steps = 0usize;
        while self.accumulator >= 1.0 && steps < cap {
            self.accumulator -= 1.0;
            self.step();
            steps += 1;
        }
        if steps == cap {
            self.accumulator = 0.0;
        }
    }

    /// One move, one deposit, one diffusion.
    ///
    /// Public so a caller can drive the model a step at a time and count the steps
    /// -- which is what the parameter sweep in `examples/physarum_sweep.rs` does,
    /// and what the tests do, since "after N steps the field is a network" is the
    /// only way to say anything useful about the model's parameters.
    pub fn step(&mut self) {
        let (width, height) = self.size();
        let sensor = self.options.sensor_distance;
        let spread = self.options.sensor_angle.to_radians();
        let turn = self.options.turn_angle.to_radians();

        for agent in &mut self.colony {
            // The three sensors: right, ahead, left.
            let right = agent.sense(
                &self.trail,
                width,
                height,
                sensor * (agent.heading + spread).cos(),
                sensor * (agent.heading + spread).sin(),
            );
            let ahead = agent.sense(
                &self.trail,
                width,
                height,
                sensor * agent.heading.cos(),
                sensor * agent.heading.sin(),
            );
            let left = agent.sense(
                &self.trail,
                width,
                height,
                sensor * (agent.heading - spread).cos(),
                sensor * (agent.heading - spread).sin(),
            );

            // Turn towards whichever sensor saw the most trail, and only turn
            // when that one is *strictly* better than going straight on. The
            // strictness is what makes the network: an agent that turns on a tie
            // wanders, and an agent that never turns on a tie follows its own trail
            // out of the field and back, which is a circle rather than a vein.
            if right > ahead && right > left {
                agent.heading += turn;
            } else if left > ahead && left > right {
                agent.heading -= turn;
            }

            agent.advance(self.options.move_step, width, height);

            let x = wrap(agent.x, width) as usize;
            let y = wrap(agent.y, height) as usize;
            let cell = &mut self.trail[y * width + x];
            *cell = (*cell + DEPOSIT).min(TRAIL_CEILING);
        }

        self.diffuse(width, height);
    }

    /// Blur and fade the trail, into the scratch buffer.
    ///
    /// The kernel is a mix between the centre and the mean of the four
    /// neighbours, scaled by [`PhysarumOptions::spread`] and then faded by
    /// [`PhysarumOptions::decay`]. Both halves are needed: without the fade the
    /// field saturates into a solid block within seconds, and without any blur at
    /// all the trail is a one-cell-wide line that the display cannot resolve into
    /// anything.
    fn diffuse(&mut self, width: usize, height: usize) {
        let spread = self.options.spread.clamp(0.0, 1.0);
        let decay = self.options.decay.clamp(0.0, 1.0);
        let centre = 1.0 - spread;
        let each = spread / 4.0;
        for y in 0..height {
            let up = (y + height - 1) % height;
            let down = (y + 1) % height;
            for x in 0..width {
                let left = (x + width - 1) % width;
                let right = (x + 1) % width;
                let total = self.trail[y * width + x] * centre
                    + (self.trail[y * width + left]
                        + self.trail[y * width + right]
                        + self.trail[up * width + x]
                        + self.trail[down * width + x])
                        * each;
                self.scratch[y * width + x] = (total * decay).min(TRAIL_CEILING);
            }
        }
        std::mem::swap(&mut self.trail, &mut self.scratch);
    }

    fn draw(&mut self) {
        let (width, height) = self.size();
        let mut frame_peak = f32::MIN;
        for value in &self.trail {
            frame_peak = frame_peak.max(*value);
        }
        if frame_peak > f32::MIN {
            // Lifted rather than tracked exactly: a field whose top value flickers
            // frame to frame would make the whole picture breathe, and the peak is
            // a calibration figure rather than something anyone is meant to see.
            self.peak += (frame_peak - self.peak) * 0.08;
        }
        let peak = self.peak.max(f32::MIN_POSITIVE);

        for y in 0..height {
            for x in 0..width {
                // Indexed, not interpolated. See `PhysarumOptions::levels`.
                let t = (self.trail[y * width + x] / peak).clamp(0.0, 1.0);
                let levels = self.palette.len().max(2);
                let index =
                    ((t * (levels - 1) as f32).round() as usize).min(levels - 1);
                let rgb = match self.palette.sample_index(index) {
                    style::Color::Rgb { r, g, b } => {
                        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0]
                    }
                    _ => [0.0, 0.0, 0.0],
                };
                self.field.set_row(x, y, rgb);
            }
        }

        self.field
            .write_to(&mut self.canvas, crossterm::style::Attribute::Reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn options(seed: u64) -> PhysarumOptions {
        PhysarumOptions {
            seed,
            ..PhysarumOptions::default()
        }
    }

    /// A dense enough board that the shipped density puts a useful number of
    /// agents on it, and small enough to grow a whole network in a test.
    ///
    /// 60x20 cells is 2,400 field rows, and the agent floor of 40 makes that
    /// denser than the shipped density would, which is deliberate: measured on this
    /// board the agents join their trails into three connected pieces after 4,000
    /// steps, while a 120x40 board at the same settings is still 28 separate
    /// fragments. The network wants room per agent, and a test board that is too
    /// large measures fragmentation rather than the model.
    fn fixture(seed: u64) -> Physarum {
        Physarum::new(options(seed), (60, 20))
    }

    /// Runs the model for a number of steps.
    fn run(physarum: &mut Physarum, steps: usize) {
        for _ in 0..steps {
            physarum.step();
        }
    }

    /// What the marked trail looks like: how much of it there is, how many
    /// separate pieces, how big the biggest is, and how ragged its edge is.
    ///
    /// The edge ratio is the part that catches the failure a connectivity number
    /// cannot. A slab and a network can both be a single connected component
    /// covering a similar area, and they are opposite pictures: a vein has almost
    /// every cell of it on the boundary, a filled region has almost none.
    struct Shape {
        marked: usize,
        components: usize,
        largest: usize,
        /// Boundary cells of the largest component, per cell of its own area. A
        /// hairline is near 1.0; a filled region is near 0.05.
        edge_ratio: f32,
    }

    fn shape(trail: &[f32], width: usize, height: usize, floor: f32) -> Shape {
        let above = |i: usize| trail[i] > floor;
        let mut seen = vec![false; width * height];
        let (mut marked, mut components, mut largest, mut best_ratio) =
            (0usize, 0usize, 0usize, 0.0f32);

        for start in 0..width * height {
            if seen[start] || !above(start) {
                continue;
            }
            components += 1;
            let mut cells = Vec::new();
            let mut queue = VecDeque::from([start]);
            seen[start] = true;
            while let Some(i) = queue.pop_front() {
                // Counted once per cell, here. An earlier version of this counted
                // the *start* of each component, which made `marked` a count of
                // components and produced figures like "the largest component is
                // 917 of 25 marked cells", which are not a thing.
                marked += 1;
                cells.push(i);
                let (x, y) = (i % width, i / width);
                for (nx, ny) in [
                    ((x + width - 1) % width, y),
                    ((x + 1) % width, y),
                    (x, (y + height - 1) % height),
                    (x, (y + 1) % height),
                ] {
                    let n = ny * width + nx;
                    if !seen[n] && above(n) {
                        seen[n] = true;
                        queue.push_back(n);
                    }
                }
            }
            if cells.len() > largest {
                largest = cells.len();
                let mut boundary = 0usize;
                for i in &cells {
                    let (x, y) = (i % width, i / width);
                    for (nx, ny) in [
                        ((x + width - 1) % width, y),
                        ((x + 1) % width, y),
                        (x, (y + height - 1) % height),
                        (x, (y + 1) % height),
                    ] {
                        if !above(ny * width + nx) {
                            boundary += 1;
                        }
                    }
                }
                best_ratio = boundary as f32 / cells.len() as f32;
            }
        }

        Shape {
            marked,
            components,
            largest,
            edge_ratio: best_ratio,
        }
    }

    /// The trail a grown field leaves, measured.
    ///
    /// The floor is absolute rather than a fraction of the peak, and that matters:
    /// the field's peak is a handful of cells where agents pile up, so a relative
    /// threshold sits above the network itself and measures only those piles.
    fn grown(seed: u64, steps: usize) -> (Shape, usize) {
        let mut physarum = fixture(seed);
        run(&mut physarum, steps);
        let (width, height) = physarum.size();
        (shape(&physarum.trail, width, height, 3.0), width * height)
    }

    #[test]
    fn the_agents_build_veins_rather_than_a_slab() {
        // The claim that matters, in the two forms that actually distinguish a
        // network from a filled region. A slab and a network can each be a single
        // connected component covering a similar area, so connectivity alone cannot
        // tell them apart -- which is exactly what happened while this was being
        // built: an earlier version of this test measured "one component, 100%
        // connected" on a picture that was three solid bands.
        //
        // 1. **Veined, not solid.** A filament has almost every cell of it on its
        //    own boundary; a filled region has almost none. Measured across board
        //    sizes from 60x20 to 400x200, veins came out between 0.40 and 1.56; a
        //    saturated slab measured 0.00. The bar is 0.30, far enough clear of a
        //    slab to be a real test and low enough not to be a test of one board.
        // 2. **Sparse.** Under a quarter of the field carrying trail. Measured
        //    0.6% to 9% for veins, and 100% for the slab.
        //
        // A third assertion -- a count of separate pieces, to say the network
        // *branches* -- was tried and dropped. That number swings from 1 to 28 with
        // the seed and the board size, so any threshold for it is a threshold for
        // one measurement rather than for the model.
        let (s, cells) = grown(3, 4_000);

        assert!(
            s.marked > 100,
            "only {} cells carry trail above the floor, out of {cells}: there is \
             no field here to be a network",
            s.marked
        );
        assert!(
            s.edge_ratio > 0.30,
            "the largest piece has an edge-to-area ratio of {:.2}, so it is a \
             filled region rather than a vein",
            s.edge_ratio
        );
        let coverage = s.marked as f32 / cells as f32;
        assert!(
            coverage < 0.25,
            "{:.1}% of the field carries trail, so the veins have merged into a \
             slab",
            coverage * 100.0
        );
    }

    #[test]
    fn a_sensor_distance_too_short_to_see_anything_cannot_build_a_network() {
        // The regression test for the calibration. The first version shipped a
        // sensor distance of 1.5 cells, which looks reasonable next to a one-cell
        // step, and produces isolated worms rather than a network: at that distance
        // all three sensors fall inside one trail cell, read the same value, and
        // the turn is a coin flip.
        //
        // The discriminating measure is the *count* of pieces and the size of the
        // biggest, not connectivity. Measured on a 200x60 board after 4,000 steps:
        // 9 cells sees 25 pieces with a largest of 60, and 1.5 cells sees 82
        // pieces with a largest of 9. Connectivity share is the wrong number here --
        // at 1.5 there is so little trail that a handful of small fragments can
        // score a high share by accident, which is what made the first version of
        // this test pass against the broken setting.
        let measure = |distance: f32| {
            let mut physarum = Physarum::new(
                PhysarumOptions {
                    sensor_distance: distance,
                    ..options(5)
                },
                (200, 60),
            );
            run(&mut physarum, 4_000);
            let (width, height) = physarum.size();
            shape(&physarum.trail, width, height, 3.0)
        };
        let good = measure(9.0);
        let bad = measure(1.5);

        assert!(
            good.components * 3 < bad.components,
            "9 cells of sensor distance left {} pieces; 1.5 left {}, which is not \
             enough of a difference to be the network",
            good.components,
            bad.components
        );
        assert!(
            good.largest * 3 > bad.largest,
            "the biggest connected piece was {} cells at 9 and {} at 1.5",
            good.largest,
            bad.largest
        );
    }

    #[test]
    fn the_network_survives_a_long_run() {
        // The model has three failure modes and this catches the two that look
        // like "there is a picture": too little decay and the trail vanishes, too
        // much and it saturates. Both are invisible to a test that only checks the
        // field is non-empty, and both show up as one or the other of the measures
        // `the_agents_build_veins_rather_than_a_slab` uses.
        let (s, cells) = grown(4, 20_000);
        assert!(
            s.marked > 100,
            "after twenty thousand steps only {} cells carry trail: the network \
             has decayed away",
            s.marked
        );
        assert!(
            s.edge_ratio > 0.30,
            "after twenty thousand steps the largest piece is a filled region \
             (edge ratio {:.2})",
            s.edge_ratio
        );
        assert!(
            s.marked * 4 < cells,
            "after twenty thousand steps {:.1}% of the field carries trail, so \
             the trail has saturated",
            s.marked as f32 / cells as f32 * 100.0
        );
    }

    #[test]
    fn the_diffusion_does_not_smear_along_the_sweep_order() {
        // The in-place version of a stencil pass propagates along the iteration
        // order rather than blurring, which is invisible in a still and obvious in
        // motion: a diagonal smear across the field. Compared against the
        // symmetric case, where a single cell's value must stay at the centre and
        // its two horizontal neighbours must come out equal.
        let (width, height) = (5usize, 5usize);
        let mut physarum = Physarum::new(options(6), (6, 4));
        physarum.trail = vec![0.0; width * height];
        physarum.scratch = vec![0.0; width * height];
        physarum.trail[2 * width + 2] = 1.0;

        physarum.diffuse(width, height);

        assert!(
            physarum.trail[2 * width + 1] == physarum.trail[2 * width + 3],
            "the two cells beside the source came out at {} and {}, which only an \
             in-place stencil would do",
            physarum.trail[2 * width + 1],
            physarum.trail[2 * width + 3]
        );
        assert!(
            physarum.trail[2 * width + 2] > physarum.trail[2 * width + 1],
            "the source cell came out weaker than the cell beside it, so the \
             kernel is not weighted towards the centre"
        );
    }

    #[test]
    fn the_trail_actually_fades() {
        // Without this the model has no "forget" and a field that has been walked
        // once is walked for ever, which fills the screen in a minute.
        let mut physarum = fixture(7);
        physarum.colony.clear();
        physarum.trail.fill(0.0);
        let (width, _) = physarum.size();
        physarum.trail[5 * width + 5] = 10.0;

        physarum.step();

        let after = physarum.trail[5 * width + 5];
        assert!(
            after < 10.0 && after > 0.0,
            "a lone cell came out as {after}; it should have spread and faded"
        );
    }

    #[test]
    fn the_trail_is_capped_so_a_few_pile_ups_cannot_own_the_palette() {
        // An agent deposits on every step, so the cells it keeps returning to
        // accumulate without limit. Left uncapped the peak measured over 300
        // while the network sat between 5 and 30, and normalising the ramp by the
        // peak draws the network in the bottom fifth of the palette.
        let mut physarum = fixture(8);
        let (width, _) = physarum.size();
        let index = 7 * width + 7;
        for _ in 0..500 {
            physarum.colony.clear();
            physarum.colony.push(Agent {
                x: 7.0,
                y: 7.0,
                heading: 0.0,
            });
            physarum.step();
        }
        assert!(
            physarum.trail[index] <= TRAIL_CEILING,
            "a cell visited five hundred times came out at {}",
            physarum.trail[index]
        );
        assert!(
            physarum.trail[index] > 0.0,
            "the capped cell is empty, so the cap is clamping the wrong side"
        );
    }

    #[test]
    fn an_agent_at_the_very_edge_of_the_field_does_not_index_past_it() {
        // `rem_euclid` can return exactly the modulus for a value a hair below
        // zero -- `(-1e-7).rem_euclid(48.0)` rounds to 48.0, not 47.9999999 -- and
        // the floor that turns a wrapped coordinate into a cell index then
        // produces 48, one row past the end. That is an index panic, in release as
        // well as debug, from a coordinate that was in range.
        let mut physarum = fixture(9);
        let (width, height) = physarum.size();
        for y in [0.0f32, height as f32 - 0.5, -0.0000001, -1.0] {
            for x in [0.0f32, width as f32 - 0.5, -0.0000001, -1.0] {
                assert!(sample(&physarum.trail, width, height, x, y).is_finite());
            }
        }

        // And through a whole step, with the colony on the boundary.
        physarum.colony = vec![Agent {
            x: 0.0,
            y: 0.0,
            heading: -std::f32::consts::FRAC_PI_4,
        }];
        for _ in 0..50 {
            physarum.step();
        }
    }

    #[test]
    fn different_seeds_grow_different_networks() {
        let mut a = fixture(11);
        let mut b = fixture(12);
        run(&mut a, 400);
        run(&mut b, 400);
        assert_ne!(a.trail, b.trail, "the seed changed nothing");
    }

    #[test]
    fn the_same_seed_grows_the_same_network() {
        let mut a = fixture(13);
        let mut b = fixture(13);
        run(&mut a, 400);
        run(&mut b, 400);
        assert_eq!(a.trail, b.trail, "the same seed produced two pictures");
    }

    #[test]
    fn the_display_peak_follows_the_field_rather_than_a_constant() {
        // The ramp is stretched to a smoothed peak, so a reconfigured deposit and
        // decay cannot leave the picture black or saturated. A hardcoded divisor
        // would, and it would look like a broken effect rather than like a config
        // that means something different.
        let mut physarum = fixture(15);
        run(&mut physarum, 300);
        physarum.draw();
        let first = physarum.peak;
        run(&mut physarum, 300);
        physarum.draw();

        assert!(first > 0.0, "the display peak never left zero");
        assert!(
            (physarum.peak - first).abs() < first,
            "the display peak moved from {first} to {}, which is not tracking it",
            physarum.peak
        );
    }

    #[test]
    fn every_emitted_cell_is_inside_the_canvas() {
        let mut physarum = fixture(16);
        for _ in 0..60 {
            physarum.advance(1.0 / 60.0);
            for (x, y, _) in physarum.get_diff() {
                assert!(x < 60 && y < 20, "({x}, {y}) is outside a 60x20 canvas");
            }
        }
    }
}
