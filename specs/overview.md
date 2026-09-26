# Project Architecture Overview

## Project Structure

`termzzz` is a collection of terminal-based screensavers and generative visual effects written in Rust. Each effect is implemented as a separate module and exposed through the library and binary entrypoints.

## Core Architecture

### Main Modules

- `main.rs`: CLI parsing, effect selection, terminal setup, and cleanup
- `lib.rs`: Public library exports
- `common.rs`: Terminal effect interface, input routing, frame loop, and rendering
- `runtime.rs`: Backend-independent input state, frame context, and crossterm adapter
- `buffer.rs`: Terminal cell buffer and differential output
- `config.rs`: TOML configuration loading and runtime option construction
- `error.rs`: Error types for the crate
- `registry.rs`: Canonical effect IDs, metadata, and the `AnyEffect` enum used by the CLI
- `check.rs`: Bounded frame-count test mode
- `ascii/`: Reusable ASCII renderer and interactive generative field
- `dvd/`: Bouncing ASCII logo with a configurable logo
- `playlist/`: Timed effect queue with blank-wipe transitions and shuffle ordering

## Effect Modules

Each effect provides a `TerminalEffect` implementation. Existing effects can use the legacy `get_diff` and `update` methods, while interactive effects can use the context and input hooks added in `common.rs`.

Current effects:

- `matrix` - Matrix digital rain
- `life` - Conway's Game of Life
- `maze` - Maze generation
- `boids` - Boids flocking simulation
- `cube` - 3D cube rotation
- `crab` - ASCII crab walking animation
- `donut` - 3D donut rotation
- `pipes` - Pipe maze animation
- `plasma` - Plasma color wave effect
- `fire` - Fire simulation
- `terrain` - Terrain generation
- `constellation` - Drifting stars and dotted connections
- `dvd` - Bouncing ASCII logo
- `blank` - Blank screen
- `ascii` - Interactive generative ASCII field

Effects are registered once in `registry.rs`. `EffectId::ALL` is the single source of truth for effect names, descriptions, default playlist durations, and whether an effect needs mouse capture. The CLI, help output, check mode, and playlists all read from that registry.

## Runtime and Rendering

`common::run_loop` uses `runtime::InputSource` to collect normalized input, update `InputState`, create a `FrameContext`, and pass it to the effect. The effect returns changed cells through `Buffer`, and the loop writes only those cells to the terminal.

The ASCII field uses a seeded radial scalar field, a large narrow ASCII glyph ramp, a bounded glyph palette, and pointer energy injection that decays over roughly two seconds of elapsed time. The renderer has no media dependencies and only depends on the existing `Buffer`/`Cell` model. Effects are initialized with a safe minimum simulation size of 6x6 while output writes are clipped to the actual terminal dimensions.

## Speed Control

`RuntimeOptions` carries a global speed multiplier clamped to `MIN_SPEED..=MAX_SPEED`. At `1.0` each effect updates once per frame with its real frame delta. Below `1.0`, `TickClock` accumulates scaled time and only runs whole 1/60s simulation quanta, so effects slow down instead of jittering. The multiplier comes from `[global] speed` or `--speed`, and `+`/`-` adjust it live. Per-effect defaults are tuned for calmer motion.

## Playlist

`Playlist` owns an ordered list of `Slot` values built from config or `--playlist`, plus a `AnyEffect` for the active entry. It accumulates the active effect's diff into a full-screen buffer, then compares that against the previously shown frame. Between effects a diagonal blank wipe masks the frame in two phases (`WipeOut` then `WipeIn`), which keeps transitions independent of what each effect draws. Sequential mode steps through the list; shuffle mode draws from a reshuffling bag so every effect runs once per round.

## Build Configuration

- Rust edition 2024 with a pinned toolchain
- Optimized release profile for small binaries
- Link-time optimization and symbol stripping
- No external image or video dependencies

## Development Workflow

- **Testing**: `cargo test`
- **Benchmarks**: `cargo bench`
- **Formatting**: `cargo fmt --all -- --check`
- **Linting**: `cargo clippy --all-features --workspace -- -D warnings`
- **CI/CD**: GitHub Actions for testing, linting, and releases
