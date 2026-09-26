use crate::buffer::Cell;
use crate::runtime::{
    CrosstermInput, FrameContext, InputEvent, InputSource, InputState, Key,
    KeyPhase,
};
use crossterm::style::Stylize;
use crossterm::{QueueableCommand, cursor, style, terminal};
use std::{
    io::{BufWriter, Result, Write},
    time::{Duration, Instant},
};

pub const MIN_EFFECT_SIZE: u16 = 6;
pub const MIN_SPEED: f32 = 0.05;
pub const MAX_SPEED: f32 = 8.0;
pub const SPEED_STEP: f32 = 0.1;

/// The generator every effect draws its randomness from.
///
/// This is deliberately *not* `ThreadRng`. `ThreadRng` cannot be seeded, so an
/// effect using it can neither be reproduced from a config value nor compared
/// against a second instance — which is why `tests/effect_contracts.rs` used to
/// report nine effects as having an unverifiable timebase. `StdRng` is seedable
/// and, for the handful of draws an effect makes per frame, costs nothing
/// measurable beside the per-cell work that dominates the frame.
pub type EffectRng = rand::rngs::StdRng;

/// The default seed, shared by every effect that has a `seed` field.
///
/// A fixed value keeps `termzzz --print-config` stable and makes a default run
/// reproducible, which is what lets the contract suite compare two instances. It
/// is not meant to be interesting; `--seed` is how you pick a different one.
pub const DEFAULT_SEED: u64 = 42;

/// Seeds a generator for one effect.
///
/// `salt` keeps two effects configured with the same `seed` from replaying an
/// identical sequence of numbers, which is not a bug but makes the effects look
/// implausibly synchronised — the same colour, the same glyph, the same instant
/// in both.
pub fn seeded_rng(seed: u64, salt: &str) -> EffectRng {
    use rand::SeedableRng;

    // FNV-1a rather than `DefaultHasher`: the standard library makes no promise
    // that `DefaultHasher`'s output is stable across releases, and a seed that
    // silently changes meaning between Rust versions would make every
    // "reproducible" run a lie.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in salt.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    EffectRng::seed_from_u64(seed ^ hash)
}

pub fn normalize_effect_size(size: (u16, u16)) -> (u16, u16) {
    (size.0.max(MIN_EFFECT_SIZE), size.1.max(MIN_EFFECT_SIZE))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeOptions {
    pub speed: f32,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self { speed: 1.0 }
    }
}

