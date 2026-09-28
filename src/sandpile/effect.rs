//! The abelian sandpile of Bak, Tang and Wiesenfeld: a board of cells holding
//! 0 to 3 grains, into which grains are dropped one at a time, and which
//! topples itself into a critical state and then avalanches forever.
//!
//! ## Why this model, of all the simulations available
//!
//! A sandpile is a poor screensaver by visual standards and the best one in
//! this crate by a wide margin, and the reason is statistical rather than
//! pictorial. Avalanche size follows a power law: the next drop causes four
//! topples or four thousand, with no way to tell in advance which. Every other
//! simulation here produces a picture whose *shape* is worth looking at, which
//! means a minute in you have seen all of it. This one produces a picture whose
//! *sequence* is unpredictable, which means you cannot.
//!
//! It is also the one model here that needs no tuning to stay interesting. It
//! is not that this effect is configured well; it is that self-organised
//! criticality is an attractor, and a system sitting on its own attractor
//! avalanches indefinitely. Nothing converges, so nothing needs resetting.
//!
//! ## The cost problem, and the shape of the fix
//!
//! Stabilising a drop is the entire cost, and it is not linear. A large
//! avalanche is quadratic in topplings, and a bad drop on a full board can
//! topple most of the grid several times over -- 50,000 to 500,000 topplings,
//! which at a few nanoseconds each is anywhere from a quarter of a millisecond
//! to several.
//!
//! So the topple loop runs against a **per-frame budget**
//! ([`SandpileOptions::max_topples_per_frame`]) and an avalanche that runs out
//! of budget is *left unfinished and resumed on the next frame*. That is not a
//! dropped frame, it is a paused one, and nothing in the picture is wrong
//! while it happens: the avalanche is persistent state, so a partially
//! stabilised board is a real intermediate state of a real avalanche rather
//! than a corrupted one. The pattern here is the same one `maze` and `pipes`
//! already use for their carve and step accumulators, and the effect it buys
//! is that a large avalanche completes in slow motion instead of stalling the
//! frame loop.
//!
//! ## The render
//!
//! Two layers, and the second is the point. The height field is drawn dim and
//! static; cells that toppled during the current drop are lit and decay. The
//! avalanche is then a bright branching fractal on a quiet critical background,
//! and the background is only ever touched by the front -- which is what keeps
//! the diff small. A settled board emits nothing at all.
//!
//! The decay is deliberately **discrete** rather than continuous
//! ([`HEAT_STEPS`]). A continuously fading highlight is a colour change in
//! every lit cell on every frame for the whole second it is visible, and the
//! output path emits a colour whenever it differs from the last one written, so
//! that is exactly the cost this crate keeps re-learning about. Quantising the
//! fade to four steps means each cell is written at most four extra times over
//! its whole flash, whatever the frame rate.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::runtime::FrameContext;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// A site topples at this many grains, and a topple removes exactly this many.
///
/// Four, because the four orthogonal neighbours each receive one.
const TOPPLE_AT: u8 = 4;

/// How many grains a stable site can hold.
const MAX_HEIGHT: u8 = TOPPLE_AT - 1;

/// Steps in an avalanche's fade, and so the number of times a lit cell is
/// rewritten as it dies.
///
/// Four, over [`SandpileOptions::flash_seconds`]. The value is a compromise
/// between a fade that is long enough to read and one that is cheap: each step
/// is one full-screen pass over the heat buffer plus one write per still-lit
/// cell, and every extra step costs that again.
const HEAT_STEPS: u8 = 4;

/// Glyphs for the height field, indexed by height, from empty to full.
///
/// A ramp for a *filled region* must not begin with a space, because the
/// sparsest step lands on the top of the region and a space there makes the
/// region invisible against whatever is behind it. This one does begin with a
/// space, and that is correct here for the opposite reason: at criticality the
/// height distribution is close to uniform over 0 to 3, so there is no "region"
/// with a top edge for the empty cells to outline. They are the majority of the
/// board, and they should recede.
const HEIGHT_GLYPHS: [char; 4] = [' ', '░', '▒', '▓'];

/// Colours for the height field, indexed by height.
///
/// Cool and near-black at zero, warming and brightening with height, so the
/// board reads as ash accumulating: the emptiest ground is the closest to the
/// background and the most saturated ground is the furthest from it.
///
/// Truecolor rather than the eight ANSI colours, and specifically because four
/// *ordered* colours is more than ANSI gives: `DarkGrey`, `Grey` and `White` are
/// three steps, and the fourth would have to reuse one of them and break the
/// ordering the ramp exists to express. Four discrete entries also means four
/// discrete colours per frame, which is the cheap case for the output path.
const HEIGHT_COLORS: [(u8, u8, u8); 4] = [
    (38, 42, 52),
    (84, 96, 114),
    (138, 154, 178),
    (198, 214, 235),
];

/// Glyphs for a lit cell, indexed by remaining heat, dimmest first.
///
/// The flash fades by *thinning* before it dims, so a dying avalanche looks
/// like it is dispersing rather than like a light being switched off.
const HEAT_GLYPHS: [char; 5] = [' ', '░', '▒', '▓', '█'];

/// Colours for a lit cell, indexed by remaining heat, dimmest first.
///
/// Warm, against the height field's cool. The two never share a hue, so a
/// toppling site is unmistakable at any board size -- which matters, because
/// the avalanche front is the only thing on screen that is *moving*.
const HEAT_COLORS: [(u8, u8, u8); 5] = [
    (58, 46, 34),
    (120, 82, 38),
    (188, 132, 52),
    (232, 186, 92),
    (255, 240, 206),
];

/// The upper bound on a cell's transient height, for a debug assertion.
///
/// A site is popped the moment it reaches [`TOPPLE_AT`], so in practice it
/// never much exceeds it. The exact figure is not important; what matters is
/// that it is bounded, because heights are `u8` and a `+= 1` with no bound is
/// how an avalanche becomes a wraparound bug that only shows up in a long run.
const TRANSIENT_HEIGHT_CEILING: u8 = 8;

