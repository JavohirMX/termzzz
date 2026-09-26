//! Contract and drift tests for the effect catalogue.
//!
//! These exist so that adding an effect, or editing an existing one, cannot
//! silently break the registry, the config surface, or the runtime contract.
//! Every assertion here is deliberately generic: it derives its expectations
//! from `EffectId::all()` and `Config` rather than hard-coding per-effect
//! values, so a newly added effect is covered without editing this file.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use termzzz::buffer::Cell;
use termzzz::common::TerminalEffect;
use termzzz::config::Config;
use termzzz::registry::{AnyEffect, EffectId};

// The effect names the CLI accepts are checked once, against
// `registry::KNOWN_IDS`, in `registry.rs`. They are deliberately not listed
// here: this file iterates `EffectId::all()` everywhere else, so a second copy
// of the names would only be able to drift.

/// Splits serialized TOML into `(section_name, body)` pairs. The `[global]`
/// table is included so a partial-config test can drop any one section.
fn split_toml_sections(contents: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            sections.push((name.to_string(), String::new()));
        } else if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    sections
}

fn default_config_toml() -> String {
    toml::to_string_pretty(&Config::default()).expect("default config serializes")
}

/// Runs `frames` update/render cycles and returns the largest number of cells
/// emitted in any single frame, asserting every emitted coordinate is inside
/// the terminal.
fn drive(id: EffectId, size: (u16, u16), frames: u64) -> usize {
    let config = Config::default();
    let mut effect = AnyEffect::build(id, &config, size);

    let mut input = termzzz::runtime::InputState::default();
    input.set_size(size);

    let (width, height) = (size.0 as usize, size.1 as usize);
    let mut worst_frame = 0usize;

    for frame in 0..frames {
        let context = termzzz::runtime::FrameContext::new(
            size,
            frame,
            Duration::from_secs_f64(frame as f64 / 60.0),
            Duration::from_secs_f64(1.0 / 60.0),
            input.clone(),
        );

        let diff = effect.get_diff_with_context(&context);
        for (x, y, _) in &diff {
            assert!(
                *x < width && *y < height,
                "{id:?} emitted out-of-bounds cell ({x}, {y}) on a {width}x{height} \
                 terminal at frame {frame}"
            );
        }
        worst_frame = worst_frame.max(diff.len());

        effect.update_with_context(&context);
    }

    worst_frame
}

// --- registry drift -------------------------------------------------------
//
// Registry completeness, name uniqueness and metadata all live in
// `registry.rs`'s own tests, next to the table they check and using
// `KNOWN_IDS` as the single hand-maintained list of effects. Nothing is
// re-listed here: a second copy of the effect names is exactly the kind of
// drift this suite exists to catch.

#[test]
fn effect_names_are_unique() {
    let unique: HashSet<&str> = EffectId::all().map(|id| id.as_str()).collect();
    assert_eq!(
        unique.len(),
        EffectId::len(),
        "two effects share a name, so one of them is unreachable"
    );
}

#[test]
fn every_effect_round_trips_through_its_name() {
    for id in EffectId::all() {
        let parsed: EffectId = id.as_str().parse().expect("name parses back");
        assert_eq!(parsed, id, "{} did not round-trip", id.as_str());
    }
}

#[test]
fn every_effect_has_usable_metadata() {
    for id in EffectId::all() {
        assert!(
            !id.description().trim().is_empty(),
            "{id:?} has an empty description, which would render a blank help line"
        );

        let duration = id.default_duration();
        assert!(
            duration.is_finite() && duration > 0.0,
            "{id:?} has a non-positive playlist duration ({duration})"
        );
    }
}

#[test]
fn unknown_effect_names_are_rejected() {
    assert!("not-an-effect".parse::<EffectId>().is_err());
    assert!("".parse::<EffectId>().is_err());
}

// --- config drift ---------------------------------------------------------

#[test]
fn default_config_is_stable_across_a_toml_round_trip() {
    let first = default_config_toml();
    let parsed: Config = toml::from_str(&first).expect("default config parses");
    let second = toml::to_string_pretty(&parsed).expect("re-serializes");

    assert_eq!(
        first, second,
        "serializing the default config is lossy or unstable; a field is most \
         likely marked #[serde(skip)] without being re-derived on load"
    );
}

