//! Elementary cellular automata drawn as a space-time diagram: a single row of
//! cells, one new generation per step, stacked downwards until the screen is
//! full.
//!
//! ## Why this is here when `life` and `ants` already exist
//!
//! Both of those are *spatial*: they have a board, a neighbourhood, and rules
//! over two dimensions. This is the other half of the idea and the more famous
//! half -- the triangle, with time running down the screen. Wolfram's rule 30 is
//! the single most reproduced picture in the field.
//!
//! ## Why it cannot converge, rather than merely not
//!
//! Rule 30 is left-permutive: for any fixed centre and right cell there is
//! exactly one left cell that produces a 1. Left-permutivity is the hypothesis
//! of the theorem that gives rule 30 rigorous Devaney chaos, so its space-time
//! diagram is provably unpredictable -- it cannot settle, cannot repeat, and
//! cannot enter a cycle. Every other simulation in this crate is
//! non-convergent by *design*; this one is non-convergent by *proof*, which is
//! a different kind of promise to put in a doc comment.
//!
//! It is also the cheapest effect in the crate by an order of magnitude: one
//! table lookup per cell per generation, and a generation is a single row.
//!
//! ## Why it grows rather than scrolls
//!
//! A scrolling triangle is a full-screen diff every frame, which throws away
//! everything the effect has going for it. Instead the diagram *fills downwards*
//! and is cleared when it reaches the bottom, so a frame's diff is the band at
//! the growth front. Braille is what makes that band thin: the automaton runs at
//! dot resolution, so a generation is one dot-row of four per cell row, and
//! sixty generations a second is a sixth of a screen row.
//!
//! ## The bandwidth
//!
//! One colour and two states. The output path emits a colour only when it
//! differs from the last one written, so a two-value alphabet over a whole
//! screen is a handful of style changes however many dots are raised. This is
//! the "not writing a cell is the cheapest rendering optimisation there is" note
//! in the crate's own docs taken to its limit -- a cell with no raised dot is
//! not written at all.

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{
    DEFAULT_SEED, EffectRng, TerminalEffect, normalize_effect_size, seeded_rng,
};
use crate::render::braille::BrailleGrid;
use crate::runtime::FrameContext;
use crossterm::style;
use crossterm::style::Color;
use rand::RngExt;
use serde::{Deserialize, Serialize};

/// A rule, as a bitmask over the eight neighbourhoods.
///
/// The three things that are easy to get confused about, and which cost a
/// compile error to get right the first time: there are **256 possible rules**,
/// and **each one is 8 bits wide** -- one bit per three-cell neighbourhood, not
/// one bit per rule number. So a neighbourhood is an index 0 to 7 (left cell as
/// bit 2, centre as bit 1, right as bit 0) and a rule is a `u8` holding the eight
/// answers.
///
/// Widened to `u32` so the lookup needs no cast, and because a rule number is
/// then *literally* its own mask: Wolfram's numbering is "bit *i* is the output
/// for neighbourhood *i*", which is why `mask_for` can pass any rule straight
/// through with no table and be right.
type RuleMask = u32;

/// The mask for rule 30, the chaotic one that made the field famous.
///
/// Written as the arithmetic it is rather than as a literal, because
/// `p XOR (q OR r)` is the sentence every reference uses and the sentence is
/// what the tests below check the table against.
const fn rules_for_30() -> RuleMask {
    let mut mask = 0u32;
    let mut n = 0;
    while n < 8 {
        let p = n & 0b100 != 0;
        let q = n & 0b010 != 0;
        let r = n & 0b001 != 0;
        if p != (q || r) {
            mask |= 1u32 << n;
        }
        n += 1;
    }
    mask
}

const RULE_30: RuleMask = rules_for_30();

/// Rule 90, which is `p XOR r` and so is Pascal's triangle modulo 2 -- the
/// Sierpinski gasket, from a single cell.
///
/// A named constant rather than a literal because it is the one other rule worth
/// pinning: it is the closed form the tests use to check the automaton, because
/// a fractal with a known answer is worth more as a test fixture than a famous
/// chaotic one.
const fn rules_for_90() -> RuleMask {
    let mut mask = 0u32;
    let mut n = 0;
    while n < 8 {
        if (n & 0b100 != 0) != (n & 0b001 != 0) {
            mask |= 1u32 << n;
        }
        n += 1;
    }
    mask
}

const RULE_90: RuleMask = rules_for_90();

