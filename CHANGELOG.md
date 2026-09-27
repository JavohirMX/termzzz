# Changelog

All notable changes to this project will be documented in this file.

## [0.2.0] - Unreleased

Everything below is in three groups: what a session of watching the effects
running turned up, then the earlier audit that found the outright bugs, then the
upgrade notes. The first group is first because it is the most recent and because
two of its entries — a terminal background setting and a solar system — are the
only genuinely new features here.

### Added
- **`[global] background` and `foreground`.** The terminal's own colours, pinned
  for the duration of the session. This is the fix for a class of complaint that
  no amount of per-effect tuning reaches: on a terminal with a tinted profile,
  every effect that draws its dark end near black looks broken rather than
  different, and the gaps in a logo let the profile show through. Emitted once as
  a single `SetBackgroundColor`/`SetForegroundColor` pair when the session starts
  and reset on the way out — including from the panic hook, which reads the same
  static the normal restore does so the two cannot disagree.

  Painting the background cell by cell was the alternative and it is the wrong
  one: it would double the byte volume of every sparse effect and need the same
  code in eight of them. The default is `reset`, so a user who never touched the
  setting gets no escape sequence at all rather than a redundant pair of resets
  that repaints for nothing.

  crossterm's `serde` feature is enabled so `style::Color` can sit in the config
  directly, which means a typo is a TOML error with a line number instead of a
  silent fallback at the moment the screen is already being drawn. The accepted
  spellings are crossterm's serde ones and differ from `FromStr`'s — `dark_grey`,
  not `darkgrey`; `rgb_(12,12,20)`, not `rgb(12,12,20)`.
- **`solarsystem`**, replacing `constellation`. A real 3D projection rather than a
  tilted drawing of one: the world is tilted about the x axis and *then* rotated
  about the vertical, and the order is the whole effect — rotating the other way
  round pivots on the ecliptic's own axis, on which a circular orbit is
  symmetric, so the camera would do nothing at all. Perspective makes a planet
  on the near side of its orbit larger than the same planet at the far side,
  which is the depth cue that stops a tilted view of concentric circles reading
  as a spiral. Drawn in braille at 2x4 density, with a depth buffer so a planet
  crossing in front of the sun is not painted over by the sun's glow.

  Orbital *periods* are the real ones, which is why the inner planets race and
  Jupiter crawls. Orbital *radii* are compressed by `radius_exponent` (0.6),
  because Neptune really is 78 times further out than Mercury and drawn to scale
  on a 24-row screen the inner system is a single dot. `radius_exponent = 1.0`
  gives true scale, at which point the effect is correct and unreadable.

### Changed
- **`terrain` is a landscape now.** It did not have one. The renderer sampled
  2D noise at every cell below a fixed horizon and mapped the value straight onto
  a glyph: no surface, nothing filled below anything, and the result was a
  full-screen wash with a blank sky above it. It was reported as "cut off,
  showing only half of my terminal", which is close to the truth — the top half
  really was empty sky and the bottom half was texture with no shape in it.
  There is now a height field sampled per column, a surface that undulates
  around the horizon, ground filled from the surface to the bottom row, and
  shading by depth below it. The surface costs one noise sample per *column*
  rather than per cell, which at 400x200 is 200 samples against 80,000.
- **`[terrain] glyphs` now defaults to ASCII** (` .:-=+*#%@`) instead of the
  shade blocks, which were described in the report as "full locks" and
  unnecessary. The cost is written down on the constant: `░▒▓█` was monotonic in
  ink *by the Unicode standard*, and no ASCII set has that property. Two adjacent
  pairs in the conventional ramp run backwards — `=` carries more than the `+`
  after it, `+` less than the `*` after it — and a test names them rather than
  asserting monotonicity, so the next person to reorder the ramp finds out that
  it is a decision.
- **`terrain`'s glyph and colour ramps now encode different things.** The glyph
  gets denser with depth and the colour gets darker, because they answer
  different questions — how much rock is there, and how much light reaches it —
  and for ground those disagree. The old effect had both encoding one scalar,
  which only worked because it had no surface for either to be relative to.