/// The colour a cleared cell is drawn in. Not used for anything visible -- a
/// height of zero is a space -- but it keeps the cleared board cheap, since a
/// blank matches what the terminal is already showing.
const EMPTY_CELL: Cell = Cell {
    symbol: ' ',
    color: style::Color::Reset,
    bg: style::Color::Reset,
    attr: style::Attribute::Reset,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandpileOptions {
    /// Grains dropped per second, at one drop site chosen uniformly at random.
    ///
    /// 300, and this is set by a tension rather than a preference. The board has
    /// to *reach* criticality, which takes on the order of as many drops as it
    /// has sites, and it also has to be watchable, which wants the drops sparse
    /// enough to follow one avalanche to its edge. At 300 a 400x200 board sees
    /// 7,500 drops in a 25-second playlist slot, which is enough to develop the
    /// branching structure without filling the board, and 300 flashes a second
    /// each living about a second means a few hundred lit cells at any moment
    /// out of 80,000 -- a sparse rain of them, not a solid sheet.
    ///
    /// Raising this does not raise the cost much, because
    /// [`SandpileOptions::max_topples_per_frame`] is what caps the frame; it
    /// raises how often that cap is reached.
    pub drops_per_second: f32,

    /// Most grains dropped in a single frame.
    ///
    /// A cap of 32. A runaway delta, from a terminal that was unfocused or a
    /// machine that was suspended, would otherwise drop thousands of grains at
    /// once and turn a frame into a stall. The cap is high enough that it is
    /// never the thing limiting the effect at its shipped settings.
    pub max_drops_per_frame: u32,

    /// Most topplings performed in a single frame.
    ///
    /// 30,000, which measures about 120 microseconds of toppling at the few
    /// nanoseconds a topple costs, and is set so that the *typical* frame is far
    /// below it and a large avalanche still completes inside one frame most of
    /// the time.
    ///
    /// It exists because the tail is unbounded. Mean avalanche size grows with
    /// the board, but the distribution is a power law, so the largest drop on a
    /// big board is orders of magnitude above the mean and will overrun any
    /// budget you pick. Overrunning is handled rather than prevented: the
    /// avalanche is left unfinished and resumed, so the cost is *bounded* rather
    /// than hoped about, and the visible consequence is that a big avalanche runs
    /// briefly in slow motion.
    pub max_topples_per_frame: u32,

    /// How long an avalanche's flash takes to fade, in seconds.
    ///
    /// 1.0, quantised to [`HEAT_STEPS`] steps. Longer reads as a lingering
    /// glow that merges neighbouring avalanches into one smear; shorter than
    /// about half a second and the eye cannot follow a branch to its tip.
    /// How many seconds an avalanche's flash takes to fade, in seconds.
    ///
    /// 1.0, quantised to [`HEAT_STEPS`] steps. Longer reads as a lingering
    /// glow that merges neighbouring avalanches into one smear; shorter than
    /// about half a second and the eye cannot follow a branch to its tip.
    pub flash_seconds: f32,

    /// How many grains to drop while the board is being brought up, per site.
    ///
    /// This is the difference between an effect that is interesting in its
    /// first second and one that is interesting after four minutes, and it is
    /// not a detail.
    ///
    /// A board of *n* sites needs on the order of *n* drops before it reaches
    /// criticality, and it is not a fraction of that: at 400x200 that is 80,000
    /// drops, which at [`SandpileOptions::drops_per_second`] is four and a half
    /// minutes of a playlist slot spent watching an empty screen fill in one site
    /// at a time. The first `frame_times` run of this effect read 101 bytes a
    /// frame and 1.1 microseconds of update, which looked like a triumph and was
    /// in fact a measurement of an empty board.
    ///
    /// 2.5 per site, which is where development *saturates*. Measured mean
    /// height on a 60x30 and a 200x100 board, both agreeing to two decimals:
    ///
    /// ```text
    ///  dose   mean height   occupied
    ///   0.5        0.50       0.40
    ///   1.0        1.00       0.65
    ///   1.5        1.49       0.81
    ///   2.0        1.96       0.90
    ///   2.5        2.10       0.92   <- plateau
    ///   3.0        2.09       0.92
    ///   6.0        2.09       0.93
    /// ```
    ///
    /// The mean tracks the dose exactly until about two drops per site, because
    /// grains pile up faster than they dissipate early on, and then stops dead.
    /// That flat region *is* criticality, and 2.5 is the first dose on it: a
    /// lower one leaves a board that is measurably emptier, and the first
    /// `frame_times` run of this effect was shipped at 1.5 before the sweep above
    /// was done, which put it on the slope at a mean height of 1.49 rather than
    /// on the plateau at 2.10.
    ///
    /// The cost is one-off and it is affordable: measured at about 50
    /// milliseconds on a 400x200 terminal, spent once at startup, which is why
    /// the effect then costs 1.1 microseconds of update a frame and almost
    /// nothing to encode.
    ///
    /// Zero disables it, which is only useful for measuring the difference.
    pub warmup_drops_per_site: f32,

    pub seed: u64,
}

impl Default for SandpileOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The derived `Default` produces zeros, and serde uses the derived one, so
    /// a config file that omitted a section would silently zero it -- a
    /// `drops_per_second` of zero is an effect that never starts.
    fn default() -> Self {
        Self {
            drops_per_second: 300.0,
            max_drops_per_frame: 32,
            max_topples_per_frame: 30_000,
            flash_seconds: 1.0,
            warmup_drops_per_site: 2.5,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Sandpile {
    screen_size: (u16, u16),
    options: SandpileOptions,
    canvas: Canvas,
    width: usize,
    height: usize,
    /// Grain count per site. Zero to [`MAX_HEIGHT`] when stable.
    heights: Vec<u8>,
    /// Remaining flash per site, zero to [`HEAT_STEPS`].
    heat: Vec<u8>,
    /// Sites that have reached [`TOPPLE_AT`] and still need to topple.
    ///
    /// Packed `x | y << 16` rather than a linear index, because a linear index
    /// needs a division and a modulo to unpack and this is the innermost loop in
    /// the effect. Coordinates are `u16` because a terminal is `u16` and the
    /// effect is never given anything wider.
    ///
    /// **Persistent across frames.** When the budget runs out this keeps its
    /// contents and the next frame resumes it, which is the entire design: see
    /// the module docs.
    stack: Vec<u32>,
    /// Sites still waiting to be lit for the current flash, drained per frame.
    lit: Vec<u32>,
    /// Elapsed time, accumulated, toward the next decay step.
    decay_accumulator: f32,
    drop_accumulator: f32,
    rng: EffectRng,
}

impl Sandpile {
    /// Linear index of a visible site.
    ///
    /// The frame path inlines this arithmetic, so this exists for the tests --
    /// and it is worth having the one named form, because a test that spells
    /// out `y * width + x` independently is a second place the layout is
    /// written down and can disagree with the first.
    #[cfg(test)]
    #[inline]
    fn index(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    /// Adds one grain, or destroys it if it would leave the board.
    ///
    /// The boundary is open, and this is where that happens: a topple on an edge
    /// site pushes a grain off the board and it is gone. That dissipation is
    /// what makes the model reach a steady state at all. In a closed system
    /// every drop is conserved and the board would simply fill up.
    fn add_grain(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = y as usize * self.width + x as usize;
        let height = self.heights[index] + 1;
        // Bounded because the stack is drained promptly; asserted because an
        // unbounded `+= 1` on a `u8` is a wraparound bug that only appears in a
        // long run, which is exactly what a screensaver is.
        debug_assert!(
            height <= TRANSIENT_HEIGHT_CEILING,
            "a site reached {height} grains, so the topple invariant is broken"
        );
        self.heights[index] = height;
        if height >= TOPPLE_AT {
            self.push(x as u16, y as u16);
        }
    }

    #[inline]
    fn push(&mut self, x: u16, y: u16) {
        self.stack.push(x as u32 | (y as u32) << 16);
    }

    /// Drains the stack against the per-frame budget.
    ///
    /// Returns the number of topplings performed, which may be less than the
    /// work the avalanche needed: see the module docs. The caller is expected to
    /// call this again next frame, and does not need to know whether it did all
    /// the work.
    fn stabilise(&mut self) -> u32 {
        let budget = self.options.max_topples_per_frame;
        let mut toppled = 0;

        while toppled < budget {
            let Some(site) = self.stack.pop() else {
                break;
            };
            let x = (site & 0xffff) as i32;
            let y = (site >> 16) as i32;

            let index = y as usize * self.width + x as usize;
            // Defensive, and not paranoia: a site can be pushed twice by two
            // different neighbours and be popped the second time already at zero.
            if self.heights[index] < TOPPLE_AT {
                continue;
            }

            self.heights[index] -= TOPPLE_AT;
            toppled += 1;

            self.light(x, y);

            // The four orthogonal neighbours, in a fixed order so a run is
            // reproducible.
            self.add_grain(x - 1, y);
            self.add_grain(x + 1, y);
            self.add_grain(x, y - 1);
            self.add_grain(x, y + 1);
        }

        toppled
    }

    /// Marks a site as part of the current avalanche.
    ///
    /// Queued rather than written directly, because a single topple can be
    /// reached again by a later branch of the same avalanche and the heat is
    /// meant to be set to full each time -- so the queue is drained once at the
    /// end of the frame and every site in it gets exactly one write.
    fn light(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        self.lit.push(x as u32 | (y as u32) << 16);
    }

    fn drop_grain(&mut self) {
        let x = self.rng.random_range(0..self.width) as i32;
        let y = self.rng.random_range(0..self.height) as i32;
        self.add_grain(x, y);
    }

    /// Advances the flash by `delta`, in [`HEAT_STEPS`] discrete steps.
    ///
    /// Time is accumulated and converted to whole steps, so a cell's fade takes
    /// the same wall-clock time at any frame rate and costs the same number of
    /// writes at any frame rate. Decrementing in one pass rather than per-cell is
    /// what makes the whole flash cost `HEAT_STEPS` passes per second.
    fn decay(&mut self, delta: f32) {
        let step_seconds = self.options.flash_seconds / HEAT_STEPS as f32;
        if step_seconds <= 0.0 {
            for heat in &mut self.heat {
                *heat = 0;
            }
            return;
        }

        self.decay_accumulator += delta;
        while self.decay_accumulator >= step_seconds {
            self.decay_accumulator -= step_seconds;
            let mut any = false;
            for heat in &mut self.heat {
                if *heat > 0 {
                    *heat -= 1;
                    any = true;
                }
            }
            if !any {
                self.decay_accumulator = 0.0;
                break;
            }
        }
    }

    fn advance(&mut self, delta: f32) {
        self.decay(delta);

        self.drop_accumulator += delta * self.options.drops_per_second;
        let mut drops = self.drop_accumulator.floor().max(0.0) as u32;
        if drops > 0 {
            self.drop_accumulator -= drops as f32;
        }
        if drops > self.options.max_drops_per_frame {
            // The surplus is discarded rather than banked. Banking it would let a
            // stalled frame buy an unbounded catch-up, which is the same stall
            // again a moment later.
            drops = self.options.max_drops_per_frame;
        }
        for _ in 0..drops {
            self.drop_grain();
        }

        self.stabilise();

        for site in self.lit.drain(..) {
            let x = (site & 0xffff) as usize;
            let y = (site >> 16) as usize;
            self.heat[y * self.width + x] = HEAT_STEPS;
        }
    }

    fn draw(&mut self) {
        self.canvas.clear();
        for y in 0..self.height {
            for x in 0..self.width {
                let index = y * self.width + x;
                let heat = self.heat[index];
                let cell = if heat > 0 {
                    let level = (heat as usize).min(HEAT_STEPS as usize);
                    let (r, g, b) = HEAT_COLORS[level];
                    Cell::new(
                        HEAT_GLYPHS[level],
                        style::Color::Rgb { r, g, b },
                        style::Attribute::Reset,
                    )
                } else {
                    // `MAX_HEIGHT` rather than the ramp's length, because that is
                    // the number that says what a settled board contains. A site
                    // at or above the topple threshold is mid-avalanche and the
                    // ramp has no entry for it, so this clamp is what keeps a
                    // transient height from indexing out of bounds on a frame
                    // that caught an avalanche part-stabilised.
                    let level =
                        (self.heights[index] as usize).min(MAX_HEIGHT as usize);
                    if level == 0 {
                        EMPTY_CELL
                    } else {
                        let (r, g, b) = HEIGHT_COLORS[level];
                        Cell::new(
                            HEIGHT_GLYPHS[level],
                            style::Color::Rgb { r, g, b },
                            style::Attribute::Reset,
                        )
                    }
                };
                self.canvas.set(x, y, cell);
            }
        }
    }
}

impl TerminalEffect for Sandpile {
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
        let (width, height) =
            (self.screen_size.0 as usize, self.screen_size.1 as usize);
        self.width = width;
        self.height = height;
        self.heights = vec![0; width * height];
        self.heat = vec![0; width * height];
        self.stack.clear();
        self.lit.clear();
        self.decay_accumulator = 0.0;
        self.drop_accumulator = 0.0;
        self.rng = seeded_rng(self.options.seed, "sandpile");
        self.warm_up();
    }
}

impl Sandpile {
    /// Builds the effect, already warmed up.
    ///
    /// The constructor finishes with [`Sandpile::reset`] rather than assembling
    /// the same state a second time, and that is the point: a board is only ever
    /// built in one place, so the warmup cannot be forgotten here and present
    /// there. It was, and the symptom was that a freshly constructed effect held
    /// an empty board until the runtime happened to call `reset` -- which it does,
    /// so nothing was ever visibly wrong and `the_board_is_already_critical_before_
    /// the_first_frame` failed on a board the program would never have shown.
    pub fn new(options: SandpileOptions, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let (width, height) = (screen_size.0 as usize, screen_size.1 as usize);

        let mut sandpile = Self {
            screen_size,
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            width,
            height,
            heights: vec![0; width * height],
            heat: vec![0; width * height],
            stack: Vec::new(),
            lit: Vec::new(),
            decay_accumulator: 0.0,
            drop_accumulator: 0.0,
            rng: seeded_rng(DEFAULT_SEED, "sandpile"),
        };
        sandpile.reset();
        sandpile
    }

    /// Drops `warmup_drops_per_site` grains per site, fully stabilising after
    /// each, to bring the board to criticality before the first frame.
    ///
    /// This runs on **every** reset, and reset is what a terminal resize calls,
    /// so a resize re-rolls the board from the seed rather than preserving it.
    /// That is the crate's existing trade for seeded effects: the alternative
    /// would be to push "was this seed set?" into the constructor, and an effect
    /// rebuilt on a resize would then draw a *new* picture under the user. A
    /// resize costing about 60 milliseconds at 400x200 is the cheaper half of
    /// that.
    ///
    /// Fully stabilising between drops rather than letting them overlap is
    /// deliberate: overlapping is faster to reach criticality, but this runs once
    /// and the difference is a fraction of the startup budget. In order, and from
    /// the seeded generator, because a warmup that is not reproducible would make
    /// two `--seed N` runs differ on their very first frame.
    ///
    /// No budget here. A truncated warmup would leave the board part-stabilised
    /// at the first frame, which is a state the effect would have to recover from
    /// and which no test could describe.
    fn warm_up(&mut self) {
        let drops = (self.width as f32
            * self.height as f32
            * self.options.warmup_drops_per_site)
            .round() as usize;
        for _ in 0..drops {
            self.drop_grain();
            while !self.stack.is_empty() {
                self.stabilise();
            }
        }
        // The warmup ran under the frame's topple budget and queued lights that
        // were never drained. Both are cleared here rather than left for the next
        // `advance`, which would light a scatter of sites the user never saw
        // topple, and `heat` is zeroed because a board with a stale flash on it is
        // not the state the tests reason about.
        self.stack.clear();
        self.lit.clear();
        self.heat.fill(0);
        self.decay_accumulator = 0.0;
    }

    /// Total grains on the board. Test-only, and the only way to state the
    /// conservation law the topple rule obeys.
    #[cfg(test)]
    fn total_grains(&self) -> u64 {
        self.heights.iter().map(|h| *h as u64).sum()
    }

    /// A board with `grain` grains at one site, for tests that need a specific
    /// starting state.
    #[cfg(test)]
    fn with_grain_at(x: usize, y: usize, grain: u8) -> Self {
        // No warmup. `new` builds a *critical* board, which is what the effect
        // wants and is the opposite of what a topple-rule test needs: setting
        // one cell to four on a full board and toppling it moves four grains into
        // neighbours that already hold some, and the test would be asserting
        // arithmetic on top of a pile it did not set up.
        let options = SandpileOptions {
            warmup_drops_per_site: 0.0,
            ..SandpileOptions::default()
        };
        let mut sandpile = Self::new(options, (20, 10));
        let index = sandpile.index(x, y);
        sandpile.heights[index] = grain;
        sandpile
    }

    /// A board driven to criticality by dropping `drops` grains **one at a time,
    /// each fully stabilised before the next**.
    ///
    /// This is the textbook measurement, and it is deliberately *not* how the
    /// effect runs. The effect drops several grains per frame and lets their
    /// avalanches overlap, because at five drops a frame that is the only way a
    /// board this size develops in a watchable time. The consequence is that in
    /// the effect's own path there is no such thing as "one avalanche" -- with
    /// drops landing every frame the stack is essentially never empty, so an
    /// avalanche counter incremented on stack-empty moments records a handful of
    /// quiescent pauses rather than a distribution of anything.
    ///
    /// So the distribution is measured here, where a drop and its avalanche are
    /// unambiguously one event, and the effect's overlapping path is tested
    /// separately for the things it can be tested for.
    #[cfg(test)]
    fn critical_board(
        width: usize,
        height: usize,
        drops: usize,
        seed: u64,
    ) -> Self {
        let options = SandpileOptions {
            seed,
            max_topples_per_frame: u32::MAX,
            // `new` warms up by default and this helper drops its own grains, so
            // the default would develop the board twice and the avalanche sizes
            // measured after it would be from a different history than the ones
            // a reader expects.
            warmup_drops_per_site: 0.0,
            ..SandpileOptions::default()
        };
        let mut sandpile = Self::new(
            options,
            (
                width.min(u16::MAX as usize) as u16,
                height.min(u16::MAX as usize) as u16,
            ),
        );
        for _ in 0..drops {
            sandpile.drop_grain();
            while !sandpile.stack.is_empty() {
                sandpile.stabilise();
            }
        }
        sandpile
    }

    /// Every avalanche size from `drops` single-grain drops onto a critical
    /// board, as the model defines them.
    ///
    /// The seed is shared across the board and the run so the two are the same
    /// board, which is what lets a caller measure a specific avalanche twice.
    #[cfg(test)]
    fn measure_avalanches(
        width: usize,
        height: usize,
        warmup: usize,
        drops: usize,
        seed: u64,
    ) -> (Self, Vec<u32>) {
        let mut sandpile = Self::critical_board(width, height, warmup, seed);
        let mut sizes = Vec::with_capacity(drops);
        for _ in 0..drops {
            sandpile.drop_grain();
            let mut toppled = 0;
            while !sandpile.stack.is_empty() {
                toppled += sandpile.stabilise();
            }
            sizes.push(toppled);
        }
        (sandpile, sizes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single topple: four grains become one in each orthogonal neighbour and
    /// nothing in either diagonal.
    ///
    /// Asserted site by site rather than by total, because a conservation check
    /// alone is satisfied by a rule that puts the grain in the wrong place -- it
    /// would still sum to four.
    #[test]
    fn a_topple_moves_four_grains_to_the_four_orthogonal_neighbours() {
        let mut sandpile = Sandpile::with_grain_at(10, 5, TOPPLE_AT);
        sandpile.push(10, 5);
        sandpile.stabilise();

        assert_eq!(sandpile.heights[sandpile.index(10, 5)], 0, "site emptied");
        for (x, y) in [(9, 5), (11, 5), (10, 4), (10, 6)] {
            assert_eq!(
                sandpile.heights[sandpile.index(x, y)],
                1,
                "orthogonal neighbour at ({x}, {y}) should hold one"
            );
        }
        for (x, y) in [(9, 4), (11, 4), (9, 6), (11, 6)] {
            assert_eq!(
                sandpile.heights[sandpile.index(x, y)],
                0,
                "diagonal at ({x}, {y}) should be untouched"
            );
        }
    }

    /// A topple away from the boundary conserves grain exactly.
    ///
    /// This is the law the model rests on, and it is also what makes the
    /// boundary check in `add_grain` necessary rather than defensive: the only
    /// place grain is allowed to disappear is off the edge.
    #[test]
    fn an_interior_topple_conserves_grain() {
        let mut sandpile = Sandpile::with_grain_at(10, 5, TOPPLE_AT);
        let before = sandpile.total_grains();
        sandpile.push(10, 5);
        sandpile.stabilise();
        assert_eq!(sandpile.total_grains(), before);
    }

    /// A topple on the boundary destroys the grain that would have gone off the
    /// board, and exactly the grains that have nowhere to go.
    ///
    /// The open boundary is what stops the board filling up, so a rule that
    /// quietly kept the grain past the edge would be a board that grows without
    /// bound -- and would look fine right up until it did. The counts are the
    /// number of *missing* neighbours, which is why the corner loses two, an
    /// edge loses one and the interior loses none.
    #[test]
    fn a_topple_loses_exactly_the_grains_that_have_nowhere_to_go() {
        // Interior: all four neighbours in range, nothing lost.
        let mut interior = Sandpile::with_grain_at(10, 5, TOPPLE_AT);
        let before = interior.total_grains();
        interior.push(10, 5);
        interior.stabilise();
        assert_eq!(
            before - interior.total_grains(),
            0,
            "interior loses nothing"
        );

        // Top edge: one neighbour is off the board, so one grain is destroyed.
        let mut edge = Sandpile::with_grain_at(10, 0, TOPPLE_AT);
        let before = edge.total_grains();
        edge.push(10, 0);
        edge.stabilise();
        assert_eq!(before - edge.total_grains(), 1, "an edge site loses one");
        assert_eq!(edge.heights[edge.index(9, 0)], 1);
        assert_eq!(edge.heights[edge.index(11, 0)], 1);
        assert_eq!(edge.heights[edge.index(10, 1)], 1);

        // Corner: two neighbours are off the board.
        let mut corner = Sandpile::with_grain_at(0, 0, TOPPLE_AT);
        let before = corner.total_grains();
        corner.push(0, 0);
        corner.stabilise();
        assert_eq!(before - corner.total_grains(), 2, "a corner loses two");
        assert_eq!(corner.heights[corner.index(1, 0)], 1);
        assert_eq!(corner.heights[corner.index(0, 1)], 1);
    }

    /// The whole point of the effect: avalanche size is heavy-tailed, so the
    /// mean is dragged far above the median and a few drops do an enormous
    /// amount of work.
    ///
    /// This is the assertion that makes it a screensaver rather than a
    /// simulation, and it is deliberately a property of the *distribution* and
    /// not of any single avalanche. A test asserting "some avalanche was large"
    /// would pass against a board with a bug that made every drop small except
    /// one; a test asserting a ratio cannot.
    ///
    /// The ratios below are measured, not chosen -- see the comment on
    /// `the_avalanche_distribution_is_heavy_tailed` for the numbers and
    /// The board reaches a critical state and stays there.
    ///
    /// This is the property the effect rests on and it is worth asserting
    /// directly, because "the board is full of sand" would satisfy every visual
    /// check. Criticality shows up as a *specific* mean height that the system
    /// finds for itself and then holds, and as almost every site being occupied:
    /// measured on a 60x30 board, the mean height lands at 2.08 and stops moving
    /// (2.077 after 4,000 drops, 2.093 after 20,000, 2.095 after 60,000) with
    /// 92% of sites non-empty.
    ///
    /// The band is narrow because the number is a measurement, not a taste. A
    /// rule that over-topples drives the mean down toward 1.0 and one that
    /// under-topples drives it up toward 3, and the effect would still be a
    /// shimmering mass of grains in both cases.
    #[test]
    fn the_board_reaches_criticality_and_holds_it() {
        let (short, _) = Sandpile::measure_avalanches(60, 30, 4_000, 1, 7);
        let (long, _) = Sandpile::measure_avalanches(60, 30, 60_000, 1, 7);

        let mean_height = |board: &Sandpile| {
            let sites = board.width * board.height;
            board.heights.iter().map(|h| *h as f64).sum::<f64>() / sites as f64
        };
        let occupied = |board: &Sandpile| {
            let sites = board.width * board.height;
            board.heights.iter().filter(|h| **h > 0).count() as f64 / sites as f64
        };

        let short_mean = mean_height(&short);
        let long_mean = mean_height(&long);

        assert!(
            (1.9..2.3).contains(&long_mean),
            "mean height settled at {long_mean:.3}, outside the 1.9-2.3 band a \
             critical sandpile occupies"
        );
        assert!(
            (long_mean - short_mean).abs() < 0.05,
            "mean height moved from {short_mean:.3} to {long_mean:.3} between 4k \
             and 60k drops, so the board is still developing rather than critical"
        );
        assert!(
            occupied(&long) > 0.85,
            "only {:.0}% of sites hold any grain, so the board has not filled in",
            occupied(&long) * 100.0
        );

        // The invariant the topple rule depends on: a board with an empty stack
        // has no unstable site on it.
        assert!(
            long.heights.iter().all(|h| *h <= MAX_HEIGHT),
            "a site was left above {MAX_HEIGHT} grains"
        );
    }

    /// Avalanche size is heavy-tailed, which is the only reason this is worth
    /// watching.
    ///
    /// A test asserting "some avalanche was large" would pass against a board
    /// whose every drop was small except one, so this asserts ratios in the
    /// distribution rather than any single sample.
    ///
    /// Two things make the obvious formulation wrong, and both were found by
    /// measuring rather than by reading. First, **the median avalanche is zero**:
    /// 57% of drops land somewhere that does nothing at all, because the drop
    /// site and all four of its neighbours are below threshold. So a ratio
    /// against the median is `something > 0`, which passes for any board
    /// whatsoever. The distribution is therefore taken over the *non-zero*
    /// avalanches, which is where the tail is. Second, the largest avalanche is
    /// a single sample and the noisiest number in the set, so it is measured
    /// and reported but not asserted on.
    ///
    /// Measured on a critical 60x30 board over 8,600 non-trivial avalanches:
    /// median 21, mean 132 (6.3x), 90th percentile 406, 99th 1,345 (64x the
    /// median). The thresholds below are a third of the measured skew and a
    /// third of the measured tail.
    #[test]
    fn the_avalanche_distribution_is_heavy_tailed() {
        let (_, sizes) = Sandpile::measure_avalanches(60, 30, 4_000, 20_000, 12345);

        let zeros = sizes.iter().filter(|s| **s == 0).count();
        let zero_fraction = zeros as f64 / sizes.len() as f64;
        // A band rather than a bound, and it is measured: a topple rule doing
        // too much drives this toward zero, one doing too little toward one.
        assert!(
            (0.3..0.8).contains(&zero_fraction),
            "{zero_fraction:.2} of drops toppled nothing, outside the measured \
             0.3-0.8 band"
        );

        let mut sorted: Vec<u32> =
            sizes.iter().copied().filter(|s| *s > 0).collect();
        sorted.sort_unstable();
        let n = sorted.len();
        assert!(n > 5_000, "expected thousands of avalanches, got {n}");

        let median = sorted[n / 2] as f64;
        let mean = sorted.iter().map(|s| *s as f64).sum::<f64>() / n as f64;
        let p99 = sorted[n * 99 / 100] as f64;

        assert!(
            mean > median * 3.0,
            "a power law drags the mean far above the median: mean {mean:.1} vs \
             median {median:.1} over {n} avalanches"
        );
        assert!(
            p99 > median * 20.0,
            "the 99th percentile should dwarf the median: p99 {p99:.1} vs \
             median {median:.1}"
        );
    }

    /// The path the effect actually runs: many overlapping drops per frame.
    ///
    /// The two tests above drive the model one drop at a time, because that is
    /// the only way to attribute an avalanche size to a drop. The effect does
    /// not do that -- it drops several grains a frame and lets their avalanches
    /// overlap, which is the only way a board this size develops in a watchable
    /// time. This test is for that path, and the property worth having is the
    /// invariant: however many drops land at once and however far the topple
    /// budget truncates them, the board must end every frame with no unstable
    /// site on it.
    ///
    /// That is not a formality. A board with a stuck unstable site would still
    /// shimmer, would still produce avalanches, and would look perfectly alive
    /// while being wrong -- and it would leak, because a site that never
    /// topples keeps its height forever and the grain count climbs off the top.
    #[test]
    fn overlapping_drops_leave_the_board_stabilised_every_frame() {
        let mut sandpile = Sandpile::new(SandpileOptions::default(), (60, 30));

        for frame in 0..1_200 {
            sandpile.advance(1.0 / 60.0);

            // The stack is allowed to be non-empty -- that is the budget
            // working. The heights are not: a site may be *transiently* above
            // threshold only if it is queued to topple, and every queued site
            // must be at or above threshold.
            let mut unstable = vec![false; sandpile.width * sandpile.height];
            for site in &sandpile.stack {
                let x = (*site & 0xffff) as usize;
                let y = (*site >> 16) as usize;
                unstable[sandpile.index(x, y)] = true;
            }
            for (index, height) in sandpile.heights.iter().enumerate() {
                if *height > MAX_HEIGHT {
                    assert!(
                        unstable[index],
                        "frame {frame}: site at index {index} holds {height} grains \
                         but is not queued to topple, so it is stuck"
                    );
                }
            }
        }

        // Twenty seconds is far enough for the board to have developed, and the
        // mean height says so. The band is the one measured on the serial path
        // in `the_board_reaches_criticality_and_holds_it`; the point here is that
        // overlapping drops reach the same state the model reaches on its own.
        let sites = sandpile.width * sandpile.height;
        let mean_height =
            sandpile.heights.iter().map(|h| *h as f64).sum::<f64>() / sites as f64;
        assert!(
            (1.7..2.4).contains(&mean_height),
            "mean height settled at {mean_height:.3} after overlapping drops, \
             outside the 1.7-2.4 band"
        );
    }

    /// Splitting an avalanche across frames must not change the avalanche.
    ///
    /// This is the load-bearing claim behind the whole cost design, and it is
    /// stated as an equivalence rather than as a bound: the *same* board, the
    /// *same* drop, stabilised one topple at a time, must perform the same
    /// number of topplings and end in the same state as the same drop
    /// stabilised in a single pass.
    ///
    /// It also asserts that the split actually happened, which is the part that
    /// would otherwise make it vacuous. An earlier version of this test counted
    /// size-one entries in a log and inferred partial recording from them --
    /// except that on a board this size a genuine single-topple avalanche is
    /// ordinary, so the test could not have distinguished the bug it was
    /// written for. Comparing two runs of one known avalanche has no such
    /// ambiguity.
    #[test]
    fn splitting_an_avalanche_across_frames_does_not_change_it() {
        let mut whole = Sandpile::critical_board(40, 20, 3_000, 7);
        let mut split = Sandpile::critical_board(40, 20, 3_000, 7);
        assert_eq!(
            whole.heights, split.heights,
            "the two boards should start identical"
        );

        whole.drop_grain();
        split.drop_grain();

        whole.options.max_topples_per_frame = u32::MAX;
        let whole_topples = whole.stabilise();
        assert!(
            whole.stack.is_empty(),
            "an unlimited budget should finish it"
        );

        let mut split_topples = 0;
        let mut split_calls = 0;
        split.options.max_topples_per_frame = 1;
        while !split.stack.is_empty() {
            split_topples += split.stabilise();
            split_calls += 1;
        }

        assert!(
            whole_topples > 1,
            "this drop should have caused a real avalanche to split, got \
             {whole_topples} topples"
        );
        assert!(
            split_calls > 1,
            "a budget of one should have needed several frames, got {split_calls}"
        );
        assert_eq!(
            whole_topples, split_topples,
            "the same avalanche cost different amounts of work at different rates"
        );
        assert_eq!(
            whole.heights, split.heights,
            "splitting the avalanche changed where it ended up"
        );
    }

    /// The flash is quantised, so a lit cell is rewritten a bounded number of
    /// times however long it lives.
    ///
    /// This is the bandwidth property rather than a visual one. A continuously
    /// fading highlight is a colour change in every lit cell on every frame for
    /// the whole second, and the output path emits a colour whenever it differs
    /// from the last one written -- so the quantisation is worth four writes per
    /// cell instead of sixty.
    #[test]
    fn a_lit_cell_steps_down_through_the_heat_levels_and_stops() {
        let options = SandpileOptions {
            drops_per_second: 0.0,
            ..SandpileOptions::default()
        };
        let mut sandpile = Sandpile::new(options, (20, 10));

        // Drive one topple directly, without a drop.
        let site = sandpile.index(10, 5);
        sandpile.heights[site] = TOPPLE_AT;
        sandpile.push(10, 5);
        sandpile.stabilise();
        // `mem::take` rather than `drain`, because `drain` holds a mutable
        // borrow of the field for the whole loop and the loop body needs the
        // same struct.
        for lit in std::mem::take(&mut sandpile.lit) {
            let x = (lit & 0xffff) as usize;
            let y = (lit >> 16) as usize;
            let index = sandpile.index(x, y);
            sandpile.heat[index] = HEAT_STEPS;
        }

        let mut seen = Vec::new();
        for _ in 0..600 {
            sandpile.decay(1.0 / 60.0);
            seen.push(sandpile.heat[site]);
        }

        // Every distinct value is one of the declared levels and no others. A
        // smooth fade would show values in between, and since the renderer maps
        // heat straight to a colour, an undeclared value would be an undeclared
        // colour and a fifth write per cell.
        let distinct: std::collections::BTreeSet<u8> =
            seen.iter().copied().collect();
        assert_eq!(
            distinct,
            [0, 1, 2, 3, 4].into_iter().collect(),
            "heat should only ever take a declared level"
        );

        // It reaches zero and *stays* there. A flash that never quite went off
        // would keep a cell in the diff forever, and a settled board emitting
        // nothing at all is the property that makes this effect cheap.
        let first_zero = seen.iter().position(|h| *h == 0).unwrap();
        assert!(
            seen[first_zero..].iter().all(|h| *h == 0),
            "heat went back up after reaching zero"
        );
        assert_eq!(sandpile.heat[site], 0);

        // And it is the last declared level, not one past it.
        assert_eq!(*seen.last().unwrap(), 0);
        assert_eq!(seen[0], HEAT_STEPS);
    }

    /// The board is already critical on the **first** frame.
    ///
    /// This is the assertion that keeps the warmup honest, and it is the one
    /// that would have caught the problem it exists to fix. The effect's first
    /// `frame_times` reading was 101 bytes a frame and 1.1 microseconds of
    /// update, which is a triumph on paper and was in fact an almost-empty board:
    /// 120 frames at 300 drops a second is 600 drops, and an 80,000-site board
    /// needs about 80,000 of them before anything structured exists.
    ///
    /// So this asks the question a user asks, which is whether the thing is
    /// already doing something when it appears. It is measured *before* a single
    /// `advance`, because a test that ran the effect for a few seconds first
    /// would pass with or without the warmup.
    /// The shipped warmup dose has reached the point where more of it stops
    /// helping.
    ///
    /// This is the assertion that makes the default a measurement rather than a
    /// number, and it is deliberately **relative**. An absolute band on mean
    /// height is a constant someone has to keep true by editing it; comparing
    /// the shipped dose against a much larger one asks the question that
    /// actually matters -- has the board stopped developing -- and answers it
    /// without a threshold that encodes a taste.
    ///
    /// The two states that differ are visible in the picture. A board at mean
    /// 1.5 is a sparse scatter on a dark field; a board at 2.1 is a dense mat
    /// with the branching avalanche scars legible across it. Shipping the first
    /// is the same mistake as the `terrain` grain rounds, except quieter.
    ///
    /// The absolute check in `the_board_is_already_critical_before_the_first_
    /// frame` is the backstop; this one is the one that would have caught the
    /// dose being lowered, because a lowered dose stops matching the plateau.
    #[test]
    fn the_shipped_warmup_dose_has_reached_the_plateau() {
        let mean_height = |dose: f32| {
            let options = SandpileOptions {
                warmup_drops_per_site: dose,
                ..SandpileOptions::default()
            };
            let sandpile = Sandpile::new(options, (120, 60));
            let sites = sandpile.width * sandpile.height;
            sandpile.heights.iter().map(|h| *h as f64).sum::<f64>() / sites as f64
        };

        let shipped = SandpileOptions::default().warmup_drops_per_site;
        let at_shipped = mean_height(shipped);
        // 2.4x the shipped dose, which the sweep puts well inside the plateau.
        let at_plateau = mean_height(shipped * 2.4);
        // And half of it, which the sweep puts on the slope.
        let at_half = mean_height(shipped * 0.5);

        assert!(
            (at_shipped - at_plateau).abs() < 0.05,
            "the shipped dose of {shipped} reaches mean height {at_shipped:.3} but \
             2.4x that reaches {at_plateau:.3}, so the board is still developing"
        );
        assert!(
            at_shipped - at_half > 0.4,
            "halving the dose only moved the mean height from {at_shipped:.3} to \
             {at_half:.3}, so the shipped dose is not on the plateau"
        );
    }

    #[test]
    fn the_board_is_already_critical_before_the_first_frame() {
        // Two sizes, because the occupancy fraction is not size-independent: the
        // fraction of sites within one hop of the open boundary shrinks as the
        // board grows, and boundary sites are emptier than interior ones. The
        // *mean height* is the size-independent measure and is the one the
        // narrow band is on; occupancy only has to rule out a nearly-empty
        // board, so its bound is loose and checked at both sizes.
        for (w, h) in [(60usize, 30usize), (200, 100)] {
            let sandpile =
                Sandpile::new(SandpileOptions::default(), (w as u16, h as u16));

            let sites = sandpile.width * sandpile.height;
            let occupied = sandpile.heights.iter().filter(|h| **h > 0).count();
            let fraction = occupied as f64 / sites as f64;
            let mean_height =
                sandpile.heights.iter().map(|h| *h as f64).sum::<f64>()
                    / sites as f64;
            println!("CRIT {w}x{h} occupied={fraction:.3} mean={mean_height:.3}");

            assert!(
                fraction > 0.85,
                "only {:.0}% of sites hold a grain on a fresh {w}x{h} board",
                fraction * 100.0
            );
            assert!(
                (1.9..2.3).contains(&mean_height),
                "mean height is {mean_height:.3} on a fresh {w}x{h} board, so the \
                 warmup did not bring it to criticality"
            );

            // And nothing is flashing: the warmup clears its own lights, so the
            // first frame shows a still critical board rather than one with a
            // scatter of highlights the user never saw topple.
            assert!(
                sandpile.heat.iter().all(|h| *h == 0),
                "the warmup left flash on the board at {w}x{h}"
            );
            assert!(sandpile.stack.is_empty(), "the warmup left work at {w}x{h}");
        }
    }

    /// Turning the warmup off gives a board that is visibly emptier, which is
    /// what makes the option's default a measured choice rather than a guess.
    #[test]
    fn without_the_warmup_the_board_starts_empty() {
        let options = SandpileOptions {
            warmup_drops_per_site: 0.0,
            ..SandpileOptions::default()
        };
        let sandpile = Sandpile::new(options, (200, 100));
        let sites = sandpile.width * sandpile.height;
        let occupied = sandpile.heights.iter().filter(|h| **h > 0).count();

        assert_eq!(
            occupied, 0,
            "a board with no warmup should be empty, and this is the contrast \
             that makes the default's value visible"
        );
        assert!(sites > 0);
    }

    /// A resize re-rolls the board, and it re-rolls it to the same place.
    ///
    /// `reset` is what a resize calls, so if the warmup were not reached from
    /// `reset` a resize would leave an empty board on screen until the next
    /// avalanche happened to fill it in -- a picture that is fine for hours and
    /// blank for the moment after you drag the window. And because the warmup is
    /// seeded it has to land in the same place both times, or a resize would
    /// silently change the effect the user was looking at.
    #[test]
    fn a_resize_rebuilds_the_board_and_the_warmup_comes_with_it() {
        let mut sandpile = Sandpile::new(SandpileOptions::default(), (120, 60));
        let before: Vec<u8> = sandpile.heights.clone();

        sandpile.update_size(120, 60);
        assert_eq!(
            sandpile.heights, before,
            "a same-size reset changed the board"
        );

        sandpile.update_size(200, 100);
        let sites = sandpile.width * sandpile.height;
        let occupied = sandpile.heights.iter().filter(|h| **h > 0).count();
        assert_eq!(sandpile.heights.len(), sites);
        assert!(
            occupied as f64 / sites as f64 > 0.85,
            "only {:.0}% of sites hold a grain after a resize, so the warmup did \
             not run on the new size",
            occupied as f64 / sites as f64 * 100.0
        );
    }

    /// A terminal below the minimum still produces a real board.
    ///
    /// Not a request for one cell: `normalize_effect_size` clamps to
    /// `MIN_EFFECT_SIZE`, so a 1x1 terminal is a real 6x6 board rather than a
    /// one-cell one. Asserting the clamp rather than ignoring it, because a
    /// board of one site would be a division by zero in three places and this is
    /// the test that would say so.
    #[test]
    fn a_terminal_below_the_minimum_still_produces_a_real_board() {
        let mut sandpile = Sandpile::new(SandpileOptions::default(), (1, 1));
        sandpile.advance(1.0 / 60.0);
        sandpile.draw();

        let (width, height) = sandpile.screen_size;
        assert!(width > 1 && height > 1, "the size should have been floored");
        assert_eq!(sandpile.heights.len(), width as usize * height as usize);
        assert_eq!(sandpile.heat.len(), sandpile.heights.len());
    }
}
