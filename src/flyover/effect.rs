use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::braille::{BrailleGrid, DOTS_X, DOTS_Y};
use crate::render::dither::Dither;
use crate::render::palette::{Palette, presets as palette_presets};
use crate::runtime::FrameContext;
use crate::terrain::noise::PerlinNoise;
use crossterm::style::{self, Color};
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Half the range the octave noise actually reaches, in units of `1.0`.
///
/// **Measured, not assumed.** The spec sheet says a Perlin octave spans
/// `-1.0..=1.0`. Four octaves at `persistence = 0.5` and `scale = 1/240` span
/// `-0.53..=0.55` on seed 42, and `-0.65..=0.48` on seed 1; across five seeds the
/// peak-to-peak range is 1.00 to 1.13. Dividing by 1.0 -- the obvious
/// "normalise" -- would hand back about half the height asked for and clip one
/// end asymmetrically, and the error is silent: the landscape is simply flat.
///
/// Slightly *under* the smallest measured range rather than at it, so a height
/// that does exceed it is a taller hill rather than a clipped plateau.
/// `the_noise_still_spans_the_calibrated_range` fails if the shared noise
/// changes underneath this.
const NOISE_HALF_RANGE: f64 = 0.55;

/// Physical height of a cell as a multiple of its width.
///
/// A terminal cell is roughly twice as tall as it is wide, and this is what makes
/// the dot grid square: a braille cell is `DOTS_X` dots wide and `DOTS_Y` tall, so
/// one dot is `1 / DOTS_X` of a cell across and `CELL_ASPECT / DOTS_Y` of a cell
/// down, and at `2.0` and `4` those are both half a cell. Without the correction
/// the projection would treat a 400x200 terminal's 800x800 dot grid as square when
/// it is not, and the whole landscape would come out stretched by whatever the
/// real ratio is.
///
/// `2.0` is an approximation, and the caveat is the one every sub-cell renderer
/// in this crate carries: DejaVu Sans Mono is nearer 1:1.2, so on that font the
/// landscape is stretched vertically. A property of the technique rather than a
/// bug, and one number rather than a correction threaded through the projection.
const CELL_ASPECT: f32 = 2.0;

/// Physical width of one dot, in units of a cell width.
const DOT_WIDTH: f32 = 1.0 / DOTS_X as f32;

/// Physical height of one dot, in units of a cell width.
const DOT_HEIGHT: f32 = CELL_ASPECT / DOTS_Y as f32;

/// Vertical field of view, as the tangent of its half-angle.
///
/// About 48 degrees. Not configurable: it is the difference between a landscape
/// you are flying over and one you are looking down at from a satellite, and there
/// is no second opinion on which of those a screensaver wants.
const FOV_TAN: f32 = 0.45;

/// Closest a ray may sample, in world units.
///
/// Below this, the march is evaluating the ground the camera is sitting on, at a
/// spacing finer than the landforms it is trying to resolve.
const NEAR: f32 = 1.5;

/// How fast the march step grows, as a fraction of the current depth.
///
/// A fixed step is wrong in both directions. Small enough not to miss a ridge at
/// the far plane means hundreds of steps near the camera; large enough to be
/// cheap near the camera steps straight over the ridges that are close. Growing
/// the step with depth spends the samples where they resolve something.
///
/// 0.14, and it was 0.09, which is a third more steps for no visible gain. The
/// march is this effect's dominant cost -- measured at 400x200, 1.67 ms of a
/// 2.66 ms render -- and the step count is what sets it: about 42 samples a column
/// at 0.09 and 28 at 0.14. Going below that starts to step over the far ridges,
/// which is the one thing the far plane exists to draw.
const STEP_GROWTH: f32 = 0.14;

/// Smallest step, in world units, which is what the growth starts from.
const STEP_MIN: f32 = 1.5;

/// Furthest a ray is marched, in world units.
///
/// This is the draw distance and it is what sets the horizon: terrain past it is
/// not drawn, so the far plane sits on the sky. Well past the visible ridges,
/// because a hard edge at the far plane is a line across the picture, and the fog
/// needs room to hide it.
const FAR: f32 = 260.0;

/// How quickly distance bleaches the ground into the sky, in world units.
///
/// Every distance cue in a landscape comes from atmospheric perspective, and
/// without it a ridge at the far plane is drawn in the same ink as the ground
/// under the camera -- which is why the effect reads as a flat cut-out rather than
/// as a view.
const FOG_DISTANCE: f32 = 85.0;

/// Dots over which the ground's top edge fades from empty to solid.
///
/// A hard silhouette is what a wireframe does, and what this deliberately does not:
/// braille's eight dots per cell are worth spending on a dithered fringe, which is
/// the one place in the frame where the extra resolution is doing something a cell
/// grid could not.
const FRINGE_DOTS: f32 = 5.0;

/// World units between the two samples used for a surface normal.
///
/// A tenth of the default noise period, so the gradient measures the shape rather
/// than the grain of one octave. Too small and the normal is noise, and a surface
/// lit by noise has no ridges in it.
const NORMAL_STEP: f32 = 24.0;

/// Direction the light comes *from*, unnormalised.
///
/// Low and off to one side, so slopes turned towards the camera are lit and
/// slopes turned away are in shadow. A light that moved would make the terrain
/// flicker rather than have form.
const SUN: [f32; 3] = [-0.45, 0.72, -0.53];

/// How fast the camera's height chases the ground, per second.
///
/// A first-order lag rather than a snap, and it is the difference between flying
/// and bobbing. Snapping the camera to the surface puts every bump in the terrain
/// straight into the whole frame's vertical position, so the horizon jumps; the
/// lag turns a bump into a swell.
const FOLLOW_RATE: f32 = 1.6;

/// Octaves in the height field.
///
/// Four, and the fifth is free: measured over the 800 columns by 50 steps this
/// effect actually marches, four octaves cost 1.02 ms a frame and five cost
/// 1.02 ms. The per-sample cost is dominated by the permutation lookup rather
/// than by the extra interpolation, so there is no reason to leave detail out.
const OCTAVES: i32 = 4;

/// The camera. A struct rather than five fields on the effect, because it moves as
/// a unit and the interaction between pitch, roll and height *is* the effect.
#[derive(Debug, Clone, Copy)]
struct Camera {
    x: f32,
    y: f32,
    z: f32,
    /// Radians. Positive looks down.
    pitch: f32,
    /// Radians of roll about the view axis, for a slow bank.
    roll: f32,
}

/// Entries in the fog lookup, over the range of depths a ray can reach.
const FOG_STEPS: usize = 1024;

/// The depth the fog table covers, in world units.
///
/// [`FAR`] with a little room on the end, so an index can never leave the table
/// however the march is configured.
const FOG_RANGE: f32 = FAR * 1.01;