- **`[terrain] relief` is new** (default 1.0): how far the surface may rise above
  and fall below the horizon, in rows per period of the base-frequency noise. In
  units of the period rather than of the screen, because the screen made the
  landscape's *shape* depend on the terminal — with a fixed horizontal period, a
  row amplitude that grows with the height made the surface steeper on a tall
  screen. Measured as silhouette step density, one setting drew a landscape at
  25% on an 80x24 and a picket fence at 76% on a 200x50.

  Measured after the rewrite, terrain went from **1.3 ms and 554 KB per frame to
  350 µs and 17 KB** at 400x200. The byte count fell 32x because unchanged sky
  cells now drop out of the diff, and the render cost 3.7x because the surface
  costs one noise sample per *column* rather than per cell — 200 samples at
  400x200 against 80,000.
- **The DVD logo is the real wordmark**, supplied as braille art and stored as a
  60x28 dot bitmap, 30x7 cells. It carries a 3D extrusion in its lower half,
  which is why it is 7 rows rather than 5, and it is kept as drawn. It is
  rendered in `braille` rather than `quadrant`, which leaves `quadrant` with no
  callers at all; it is kept, being tested and still the only sub-cell on both
  axes in the crate, but it does not have one any more. The flat silhouette also
  made `quadrant`'s second colour pointless, and it had been painting a black
  background behind every edge cell to use it.
  Recolouring moved from every wall bounce to every third, which is the most
  likely source of the flicker: recolouring a solid 30x7 slab every 1.4 seconds
  is a large simultaneous change. The speed cap is 18 cells a second, down from
  24 — the cap is about how many sub-cell samples land per frame, not how fast
  the logo crosses, and braille's 2 dots per cell make 18 into 36 samples against
  60 frames. Note that the obvious-looking justification for 18 is wrong and is
  recorded as such in the code: scaling 24 by the old logo width over the new
  gives 18.4, but that product means nothing, and holding the time to cross one
  logo width fixed would want 31, not 18. The real reason 18 is right is that it
  is slower than before and the complaint was that the effect was too lively.
  The frame is 596 bytes at 400x200 because cells with no raised dot are left
  untouched rather than written as a blank braille glyph, and that is flat across
  terminal sizes.
- **`[plasma] glyphs` defaults to ASCII** (` .-:;+X#%@`). The block set was
  monotonic in ink *by the Unicode standard* and no ASCII set has that property;
  the cost is written on the constant and a test names the ties it cannot resolve
  rather than asserting monotonicity. An existing plasma test was sweeping 18% of
  the field's value cycle, which was enough for a 5-step ramp and not for a
  10-step one — it claimed the value was not pinned to either end of the ramp
  without examining enough of the ramp to know. Widened to a full cycle, so the
  assertion got harder.

### Fixed
#### Watching them run
A second pass over the effects, driven by watching them run rather than reading
them.

- **The ink's white was not white, and had been reported twice.** The top stop of
  the ramp was `rgb(230, 243, 227)` — 6.6% saturated green — and the first report
  of it was not acted on. It is `rgb(255, 255, 255)` now, and there is a
  `[ink] colors` option so the ramp can be pinned to a single hue if the phosphor
  look is not wanted. Two further bugs surfaced while fixing it: the field was
  the last in the crate still setting `Attribute::Bold`, and it did so on exactly
  `value >= 0.72` — the white that was being complained about — so on a terminal
  that honours the hint there was a second brightness term on top of the ramp's.
  And a configured `colors` list was accepted by the config and then discarded by
  the renderer, which passed an empty vector.
- **The crabs overlapped and flickered.** They had no runtime separation at all,
  only spawn-time placement; pairs sharing a drawn column went from 28 of 39,600
  at 200x50 to zero. `WALK_GAIN` is back to 1.0, so 2.1–3.9 cells a second rather
  than the 10.5–19.5 that a previous change introduced. Separately, the leg
  animation is a plain 0.2 s timer and is *not* derived from the walk speed, so
  slowing the body left the legs snapping five times a second — which is what
  "they change too fast" was about. Now 0.4 s, and the test's bound came down from
  8 Hz to 4 to match.
  Two bugs had to be fixed to make separation work at all, neither of them in the
  brief: the collision response was reversing *both* crabs on an overtake, which
  converts a catch-up into a head-on pair that is more closing (571 reversals in
  600 frames, the colony vibrating in place), and a crab between two head-on
  neighbours was being reversed once per pair per frame.
