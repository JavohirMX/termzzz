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
termzzz mandelbrot # Escape-time Mandelbrot set, zooming
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
termzzz ink         # Interactive generative field you pour ink into

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
- **Effects**: 16 screensavers and visual effects, plus a playlist mode
- **Platforms**: macOS and Linux
- **Configuration**: `~/.config/termzzz.toml`

## Current Focus
Ship the `termzzz` 0.2.0 release. Global speed control, the DVD logo, playlist
mode, and the effect registry are all done. What remains is the canonical GitHub
repository and the distribution metadata (crates.io publication, Homebrew, Nix).

Open engineering work, in priority order:

1. **`mandelbrot` render cost at large sizes.** 11.8 ms per frame at 400x200,
   which is over half a 60 Hz budget spent before a single byte is written, and
   it is the only effect still over `frame_times`'s 2 ms budget at that size
   (`life` is also over, but only on its `update x4` worst case at high speed).
   The cost is very close to linear in `max_iterations`, because nearly every
   interior pixel spends the whole budget discovering it never escapes. Lowering
   the default is the first thing to try; a region-marking rewrite is where the
   real win is, and that is a rewrite rather than a tweak
2. **More effects on the sub-cell renderer.** `starfield` and `flow` want
   braille (line art, one colour per cell); `physarum` and a Gray-Scott
   reaction-diffusion want half-block (smooth colour). Each is roughly 150-350
   lines now that the renderer exists. Note that `cell.bg` only reached the
   terminal very recently, so half-block work is the first thing to have actually
   exercised that path

The output path was rebuilt recently and is worth knowing before touching it. It
does not use crossterm's `PrintStyledContent`, because that emits the attributes
between the colour and the glyph and then follows every styled glyph with a full
reset — so a style cache is wrong from the second cell of any run, and
`Attribute::Reset` erases the colour outright. The encoder is hand-built from
crossterm's individual commands, `Cell::attr` is read as additive-only, and
`common.rs` carries a terminal model that the output is replayed through. Seven
tests there fail against the old encoder. If you add a test for this layer,
check it fails against the old code too.

`EffectHost` and `Playlist` both accumulate a diff into their own frame buffer
and wipe a copy, because wiping the diff desynchronises the terminal from the
effect's canvas. Neither can use `Canvas`: `Canvas::commit` swaps its surfaces,
which is right for an effect that repaints everything each frame and wrong for
anything that accumulates a delta. There is more on this in
`specs/overview.md`.

Effect switching is done: `n` and `p` walk the catalogue behind a diagonal wipe,
via `EffectHost` and the `FrameTarget` seam in the frame loop. A repeat is
dropped at the key handler, so a held key advances one effect per transition
rather than strobing. Note the mouse decision recorded in `main.rs` -- capture is
enabled for the whole session because `n` can bring up the ink field at any
moment.

Effects also throttle to `[global] idle_fps` while the terminal is unfocused,
and freeze rather than slow down, so nothing jumps on resume. Two things to know
before touching it: the rate is clamped away from zero on purpose, because a
terminal that never reports focus changes would otherwise leave the program
frozen with no way back; and `InputState` has a *hand-written* `Default` solely
so `focused` starts `true`. The derive would give `false`, and every test,
`--check`, and the first frame of a real run would then believe nobody was
watching.

One trap worth repeating: `--print-config` writes every default to disk, so a
user with a generated `~/.config/termzzz.toml` pins whatever the defaults were
when they generated it. That is how the donut's 60x-too-slow rotation speed
survived a default change in this project -- the new default never reached anyone
with an existing config.

Three smaller things, all optional:

- `AnyEffect` could become `Box<dyn TerminalEffect>` now that the loop is
  dynamic, deleting the enum and its 96 forwarding arms. It is not free: one
  playlist test reaches into `AnyEffect::Ascii` to read `paused`, which would
  need either an `as_any` on the trait or a new `is_paused` method that only
  `AsciiField` would override. Judge it when something else wants the box.
- The `run_loop*` wrappers still exist as five signatures over one loop. They
  are each three lines, so they are cheap, but they could collapse to one
  function with a small options struct.
- `transition` means the whole switch for `EffectHost` and each half for
  `Playlist`, so one `--transition` flag gives two durations. Both readings are
  pinned by a test; pick one and reconcile them if the two are ever meant to
  feel the same.

The `Canvas` work is done: all fifteen effects hold one instead of open-coding
the double-buffer pattern, and none of them allocates a full-screen `Buffer` per
frame any more.

The sub-cell renderer is done too, in `src/render/`: `braille` (8x density, one
colour per cell), `halfblock` (2x vertical, two colours per cell, which is what
gave `Cell` its background), `dither` and `palette`. Only `halfblock` is in use,
by `mandelbrot`; `braille` and `dither` have no callers yet. Note the
aspect-ratio caveat: both sub-cell modes assume a cell about twice as tall as it
is wide, and DejaVu Sans Mono is nearer 1:1.2, so output looks vertically squashed
there. That is a property of the technique, not a bug to fix.

Determinism is done: all nine effects that used an unseeded generator now carry a
`seed` option and a seeded `StdRng`, `--seed <N>` overrides all of them, and
`tests/effect_contracts.rs` asserts both reproducibility and seed sensitivity for
every effect. Because they became comparable, that suite also found that five of
them advanced by a fixed step per rendered frame — `maze`, `boids`, `crab`,
`pipes` and `constellation` ignored the speed keys — which is fixed too.

Not worth doing soon: further performance work at ordinary sizes. Nothing is
dropping frames at 200x50 — the worst effect there uses about 10% of a 60 fps
budget, and `frame_times` now passes its own 2 ms check at that size outright.
At 400x200, which is an eight-times-heavier terminal than that budget assumes,
`mandelbrot` and `life` are still over; `life` only on its `update x4` worst case
at high `--speed`, and `mandelbrot` on render, which is item 1 above.

Two numbers in the frame table went *up* in the recent work, and both are
correct. `mandelbrot` went from 31 KB to 215 KB of escape sequences per frame at
400x200, and `matrix` from 63 KB to 67 KB. Each was previously cheap because it
was broken: the mandelbrot was showing a flat wash with no detail to change, and
the matrix drop tails were all one saturated colour because a truncating cast
destroyed the fade. A byte count is only meaningful next to what is on screen.

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
