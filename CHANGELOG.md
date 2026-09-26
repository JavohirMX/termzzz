# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - Unreleased

### Fixed
- **The donut now visibly spins.** `rotation_speed_a` and `rotation_speed_b` are
  radians per *second* — the effect multiplies them by the frame delta — but the
  values were the old per-frame numbers, so they were being applied 60× too
  slowly. One revolution took 4 minutes 46 seconds, and ten seconds of watching
  turned the torus 12.6°, which does not read as a slow animation but as a
  broken effect. The defaults are now 1.32 and 0.60 rad/s, a revolution every
  4.8 and 10.5 seconds
- Effects now slow down when the terminal window does not have focus, instead of
  spending a core on a screensaver nobody is looking at. It drops to
  `[global] idle_fps` (4 by default) and the simulation is frozen rather than fed
  a coarser delta, so nothing jumps when you come back. Set
  `[global] pause_when_unfocused = false` to disable

  Two notes. This *throttles* rather than stops, deliberately: focus reporting
  is off by default in most terminals, and a screensaver that stopped and could
  not be restarted would be a far worse failure than one that runs quietly at
  4 fps. And quitting still works while unfocused, with up to one second of
  latency at the slowest rate.
- **Colours are no longer washed out.** The output path was skipping the colour
  on every cell after the first of a run, because it tracked the style as still
  being in effect when the terminal had in fact reset it after each glyph. Those
  cells were drawn in the terminal's default foreground, which is white on a dark
  profile. Cells tagged `Attribute::Reset` were worse: that is SGR 0, so it
  cleared the colour that had just been set, before the glyph was drawn. The
  mandelbrot tags every cell `Reset` and was rendering entirely white. The
  encoder is now built from crossterm's individual commands so it controls what
  is emitted and when
- A cell's background colour now reaches the terminal. It was never written at
  all, so the half-block glyph `▀` painted its bottom half in the terminal's
  default background -- which cost the mandelbrot half its vertical resolution
  and the two-colours-per-cell capability the sub-cell renderer exists for
- A cleared cell is now a space in the terminal's own background rather than a
  space in black. On a light terminal profile a cleared cell was a black block,
  and a wipe could not reproduce a clear because the two used different blanks
- Switching effects with `n` and `p` no longer leaves the previous effect on
  screen. The host wiped the effect's *diff* while the effect had already
  committed the un-wiped frame to its own canvas, so the terminal held cells the
  effect believed were painted and never re-sent them. Both the host and the
  playlist now accumulate the delta into a frame buffer and wipe a copy
- `--playlist` no longer flickers sixty times a second. `Canvas::commit` swaps
  its two surfaces, so a consumer that accumulates a diff was handed the frame
  from *before* the one it had just emitted, alternating between a frame and a
  nearly empty one
- Switching effects no longer flashes the outgoing effect at full brightness for
  one frame. A `Swap` phase reported "no wipe", and the rebuild happened after
  that frame's cells were produced
- Holding `n` or `p` no longer strobes. A held key arrives as a stream of
  repeats, and each one advanced the effect *and* restarted the transition, so
  the wipe never got past its first frame and never hid anything while sixteen
  effects were cycled through at key-repeat rate
- `--transition` now reaches `n` and `p`. It was parsed into the playlist options
  and nothing else, so the wipe behind a keypress always used the host's default
- The transition now lasts the configured duration. Each phase compared against a
  hardcoded `1.0`, so `transition` had no effect on either length or progress and
  every switch took two seconds
- The frame loop no longer blocks for up to ten milliseconds waiting for input on
  every frame, which was most of a 60 Hz budget spent asleep and was counted as
  elapsed time handed to the effect
- The frame loop holds a fixed cadence instead of sleeping for the remainder of
  the last frame, which could not repay an oversleep and so drifted
- The mandelbrot no longer zooms past the point where it can resolve anything.
  The depth limit is now derived from the iteration budget and the field's `f32`
  precision, and `recentre` restarts at a scale where the boundary is across the
  screen instead of the widest view, where the set is a small blob in a lot of
  black. A tour went from about 36 seconds to 17.5, most of the old one a flat
  wash
- The mandelbrot no longer panics on a `max_iterations` of 23 or less. The
  iteration budget clamped against a floor above its own ceiling
- The donut's brightness ramp is no longer inverted. Dim glyphs were painted in
  the two lightest colours in the palette and bright ones in yellow and orange,
  and the top of both the glyph and colour ramps was unreachable
- Matrix drop tails no longer saturate to full-bright green. A `.clamp(10, 256)
  as u8` truncated 256 to 0, so the bottom two-thirds of every drop was one flat
  colour instead of a fade
