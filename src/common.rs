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
    let mut speed = 1.0;
    process_runtime_events(effect, &mut source, &mut input, &mut size, &mut speed)
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

pub fn run_loop_with_options<W, TE>(
    stdout: &mut W,
    effect: &mut TE,
    iterations: Option<usize>,
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
    TE: TerminalEffect,
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
        RuntimeOptions::default(),
    )
}

pub fn run_loop_with_source_and_size_and_options<W, TE, S>(
    stdout: &mut W,
    effect: &mut TE,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
    options: RuntimeOptions,
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

fn process_runtime_events<TE, S>(
    effect: &mut TE,
    source: &mut S,
    input: &mut InputState,
    size: &mut (u16, u16),
    speed: &mut f32,
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