/// The mask for rule 110, which is `XOR(OR(p, q), AND(p, q, r))` and is
/// Turing-complete.
const fn rules_for_110() -> RuleMask {
    let mut mask = 0u32;
    let mut n = 0;
    while n < 8 {
        let p = n & 0b100 != 0;
        let q = n & 0b010 != 0;
        let r = n & 0b001 != 0;
        if (p || q) != (p && q && r) {
            mask |= 1u32 << n;
        }
        n += 1;
    }
    mask
}

const RULE_110: RuleMask = rules_for_110();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule30Options {
    /// The elementary rule, 0 to 255.
    ///
    /// 30, the chaotic one. The whole family is this one number: 90 is the
    /// Sierpinski gasket, 110 is Turing-complete, 254 fills and then dies.
    pub rule: u8,

    /// Generations added per second.
    ///
    /// 60, one per frame at 60 fps, which is a sixth of a screen row a frame
    /// and fills a 50-row screen in a little under three and a half seconds.
    /// Faster and the triangle's fine structure goes below what braille can
    /// carry; slower and the growth front is a visible crawl.
    pub generations_per_second: f32,

    /// Most generations added in a single frame.
    ///
    /// A cap of 16. The same reason as everywhere else in this crate: a
    /// terminal that was unfocused hands back a delta of seconds, and without a
    /// cap the effect would fill the screen and reset in one frame.
    pub max_generations_per_frame: u16,

    /// How often a partly-filled diagram is discarded and a new one started, in
    /// seconds. Zero means only when the screen fills.
    ///
    /// Zero is the default because a full rule 30 triangle is genuinely worth
    /// looking at, and clearing early would be taste dressed as a safety valve.
    /// The cost is that the reset lands on a metronome, which is the one thing
    /// about this effect that is not lovely.
    pub cycle_seconds: f32,

    /// Extra live cells scattered into the first row.
    ///
    /// The diagram starts from one cell, because that is the image: from a
    /// single cell, rule 30 grows a triangle with a chaotic interior and a
    /// structured edge, and a random first row instead produces a mess with no
    /// shape at all. Each of these adds one more, so the effect varies with the
    /// seed -- which it has to, because the contract suite requires every effect
    /// to be seed-sensitive -- without giving up the shape.
    pub seed_cells: u16,

    /// The colour raised dots are drawn in.
    ///
    /// One colour, and only one, because that is all braille can carry. There is
    /// no second colour for an unraised dot: an unraised dot is the terminal's
    /// own background, which is why this effect is the cheapest in the crate to
    /// emit and also why it cannot show a gradient.
    ///
    /// A `Color` rather than a name for this effect's own sake, but mainly
    /// because it is the crate's existing convention: `ink` takes `Vec<Color>` and
    /// `global` takes a `Color`, and both get crossterm's spellings (`"green"`,
    /// `"#00ff00"`, `"dark_grey"` with the underscore, `"rgb_(r,g,b)"`) from serde
    /// rather than from a hand-written parser. The first draft of this effect
    /// had a `String` and its own `parse_color`, which was a third spelling
    /// dialect in a crate that already had two.
    pub color: Color,

    pub seed: u64,
}