/// The core config-drift guard. Omitting any single section from a user's
/// config file must leave every other section at its real defaults. This
/// fails loudly when an options struct's `Default` derive disagrees with the
/// values its builder actually uses.
#[test]
fn omitting_any_single_config_section_preserves_real_defaults() {
    let full = default_config_toml();
    let sections = split_toml_sections(&full);
    assert!(
        sections.len() > 1,
        "expected the default config to serialize into multiple sections"
    );

    let mut zeroed: Vec<String> = Vec::new();

    for (omitted, _) in &sections {
        let mut partial = String::new();
        for (name, body) in &sections {
            if name == omitted {
                continue;
            }
            partial.push_str(&format!("[{name}]\n{body}"));
        }

        let parsed: Config = toml::from_str(&partial).unwrap_or_else(|error| {
            panic!(
                "config without the [{omitted}] section failed to parse: {error}"
            )
        });
        let reserialized = toml::to_string_pretty(&parsed).expect("re-serializes");

        if full != reserialized {
            zeroed.push(omitted.clone());
        }
    }

    assert!(
        zeroed.is_empty(),
        "dropping these sections from a user config silently resets them to zero \
         instead of keeping their real defaults: {zeroed:?}. Each of these options \
         structs derives Default while its real defaults live in #[builder(default)]."
    );
}

#[test]
fn an_empty_config_file_yields_real_defaults() {
    let parsed: Config = toml::from_str("").expect("empty config parses");
    assert_eq!(
        default_config_toml(),
        toml::to_string_pretty(&parsed).expect("re-serializes"),
        "an empty config file should behave exactly like no config file"
    );
}

#[test]
fn every_effect_builds_from_the_default_config() {
    for id in EffectId::all() {
        let _ = AnyEffect::build(id, &Config::default(), (80, 24));
    }
}

/// Every registered effect needs somewhere to keep its settings, or a user has
/// no way to configure it. The section name comes from the registry, so this
/// fails as soon as an effect is added without a matching `Config` field.
#[test]
fn every_effect_has_a_config_section() {
    let serialized =
        toml::to_string_pretty(&Config::default()).expect("serializes");

    for id in EffectId::all() {
        let section = id.config_section();
        assert!(
            serialized.contains(&format!("[{section}]")),
            "{} registers the config section {section:?}, but Config has no \
             matching field",
            id.as_str()
        );
    }
}

// --- runtime contract: smoke ---------------------------------------------

/// The terminal clamps effect size to `MIN_EFFECT_SIZE`, so 6x6 is the real
/// minimum the CLI can ever ask for. Every effect must cope with it.
#[test]
fn every_effect_survives_the_smallest_supported_terminal() {
    for id in EffectId::all() {
        drive(id, (6, 6), 120);
    }
}

#[test]
fn every_effect_survives_a_normal_terminal() {
    for id in EffectId::all() {
        drive(id, (80, 24), 200);
    }
}

/// Wide-and-short and tall-and-narrow catch effects that assume a roughly
/// square aspect ratio.
#[test]
fn every_effect_survives_extreme_aspect_ratios() {
    for id in EffectId::all() {
        drive(id, (200, 8), 120);
        drive(id, (8, 200), 120);
    }
}

#[test]
fn every_effect_survives_a_large_terminal() {
    for id in EffectId::all() {
        drive(id, (200, 50), 90);
    }
}

/// Long runs catch effects that accumulate state, grow a collection without
/// bound, or only fail once the simulation reaches a certain phase. Ten
/// seconds of wall clock is enough to reach a playlist's per-effect duration.
#[test]
fn every_effect_survives_a_long_run() {
    for id in EffectId::all() {
        drive(id, (80, 24), 600);
    }
}

/// `update_size` is a public entry point on a public type, so it has to leave
/// the effect renderable on its own. The runtime happens to call `reset`
/// straight afterwards, but that ordering is not part of the trait contract
/// and a new caller should not be able to corrupt the screen by omitting it.
#[test]
fn effects_stay_in_bounds_when_resized_without_a_reset() {
    let mut escaped: Vec<&str> = Vec::new();
    let mut panicked: Vec<&str> = Vec::new();

    for id in EffectId::all() {
        // Each effect is probed in isolation so one panic does not hide the
        // rest of the report.
        let probe = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let config = Config::default();
            let mut effect = AnyEffect::build(id, &config, (80, 24));
            effect.update_size(20, 10);

            let mut input = termzzz::runtime::InputState::default();
            input.set_size((20, 10));
            let context = termzzz::runtime::FrameContext::new(
                (20, 10),
                0,
                Duration::ZERO,
                Duration::from_secs_f64(1.0 / 60.0),
                input,
            );

            effect
                .get_diff_with_context(&context)
                .iter()
                .any(|(x, y, _)| *x >= 20 || *y >= 10)
        }));

        match probe {
            Ok(true) => escaped.push(id.as_str()),
            Ok(false) => {}
            Err(_) => panicked.push(id.as_str()),
        }
    }

    assert!(
        panicked.is_empty() && escaped.is_empty(),
        "resize handling is broken. panicked with an out-of-bounds write into \
         its own buffer: {panicked:?}. emitted cells outside the new terminal \
         size: {escaped:?}. update_size has to leave the previous-frame buffer \
         at the new size, because Buffer::diff derives cell coordinates from \
         the previous buffer's dimensions."
    );
}

