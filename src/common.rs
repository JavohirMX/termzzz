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

pub fn normalize_effect_size(size: (u16, u16)) -> (u16, u16) {
    (size.0.max(MIN_EFFECT_SIZE), size.1.max(MIN_EFFECT_SIZE))
}

pub trait DefaultOptions {
    type Options;

    fn default_options(width: u16, height: u16) -> Self::Options;
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

pub fn process_input<TE>(effect: &mut TE) -> Result<bool>
where
    TE: TerminalEffect,
{
    let mut size = terminal::size()?;
    let mut input = InputState::default();
    input.set_size(size);
    let mut source = CrosstermInput;
    process_runtime_events(effect, &mut source, &mut input, &mut size)
}

pub fn run_loop<W, TE>(
    stdout: &mut W,
    effect: &mut TE,
    iterations: Option<usize>,
) -> Result<f64>
where
    W: Write,
    TE: TerminalEffect,
{
    let mut source = CrosstermInput;
    run_loop_with_source(stdout, effect, iterations, &mut source)
}

pub fn run_loop_with_source<W, TE, S>(
    stdout: &mut W,
    effect: &mut TE,
    iterations: Option<usize>,
    source: &mut S,
) -> Result<f64>
where
    W: Write,
    TE: TerminalEffect,
    S: InputSource,
{
    let size = terminal::size()?;
    run_loop_with_source_and_size(stdout, effect, iterations, source, size)
}

pub fn run_loop_with_source_and_size<W, TE, S>(
    stdout: &mut W,
    effect: &mut TE,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
) -> Result<f64>
where
    W: Write,
    TE: TerminalEffect,
    S: InputSource,
{
    run_loop_with_source_and_size_and_options(
        stdout,
        effect,
        iterations,
        source,
        initial_size,
    )
}

pub fn run_loop_with_source_and_size_and_options<W, TE, S>(
    stdout: &mut W,
    effect: &mut TE,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
) -> Result<f64>
where
    W: Write,
    TE: TerminalEffect,
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

        if !process_runtime_events(effect, source, &mut input, &mut size)? {
            break;
        }

        let context = FrameContext::new(size, frame, elapsed, delta, input.clone());
        let queue = effect.get_diff_with_context(&context);
        for (x, y, cell) in queue {
            if x >= size.0 as usize || y >= size.1 as usize {
                continue;
            }
            buffered_stdout.queue(cursor::MoveTo(x as u16, y as u16))?;
            buffered_stdout.queue(style::PrintStyledContent(
                cell.symbol.with(cell.color).attribute(cell.attr),
            ))?;
        }
        buffered_stdout.flush()?;

        effect.update_with_context(&context);

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

fn process_runtime_events<TE, S>(
    effect: &mut TE,
    source: &mut S,
    input: &mut InputState,
    size: &mut (u16, u16),
) -> Result<bool>
where
    TE: TerminalEffect,
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
