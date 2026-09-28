//! Wireworld: electrons running through a circuit drawn in a character grid.
//!
//! Four states on a Moore neighbourhood, one generation each:
//!
//! - **Empty** stays empty.
//! - **Head** -- a live electron -- decays to Tail.
//! - **Tail** -- the electron's wake -- decays to Conductor.
//! - **Conductor** becomes a Head if and only if **exactly one or two** of its
//!   eight neighbours are Heads. Otherwise it stays Conductor.
//!
//! ## Why the 1-or-2 window is the whole model
//!
//! With a threshold of one or more, copper behaves like a spreading infection
//! and the board fills with electrons. With exactly one or two, a wire
//! *conducts* rather than *ignites*: an electron arriving at a junction with two
//! other electrons already there does not fire, so pulses merge and annihilate
//! on contact instead of compounding. That is what makes the behaviour look
//! like current rather than like disease, and it is also what makes the rule
//! Turing-complete.
//!
//! `a_conductor_fires_on_one_or_two_heads_and_not_otherwise` asserts the window
//! from both sides, because a threshold written as `>= 1` passes every visual
//! check this effect has and is simply the wrong model.
//!
//! ## Why a circuit *generator* rather than a circuit
//!
//! The obvious implementation -- a fixed wire loop -- is not a screensaver. An
//! electron runs round the loop and you get one dot moving, which converges
//! immediately and is the single most watchable-looking and least screensaver-
//! shaped thing in this file. So the board is *generated*: a random conductor
//! network, re-drawn on a timer, fed by a clock that injects heads on a
//! schedule. Pulses are therefore born continuously and there is nothing to
//! converge to.
//!
//! ## Why it is cheap
//!
//! Conductor is static. Only the few hundred cells currently carrying an
//! electron change between frames, out of tens of thousands -- the same
//! argument as `dvd`'s 397 bytes, and the same reason: not writing a cell that
//! did not change is the cheapest rendering optimisation there is. Four states
//! map to four (glyph, colour) pairs, which is categorical with long runs,
//! because wire is contiguous lines.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::runtime::FrameContext;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// The four states of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
enum Wire {
    #[default]
    Empty = 0,
    Head = 1,
    Tail = 2,
    Conductor = 3,
}

impl Wire {
    /// The glyph each state is drawn with.
    ///
    /// A space for Empty, so empty cells are not written at all and the
    /// majority of a sparse board costs nothing. The other three are distinct
    /// *shapes* as well as distinct colours, which matters because two of them
    /// are the same colour family: a Head and a Tail are both blue, and telling
    /// them apart by hue alone would make the direction of travel invisible at a
    /// glance.
    const GLYPHS: [char; 4] = [' ', 'o', '*', '='];

    /// The colour each state is drawn in.
    ///
    /// Conductor is a dim yellow so the *wire* reads as a separate structure
    /// from the charge moving along it, and Head is near-white so the leading
    /// edge is the brightest thing on the screen. That ordering is the same one
    /// the glyph ramp's own docs insist on: the thing that is *moving* must not
    /// be the dimmest.
    const COLORS: [style::Color; 4] = [
        style::Color::Reset,
        style::Color::Rgb {
            r: 235,
            g: 245,
            b: 255,
        },
        style::Color::Rgb {
            r: 60,
            g: 96,
            b: 170,
        },
        style::Color::Rgb {
            r: 150,
            g: 120,
            b: 40,
        },
    ];