/// No effect may repaint more cells in a single frame than the terminal has.
/// This is the cheap guard against a new effect accidentally going quadratic.
#[test]
fn no_effect_repaints_more_cells_than_the_screen_has() {
    let size = (120u16, 40u16);
    let cells = (size.0 as usize) * (size.1 as usize);

    for id in EffectId::all() {
        let worst = drive(id, size, 30);
        assert!(
            worst <= cells,
            "{id:?} repainted {worst} cells in a single frame on a screen with \
             only {cells} cells"
        );
    }
}

// --- simulation contract --------------------------------------------------

/// `update_with_context` exists so an effect can advance by real elapsed
/// time. An effect that ignores `context.delta` runs at a different speed on
/// a 30 Hz terminal than on a 144 Hz one, and `+`/`-` stops meaning what the
/// help text says. Effects whose output is genuinely static (they render
/// nothing after their first frame) are exempt.
/// `TerminalEffect` provides two update paths: a bare `update()` that takes no
/// timing information, and `update_with_context()` whose default implementation
/// just calls `update()`. An effect that does not override the latter advances
/// by an amount unrelated to elapsed time, so it runs at a different speed on a
/// 30 Hz terminal than on a 144 Hz one, and the `+`/`-` speed keys do not mean
/// what the help text says.
///
/// Effects that keep an internal fixed-step accumulator (life's generations per
/// second, for example) are fine: they consume `context.delta` and choose their
/// own quantum. So this test only asks whether the override exists.
///
/// Effects that draw from an unseeded generator cannot be compared against a
/// second instance and are reported rather than silently skipped.
#[test]
fn effects_advance_using_the_frame_delta() {
    let size = (60u16, 20u16);
    let config = Config::default();

    // Two rates, and the same number of frames at each.
    //
    // The obvious probe — one frame at a large delta — cannot tell a
    // delta-driven effect from a fixed-step one. Every effect here caps its
    // delta at 0.1s so a stall cannot teleport it, and at 9 cells per second
    // 0.1s is 0.9 of a cell: the position truncates to the same integer either
    // way and the frames come out identical. Running the same sixty frames at
    // three times the rate makes the difference many cells wide, which no amount
    // of sub-cell truncation can hide, while 0.05s stays under every cap in the
    // crate (the 0.1s delta clamps, and fire's four-steps-per-frame ceiling
    // admits the three steps 0.05s asks for).
    const FRAMES: u64 = 60;
    let nominal = Duration::from_secs_f64(1.0 / 60.0);
    let faster = Duration::from_secs_f64(0.05);

    let run = |id: EffectId, delta: Duration| {
        let mut effect = AnyEffect::build(id, &config, size);
        let mut input = termzzz::runtime::InputState::default();
        input.set_size(size);

        for frame in 0..FRAMES {
            let step = termzzz::runtime::FrameContext::new(
                size,
                frame,
                delta * frame as u32,
                delta,
                input.clone(),
            );
            effect.get_diff_with_context(&step);
            effect.update_with_context(&step);
        }

        let view = termzzz::runtime::FrameContext::new(
            size,
            FRAMES,
            delta * FRAMES as u32,
            delta,
            input,
        );
        effect.get_diff_with_context(&view)
    };

    let mut ignore_delta: Vec<&str> = Vec::new();
    let mut nondeterministic: Vec<&str> = Vec::new();

    for id in EffectId::all() {
        // A static effect has no time dependence to express, so every rate
        // renders the same still image. Compare two settled frames rather than
        // the first one: a lazily generated effect renders its content on its
        // first frame and nothing on every frame after, which would otherwise
        // look like animation.
        let is_static = {
            let mut effect = AnyEffect::build(id, &config, size);
            let mut input = termzzz::runtime::InputState::default();
            input.set_size(size);
            let mut settled = Vec::new();
            for frame in 0..40u64 {
                let step = termzzz::runtime::FrameContext::new(
                    size,
                    frame,
                    nominal * frame as u32,
                    nominal,
                    input.clone(),
                );
                let diff = effect.get_diff_with_context(&step);
                if frame == 5 {
                    settled = diff;
                }
                effect.update_with_context(&step);
            }
            let later = termzzz::runtime::FrameContext::new(
                size,
                40,
                nominal * 40,
                nominal,
                input,
            );
            effect.get_diff_with_context(&later) == settled
        };

        if is_static {
            continue;
        }

        // An effect that draws from an unseeded generator produces different
        // output every time it is built, so two instances can never be
        // compared. Sampled several times rather than twice: a single pair can
        // coincide by chance, which would make this check flaky in both
        // directions.
        let baseline = run(id, nominal);
        let repeats_match = (0..4).all(|_| run(id, nominal) == baseline);
        if !repeats_match {
            nondeterministic.push(id.as_str());
            continue;
        }

        if baseline == run(id, faster) {
            ignore_delta.push(id.as_str());
        }
    }

    assert!(
        ignore_delta.is_empty(),
        "these effects render the same frame at 60fps as they do at 20fps, so \
         their speed depends on the terminal refresh rate: {ignore_delta:?}. \
         Override update_with_context and drive the simulation from \
         context.delta."
    );

    // Not a failure. An effect that draws from an unseeded generator cannot be
    // compared against a second instance, so this test cannot see whether it
    // honours the delta. Reported rather than silently skipped, because it is a
    // real coverage gap: making these effects seedable would both close the gap
    // and give users reproducible output.
    if !nondeterministic.is_empty() {
        eprintln!(
            "note: timebase unverified for unseeded effects: {nondeterministic:?}"
        );
    }
}