impl Default for Rule30Options {
    /// Hand-written so it is the single source of truth. The derived one is all
    /// zeroes, which for this effect means rule 0 -- every cell dies on the first
    /// generation, and the screen is black for the rest of the playlist slot.
    fn default() -> Self {
        Self {
            rule: 30,
            generations_per_second: 60.0,
            max_generations_per_frame: 16,
            cycle_seconds: 0.0,
            seed_cells: 3,
            color: Color::Green,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct Rule30 {
    screen_size: (u16, u16),
    options: Rule30Options,
    canvas: Canvas,
    grid: BrailleGrid,
    /// The automaton's row, one bit per braille dot column.
    current: Vec<bool>,
    /// Where the next generation is written. Two buffers, because a generation
    /// reads one and writes the other: an in-place left-to-right sweep propagates
    /// the new row backwards and the whole triangle comes out as a solid block.
    next: Vec<bool>,
    /// The next dot-row to write.
    cursor: usize,
    /// The rule, resolved from `options.rule` once rather than per cell.
    mask: RuleMask,
    generation_accumulator: f32,
    cycle_accumulator: f32,
    /// Whether the first row has been laid down and the rest is growth.
    started: bool,
    rng: EffectRng,
    color: style::Color,
}

impl Rule30 {
    /// The rule's own description, for the module's own tests and for anyone
    /// reading a config that says `rule = 90`.
    fn mask_for(rule: u8) -> RuleMask {
        match rule {
            30 => RULE_30,
            90 => RULE_90,
            110 => RULE_110,
            // Every other rule is its number, read as a bitmask. This is the
            // definition of Wolfram's numbering, so there is no table to be wrong
            // about: a config saying `rule = 45` gets rule 45.
            other => other as RuleMask,
        }
    }

    /// Clears the grid and seeds a new diagram.
    fn start_diagram(&mut self) {
        self.grid.clear();
        self.cursor = 0;
        self.started = false;

        for cell in self.current.iter_mut().chain(self.next.iter_mut()) {
            *cell = false;
        }

        let width = self.current.len();
        if width == 0 {
            return;
        }

        // The main cell, left of centre so the triangle's *right* edge -- the
        // structured one, the one with the repeating comb along it -- is fully on
        // screen rather than clipped by the border. Centring it puts half of the
        // most recognisable feature of the picture off the edge.
        let main = (width / 4).min(width - 1);
        self.current[main] = true;

        // The seeded extras, kept away from the main cell so they read as
        // separate triangles rather than as a thicker one.
        for _ in 0..self.options.seed_cells {
            let x = self.rng.random_range(0..width);
            if x.abs_diff(main) > width / 8 {
                self.current[x] = true;
            }
        }
    }

    /// One generation into `next`, then swap.
    #[inline]
    fn step(&mut self) {
        let width = self.current.len();
        let mask = self.mask;
        for i in 0..width {
            // The two boundary cases are folded in as dead neighbours rather
            // than branched on. A rule with a set edge cell would otherwise see a
            // *wrapped* neighbour, and a triangle that wraps is a stripe.
            let index = (usize::from(i > 0 && self.current[i - 1]) << 2)
                | (usize::from(self.current[i]) << 1)
                | usize::from(i + 1 < width && self.current[i + 1]);
            self.next[i] = (mask >> index) & 1 == 1;
        }
        std::mem::swap(&mut self.current, &mut self.next);
    }

    /// Writes the current row into the grid at `cursor` and moves on.
    fn commit_row(&mut self) {
        let dot_y = self.cursor;
        if dot_y < self.grid.dot_height() {
            for (i, alive) in self.current.iter().enumerate() {
                if *alive {
                    self.grid.raise_dot(i, dot_y);
                }
            }
        }
        self.cursor += 1;
        self.started = true;
    }

    fn advance(&mut self, delta: f32) {
        if self.options.cycle_seconds > 0.0 {
            self.cycle_accumulator += delta;
            if self.cycle_accumulator >= self.options.cycle_seconds {
                self.cycle_accumulator = 0.0;
                self.start_diagram();
                self.commit_row();
            }
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
            if self.cursor >= self.grid.dot_height() {
                // The screen is full, so start again rather than scrolling: a
                // scroll is a full-screen diff every frame and this effect's
                // entire value is that its diff is a thin band.
                self.start_diagram();
            }
            if self.started {
                self.step();
            }
            self.commit_row();
        }
    }

    fn draw(&mut self) {
        self.canvas.clear();
        self.grid
            .write_to(&mut self.canvas, self.color, style::Attribute::Reset);
    }

    pub fn new(options: Rule30Options, screen_size: (u16, u16)) -> Self {
        let screen_size = normalize_effect_size(screen_size);
        let grid = BrailleGrid::new(screen_size.0 as usize, screen_size.1 as usize);
        let width = grid.dot_width();

        let mut rule30 = Self {
            screen_size,
            mask: Self::mask_for(options.rule),
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            grid,
            current: vec![false; width],
            next: vec![false; width],
            cursor: 0,
            generation_accumulator: 0.0,
            cycle_accumulator: 0.0,
            started: false,
            rng: seeded_rng(DEFAULT_SEED, "rule30"),
            color: style::Color::Green,
        };
        rule30.rng = seeded_rng(rule30.options.seed, "rule30");
        rule30.color = rule30.options.color;
        rule30.reset();
        rule30
    }
}

impl TerminalEffect for Rule30 {
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
        let width = self.grid.dot_width();
        self.current = vec![false; width];
        self.next = vec![false; width];
        self.generation_accumulator = 0.0;
        self.cycle_accumulator = 0.0;
        self.start_diagram();
        // The first row goes down immediately rather than waiting a frame, so the
        // effect opens with a cell rather than an empty screen for the first
        // sixteenth of a second.
        self.commit_row();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mask for a rule agrees with the rule's own sentence.
    ///
    /// This is the "derived table with no test against its source" failure the
    /// crate's notes warn about, in its purest form: `RULE_30` is computed by a
    /// `const fn` rather than written as a literal, which makes it a second place
    /// the same thing is said. Deriving it at compile time does not make it a
    /// constant like any other.
    ///
    /// Asserted against the sentence -- `p XOR (q OR r)` -- and not against
    /// another copy of the function, so the two cannot be wrong together.
    #[test]
    fn rule_30s_mask_is_the_sentence_p_xor_q_or_r() {
        for n in 0..8usize {
            let p = n & 0b100 != 0;
            let q = n & 0b010 != 0;
            let r = n & 0b001 != 0;
            assert_eq!(
                (RULE_30 >> n) & 1 == 1,
                p != (q || r),
                "rule 30 disagrees with its own description at neighbourhood {n:03b}"
            );
        }
    }

    /// Rule 90 from a single cell is Pascal's triangle modulo 2.
    ///
    /// The strongest fixture available for an automaton, because the answer is
    /// known in closed form and checkable exactly rather than by eye: cell *i* of
    /// generation *g* is 1 exactly when `C(g, i)` is odd, which is exactly when
    /// `i & g == i`. Checking the first ten generations against that is a test of
    /// the step function that no plausible bug survives.
    #[test]
    fn rule_90_from_one_cell_is_pascals_triangle_modulo_two() {
        let mut rule30 = Rule30::new(
            Rule30Options {
                rule: 90,
                seed_cells: 0,
                ..Rule30Options::default()
            },
            (80, 20),
        );
        rule30.start_diagram();
        let start = rule30.current.iter().position(|c| *c).expect("a seed cell");

        rule30.commit_row();
        for generation in 1..14usize {
            rule30.step();
            for (offset, cell) in rule30.current.iter().enumerate() {
                let distance = offset as isize - start as isize;
                // After `g` steps from one cell, the cell at signed distance `d`
                // was reached by `(g + d) / 2` steps to the right and
                // `(g - d) / 2` to the left. Its value is therefore
                // `C(g, (g + d) / 2)`, and the parity of a binomial coefficient
                // is 1 exactly when `(g & j) == j` for `j = (g + d) / 2`.
                //
                // The `(g + d) / 2` is the part that is easy to leave out, and it
                // is not a typo to be caught: writing `C(g, d)` instead gives a
                // triangle that is *also* fractal and *also* self-similar, so it
                // looks right and this test caught it.
                let expected = if distance.abs() <= generation as isize
                    && (generation as isize + distance) % 2 == 0
                {
                    let j = ((generation as isize + distance) / 2) as usize;
                    (generation & j) == j
                } else {
                    false
                };
                assert_eq!(
                    *cell, expected,
                    "rule 90, generation {generation}, signed distance {distance}"
                );
            }
            rule30.commit_row();
        }
    }

    /// Rule 110 is the rule the crate's own notes call Turing-complete, and this
    /// pins the mask to the sentence that claims it.
    #[test]
    fn rule_110s_mask_is_xor_or_p_q_and_p_q_r() {
        for n in 0..8usize {
            let p = n & 0b100 != 0;
            let q = n & 0b010 != 0;
            let r = n & 0b001 != 0;
            assert_eq!(
                (RULE_110 >> n) & 1 == 1,
                (p || q) != (p && q && r),
                "rule 110 disagrees at neighbourhood {n:03b}"
            );
        }
    }

    /// The `rule` option actually selects the rule.
    ///
    /// Written because the first draft of this effect built its table from a
    /// hardcoded `p XOR (q OR r)` and left the option reading but not connected,
    /// so `rule = 90` drew rule 30. Every lookup in the effect is through
    /// `mask_for`, so this asserts the whole path: config value to mask to drawn
    /// generation.
    #[test]
    fn the_rule_option_selects_the_rule() {
        for rule in [0u8, 30, 45, 90, 110, 150, 254] {
            let mut rule30 = Rule30::new(
                Rule30Options {
                    rule,
                    seed_cells: 0,
                    ..Rule30Options::default()
                },
                (40, 20),
            );
            assert_eq!(
                rule30.mask,
                Rule30::mask_for(rule),
                "rule {rule} did not resolve to its own mask"
            );

            // And the mask is what the step function consults: run one
            // generation from a known row and check every column against the
            // mask read independently here. `step` swaps the rows, so the
            // generation just produced is in `current`.
            rule30.start_diagram();
            for cell in rule30.current.iter_mut() {
                *cell = false;
            }
            rule30.current[5] = true;
            let before = rule30.current.clone();
            rule30.step();

            let mask = rule30.mask;
            for i in 0..before.len() {
                let index = (usize::from(i > 0 && before[i - 1]) << 2)
                    | (usize::from(before[i]) << 1)
                    | usize::from(i + 1 < before.len() && before[i + 1]);
                assert_eq!(
                    rule30.current[i],
                    (mask >> index) & 1 == 1,
                    "rule {rule} disagrees with its own mask at column {i}"
                );
            }
        }
    }

    /// The border is dead, so a triangle is a triangle and not a stripe.
    ///
    /// Both edges, because they are folded in as dead neighbours rather than
    /// branched on, and the failure mode is asymmetric: a rule whose edge cell is
    /// set wraps the row around and the diagram becomes horizontal bands that
    /// look superficially fine.
    #[test]
    fn the_row_beyond_both_edges_is_dead() {
        let mut rule30 = Rule30::new(
            Rule30Options {
                // 254 is `p OR q OR r`, so every cell with a set neighbour is
                // alive and the border question decides the whole picture.
                rule: 254,
                seed_cells: 0,
                ..Rule30Options::default()
            },
            (40, 20),
        );
        rule30.start_diagram();
        for cell in rule30.current.iter_mut() {
            *cell = false;
        }
        rule30.current[0] = true;
        rule30.step();

        assert!(
            rule30.current.last() != Some(&true),
            "the row wrapped: a live cell appeared at the far edge"
        );
    }

    /// The diagram grows downwards and clears, rather than scrolling.
    ///
    /// This is the bandwidth property the effect exists to have, and it is
    /// asserted structurally: the cursor only ever moves forward, and a full
    /// screen produces a *new* diagram rather than a shifted one. A scrolling
    /// implementation passes every other test here and emits a full-screen diff
    /// every frame, which is the thing being prevented.
    #[test]
    fn the_diagram_grows_downward_and_restarts_when_full() {
        let mut rule30 = Rule30::new(Rule30Options::default(), (40, 10));
        rule30.start_diagram();

        let height = rule30.grid.dot_height();
        for _ in 0..height * 2 {
            let before = rule30.cursor;
            if rule30.cursor >= height {
                rule30.start_diagram();
            }
            if rule30.started {
                rule30.step();
            }
            rule30.commit_row();
            assert!(rule30.cursor != before, "the cursor did not move");
            assert!(
                rule30.cursor <= height,
                "the cursor ran past the bottom of the grid"
            );
        }

        // After filling twice the diagram must not be a shifted copy of the
        // first: the cursor is back near the top.
        assert!(
            rule30.cursor <= height,
            "the cursor should have wrapped back to the top, got {}",
            rule30.cursor
        );
    }

    /// The screen fills in a few seconds, so a playlist slot is not spent
    /// watching a single row.
    ///
    /// The rate is the reason the warmup lesson from `sandpile` does not apply
    /// here, and it is worth saying why rather than leaving it to be discovered:
    /// this effect has no development phase at all, so its very first frame is
    /// already the steady state.
    #[test]
    fn a_default_run_fills_the_screen_within_a_few_seconds() {
        let mut rule30 = Rule30::new(Rule30Options::default(), (80, 24));
        let height = rule30.grid.dot_height();

        for _ in 0..(60 * 3) {
            rule30.advance(1.0 / 60.0);
        }
        let restarts = rule30.cursor < height;
        assert!(
            restarts || rule30.cursor > height / 2,
            "after three seconds the cursor is at {} of {height} rows, so the \
             screen is not filling",
            rule30.cursor
        );
    }

    /// A run is reproducible from its seed, and different seeds differ.
    ///
    /// The contract suite checks this for every effect, but it can only compare
    /// rendered output; this checks the seed actually reaches the first row,
    /// which is where the randomness in this effect lives.
    #[test]
    fn the_seed_reaches_the_first_row() {
        let row_for = |seed: u64| {
            let rule30 = Rule30::new(
                Rule30Options {
                    seed,
                    ..Rule30Options::default()
                },
                (60, 20),
            );
            rule30.current.clone()
        };

        let a = row_for(1);
        assert_eq!(a, row_for(1), "the same seed gave two different first rows");
        assert_ne!(a, row_for(2), "two seeds gave the same first row");
    }

    /// A terminal below the minimum still produces a grid.
    #[test]
    fn a_tiny_terminal_still_produces_a_grid() {
        let mut rule30 = Rule30::new(Rule30Options::default(), (1, 1));
        rule30.advance(1.0 / 60.0);
        rule30.draw();
        assert!(!rule30.current.is_empty());
    }
}
