//! Contract and drift tests for the effect catalogue.
//!
//! These exist so that adding an effect, or editing an existing one, cannot
//! silently break the registry, the config surface, or the runtime contract.
//! Every assertion here is deliberately generic: it derives its expectations
//! from `EffectId::all()` and `Config` rather than hard-coding per-effect
//! values, so a newly added effect is covered without editing this file.

use std::cell::Cell as StdCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use termzzz::buffer::Cell;
use termzzz::clock::Clock;
use termzzz::common::TerminalEffect;
use termzzz::config::Config;
use termzzz::registry::{AnyEffect, EffectId};

thread_local! {
    /// The synthetic "now" that [`clock_frame`] hands the clock effect.
    ///
    /// `Clock`'s time source is a bare `fn() -> SystemTime`, so it cannot
    /// capture the frame it is being drawn for. The frame index has to reach it
    /// out of band, and a thread-local is the seam that lets a `fn` pointer read
    /// state that changes per frame.
    static CLOCK_NOW: StdCell<Option<SystemTime>> = const { StdCell::new(None) };
}

/// The clock effect's time source, returning whatever `CLOCK_NOW` holds.
///
/// Falling back to the epoch rather than to `SystemTime::now()` is deliberate:
/// a real clock would make this suite's result depend on how fast the machine
/// running it is, which is the exact failure this exists to prevent.
fn clock_frame() -> SystemTime {
    CLOCK_NOW.with(|now| now.get().unwrap_or(UNIX_EPOCH))
}

/// Sets the time `clock_frame` returns for the next draw.
///
/// A frame at 60 Hz, so this matches the `delta` the caller passes to
/// `FrameContext` and the clock advances at the rate it will actually be run
/// at. Driving it with the wall clock instead measures the *host* rather than
/// the effect: this suite takes 51-65s in a debug build and 10s under `nix
/// build`, which is release, and against `STILL_RUN_FRAMES` of 30 that is the
/// difference between passing and failing for reasons the effect cannot control.
fn set_clock_frame(frame: u64) {
    CLOCK_NOW.with(|now| {
        now.set(Some(
            UNIX_EPOCH + Duration::from_secs_f64(frame as f64 / 60.0),
        ))
    });
}

/// Builds `id`, giving the clock effect an injected time source.
///
/// Every other effect goes through the registry unchanged; the match is on the
/// variant rather than on the name so a rename cannot quietly stop applying it.
fn build_with_injected_time(
    id: EffectId,
    config: &Config,
    size: (u16, u16),
) -> AnyEffect {
    match id {
        EffectId::Clock => AnyEffect::Clock(Clock::with_clock(
            config.get_clock_options(),
            size,
            clock_frame,
        )),
        _ => AnyEffect::build(id, config, size),
    }
}

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