#[test]
fn cells_stay_well_formed_under_repeated_input() {
    let size = (40u16, 16u16);
    let config = Config::default();
    let size_for_input = size;

    for id in EffectId::all() {
        let mut effect = AnyEffect::build(id, &config, size);
        let mut input = termzzz::runtime::InputState::default();
        input.set_size(size_for_input);

        for frame in 0..400u64 {
            // Poke every interactive affordance the runtime can deliver.
            if frame % 7 == 0 {
                effect.handle_input(&termzzz::runtime::InputEvent::Pointer {
                    position: ((frame % 39) as u16, ((frame * 3) % 15) as u16),
                    phase: termzzz::runtime::PointerPhase::Moved,
                    button: termzzz::runtime::PointerButton::Left,
                });
            }
            if frame % 11 == 0 {
                effect.handle_input(&termzzz::runtime::InputEvent::Key {
                    key: termzzz::runtime::Key::Char('r'),
                    phase: termzzz::runtime::KeyPhase::Pressed,
                });
            }

            let context = termzzz::runtime::FrameContext::new(
                size,
                frame,
                Duration::from_secs_f64(frame as f64 / 60.0),
                Duration::from_secs_f64(1.0 / 60.0),
                input.clone(),
            );

            for (x, y, cell) in effect.get_diff_with_context(&context) {
                assert!(
                    x < size.0 as usize && y < size.1 as usize,
                    "{id:?} emitted ({x}, {y}) outside bounds under input"
                );
                assert!(
                    !cell.symbol.is_control() || cell.symbol == '\n',
                    "{id:?} emitted a control character {:?}",
                    cell.symbol
                );
            }

            effect.update_with_context(&context);
        }
    }
}

/// A resize has to leave the effect renderable at its new size. The runtime
/// calls `update_size` and then `reset`, and a user can resize repeatedly, so
/// the previous-frame buffer has to be rebuilt at the new size or the reported
/// cell coordinates are computed against stale dimensions.
#[test]
fn every_effect_stays_in_bounds_after_resizing() {
    let mut offenders: Vec<String> = Vec::new();

    for id in EffectId::all() {
        let config = Config::default();
        let mut effect = AnyEffect::build(id, &config, (40, 16));
        let mut input = termzzz::runtime::InputState::default();

        // Grow and shrink, so both an undersized and an oversized previous
        // buffer get exercised.
        let sizes = [(20u16, 10u16), (80, 30), (6, 6), (61, 23), (40, 16)];

        for (cycle, &(width, height)) in sizes.iter().enumerate() {
            effect.update_size(width, height);
            effect.reset();
            input.set_size((width, height));

            let context = termzzz::runtime::FrameContext::new(
                (width, height),
                cycle as u64,
                Duration::from_secs_f64(cycle as f64 / 60.0),
                Duration::from_secs_f64(1.0 / 60.0),
                input.clone(),
            );

            let escaped = effect
                .get_diff_with_context(&context)
                .iter()
                .any(|(x, y, _)| *x >= width as usize || *y >= height as usize);

            if escaped {
                offenders.push(format!("{} at {width}x{height}", id.as_str()));
                break;
            }
            effect.update_with_context(&context);
        }
    }

    assert!(
        offenders.is_empty(),
        "these effects emitted cells outside the terminal they had just been \
         resized to: {offenders:?}. update_size and reset have to leave the \
         previous-frame buffer at the new size."
    );
}

