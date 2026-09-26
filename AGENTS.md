# Project Overview

## Quick Reference

`termzzz` is a collection of terminal-based screensavers and generative visual effects written in Rust.

### Main Commands
```bash
# Build
cargo build --release

# Run effects
termzzz matrix      # Matrix digital rain
termzzz life        # Conway's Game of Life
termzzz maze        # Maze generation
termzzz boids       # Boids flocking simulation
termzzz cube        # 3D cube rotation
termzzz crab        # ASCII crab animation
termzzz donut       # 3D donut rotation
termzzz pipes       # Pipe maze animation
termzzz plasma      # Plasma effect
termzzz fire        # Fire simulation
termzzz terrain     # Terrain generation
termzzz constellation # Drifting stars and dotted connections
termzzz dvd         # Bouncing ASCII logo
termzzz blank       # Blank screen
termzzz ascii       # Interactive generative ASCII field

# Run a timed playlist of effects
termzzz --playlist matrix,dvd,plasma
termzzz --shuffle

# Reproducible runs: every effect that uses randomness honours the seed
termzzz matrix --seed 1234

# Development
cargo test        # Run tests
cargo bench       # Run benchmarks (criterion)

# Frame costs per effect, with ANSI volume and a budget check
cargo run --release --bin frame_times

# Code Quality
cargo fmt --check  # Check formatting
cargo test --lib   # Run library tests
cargo clippy       # Run linter
```

### Installation
- **Local checkout**: `cargo install --path .`
- **Crates.io**: `cargo install termzzz` after publication is enabled
- **GitHub**: canonical repository and release links are pending selection

## Project Status
- **Version**: 0.2.0
- **Effects**: 15 screensavers and visual effects, plus a playlist mode
- **Platforms**: macOS and Linux
- **Configuration**: `~/.config/termzzz.toml`

## Current Focus
Ship the `termzzz` 0.2.0 release. Global speed control, the DVD logo, playlist
mode, and the effect registry are all done. What remains is the canonical GitHub
repository and the distribution metadata (crates.io publication, Homebrew, Nix).

Open engineering work, in priority order:

1. **The contract refactor** (`Canvas` plus `Box<dyn TerminalEffect>`), which
   deletes the 14 copies of the double-buffer pattern and the per-frame
   full-screen buffer allocation, and makes dispatch a vtable. It is also what
   makes swapping the running effect from a keypress possible.
2. **The sub-cell renderer.** Braille first, which needs no change to `Cell`
   because it is monochrome per cell; half-block afterwards, which needs `Cell`
   to grow a background colour.

Determinism is done: all nine effects that used an unseeded generator now carry a
`seed` option and a seeded `StdRng`, `--seed <N>` overrides all of them, and
`tests/effect_contracts.rs` asserts both reproducibility and seed sensitivity for
every effect. Because they became comparable, that suite also found that five of
them advanced by a fixed step per rendered frame — `maze`, `boids`, `crab`,
`pipes` and `constellation` ignored the speed keys — which is fixed too.

Not worth doing soon: further performance work. Nothing is dropping frames — the
worst effect uses about 2% of a 60 fps budget at 200x50. `life`, `plasma` and
`fire` exceed the 2 ms budget at 400x200, and did so before the determinism work
too.

## Working Practices

### Adding an effect

Adding an effect means one `EFFECT_SPECS` entry, one `EffectId` variant, one line
in the dispatch macro invocation, one arm each in `AnyEffect::build` and
`AnyEffect::id`, one `Config` field, and one accessor. All of those are caught by
the compiler or by a test, so an effect cannot be silently half-registered — which
is what used to happen, invisibly to `--help`, argument parsing, and playlists.

`tests/effect_contracts.rs` picks up a new effect automatically, because every
assertion in it iterates `EffectId::all()`. Do not add per-effect cases there.

### Git safety

**Never use `git checkout -- <file>` or `git restore <file>` to undo your own
edit.** The working tree is the only copy of uncommitted work, so a bulk revert
silently discards every earlier uncommitted change in those files. This has
already cost one session about a dozen fixes.

- To undo an edit, re-apply it with the edit tool.
- If a bulk revert really is the right call, `git stash` first.
- Before naming a file in a revert, check `git diff --stat <file>` to confirm it
  has no uncommitted work in it.

### Verifying changes

`cargo test`, `cargo fmt --all -- --check`, and
`cargo clippy --all-features --workspace --all-targets -- -D warnings` all have to
pass before a change is done. For anything touching an effect's simulation, also
run `cargo run --release --bin frame_times` and compare against the previous
table.

Prefer a test that fails without the fix over a test that only passes with it. The
bug found most cheaply in this project was a `Buffer::diff` coordinate bug, and the
reason it was found cheaply is that `tests/effect_contracts.rs` was written before
the fixes rather than after them.

## Architecture
See `specs/overview.md` for the project architecture and technical overview.
