//! Falling sand: discrete materials under gravity, in a character grid.
//!
//! Each cell holds one material. Powder falls and piles at an angle of repose;
//! liquids fall and then spread sideways; fire rises and ignites what it
//! touches; plants grow from a wet seed next to a solid.
//!
//! ## Why the direction of iteration is the whole simulation
//!
//! **Rows are swept bottom-up.** A top-down sweep lets a particle fall all the
//! way to the floor in a single step, because by the time the sweep reaches the
//! lower row the particle is already there. The pile then has no mass: it is a
//! line, it holds no angle, and it does not slump. Bottom-up, a particle can
//! move at most one cell per step, which is the invariant that makes a heap read
//! as a heap.
//!
//! `a_grain_falls_one_row_per_step_and_no_further` pins it, by stepping a
//! single grain down an empty column and checking it takes exactly N steps to
//! reach row N.
//!
//! ## Why the hourglass is the default and not a box
//!
//! A pile converges to its angle of repose and stops, which makes a plain
//! container the worst possible screensaver: four seconds of interesting settling
//! and then a frozen picture. The hourglass is **perpetually self-resetting by
//! construction** -- sand fed through a narrow neck into a stone chamber -- so
//! the board is never in equilibrium and never needs a reset of its own. A closed
//! box that fills up is strictly worse than the same effect with a source.
//!
//! ## Why Margolus blocks
//!
//! Powder that falls diagonally picks a side at random, and a fixed scan order
//! biases that choice: everything leans the same way, and the pile develops a
//! permanent lean that reads as a rendering artefact. The Margolus neighbourhood
//! fixes it properly by partitioning the board into 2x2 blocks and shuffling the
//! block offset between sweeps, which makes all four block positions equally
//! likely and the diagonals isotropic.
//!
//! The honest budget: on a 400x200 board that is four gravity passes and four
//! lateral passes a frame. Twelve and twenty-four does not fit, and the reason it
//! is written this way is so that the limit is a constant in the source rather
//! than a thing to discover in `frame_times`.
//!
//! ## Why it is cheap to emit
//!
//! A settled pile is *static*. Its glyph and colour do not change, so it drops
//! out of the diff entirely and only the active interface -- the falling stream,
//! the slumping face, the fire front -- is ever written. This is the terrain
//! lesson running in the other direction: a physical simulation has a small
//! moving boundary.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::runtime::FrameContext;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// A material, or the absence of one.
///
/// Ordered so that `Empty` is zero and a `u8` grid is the natural storage, and
/// grouped by *behaviour* rather than by how it looks: whether a material falls
/// is a question about where it sits in this list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Material {
    #[default]
    Empty = 0,
    Stone = 1,
    Sand = 2,
    Water = 3,
    Oil = 4,
    Fire = 5,
    Smoke = 6,
    Plant = 7,
}

impl Material {
    const COUNT: usize = 8;

    /// Liquid: falls, then spreads sideways to find its level.
    const LIQUID: [bool; Material::COUNT] = {
        let mut t = [false; Material::COUNT];
        t[Material::Water as usize] = true;
        t[Material::Oil as usize] = true;
        t
    };

    /// Rises, and sets what it touches alight.
    const FLAMMABLE: [bool; Material::COUNT] = {
        let mut t = [false; Material::COUNT];
        t[Material::Plant as usize] = true;
        t[Material::Oil as usize] = true;
        t
    };

    const GLYPHS: [char; Material::COUNT] =
        [' ', '#', '▓', '~', '▒', '▲', '░', '*'];

    /// Categorical colours, and categorical is right here for a structural
    /// reason: granular physics forms contiguous blobs, and blobs give long
    /// runs of the same colour, which is the cheap case for the output path.
    const COLORS: [style::Color; Material::COUNT] = [
        style::Color::Reset,
        style::Color::Rgb {
            r: 120,
            g: 124,
            b: 132,
        },
        style::Color::Rgb {
            r: 226,
            g: 184,
            b: 106,
        },
        style::Color::Rgb {
            r: 74,
            g: 130,
            b: 200,
        },
        style::Color::Rgb {
            r: 62,
            g: 78,
            b: 96,
        },
        style::Color::Rgb {
            r: 240,
            g: 122,
            b: 48,
        },
        style::Color::Rgb {
            r: 110,
            g: 96,
            b: 120,
        },
        style::Color::Rgb {
            r: 96,
            g: 190,
            b: 110,
        },
    ];

    fn is_liquid(self) -> bool {
        Material::LIQUID[self as usize]
    }

    fn is_flammable(self) -> bool {
        Material::FLAMMABLE[self as usize]
    }