impl RuntimeOptions {
    pub fn new(speed: f32) -> Self {
        Self {
            speed: speed.clamp(MIN_SPEED, MAX_SPEED),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TickClock {
    accumulator: f32,
}

impl TickClock {
    pub const QUANTUM: f32 = 1.0 / 60.0;
    pub const MAX_STEPS: usize = 4;

    pub fn advance(&mut self, delta: Duration, speed: f32) -> usize {
        self.accumulator += delta.as_secs_f32() * speed.clamp(MIN_SPEED, MAX_SPEED);
        let mut steps = 0;
        while self.accumulator + f32::EPSILON >= Self::QUANTUM
            && steps < Self::MAX_STEPS
        {
            self.accumulator -= Self::QUANTUM;
            steps += 1;
        }
        if steps == Self::MAX_STEPS {
            self.accumulator = 0.0;
        }
        steps
    }
}

pub trait TerminalEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)>;
    fn update(&mut self);
    fn update_size(&mut self, width: u16, height: u16);
    fn reset(&mut self);

    fn handle_input(&mut self, _event: &InputEvent) {}

    fn get_diff_with_context(
        &mut self,
        _context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        self.get_diff()
    }

    fn update_with_context(&mut self, _context: &FrameContext) {
        self.update();
    }
}

/// Writes changed cells to the terminal.
///
/// Cells outside `size` are skipped: a diff is compared against the previous
/// frame, and an effect that has not yet caught up with a resize can report
/// coordinates derived from the old dimensions.
///
/// Two things keep the output small. A cursor move is only emitted when the cell
/// is not the one immediately after the previous one, so a run of cells along a
/// row costs one move rather than one per cell. And a styled glyph is only
/// emitted when the style actually changes, so a run of identically coloured
/// cells costs a bare glyph. Without this, repainting a full screen costs a
/// cursor move and a colour change per cell, which measured at over a megabyte
/// of escape sequences per frame for the plasma effect on a 400x200 terminal.
///
/// This is a free function rather than an inline loop so the output path can be
/// measured and tested without running a whole frame loop.
pub fn write_cells<W: Write>(
    out: &mut W,
    size: (u16, u16),
    cells: &[(usize, usize, Cell)],
) -> Result<()> {
    let (max_x, max_y) = (size.0 as usize, size.1 as usize);

    // Where the terminal cursor sits once everything written so far has been
    // drawn, and the style it is currently in.
    let mut cursor: Option<(u16, u16)> = None;
    let mut active: Option<(style::Color, style::Attribute)> = None;

    for &(x, y, cell) in cells {
        if x >= max_x || y >= max_y {
            continue;
        }
        let column = x as u16;
        let row = y as u16;

        if cursor != Some((column, row)) {
            out.queue(cursor::MoveTo(column, row))?;
        }

        if active == Some((cell.color, cell.attr)) {
            out.queue(style::Print(cell.symbol))?;
        } else {
            out.queue(style::PrintStyledContent(
                cell.symbol.with(cell.color).attribute(cell.attr),
            ))?;
            active = Some((cell.color, cell.attr));
        }

        cursor = Some((column + 1, row));
    }

    Ok(())
}

pub fn process_input<TE>(effect: &mut TE) -> Result<bool>
where
    TE: TerminalEffect,
{
    let mut size = terminal::size()?;
    let mut input = InputState::default();
    input.set_size(size);
    let mut source = CrosstermInput;
    let mut speed = 1.0;
    process_runtime_events(effect, &mut source, &mut input, &mut size, &mut speed)
}

pub fn run_loop<W>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
) -> Result<f64>
where
    W: Write,
{
    let mut source = CrosstermInput;
    run_loop_with_source(stdout, effect, iterations, &mut source)
}

pub fn run_loop_with_options<W>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
{
    let mut source = CrosstermInput;
    let size = terminal::size()?;
    run_loop_with_source_and_size_and_options(
        stdout,
        effect,
        iterations,
        &mut source,
        size,
        options,
    )
}

pub fn run_loop_with_source<W, S>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    source: &mut S,
) -> Result<f64>
where
    W: Write,
    S: InputSource,
{
    let size = terminal::size()?;
    run_loop_with_source_and_size(stdout, effect, iterations, source, size)
}

pub fn run_loop_with_source_and_size<W, S>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
) -> Result<f64>
where
    W: Write,
    S: InputSource,
{
    run_loop_with_source_and_size_and_options(
        stdout,
        effect,
        iterations,
        source,
        initial_size,
        RuntimeOptions::default(),
    )
}

/// The one frame loop. Every other `run_loop*` is a convenience wrapper over it.
///
/// Takes a trait object rather than a generic. A generic would monomorphise the
/// entire loop once per effect type -- fifteen copies of the timing, input and
/// encoding code -- and would forbid holding effects of different types at once,
/// which is what swapping the running effect needs.
pub fn run_loop_with_source_and_size_and_options<W, S>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
    S: InputSource,
{
    if iterations == Some(0) {
        return Ok(0.0);
    }

    let mut size = (initial_size.0.max(1), initial_size.1.max(1));
    let mut input = InputState::default();
    input.set_size(size);
    let effect_size = normalize_effect_size(size);
    if effect_size != size {
        effect.update_size(effect_size.0, effect_size.1);
        effect.reset();
    }
    let started_at = Instant::now();
    let mut previous_frame = started_at;
    let mut frame = 0u64;
    let mut is_running = true;
    let mut frames_per_second = 0.0;
    let mut speed = options.speed.clamp(MIN_SPEED, MAX_SPEED);
    let mut tick_clock = TickClock::default();
    let target_frame_duration = Duration::from_secs_f64(1.0 / 60.0);
    let mut buffered_stdout = BufWriter::new(stdout);

    while is_running {
        let frame_started_at = Instant::now();
        let delta = frame_started_at
            .saturating_duration_since(previous_frame)
            .min(Duration::from_millis(100));
        let elapsed = frame_started_at.duration_since(started_at);
        previous_frame = frame_started_at;
        input.begin_frame();

        if !process_runtime_events(
            effect, source, &mut input, &mut size, &mut speed,
        )? {
            break;
        }

        let context = FrameContext::new(size, frame, elapsed, delta, input.clone());
        let diff = effect.get_diff_with_context(&context);
        write_cells(&mut buffered_stdout, size, &diff)?;
        buffered_stdout.flush()?;

        if (speed - 1.0).abs() < f32::EPSILON {
            effect.update_with_context(&context);
        } else {
            let steps = tick_clock.advance(delta, speed);
            let mut tick_context = context.clone();
            tick_context.delta = Duration::from_secs_f64(TickClock::QUANTUM as f64);
            for _ in 0..steps {
                effect.update_with_context(&tick_context);
            }
        }

        let frame_duration = frame_started_at.elapsed();
        if frame_duration < target_frame_duration {
            std::thread::sleep(target_frame_duration - frame_duration);
        }

        let measured_duration =
            frame_started_at.elapsed().as_secs_f64().max(f64::EPSILON);
        frames_per_second = (frames_per_second + 1.0 / measured_duration) / 2.0;

        frame += 1;
        if iterations.is_some_and(|limit| frame as usize >= limit) {
            is_running = false;
        }
    }

    Ok(frames_per_second)
}

