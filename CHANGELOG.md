# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - Unreleased

### Fixed
- The output path no longer emits a cursor move and a colour change per cell. A
  move is only sent when a cell is not the one after the previous one, and a
  colour is only re-sent when it changes. This cuts ANSI volume per frame by
  18% to 62% depending on the effect, and by 46% for fire
- A configuration file that omits a section no longer silently zeroes that
  section. `Config` now falls back to the real per-effect defaults for any
  section the user did not mention, instead of to each options struct's derived
  all-zero `Default`. Previously a config containing only `[dvd]` reset the
  other fourteen sections, which made the donut panic, the cube invisible and
  the terrain a solid block
- Resizing the terminal no longer makes effects emit cells outside the new
  screen, and no longer panics in Conway's Game of Life. Cell coordinates are
  now derived from the frame being produced rather than the stale previous frame
- A panic no longer leaves the terminal in raw mode on the alternate screen with
  no way out. Terminal setup and teardown moved into a single `TerminalSession`,
  and a panic hook restores the terminal before the process aborts
- `terrain`'s `seed` option now does something. The seed was stored and then
  ignored, so every seed rendered the identical landscape. The permutation table
  is now shuffled per seed, and rebuilt on reset
- `terrain` no longer renders a flat solid screen when `octaves` is zero or
  negative, which produced a division by zero and a NaN that no branch matched
- The matrix colour ramp now reaches its start and end colours exactly. The
  second half was divided by the wrong distance, so the interpolation ran to
  2.0 and extrapolated past the end colour into black, and the first index never
  reached 0, so the drop style wanting a pure white head never got one
- Rain drops no longer take their style from the ambient thread random generator
  while a generator passed in as an argument went unused, so seeding a drop now
  actually seeds it
- Rain drops no longer ignore sub-millisecond timesteps. The delta was truncated
  to whole milliseconds, so a fast terminal produced no movement at all
- The matrix and boids effects no longer overflow their drop and boid counts on
  terminals past 256x256, where the width and height were multiplied as `u16`
- The crab effect measures its sprite width in characters rather than bytes, so a
  frame whose widest line contains a multi-byte character no longer skews its
  bounce
- Conway's Game of Life no longer derives its simulation state from the last
  rendered frame, which made every generation stepped between two renders evolve
  the same stale input. Seeded gliders are now subject to the rules, so they live
  and travel instead of being replaced each generation, and the 270-degree
  rotation no longer produces a malformed shape
- A shuffled playlist no longer repeats the current effect. It retried once, which
  still returned the same effect about one time in fourteen
- The maze wall texture no longer erodes away over a long run, because the
  per-frame sparkle was written into the persistent wall template
- The donut no longer panics on an empty `luminance_chars`, and no longer
  renders a blank screen when `distance` is small enough to put the torus behind
  the camera
- Maze no longer generates its wall texture twice per reset, and no longer
  panics on a zero-sized terminal

### Changed
- The release profile optimizes for speed rather than size. This is a real-time
  renderer doing per-cell trigonometry sixty times a second, and `opt-level = "s"`
  suppressed the inlining that work depends on. Costs roughly 23% more binary
  size and buys 28% to 98% on the heavier effects
- The donut, plasma and fire effects advance by elapsed time rather than by a
  fixed step per frame, so their speed no longer depends on the terminal refresh
  rate and the `+`/`-` keys mean what the help text says
- The donut hoists its loop invariants and uses precomputed sine and cosine
  tables, roughly halving its frame cost
- `Buffer::diff` derives coordinates from the frame being produced, and no longer
  scans the whole screen to find the handful of cells a small logo changed
- Conway's Game of Life counts live neighbours without allocating a vector per
  cell per generation

### Added
- Effect contract and drift tests covering the registry, the config surface,
  resize handling, timing and input handling for every registered effect
- Per-effect frame benchmarks, and a `frame_times` binary reporting update,
  worst-case update at high speed, render, output-encoding cost and ANSI volume
  per frame at three terminal sizes. It exits non-zero when an effect exceeds
  its frame budget
- Tests for the output encoding, the colour ramp, and the noise generator
- Context-driven input and frame state through the runtime module
- Interactive generative ASCII field effect available with `termzzz ascii`
- Pointer interaction, pause/resume, reseeding, palette cycling, and brush-size controls
- Resize-safe rendering with screen-derived option updates for existing effects
- `terrain` is now available through the normal CLI validator
- Bouncing ASCII logo effect available with `termzzz dvd`, with a configurable multi-line logo
- Timed playlist mode via `--playlist`, `--shuffle`, and `--transition`, also configurable in `[playlist]`
- Central effect registry so the CLI, help output, check mode, and playlists share one source of truth
- Global speed control through `[global] speed`, `--speed <MULT>`, and the `+`/`-` keys

### Changed
- `+` and `-` change global animation speed; the mouse wheel resizes the ASCII brush
- Conway's Game of Life advances at a configurable generations-per-second rate
- Matrix, cube, and DVD effects advance on real frame deltas rather than fixed step counts
- Calmer defaults for plasma, boids, donut, pipes, cube, crab, and life
- Check mode now honours the configured effect options and global speed
- Release and crates.io workflows now require explicit manual dispatch

### Compatibility
- `termzzz` is a clean break from earlier package and command names, and provides no compatibility aliases