- **Conway's Game of Life changed every cell's character every frame.** It now
  runs at 3 generations a second rather than 8, and the glyph is banded into
  three bands of three generations, so cells that changed character per
  generation fell from 94% to 14.3%. The colour ramp was left continuous and
  carries the gradient the glyph gave up. The bands divide evenly rather than
  following the age curve, because bands sized by population give the newborn
  band — where 92% of a soup lives — the shortest hold.
- **The cube is edges-only again.** `filled` defaults to false. The filled version
  is still there and still works; the report was that the wireframe gave a better
  *feeling*, because a filled near face runs its own dither up to the same
  densities as its edges and the silhouette stops being a silhouette.
- **The donut gained a `palette` knob** and its dark end was lifted, measured as
  CIE76 ΔE against a greyish-blue `rgb(60,70,85)`: the darkest stop went from
  ΔE 24.6 to 67.8, and the worst of the twelve shades from 24.6 to 55.1. The
  cost is real and is in the comment: a torus on a black background is now a
  mid-tone mass where it had a deep shadow, because the ramp's usable range
  starts at 52 rather than 0. Pin `[global] background` if you want the old
  relationship.
- **The matrix's character pool is now defended rather than explained.** The pool
  is mostly katakana because a pool of one script is a pool of a few distinct
  shapes, which is not visible in the code and which a future reader would
  plausibly "fix". It is a test now, counting by range over the live pool rather
  than against a copy of the string — a copy measures the copy, and the first
  version of this test was satisfied by a pool trimmed to 16 of 54 characters.

### Fixed
#### Second pass
A round driven by watching the effects run rather than reading them. Four of
these were bug reports, and two of the four turned out not to be bugs at all.

- **The crabs could not go up, because there was no slope.** `ground_row`
  returned one constant row for the whole screen and every crab's `y` was
  clamped to it, so a crab could leave the sand only by hopping. Nothing was
  broken; the feature had never been built, and every piece of prose in the file
  called it a *seabed*, which is what made an absence look like a fault. The
  seabed is a seeded height profile now, the sand is filled from it to the bottom
  of the screen, and it scrolls at exactly the rate the crabs walk so an animal
  stays put relative to the ground under it.

  Two bugs the slope exposed, both of which had been latent behind the flat
  ground. `update` sampled the ground and *then* moved in x, so `position.1` was
  pinned to the profile at the crab's old column. And "airborne" was a float
  comparison against a moving sample, so on a slope a walking crab sat a
  hundredth of a row off its own ground depending on its direction — read as
  "in the air", and a crab walking downhill could never turn away from a
  neighbour. The collision response was silently dead for half the colony. It is
  an explicit flag now: the state is known, so it is stored rather than inferred
  from a measurement.
- **The solar system left frames behind.** `get_diff` cleared the braille grid
  and the two per-cell arrays but never called `canvas.clear()`. `Canvas::commit`
  *swaps* its two surfaces rather than clearing the one it hands back, so each
  frame was painted onto whatever the surface held two frames ago. Cells inked
  last frame and not inked now were never written as cleared, so they survived —
  and the diff, which compares the two surfaces, never mentioned them.
  Frame 2's diff was still *correct*, which is why the damage took three frames
  to appear and why a two-frame regression test passed against the bug.
- **`[terrain]` had no body.** Both the glyph and the colour were functions of
  depth alone, so every column below its own surface drew identically and the
  picture was one smooth vertical ramp under a slightly wavy top edge. That is
  not a landscape and not a cross-section either; it is a gradient with a border,
  and it is why the effect was reported as unreadable. The glyph is now a
  two-dimensional field sampled at (column, depth) — banded, because the vertical
  period is the shorter of the two and that is what says sediment rather than
  static — while the colour keeps the depth shading, which is the one thing in the
  frame that is a clean function of a single quantity.