fn process_runtime_events<S>(
    effect: &mut dyn TerminalEffect,
    source: &mut S,
    input: &mut InputState,
    size: &mut (u16, u16),
    speed: &mut f32,
) -> Result<bool>
where
    S: InputSource,
{
    let events = source.poll(Duration::from_millis(10))?;
    if events.iter().any(is_quit_event) {
        return Ok(false);
    }

    let last_resize = events.iter().rev().find_map(|event| match event {
        InputEvent::Resize { size } => Some(*size),
        _ => None,
    });
    if let Some(new_size) = last_resize {
        *size = (new_size.0.max(1), new_size.1.max(1));
        input.set_size(*size);
        let effect_size = normalize_effect_size(*size);
        effect.update_size(effect_size.0, effect_size.1);
        effect.reset();
    }

    for event in events {
        if matches!(event, InputEvent::Resize { .. }) {
            continue;
        }
        input.apply(event);
        if let InputEvent::Key { key, phase } = event
            && matches!(phase, KeyPhase::Pressed | KeyPhase::Repeated)
        {
            match key {
                Key::Char('+') | Key::Char('=') => {
                    *speed = (*speed + SPEED_STEP).min(MAX_SPEED);
                    continue;
                }
                Key::Char('-') => {
                    *speed = (*speed - SPEED_STEP).max(MIN_SPEED);
                    continue;
                }
                _ => {}
            }
        }
        if !matches!(event, InputEvent::Ignored) {
            effect.handle_input(&event);
        }
    }
    Ok(true)
}