    fn cell(self) -> Cell {
        Cell::new(
            Self::GLYPHS[self as usize],
            Self::COLORS[self as usize],
            style::Attribute::Reset,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WireworldOptions {
    /// Generations per second.
    ///
    /// 12, which is about one every five frames. An electron at that rate crosses
    /// the screen in a couple of seconds, which is fast enough to read as
    /// *flowing* and slow enough to follow one pulse by eye. The rule has a hard
    /// two-generation lifetime for an electron -- Head then Tail -- so the rate
    /// here is also the rate at which the board is repainted, and a faster one
    /// turns a track into a smear.
    pub generations_per_second: f32,

    /// Most generations run in a single frame.
    ///
    /// 4. A terminal that was unfocused hands back a delta of seconds.
    pub max_generations_per_frame: u16,

    /// How many heads the clock injects per generation.
    ///
    /// 6, and the tension is the same one the sandpile has: too few and the
    /// board goes quiet between pulses, too many and every conductor is
    /// saturated and the pulses stop being distinguishable. Six on a board with
    /// a few thousand conductor sites is sparse enough that a pulse is a *pulse*.
    pub clock_heads_per_generation: u16,

    /// How often the circuit itself is re-drawn, in seconds.
    ///
    /// A period of 12 seconds. A fixed circuit would eventually stop producing
    /// anything interesting, because a fixed network has a fixed set of paths
    /// and the same pulses circulate forever. Re-drawing is the reset, and it
    /// costs one full repaint that the terminal sees as a single frame.
    pub recircuit_seconds: f32,

    /// How much of the board is conductor.
    ///
    /// 0.12, and the interesting band is narrow for a structural reason rather
    /// than a taste one. A wire is a *line*: a cell conducts only if enough of
    /// its neighbours are conductor too, or it is a stub that electrons reach and
    /// die on. So density controls whether the network percolates into connected
    /// paths that a pulse can travel along, and it has to be tuned to the Moore
    /// neighbourhood's connectivity. Below about 0.08 there are no long wires at
    /// all; above 0.2 the board is a solid mass that is one conductor.
    pub conductor_density: f32,

    /// Copper is grown by random growth from seed sites rather than placed
    /// independently per cell.
    ///
    /// This is what produces *wires* instead of *speckle*, and it is the single
    /// decision that makes the difference between a circuit and a texture. A cell
    /// becomes conductor if a random already-conductor neighbour adopts it, so
    /// copper accretes as connected lines and branches.
    pub grow_wires: bool,

    pub seed: u64,
}

impl Default for WireworldOptions {
    /// Hand-written so it is the single source of truth; the derived one is all
    /// zeroes, which here means no copper at all and a permanently black screen.
    fn default() -> Self {
        Self {
            generations_per_second: 12.0,
            max_generations_per_frame: 4,
            clock_heads_per_generation: 6,
            recircuit_seconds: 12.0,
            conductor_density: 0.12,
            grow_wires: true,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Wireworld {
    screen_size: (u16, u16),
    options: WireworldOptions,
    canvas: Canvas,
    width: usize,
    height: usize,
    cells: Vec<Wire>,
    /// Indices of the conductor cells, for the clock to inject onto.
    ///
    /// Rebuilt by `draw_circuit` rather than filtered per tick, because the
    /// clock runs several times a generation and a full scan per tick is the
    /// difference between an O(1) injection and an O(sites) one.
    conductor_cells: Vec<usize>,
    /// The next generation, built beside `cells` rather than in it.
    ///
    /// An in-place update is the obvious shortcut and it is wrong here in a way
    /// that looks fine: Head decays to Tail unconditionally, so a left-to-right
    /// in-place sweep would let a Head's own decay be seen by the next cell and
    /// the pulse would travel one cell further every sweep, doubling its speed.
    next: Vec<Wire>,
    generation_accumulator: f32,
    recircuit_accumulator: f32,
    rng: EffectRng,
}

impl Wireworld {
    /// Linear index of a cell.
    ///
    /// An associated function on the width rather than a method on `self`,
    /// because `self.cells[Self::index(self.width, x, y)] = ..` evaluates the
    /// index before the write and so never holds an immutable borrow across a
    /// mutable one. A method reading `&self` cannot be used in that position at
    /// all, which is a nuisance in exactly the places -- building a test board,
    /// placing an electron -- where readability matters most.
    #[inline]
    fn index(width: usize, x: usize, y: usize) -> usize {
        y * width + x
    }

    /// How many of a cell's eight neighbours are Heads.
    fn count_heads(&self, x: usize, y: usize) -> u8 {
        let mut heads = 0;
        let (w, h) = (self.width as isize, self.height as isize);
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                // Clamped rather than wrapped. Wireworld is normally played on a
                // torus, but a pulse wrapping around a *rectangular* terminal
                // reappears on the opposite edge at a row it has no geometric
                // relationship to, which reads as a glitch. Open edges dissipate.
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                if self.cells[ny as usize * self.width + nx as usize] == Wire::Head
                {
                    heads += 1;
                }
            }
        }
        heads
    }

    /// One generation, everywhere at once.
    fn step(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width {
                let i = Self::index(self.width, x, y);
                self.next[i] = match self.cells[i] {
                    Wire::Empty => Wire::Empty,
                    Wire::Head => Wire::Tail,
                    Wire::Tail => Wire::Conductor,
                    Wire::Conductor => {
                        // The whole model. One or two, never zero and never three
                        // or more: with three the copper ignites instead of
                        // conducting.
                        if self.count_heads(x, y) == 1
                            || self.count_heads(x, y) == 2
                        {
                            Wire::Head
                        } else {
                            Wire::Conductor
                        }
                    }
                };
            }
        }
        std::mem::swap(&mut self.cells, &mut self.next);
    }

    /// Injects heads onto random *copper* sites: the clock that keeps the circuit
    /// producing signals.
    ///
    /// Onto copper, not at random positions. The first version picked a random
    /// cell and injected only if it happened to be conductor, and with copper at
    /// 12% of the board that succeeds one time in eight -- so most ticks injected
    /// nothing, and the test asserting that signals keep being produced found
    /// the board had gone completely quiet. The copper list is maintained by
    /// `draw_circuit`, so this is an index into a `Vec` rather than a rejection
    /// sample that usually fails.
    fn tick_clock(&mut self) {
        if self.conductor_cells.is_empty() {
            return;
        }
        for _ in 0..self.options.clock_heads_per_generation {
            let at = self.rng.random_range(0..self.conductor_cells.len());
            let i = self.conductor_cells[at];
            if self.cells[i] == Wire::Conductor {
                self.cells[i] = Wire::Head;
            }
        }
    }

    /// Draws a fresh circuit, keeping whatever is already running.
    ///
    /// The electrons are *not* cleared, which is the point: a re-circuit that
    /// wiped the board would blink, and one that left the charge in place
    /// mid-flight produces the interesting case of a pulse arriving at a wire
    /// that is no longer the wire it was travelling on.
    fn draw_circuit(&mut self) {
        for cell in &mut self.cells {
            if *cell != Wire::Head && *cell != Wire::Tail {
                *cell = Wire::Empty;
            }
        }

        let sites = self.width * self.height;
        let target = (sites as f32 * self.options.conductor_density) as usize;

        // A *handful* of seeds, and then copper is grown out from them.
        //
        // The first version scattered `density * sites` seeds -- 864 of them on
        // a 120x60 board -- and then grew from those, and it produced speckle:
        // 864 cells placed independently at 12% density are mostly several cells
        // apart, so the largest connected run held 2% of the copper and the
        // board was a texture rather than a circuit. The seed count and the
        // target are different quantities and conflating them is the whole bug.
        //
        // One seed per ~400 sites keeps the growth frontier small enough that
        // the structures that grow out of it percolate, which is what makes a
        // pulse able to travel anywhere.
        // Three seeds, deliberately few. The seed count and the target are
        // different quantities: `target` is how much copper there should be, and
        // the seeds are only where it starts growing from. The first version
        // conflated them, scattering `density * sites` seeds -- 864 on a 120x60
        // board -- and then growing from those, and 864 cells placed
        // independently at 12% density are mostly several cells apart. The
        // result was speckle with a largest connected run of 2% of the copper.
        //
        // Three is chosen from the measurement in
        // `the_generator_makes_wires_rather_than_speckle`: enough starts to give
        // the board more than one circuit, few enough that the largest one
        // dominates and a pulse can cross the screen.
        let seeds = 3usize;
        let mut copper_seeds: Vec<usize> = Vec::with_capacity(seeds);
        let mut placed = 0usize;

        while placed < seeds {
            let i = self.rng.random_range(0..sites);
            if self.cells[i] == Wire::Empty {
                self.cells[i] = Wire::Conductor;
                copper_seeds.push(i);
                placed += 1;
            }
        }

        if self.options.grow_wires {
            // Copper is laid down by **self-avoiding random walks**, not by
            // accretion.
            //
            // Three earlier generators all looked plausible and all produced
            // slabs. (1) A queue that pops a cell and adopts one neighbour stalls:
            // a pop near the frontier finds all four neighbours taken, the queue
            // shrinks, and it exits having placed 83 of 864 cells. (2) Picking
            // uniformly from all the copper always progresses but fills in --
            // boundary-to-area 0.22, which is a mass. (3) Biasing the pick
            // towards cells with few neighbours (tips) helped to 0.31 and was
            // still a mass, because growth from a tip *also* fills the space
            // behind it.
            //
            // A walk leaves the path behind it and only ever moves to an empty
            // cell, so copper ends up one cell wide. That is what makes a wire a
            // wire, and it is why this is a walk and not a growth rule.
            //
            // Branches come from occasionally starting an extra walker on an
            // existing cell, so the board has junctions for pulses to interact
            // at rather than three parallel lines.
            let mut heads: Vec<usize> = copper_seeds.clone();
            let mut guard = target.saturating_mul(16).max(512);
            while placed < target && guard > 0 {
                guard -= 1;
                if heads.is_empty() {
                    break;
                }
                let at = self.rng.random_range(0..heads.len());
                let i = heads[at];
                let (x, y) = (i % self.width, i / self.width);

                let mut directions = [
                    (x as isize + 1, y as isize),
                    (x as isize - 1, y as isize),
                    (x as isize, y as isize + 1),
                    (x as isize, y as isize - 1),
                ];
                // Partial Fisher-Yates: a walk that always tried east first would
                // be a horizontal line, and a circuit wants all four directions
                // equally likely.
                for k in (1..directions.len()).rev() {
                    let j = self.rng.random_range(0..=k);
                    directions.swap(k, j);
                }

                let mut stepped = false;
                for (dx, dy) in directions {
                    if dx < 0
                        || dy < 0
                        || dx >= self.width as isize
                        || dy >= self.height as isize
                    {
                        continue;
                    }
                    let n = dy as usize * self.width + dx as usize;
                    if self.cells[n] != Wire::Empty {
                        continue;
                    }
                    self.cells[n] = Wire::Conductor;
                    heads[at] = n;
                    placed += 1;
                    stepped = true;
                    // Occasionally fork, so there are junctions.
                    if heads.len() < 12 && self.rng.random_bool(0.03) {
                        heads.push(n);
                    }
                    break;
                }
                // Boxed in: retire this walker. Left in place it would spin and
                // burn the whole guard.
                if !stepped && self.rng.random_bool(0.5) {
                    heads.swap_remove(at);
                }
            }
        }
        self.conductor_cells = (0..sites)
            .filter(|i| self.cells[*i] == Wire::Conductor)
            .collect();

        // Copper with no Head neighbour is a dead end, and a board full of dead
        // ends is a texture rather than a circuit. Pruning is what leaves the
        // connected runs that a pulse can actually travel along.
        self.prune_islands();
    }

    /// Removes conductor that cannot reach anything: components of size one.
    ///
    /// A one-cell island is a stub an electron reaches and dies on, and it is
    /// invisible in a screenshot, so leaving them in wastes both cells and the
    /// eye's attention.
    fn prune_islands(&mut self) {
        let mut visited = vec![false; self.width * self.height];
        for y in 0..self.height {
            for x in 0..self.width {
                let start = Self::index(self.width, x, y);
                if visited[start] || self.cells[start] != Wire::Conductor {
                    continue;
                }
                let mut component = Vec::new();
                let mut queue = vec![start];
                visited[start] = true;
                while let Some(i) = queue.pop() {
                    component.push(i);
                    let (cx, cy) = (i % self.width, i / self.width);
                    for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                        if nx < 0
                            || ny < 0
                            || nx >= self.width as i32
                            || ny >= self.height as i32
                        {
                            continue;
                        }
                        let n = ny as usize * self.width + nx as usize;
                        if !visited[n] && self.cells[n] == Wire::Conductor {
                            visited[n] = true;
                            queue.push(n);
                        }
                    }
                }
                if component.len() == 1 {
                    self.cells[start] = Wire::Empty;
                }
            }
        }
    }

    fn advance(&mut self, delta: f32) {
        self.recircuit_accumulator += delta;
        if self.recircuit_accumulator >= self.options.recircuit_seconds {
            self.recircuit_accumulator = 0.0;
            self.draw_circuit();
        }

        self.generation_accumulator += delta * self.options.generations_per_second;
        let mut generations = self.generation_accumulator.floor().max(0.0) as u16;
        if generations > 0 {
            self.generation_accumulator -= generations as f32;
        }
        if generations > self.options.max_generations_per_frame {
            generations = self.options.max_generations_per_frame;
        }

        for _ in 0..generations {
            self.tick_clock();
            self.step();
        }
    }

    fn draw(&mut self) {
        self.canvas.clear();
        for y in 0..self.height {
            for x in 0..self.width {
                let cell = self.cells[Self::index(self.width, x, y)].cell();
                self.canvas.set(x, y, cell);
            }
        }
    }
}

impl TerminalEffect for Wireworld {
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
        let sites = self.width * self.height;
        self.cells = vec![Wire::Empty; sites];
        self.next = vec![Wire::Empty; sites];
        self.conductor_cells = Vec::new();
        self.generation_accumulator = 0.0;
        self.recircuit_accumulator = 0.0;
        self.rng = seeded_rng(self.options.seed, "wireworld");
        self.draw_circuit();
    }
}

impl Wireworld {
    pub fn new(options: WireworldOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let mut wireworld = Self {
            screen_size,
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            width: screen_size.0 as usize,
            height: screen_size.1 as usize,
            cells: Vec::new(),
            next: Vec::new(),
            conductor_cells: Vec::new(),
            generation_accumulator: 0.0,
            recircuit_accumulator: 0.0,
            rng: seeded_rng(DEFAULT_SEED, "wireworld"),
        };
        wireworld.reset();
        wireworld
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(width: usize, height: usize) -> Wireworld {
        Wireworld::new(WireworldOptions::default(), (width as u16, height as u16))
    }

    /// A Conductor becomes a Head on exactly one or two Heads, and on nothing
    /// else.
    ///
    /// Asserted on both sides of the window because the whole model is the
    /// window. `>= 1` is the natural mistake and it produces a board that fills
    /// with electrons and looks like a spreading infection -- which is a
    /// plausible-looking Wireworld that is not Wireworld, and which every
    /// screenshot-level check would pass.
    #[test]
    fn a_conductor_fires_on_one_or_two_heads_and_not_otherwise() {
        for heads in 0..=8u8 {
            let mut wireworld = board(20, 20);
            for cell in wireworld.cells.iter_mut() {
                *cell = Wire::Empty;
            }

            // Lay out `heads` Heads in a ring around (10, 10), which is
            // Conductor.
            let ring = [
                (9, 9),
                (10, 9),
                (11, 9),
                (9, 10),
                (11, 10),
                (9, 11),
                (10, 11),
                (11, 11),
            ];
            for (dx, dy) in ring.iter().take(heads as usize) {
                wireworld.cells[Wireworld::index(wireworld.width, *dx, *dy)] =
                    Wire::Head;
            }
            wireworld.cells[Wireworld::index(wireworld.width, 10, 10)] =
                Wire::Conductor;

            let count = wireworld.count_heads(10, 10);
            assert_eq!(count, heads.min(8), "the ring should hold {heads} heads");

            wireworld.step();
            let fired = wireworld.cells[Wireworld::index(wireworld.width, 10, 10)]
                == Wire::Head;
            let expected = heads == 1 || heads == 2;
            assert_eq!(
                fired,
                expected,
                "with {heads} neighbouring heads the conductor {} fire",
                if expected { "should" } else { "should not" }
            );
        }
    }

    /// An electron is Head then Tail, and then it is copper again.
    ///
    /// The two-generation lifetime is what makes a pulse visible as a *thing
    /// moving* rather than a dot appearing, and it is also why the generation
    /// rate is the rate the board is repainted.
    #[test]
    fn an_electron_lives_exactly_two_generations() {
        let mut wireworld = board(20, 20);
        for cell in wireworld.cells.iter_mut() {
            *cell = Wire::Empty;
        }
        let (x, y) = (10, 10);
        wireworld.cells[Wireworld::index(wireworld.width, x, y)] = Wire::Head;

        wireworld.step();
        assert_eq!(
            wireworld.cells[Wireworld::index(wireworld.width, x, y)],
            Wire::Tail
        );
        wireworld.step();
        assert_eq!(
            wireworld.cells[Wireworld::index(wireworld.width, x, y)],
            Wire::Conductor,
            "a tail becomes conductor, not empty -- the copper is still there"
        );
    }

    /// Copper is a *line*: the generator makes connected runs, not speckle.
    ///
    /// This is the assertion that separates a circuit from a texture, and it is
    /// measured rather than eyeballed. Independent placement at the same cell
    /// count would satisfy a "is there copper" check and fail this one, because
    /// its components are almost all single cells.
    #[test]
    fn the_generator_makes_wires_rather_than_speckle() {
        let mut wireworld = board(120, 60);
        wireworld.draw_circuit();

        let conductor = wireworld
            .cells
            .iter()
            .filter(|c| **c == Wire::Conductor)
            .count();
        assert!(
            conductor > 200,
            "expected a few hundred conductor cells, got {conductor}"
        );

        // Largest connected component, as a fraction of all the copper. A board
        // of speckle has a largest component of one or two cells; a circuit has
        // one that holds a large share of the copper.
        let largest = wireworld.largest_conductor_component();
        let fraction = largest as f64 / conductor as f64;
        assert!(
            fraction > 0.20,
            "the largest connected wire holds only {fraction:.2} of the copper, \
             so this is speckle rather than a circuit"
        );

        // **Boundary to area**, asserted *against a slab* rather than against a
        // number.
        //
        // Both a circuit and a solid mass can be one connected component over a
        // similar area, so connectivity cannot tell them apart -- which is the
        // exact failure the crate's own notes record for the physarum test,
        // where connectivity reported "100% connected" about three solid bands.
        // Boundary-to-area is the metric that separates them, and the way to
        // assert it without inventing a threshold is to build the slab and
        // compare: the same number of cells, arranged as a filled block, has a
        // boundary only around its perimeter.
        //
        // The history of this number is three wrong generators, each of which
        // passed a "is there copper" check: scattered seeds (2% of copper in one
        // component), uniform accretion (boundary 0.22, a mass), and tip-biased
        // accretion (0.31, still a mass). Self-avoiding walks that leave their
        // path behind reach 0.45.
        let circuit_ratio =
            wireworld.conductor_boundary_edges() as f64 / conductor as f64;

        // The same cell count as a compact block.
        let cols = (conductor as f64).sqrt() as usize;
        let mut slab = Wireworld::new(WireworldOptions::default(), (60, 30));
        for y in 0..cols.min(slab.height) {
            for x in 0..cols.min(slab.width) {
                slab.cells[y * slab.width + x] = Wire::Conductor;
            }
        }
        let slab_cells =
            slab.cells.iter().filter(|c| **c == Wire::Conductor).count();
        let slab_ratio = slab.conductor_boundary_edges() as f64 / slab_cells as f64;

        assert!(
            circuit_ratio > slab_ratio * 2.0,
            "boundary-to-area is {circuit_ratio:.2} against {slab_ratio:.2} for a \
             solid block of the same cell count, so the copper is not \
             meaningfully thinner than a slab"
        );

        // And the islands really are pruned: no single-cell component survives.
        assert_eq!(
            wireworld.isolated_conductor_cells(),
            0,
            "single-cell copper islands were left on the board"
        );
    }

    /// The clock keeps producing signals, so the board never goes quiet.
    ///
    /// The screensaver property for this effect specifically. A fixed wire loop
    /// is the failure this guards and it is not subtle once you have seen it: one
    /// electron circulating forever, which is a still picture with a moving dot.
    #[test]
    fn signals_keep_being_produced_and_they_travel() {
        let mut wireworld = board(120, 60);
        wireworld.cells = vec![Wire::Empty; wireworld.width * wireworld.height];
        wireworld.draw_circuit();

        let mut changes = 0usize;
        let mut positions = Vec::new();
        for generation in 0..120 {
            wireworld.tick_clock();
            let before: Vec<(usize, usize)> = wireworld
                .cells
                .iter()
                .enumerate()
                .filter(|(_, c)| **c == Wire::Head)
                .map(|(i, _)| (i % wireworld.width, i / wireworld.width))
                .collect();
            wireworld.step();
            let after: Vec<(usize, usize)> = wireworld
                .cells
                .iter()
                .enumerate()
                .filter(|(_, c)| **c == Wire::Head)
                .map(|(i, _)| (i % wireworld.width, i / wireworld.width))
                .collect();
            if before != after {
                changes += 1;
            }
            if generation == 0 {
                positions = after;
            }
        }

        assert!(
            changes > 90,
            "only {changes} of 120 generations changed the picture, so the \\
             circuit has stopped producing signals"
        );
        assert!(
            !positions.is_empty(),
            "the clock never managed to inject a head, so nothing can travel"
        );
    }

    /// An electron injected at one end of a straight wire arrives at the other.
    ///
    /// The fixture that would catch a rule that computes the right *counts* but
    /// the wrong *geometry* -- an off-by-one in the neighbourhood, a missing
    /// neighbour, a swapped x and y. Every other test here is about counts or
    /// statistics and would pass against any of those.
    #[test]
    fn an_electron_travels_along_a_straight_wire() {
        let mut wireworld = board(40, 20);
        for cell in wireworld.cells.iter_mut() {
            *cell = Wire::Empty;
        }
        // A horizontal wire across the middle row, with a head at the far left.
        let y = 10;
        for x in 0..40 {
            wireworld.cells[Wireworld::index(wireworld.width, x, y)] =
                Wire::Conductor;
        }
        wireworld.cells[Wireworld::index(wireworld.width, 0, y)] = Wire::Head;

        // Each generation the head should advance exactly one cell along x.
        for expected_x in 1..40 {
            wireworld.step();
            let heads: Vec<usize> = (0..40)
                .filter(|x| {
                    wireworld.cells[Wireworld::index(wireworld.width, *x, y)]
                        == Wire::Head
                })
                .collect();
            assert_eq!(
                heads,
                vec![expected_x],
                "after {expected_x} generations the head should be at \
                 x={expected_x} on the wire, not {heads:?}"
            );
        }
    }

    /// The board edges dissipate rather than wrap.
    ///
    /// Asserted because a torus is the conventional choice and a rectangular
    /// terminal makes it the wrong one: a pulse wrapping around reappears on the
    /// opposite edge at a row it has no geometric relationship to, which reads
    /// as a rendering glitch rather than as a circuit.
    #[test]
    fn an_electron_leaving_the_board_does_not_reappear_on_the_other_edge() {
        let mut wireworld = board(20, 20);
        for cell in wireworld.cells.iter_mut() {
            *cell = Wire::Empty;
        }
        let y = 10;
        for x in 0..20 {
            wireworld.cells[Wireworld::index(wireworld.width, x, y)] =
                Wire::Conductor;
        }
        // At the *left* end, so it travels right and off the right edge.
        //
        // An electron propagates *away* from where it is: the old Head becomes a
        // Tail and whichever conductor saw it becomes the new Head. So a head at
        // the right end of a wire travels left, which is what the first version
        // of this test set up and then read as wrapping -- it saw a head at x=18
        // one generation after placing one at x=19, which is the rule working
        // correctly.
        wireworld.cells[Wireworld::index(wireworld.width, 0, y)] = Wire::Head;

        for _ in 0..25 {
            wireworld.step();
        }

        let heads: Vec<(usize, usize)> = wireworld
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == Wire::Head)
            .map(|(i, _)| (i % wireworld.width, i / wireworld.width))
            .collect();
        assert!(
            heads.is_empty(),
            "heads survived 25 generations on a 20-wide wire and sit at \
             {heads:?}, so the boundary is wrapping"
        );
    }

