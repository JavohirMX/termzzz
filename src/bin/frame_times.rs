//! Prints per-effect frame cost as a plain table.
//!
//! Criterion is the right tool for tracking change over time, but its confidence
//! intervals are too wide to answer one-off questions like "is `opt-level = 3`
//! worth the extra binary size". This runs a fixed number of frames, reports the
//! median of several trials, and exits non-zero if any effect misses its budget,
//! so it can be used as a smoke check in CI.
//!
//! Four things are reported per effect, because they are unrelated costs and
//! averaging them hides all of them:
//!
//! * `update` — one simulation step.
//! * `update x4` — the worst case. `TickClock::MAX_STEPS` is 4, so at high
//!   `--speed` the runtime calls `update` up to four times per frame, and most
//!   `update` bodies cost roughly the same however many times they run.
//! * `render` — turning the simulation into a diff.
//! * `encode` + `bytes` — turning the diff into terminal output. The timing is
//!   CPU-side only: writing to a `Vec` never blocks, so it excludes real
//!   terminal latency and is a lower bound. The byte count is the portable
//!   proxy for how much work the terminal emulator is being asked to do.
//!
//! Usage: `cargo run --release --bin frame_times [--sizes] [--budget-ms N]`

use std::time::{Duration, Instant};

use termzzz::common::{TerminalEffect, write_cells};
use termzzz::config::Config;
use termzzz::registry::{AnyEffect, EffectId};
use termzzz::runtime::{FrameContext, InputState};

/// A 60 Hz frame is 16.6 ms. Anything under a tenth of that is comfortable.
const DEFAULT_BUDGET_MS: u64 = 2;
const WARMUP_FRAMES: u64 = 30;
const MEASURED_FRAMES: u64 = 120;
const TRIALS: usize = 5;
/// Matches `common::TickClock::MAX_STEPS`, the most times the runtime will call
/// `update` in a single frame.
const MAX_UPDATE_CALLS: u32 = 4;

const SIZES: &[(u16, u16)] = &[(80, 24), (200, 50), (400, 200)];

fn main() {
    let mut sizes: Vec<(u16, u16)> = SIZES.to_vec();
    let mut budget_ms = DEFAULT_BUDGET_MS;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                // A single WxH size, for bisecting a regression.
                let raw = args.next().unwrap_or_else(|| "200x50".to_string());
                let (w, h) = raw
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .unwrap_or((200, 50));
                sizes = vec![(w, h)];
            }
            "--budget-ms" => {
                budget_ms = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(DEFAULT_BUDGET_MS);
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let budget = Duration::from_millis(budget_ms);
    let config = Config::default();
    let mut over_budget: Vec<String> = Vec::new();

    for &(width, height) in &sizes {
        let cells = width as usize * height as usize;
        println!("\n{width}x{height} terminal ({cells} cells)");
        println!(
            "{:<14} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "effect", "update", "upd x4", "render", "encode", "bytes"
        );
        println!("{}", "-".repeat(70));

        for id in EffectId::all() {
            let update = median_of(&config, id, width, height, measure_update);
            let render = median_of(&config, id, width, height, measure_render);
            let (encode, bytes) = median_encode(&config, id, width, height);

            // The per-frame worst case: several simulation steps, one render,
            // and the output for it.
            let total = update * MAX_UPDATE_CALLS + render + encode;
            if total > budget {
                over_budget.push(format!("{} at {width}x{height}", id.as_str()));
            }

            println!(
                "{:<14} {:>10} {:>10} {:>10} {:>10} {:>10}",
                id.as_str(),
                fmt(update),
                fmt(update * MAX_UPDATE_CALLS),
                fmt(render),
                fmt(encode),
                bytes,
            );
        }
    }

    println!(
        "\n{measured_frames} frames x {TRIALS} trials, median reported. \\
         `upd x4` is the per-frame worst case at high --speed. \\
         `encode` is CPU-side only and excludes terminal latency; `bytes` is \\
         the ANSI volume per frame.",
        measured_frames = MEASURED_FRAMES
    );

    if !over_budget.is_empty() {
        eprintln!("over the {budget_ms}ms budget: {over_budget:?}");
        std::process::exit(1);
    }
}

