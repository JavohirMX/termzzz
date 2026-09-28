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

/// The trail value above which a cell counts as carrying trail.
///
/// 3.0, absolute rather than a fraction of the peak, and the reason is the same
/// one the tests give: the field's peak is a handful of cells where agents pile
/// up, so a relative floor sits above the network itself and measures only those
/// piles. It is also the floor the convergence check uses, and the two agreeing
/// is deliberate -- "settled" is measured on the same set of cells the shape tests
/// measure, or the effect can satisfy one and not the other.
const TRAIL_FLOOR: f32 = 3.0;

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

/// Decay the field is annealed *to*, and the ceiling it must not pass.
///
/// 0.995, and this is the number that makes the model settle at all. The shipped
/// `DECAY` of 0.90 does not converge: it is a steady state in the *statistical*
/// sense -- the same coverage, the same statistics, every thirty steps -- but not
/// a fixed point, because the diffusion keeps eroding veins that the agents keep
/// rebuilding, slightly elsewhere each time.
///
/// Measured churn (the fraction of marked cells that changed marked-state in
/// thirty steps) at 4,000 steps, seed 3:
///
/// ```text
/// decay    60x20    200x50   400x200
/// 0.90     0.35     0.41     1.40
/// 0.96     0.25     0.25     0.55
/// 0.98     0.10     0.06     0.28
/// 0.99     0.03     0.07     0.14
/// ```
///
/// A churn of 0.35 is a picture redrawing a third of itself twice a second, which
/// is not stability and does not look like it. Raising decay gets there, at the
/// cost of coverage -- at 0.99 permanently the 60x20 board is 46% covered, past
/// the 25% that `the_agents_build_veins_rather_than_a_slab` rejects as a slab.
/// Which is why the anneal exists as a *phase* rather than a new default.
const ANNEALED_DECAY: f32 = 0.995;

/// Steps over which decay ramps from the configured value to [`ANNEALED_DECAY`].
///
/// 2,500, which is about 42 seconds at one step per frame. Long enough that the
/// thickening is not a visible event, and short enough that the hold still comes
/// within a screensaver's attention span on a small terminal.
const ANNEAL_STEPS: u64 = 2_500;

/// How often convergence is measured, in steps.
const CHURN_EVERY: u32 = 30;

/// Churn at or below which the field counts as settled.
///
/// 0.15, and it is a measured band rather than a round number. The annealed model
/// settles at 0.067 to 0.109 across board sizes from 60x20 to 400x200, and the
/// un-annealed model sits at 0.35 and above. 0.15 is inside the gap, closer to the
/// top than the bottom so a board that settles a little slower is not held
/// forever. The cost of it being too *low* is the effect never holding, which is
/// the failure this whole feature is about.
const CHURN_SETTLED: f32 = 0.15;

/// Coverage below which a field is never called settled, as a fraction of the
/// field.
///
/// 0.002, so 640 of 320,000 field rows at 400x200. **The half of the convergence
/// test that carries the weight**, and for the same reason
/// `the_agents_build_veins_rather_than_a_slab` needs an edge ratio: churn alone is
/// satisfiable by its opposite. An empty field has zero churn and is perfectly
/// "converged", so a detector reading churn alone would declare victory the
/// instant the agents died, or on a resize before the first step, and then hold a
/// blank screen for ever.
const MIN_SETTLED_COVERAGE: f32 = 0.002;

/// Seconds the finished network is held before the field starts to dim.
///
/// Four, and the point of a hold is that the picture is *still*. A hold long
/// enough to notice would be one during which the terminal could be switched off
/// and back on and the effect would be indistinguishable from a still image; four
/// is long enough to read as a held image and short enough that the fade is
/// clearly the next thing that happens.
const DEFAULT_HOLD_SECONDS: f32 = 4.0;

/// Seconds the field takes to dim to nothing before it is re-seeded.
const FADE_SECONDS: f32 = 1.5;