- **The DVD logo flickered, and the colour was not why.** The last release moved
  the default from recolouring on every wall hit to every third, on the theory
  that a hue change across a solid 30x7 slab is a strobe. It is a large
  simultaneous change; it was not the flicker. The flicker is the speed: a
  braille glyph *is* its bit pattern, so moving the logo one dot rewrites nearly
  every cell it passes through, and at 18 cells a second that is 36 dot-steps
  against 60 frames — a whole-logo repaint on three frames in five. The cap is
  now 5, taken from the reference implementation's measured pace rather than
  from a ratio, and recolouring on every bounce is the default again because wall
  hits are eight seconds apart instead of one and a half.

  The cap was 18 because of `24 * 23/30`, which holds
  `cells_per_second * width` constant — not a quantity that means anything, and
  explicitly not the time to cross one logo width, which would want 31. A test
  asserted 18 while its own comment claimed to preserve something it did not.

### Changed
- **`[cube]` is an X-ray wireframe.** Hidden-line removal is gone: all twelve
  edges are drawn, with the three facing away dimmer than the three facing
  toward you. The dimming is the cube's own depth ramp rather than a per-edge
  flag, so it stays one function of `z` — but the ramp needed extending, because
  it was normalised against the *visible* faces' own planes and so the farthest
  front-facing face sat at the top of the range. A far edge clamped past it landed
  on the same ramp stop: the brightest far edge and the dimmest near edge measured
  **114.9 and 114.9**, identical. The range now reaches 15% of the way past the
  front faces to the farthest face of any kind.
- **`[life]` draws every live cell with one character.** `@`, for every age from
  0 to `MAX_AGE`. Age is still shown, by colour, which was always continuous.

  This is the third version in one direction, each removing a way for the
  character to move: random per generation, then a continuous ramp of the age,
  then three bands of three, now a constant. Age was being shown *twice* — once
  by colour and once by character — and only one of the two was wanted.

  Corner markers on the cube still skip the hidden corner, deliberately: the
  marker is flat white, so marking the far corner would make it the brightest
  thing on screen while its three dim edges recede.
- **`[plasma]` is sixteen glyphs and no longer repeats.** The three moving time
  terms are now `1`, `√2` and `√3` over a common factor of 2, so no interval
  advances all three by a whole number of cycles. The old set was all rational
  and had an exact period of `8π` — 25.13 seconds, after which the screen
  returned cell for cell. The factor of 2 is there to keep the field's existing
  ordering of speeds; the unscaled set would have made the slow ripple the
  fastest term.

  Sixteen steps, all sixteen used. The busiest went from 22.4% of the screen to
  8.8% and the sparsest from 0.20% to 3.7%, measured over 200 frames at 200x50.
  The re-spacing is a boundary table fitted to the field's measured quantiles
  rather than a gamma curve: `t^g` can only tilt a bell, and the only thing that
  flattens one is an S-curve.

  Sixteen *reliably separated* ASCII bands is not available — below about a
  quarter-cell of ink, ASCII is a crowd of one-mark glyphs, and the ties in the
  ink table go from three to nine. Sixteen steps is what the technique carries.
- **`[ink]` has named colour presets**: `green`, `orange`, `blue`, `magenta`,
  `ice`, `amber`. Every one runs from near-black to exactly `rgb(255,255,255)`
  at the top — the tinted-white top was reported twice in this project, so there
  are two tests on it. `green` is the existing phosphor ramp by identity, not a
  copy, so the default is unchanged and the two cannot drift apart.

  `palette` takes precedence over the `colors` list. That is forced rather than
  chosen: the default `colors` is always populated, so "the list wins when set"
  would mean "the list never wins", and a generated config could never be told to
  use orange. `palette = ""` is the way back to the list.
