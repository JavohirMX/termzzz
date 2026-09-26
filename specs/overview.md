# Project Architecture Overview

## Project Structure

`termzzz` is a collection of terminal-based screensavers and generative visual effects written in Rust. Each effect is implemented as a separate module and exposed through the library and binary entrypoints.

## Core Architecture

### Main Modules

- `main.rs`: CLI parsing, effect selection, terminal setup, and cleanup
- `lib.rs`: Public library exports
- `common.rs`: Terminal effect interface, input routing, frame loop, and output encoding
- `runtime.rs`: Backend-independent input state, frame context, and crossterm adapter
- `buffer.rs`: Terminal cell buffer and differential output
- `config.rs`: TOML configuration loading and runtime option construction
- `error.rs`: Error types for the crate
- `registry.rs`: The `EFFECT_SPECS` table, effect ids, and the `AnyEffect` enum used by the CLI
- `session.rs`: Terminal setup and teardown, including panic safety
- `check.rs`: Bounded frame-count test mode
- `ascii/`: Reusable ASCII renderer and interactive generative field
- `dvd/`: Bouncing ASCII logo with a configurable logo
- `playlist/`: Timed effect queue with blank-wipe transitions and shuffle ordering

## Effect Modules

Each effect provides a `TerminalEffect` implementation. The trait has two update
paths: a bare `update()` that takes no timing information, and
`update_with_context()`, whose default implementation just calls `update()`. An
effect that does not override the latter advances by an amount unrelated to
elapsed time, so it runs at a different speed on a 30 Hz terminal than on a
144 Hz one, and the `+`/`-` speed keys stop meaning what the help text says.
Effects that keep an internal fixed-step accumulator, such as Conway's Life with
its generations-per-second, are fine: they consume `context.delta` and pick their
own quantum. `tests/effect_contracts.rs` enforces which is which.

For the current effect list, names, descriptions, default playlist durations, and
mouse requirements, see `registry::EFFECT_SPECS` — one table, which the CLI,
`--help`, check mode, and playlists all read. It is the single source of truth;
the list is deliberately not duplicated here.

Adding an effect means one `EFFECT_SPECS` entry, one enum variant, one line in the
dispatch macro invocation, one arm each in `AnyEffect::build` and `AnyEffect::id`,
one `Config` field, and one accessor. Every one of those is caught by the compiler
or by a test, so a new effect cannot be silently half-registered.

## Runtime and Rendering

`common::run_loop` uses `runtime::InputSource` to collect normalized input, update
`InputState`, create a `FrameContext`, and pass it to the effect. The effect
returns changed cells, and `common::write_cells` encodes them.

Output encoding is the one place that talks to the terminal, and it is deliberately
terse. A cursor move is emitted only when a cell is not the one immediately after
the previous one, so a run along a row costs one move rather than one per cell. A
styled glyph is emitted only when the colour or attributes actually change. Without
this, repainting a full screen costs a cursor move and a colour change per cell,
which measured at over a megabyte of escape sequences per frame for the plasma
effect on a 400x200 terminal. The change cut ANSI volume per frame by 18% to 62%
depending on the effect.

`Buffer::diff` derives cell coordinates from the frame being produced, not from the
previous frame. Deriving them from the previous one meant that a buffer left over
from before a resize reported coordinates computed against the old dimensions,
sending cells off the edge of the screen.

Effects are initialized with a safe minimum simulation size of 6x6
(`common::MIN_EFFECT_SIZE`), which the runtime enforces before handing a size to
an effect. Output writes are clipped to the actual terminal dimensions regardless.

The ASCII field is a domain-warped wave field: the sample coordinates are
displaced by a sine and a cosine before three wave terms and a diagonal ripple are
summed. That value indexes a narrow ASCII glyph ramp through a bounded palette, and
the pointer injects energy that decays over time. The renderer has no media
dependencies and only depends on the existing `Buffer`/`Cell` model.

## Terminal Safety

`session::TerminalSession` owns raw mode, the alternate screen, cursor visibility
and mouse capture, and restores all of it on drop. The release profile sets
`panic = "abort"`, which means `Drop` does not run when a panic unwinds the stack,
so `session::install_panic_hook` performs the same restore from inside the panic
hook before the process aborts. Without it, any panic left the terminal in raw
mode on the alternate screen, where Ctrl-C is not a signal and there is no shell
prompt to return to.

## Speed Control

`RuntimeOptions` carries a global speed multiplier clamped to `MIN_SPEED..=MAX_SPEED`. At `1.0` each effect updates once per frame with its real frame delta. Below `1.0`, `TickClock` accumulates scaled time and only runs whole 1/60s simulation quanta, so effects slow down instead of jittering. The multiplier comes from `[global] speed` or `--speed`, and `+`/`-` adjust it live. Per-effect defaults are tuned for calmer motion.

## Playlist

`Playlist` owns an ordered list of `Slot` values built from config or `--playlist`, plus a `AnyEffect` for the active entry. It accumulates the active effect's diff into a full-screen buffer, then compares that against the previously shown frame. Between effects a diagonal blank wipe masks the frame in two phases (`WipeOut` then `WipeIn`), which keeps transitions independent of what each effect draws. Sequential mode steps through the list; shuffle mode draws from a reshuffling bag so every effect runs once per round.

## Build Configuration

- Rust edition 2024 with a pinned toolchain
- Release profile optimized for **speed**, not size. This is a real-time renderer
  doing per-cell trigonometry sixty times a second, and `opt-level = "s"`
  suppresses the inlining that work depends on. Measured: 23% more binary size
  buys 28% to 98% on the heavier effects. Do not "optimize" this back to size
  without re-measuring.
- Link-time optimization, one codegen unit, and symbol stripping
- `panic = "abort"`, paired with the panic hook described above
- No external image or video dependencies

## Testing

`cargo test` runs unit tests, the `runtime_and_ascii` integration tests, and
`effect_contracts`.

`tests/effect_contracts.rs` is the safety net for the effect catalogue as a whole.
Every assertion in it derives its expectations from `EffectId::all()` and
`Config`, so a newly added effect is covered without editing the file. It covers:

- registry completeness, unique names, and usable metadata
- config stability across a TOML round trip, and that omitting any single section
  leaves every other section at its real defaults rather than zeroed
- every effect surviving a long run, the smallest supported terminal, and extreme
  aspect ratios, with all emitted cells in bounds
- every effect staying in bounds after a resize, with and without a `reset`
- every effect advancing by `context.delta` rather than a fixed step per call
- no effect repainting more cells in one frame than the screen has

That last group exists because six real bugs were found by writing these tests
rather than by reading the code, including a panic on every terminal resize and a
config trap that silently zeroed thirteen of fifteen sections.

## Development Workflow

- **Testing**: `cargo test`
- **Frame costs**: `cargo run --release --bin frame_times` — prints update,
  worst-case update at high `--speed`, render, output-encoding cost, and ANSI
  volume per frame, at three terminal sizes. Exits non-zero if an effect exceeds
  its frame budget, so it works as a CI gate. `--size WxH` narrows it to one size.
- **Benchmarks**: `cargo bench` (criterion, for tracking change over time; use
  `frame_times` for one-off questions, since its numbers are far easier to read)
- **Formatting**: `cargo fmt --all -- --check`
- **Linting**: `cargo clippy --all-features --workspace --all-targets -- -D warnings`
- **CI/CD**: GitHub Actions for testing, linting, and releases

The frame-cost table has a noise floor of a few times between runs on a loaded
machine, so treat a single-digit percentage difference as meaningless and re-run
before concluding anything.