/// Where the effect is in its grow, hold, fade cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Laying trail, with decay annealed upward.
    Growing,
    /// Converged and frozen. No simulation runs, so the frame is identical to the
    /// last one and the diff is empty.
    Holding,
    /// Dimming towards nothing, after which the field is re-seeded.
    Fading,
}

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

    /// Let the network settle, hold, fade, and grow again.
    ///
    /// **On by default, and the reason is in `ANNEALED_DECAY`.** Without it this
    /// model never reaches a fixed point: it sits at a steady state statistically
    /// and never stops redrawing a third of its own marked cells every half
    /// second, forever. Which for a screensaver means the thing is still busy
    /// after an hour, and a screensaver that never rests is a monitor decoration.
    ///
    /// Turning it off restores the previous behaviour exactly -- the anneal and
    /// the cycle both hang off this flag, and `step` is untouched either way, so
    /// a caller driving the model directly sees the same model as before.
    pub settle: bool,

    /// Seconds to hold the finished network before fading.
    ///
    /// See [`FADE_SECONDS`] for the other half of the cycle. A hold is what makes
    /// the effect read as having *finished* something: without it the fade starts
    /// while the network is still visibly forming, and the picture never resolves.
    pub hold_seconds: f32,

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
            settle: true,
            hold_seconds: DEFAULT_HOLD_SECONDS,
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
    /// Where the effect is in its grow, hold, fade cycle. See [`Phase`].
    phase: Phase,
    /// Simulation steps since the field was last re-seeded. Drives the anneal.
    age_steps: u64,
    /// Decay used by the *next* diffusion pass.
    ///
    /// A field rather than a read of `options.decay`, because the anneal needs the
    /// original value to ramp from and `options` is what the user configured; if
    /// the anneal wrote back into `options` then re-seeding could not tell the
    /// configured decay from the annealed one, and every cycle would start from
    /// wherever the last one stopped.
    decay: f32,
    /// The marked set from the last convergence check, for differencing.
    ///
    /// Reused rather than rebuilt: at 400x200 this is 320,000 entries and the
    /// check runs every 30 steps, so allocating it per check would put a
    /// 320 KB allocation in the frame budget twice a second.
    marked: Vec<bool>,
    /// True once [`marked`](Self::marked) holds a real measurement, so the first
    /// check has something to difference against.
    marked_valid: bool,
    /// Seconds left in the current phase's timer.
    phase_timer: f32,
    /// Multiplier on the drawn trail, which is what the fade turns down.
    fade: f32,
    /// Churn from the most recent convergence check, for inspection.
    ///
    /// Stored rather than recomputed on demand because it costs a full pass over
    /// the field to measure, and a caller that wants to know how close the model
    /// is to settling should not have to pay that to find out -- nor should a test
    /// have to reimplement the measurement to check that the anneal works.
    last_churn: Option<f32>,
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
        self.marked = vec![false; width * height];
        self.accumulator = 0.0;
        self.peak = 1.0;
        self.rng = seeded_rng(self.options.seed, "physarum");
        self.start_growing();
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
        //
        // Truncated before expanding, and the order matters: `expand` samples
        // across the whole ramp, so truncating afterwards would leave the
        // ascending half's stops spread over the first half of the table and the
        // descending half's over the second. `OCEAN` is mirrored for the cycling
        // sampler, and taken whole it drew the *peak* of the field -- the
        // brightest cell on screen -- in near-black navy, with a pale band in the
        // middle where a mid-height trail happened to land. See
        // `Palette::truncated_at_peak`.
        let palette = Palette::new(
            Palette::new(
                palette_presets::by_name(&options.palette)
                    .unwrap_or(palette_presets::OCEAN)
                    .to_vec(),
            )
            .truncated_at_peak()
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
            phase: Phase::Growing,
            age_steps: 0,
            decay: options.decay.clamp(0.0, 1.0),
            marked: vec![false; width * height],
            marked_valid: false,
            phase_timer: 0.0,
            fade: 1.0,
            last_churn: None,
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

    /// Puts the effect back at the start of a growth, without re-seeding.
    ///
    /// Called from `reset` and from the end of a fade. The annealed decay is put
    /// back to the configured one *here* rather than left wherever the last cycle
    /// finished, because that is the whole reason `decay` is a field: a cycle that
    /// started from 0.995 would never grow a network again.
    fn start_growing(&mut self) {
        self.phase = Phase::Growing;
        self.age_steps = 0;
        self.decay = self.options.decay.clamp(0.0, 1.0);
        self.marked_valid = false;
        self.phase_timer = 0.0;
        self.fade = 1.0;
        self.last_churn = None;
    }

    /// The decay for the current point in the anneal.
    ///
    /// A linear ramp, clamped at both ends, so the field spends the first
    /// `ANNEAL_STEPS` at the configured decay and the rest of its life at
    /// [`ANNEALED_DECAY`]. Linear because anything cleverer is a curve to
    /// calibrate and the measurement only says the endpoints.
    fn annealed_decay(&self) -> f32 {
        let start = self.options.decay.clamp(0.0, 1.0);
        if self.age_steps >= ANNEAL_STEPS {
            return ANNEALED_DECAY.min(1.0);
        }
        let t = self.age_steps as f32 / ANNEAL_STEPS as f32;
        start + (ANNEALED_DECAY - start) * t
    }

    /// The cycle, which lives in `advance` rather than in `step`.
    ///
    /// `step` is a pure simulation step and stays one: it is `pub`, and both the
    /// parameter sweep in `examples/physarum_sweep.rs` and the tests drive it
    /// directly to count steps. Putting the hold here instead of there means a
    /// caller asking for a thousand steps gets a thousand steps of the model they
    /// already know, and the effect's *behaviour over time* is the frame path's
    /// business rather than the model's.
    ///
    /// Three phases, and the third one is not optional. `Growing` simulates and
    /// anneals; on convergence it becomes `Holding`, which simulates nothing and so
    /// emits an empty diff; after the hold it becomes `Fading`, which dims; and
    /// then the field is re-seeded. A screensaver that reaches `Holding` and stops
    /// there is a still picture, which is a fine thing to have *seen* and a poor
    /// thing to be left looking at.
    fn advance(&mut self, delta: f32) {
        match self.phase {
            Phase::Growing => {}
            Phase::Holding => {
                self.phase_timer -= delta;
                if self.phase_timer <= 0.0 {
                    self.phase = Phase::Fading;
                    self.phase_timer = FADE_SECONDS;
                }
                return;
            }
            Phase::Fading => {
                self.phase_timer -= delta;
                // Down to nothing and no further: a negative multiplier would put
                // the ramp's bright end at the top of the palette, so the field
                // would flash bright on its way out.
                self.fade = (self.phase_timer / FADE_SECONDS).clamp(0.0, 1.0);
                if self.phase_timer <= 0.0 {
                    self.reseed();
                }
                return;
            }
        }

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

            // The cycle's bookkeeping, here and not in `step`. It was in `step`
            // first, and it broke three tests that call `step` directly --
            // including `the_agents_build_veins_rather_than_a_slab`, whose edge
            // ratio fell to 0.10 because the anneal had thickened the field into
            // the slab that test exists to reject. Those tests, and
            // `examples/physarum_sweep.rs`, drive the *model*, and the model is
            // what they were written to measure. The cycle is a decision about
            // what the effect does over time, and it belongs to the path that owns
            // elapsed time.
            self.age_steps = self.age_steps.saturating_add(1);
            if self.options.settle {
                self.decay = self.annealed_decay();
                if self.age_steps % u64::from(CHURN_EVERY) == 0 {
                    self.check_convergence();
                    // One convergence check per frame is plenty, and stopping here
                    // means a `substeps` of 4 does not check four times on the
                    // frame that happens to land on the interval.
                    if self.phase != Phase::Growing {
                        break;
                    }
                }
            }
            steps += 1;
        }
        if steps == cap {
            self.accumulator = 0.0;
        }
    }

    /// Whether the field has stopped changing, and how much of it did.
    ///
    /// Churn is the fraction of *marked* cells whose marked-state changed since the
    /// last check -- not the fraction of the field, so a sparse network is judged
    /// on its own structure rather than on how much of the screen is empty. A
    /// network is a small thing moving on a large field, and measuring it against
    /// the field would say a settled network on a 400x200 terminal is 3% changed,
    /// which is the same number as a violently churning one.
    ///
    /// The coverage clause is the other half and is not optional; see
    /// [`MIN_SETTLED_COVERAGE`].
    fn check_convergence(&mut self) -> Option<f32> {
        let (width, height) = self.size();
        let mut marked = 0usize;
        let mut changed = 0usize;
        for (index, value) in self.trail.iter().enumerate() {
            let now = *value > TRAIL_FLOOR;
            if now {
                marked += 1;
            }
            if self.marked_valid && self.marked[index] != now {
                changed += 1;
            }
            self.marked[index] = now;
        }
        self.marked_valid = true;

        // The first check has nothing to difference against, so it establishes the
        // baseline rather than reporting a churn of zero -- which would be a field
        // declaring itself settled on the strength of a measurement it never took.
        if changed == 0 && marked == 0 {
            return None;
        }
        let churn = changed as f32 / marked.max(1) as f32;
        let coverage = marked as f32 / (width * height).max(1) as f32;
        self.last_churn = Some(churn);

        // **Nothing is called settled before the anneal is over.** Without this the
        // detector fires on the first check, at step 30, because the field
        // `seed_trail` lays is already stationary: five blobs that nothing has
        // disturbed yet have a churn of zero, and their coverage is comfortably
        // over the floor. So the effect would announce a finished network half a
        // second after starting, hold five blobs, and only then begin growing --
        // and it did, which is how this was found.
        //
        // The bound is [`ANNEAL_STEPS`] rather than some smaller "long enough to
        // be sure" number because the anneal is the thing that makes a fixed point
        // reachable at all. There is no point looking for one before finishing the
        // work that produces it.
        if self.age_steps < ANNEAL_STEPS {
            return Some(churn);
        }

        if churn <= CHURN_SETTLED && coverage >= MIN_SETTLED_COVERAGE {
            self.phase = Phase::Holding;
            self.phase_timer = self.options.hold_seconds.max(0.0);
        }
        Some(churn)
    }

    /// Clears the field and starts a fresh growth.
    fn reseed(&mut self) {
        self.trail.iter_mut().for_each(|value| *value = 0.0);
        self.scratch.iter_mut().for_each(|value| *value = 0.0);
        self.start_growing();
        self.spawn();
        self.seed_trail();
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
        // The annealed decay, not the configured one. See `Physarum::decay` and
        // `ANNEALED_DECAY` for why this is the difference between a model that
        // settles and one that redraws a third of itself twice a second for ever.
        let decay = self.decay.clamp(0.0, 1.0);
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
                //
                // `fade` is the fade-out, and it scales the *value* rather than
                // the drawn colour so the whole ramp dims together -- dimming the
                // colour instead would slide every cell down the ramp, so a fade
                // would walk the network from cyan to navy through four steps
                // rather than getting darker.
                let t =
                    (self.trail[y * width + x] * self.fade / peak).clamp(0.0, 1.0);
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
    use crate::render::palette::luminance;
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

    /// The colour the renderer would give a cell at `t` of the field's scale.
    fn colour_at(physarum: &Physarum, t: f32) -> style::Color {
        let levels = physarum.palette.len().max(2);
        let index = ((t * (levels - 1) as f32).round() as usize).min(levels - 1);
        physarum.palette.sample_index(index)
    }

    /// The brightest cell on screen must be the brightest colour available.
    ///
    /// The renderer normalises the field by its own smoothed peak, so the cell
    /// holding the peak always draws at `t = 1.0` -- the top of the scale, by
    /// construction. So the top of the scale has to *be* the ramp's brightest
    /// colour, and before the ramp was truncated it was not: `OCEAN` is mirrored
    /// for the cycling sampler, `sample_index` walks the stops in order, and the
    /// last stop is `(0, 10, 50)`, a near-black navy. The field's own peak was
    /// being drawn in the darkest colour in the palette, with a pale band where a
    /// mid-height trail landed, which inverts the reading of the picture: the
    /// thickest, most reinforced vein was the dimmest thing on screen.
    ///
    /// Asserted on the colour rather than on the count, because "the top index is
    /// the last stop" is a tautology. What is being claimed is that the last stop
    /// is the brightest one, and that is a property of which ramp was handed in.
    #[test]
    fn the_peak_of_the_field_is_drawn_in_the_brightest_colour() {
        let physarum = fixture(3);
        let top = colour_at(&physarum, 1.0);
        let levels = physarum.palette.len();
        for index in 0..levels {
            let other = physarum.palette.sample_index(index);
            assert!(
                luminance(top) >= luminance(other) - 1e-4,
                "t = 1.0 drew at {top:?} but stop {index} is {other:?}, which is \
                 brighter"
            );
        }
        // And the contrast is real rather than a rounding difference.
        assert!(
            luminance(top) > 0.5,
            "the top of the scale is {top:?}, luma {:.3}, which is not a bright \
             colour",
            luminance(top)
        );
    }

    /// The peak of a real, grown field is what the test above needs a peak of.
    ///
    /// Without this, `the_peak_of_the_field_is_drawn_in_the_brightest_colour`
    /// could pass on a field that never grew anything, and would then be measuring
    /// the palette rather than the model. The floor is the same absolute 3.0 the
    /// shape helper uses, for the reason recorded there: the field's peak is a
    /// handful of pile-up cells, so a threshold relative to it would sit above
    /// the network itself.
    #[test]
    fn a_grown_field_has_a_cell_at_the_very_top_of_the_scale() {
        let mut physarum = fixture(3);
        run(&mut physarum, 4_000);
        let (width, height) = physarum.size();
        let peak = physarum
            .trail
            .iter()
            .copied()
            .fold(0.0f32, f32::max)
            .max(f32::MIN_POSITIVE);
        let at_peak = physarum
            .trail
            .iter()
            .filter(|value| **value >= peak * 0.99)
            .count();
        assert!(
            at_peak > 0,
            "no cell reaches the peak of {peak} on a {width}x{height} field, so \
             the scale is never exercised at its top"
        );
    }

    /// Drives the effect through its frame path rather than through `step`.
    ///
    /// This is the difference the settle cycle turns on: `step` is a simulation
    /// step and does not know about phases, so a test that wants to watch the
    /// effect settle has to go through `advance` or it will never see a hold
    /// happen no matter how long it runs.
    fn run_frames(physarum: &mut Physarum, seconds: f32) {
        let frame = 1.0 / 60.0;
        let mut left = (seconds * 60.0).round() as usize;
        while left > 0 {
            physarum.update();
            left -= 1;
        }
        let _ = frame;
    }

    /// The field settles, holds, and starts again.
    ///
    /// The whole feature in one test: from a fresh field, the effect must reach
    /// `Holding` on its own, stay there for the configured hold, and then leave.
    ///
    /// The claim is deliberately about the *phase* rather than about a picture. A
    /// picture claim -- "the trail stops changing" -- is what the churn measure is
    /// for, and asserting it here as well would be asserting the same arithmetic
    /// twice. What this adds is that the machinery is wired to the frame path at
    /// all, which is the thing a caller would notice if it were not.
    #[test]
    fn the_network_settles_holds_and_then_grows_again() {
        let mut physarum = Physarum::new(options(3), (60, 20));

        // Long enough for the anneal to complete and the churn to fall, in frames
        // rather than steps. 120 seconds of simulated time.
        let mut held_at = None;
        for frame in 0..7_200 {
            physarum.update();
            if physarum.phase == Phase::Holding && held_at.is_none() {
                held_at = Some(frame);
            }
        }

        let held_at = held_at.expect(
            "the field never reached Holding in 120 seconds, so the anneal did not \
             bring the churn down far enough for the detector to fire",
        );
        // 2,500 anneal steps is 42 seconds, and the churn needs to fall after that,
        // so a hold before two minutes means the threshold is far too loose.
        assert!(
            held_at > 2_400,
            "it settled after only {held_at} frames, which is inside the anneal \
             itself"
        );

        // And it must come back. A screensaver that reaches the hold and stops is
        // a still picture, which is worth seeing once and a poor thing to be left
        // looking at.
        run_frames(&mut physarum, 12.0);
        assert_eq!(
            physarum.phase,
            Phase::Growing,
            "after the hold and the fade the effect should be growing again"
        );
        assert!(
            physarum.age_steps < ANNEAL_STEPS,
            "the new cycle started at age {} steps, so the anneal will not run \
             again",
            physarum.age_steps
        );
    }

    /// A held field emits nothing at all, which is the cheapest still picture there is.
    ///
    /// `Canvas::commit` diffs, and holding runs no simulation, so two frames of a
    /// held field are byte-identical and the second is empty. This is worth
    /// asserting separately from the phase test because it is the *reason* a hold
    /// is worth having, and it is the thing that would break first if the hold
    /// ever came to advance a counter.
    #[test]
    fn a_held_field_is_drawn_once_and_then_not_at_all() {
        let mut physarum = Physarum::new(options(3), (60, 20));
        // Straight to the hold, rather than simulating 100 seconds to get there.
        physarum.phase = Phase::Holding;
        physarum.phase_timer = 10.0;
        physarum.fade = 1.0;

        let first = physarum.get_diff();
        assert!(
            !first.is_empty(),
            "the first frame of a hold painted nothing, so there is no picture to \
             hold"
        );
        for _ in 0..30 {
            assert!(
                physarum.get_diff().is_empty(),
                "a held field still emitted cells, so it is not actually frozen"
            );
        }
    }

    /// An empty field is never called settled, and neither is one that has not
    /// started.
    ///
    /// Two ways the measure can be satisfied by nothing, both found by running it.
    ///
    /// An **empty** field has zero churn. It is trivially "converged" by the churn
    /// half alone, and a detector reading only churn would declare victory the
    /// instant the agents stopped depositing -- on a board that had just been
    /// resized, or one where `sensor_distance` is too short for anything to grow
    /// -- and then hold a blank screen for ever.
    ///
    /// A field that has **not started** is the same trap from the other side. The
    /// five blobs `seed_trail` lays are stationary: nothing has disturbed them
    /// yet, so their churn is zero and their coverage is well over the floor. The
    /// detector fired on the first check at step 30 and held five blobs for four
    /// seconds before growing anything at all.
    ///
    /// So the opposite is constructed rather than argued about, once for each: a
    /// field with no trail in it, and a field that has run but is still early.
    #[test]
    fn an_empty_field_is_never_called_settled() {
        let mut physarum = Physarum::new(options(3), (60, 20));
        physarum.trail.iter_mut().for_each(|value| *value = 0.0);
        physarum.marked_valid = true;
        physarum.marked.iter_mut().for_each(|mark| *mark = false);
        physarum.phase = Phase::Growing;

        // Well past the first check, so the "not enough of a measurement" path
        // cannot be what is being reported.
        for _ in 0..600 {
            physarum.update();
        }
        assert_eq!(
            physarum.phase,
            Phase::Growing,
            "a field with no trail in it was called settled"
        );
    }

    /// A field that has only just started is not settled, however stationary it is.
    ///
    /// The regression test for the bug above, kept separate because it has a
    /// different cause and would be fixed differently: the empty-field guard is
    /// about coverage, this one is about *time*, and adding a coverage clause does
    /// nothing about it. A freshly seeded field is fully covered and completely
    /// still.
    #[test]
    fn a_field_that_has_not_finished_growing_is_not_settled() {
        let mut physarum = Physarum::new(options(3), (60, 20));
        // Just under the first check, and then a little past it. The churn here is
        // genuinely near zero -- the field is five undisturbed blobs.
        for _ in 0..(CHURN_EVERY as usize * 4) {
            physarum.update();
            assert_eq!(
                physarum.phase,
                Phase::Growing,
                "the effect held a network it had barely started growing"
            );
        }
    }

    /// An un-annealed field is not settled either, so the threshold has a floor.
    ///
    /// The counterpart to the empty field. Without it a threshold of, say, 0.9
    /// would pass the empty-field test and also pass this one, because the shipped
    /// model's churn is 0.35 to 1.4 -- anything above 1.4 is not a test at all.
    #[test]
    fn the_shipped_decay_alone_never_settles() {
        let mut physarum = Physarum::new(
            PhysarumOptions {
                settle: false,
                ..options(3)
            },
            (60, 20),
        );
        for _ in 0..7_200 {
            physarum.update();
        }
        assert_eq!(
            physarum.phase,
            Phase::Growing,
            "settle = false still reached a hold, so the flag is not wired up"
        );
    }

    /// The anneal actually lowers the churn, which is the whole reason it exists.
    ///
    /// Everything else in this file assumes the anneal works. If it silently
    /// stopped -- a constant went back to the configured decay, a division
    /// inverted -- the settle test would fail, but with a message about timing
    /// rather than about the model, and the first thing anyone would go looking at
    /// is the hold. This measures the churn directly, on both sides.
    #[test]
    fn the_anneal_lowers_the_churn_it_exists_to_lower() {
        // The same field, grown two ways.
        let churn_of = |settle: bool| -> f32 {
            let mut physarum = Physarum::new(
                PhysarumOptions {
                    settle,
                    ..options(3)
                },
                (60, 20),
            );
            let mut last = f32::INFINITY;
            // Long enough to be past the anneal, which is 2,500 steps.
            for _ in 0..9_000 {
                physarum.update();
                if let Some(churn) = physarum.last_churn {
                    last = churn;
                }
                if physarum.phase != Phase::Growing {
                    break;
                }
            }
            last
        };

        let annealed = churn_of(true);
        let shipped = churn_of(false);
        assert!(
            annealed < CHURN_SETTLED,
            "with the anneal the churn is {annealed:.3}, which is above the \
             {CHURN_SETTLED} the detector uses, so the anneal is not reaching a \
             fixed point"
        );
        assert!(
            annealed < shipped / 2.0,
            "the anneal gave churn {annealed:.3} against {shipped:.3} un-annealed, \
             which is not a meaningful reduction"
        );
    }

    /// The decay the effect actually uses follows the anneal, and only under it.
    ///
    /// Tracked as the *maximum* decay seen during growth rather than the value at
    /// a wall-clock time, because the effect settles and re-seeds several times in
    /// any minute and a reading taken "after seventy seconds" may well be four
    /// steps into a fresh cycle. A test that assumed otherwise passed against a
    /// completely broken anneal by catching the effect between cycles.
    #[test]
    fn the_decay_in_use_is_the_annealed_one_and_resets_each_cycle() {
        let mut physarum = Physarum::new(options(3), (60, 20));
        assert_eq!(
            physarum.decay, DECAY,
            "a fresh effect should start at the configured decay"
        );

        let mut peak_decay = physarum.decay;
        let mut previous_age = 0u64;
        let mut rising = true;
        for _ in 0..12_000 {
            physarum.update();
            if physarum.phase != Phase::Growing {
                continue;
            }
            if physarum.decay < peak_decay && physarum.age_steps > previous_age {
                // Inside one growth the decay only ever climbs, so a fall means the
                // cycle restarted and the reading is not comparable.
                rising = false;
                break;
            }
            if physarum.age_steps > previous_age {
                rising = true;
                previous_age = physarum.age_steps;
                peak_decay = peak_decay.max(physarum.decay);
            }
        }

        assert!(rising, "the decay fell without the cycle restarting");
        assert!(
            peak_decay >= 0.99,
            "the decay only ever reached {peak_decay}, so the anneal does not get \
             near the {ANNEALED_DECAY} the settled state needs"
        );
        assert!(
            peak_decay <= ANNEALED_DECAY,
            "the decay reached {peak_decay}, past the {ANNEALED_DECAY} ceiling"
        );

        // Through a hold and a fade and a re-seed it comes back to the configured
        // value. This is the regression the field exists for: without the reset, the
        // second cycle would start at 0.995 and never grow a network again, and the
        // effect would look settled and broken at the same time.
        //
        // "Back to the configured value" is a bound and not an equality, because
        // the effect starts stepping again the moment the re-seed lands, and the
        // anneal climbs from that instant. A tenth of a second is six frames, which
        // is 6/2500 of the ramp -- so the bound below is wide enough for a partial
        // reset to pass and tight enough that a reset which left the decay at 0.9
        // plus a tenth would not.
        physarum.phase = Phase::Fading;
        physarum.phase_timer = 0.01;
        run_frames(&mut physarum, 0.1);
        let rise = ANNEALED_DECAY - DECAY;
        assert!(
            physarum.decay < DECAY + rise * 0.01,
            "a new cycle is annealing from {} rather than from the configured \
             {DECAY}, so it would never grow a network again",
            physarum.decay
        );
        assert!(
            physarum.age_steps < 10,
            "a new cycle is already {} steps old",
            physarum.age_steps
        );
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