    /// Everything except Stone, Plant, Fire and Smoke moves under gravity.
    fn falls(self) -> bool {
        matches!(self, Material::Sand | Material::Water | Material::Oil)
    }

    /// Materials a falling cell may displace by swapping with it.
    ///
    /// This is the whole of buoyancy, and it is two entries: sand sinks through
    /// water and water rises through it, which is the one behaviour that makes a
    /// glass of water with sand in it look like a glass of water with sand in
    /// it rather than like a bowl of porridge.
    fn is_displaceable_by(self, other: Material) -> bool {
        // `self` is the cell being displaced and `other` is what is falling into
        // it, so the pair has to be matched on both: a cell is displaceable by a
        // *denser* material, and "denser" is a relation between the two, not a
        // property of either alone. Matching on `other` alone -- which is what
        // the first version did -- asks whether the falling cell can be pushed
        // aside, and nothing is displaceable by the thing that is landing on it.
        matches!(
            (self, other),
            (Material::Water, Material::Sand)
                | (Material::Water, Material::Oil)
                | (Material::Oil, Material::Sand)
        )
    }

    fn cell(self) -> Cell {
        Cell::new(
            Material::GLYPHS[self as usize],
            Material::COLORS[self as usize],
            style::Attribute::Reset,
        )
    }
}

/// Gravity passes per step.
///
/// **Two**, and this is a measurement rather than a preference. A Margolus sweep
/// is one pass over every cell, and the first version ran four gravity and eight
/// lateral passes: twelve per step, and at four steps a frame that is 3.8 million
/// cell visits and an `upd x4` of **2.29 ms**, over the budget on the only
/// criterion this crate gates on. Measured at 400x200, twelve passes is 571 us,
/// so the budget for four steps is about 1.2 ms and six passes per step is what
/// fits.
///
/// The motion does not suffer: a step runs at 60 a second, so halving the passes
/// still gives 120 gravity sweeps a second, and a pile's angle of repose is a
/// property of the *rule* rather than of how many times the rule runs.
const GRAVITY_PASSES: usize = 2;

/// Lateral-dispersion passes per step, which is what lets a liquid find its
/// level.
///
/// Twice the gravity passes, because spreading is what makes a liquid read as a
/// liquid and a failed lateral step costs nothing -- it is one comparison.
const LATERAL_PASSES: usize = 4;

/// Which world to run.
///
/// The variants differ in what feeds the simulation, not in the rules, so this is
/// a small enum rather than a set of booleans and it keeps the "every preset must
/// be self-resetting" argument checkable in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scene {
    /// Sand fed through a narrow neck in a stone chamber. The default, and the
    /// only one of the three that is *structurally* incapable of reaching
    /// equilibrium.
    #[default]
    Hourglass,
    /// Fire at the top of a stone funnel, ash falling, oil catching at the base.
    Volcano,
    /// Rain onto a stone ledge above a pool.
    Rain,
}