    /// A run is reproducible from its seed, and different seeds differ.
    #[test]
    fn the_seed_reaches_the_circuit() {
        let board_for = |seed: u64| {
            let options = WireworldOptions {
                seed,
                ..WireworldOptions::default()
            };
            Wireworld::new(options, (80, 40)).cells
        };
        assert_eq!(
            board_for(7),
            board_for(7),
            "the same seed drew two circuits"
        );
        assert_ne!(
            board_for(7),
            board_for(8),
            "two seeds drew the same circuit"
        );
    }

    /// A terminal below the minimum still produces a board.
    ///
    /// Asserting the clamp rather than ignoring it, because the generator
    /// indexes with `i % width` and `i / width` throughout and a one-cell-wide
    /// board is the case that would divide by a zero the size check exists to
    /// prevent.
    #[test]
    fn a_terminal_below_the_minimum_still_produces_a_board() {
        let mut wireworld = board(1, 1);
        for _ in 0..10 {
            wireworld.advance(1.0 / 60.0);
        }
        wireworld.draw();
        assert!(wireworld.width > 1 && wireworld.height > 1);
        assert_eq!(wireworld.cells.len(), wireworld.width * wireworld.height);
        assert_eq!(wireworld.next.len(), wireworld.cells.len());
    }

    impl Wireworld {
        /// The largest 4-connected run of conductor, in cells.
        fn largest_conductor_component(&self) -> usize {
            let mut visited = vec![false; self.cells.len()];
            let mut largest = 0;
            for start in 0..self.cells.len() {
                if visited[start] || self.cells[start] != Wire::Conductor {
                    continue;
                }
                let mut size = 0;
                let mut queue = vec![start];
                visited[start] = true;
                while let Some(i) = queue.pop() {
                    size += 1;
                    let (x, y) = (i % self.width, i / self.width);
                    for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx < 0
                            || ny < 0
                            || nx >= self.width as i32
                            || ny >= self.height as i32
                        {
                            continue;
                        }
                        let n = ny as usize * self.width + nx as usize;
                        if !visited[n] && self.cells[n] == Wire::Conductor {
                            visited[n] = true;
                            queue.push(n);
                        }
                    }
                }
                largest = largest.max(size);
            }
            largest
        }