/// The effect is called `ink`, and the old name is gone with no alias.
///
/// `ascii` named the medium rather than the effect -- eleven other effects draw
/// characters -- and the thing you actually interact with is ink poured into a
/// field. It was renamed deliberately, and deliberately *without* a deprecated
/// alias, so these are the consequences rather than accidents:
///
/// - `termzzz ascii` is now an unknown effect, and the CLI says so.
/// - an old `[ascii]` config section is silently ignored, because serde has no
///   `deny_unknown_fields` and inventing a hard failure there would break anyone
///   carrying a stale key from any earlier rename.
/// - an old `effect = "ascii"` playlist entry no longer resolves, which is the
///   one that loses data, so `Playlist::build_slots` reports it rather than
///   shortening the playlist in silence.
///
/// That last one is not hypothetical: the crate's own playlist test named
/// `"ascii"` and started failing the moment the rename landed, which is how the
/// reporting was found to be worth having.
#[test]
fn the_field_is_called_ink_and_ascii_is_gone() {
    assert!("ink".parse::<EffectId>().is_ok());
    assert_eq!("ink".parse::<EffectId>().unwrap(), EffectId::Ink);

    // No alias. If this ever starts passing, someone has added a compatibility
    // shim that was not asked for and that hides the rename from users.
    assert!(
        "ascii".parse::<EffectId>().is_err(),
        "'ascii' parses again, so a deprecated alias has been added"
    );

    // The config section followed the name, as `registry.rs` requires of every
    // effect -- so `[ink]` is the section and `[ascii]` is inert.
    let config = Config::default();
    assert_eq!(config.get_ink_options().seed, 42);
    let renamed: Config =
        toml::from_str("[ink]\ntime_scale = 2.0\n").expect("[ink] parses");
    assert_eq!(renamed.ink.time_scale, 2.0);
    let stale: Config = toml::from_str("[ascii]\ntime_scale = 2.0\n")
        .expect("a stale section is ignored");
    assert_eq!(
        stale.ink.time_scale, 1.0,
        "a stale [ascii] section was honoured, so the rename did not take"
    );

    // Mouse capture is enabled for the whole session because at least one effect
    // wants it. There used to be exactly one -- the reason the sentence above
    // could be "the only effect" -- and now there are two, `boids` having grown a
    // click-to-scatter force. What matters is not the number but that the
    // decision is being read from the table rather than hard-coded, because
    // `main.rs` enables capture per *playlist* and so a playlist naming only
    // `boids` would get no capture at all if this one were left at `false`.
    assert!(EffectId::Ink.needs_mouse());
    assert!(EffectId::Boids.needs_mouse());
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

/// A section that names only *some* of its keys must still load.
///
/// This is the other half of `omitting_any_single_config_section_preserves_real_
/// defaults`, and it is the one that bites when a knob is added. `Config` carries
/// a container-level `#[serde(default)]`, which covers a missing section but not
/// a missing key inside a section that is present -- so without the same
/// attribute on each effect's own options struct, a hand-written
///
/// ```toml
/// [plasma]
/// time_scale = 2.0
/// ```
///
/// is a startup deserialisation error rather than "the other plasma settings at
/// their defaults".
///
/// It matters more than it looks, because `--print-config` writes every key to
/// disk. A generated config is therefore pinned to whatever the defaults were the
/// day it was generated, which means every future knob arrives as "a key the
/// user's file does not have" -- this is the normal case, not the exotic one.
#[test]
fn omitting_any_single_key_keeps_its_real_default() {
    let full = default_config_toml();
    let sections = split_toml_sections(&full);

    let mut broken: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (section, body) in &sections {
        let lines: Vec<&str> = body.lines().collect();

        // A key is `name = ...` at bracket depth zero. Depth matters because
        // `luminance_chars` serialises as a multi-line array, and without this
        // each of its element lines reads as a key of its own.
        let mut entries: Vec<(&str, usize, usize)> = Vec::new();
        let mut depth: i32 = 0;
        let mut start: Option<usize> = None;
        // TOML multi-line strings span lines, and a value inside one is not a key.
        // `dvd.logo` is a 60-by-28 dot bitmap, so the serialised config contains
        // one, and without this the scanner treats each row of the letter as a key
        // of its own -- which it did, until this test caught it.
        let mut in_multiline = false;
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if in_multiline {
                if trimmed.contains("\"\"\"") {
                    in_multiline = false;
                }
                continue;
            }
            if depth == 0
                && let Some((name, _)) = trimmed.split_once('=')
                && !name.trim().is_empty()
                && name.trim().chars().all(|c| c.is_alphanumeric() || c == '_')
            {
                start = Some(index);
            }
            if trimmed.contains("\"\"\"") || trimmed.contains("'''") {
                // A multi-line string opened and closed on one line is a single
                // line entry; one that opened here continues, and `start` stays put
                // so the entry ends at its closing delimiter.
                if trimmed.matches("\"\"\"").count() % 2 == 1
                    || trimmed.matches("'''").count() % 2 == 1
                {
                    in_multiline = true;
                }
            } else {
                depth += trimmed.matches(['[', '{']).count() as i32
                    - trimmed.matches([']', '}']).count() as i32;
            }
            if depth == 0
                && !in_multiline
                && let Some(begin) = start.take()
            {
                entries.push((
                    trimmed.split('=').next().unwrap_or("").trim(),
                    begin,
                    index,
                ));
            }
        }

        for (key, begin, end) in entries {
            checked += 1;

            let mut partial = String::new();
            for (name, other) in &sections {
                partial.push_str(&format!("[{name}]\n"));
                if name == section {
                    for (index, line) in other.lines().enumerate() {
                        if index < begin || index > end {
                            partial.push_str(line);
                            partial.push('\n');
                        }
                    }
                } else {
                    partial.push_str(other);
                }
            }

            // A missing key must fall back to the real default, which means the
            // round trip has to come back out identical to the full config.
            let parsed: Config = match toml::from_str(&partial) {
                Ok(parsed) => parsed,
                Err(error) => {
                    broken
                        .push(format!("{section}.{key} failed to parse: {error}"));
                    continue;
                }
            };
            let reserialized =
                toml::to_string_pretty(&parsed).expect("re-serializes");
            if reserialized != full {
                broken.push(format!(
                    "{section}.{key} did not fall back to its default \
                     (the file is missing that key's real default)"
                ));
            }
        }
    }

    assert!(
        checked > 40,
        "only {checked} keys were exercised, so this test is not covering the config"
    );
    assert!(
        broken.is_empty(),
        "these keys cannot be left out of a user's config file, so adding any one \
         of them as a new setting would break startup for anyone with a \
         hand-edited config:\n{}",
        broken.join("\n")
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

/// Every effect that takes a palette actually uses the name it is given.
///
/// The chain shuffle mode depends on is three links -- the name resolves against
/// a table, `Config::set_palette` stores it, and the effect reads it -- and the
/// first two are cheap to get right and the third is the one that matters. Each
/// of these effects falls back to its default on a name it does not recognise,
/// so a palette that was accepted, stored, and then ignored looks exactly like a
/// shuffle that quietly did nothing.
///
/// Compared on the *rendered frame* rather than on any internal field, because
/// the internal field is the thing under suspicion.
#[test]
fn a_palette_name_reaches_the_pixels() {
    let size = (60u16, 20u16);
    let frames = 24u64;

    let render = |id: EffectId, palette: &str| {
        let mut config = Config::default();
        config.set_palette(id, palette);

        let mut effect = AnyEffect::build(id, &config, size);
        let mut input = termzzz::runtime::InputState::default();
        input.set_size(size);

        // Accumulated rather than the last diff, for the reason the seed test
        // gives: a slow effect changes few cells on any one frame, and a frame
        // that changes nothing would make two palettes look identical.
        let mut drawn: BTreeMap<(usize, usize), Cell> = BTreeMap::new();
        for frame in 0..frames {
            let step = termzzz::runtime::FrameContext::new(
                size,
                frame,
                Duration::from_secs_f64(frame as f64 / 60.0),
                Duration::from_secs_f64(1.0 / 60.0),
                input.clone(),
            );
            for (x, y, cell) in effect.get_diff_with_context(&step) {
                drawn.insert((x, y), cell);
            }
            effect.update_with_context(&step);
        }
        drawn
    };

    for id in EffectId::all() {
        let pool = id.spec().shuffle_palettes;
        if pool.len() < 2 {
            continue;
        }
        let first = render(id, pool[0]);
        let second = render(id, pool[1]);
        assert_ne!(
            first,
            second,
            "{} renders identically for {} and {}, so the palette name is not \
             reaching the simulation or the render",
            id.as_str(),
            pool[0],
            pool[1]
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

// --- effects that read the wall clock -------------------------------------
//
// Two of the tests below cannot be run against `clock`, and neither of them can
// be made to work by giving the effect a seed. `clock` displays the current time
// and has no randomness at all, so:
//
//   - **Reproducibility is undefined, not unimplemented.** Two runs a
//     microsecond apart legitimately differ. A seed cannot fix that, because
//     there is nothing random to pin; the only way to make two runs match would
//     be to make the clock lie.
//   - **Advancing on the frame delta is a bug.** `effects_advance_using_the_frame
//     _delta` exists to catch an effect whose speed depends on the terminal's
//     refresh rate. A clock derives its display from the wall clock precisely so
//     that it does not.
//
// It is a one-entry list and the entry is checked by name, so an effect cannot
// quietly acquire an exemption: `every_effect_id_has_a_spec_entry` and the
// tripwire in `src/host.rs` both fail the moment the catalogue changes size, and
// a name on this list that matches no effect is a dead entry that would be
// invisible until somebody needed it.
//
// **Neither of these is a coverage gap being papered over.** There is no version
// of a clock that satisfies either test, so the honest response is to say which
// effect is exempt and why rather than to weaken the assertion for everything.
const WALL_CLOCK: &[&str] = &["clock"];

// --- the screensaver property ---------------------------------------------
//
// Everything else in this file checks that an effect is *well formed*: it stays
// in bounds, it honours its seed, it moves when the speed key changes. Nothing
// here checks that it is *worth watching*, and that is the property this crate
// has actually been getting wrong. Two effects in its history were measurably
// efficient and read as nothing -- `terrain`'s grain round, where a byte count
// said the effect was getting busier and the picture was getting worse, and a
// `plasma` change attributed to a glyph recalibration before the pre-change
// binary had been measured. A frame table cannot tell you whether an effect
// reads.
//
// The one mechanical proxy for "reads" is convergence. A screensaver that
// settles into a fixed point and then shows a still image for the rest of its
// playlist slot is broken, however fast it renders and however few bytes it
// emits -- a still image is zero bytes and zero interest. So this asserts that
// no effect's diff goes permanently quiet.

/// The longest run of consecutive frames an effect may show nothing new.
///
/// 30 frames, which is half a second at 60 fps. The number is a granularity,
/// not a tuned threshold: the assertion is that the picture is never still for
/// longer than this, so any value works and the point is to keep it short
/// enough to catch a stall and long enough that a legitimately slow effect
/// survives it.
///
/// Measured against all twenty effects at 200x50, this separates cleanly. The
/// stillest genuinely-alive effect is `dvd`, which emitted a change on 73 of
/// 180 window frames (41%) because the logo is small and does not cross a cell
/// boundary on every frame -- and even so, the chance of a 30-frame run landing
/// entirely inside one of its rest gaps is under 1e-6. The two effects that do
/// fail are listed in [`DELIBERATELY_CONVERGENT`].
///
/// A percentage of "frames that changed something" was the obvious formulation
/// and it needed a magic number in a gap between two measured populations
/// (5% for `life`, 41% for `dvd`), which is the kind of constant this repo has
/// been bitten by before. This needs no threshold at all.
const STILL_RUN_FRAMES: usize = 30;

/// Frames of settling before the measurement window opens.
///
/// 300, or five seconds. Generous on purpose: an effect that converges needs
/// time to get there, and a measurement that starts too early will call a slow
/// transition still. This is a *floor* on how patient the test is, and the
/// effects that pass immediately are the ones that are never close.
const SETTLE_FRAMES: u64 = 300;

/// The measurement window, in frames. Three seconds.
const WINDOW_FRAMES: u64 = 180;

/// Effects that are *supposed* to stop changing, with the reason each one does.
///
/// This is a short list and every entry is a property of the model or a
/// deliberate feature rather than a defect in the effect, which is the test for
/// whether an entry belongs here. An entry that is "it converges, we did not fix
/// it" is a bug wearing an exemption.
///
/// - `blank` is a blank screen. It is in the catalogue as a deliberate nothing,
///   the thing a playlist uses as a rest between effects.
///
/// - `physarum` settles on purpose. The plasmodium model anneals its decay from
///   0.90 to 0.995 over 2,500 steps, holds the finished network for
///   `hold_seconds` -- 4 seconds, or 240 frames against `STILL_RUN_FRAMES`'s 30 --
///   and then fades and re-seeds. At the shipped 0.90 it never converged at all:
///   churn sat at 0.35 to 1.4, meaning a third of the marked field was being
///   redrawn twice a second for ever.
///
///   **It is on this list because the window is too short to catch it otherwise,
///   which is worth being explicit about.** The measurement here runs
///   `SETTLE_FRAMES + WINDOW_FRAMES` = 480 frames, about eight seconds, and
///   physarum does not reach its hold until roughly 45. Left off this list it
///   would pass by never getting there, which is the failure mode this suite
///   exists to prevent and not a smaller version of it. The exemption is the
///   honest response to a *deliberate* hold; a hold that arrived by accident would
///   need the hold removed instead.
///
/// - `life` is Conway's Life, and Conway's Life converges. A random soup on a
///   finite bounded board settles into still lifes and blinkers, after which
///   nothing moves -- measured here at 9 changes in 180 frames, against `dvd`'s
///   73. That is the model behaving correctly, not an effect failing. It becomes
///   only truer with the `rule` option, where `seeds` dies out entirely and
///   `2x2` saturates the whole board.
///
/// Notably absent: `maze`, which is on a four-second playlist duration and reads
/// like an effect that holds. It does not. It emits wall texture indefinitely,
/// about six cells a frame, and its short duration is a pacing decision about
/// playlists rather than a claim about its own output.
const DELIBERATELY_CONVERGENT: &[&str] = &["blank", "physarum", "life"];

/// No effect may hold a still picture for longer than [`STILL_RUN_FRAMES`].
///
/// **What this cannot see.** It is a proxy for convergence, and it is a coarse
/// one. An effect that changes a single cell every half second passes it while
/// being just as static in the way that matters; so does an effect that cycles
/// between two states in place. Neither is caught here, and both would need a
/// hand-written assertion about that effect's own dynamics to be caught at all.
///
/// It also says nothing about whether an effect looks good, which is the failure
/// the two rounds of `terrain` grain actually were. This catches the case where
/// an effect stops entirely, which is the mechanical way the other one starts.
#[test]
fn no_effect_settles_into_a_still_picture() {
    let size = (200u16, 50u16);
    let total = SETTLE_FRAMES + WINDOW_FRAMES;
    let mut offenders: Vec<String> = Vec::new();

    for id in EffectId::all() {
        let name = id.as_str();
        if DELIBERATELY_CONVERGENT.contains(&name) {
            continue;
        }

        let config = Config::default();
        let mut effect = build_with_injected_time(id, &config, size);

        let mut input = termzzz::runtime::InputState::default();
        input.set_size(size);

        let mut run = 0usize;
        let mut longest = 0usize;

        for frame in 0..total {
            set_clock_frame(frame);
            let context = termzzz::runtime::FrameContext::new(
                size,
                frame,
                Duration::from_secs_f64(frame as f64 / 60.0),
                Duration::from_secs_f64(1.0 / 60.0),
                input.clone(),
            );

            let changed = effect.get_diff_with_context(&context).len();

            if frame < SETTLE_FRAMES {
                // Not measured. The settling window is allowed to be still,
                // which is the whole reason it exists.
            } else if changed == 0 {
                run += 1;
                longest = longest.max(run);
                if run > STILL_RUN_FRAMES {
                    offenders.push(format!(
                        "{name} showed nothing new for {run} frames \
                         (from frame {frame} of {total})"
                    ));
                    break;
                }
            } else {
                run = 0;
            }

            effect.update_with_context(&context);
        }
    }

    assert!(
        offenders.is_empty(),
        "these effects settled into a still picture, which means a playlist \
         slot would show a frozen frame for its whole duration:\n  {}",
        offenders.join("\n  ")
    );
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

    // Returns the final frame *and* the set of every cell the run touched.
    //
    // Two comparators, because neither alone works. The final frame is what
    // distinguishes a full-repaint effect (`terrain`, `plasma`, `mandelbrot`),
    // which repaints every cell every frame and so has a non-empty last diff
    // whose *content* depends on how far it got. The set of touched cells is
    // what distinguishes a sparse one (`crab`, `matrix`, `boids`), where the
    // last frame is frequently empty no matter what -- which is exactly the trap
    // this ran into: at three cells a second a crab moves 0.05 of a cell per
    // frame at 60 Hz, so its final diff was empty at both rates and comparing
    // two empty vectors reported a delta-driven effect as ignoring its delta.
    //
    // A delta-*ignoring* effect produces the same final frame and touches the
    // same cells at both rates, so it fails both and is still caught.
    let run = |id: EffectId, delta: Duration| {
        let mut effect = AnyEffect::build(id, &config, size);
        let mut input = termzzz::runtime::InputState::default();
        input.set_size(size);
        let mut touched: BTreeSet<(usize, usize)> = BTreeSet::new();

        for frame in 0..FRAMES {
            let step = termzzz::runtime::FrameContext::new(
                size,
                frame,
                delta * frame as u32,
                delta,
                input.clone(),
            );
            for (x, y, _) in effect.get_diff_with_context(&step) {
                touched.insert((x, y));
            }
            effect.update_with_context(&step);
        }

        let view = termzzz::runtime::FrameContext::new(
            size,
            FRAMES,
            delta * FRAMES as u32,
            delta,
            input,
        );
        (effect.get_diff_with_context(&view), touched)
    };

    let mut ignore_delta: Vec<&str> = Vec::new();
    let mut nondeterministic: Vec<&str> = Vec::new();

    for id in EffectId::all() {
        if WALL_CLOCK.contains(&id.as_str()) {
            continue;
        }

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

        // Delta-driven if *either* comparator sees a difference. See the comment
        // on `run`: a full-repaint effect is caught by its last frame and a
        // sparse one by the ground it covered, and requiring both would fail
        // each kind on the other's blind spot.
        let other = run(id, faster);
        if baseline.0 == other.0 && baseline.1 == other.1 {
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

/// No effect panics at any terminal size, however narrow.
///
/// A terminal can report a width of 4 — a vertical split does it — and
/// `update_size` clamps to 1 rather than to anything a size-dependent guard
/// could lean on. `life` seeded gliders with `random_range(2..width - 3 + 1)`,
/// which is empty at exactly 4, and `rand` asserts on an empty range; the guard
/// in front of it tested `width <= 3`, so it admitted the one width that broke.
///
/// Two details make this able to see that, and both were the reason it hid:
///
/// - **Narrow sizes.** The resize test above stops at 6, which is one above the
///   interesting one.
/// - **Enough frames to reach a step.** `life` advances a generation through an
///   accumulator, and `generations_per_second` defaults to 3.0, so the handful
///   of updates elsewhere in this file never arrive at the glider code at all.
///   A defect behind a slow accumulator hides behind the accumulator, so this
///   drives a full second of frames at 60 Hz.
///
/// Panicking is a contract violation rather than a returned error, so this is a
/// `catch_unwind` sweep: the assertion is about which effect blew up and at what
/// size, and the panic hook is silenced so the report is the message below.
#[test]
fn no_effect_panics_at_any_terminal_size() {
    let mut offenders: Vec<String> = Vec::new();

    // Widths 1..=12 covers every size-dependent guard's off-by-one; the second
    // pair is tall-and-thin, which is the other axis of the same arithmetic.
    let sizes: Vec<(u16, u16)> = (1..=12)
        .flat_map(|w| [(w, 24u16), (w, 4), (24, w)])
        .collect();

    for id in EffectId::all() {
        let config = Config::default();
        for &(width, height) in &sizes {
            let mut effect = AnyEffect::build(id, &config, (width, height));
            let mut input = termzzz::runtime::InputState::default();
            input.set_size((width, height));
            let delta = Duration::from_secs_f64(1.0 / 60.0);

            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for frame in 0..70 {
                        let context = termzzz::runtime::FrameContext::new(
                            (width, height),
                            frame,
                            delta,
                            delta,
                            input.clone(),
                        );
                        let _ = effect.get_diff_with_context(&context);
                        effect.update_with_context(&context);
                    }
                }));

            if outcome.is_err() {
                offenders.push(format!("{} at {width}x{height}", id.as_str()));
                break;
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these effects panicked at a terminal size a resize can produce: \
         {offenders:?}. A guard in front of a size-dependent range has to \
         exclude every width the range is empty at, not the width the range \
         starts being small."
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

    // `blank` draws a constant field, and that is the whole list.
    //
    // It used to be five entries and every one of the other four was wrong, in
    // three different ways. `plasma`, `cube` and `donut` were "pure functions of
    // time" -- true when they had no seed at all, and all three grew one in this
    // round. `terrain` was "renders once and then never again", which stopped
    // being true three rounds ago when the height field started scrolling.
    //
    // The reason this was dangerous rather than merely stale: the list
    // `continue`s *after* the reproducibility half, so all four still passed
    // their reproducibility check while silently skipping the seed-sensitivity
    // one. Three separate agents flagged it as out of their reach.
    const SEED_INSENSITIVE: &[&str] = &["blank"];

    let mut not_reproducible: Vec<&str> = Vec::new();
    let mut seed_ignored: Vec<&str> = Vec::new();

    for id in EffectId::all() {
        let name = id.as_str();

        if WALL_CLOCK.contains(&name) {
            continue;
        }

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

/// The global terminal colours are the one place a user's terminal, rather than
/// an effect's canvas, is mutated. That makes them worth pinning from both
/// ends: the default must not touch the terminal at all, and a value that does
/// not parse must be refused at load rather than at draw.
#[test]
fn the_terminal_colours_default_to_leaving_the_terminal_alone() {
    let config = Config::default();
    assert_eq!(config.global.background, crossterm::style::Color::Reset);
    assert_eq!(config.global.foreground, crossterm::style::Color::Reset);
    assert!(
        crossterm::style::Color::Reset != crossterm::style::Color::Black,
        "if Reset ever equals Black this test is asserting nothing, and the \
         session would repaint every user's terminal on every start"
    );
}

#[test]
fn terminal_colours_parse_from_names_and_hex() {
    // `r##` rather than `r#`, because the hex literal below starts with `"#`,
    // which is exactly the sequence that closes a one-hash raw string.
    let parsed: Config = toml::from_str(
        r##"
[global]
background = "#0c0c14"
foreground = "dark_grey"
"##,
    )
    .expect("a hex triple and a colour name both parse");

    assert_eq!(
        parsed.global.background,
        crossterm::style::Color::Rgb {
            r: 0x0c,
            g: 0x0c,
            b: 0x14
        }
    );
    assert_eq!(parsed.global.foreground, crossterm::style::Color::DarkGrey);
}

/// `--print-config` writes every default to disk, so a user who has ever run it
/// has pinned whatever the defaults were on that day. A value that serialises
/// to a spelling the parser then rejects would make that generated config
/// unloadable, which is the one failure mode `--print-config` must not have.
#[test]
fn a_generated_config_reloads_unchanged() {
    let mut configured = Config::default();
    configured.global.background = crossterm::style::Color::Rgb {
        r: 0x0c,
        g: 0x0c,
        b: 0x14,
    };
    configured.global.foreground = crossterm::style::Color::DarkGrey;

    let printed = toml::to_string_pretty(&configured).expect("serializes");
    let reloaded: Config =
        toml::from_str(&printed).expect("what --print-config writes must parse");
    assert_eq!(reloaded.global.background, configured.global.background);
    assert_eq!(reloaded.global.foreground, configured.global.foreground);
}

/// A typo has to be an error naming the value, not a silent fallback.
///
/// The failure this guards is quiet and total: a config saying
/// `background = "blak"` that fell back to `Reset` would look like the setting
/// simply not working, with nothing anywhere pointing at the misspelling.
#[test]
fn an_unparseable_terminal_colour_is_refused_with_the_value_in_the_message() {
    let error = toml::from_str::<Config>(
        r#"
[global]
background = "blak"
"#,
    )
    .expect_err("a misspelled colour name must not parse");

    let message = error.to_string();
    assert!(
        message.contains("blak"),
        "the error should quote the rejected value so the user can find it, \
         got: {message}"
    );
}