fn is_quit_event(event: &InputEvent) -> bool {
    match event {
        InputEvent::Quit => true,
        InputEvent::Key { key, phase } => {
            matches!(phase, KeyPhase::Pressed | KeyPhase::Repeated)
                && matches!(key, Key::Char('q') | Key::Escape | Key::CtrlC)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::style::Color;

    fn cell(symbol: char) -> Cell {
        Cell::new(symbol, Color::Red, style::Attribute::Bold)
    }

    /// Counts cursor moves, which are the CSI sequences ending in `H`.
    ///
    /// Counting every escape byte is not useful here: crossterm emits three of
    /// them per styled glyph (colour, attribute, reset), so the number being
    /// asserted has to be the cursor moves specifically.
    fn count_moves(out: &str) -> usize {
        out.split("\u{1b}[").filter(|seq| seq.contains('H')).count()
    }

    fn encoded(size: (u16, u16), cells: &[(usize, usize, Cell)]) -> String {
        let mut out: Vec<u8> = Vec::new();
        write_cells(&mut out, size, cells).expect("writing to a Vec cannot fail");
        String::from_utf8(out).expect("crossterm emits utf8")
    }

    #[test]
    fn writes_nothing_for_an_empty_diff() {
        assert_eq!(encoded((10, 10), &[]), "");
    }

    #[test]
    fn encodes_a_cursor_move_and_the_styled_glyph() {
        let out = encoded((10, 10), &[(3, 4, cell('x'))]);
        assert!(
            out.contains("\u{1b}[5;4H"),
            "expected a move to row 5 column 4, got {out:?}"
        );
        assert!(out.contains('x'), "expected the glyph, got {out:?}");
    }

    #[test]
    fn drops_cells_outside_the_terminal() {
        // A diff produced against a stale previous frame can report coordinates
        // from the old size. Those must not reach the terminal.
        let cells = [
            (0usize, 0usize, cell('a')),
            (99, 0, cell('b')),
            (0, 99, cell('c')),
        ];
        let out = encoded((10, 10), &cells);

        assert!(out.contains('a'), "the in-bounds cell was dropped");
        assert!(!out.contains('b'), "a cell past the right edge was written");
        assert!(
            !out.contains('c'),
            "a cell past the bottom edge was written"
        );
    }

    #[test]
    fn the_last_row_and_column_are_inside_the_terminal() {
        // Off-by-one guard: index 9 of a 10-wide terminal is the last column,
        // not out of bounds.
        let cells = [(9usize, 9usize, cell('z'))];
        let out = encoded((10, 10), &cells);

        assert!(
            out.contains('z'),
            "the last cell was treated as out of bounds"
        );
    }

    #[test]
    fn encodes_every_cell_in_a_dense_diff() {
        let cells: Vec<(usize, usize, Cell)> =
            (0..100).map(|i| (i % 10, i / 10, cell('#'))).collect();
        let out = encoded((10, 10), &cells);

        assert_eq!(out.matches('#').count(), 100);
    }

    #[test]
    fn a_full_screen_diff_stays_within_budget() {
        // The output path is the one place that talks to the terminal, so its
        // volume is worth pinning. A full repaint used to cost a cursor move and
        // a colour change per cell, which measured at over a megabyte per frame
        // for a 200x50 screen.
        let cells: Vec<(usize, usize, Cell)> = (0..200 * 50)
            .map(|i| (i % 200, i / 200, cell('#')))
            .collect();
        let out = encoded((200, 50), &cells);

        assert_eq!(out.matches('#').count(), 200 * 50);
        assert!(
            out.len() < 200 * 50 * 2,
            "a full-screen repaint of uniformly styled cells encoded to {} bytes, \
             which is more than the two bytes per glyph it needs",
            out.len()
        );
    }

    #[test]
    fn a_row_of_adjacent_cells_costs_one_cursor_move() {
        let cells: Vec<(usize, usize, Cell)> =
            (0..10).map(|x| (x, 0, cell('#'))).collect();
        let out = encoded((20, 5), &cells);

        assert_eq!(
            count_moves(&out),
            1,
            "expected a single cursor move for ten adjacent cells, got {out:?}"
        );
    }

    #[test]
    fn a_gap_in_a_row_forces_a_new_cursor_move() {
        let cells = [(0usize, 0usize, cell('#')), (5, 0, cell('#'))];
        let out = encoded((20, 5), &cells);

        assert_eq!(
            count_moves(&out),
            2,
            "expected a cursor move per cell across the gap, got {out:?}"
        );
    }

    #[test]
    fn wrapping_to_the_next_row_forces_a_new_cursor_move() {
        // The last column of a row is not followed by the first column of the
        // next one, even though the x coordinate appears to continue.
        let cells = [(4usize, 0usize, cell('#')), (0, 1, cell('#'))];
        let out = encoded((5, 5), &cells);

        assert_eq!(
            count_moves(&out),
            2,
            "expected a cursor move per row, got {out:?}"
        );
    }

    #[test]
    fn a_style_change_is_emitted_even_mid_row() {
        let plain = Cell::new('#', Color::Red, style::Attribute::Bold);
        let other = Cell::new('#', Color::Blue, style::Attribute::Bold);
        let cells = [(0usize, 0usize, plain), (1, 0, other), (2, 0, plain)];
        let out = encoded((10, 5), &cells);

        // Still a single cursor move: only the style changed, not the position.
        assert_eq!(count_moves(&out), 1, "got {out:?}");
        // But every cell re-states its colour, because it alternates.
        assert_eq!(out.matches('#').count(), 3, "got {out:?}");
    }
}