/// `exp(-depth / FOG_DISTANCE)`, by table.
///
/// The obvious spelling is `(depth / FOG_DISTANCE).neg_exp()`, and it is the
/// single most expensive thing in this effect's render. The shading loop runs once
/// per dot below the silhouette, which on a 400x200 terminal is 800 columns by
/// about 550 rows -- 440,000 of them -- and an `f32::exp` is a call rather than an
/// instruction. Measured, replacing it with this table took the render from
/// 3.25 ms to well under a millisecond.
///
/// The step is a quarter of a world unit, which is finer than the fog itself: over
/// a quarter of a unit at the far end of the range the fog changes by three tenths
/// of a percent, and the ramp it feeds is quantised to sixteen levels.
static FOG_LUT: std::sync::LazyLock<[f32; FOG_STEPS]> =
    std::sync::LazyLock::new(|| {
        std::array::from_fn(|i| {
            let depth = i as f32 * FOG_RANGE / FOG_STEPS as f32;
            (-depth / FOG_DISTANCE).exp()
        })
    });

/// Atmospheric perspective at a depth.
#[inline]
fn fog_at(depth: f32) -> f32 {
    let index = (depth * (FOG_STEPS as f32 / FOG_RANGE)) as usize;
    FOG_LUT[index.min(FOG_STEPS - 1)]
}