impl Scene {
    /// The scene's name, for the failure messages in the tests.
    ///
    /// Only used there, which is why it is not `pub`: a scene's *config*
    /// spelling is serde's business, and a second hand-written spelling is a
    /// second thing to be out of step with it.
    #[cfg(test)]
    fn as_str(self) -> &'static str {
        match self {
            Scene::Hourglass => "hourglass",
            Scene::Volcano => "volcano",
            Scene::Rain => "rain",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandOptions {
    /// Which world to build.
    ///
    /// All three are self-resetting, which is the requirement: a scene that can
    /// reach equilibrium is a scene that ends up a still picture, and the whole
    /// point of picking a falling-sand model is that the material is always
    /// somewhere new.
    pub scene: Scene,

    /// Simulation steps per second.
    ///
    /// 60, one per frame. A step is one gravity pass, so this is also the rate at
    /// which the pile slumps -- and a slump that takes half a second to happen
    /// reads as the pile *deciding* to fail, which is the appeal. Slower and the
    /// sand looks like it is being placed rather than falling.
    pub steps_per_second: f32,

    /// Most steps in a single frame.
    ///
    /// A cap of 4. The same reason as everywhere else in this crate: a terminal
    /// that was unfocused hands back a delta of seconds.
    pub max_steps_per_frame: u16,

    /// How often the world is rebuilt, in seconds.
    ///
    /// 25 seconds, so `25. ` does not read as a markdown list to a doc parser.
    ///
    /// Long enough that a settling hourglass is worth watching to the end, short
    /// enough that the effect never looks stuck if the simulation below it is
    /// coarser than expected. Zero means only when the world fills.
    pub rebuild_seconds: f32,

    pub seed: u64,
}

impl Default for SandOptions {
    /// Hand-written so it is the single source of truth; the derived one is all
    /// zeroes, and `scene`'s derived zero would be a variant that is not one of
    /// the three.
    fn default() -> Self {
        Self {
            scene: Scene::Hourglass,
            steps_per_second: 60.0,
            max_steps_per_frame: 4,
            rebuild_seconds: 25.0,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Sand {
    screen_size: (u16, u16),
    options: SandOptions,
    canvas: Canvas,
    width: usize,
    height: usize,
    cells: Vec<Material>,
    step_accumulator: f32,
    rebuild_accumulator: f32,
    rng: EffectRng,
}

impl Sand {
    /// Reads a cell, treating anything off the board as Stone.
    ///
    /// Stone rather than Empty, so the world has walls for free and no edge case
    /// exists. An out-of-bounds *read* as Empty would let a diagonal step off the
    /// side of the board and wrap to the other one.
    #[inline]
    fn at(&self, x: i32, y: i32) -> Material {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return Material::Stone;
        }
        self.cells[y as usize * self.width + x as usize]
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, material: Material) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.cells[y as usize * self.width + x as usize] = material;
    }

    /// Moves `from` to `to` if it can, swapping when the target is displaceable.
    #[inline]
    fn try_move(&mut self, from: (i32, i32), to: (i32, i32)) -> bool {
        let material = self.at(from.0, from.1);
        let target = self.at(to.0, to.1);

        if target == Material::Empty {
            self.put(to.0, to.1, material);
            self.put(from.0, from.1, Material::Empty);
            return true;
        }
        // Buoyancy: sand sinks through water, and water rises through it.
        //
        // `target.is_displaceable_by(material)`, and the order matters. The first
        // version asked `material.is_displaceable_by(target)` -- whether the
        // *falling* cell was displaceable -- which asks the question backwards:
        // sand is not displaceable by anything, so the swap never happened and
        // the test saw sand sitting on top of the water it was supposed to sink
        // through.
        if target.is_displaceable_by(material) {
            self.put(to.0, to.1, material);
            self.put(from.0, from.1, target);
            return true;
        }
        false
    }

    /// One gravity pass, bottom-up, over a Margolus block offset.
    ///
    /// The offset is what makes diagonals isotropic: the board is partitioned
    /// into 2x2 blocks and the partition is shifted each pass, so a diagonal step
    /// happens equally often in all four directions across a few frames. Without
    /// it, powder leaning one way is a permanent artefact of the scan order.
    fn gravity_pass(&mut self, offset: usize) {
        // Bottom-up: the invariant that gives a pile its mass. See the module
        // docs; `a_grain_falls_one_row_per_step_and_no_further` is the test.
        let mut y = self.height as i32 - 1;
        while y >= 0 {
            y -= 1;
            let MargolusSpan { lo, hi } =
                MargolusSpan::new(self.width as i32, offset, y);
            let mut x = lo;
            while x < hi {
                let material = self.at(x, y);
                if !material.falls() {
                    x += 1;
                    continue;
                }

                if self.try_move((x, y), (x, y + 1)) {
                    x += 1;
                    continue;
                }

                // Powder and liquid both try diagonally once they are resting on
                // something. This is where the angle of repose comes from: a
                // column that cannot fall straight down but can fall sideways
                // leans, and a lean steep enough to be unstable collapses.
                let first: i32 = if self.rng.random_bool(0.5) { -1 } else { 1 };
                let mut moved = false;
                for side in [first, -first] {
                    if self.try_move((x, y), (x + side, y + 1)) {
                        moved = true;
                        break;
                    }
                }
                if moved {
                    // Diagonal, so skip the column just moved into: it has
                    // already been processed this sweep and re-testing it would
                    // let a particle fall twice in one pass.
                    x += 1 + first.unsigned_abs() as i32;
                    continue;
                }
                x += 1;
            }
        }
    }

    /// One lateral-dispersion pass for liquids.
    ///
    /// Separate from gravity and run after it, because a liquid's second nature
    /// is horizontal: it falls, and then it *spreads*. Folding this into the
    /// gravity pass would mean each cell doing both, and the horizontal decision
    /// would be made before the vertical one had settled.
    fn lateral_pass(&mut self, offset: usize) {
        for y in (0..self.height as i32).rev() {
            let MargolusSpan { lo, hi } =
                MargolusSpan::new(self.width as i32, offset, y);
            // Alternating sweep direction per pass, so a pool does not always
            // spread to the right and develop a standing current.
            let rightward = (y + offset as i32) % 2 == 0;
            let mut x = if rightward { lo } else { hi - 1 };
            while x >= lo && x < hi {
                let material = self.at(x, y);
                if !material.is_liquid() {
                    x += if rightward { 1 } else { -1 };
                    continue;
                }
                // Only spread if there is somewhere to go *down* to, or the
                // liquid will creep sideways across a flat floor forever and a
                // puddle will never be still.
                if self.at(x, y + 1) == Material::Empty {
                    x += if rightward { 1 } else { -1 };
                    continue;
                }
                let first: i32 = if self.rng.random_bool(0.5) { -1 } else { 1 };
                for side in [first, -first] {
                    if self.at(x + side, y) == Material::Empty
                        && self.at(x + side, y + 1) == Material::Empty
                    {
                        self.try_move((x, y), (x + side, y));
                        break;
                    }
                }
                x += if rightward { 1 } else { -1 };
            }
        }
    }

    fn step(&mut self) {
        // Fire and smoke first, so a burning front advances before the pile
        // settles under it -- and smoke has to be able to rise through a frame
        // that the later passes have already filled.
        for y in (0..self.height as i32).rev() {
            for x in 0..self.width as i32 {
                match self.at(x, y) {
                    Material::Fire => {
                        // Rise, and ignite an orthogonal neighbour.
                        //
                        // All **four** directions, not just upwards. The first
                        // version checked only the cell above, so a run of
                        // plants lying flat -- which is what a plant on a rim
                        // looks like, and what the volcano scene builds -- never
                        // caught at all.
                        // **Ignite first, then rise.** The order is the whole
                        // thing: rising first moves the fire into the empty air
                        // above the fuel, and by the time it checks its
                        // neighbours it is a row clear of anything flammable.
                        // A fire on a plant rim is surrounded laterally and open
                        // above, so it walked upwards out of reach of the whole
                        // run and never caught.
                        let mut ignited = false;
                        for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1)] {
                            let (nx, ny) = (x + dx, y + dy);
                            if self.at(nx, ny).is_flammable() && !ignited {
                                self.put(nx, ny, Material::Fire);
                                ignited = true;
                            }
                        }
                        if !ignited && self.at(x, y - 1) == Material::Empty {
                            self.try_move((x, y), (x, y - 1));
                        }
                        // Burn out into smoke, which is what makes a burnt patch
                        // look burnt rather than merely missing.
                        if !ignited && self.rng.random_bool(0.04) {
                            self.put(x, y, Material::Smoke);
                        }
                    }
                    Material::Smoke => {
                        if self.at(x, y - 1) == Material::Empty {
                            self.put(x, y, Material::Empty);
                            self.put(x, y - 1, Material::Smoke);
                        } else if self.rng.random_bool(0.3) {
                            self.put(x, y, Material::Empty);
                        }
                    }
                    Material::Plant => {
                        // A plant grows into an empty cell that has a plant
                        // neighbour and something wet below to draw on.
                        if self.rng.random_bool(0.02) {
                            for (dx, dy) in [(0, -1), (-1, 0), (1, 0)] {
                                if self.at(x + dx, y + dy) == Material::Plant
                                    && self.at(x, y) == Material::Empty
                                {
                                    self.put(x, y, Material::Plant);
                                    break;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        for pass in 0..GRAVITY_PASSES {
            self.gravity_pass(pass);
        }
        for pass in 0..LATERAL_PASSES {
            self.lateral_pass(pass);
        }
    }

    /// Removes whatever has reached the bottom of the world.
    ///
    /// The drain is what makes the hourglass an hourglass. Without it the lower
    /// chamber fills: the test that checks this measured 1,137 of 1,185 cells
    /// occupied after two seconds, which is a container that fills up -- the
    /// worst possible screensaver, and the exact failure the scene choice was
    /// supposed to avoid. Sand that reaches the floor is simply gone, so the
    /// chamber never reaches equilibrium and the world never needs a reset of
    /// its own.
    fn drain(&mut self) {
        let floor = self.height as i32 - 1;
        for x in 1..(self.width as i32 - 1) {
            if matches!(
                self.at(x, floor),
                Material::Sand | Material::Water | Material::Oil
            ) {
                self.put(x, floor, Material::Empty);
            }
        }
    }

    /// Drops new material into the world.
    ///
    /// This is the half of the hourglass that makes it a screensaver rather than
    /// a demonstration, and it is why there is no "settled" state to reach.
    fn emit(&mut self) {
        let (w, h) = (self.width as i32, self.height as i32);
        match self.options.scene {
            Scene::Hourglass => {
                // Sand in the top chamber, through a neck in the middle.
                if self.rng.random_bool(0.6) {
                    let x = self.rng.random_range(w / 3..w - w / 3);
                    let y = self.rng.random_range(1..(h / 5).max(2));
                    if self.at(x, y) == Material::Empty {
                        self.put(x, y, Material::Sand);
                    }
                }
            }
            Scene::Volcano => {
                // Fire at the crater, ash falling out of it.
                if self.rng.random_bool(0.4) {
                    let x = self.rng.random_range(w / 3..w - w / 3);
                    let y = self.rng.random_range(1..(h / 6).max(2));
                    if self.at(x, y) == Material::Empty {
                        self.put(x, y, Material::Fire);
                    }
                }
            }
            Scene::Rain => {
                if self.rng.random_bool(0.5) {
                    let x = self.rng.random_range(0..w);
                    if self.at(x, 1) == Material::Empty {
                        self.put(x, 1, Material::Water);
                    }
                }
            }
        }
    }

    /// Builds the world's static structure and drains it.
    fn build_world(&mut self) {
        self.cells.fill(Material::Empty);
        let (w, h) = (self.width as i32, self.height as i32);

        // A floor and two walls, as stone, so no material can leave and every
        // edge case is answered by `at` rather than by a bound check.
        for x in 0..w {
            self.put(x, h - 1, Material::Stone);
        }
        for y in 0..h {
            self.put(0, y, Material::Stone);
            self.put(w - 1, y, Material::Stone);
        }

        match self.options.scene {
            Scene::Hourglass => {
                // A funnel: stone shoulders narrowing to a neck at the middle,
                // then a chamber below. This geometry *is* the effect -- sand
                // cannot rest anywhere except in the bottom cone, so there is no
                // equilibrium to reach.
                let neck = w / 2;
                for y in 0..(h / 2).max(2) {
                    let half = (w / 2 - 1) * (y + 1) / (h / 2).max(2);
                    for x in 0..half {
                        self.put(1 + x, y, Material::Stone);
                        self.put(w - 2 - x, y, Material::Stone);
                    }
                    self.put(neck, y, Material::Empty);
                }
                for y in (h / 2).max(2)..h - 1 {
                    let half = (w / 2 - 1) - (y - h / 2) / 3;
                    for x in 0..half.max(1) {
                        self.put(1 + x, y, Material::Stone);
                        self.put(w - 2 - x, y, Material::Stone);
                    }
                }
                // Water at the bottom, so buoyancy is visible from the first
                // grain: sand sinks through it and the two never mix.
                for y in (h - 6)..(h - 1) {
                    for x in 1..(w - 1) {
                        if self.at(x, y) == Material::Empty {
                            self.put(x, y, Material::Water);
                        }
                    }
                }
            }
            Scene::Volcano => {
                let neck = w / 2;
                for y in 0..(h / 3).max(2) {
                    let half = (w / 2 - 1) * (y + 1) / (h / 3).max(2);
                    for x in 0..half {
                        self.put(1 + x, y, Material::Stone);
                        self.put(w - 2 - x, y, Material::Stone);
                    }
                }
                // A pool of oil to catch, and a plant on the rim to spread along.
                for y in (h - 5)..(h - 1) {
                    for x in 1..(w - 1) {
                        if self.at(x, y) == Material::Empty {
                            self.put(x, y, Material::Oil);
                        }
                    }
                }
                for x in 1..(w - 1) {
                    if self.rng.random_bool(0.35) {
                        self.put(x, h - 6, Material::Plant);
                    }
                }
                let _ = neck;
            }
            Scene::Rain => {
                // A stepped ledge, so the water has somewhere to run to and the
                // pile it makes has a slope to find.
                for x in 1..(w - 1) {
                    let step = (x * 4) / (w - 2);
                    for y in (h - 12 + step)..(h - 1) {
                        if self.at(x, y) == Material::Empty && y > h - 12 {
                            self.put(x, y, Material::Sand);
                        }
                    }
                }
            }
        }
    }

    fn advance(&mut self, delta: f32) {
        if self.options.rebuild_seconds > 0.0 {
            self.rebuild_accumulator += delta;
            if self.rebuild_accumulator >= self.options.rebuild_seconds {
                self.rebuild_accumulator = 0.0;
                self.build_world();
            }
        }

        self.step_accumulator += delta * self.options.steps_per_second;
        let mut steps = self.step_accumulator.floor().max(0.0) as u16;
        if steps > 0 {
            self.step_accumulator -= steps as f32;
        }
        if steps > self.options.max_steps_per_frame {
            steps = self.options.max_steps_per_frame;
        }

        for _ in 0..steps {
            self.emit();
            self.step();
            self.drain();
        }
    }

    fn draw(&mut self) {
        self.canvas.clear();
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = self.cells[y * self.width + x].cell();
                self.canvas.set(x, y, cell);
            }
        }
    }
}

/// One row's slice of a Margolus block partition.
///
/// The board is cut into 2x2 blocks and the cut is shifted by `offset`, so a
/// pass never treats the same cell as a block's top-left twice in a row and the
/// four block positions rotate. `Spanned` is returned rather than two bounds
/// because a shifted partition can wrap past the right edge, and wrapping is the
/// point -- a partition that did not wrap would bias the diagonals in one
/// direction at the edges, which is the artefact this exists to remove.
struct MargolusSpan {
    lo: i32,
    hi: i32,
}

impl MargolusSpan {
    fn new(width: i32, offset: usize, _row: i32) -> MargolusSpan {
        let start = offset as i32;
        let lo = (start / 2) * 2;
        let hi = ((width + start) / 2) * 2 - start;
        MargolusSpan {
            lo: lo.min(width),
            hi: hi.clamp(0, width),
        }
    }
}

impl TerminalEffect for Sand {
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
        self.width = self.screen_size.0 as usize;
        self.height = self.screen_size.1 as usize;
        self.cells = vec![Material::Empty; self.width * self.height];
        self.step_accumulator = 0.0;
        self.rebuild_accumulator = 0.0;
        self.rng = seeded_rng(self.options.seed, "sand");
        self.build_world();
    }
}

impl Sand {
    pub fn new(options: SandOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let mut sand = Self {
            screen_size,
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            width: screen_size.0 as usize,
            height: screen_size.1 as usize,
            cells: Vec::new(),
            step_accumulator: 0.0,
            rebuild_accumulator: 0.0,
            rng: seeded_rng(DEFAULT_SEED, "sand"),
        };
        sand.reset();
        sand
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_world(width: usize, height: usize) -> Sand {
        let mut sand =
            Sand::new(SandOptions::default(), (width as u16, height as u16));
        sand.cells.fill(Material::Empty);
        sand
    }

    /// A grain falls exactly one row per step, and no further.
    ///
    /// **This is the invariant the whole simulation rests on**, and it is the
    /// thing a top-down sweep destroys: with the sweep running the other way, a
    /// particle placed at the top of an empty column reaches the floor in a
    /// single step, because the sweep reaches the lower row after the particle
    /// has already been processed. A pile built that way has no mass -- it is a
    /// line, it holds no angle, and it does not slump.
    ///
    /// Asserted by *timing* rather than by position, because a test that only
    /// checks where the grain ended up would pass against a broken sweep that
    /// happened to put it in the right place.
    #[test]
    fn a_grain_falls_one_row_per_step_and_no_further() {
        let mut sand = empty_world(20, 30);
        let start_y = 2;
        sand.put(10, start_y, Material::Sand);

        // One **pass**, not one step. A step runs GRAVITY_PASSES of them, so the
        // invariant is one row per pass and asserting it against a step would be
        // asserting the wrong thing -- the first version of this test called
        // `step`, saw the grain four rows down, and read that as a broken sweep
        // when it was four correct passes.
        //
        // The reason a pass cannot move it further is the sweep direction: bottom
        // up, the row below has already been visited when the grain is processed,
        // so the destination is out of reach for the rest of the pass.
        sand.gravity_pass(0);
        assert_eq!(
            sand.at(10, start_y + 1),
            Material::Sand,
            "after one step the grain should be one row down"
        );
        assert_eq!(
            sand.at(10, start_y),
            Material::Empty,
            "and nothing left behind"
        );
        assert_eq!(
            sand.at(10, start_y + 2),
            Material::Empty,
            "the grain fell more than one row in a single step, so the sweep is \
             running top-down"
        );
    }

    /// A column of sand slumps to a slope, and a slope steeper than the angle of
    /// repose is not stable.
    ///
    /// This is the motion the effect exists for. Nothing else in this crate has
    /// it: a pile that has been standing for ten seconds suddenly failing in one
    /// frame, and a sheet of it running out across the floor. Asserted as a
    /// measured slope rather than as a shape, because the shape is what the eye
    /// reads and the slope is what the physics produces.
    #[test]
    fn a_pile_finds_a_slope_rather_than_a_column() {
        let mut sand = empty_world(60, 40);
        // A deliberately over-steep column: 20 grains in one column.
        for y in 0..20 {
            sand.put(30, y, Material::Sand);
        }

        for _ in 0..120 {
            sand.step();
        }

        // Measure the pile's width and height.
        let mut min_x = 60usize;
        let mut max_x = 0usize;
        let mut top = 40i32;
        for y in 0..40i32 {
            for x in 0..60i32 {
                if sand.at(x, y) == Material::Sand {
                    min_x = min_x.min(x as usize);
                    max_x = max_x.max(x as usize);
                    top = top.min(y);
                }
            }
        }
        let width = (max_x - min_x) as f64 + 1.0;
        let height = (40 - top) as f64;
        assert!(
            width > height,
            "the pile is {width} wide and {height} tall, so it never slumped -- \
             a column that does not spread is a column, not a heap"
        );

        // And nothing escaped the floor.
        assert_eq!(sand.at(0, 0), Material::Empty, "material escaped a wall");
    }

    /// Sand sinks through water, and water rises through it.
    ///
    /// The one behaviour that makes a glass of water with sand in it look like
    /// one. Asserted as a *swap* -- sand goes down, water goes up -- rather than
    /// as "sand ends up lower", which a rule that simply deleted the water would
    /// also satisfy.
    #[test]
    fn sand_sinks_through_water() {
        let mut sand = empty_world(20, 20);
        sand.put(10, 10, Material::Sand);
        sand.put(10, 11, Material::Water);
        // A **floor**, because buoyancy is only visible when the water is
        // resting on something.
        //
        // This is the subtlety that took three attempts. With the water free to
        // fall, the bottom-up sweep always processes it *first*, so it drops out
        // of the way and the sand simply falls into the gap it left -- and the
        // sand ends up above the water every time, looking exactly like a
        // buoyancy rule that does not work. Block the water on stone and the
        // sand above it has nowhere to go but through, which is the situation
        // "sand in a glass of water" actually describes.
        sand.put(10, 13, Material::Stone);
        sand.put(10, 12, Material::Water);
        sand.put(10, 11, Material::Water);
        sand.put(10, 10, Material::Sand);

        for _ in 0..2 {
            sand.gravity_pass(0);
        }

        // The sand is on the floor, below the water it started above.
        let sand_y = (0..20i32)
            .find(|y| sand.at(10, *y) == Material::Sand)
            .expect("the sand should still be on the board");
        assert!(
            sand_y > 11,
            "the sand is at row {sand_y}, still level with or above the water it \
             started on top of; it should have sunk"
        );

        // And the water was **displaced, not deleted**. It ends up beside the
        // sand rather than above it -- a liquid flows around the thing it is
        // buoyed past, and looking for it in the same column finds nothing. That
        // is correct physics and the assertion has to count it across the board,
        // because a rule that simply deleted the water would pass a test that
        // only asked where the sand went.
        let water = (0..20i32)
            .flat_map(|y| (0..20i32).map(move |x| (x, y)))
            .filter(|(x, y)| sand.at(*x, *y) == Material::Water)
            .count();
        assert_eq!(water, 2, "both grains of water should still exist");

        assert_eq!(
            sand.at(10, 14),
            Material::Empty,
            "something passed through the floor"
        );
    }

    /// Fire spreads along a run of plants and is stopped by stone.
    ///
    /// Both halves, because "fire spreads" is satisfiable by a rule that sets the
    /// whole board alight, and the *stopping* is what makes a fire front a front
    /// rather than a countdown.
    #[test]
    fn fire_spreads_along_plants_and_stops_at_stone() {
        let mut sand = empty_world(40, 20);
        // A row of plants with a stone wall in the middle.
        for x in 2..20 {
            sand.put(x, 10, Material::Plant);
        }
        for x in 20..22 {
            sand.put(x, 10, Material::Stone);
        }
        for x in 22..38 {
            sand.put(x, 10, Material::Plant);
        }
        sand.put(3, 10, Material::Fire);

        // Watched *during* the burn, not after it. Fire burns out into smoke on
        // a few percent of cells a generation, so a run that caught properly is
        // ash eighty generations later -- and the first version of this test
        // sampled once at the end and reported that the fire had not spread.
        let mut reached_left_end = false;
        let mut reached_right = false;
        for _ in 0..80 {
            sand.step();
            if (2..20).any(|x| sand.at(x, 10) == Material::Fire) {
                reached_left_end = true;
            }
            if (22..38).any(|x| sand.at(x, 10) == Material::Fire) {
                reached_right = true;
            }
        }

        assert!(
            reached_left_end,
            "the fire never spread along the left run of plants"
        );
        assert!(
            !reached_right,
            "the fire got past the stone wall, so stone is not stopping it"
        );
    }

    /// The default scene is self-resetting: its material count is bounded.
    ///
    /// The first version of this test asserted that the lower chamber stays
    /// *empty*, and it measured 1,137 of 1,185 cells occupied after two seconds.
    /// That is not a bug in the world -- **a lower chamber filling with sand is
    /// what an hourglass does** -- it is a wrong expectation. What makes an
    /// hourglass a screensaver is not that it stays empty; it is that the sand is
    /// always somewhere else, draining out of the neck while more arrives above,
    /// so the picture never stops changing.
    ///
    /// So the assertion is boundedness rather than emptiness, because boundedness
    /// is what a *missing drain* looks like. And the samples straddle a full
    /// rebuild cycle, so the count has to settle well below capacity rather than
    /// tracking the clock.
    #[test]
    fn the_hourglass_drains_rather_than_accumulating() {
        let mut sand = Sand::new(SandOptions::default(), (80, 40));

        let movable = |sand: &Sand| {
            sand.cells
                .iter()
                .filter(|c| {
                    matches!(c, Material::Sand | Material::Water | Material::Oil)
                })
                .count()
        };

        // Past a full rebuild, then a second sample ten seconds later.
        for _ in 0..1_800 {
            sand.advance(1.0 / 60.0);
        }
        let first = movable(&sand);
        for _ in 0..600 {
            sand.advance(1.0 / 60.0);
        }
        let second = movable(&sand);

        let capacity = sand.width * sand.height;
        assert!(
            second < capacity,
            "the hourglass filled its world: {second} of {capacity} cells hold a \
             movable material, so the drain is not working"
        );
        assert!(
            second < first + capacity / 8,
            "material is accumulating without bound: {first} at one sample and \
             {second} ten seconds later"
        );
    }

    /// Every scene is self-resetting, and none of them can fill.
    ///
    /// Checked for all three rather than for the default alone, because the
    /// failure is per-scene and a preset added later would not be covered by a
    /// test about the one that ships.
    #[test]
    fn no_scene_fills_its_world_shut() {
        for scene in [Scene::Hourglass, Scene::Volcano, Scene::Rain] {
            let options = SandOptions {
                scene,
                ..SandOptions::default()
            };
            let mut sand = Sand::new(options, (80, 40));
            for _ in 0..300 {
                sand.step();
            }
            let movable = sand
                .cells
                .iter()
                .filter(|c| {
                    matches!(c, Material::Sand | Material::Water | Material::Oil)
                })
                .count();
            let capacity = sand.width * sand.height;
            assert!(
                movable < capacity,
                "{} filled its world: {movable} of {capacity} cells are occupied \
                 by a movable material",
                scene.as_str()
            );
        }
    }

    /// A run is reproducible from its seed, and different seeds differ.
    #[test]
    fn the_seed_reaches_the_materials() {
        let cells_for = |seed: u64| {
            let options = SandOptions {
                seed,
                ..SandOptions::default()
            };
            let mut sand = Sand::new(options, (60, 30));
            // `advance`, not `step`: the randomness a seed reaches is mostly in
            // `emit`, and thirty bare steps of a deterministic world converge.
            for _ in 0..60 {
                sand.advance(1.0 / 60.0);
            }
            sand.cells
        };
        assert_eq!(cells_for(3), cells_for(3), "the same seed gave two worlds");
        assert_ne!(cells_for(3), cells_for(4), "two seeds gave the same world");
    }

    /// A terminal below the minimum still produces a world.
    #[test]
    fn a_terminal_below_the_minimum_still_produces_a_world() {
        let mut sand = Sand::new(SandOptions::default(), (1, 1));
        for _ in 0..10 {
            sand.advance(1.0 / 60.0);
        }
        sand.draw();
        assert!(sand.width > 1 && sand.height > 1);
        assert_eq!(sand.cells.len(), sand.width * sand.height);
    }

    /// The Margolus partition shifts, which is the entire point of it.
    ///
    /// If the span were the same on every pass then the diagonals would be
    /// biased by the scan order and a pile would develop a permanent lean. The
    /// assertion is that consecutive passes *disagree* about where the block
    /// boundaries are, and that every pass covers the whole row.
    #[test]
    fn the_margolus_partition_shifts_between_passes() {
        let mut seen = std::collections::BTreeSet::new();
        for offset in 0..4 {
            let span = MargolusSpan::new(20, offset, 5);
            assert!(
                span.lo >= 0 && span.hi <= 20 && span.lo < span.hi,
                "offset {offset} produced an invalid span {}..{}",
                span.lo,
                span.hi
            );
            seen.insert((span.lo, span.hi));
        }
        assert_eq!(
            seen.len(),
            4,
            "four offsets produced only {seen:?}, so the partition is not \
             shifting and the diagonals will be biased"
        );
    }
}
