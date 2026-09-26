use criterion::{Criterion, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};
use termzzz::{
    common::{self, TerminalEffect},
    rain::{digital_rain, rain_drop},
};

/// A generator that differs on every call, so a measurement never depends on
/// how much randomness the previous iteration happened to consume.
///
/// `rand::rng()` used to serve this purpose and cannot any more: it is not
/// seedable, and the effects now take a seedable generator so they can be
/// reproduced.
fn fresh_rng(tag: &str) -> common::EffectRng {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    common::seeded_rng(COUNTER.fetch_add(1, Ordering::Relaxed), tag)
}

fn get_sane_options() -> digital_rain::DigitalRainOptions {
    digital_rain::DigitalRainOptions {
        drops_range: (10, 20),
        speed_range: (2, 16),
        ..Default::default()
    }
}

/// Drives the full frame loop, including terminal encoding.
///
/// This needs a real terminal, because `run_loop` constructs a crossterm input
/// source and queries the terminal size. It is therefore not runnable in CI and
/// reports nothing useful when it fails, so the failure is surfaced instead of
/// being swallowed.
fn run_loop_benchmark(_c: &mut Criterion) {
    let mut cc = Criterion::default()
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .sample_size(100);

    cc.bench_function("run_loop/three_frames", |b| {
        let options = get_sane_options();
        let mut rain = digital_rain::DigitalRain::new(options, (80, 40));

        b.iter(|| {
            // Allocated inside the closure: hoisting it out would let the sink
            // accumulate every frame of every iteration and grow without bound,
            // so the reported cost would drift upward for the whole run.
            let mut stdout = Vec::new();
            let result =
                common::run_loop(black_box(&mut stdout), &mut rain, Some(3));
            black_box(result.expect("run_loop needs an interactive terminal"));
        });
    });
}

fn vertical_worm_benchmark(c: &mut Criterion) {
    c.bench_function("raindrop/new_1000", |b| {
        b.iter(|| {
            let mut rng = fresh_rng("rain_bench");
            for index in 1..=1000 {
                black_box(rain_drop::RainDrop::new(
                    (80, 40),
                    &get_sane_options(),
                    index,
                    &mut rng,
                ));
            }
        })
    });

    let options = get_sane_options();
    let delta = Duration::from_millis(50);
    let mut drops: Vec<rain_drop::RainDrop> = Vec::with_capacity(1000);
    {
        let mut rng = fresh_rng("rain_bench");
        for index in 1..=1000 {
            drops.push(rain_drop::RainDrop::new(
                (80, 40),
                &options,
                index,
                &mut rng,
            ));
        }
    }

    c.bench_function("raindrop/update_1000", |b| {
        // A fresh generator per iteration, so the measurement does not depend on
        // how much randomness the previous iteration happened to consume.
        b.iter_batched(
            || {
                let mut rng = fresh_rng("rain_bench");
                let mut fresh: Vec<rain_drop::RainDrop> = Vec::with_capacity(1000);
                for index in 1..=1000 {
                    fresh.push(rain_drop::RainDrop::new(
                        (80, 40),
                        &options,
                        index,
                        &mut rng,
                    ));
                }
                fresh
            },
            |mut drops| {
                let mut rng = fresh_rng("rain_bench");
                for drop in drops.iter_mut() {
                    drop.update((80, 40), &options, delta, &mut rng);
                }
                black_box(drops)
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

fn digital_rain_benchmark(c: &mut Criterion) {
    c.bench_function("rain/new", |b| {
        b.iter(|| {
            // Constructed inside the closure on purpose: this benchmark is about
            // the cost of building an effect.
            black_box(digital_rain::DigitalRain::new(get_sane_options(), (80, 40)));
        })
    });

    // The old version of this benchmark built a fresh effect inside `b.iter`,
    // so it measured construction plus ten updates and reported it as ten
    // updates. The effect is now built once per batch instead.
    let options = get_sane_options();
    c.bench_function("rain/update_10", |b| {
        b.iter_batched(
            || digital_rain::DigitalRain::new(options.clone(), (80, 40)),
            |mut rain| {
                for _ in 0..10 {
                    rain.update();
                }
                black_box(rain)
            },
            criterion::BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    run_loop_benchmark,
    vertical_worm_benchmark,
    digital_rain_benchmark
);
criterion_main!(benches);