- **`--random`** seeds a run from the operating system, so a launch looks
  different every time. `--seed` wins if both are given, and says so rather than
  quietly ignoring one: an explicit seed is a request to reproduce something, and
  picking the random one silently would make `--seed 1234 --random`
  unreproducible while appearing to honour the seed.

  It is not `--shuffle`, which has meant "play the playlist in random order" for
  some time. A flag that reseeds every effect cannot share a name with one that
  reorders a list; the collision was found by trying to add it.
- **`solarsystem` no longer rotates.** `camera_speed` defaults to zero, so the
  viewpoint is fixed and the motion is inside the system. The option is kept
  because the swing is what makes the projected orbits change orientation, and
  that is the 3D cue rather than a flourish.

### Known
- **`life.step_generation` scans a `HashMap`** and costs about 3.94 ms on its
  `update x4` worst case at 400x200, which is the only reason `life` is over the
  2 ms budget.
- **`mandelbrot` render is 11.9 ms at 400x200**, and its cost is very close to
  linear in `max_iterations` because nearly every interior pixel spends the whole
  budget discovering it never escapes. A region-marking rewrite is where the real
  win is.
- **`termzzz constellation` is now `termzzz solarsystem`.** Renamed with no
  deprecated alias, for the same reason as `ascii` → `ink`: the old name says
  star chart and the effect is an orrery, so keeping it would have meant
  shipping a solar system under a label that describes something else. An old
  `[constellation]` config section is **silently ignored** and the effect runs at
  its defaults; rename it to `[solarsystem]` to keep your settings. An old
  `effect = "constellation"` playlist entry is reported on stderr with the list
  of names this build knows, rather than dropped in silence.
- **`[terrain] scroll_speed` now measures something else.** It was a vertical
  offset applied to a field sampled per cell; it is now how fast the landscape
  travels sideways past the camera, in cells of the field per second. The default
  of 0.9 is kept because it lands in the right band for the new model, but the
  two are not the same measurement and there is no way to convert one into the
  other, so an old value will look different rather than wrong.

### Fixed
#### The audit
Fourteen effects were audited against what they actually draw. Most of what came
back was not a matter of taste, and several of these effects had never once shown
their output to anyone.

- **The donut was drawing a fifth of itself.** The angle tables were built at a
  fixed 0.02 rad step, which is only correct at one sample count, and indexed
  absolutely — so any terminal under 50 rows got a strict prefix of the table.
  At 40x12 it swept 23.6% of a turn, and at the 6-row minimum the screen was
  *blank*. The missing wedge is fixed in object space, so it was a permanent
  chunk of the torus that is never drawn and tumbles with it. Now a full turn at
  every size. Its colours are also a perceptually-uniform ramp rather than a
  fixed hue set reordered by luminance, which is what scatters hues across the
  brightness range; four of twelve shades had never been drawn at all
- **The maze never showed you a finished maze.** It reset on the frame after the
  carve completed, so a completed maze existed for exactly one frame. Underneath
  that, the depth-first search popped its stack twice at a dead end and discarded
  the parent node, so it carved **4 of 200** reachable cells rather than 200.
  Generation also never finished on a large terminal — five and a half minutes at
  400x200, longer than its own playlist slot. Now: a six-second hold, a real
  spanning tree, and a rate that scales with the screen
- **The mandelbrot was one flat colour.** Its escape counts were mapped onto the
  ramp with a curve that reached the top at five iterations, and the sampler
  clamps, so nearly every visible pixel was a single `rgb(16, 8, 40)`. A second
  bug compounded it: the palette offset grew without bound, so after about seven
  seconds the entire frame was flat until the camera recentred. It also cut to an
  unrelated point at the depth limit; it now pans to a nearby coastline and pulls
  back over 1.5s. `palette = "ember" | "ocean" | "magma" | "contrast"` is new
- **The matrix grey lines were flashing white**, for two independent reasons. Some
  drops were drawn "dark grey *and bold*", and bold on a 256-colour palette index
  is a brightening hint that terminals answer by rendering the bright variant. And
  the back layer's ramp started at near-white while being indexed by absolute
  position, so the pale stretch grew with the window — about 20 cells at 200 rows
  against 2 at 50