- The donut and the matrix no longer leave a stale drawing surface behind on
  `update_size`, which left a growing terminal partly unpainted and a shrinking
  one keeping stale pixels
- Plasma no longer repaints a sixth of the screen every frame. Its palette
  advanced ten entries a second, and at 400x200 that measured 824 KB of escape
  sequences per frame, or 49 MB/s. With the palette work and the encoder fix it is
  483 KB
- Fire's core is no longer pure white, and a no-colour config no longer paints
  every blank cell white
- The dvd logo no longer starts on a near-white colour, which made it invisible
  on a light terminal for a sixth of its cycle
- The maze's carved path is no longer a white bar, and effects no longer paint
  with a blanket `Bold` that brightens an already-pale colour toward white
- The output path no longer emits a cursor move and a colour change per cell. A
  move is only sent when a cell is not the one after the previous one, and a
  colour is only re-sent when it changes. This cuts ANSI volume per frame by
  18% to 62% depending on the effect, and by 46% for fire
- A configuration file that omits a section no longer silently zeroes that
  section. `Config` now falls back to the real per-effect defaults for any
  section the user did not mention, instead of to each options struct's derived
  all-zero `Default`. Previously a config containing only `[dvd]` reset every
  other section, which made the donut panic, the cube invisible and the terrain a
  solid block
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
- Nine effects no longer ignore the global speed control. `maze`, `boids`,
  `crab`, `pipes` and `constellation` advanced by a fixed step per rendered frame
  or did their work inside `get_diff`, so their speed depended on the terminal
  refresh rate and the `+`/`-` keys changed only how often they were drawn. All
  five now integrate `context.delta`, and `maze` and `pipes` accumulate elapsed
  time into whole cells so they advance at a fixed rate regardless of frame rate
- Nine effects are now reproducible. `matrix`, `life`, `maze`, `boids`, `crab`,
  `dvd`, `pipes`, `fire` and `constellation` drew from an unseeded thread-local
  generator, so no two runs looked alike and the contract suite could not check
  their timing at all. Each now carries a `seed` option and a seeded generator,
  reseeded on reset so a resize starts a fresh run rather than continuing one

### Upgrade notes
### Upgrade notes
Three changes in this release can be silently cancelled or silently lost by a
config file written by an earlier build. They are collected here because
`--print-config` writes every default to disk, which means a generated config
pins whatever the defaults were the day it was generated.
- **`termzzz ascii` is now `termzzz ink`.** Renamed with no deprecated alias, so
  the old name is an unknown effect and the CLI says so. Three consequences:
  - an old `[ascii]` config section is **silently ignored** and the effect runs at
    its defaults. Serde has no `deny_unknown_fields` here on purpose: a hard
    failure would also break anyone carrying a stale key from any earlier rename,
    and an ignored key is a much better failure than a refusal to launch. Rename
    the section to `[ink]` to keep your settings.
  - an old `effect = "ascii"` entry in a **playlist is now reported on stderr**
    along with the list of names this build knows, rather than dropped in
    silence as it used to be. This is the one that used to lose data without
    trace.
  - `needs_mouse` is unchanged, so mouse capture is still enabled for the whole
    session and the interactive controls (`r`, `Space`, `[`, `]`, wheel, pointer)
    still work. They are just under a different name.
- **`[terrain]` `scale` now means terminal cells per period of the base-frequency
  noise**, where it used to be a bare frequency multiplier. An existing
  `scale = 0.02` now means 0.02 cells per period -- effectively a flat wash --
  rather than one noise cell per 50 cells. **Delete the line** to get the new
  default, which is sized for an ordinary terminal.

  This is deliberate. The intuitive reading of `scale` -- a count of noise cells
  across the screen -- is the wrong one: holding that count fixed means a 400x200
  terminal spreads the same periods over five times as many cells and the ground
  goes flat. Measured as the fraction of neighbouring ground cells changing ramp
  step, the count reading gave 14.2%/17.7% at 80x24 against 3.1%/3.8% at 400x200;
  fixing the period instead makes it size-independent at 15.3%/17.8%. Shipping
  the count reading would have meant a version that looks like terrain on a small
  terminal and like a smear on a large one.
- **If you have a `~/.config/termzzz.toml` generated by an earlier build, delete
  its `[donut]` `rotation_speed_a` and `rotation_speed_b` lines** (or set them to
  `1.32` and `0.6`). `--print-config` writes every default to disk, so an
  existing config pins the old, 60x-too-slow values and the faster defaults will
  not reach you.