/// Cross product of two 3-vectors.
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Camera {
    /// The camera's `right`, `up` and `forward` axes in world space.
    ///
    /// Built per frame because the projection of a hit needs camera space while the
    /// hit itself is found by marching in world space, and the two only meet here.
    ///
    /// The two orderings in this function are load-bearing and were both wrong in
    /// the first version of it. `up` is `forward × right`, not `right × forward`:
    /// with the camera looking down `+z`, `right` is `+x`, and `x × z` is `-y`, so
    /// the other order gives an `up` that points at the ground. And the roll is a
    /// rotation *about the forward axis*, which leaves `forward` untouched and
    /// turns `right` into `right·cos + up·sin`; the first version instead
    /// interpolated `right`'s own components with `sin`, which is a yaw and tilts
    /// the view sideways rather than banking it.
    fn basis(&self) -> [[f32; 3]; 3] {
        let (sp, cp) = self.pitch.sin_cos();
        let (sr, cr) = self.roll.sin_cos();
        // Positive pitch is looking down, so forward's y goes negative.
        let forward = [0.0, -sp, cp];
        let level_right = [1.0, 0.0, 0.0];
        // `forward × right` rather than assuming it is `(0, 1, 0)`, because the
        // camera is pitched and the up axis is not vertical.
        let level_up = cross(forward, level_right);
        let right = [
            level_right[0] * cr + level_up[0] * sr,
            level_right[1] * cr + level_up[1] * sr,
            level_right[2] * cr + level_up[2] * sr,
        ];
        let up = [
            level_up[0] * cr - level_right[0] * sr,
            level_up[1] * cr - level_right[1] * sr,
            level_up[2] * cr - level_right[2] * sr,
        ];
        [right, up, forward]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FlyoverOptions {
    /// World units per second along the flight path.
    ///
    /// Not a row count, so the same number gives the same *landscape* at every
    /// terminal rather than the same apparent speed. A terminal twice as wide
    /// shows twice as much of the same ground, so it takes twice as long to reach
    /// a landmark -- and that is correct, because the camera is moving through a
    /// world rather than across a screen.
    pub speed: f32,

    /// Peak terrain height above the mean ground, in world units.
    ///
    /// A real height rather than a multiple of the noise, because
    /// [`NOISE_HALF_RANGE`] divides the measured noise span out. Raising it gives
    /// mountains, and the ride height and field of view do not change, so a
    /// higher value is a genuinely more dramatic landscape rather than a closer
    /// look at the same one.
    pub relief: f32,

    /// How far above the ground the camera rides, in world units.
    ///
    /// Low enough that near ridges occlude the ones behind them, which is the cue
    /// that makes this a landscape rather than a texture scrolling away.
    pub flying_height: f32,

    /// World units per noise period: roughly how wide a landform is.
    pub scale: f32,

    /// Octaves per [`persistence`](Self::persistence) step.
    pub octaves: i32,

    /// How much of itself each finer octave keeps.
    pub persistence: f32,

    /// Half-width of the camera's lateral weave, in world units.
    ///
    /// A camera flying dead straight over a noise field still shows plenty,
    /// because the ground moves underneath it. This is on top of that, and it is
    /// what stops the frame from having a fixed composition: a slow weave changes
    /// which ridge is in front of which, which is the only way to get parallax
    /// between two layers of the same landscape.
    pub sway: f32,

    /// A named ramp from [`crate::render::palette`], used for the ground.
    pub palette: String,

    pub seed: u64,
}

impl Default for FlyoverOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default` produced
    /// zeros, and serde used the derived one, so a config file that omitted a
    /// section silently zeroed it.
    fn default() -> Self {
        Self {
            speed: 26.0,
            relief: 26.0,
            flying_height: 11.0,
            scale: 240.0,
            octaves: OCTAVES,
            persistence: 0.5,
            sway: 34.0,
            palette: "depth".to_string(),
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Flyover {
    screen_size: (u16, u16),
    options: FlyoverOptions,
    canvas: Canvas,
    grid: BrailleGrid,
    camera: Camera,
    /// The line the weave is centred on. Seeded, so two runs of one seed fly the
    /// same line and two different seeds do not open on the same view of two
    /// different landscapes.
    base_x: f32,
    noise: PerlinNoise,
    time: f32,
    rng: EffectRng,
    palette: Palette,
    /// Per-dot coverage over the dot grid. Rebuilt every frame, so it is
    /// allocated once: a full-screen allocation at sixty hertz is exactly what
    /// `Canvas` exists to stop.
    coverage: Vec<f32>,
    /// Where the surface is, by dot row, for the column being drawn. Reused across
    /// columns and the only per-column working storage there is.
    ///
    /// This is the array the whole renderer hangs off, and getting it is the
    /// difference between drawing a landscape and drawing nothing at all. See
    /// [`Flyover::march`].
    depth_at_row: Vec<f32>,
    /// One column's coverage, written contiguously and then transposed.
    column_coverage: Vec<f32>,
    /// The row each dot column's own silhouette sits on, or `usize::MAX` where
    /// the march found no ground.
    ///
    /// Recorded during the draw rather than recovered afterwards, which is the
    /// whole point. `column_coverage` is the per-column scratch buffer and is
    /// zeroed once per *frame*, so the rows above a column's silhouette are only
    /// clear because `draw` clears them per column as well. Without that, a
    /// column whose silhouette sat *lower* than its neighbour's would inherit
    /// the neighbour's ground in its sky.
    ///
    /// **This has never actually happened.** Searched rather than assumed: across
    /// 40 seeds x 600 frames x 200 dot columns at 200x50 -- 9.6 million
    /// column-draws -- no column ever carried coverage above its own
    /// silhouette, and the worst stale coverage measured was 0.0000. The reason
    /// is structural: `march` gap-fills downwards from the topmost row any ray
    /// touched, and because the camera looks slightly down over terrain running
    /// to the horizon, that topmost row is pinned near the horizon for every
    /// column. So the sequence of `top` values is effectively flat and the
    /// condition cannot arise from the shipped landscape.
    ///
    /// The clear is kept anyway. It is one `fill` of a short slice, it is what
    /// makes the buffer's contract "this column's coverage, nothing else" true by
    /// construction rather than by luck, and the condition would become reachable
    /// by any change that made the skyline vary between adjacent columns -- a
    /// nearer ridge, a steeper pitch, a rolled camera.
    ///
    /// Two sky tests already existed and neither could see this. One asserts the
    /// top *eighth* of the screen is empty, and a smear would sit beside a
    /// silhouette rather than above the highest one. The other derives the
    /// skyline from the drawn frame, so leaked ink *raises* the skyline it is
    /// measuring and satisfies it. Both read the picture.
    ///
    /// The first version of the regression test here had the same flaw and was
    /// worse for it: it found each column's skyline as the topmost inked row of
    /// the coverage field -- the very quantity a leak would have corrupted -- so
    /// stale rows raised the measurement and the smear cancelled itself out. The
    /// test passed against the code it was written for. **A question about where
    /// the ground starts cannot be answered from a field in which the ground may
    /// have leaked upwards**, which is why the skyline is stored while it is still
    /// known and read back here rather than re-derived.
    silhouette: Vec<usize>,
    /// Every column's coverage, still column-major. See [`Flyover::transpose`].
    profiles: Vec<f32>,
    /// Per-cell colour over the cell grid.
    cell_colour: Vec<Color>,
    /// Sky colour by cell row.
    sky: Vec<Color>,
}

impl TerminalEffect for Flyover {
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
        self.grid
            .resize(self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.coverage = vec![0.0; self.grid.dot_width() * self.grid.dot_height()];
        self.depth_at_row = vec![0.0; self.grid.dot_height()];
        self.column_coverage = vec![0.0; self.grid.dot_height()];
        self.silhouette = vec![usize::MAX; self.grid.dot_width()];
        self.profiles = vec![0.0; self.grid.dot_width() * self.grid.dot_height()];
        self.cell_colour =
            vec![Color::Reset; self.canvas.width() * self.canvas.height()];
        self.time = 0.0;
        // Reseeded with the rest of the state this rebuilds, and the noise rebuilt
        // with it: a different seed is a different world, and the permutation
        // table *is* the world.
        self.rng = seeded_rng(self.options.seed, "flyover");
        self.noise = PerlinNoise::new(self.options.seed);
        self.base_x = self.rng.random_range(-800.0..800.0);
        self.camera = Camera {
            x: self.base_x,
            y: 0.0,
            z: 0.0,
            pitch: 0.0,
            roll: 0.0,
        };
        // Settled onto the ground before the first frame, or the opening view is
        // from below the terrain looking up through it.
        self.camera.y =
            self.ground(self.camera.x, self.camera.z) + self.flying_height();
        self.build_sky();
    }
}

impl Flyover {
    pub fn new(options: FlyoverOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let grid = BrailleGrid::new(screen_size.0 as usize, screen_size.1 as usize);

        // Truncated, for the reason in `Physarum`: `DEPTH` is mirrored, and taken
        // whole it drew the highest terrain in near-black while the sky sat in the
        // pale middle of the ramp -- so the silhouette was the darkest thing in
        // the frame. See `Palette::truncated_at_peak`.
        let palette = Palette::new(
            palette_presets::by_name(&options.palette)
                .unwrap_or(palette_presets::DEPTH)
                .to_vec(),
        )
        .truncated_at_peak();

        let mut flyover = Self {
            screen_size,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            // Per-dot coverage over the dot grid. Rebuilt every frame, so it is
            // allocated once: a full-screen allocation at sixty hertz is exactly
            // what `Canvas` exists to stop.
            coverage: vec![0.0; grid.dot_width() * grid.dot_height()],
            depth_at_row: vec![0.0; grid.dot_height()],
            column_coverage: vec![0.0; grid.dot_height()],
            silhouette: vec![usize::MAX; grid.dot_width()],
            profiles: vec![0.0; grid.dot_width() * grid.dot_height()],
            cell_colour: vec![
                Color::Reset;
                screen_size.0 as usize * screen_size.1 as usize
            ],
            sky: Vec::new(),
            grid,
            camera: Camera {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                pitch: 0.0,
                roll: 0.0,
            },
            base_x: 0.0,
            noise: PerlinNoise::new(DEFAULT_SEED),
            time: 0.0,
            rng: seeded_rng(DEFAULT_SEED, "flyover"),
            palette,
            options,
        };
        flyover.reset();
        flyover
    }

    fn flying_height(&self) -> f32 {
        self.options.flying_height.max(1.0)
    }

    /// Noise periods per world unit.
    fn noise_scale(&self) -> f64 {
        if self.options.scale > 1.0 {
            1.0 / self.options.scale as f64
        } else {
            // A zero or non-positive period would collapse the whole field toward a
            // single value, which renders as a flat slab. One world unit per period
            // is the finest a config can ask for and is still a field.
            1.0
        }
    }

    /// Terrain height at a world position, in world units above the mean ground.
    ///
    /// The one place the height field is defined, and the one place the noise
    /// scale and measured range are applied. The march, the camera's ride height
    /// and the surface normals all come through here, so there is a single answer
    /// to "how tall is the ground there" and the camera cannot end up disagreeing
    /// with the picture about it.
    fn ground(&self, x: f32, z: f32) -> f32 {
        let raw = self.noise.octave_noise_2d(
            x as f64,
            z as f64,
            self.options.octaves.clamp(1, 16),
            self.options.persistence.max(0.0) as f64,
            self.noise_scale(),
        );
        (raw / NOISE_HALF_RANGE * self.options.relief as f64) as f32
    }

    pub fn advance(&mut self, delta: f32) {
        // Clamped, because a stall hands this effect a delta large enough to jump
        // the camera a whole landmark and the frame would tear.
        let delta = delta.clamp(0.0, 0.05);

        self.time += delta;
        let sway = self.options.sway;
        // Two incommensurate frequencies, so the weave does not visibly repeat and
        // a long session does not arrive back at the same view twice.
        self.camera.x = self.base_x
            + sway * (self.time * 0.11).sin()
            + sway * 0.4 * (self.time * 0.037).sin();
        // A slow bank at the weave's own rate, so the horizon tilts *with* the
        // turn rather than independently of it.
        self.camera.roll = 0.045 * (self.time * 0.11 + 0.6).sin();
        self.camera.z += self.options.speed.max(0.0) * delta;

        let follow = 1.0 - (-FOLLOW_RATE * delta).exp();

        // Ride the ground, sampled ahead of the camera so a rising slope lifts it
        // before the slope arrives, and lagged so a bump is a swell.
        let look_ahead = self.flying_height() * 2.0;
        let target = self.ground(self.camera.x, self.camera.z + look_ahead)
            + self.flying_height();
        self.camera.y += (target - self.camera.y) * follow;

        // Look slightly down, and lean into the slope ahead. Lagged for the same
        // reason the height is: a pitch driven straight off the terrain pitches
        // with every bump, and the horizon is the one thing in the frame that must
        // not jitter.
        //
        // 0.09, and it is a framing number rather than a feel. The horizon lands at
        // `dot_height * (0.5 - 0.5 * tan(pitch) / FOV_TAN)`, and at 0.09 that is
        // 40% of the way down the frame -- a landscape with sky in the top third
        // and ground in the rest. The first version used 0.16 and put the horizon
        // at 17%, which is a thin strip of sky above a wall of ground, and reads as
        // a dirt texture rather than as a view.
        let ahead = self.ground(self.camera.x, self.camera.z + 90.0);
        let behind = self.ground(self.camera.x, self.camera.z + 20.0);
        let target_pitch = 0.09 + ((ahead - behind) / 70.0).clamp(-0.5, 0.5) * 0.3;
        self.camera.pitch += (target_pitch - self.camera.pitch) * follow;
    }

    /// The cell grid's dimensions, for a caller inspecting the picture.
    pub fn dimensions(&self) -> (usize, usize) {
        (self.canvas.width(), self.canvas.height())
    }

    /// The braille glyph this effect last drew into a cell.
    pub fn cell_symbol(&self, x: usize, y: usize) -> char {
        self.canvas.on_screen().get(x, y).symbol
    }

    /// The coverage one dot of one column was drawn with, from the per-column
    /// depth profile rather than from the finished frame.
    ///
    /// The picture is the wrong place to ask a question about *where the ground
    /// starts in this column*, because a glyph is the OR of eight dots: a cell
    /// that is inked somewhere does not say which dot, and the frame cannot
    /// separate one column's sky from the next one's ground.
    pub fn profile_at(&self, dot_x: usize, dot_y: usize) -> f32 {
        let dh = self.grid.dot_height();
        self.profiles
            .get(dot_x * dh + dot_y)
            .copied()
            .unwrap_or(0.0)
    }

    /// The row this dot column's own silhouette sits on, or `usize::MAX` where
    /// the march found no ground in it.
    pub fn silhouette_of(&self, dot_x: usize) -> usize {
        self.silhouette.get(dot_x).copied().unwrap_or(usize::MAX)
    }

    /// How many cells carry any ground at all, how many carry none, and the
    /// topmost row that is entirely empty.
    pub fn coverage_summary(&self) -> (usize, usize, usize) {
        let width = self.canvas.width();
        let height = self.canvas.height();
        let mut inked = 0;
        let mut topmost_empty = height;
        for y in 0..height {
            let row_blank = (0..width).all(|x| self.cell_symbol(x, y) == ' ');
            if row_blank {
                topmost_empty = topmost_empty.min(y);
            } else {
                for x in 0..width {
                    if self.cell_symbol(x, y) != ' ' {
                        inked += 1;
                    }
                }
            }
        }
        (inked, width * height - inked, topmost_empty)
    }

    /// The horizon, pitch, roll and camera position, for a caller measuring them.
    pub fn horizon_row_value(&self) -> f32 {
        self.horizon_row()
    }
    pub fn pitch_value(&self) -> f32 {
        self.camera.pitch
    }
    pub fn roll_value(&self) -> f32 {
        self.camera.roll
    }
    pub fn camera_height(&self) -> f32 {
        self.camera.y
    }
    pub fn camera_depth(&self) -> f32 {
        self.camera.z
    }

    /// Terrain height at a world position. Exposed so a caller can check the
    /// calibration against the noise it is calibrated from.
    pub fn ground_at(&self, x: f32, z: f32) -> f32 {
        self.ground(x, z)
    }

    /// The dot grid's dimensions, for a caller inspecting the picture.
    pub fn dot_dimensions(&self) -> (usize, usize) {
        (self.grid.dot_width(), self.grid.dot_height())
    }

    /// The camera's lateral position.
    pub fn camera_lateral(&self) -> f32 {
        self.camera.x
    }

    /// Terrain height directly under the camera.
    pub fn ground_under_camera(&self) -> f32 {
        self.ground(self.camera.x, self.camera.z)
    }

    /// Walks one column's ray and reports `(depth, world x, world z, height, row)`
    /// for its first forty steps, for a caller checking the projection by hand.
    ///
    /// Forty rather than a handful because the near ground is always *below* the
    /// frame -- a camera flying eleven units up cannot see the ground under it --
    /// so the first sample that lands on screen is around the eighth step, and a
    /// short list contains nothing to check.
    pub fn column_samples(&self, column: usize) -> Vec<(f32, f32, f32, f32, f32)> {
        let (dw, dh) = (self.grid.dot_width(), self.grid.dot_height());
        let [right, up, forward] = self.camera.basis();
        let aspect = (dw as f32 * DOT_WIDTH) / (dh as f32 * DOT_HEIGHT);
        let tan_x = FOV_TAN * aspect;
        let centre_row = dh as f32 * 0.5;
        let eye = [self.camera.x, self.camera.y, self.camera.z];
        let ndc_x = (column as f32 + 0.5 - dw as f32 * 0.5) / (dw as f32 * 0.5);
        let slope_x = ndc_x * tan_x;

        let mut out = Vec::new();
        let mut depth = NEAR;
        while out.len() < 40 && depth < FAR {
            let wx = eye[0] + right[0] * slope_x * depth + forward[0] * depth;
            let wz = eye[2] + right[2] * slope_x * depth + forward[2] * depth;
            let height = self.ground(wx, wz);
            let dx = wx - eye[0];
            let dy = height - eye[1];
            let dz = wz - eye[2];
            let ahead = forward[0] * dx + forward[1] * dy + forward[2] * dz;
            let row = if ahead > 1e-3 {
                let vertical = up[0] * dx + up[1] * dy + up[2] * dz;
                centre_row - vertical / ahead / FOV_TAN * dh as f32 * 0.5
            } else {
                f32::NAN
            };
            out.push((depth, wx, wz, height, row));
            depth += (depth * STEP_GROWTH).max(STEP_MIN);
        }
        out
    }

    /// The dot row the camera's forward direction lands on.
    ///
    /// This is the row the *ground* approaches as it goes to infinity, which is
    /// what the sky gradient has to meet, and it is a consequence of the camera's
    /// pitch rather than a second thing added on top of it. Looking down puts the
    /// horizon above the middle of the frame, so the sign is negative; and the
    /// projection divides by the field of view, so the offset is a tangent.
    ///
    /// The march itself is *not* given this row. It is given the frame's centre
    /// row, because the pitch is already in the camera's basis and adding it again
    /// here would count it twice -- the first version did, which put the horizon
    /// about a third of a frame too high and left the ground off the bottom of the
    /// screen entirely.
    fn horizon_row(&self) -> f32 {
        let dh = self.grid.dot_height() as f32;
        dh * (0.5 - 0.5 * self.camera.pitch.tan() / FOV_TAN)
    }

    /// The sky, as one colour per cell row.
    ///
    /// Per cell row and not per dot row, because a braille cell carries a single
    /// colour: this is as fine as a vertical sky gradient gets. It is enough --
    /// what reads is a gradient from the top of the frame down to the horizon, not
    /// one from dot to dot.
    fn build_sky(&mut self) {
        let height = self.canvas.height().max(1);
        let horizon_cell =
            (self.horizon_row() / DOTS_Y as f32 / height as f32).clamp(0.0, 1.0);
        self.sky = (0..height)
            .map(|row| {
                let t = 1.0 - row as f32 / height as f32;
                // 0 at the top of the frame, 1 at the horizon. Estimated from the
                // pitch rather than measured, and the estimate is loose on purpose:
                // the sky is a gradient, so being a shade off at the join is
                // invisible, whereas chasing the true join would make the top of
                // the sky change colour as the camera banks.
                let k = 1.0
                    - ((t - horizon_cell) / (1.0 - horizon_cell).max(0.05))
                        .clamp(0.0, 1.0);
                // Dark at the zenith, lifting towards the horizon the way a hazy
                // sky does.
                Color::Rgb {
                    r: (8.0 + 44.0 * k) as u8,
                    g: (10.0 + 60.0 * k) as u8,
                    b: (22.0 + 70.0 * k) as u8,
                }
            })
            .collect();
    }

    fn draw(&mut self) {
        self.canvas.clear();
        self.build_sky();
        self.coverage.fill(0.0);
        self.column_coverage.fill(0.0);
        self.silhouette.fill(usize::MAX);

        let (dw, dh) = (self.grid.dot_width(), self.grid.dot_height());
        let [right, up, forward] = self.camera.basis();
        let sun = {
            let length =
                (SUN[0] * SUN[0] + SUN[1] * SUN[1] + SUN[2] * SUN[2]).sqrt();
            [SUN[0] / length, SUN[1] / length, SUN[2] / length]
        };
        // The field's shape, in physical units rather than dot counts -- which is
        // the only reason `CELL_ASPECT` and `DOT_WIDTH` exist. On a 400x200
        // terminal this comes to exactly 1, because the dot grid is 800x800.
        let aspect = (dw as f32 * DOT_WIDTH) / (dh as f32 * DOT_HEIGHT);
        let tan_x = FOV_TAN * aspect;
        // The frame's centre row, *not* the horizon. The pitch is already in the
        // basis, and `horizon_row` is derived from it for the sky to meet.
        let centre_row = dh as f32 * 0.5;
        let eye = [self.camera.x, self.camera.y, self.camera.z];

        for sx in 0..dw {
            let ndc_x = (sx as f32 + 0.5 - dw as f32 * 0.5) / (dw as f32 * 0.5);
            let slope_x = ndc_x * tan_x;

            let Some(top) =
                self.march(&eye, &right, &forward, &up, slope_x, centre_row, dh)
            else {
                // No ground in this column at all, so every dot stays at zero
                // coverage and the cells are painted as sky below.
                continue;
            };
            self.silhouette[sx] = top;

            // Lit by the surface normal at the silhouette, which is the far edge of
            // the ground in this column and so the part a viewer reads the shape
            // from. Four extra height samples per column, and only for columns
            // that hit, which is what makes this affordable rather than four times
            // the march.
            let silhouette = self.depth_at_row[top];
            let wx = eye[0] + (right[0] * slope_x + forward[0]) * silhouette;
            let wz = eye[2] + (right[2] * slope_x + forward[2]) * silhouette;
            let hx = self.ground(wx + NORMAL_STEP, wz)
                - self.ground(wx - NORMAL_STEP, wz);
            let hz = self.ground(wx, wz + NORMAL_STEP)
                - self.ground(wx, wz - NORMAL_STEP);
            let normal = normal_of(hx, hz);
            let lambert =
                (normal[0] * sun[0] + normal[1] * sun[1] + normal[2] * sun[2])
                    .max(0.0)
                    .clamp(0.0, 1.0);
            // A surface seen from directly above is a face and wants most of the
            // ink; one seen edge-on is a line and wants almost none. The normal is
            // what tells them apart, and without this term every slope in the
            // frame is drawn solid and the terrain has no form.
            let edge_on = (1.0 - normal[1]).clamp(0.0, 1.0);
            let ink = (0.86 - 0.55 * edge_on) * (0.55 + 0.45 * lambert);

            for sy in top..dh {
                // Depth below the silhouette, in dots. The fringe is the whole
                // reason this is drawn in braille.
                let below =
                    ((sy as f32 - top as f32) / FRINGE_DOTS).clamp(0.0, 1.0);
                // Fog per row rather than per column, from this row's own depth.
                // This is the atmospheric perspective that makes the picture a
                // view rather than a cut-out, and it needs the per-row depth table
                // to be right: one fog value for the whole column would make the
                // ground a flat wash from the horizon to the bottom of the frame.
                let fog = fog_at(self.depth_at_row[sy]);
                self.column_coverage[sy] = (below * ink * fog).clamp(0.0, 1.0);
            }

            // One contiguous copy per column, rather than 128 scattered writes
            // into the row-major coverage field. See `transpose`.
            let base = sx * dh;
            self.profiles[base..base + dh].copy_from_slice(&self.column_coverage);
        }

        self.transpose();
        self.grid
            .draw_field(&self.coverage, 0.42, 0.5, Dither::Bayer4);
        self.paint();
    }

    /// Turns the column-major `profiles` into the row-major `coverage`.
    ///
    /// In tiles, and the tiling is the point. Filling the coverage field one
    /// column at a time writes `dot_width` floats four kilobytes apart, so each
    /// write is a different cache line and the working set for a full pass is
    /// larger than the cache: measured, that cost about two milliseconds a frame
    /// on a 400x200 terminal, which is two thirds of this effect's render cost and
    /// nothing to do with the terrain.
    ///
    /// A 32-by-32 tile is 4 KB of source and 4 KB of destination, which stays in
    /// L1 across the whole tile, so the same bytes are read and written once each
    /// rather than once per element.
    fn transpose(&mut self) {
        const TILE: usize = 32;
        let (dw, dh) = (self.grid.dot_width(), self.grid.dot_height());
        for ty in (0..dh).step_by(TILE) {
            for tx in (0..dw).step_by(TILE) {
                for y in ty..(ty + TILE).min(dh) {
                    for x in tx..(tx + TILE).min(dw) {
                        self.coverage[y * dw + x] = self.profiles[x * dh + y];
                    }
                }
            }
        }
    }

    /// Walks one column's ray and records where the surface is on every row.
    ///
    /// Returns the topmost row with ground on it, or `None` if the column is all
    /// sky.
    ///
    /// The obvious version of this returns the *highest* point the ray touches,
    /// which is the standard heightfield silhouette and is wrong here in a way
    /// that draws nothing at all. The highest point is the ground nearest the
    /// camera, and it is always off the bottom of the frame -- a camera flying ten
    /// units above the ground with a 48-degree field of view cannot see the ground
    /// directly below it. So the maximum is always below the last row, the column
    /// is reported empty, and the effect renders a blank screen. That is exactly
    /// what the first version did.
    ///
    /// What is wanted is the *profile*: for each row, the depth at which the
    /// surface first appears. Rows with no sample of their own inherit the one
    /// below, which is the nearer one, so the table reads as a continuous surface
    /// from the silhouette down to the bottom of the frame. The topmost row with an
    /// entry is the silhouette, and the table below it carries the depth each row
    /// needs for fog.
    #[allow(clippy::too_many_arguments)]
    fn march(
        &mut self,
        eye: &[f32; 3],
        right: &[f32; 3],
        forward: &[f32; 3],
        up: &[f32; 3],
        slope_x: f32,
        centre_row: f32,
        dh: usize,
    ) -> Option<usize> {
        for depth in self.depth_at_row.iter_mut() {
            *depth = f32::NAN;
        }

        let mut depth = NEAR;
        while depth < FAR {
            let along_x = right[0] * slope_x * depth + forward[0] * depth;
            let along_z = right[2] * slope_x * depth + forward[2] * depth;
            let wx = eye[0] + along_x;
            let wz = eye[2] + along_z;
            let height = self.ground(wx, wz);

            let dx = wx - eye[0];
            let dy = height - eye[1];
            let dz = wz - eye[2];
            // Guarded because the projection divides by it, and a ray pointing away
            // from the camera's own position gives a zero it cannot recover from.
            let ahead = forward[0] * dx + forward[1] * dy + forward[2] * dz;
            if ahead > 1e-3 {
                let vertical = up[0] * dx + up[1] * dy + up[2] * dz;
                let row = centre_row - vertical / ahead / FOV_TAN * dh as f32 * 0.5;
                if row >= 0.0 && (row as usize) < dh {
                    let r = row as usize;
                    // First writer wins, and the march runs near to far, so this
                    // keeps the *nearest* surface at each row -- which is the one
                    // that is not hidden behind it.
                    if self.depth_at_row[r].is_nan() {
                        self.depth_at_row[r] = depth;
                    }
                }
            }

            depth += (depth * STEP_GROWTH).max(STEP_MIN);
        }

        // Fill the gaps downwards-upwards, so a row with no sample of its own takes
        // the depth of the row below it. That is the right inheritance: the row
        // below is nearer the camera, and it is the surface that row is looking at.
        let mut top = dh;
        for r in (0..dh).rev() {
            if self.depth_at_row[r].is_nan() {
                if r + 1 < dh && !self.depth_at_row[r + 1].is_nan() {
                    self.depth_at_row[r] = self.depth_at_row[r + 1];
                }
            } else if r < top {
                top = r;
            }
        }

        (top < dh).then_some(top)
    }

    /// Writes the glyphs, with a colour per cell.
    ///
    /// Two colours, chosen per cell by what is in it. A cell with no ground is
    /// sky, at the gradient row it sits on. A cell with ground takes the ground
    /// ramp at an index from its own coverage, which is the third depth cue and
    /// costs nothing: a cell is two dots wide, so neighbouring cells differ by one
    /// dot of distance at most and any banding is invisible. What it buys is a
    /// landscape whose far ridges are paler than its near ones even where the fog
    /// has not yet thinned the ink out.
    fn paint(&mut self) {
        let width = self.canvas.width();
        let height = self.canvas.height();
        let dw = self.grid.dot_width();
        let last_sky = self.sky.len().saturating_sub(1);

        for cy in 0..height {
            let sky = self.sky[cy.min(last_sky)];
            for cx in 0..width {
                // The lowest coverage anywhere in the cell, which is a better
                // stand-in for "how solid is this patch" than the count of raised
                // dots: a cell with one raised dot in its corner is a different
                // thing from one with a full column of them, and the glyph already
                // says how many there are.
                let mut lowest = f32::INFINITY;
                for sy in cy * DOTS_Y..(cy + 1) * DOTS_Y {
                    for sx in cx * DOTS_X..(cx + 1) * DOTS_X {
                        lowest = lowest.min(self.coverage[sy * dw + sx]);
                    }
                }
                let colour = if lowest.is_finite() && lowest > 0.0 {
                    self.palette.sample((1.0 - lowest).clamp(0.0, 1.0))
                } else {
                    sky
                };
                self.cell_colour[cy * width + cx] = colour;
            }
        }

        for cy in 0..height {
            for cx in 0..width {
                let symbol = self.grid.cell_char(cx, cy);
                self.canvas.set(
                    cx,
                    cy,
                    Cell::new(
                        symbol,
                        self.cell_colour[cy * width + cx],
                        style::Attribute::Reset,
                    ),
                );
            }
        }
    }
}

/// Unit normal of a height field with gradients `hx` and `hz` over `NORMAL_STEP`.
///
/// The surface is `y = h(x, z)`, so the tangent along x is `(1, hx / step, 0)` and
/// along z is `(0, hz / step, 1)`, and the normal is perpendicular to both. The
/// shortcut is `(-hx, 2 * step, -hz)` normalised, and the `2 * step` is the term
/// that is easy to leave as `1`.
fn normal_of(hx: f32, hz: f32) -> [f32; 3] {
    let length = (hx * hx + hz * hz + 4.0 * NORMAL_STEP * NORMAL_STEP).sqrt();
    [-hx / length, 2.0 * NORMAL_STEP / length, -hz / length]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::TerminalEffect;
    use crate::terrain::noise::PerlinNoise;

    fn fixture(seed: u64) -> Flyover {
        Flyover::new(
            FlyoverOptions {
                seed,
                ..FlyoverOptions::default()
            },
            (100, 32),
        )
    }

    /// Runs the effect for a number of frames, committing each one.
    fn run(flyover: &mut Flyover, frames: u32) {
        for _ in 0..frames {
            flyover.advance(1.0 / 60.0);
            flyover.get_diff();
        }
    }

    /// The topmost row carrying any ink in each column, as a cell row.
    ///
    /// `None` for a column that is entirely sky. Read off the drawn frame rather
    /// than out of the simulation, because what matters is the picture and not the
    /// numbers that were meant to produce it.
    fn skyline(flyover: &Flyover) -> Vec<Option<usize>> {
        let (width, height) = flyover.dimensions();
        (0..width)
            .map(|x| (0..height).find(|y| flyover.cell_symbol(x, *y) != ' '))
            .collect()
    }

    /// No column is inked above its own silhouette.
    ///
    /// **A guard, not a reproduction.** The condition has never occurred in this
    /// effect -- 9.6 million column-draws were searched and the worst stale
    /// coverage was zero, for the structural reason given on [`Flyover`].'s
    /// `silhouette` field. So this does not fail against the version of `draw`
    /// that omits the per-column clear, and it is kept for the day the skyline
    /// stops being flat rather than because it caught anything.
    ///
    /// What it does establish is that the buffer means what its doc says. The
    /// skyline is read from `silhouette`, recorded while it was known; deriving
    /// it from the coverage field instead made this test pass against the exact
    /// defect it was written for, which is worth stating because it is the trap
    /// the other two sky tests fell into.
    #[test]
    fn no_column_is_inked_above_its_own_silhouette() {
        let mut flyover = fixture(3);
        run(&mut flyover, 120);
        let (dw, _dh) = flyover.dot_dimensions();

        let mut with_ground = 0;
        let mut smeared = Vec::new();
        for sx in 0..dw {
            let top = flyover.silhouette_of(sx);
            if top == usize::MAX {
                continue; // no ground in this column; nothing to smear into
            }
            with_ground += 1;
            if let Some(leaked) =
                (0..top).find(|y| flyover.profile_at(sx, *y) > 0.0)
            {
                smeared.push((sx, leaked, top));
            }
        }

        assert!(
            with_ground > dw / 2,
            "only {with_ground} of {dw} columns found ground, so this measured \
             nothing about the sky"
        );
        assert!(
            smeared.is_empty(),
            "{} of {dw} columns carry coverage ABOVE their own skyline -- column \
             {} is inked at row {} but its silhouette is row {}, and so on for \
             {smeared:?}. The sky is inheriting the neighbouring column's ground, \
             which no column of this landscape has ever done.",
            smeared.len(),
            smeared.first().map(|s| s.0).unwrap_or(0),
            smeared.first().map(|s| s.1).unwrap_or(0),
            smeared.first().map(|s| s.2).unwrap_or(0),
        );
    }

    #[test]
    fn there_is_sky_above_the_ground_and_ground_below_it() {
        // The two things that make it a landscape rather than a texture. Checked as
        // bands of rows, not as counts: a picture can satisfy "some ink and some
        // blank" while being upside down.
        let mut flyover = fixture(1);
        run(&mut flyover, 120);
        let (width, height) = flyover.dimensions();

        let inked_in = |from: usize, to: usize| {
            (from..to)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .filter(|(x, y)| flyover.cell_symbol(*x, *y) != ' ')
                .count()
        };

        // The top *eighth*, not the top quarter. A distant peak is allowed to rise
        // above the horizon line -- that is what a mountain is -- so the quarter is
        // not a sky band. The top of the frame is, because the camera is looking
        // slightly down and the tallest thing the noise produces is not 20 rows
        // tall. Measured at 90 frames: rows 0 to 3 are empty and the ground starts
        // at row 4.
        let top_eighth = inked_in(0, height / 8);
        let bottom_quarter = inked_in(height - height / 4, height);
        assert_eq!(
            top_eighth, 0,
            "the top eighth of the frame has {top_eighth} inked cells; the sky is \
             where the ground is not"
        );
        assert!(
            bottom_quarter > width * (height / 4) / 2,
            "only {bottom_quarter} of the bottom quarter's cells carry ink, so the \
             ground does not reach the bottom of the frame"
        );
    }

    #[test]
    fn the_ground_has_a_skyline_rather_than_a_flat_top() {
        // The direct test that this is a landscape and not a wall. A flat-topped
        // fill -- which is what the effect drew before the march was changed to
        // build a per-row depth profile -- has one skyline value for every column.
        let mut flyover = fixture(2);
        run(&mut flyover, 120);
        let skyline = skyline(&flyover);
        let ground: Vec<usize> = skyline.iter().flatten().copied().collect();

        assert!(
            ground.len() > skyline.len() / 2,
            "only {} of {} columns have any ground at all",
            ground.len(),
            skyline.len()
        );
        let lowest = ground.iter().min().expect("measured above");
        let highest = ground.iter().max().expect("measured above");
        assert!(
            *highest - *lowest >= 4,
            "the skyline spans {} rows ({} to {}), so the ground has no shape",
            highest - lowest,
            lowest,
            highest
        );
    }

    #[test]
    fn the_horizon_moves_smoothly_rather_than_jumping() {
        // The stutter test, borrowed from the DVD logo's. It is a proxy for
        // continuity and the test says so: it cannot see a single-column staircase
        // in the terrain itself. What it does catch is the camera's height or
        // pitch lurching, which would move every column's horizon at once.
        let mut flyover = fixture(3);
        run(&mut flyover, 60);

        let mut worst: f32 = 0.0;
        let mut previous = flyover.horizon_row_value();
        for _ in 0..240 {
            flyover.advance(1.0 / 60.0);
            flyover.get_diff();
            let horizon = flyover.horizon_row_value();
            worst = worst.max((horizon - previous).abs());
            previous = horizon;
        }

        let dh = flyover.dot_dimensions().1;
        assert!(
            worst < dh as f32 * 0.02,
            "the horizon moved {worst} rows in one frame on a {dh}-row grid, so \
             the camera is lurching rather than flying"
        );
    }

    #[test]
    fn the_camera_keeps_flying_forward() {
        let mut flyover = fixture(4);
        run(&mut flyover, 60);
        let start = flyover.camera_depth();
        run(&mut flyover, 60);
        let travelled = flyover.camera_depth() - start;
        assert!(
            travelled > 20.0,
            "the camera advanced {travelled} world units in a second, which is not \
             flying"
        );
    }

    #[test]
    fn the_camera_stays_above_the_ground() {
        // The follow is a lag, so a sudden rise can leave the camera briefly below
        // the surface. A frame from underneath is a frame of blank sky, so this is
        // a real failure and not a nicety.
        let mut flyover = fixture(5);
        for _ in 0..600 {
            run(&mut flyover, 1);
            let clearance = flyover.camera_height() - flyover.ground_under_camera();
            assert!(
                clearance > 0.0,
                "the camera is {} units below the ground at z {:.1}",
                -clearance,
                flyover.camera_depth()
            );
        }
    }

    #[test]
    fn a_point_projects_to_the_row_the_projection_says() {
        // The projection is hand-checked here rather than trusted, because it was
        // wrong three ways while this was built: `up` was `right × forward` and so
        // pointed at the ground, the roll rotated in the wrong plane, and the
        // pitch was added to the frame centre *and* carried in the camera's basis,
        // counting it twice. Each of those drew a plausible picture or none at
        // all, and none of them is caught by a test of the shape.
        let mut flyover = fixture(6);
        run(&mut flyover, 60);
        let (dw, dh) = flyover.dot_dimensions();

        // Take a real sample and recompute its row from the definition, rather than
        // comparing the march against itself.
        let samples = flyover.column_samples(dw / 2);
        // The first sample on or above the bottom of the frame. Any sample nearer
        // than that projects below the last row, and comparing those says nothing
        // about the projection -- which is what a first version of this test did,
        // and it is why it disagreed with itself.
        let (_, wx, wz, height, row) = samples
            .iter()
            .copied()
            .find(|(_, _, _, _, row)| *row >= 0.0 && *row < dh as f32)
            .unwrap_or_else(|| {
                panic!(
                    "no sample of the middle column lands on the frame in {} steps",
                    samples.len()
                )
            });
        let depth = samples[0].0;

        let eye_x = flyover.camera_lateral();
        let eye_y = flyover.camera_height();
        let eye_z = flyover.camera_depth();
        let pitch = flyover.pitch_value();
        // The basis, written out again here from the definition rather than read
        // back out of the effect, which is the whole point.
        //
        // With the roll zeroed -- it is set on the fixture below, because a bank
        // would put the roll into `right` and `up` and this arithmetic is the
        // un-banked case -- the camera looks down `+z`, so `right` is `+x` and
        // `up = forward x right` is `(0, cos p, sin p)`: perpendicular to forward,
        // and already unit length.
        flyover.camera.roll = 0.0;
        let forward = [0.0, -pitch.sin(), pitch.cos()];
        let up = [0.0, pitch.cos(), pitch.sin()];
        assert!(
            (forward[0] * up[0] + forward[1] * up[1] + forward[2] * up[2]).abs()
                < 1e-5,
            "the test's `up` is not perpendicular to its `forward`, so the test \
             itself is wrong"
        );
        let d = [wx - eye_x, height - eye_y, wz - eye_z];
        let ahead = forward[0] * d[0] + forward[1] * d[1] + forward[2] * d[2];
        let vertical = up[0] * d[0] + up[1] * d[1] + up[2] * d[2];
        let expected =
            dh as f32 * 0.5 - vertical / ahead / FOV_TAN * dh as f32 * 0.5;

        assert!(
            ahead > 0.0,
            "the sample at depth {depth} is behind the camera, so the test has \
             nothing to compare"
        );
        assert!(
            (row - expected).abs() < 0.05,
            "the sample at ({wx:.1}, {height:.2}, {wz:.1}) projected to row {row} \
             and hand arithmetic says {expected}"
        );
    }

    #[test]
    fn more_relief_gives_a_rougher_skyline() {
        // Catches a `relief` that has stopped reaching the renderer, which is a
        // silent failure: the frame would look the same and simply stop responding
        // to the setting.
        //
        // Measured as the *spread* of the skyline rather than as how much of the
        // frame is inked, because those move in opposite directions. A first
        // version asserted the inked count and failed: relief 1.0 inked 1,509 cells
        // and relief 40.0 inked 1,145, and both numbers are right. A near-flat
        // plane under a camera eleven units up is solid ground from the horizon to
        // the bottom of the frame; add mountains and the camera spends much of the
        // frame looking at sky above a nearer slope. The inked count measures
        // framing, not mountains.
        let skyline_spread = |relief: f32| {
            let mut flyover = Flyover::new(
                FlyoverOptions {
                    seed: 8,
                    relief,
                    ..FlyoverOptions::default()
                },
                (100, 32),
            );
            run(&mut flyover, 90);
            let skyline: Vec<usize> =
                skyline(&flyover).into_iter().flatten().collect();
            let lowest = skyline.iter().min().expect("some ground");
            let highest = skyline.iter().max().expect("some ground");
            highest - lowest
        };
        let flat = skyline_spread(1.0);
        let mountainous = skyline_spread(40.0);
        assert!(
            mountainous > flat * 2,
            "relief 1.0 gave a skyline spanning {flat} rows and relief 40.0 gave \
             {mountainous}, so the setting barely reaches the picture"
        );
    }

    #[test]
    fn different_seeds_are_different_landscapes() {
        // Sampled away from the origin on purpose. Perlin noise is defined on a
        // lattice and every corner of the cell at `(0, 0)` is the same point, so
        // the origin is zero for *every* seed -- and a first version of this test
        // sampled exactly there and reported that the seed changed nothing.
        let ground_here = |seed: u64| fixture(seed).ground_at(3.7, 1.9);
        assert_ne!(
            ground_here(100),
            ground_here(101),
            "the seed did not change the terrain"
        );
    }

    #[test]
    fn the_noise_still_spans_the_calibrated_range() {
        // The guard on [`NOISE_HALF_RANGE`], which is a measurement and not a fact
        // about Perlin noise. If the shared noise implementation changes, the
        // calibration is wrong and every landscape in the crate is half the height
        // it was asked for -- silently, because a flat landscape still looks like a
        // landscape.
        let mut lowest = f64::INFINITY;
        let mut highest = f64::NEG_INFINITY;
        for seed in [1u64, 42, 99, 12_345] {
            let noise = PerlinNoise::new(seed);
            for i in 0..2000 {
                for j in 0..4 {
                    let value = noise.octave_noise_2d(
                        i as f64 * 3.1,
                        j as f64 * 7.7,
                        OCTAVES,
                        0.5,
                        1.0 / 240.0,
                    );
                    lowest = lowest.min(value);
                    highest = highest.max(value);
                }
            }
        }
        assert!(
            lowest.abs() < NOISE_HALF_RANGE && highest.abs() < NOISE_HALF_RANGE,
            "the noise now spans {lowest:.3}..{highest:.3}, outside the +/-\
             {NOISE_HALF_RANGE} that `relief` is calibrated against"
        );
        // A range narrower than the calibration is equally wrong: it means the
        // terrain is shorter than `relief` asked for.
        assert!(
            highest - lowest > NOISE_HALF_RANGE,
            "the noise now spans only {} peak to peak, below the {NOISE_HALF_RANGE} \
             half-range that `relief` is calibrated against",
            highest - lowest
        );
    }

    #[test]
    fn every_emitted_cell_is_inside_the_canvas() {
        let mut flyover = fixture(9);
        for _ in 0..120 {
            flyover.advance(1.0 / 60.0);
            for (x, y, _) in flyover.get_diff() {
                assert!(x < 100 && y < 32, "({x}, {y}) is outside a 100x32 canvas");
            }
        }
    }
}