- **`--seed` did not work for the matrix.** The character pool was built by
  iterating a `HashMap`, whose order is randomised per process, so the same seed
  produced different rain on every run. The determinism test could not see it,
  because both instances it compares live in one process
- **The DVD logo moved on 86 frames in 600** and said `DUD`. The position was
  integrated correctly in f64 and then thrown away by `as usize` at draw time, so
  the logo sat still for six frames out of seven and then jumped a whole cell. The
  middle letter was a `U`. The colour only changed on a corner, which first
  happens after 97 seconds, so it was effectively one colour forever. Now a
  five-row block-letter `DVD`, sub-cell placement on both axes, and a recolour on
  every wall hit
- **Fading to black was a hard step, not a fade.** `lerp`'s match only handled
  two `Rgb` values, so `Color::Black` fell into the "endpoints win" fallback and
  `lerp(Black, ink, 0.2)` returned `Black`. Code that reads exactly like a working
  gradient and silently did nothing
- **`cargo clippy --all-features` is now clean**, as is `cargo fmt --check`
- **A user config can omit any single key**, not just whole sections. Only 2 of
  17 effect option structs carried `#[serde(default)]`, so a hand-written
  `[plasma]` section naming one key was a startup error. This mattered more than
  it sounds: `--print-config` writes every key to disk, so adding any new setting
  would have broken startup for most users
- **A playlist no longer swallows names it cannot resolve.** An unresolvable
  entry was filtered out in silence and the playlist simply came out shorter. It
  now reports them on stderr with the list of names the build knows

### Changed
- **Effects were rebuilt for legibility**, and the ones that were quietly broken
  are the substance of this release:
  - `life` drew 32 halfwidth katakana, and the glyph was a *uniform random pick
    redrawn every generation* — it depended on nothing, so the eye had nothing to
    track but the absence of glyphs. Cells now have an age and both glyph and
    colour follow it
  - `boids` had two of its three flocking rules attenuated to ~3% of the
    separation force by their own normalisation, so it read as a gas; the screen
    was cleared every frame so there were no trails; and the colour formula
    could not produce the white its own comment described
  - `plasma` drew a static `*` on every cell, and its fourth radial term was
    anchored at the origin, whose vertical gradient is zero at the top and
    maximal at the bottom — which is why the bottom rows flashed
  - `terrain` rendered one frame and then returned an empty diff forever, the
    only fully static effect here, with the shortest playlist slot of the sixteen
  - `constellation` covered a third of the screen in connection dots and rebuilt
    its whole graph every frame, so the mesh shimmered; star count was a flat 65
    at every size, so at 6x6 twenty-nine of sixty-five stars were silently
    overwritten
  - `cube` destroyed all eight corners — each of the three edges meeting at a
    vertex overwrote the other two — and ran off the top and bottom of the screen
    at some `cube_size` values
  - `crab` moved one column every 21 frames, had two walk poses, and its
    left-facing frames were not mirrors
  - `ink` (see the rename below) drew a top ramp stop that was 6.6% saturated
    green, so nothing was ever white
- **A shared `GlyphRamp`** now backs the character sets for eight effects, with
  named presets. Read its module doc before choosing one: the *ordering* is the
  whole point, and a ramp that puts the light characters on the ones covering
  most of the screen makes most of the frame the brightest thing on it
- **A `QuadrantMask` renderer**, for 2x2 per cell in two colours. `▀` and `▌`
  cannot be combined — a cell has one foreground and one background, so a vertical
  split spends both — and this is what a moving logo needs
- **Every effect's options struct carries `#[serde(default)]`**, so a config file
  may omit any key and inherit the real default
- `maze` gained `hold_seconds`; `dvd` gained `slope`, `color_change` and a
  block-letter default logo; `mandelbrot`, `plasma`, `terrain`, `boids` and
  `constellation` gained tunable glyph ramps and other options. All are listed in
  `--print-config`
- **`Attribute::Bold` is no longer used as decoration.** Eight effects had it,
  often on every cell. Many terminals treat bold on a foreground as a brightening
  hint, which pushes a ramp's hot end toward white — the same class of bug that
  made eleven of the sixteen effects look washed out
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