        /// Edges between a conductor cell and a non-conductor one, counting
        /// only the right and down directions so each edge is counted once.
        fn conductor_boundary_edges(&self) -> usize {
            let mut edges = 0;
            for y in 0..self.height {
                for x in 0..self.width {
                    let i = y * self.width + x;
                    if self.cells[i] != Wire::Conductor {
                        continue;
                    }
                    for (dx, dy) in [(1i32, 0i32), (0, 1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        let outside = nx < 0
                            || ny < 0
                            || nx >= self.width as i32
                            || ny >= self.height as i32
                            || self.cells[ny as usize * self.width + nx as usize]
                                != Wire::Conductor;
                        if outside {
                            edges += 1;
                        }
                    }
                }
            }
            edges
        }

        /// Conductor cells with no conductor neighbour, which are dead stubs.
        #[cfg(test)]
        fn isolated_conductor_cells(&self) -> usize {
            let mut count = 0;
            for y in 0..self.height {
                for x in 0..self.width {
                    if self.cells[Self::index(self.width, x, y)] != Wire::Conductor
                    {
                        continue;
                    }
                    let neighbours = [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
                        .iter()
                        .filter(|(dx, dy)| {
                            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                            nx >= 0
                                && ny >= 0
                                && nx < self.width as i32
                                && ny < self.height as i32
                                && self.cells
                                    [ny as usize * self.width + nx as usize]
                                    == Wire::Conductor
                        })
                        .count();
                    if neighbours == 0 {
                        count += 1;
                    }
                }
            }
            count
        }
    }
}