- A `[global]` section written by an earlier build has no `pause_when_unfocused`
  or `idle_fps`. Both fall back to their defaults (`true` and `4.0`) because
  `Config` inherits from the real defaults for anything a file omits, so an old
  config keeps working and gets the new behaviour. Every effect's options struct
  now carries `#[serde(default)]` too, so a section missing a *key* is no longer
  a startup error either -- only a whole missing section was ever covered before.

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
- `+` and `-` change global animation speed; the mouse wheel resizes the ASCII brush
- Conway's Game of Life advances at a configurable generations-per-second rate
- Matrix, cube, and DVD effects advance on real frame deltas rather than fixed step counts
- Calmer defaults for plasma, boids, donut, pipes, cube, crab, and life
- Check mode now honours the configured effect options and global speed
- Release and crates.io workflows now require explicit manual dispatch
- The pipes effect simulates in `update` and only renders in `get_diff`, which is
  what let its growth be driven by elapsed time. Its render cost drops about 20%
  because the frame is no longer rebuilt as a side effect of drawing it

### Added
- Effect contract and drift tests covering the registry, the config surface,
  resize handling, timing and input handling for every registered effect
- Per-effect frame benchmarks, and a `frame_times` binary reporting update,
  worst-case update at high speed, render, output-encoding cost and ANSI volume
  per frame at three terminal sizes. It exits non-zero when an effect exceeds
  its frame budget
- Tests for the output encoding, the colour ramp, and the noise generator
- Context-driven input and frame state through the runtime module
- Interactive generative field effect available with `termzzz ink` (was `termzzz ascii`)
- Pointer interaction, pause/resume, reseeding, palette cycling, and brush-size controls
- Resize-safe rendering with screen-derived option updates for existing effects
- `terrain` is now available through the normal CLI validator
- Bouncing ASCII logo effect available with `termzzz dvd`, with a configurable multi-line logo
- Timed playlist mode via `--playlist`, `--shuffle`, and `--transition`, also configurable in `[playlist]`
- Central effect registry so the CLI, help output, check mode, and playlists share one source of truth
- Global speed control through `[global] speed`, `--speed <MULT>`, and the `+`/`-` keys
- A `seed` option on every effect that uses randomness, and a `--seed <N>` flag to
  override all of them for one run, so a run can be reproduced or shared
- Contract tests asserting that every seeded effect is reproducible and that a
  different seed actually changes what it draws, and that every animated effect
  renders differently at 60fps than at 20fps

### Changed
- The frame loop takes `&mut dyn TerminalEffect` instead of a generic. It was
  monomorphised once per effect type, so the timing, input and encoding code
  existed fifteen times over in the binary
- All fifteen effects draw through a shared `Canvas` instead of open-coding the
  double-buffer pattern, and none allocates a full-screen `Buffer` per frame any
  more. The maze blits its wall template rather than cloning it, and the pipes
  render about 8% faster at 400x200 as a side effect of simulating in `update`

### Added
- A sub-cell renderer in `src/render/`. `braille` packs eight dots into a cell
  for 8x density in one colour; `halfblock` splits a cell into foreground over
  background for 2x vertical resolution in two colours, which is what gives
  smooth gradients; `dither` turns a hard threshold into a gradient with a
  stable Bayer offset; `palette` holds the colour ramps effects share

### Added
- `n` and `p` move to the next and previous effect without leaving the session,
  behind the same diagonal wipe the playlist uses. `EffectHost` owns the running
  effect, the cycle order and the transition; `FrameTarget` is the seam that lets
  one frame loop serve both a swappable effect and a fixed one
- `mandelbrot`, an escape-time renderer for the Mandelbrot set that zooms
  continuously into the boundary and re-picks its coastline by seeded rejection
  sampling when it gets too deep to resolve. Drawn with half-blocks, so the
  escape-time bands are read as colour rather than density. `max_iterations` is
  the quality dial and the dominant cost, and it now also sets how deep the camera
  may zoom: the deepest useful scale moves one decade per doubling of the budget,
  and a second term stops the camera where `f32` sample coordinates stop being
  distinguishable from one another. `MIN_USEFUL_ITERATIONS` floors the budget
  itself

### Changed
- `Cell` carries a background colour. Only the half-block glyph `▀` needs it,
  and `Cell::new` keeps its three arguments and leaves the background at the
  terminal default, so no existing call site changed and no effect that draws
  one colour per cell can accidentally paint a second

### Removed
- The `DefaultOptions` trait and its thirteen implementations. Nothing called it,
  and its hand-written copies of the per-effect defaults had drifted from the real
  ones in nine places across five effects — `plasma` still claimed a colour speed
  of 150.0 against a real 20.0, and `pipes` a line count of 5 against a real 3.
  Each options struct's own `Default` is now the only place a default is written

### Compatibility
- `termzzz` is a clean break from earlier package and command names, and provides no compatibility aliases
