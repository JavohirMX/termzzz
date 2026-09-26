//! Per-effect frame benchmarks.
//!
//! These drive the effects directly rather than through `common::run_loop`,
//! because that constructs a crossterm input source and queries the terminal
//! size, which needs an interactive terminal and cannot run in CI.
//!
//! `update` and `render` are measured separately and deliberately not summed.
//! They are unrelated costs: an effect can have a cheap simulation and an
//! expensive raster, or the reverse, and averaging them hides both.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::time::Duration;

use termzzz::common::TerminalEffect;
use termzzz::config::Config;
use termzzz::registry::{AnyEffect, EffectId};
use termzzz::runtime::{FrameContext, InputState};

/// A representative large terminal. The interesting cost here is per-cell work,
/// so it scales with the cell count.
const SIZE: (u16, u16) = (200, 50);

const DELTA: Duration = Duration::from_micros(16_666);

fn context(frame: u64, input: &InputState) -> FrameContext {
    FrameContext::new(SIZE, frame, DELTA * frame as u32, DELTA, input.clone())
}

fn input() -> InputState {
    let mut state = InputState::default();
    state.set_size(SIZE);
    state
}

fn measure(c: &mut Criterion, id: EffectId) {
    let config = Config::default();
    let update_name = format!("{}/update", id.as_str());
    let render_name = format!("{}/render", id.as_str());

    c.bench_function(update_name.as_str(), |b| {
        b.iter_batched(
            || AnyEffect::build(id, &config, SIZE),
            |mut effect| {
                let input = input();
                for frame in 0..10u64 {
                    effect.update_with_context(&context(frame, &input));
                }
                black_box(effect)
            },
            criterion::BatchSize::SmallInput,
        );
    });

    c.bench_function(render_name.as_str(), |b| {
        b.iter_batched(
            || {
                let mut effect = AnyEffect::build(id, &config, SIZE);
                // Let the simulation reach a steady state first, so this
                // measures rendering rather than first-frame setup.
                let input = input();
                for frame in 0..30u64 {
                    effect.update_with_context(&context(frame, &input));
                }
                (effect, input)
            },
            |(mut effect, input)| {
                for frame in 0..10u64 {
                    black_box(
                        effect.get_diff_with_context(&context(frame, &input)),
                    );
                }
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn effect_benchmarks(c: &mut Criterion) {
    for id in EffectId::all() {
        measure(c, id);
    }
}

criterion_group!(benches, effect_benchmarks);
criterion_main!(benches);
