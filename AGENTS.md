# Project Overview

## Quick Reference

`termzzz` is a collection of terminal-based screensavers and generative visual effects written in Rust.

### Main Commands
```bash
# Build
cargo build --release

# Run effects
termzzz matrix      # Matrix digital rain
termzzz life        # Conway's Game of Life, cell age in the glyph
termzzz mandelbrot # Escape-time Mandelbrot set, zooming and panning
termzzz maze        # Maze generation, held when finished
termzzz boids       # Boids flocking simulation, with trails
termzzz cube        # 3D cube rotation, filled and depth-shaded
termzzz crab        # ASCII crabs scuttling along a seabed
termzzz donut       # 3D donut rotation
termzzz pipes       # Pipe maze animation
termzzz plasma      # Plasma effect, value in the glyph
termzzz fire        # Fire simulation
termzzz terrain     # A landscape: height field, surface, filled ground
termzzz solarsystem # 3D orrery: tilted orbits, real periods
termzzz dvd         # The real DVD wordmark, bouncing, in braille
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
mode, the effect registry, a pinnable terminal background, and a 3D solar system
are all done. What remains is the canonical GitHub repository and the
distribution metadata (crates.io publication, Homebrew, Nix).

Open engineering work, in priority order:

1. **`mandelbrot` render cost at large sizes.** 11.9 ms per frame at 400x200,
   which is over half a 60 Hz budget spent before a single byte is written. The
   cost is very close to linear in `max_iterations`, because nearly every interior
   pixel spends the whole budget discovering it never escapes. Lowering the
   default is the first thing to try; a region-marking rewrite is where the real
   win is, and that is a rewrite rather than a tweak. `life` is over the 2 ms
   budget on its `update x4` worst case only, and only because
   `step_generation` scans a `HashMap`.
   `terrain` is *no longer* over, and by a lot: the height-field rewrite took it
   from 1.3 ms and 554 KB to 350 µs and 17 KB, because the surface costs one
   noise sample per column rather than per cell and unchanged sky cells drop out
   of the diff
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

The sub-cell renderers are in `src/render/`: `braille` (8x density, one colour
per cell), `halfblock` (2x vertical, two colours per cell, which is what gave
`Cell` its background), `quadrant` (2x on *both* axes, two-tone), plus `dither`
and `palette`. Three of the four have callers: `mandelbrot` uses `halfblock`,
`cube` uses `braille` and `dither`, `dvd` and `solarsystem` use `braille`.

`quadrant` currently has **none**. `dvd` was its only caller and moved to
`braille` when the real DVD wordmark arrived, because that logo is a
single-colour silhouette and braille's trade is density rather than hue. It is
kept rather than deleted -- it is tested, and it is still the only sub-cell on
both axes in the crate -- but do not describe it as having a caller.

Note the aspect-ratio caveat, which applies to all of them: they assume a cell
about twice as tall as it is wide, and DejaVu Sans Mono is nearer 1:1.2, so output
looks vertically squashed there. That is a property of the technique, not a bug to
fix.

`quadrant` exists because `▀` and `▌` cannot be combined -- a cell has one
foreground and one background, so a vertical split spends both. It is
deliberately limited to a *two-tone* bitmap, since four arbitrary colours cannot be
shown in two. Its font risk is local rather than global: a solid interior is `█`
and stays solid, so a terminal missing `▛▜▙▟` only loses a shape's outline.

Braille is one colour per cell, which is a real constraint and not a detail: an
effect that wants per-dot colour has to keep a parallel per-cell colour array, and
`dvd` and `solarsystem` both pair theirs with a depth buffer. Without the buffer
the first thing drawn wins every contested cell -- `solarsystem` draws the sun
first, so a planet crossing in front of it would vanish for the length of the
conjunction.

There is also a shared `GlyphRamp` in the same directory, and five effects draw
their characters from it: `cube`, `donut`, `life`, `plasma` and `terrain`. (`crab`
uses only the module's character-width filter, not the ramp. This said "eight" for
a while, which is how a count drifts when nobody checks it against `rg`.) Read its
module doc before choosing a set: the ORDERING is the whole point, and a ramp that
puts the light characters on the ones covering most of the screen makes the
majority of the frame the brightest thing on it. `presets::BLOCKS` is the only set
whose ordering Unicode defines rather than taste, which made it the right default
for anything using the character as a *value* rather than as texture — and
`terrain` and `plasma` have both now moved off it to ASCII, with the cost written
down on each constant. Two things that cost is worth knowing before moving a third:
an ASCII ramp cannot be monotonic in ink at all, and a ramp for a *filled region*
must not begin with a space, because the sparsest step lands on the row at the top
of the region and a space there makes the region invisible against whatever is
behind it.

Determinism is done: all nine effects that used an unseeded generator now carry a
`seed` option and a seeded `StdRng`, `--seed <N>` overrides all of them, and
`tests/effect_contracts.rs` asserts both reproducibility and seed sensitivity for
every effect. Because they became comparable, that suite also found that five of
them advanced by a fixed step per rendered frame — `maze`, `boids`, `crab`,
`pipes` and `constellation` (now `solarsystem`) ignored the speed keys —
which is fixed too.

Not worth doing soon: further performance work at ordinary sizes. Nothing is
dropping frames at 200x50 — the worst effect there uses about 10% of a 60 fps
budget, and `frame_times` now passes its own 2 ms check at that size outright.
At 400x200, which is an eight-times-heavier terminal than that budget assumes,
`mandelbrot` and `life` are still over; `life` only on its `update x4` worst case
at high `--speed`, and `mandelbrot` on render, which is item 1 above.

Two numbers in the frame table went *up* in the earlier audit, and both are
correct. `mandelbrot` went from 31 KB to 215 KB of escape sequences per frame at
400x200, and `matrix` from 63 KB to 67 KB. Each was previously cheap because it
was broken: the mandelbrot was showing a flat wash with no detail to change, and
the matrix drop tails were all one saturated colour because a truncating cast
destroyed the fade. A byte count is only meaningful next to what is on screen.

Two numbers then went sharply *down*, and both are worth understanding rather
than just noting. `terrain` fell from 554 KB to 17 KB because the height-field
rewrite leaves the sky as a space that matches the cleared cell, so unchanged sky
drops out of the diff entirely — the effect got *more* structured and 32x cheaper
to emit. `dvd` is 596 bytes at 400x200 for the same reason in braille: a cell with
no raised dot is not written. **Not writing a cell is the cheapest rendering
optimisation there is**, and both of these found it by accident rather than by
looking for it. Check whether an effect is writing cells that did not change before
optimising anything it computes.

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