fn fmt(duration: Duration) -> String {
    let micros = duration.as_secs_f64() * 1_000_000.0;
    if micros < 1.0 {
        format!("{micros:.2}us")
    } else if micros < 1_000.0 {
        format!("{micros:.1}us")
    } else {
        format!("{:.2}ms", micros / 1_000.0)
    }
}

/// Median of `TRIALS` samples of a per-frame measurement.
fn median_of<T>(
    config: &Config,
    id: EffectId,
    width: u16,
    height: u16,
    measure: fn(&Config, EffectId, u16, u16) -> T,
) -> T
where
    T: Ord + Copy,
{
    let mut samples: Vec<T> = (0..TRIALS)
        .map(|_| measure(config, id, width, height))
        .collect();
    samples.sort_unstable();
    samples[TRIALS / 2]
}

/// The measurement with the median encode time is the one whose byte count is
/// reported, so the two columns describe the same frame.
fn median_encode(
    config: &Config,
    id: EffectId,
    width: u16,
    height: u16,
) -> (Duration, usize) {
    let mut samples: Vec<(Duration, usize)> = (0..TRIALS)
        .map(|_| measure_encode(config, id, width, height))
        .collect();
    samples.sort_by_key(|(duration, _)| *duration);
    samples[TRIALS / 2]
}

fn make_input(width: u16, height: u16) -> InputState {
    let mut input = InputState::default();
    input.set_size((width, height));
    input
}

fn context(
    width: u16,
    height: u16,
    frame: u64,
    input: &InputState,
) -> FrameContext {
    FrameContext::new(
        (width, height),
        frame,
        Duration::from_micros(16_666 * frame),
        Duration::from_micros(16_666),
        input.clone(),
    )
}

/// A settled effect, so the measurement covers a steady state rather than
/// first-frame setup.
fn settled(config: &Config, id: EffectId, width: u16, height: u16) -> AnyEffect {
    let mut effect = AnyEffect::build(id, config, (width, height));
    let input = make_input(width, height);
    for frame in 0..WARMUP_FRAMES {
        effect.update_with_context(&context(width, height, frame, &input));
    }
    effect
}

fn measure_update(
    config: &Config,
    id: EffectId,
    width: u16,
    height: u16,
) -> Duration {
    let input = make_input(width, height);
    let mut effect = AnyEffect::build(id, config, (width, height));
    for frame in 0..WARMUP_FRAMES {
        effect.update_with_context(&context(width, height, frame, &input));
    }

    let started = Instant::now();
    for frame in 0..MEASURED_FRAMES {
        effect.update_with_context(&context(width, height, frame, &input));
    }
    started.elapsed() / MEASURED_FRAMES as u32
}

fn measure_render(
    config: &Config,
    id: EffectId,
    width: u16,
    height: u16,
) -> Duration {
    let input = make_input(width, height);
    let mut effect = settled(config, id, width, height);

    let started = Instant::now();
    for frame in 0..MEASURED_FRAMES {
        std::hint::black_box(
            effect.get_diff_with_context(&context(width, height, frame, &input)),
        );
    }
    started.elapsed() / MEASURED_FRAMES as u32
}

/// Times the output path and counts the bytes it produces.
fn measure_encode(
    config: &Config,
    id: EffectId,
    width: u16,
    height: u16,
) -> (Duration, usize) {
    let input = make_input(width, height);
    let mut effect = settled(config, id, width, height);
    let size = (width, height);

    let mut total = Duration::ZERO;
    let mut bytes = 0usize;

    for frame in 0..MEASURED_FRAMES {
        let diff =
            effect.get_diff_with_context(&context(width, height, frame, &input));

        let mut sink: Vec<u8> = Vec::with_capacity(diff.len() * 16);
        let started = Instant::now();
        write_cells(
            &mut sink,
            size,
            &diff,
            termzzz::session::SessionColors::default(),
        )
        .expect("writing to a Vec cannot fail");
        total += started.elapsed();

        bytes += sink.len();
        effect.update_with_context(&context(width, height, frame, &input));
    }

    (
        total / MEASURED_FRAMES as u32,
        bytes / MEASURED_FRAMES as usize,
    )
}