// --- cell model -----------------------------------------------------------

#[test]
fn a_blank_cell_is_the_identity_for_diffing() {
    let mut buffer = termzzz::buffer::Buffer::new(4, 3);
    let blank = Cell::default();
    buffer.fill_with(&blank);

    assert!(
        buffer.diff(&buffer).is_empty(),
        "a buffer diffed against an identical copy must be empty"
    );
}

/// Every seeded effect must be reproducible, and a different seed must actually
/// change what it draws.
///
/// This is the contract that made the timebase check above able to see the
/// delta at all: it compares two independently built instances, which is only
/// meaningful if both are deterministic. It is also the user-facing promise —
/// `--seed` is only worth documenting if it holds.
#[test]
fn seeded_effects_are_reproducible_and_seed_sensitive() {
    let size = (60u16, 20u16);
    let frames = 45u64;

    // Drives an effect for a while and returns everything it drew, as a map
    // from coordinate to the last cell written there.
    //
    // Accumulating rather than returning the last frame matters. A diff only
    // reports what changed since the previous frame, and a slow effect covering
    // less than a cell per frame changes nothing on any given frame — so the
    // final diff of a perfectly good run is empty, and two different seeds look
    // identical because neither drew a cell. The accumulated footprint cannot
    // be empty by accident.
    let render = |id: EffectId, seed: u64| {
        let mut config = Config::default();
        config.override_seed(seed);

        let mut effect = AnyEffect::build(id, &config, size);
        let mut input = termzzz::runtime::InputState::default();
        input.set_size(size);

        let mut drawn: BTreeMap<(usize, usize), Cell> = BTreeMap::new();
        let mut record = |cells: Vec<(usize, usize, Cell)>| {
            for (x, y, cell) in cells {
                drawn.insert((x, y), cell);
            }
        };

        for frame in 0..frames {
            let delta = Duration::from_secs_f64(1.0 / 60.0);
            let step = termzzz::runtime::FrameContext::new(
                size,
                frame,
                delta * frame as u32,
                delta,
                input.clone(),
            );
            record(effect.get_diff_with_context(&step));
            effect.update_with_context(&step);
        }

        let view = termzzz::runtime::FrameContext::new(
            size,
            frames,
            Duration::from_secs_f64(1.0),
            Duration::from_secs_f64(1.0 / 60.0),
            input,
        );
        record(effect.get_diff_with_context(&view));
        drawn
    };

    // `blank` draws a constant field and `plasma`, `cube` and `donut` are pure
    // functions of time, so a seed cannot change them. Terrain renders once and
    // then never again, so its settled frame is empty either way. Those five
    // have no business being seed-sensitive; the rest do.
    const SEED_INSENSITIVE: &[&str] =
        &["blank", "cube", "donut", "plasma", "terrain"];

    let mut not_reproducible: Vec<&str> = Vec::new();
    let mut seed_ignored: Vec<&str> = Vec::new();

    for id in EffectId::all() {
        let name = id.as_str();

        // Sampled more than twice: one pair can coincide by chance, which would
        // make this check flaky in both directions.
        let baseline = render(id, 42);
        if !(0..3).all(|_| render(id, 42) == baseline) {
            not_reproducible.push(name);
            continue;
        }

        if SEED_INSENSITIVE.contains(&name) {
            continue;
        }

        // A different seed has to produce a different picture. An effect that
        // stores the seed and then ignores it — which terrain's noise generator
        // used to do — passes the reproducibility half of this and fails here.
        if render(id, 43) == baseline {
            seed_ignored.push(name);
        }
    }

    assert!(
        not_reproducible.is_empty(),
        "these effects render differently on two runs with the same seed, so they \\
         are drawing from an unseeded generator: {not_reproducible:?}. Store a \\
         StdRng built from the configured seed instead of calling rand::rng()."
    );

    assert!(
        seed_ignored.is_empty(),
        "these effects render identically for seed 42 and seed 43, so the seed \\
         reaches the struct but not the simulation: {seed_ignored:?}"
    );
}
