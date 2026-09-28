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
termzzz terrain     # A side-view landscape: two ridges, parallax, sky
termzzz solarsystem # 3D orrery: tilted orbits, real periods
termzzz dvd         # The real DVD wordmark, bouncing, in braille
termzzz newton      # Newton's method basins, hue by which root, ink by iterations
termzzz blank       # Blank screen
termzzz ink         # Interactive generative field you pour ink into

# Run a timed playlist of effects
termzzz --playlist matrix,dvd,plasma
termzzz --shuffle

# Reproducible runs: every effect that uses randomness honours the seed
termzzz matrix --seed 1234

# Or the other way: a launch that looks different every time. --seed wins if
# both are given, and the run says so rather than quietly ignoring one.
termzzz --random

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
- **Effects**: 20 screensavers and visual effects, plus a playlist mode
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
   `plasma` is at **2.01 ms — straddling the 2 ms line**, and the two halves of it
   moved in opposite directions. The field rewrite took its ANSI volume from
   476 KB to **265 KB**, because a field with a diagonal term in it has more
   structure and rewrites fewer cells. Its render went the other way, to 1.48 ms,
   because the diagonal term is two more sines per cell on top of a glyph remap
   that is now a lookup table rather than a binary search. It was 2.05 ms before
   this round and it is 2.01 ms now, so the field rewrite bought about 40 µs and
   the diagonal cost about 400. **That byte count is not a new low.** It emitted
   466 KB before this round, because a field peaked in the middle was already
   rewriting most of the screen every frame. Measure the old code before
   attributing a byte count to a change, and re-run `frame_times` before
   believing one — a single run of it read 2.50 ms for this row against 2.01 ms
   on the next two, which is contention from the release build in the same
   command and not a property of the effect.
   **`flyover` is now over it too, at 2.86 ms and 351 KB**, and unlike the
   others that is a landscape render rather than a pathological case. Splitting
   the render by phase: the ray march is 1.47 ms of it, at 800 dot columns and
   about 28 noise samples each; the tile transpose, the braille dither and the
   per-cell paint are 0.96 ms between them, and the rest is the canvas diff. Two attempts to improve it failed and both are worth
   knowing about, because they were confident and wrong: a tiled transpose for the
   scattered writes changed nothing, and an `exp` lookup table for the fog changed
   nothing either. The march is the cost and the only levers on it are the step
   growth and the draw distance. `ants`, `physarum` and `ripple` all came in
   *under* budget, so the gate is not simply worse than it was — it is one more
   row on a list that was already red.
   `terrain` is at 373 µs and 16 KB at 400x200, from 1.3 ms and 554 KB before the
   height-field rewrite — because the surface costs one noise sample per column
   rather than per cell, and unchanged sky cells drop out of the diff
2. **More effects on the sub-cell renderer.** `starfield` and `flow` want
   braille (line art, one colour per cell); `physarum` and a Gray-Scott
   reaction-diffusion want half-block (smooth colour). Each is roughly 150-350
   lines now that the renderer exists. Note that `cell.bg` only reached the
   terminal very recently, so half-block work is the first thing to have actually
   exercised that path.

   **Partly done, and the two that landed changed the shape of the question.**
   `physarum` shipped, along with `flyover` (braille), `ripple` (half-block) and
   `ants` (cell grid). What that round established is that the renderer is not
   what decides which effect is cheap, and the four things below are worth more
   than the remaining list of names.

   - **How many colours a frame uses is a bandwidth setting, not a taste one.**
     The encoder emits a colour only when it differs from the last one written, so
     a smoothly interpolated field is a colour change in every cell. `ripple` was
     emitting 2.79 MB a frame and `physarum` 410 KB, and they are now 73 KB and
     75 KB. `ripple` also had to be made a *coarser pattern* — its first wave
     number put a ring every eleven cells, which is a colour step every two of
     them. **Quantising the ramp alone moved the byte count four percent**, which
     is the number to remember: the changes were coming from the spatial frequency,
     not the depth of the ramp. Both are now documented options (`levels`).

   - **A model with a scale in it has one parameter that decides whether it
     works, and it is not the one you reach for.** Physarum's sensor *distance* is
     decisive — 1.5 cells gives isolated worms, 9 gives a network — because the
     agents move one cell per step and that is how far ahead they can see. The
     sensor *angle*, which every reference document leads with, barely matters.
     Sweep it and write down what you measured, because the intuition about which
     knob matters was wrong here and would have been again.

   - **A model that grows structure needs a measure that cannot be satisfied by
     its opposite.** Physarum's test wants to know it built a network; a slab and
     a network can each be one connected component over a similar area, so
     connectivity said "100% connected" about a picture of three solid bands. The
     boundary-to-area ratio is what tells them apart, and coverage is the second
     half. Both are asserted, and the first version of that test is written up in
     the test's own comment because it is the clearest example in the crate of a
     metric agreeing with a broken picture.

   - **Where a walk can go, the highest point is often off-screen.** Flyover's
     march returned the highest point its ray touched, which is the standard
     heightfield silhouette, and it is *always* below the last row: a camera flying
     eleven units above the ground cannot see the ground under it. Every column
     reported empty. What a height field wants is the *profile* — the depth at each
     row — and the per-row table is also what the fog needs. Three of flyover's
     camera bugs (`up` was `right × forward`, the roll rotated in the wrong plane,
     the pitch was counted twice) drew plausible pictures or none, and none of
     them is visible to a test of the shape. Recompute the projection by hand from
     the definition in a test; that is the only thing that catches it.

   Still on the list, unchanged: `starfield`, `flow`, and Gray-Scott.

3. **`newton` shipped, and it is the second fractal in the crate.** Worth reading
   before adding a third, because the design question and the cost question turned
   out to be the same question and both answers were the opposite of what I
   expected.

   The idea: iterate `z <- z - p(z)/p'(z)` from every cell, colour by *which root*
   each one reaches, shade by how many iterations it took. The mandelbrot colours
   by a single scalar, so a sequential ramp is the right instrument for it and
   every preset in `render/palette` is one. This needs **categorical** colour, and
   the crate had none — hence `oklab_hue` and `perceptual_distance` in
   `palette.rs`, and `INKED` in `glyph_ramp.rs`.

   **The cost is 3.45 ms at 400x200 — over the 2 ms line, and a third of the
   mandelbrot's 11.50 ms.** That is one more red row, and it is on the list
   deliberately: the picture is a categorical one the crate could not draw before,
   at a terminal eight times heavier than the budget assumes. At 200x50 it is
   0.43 ms and at 80x24 it is 0.08 ms, so the ordinary sizes are free.

   Four things came out of it that generalise past this effect.

   - **A const generic was worth a third of the frame.** With the degree as a
     runtime value, both loops in the iteration have a runtime trip count, so LLVM
     unrolls neither and the effect ran at 65 ns/sample against 29 for identical
     arithmetic with the degree fixed at 3. Passing the degree as a const generic
     took 5.25 ms to 3.45 ms. **Before blaming arithmetic for a slow effect, check
     whether the loop bounds are constants.** The iteration counts are 3, 4 and 5,
     which is as close to a compile-time special case as a config value gets.

   - **Two optimisations that looked obviously right and were not, both
     measured.** Testing `|p(z)|` instead of the nearest root is the standard cheap
     test and is *worse on both axes* here — 7.02 iterations against 6.02, 37.5
     ns/sample against 28.9 — because it needs more iterations to reach an
     equivalent distance. And hinting the previous iteration's root, exploiting
     that a basin is sticky, came out at 2.81 and 2.46 ms against 3.13 and 2.39 for
     the plain version: inside the ~30% run-to-run noise, with identical iteration
     counts and *zero* root disagreements. Not worth the branch.

   - **A measurement written down on a noisy run was backwards, and nothing
     noticed for a day.** The file's docs claimed lowering the iteration cap makes
     the effect *slower*, mirroring the mandelbrot where the cap is the dominant
     cost. Measured properly, interleaved to average out machine drift:

     ```text
     cap   render    mean iters   never converged
       6    4.81 ms      4.95          43.8%
      12    5.95 ms      6.06           7.8%
      24    6.16 ms      6.29           0.36%
      48    6.21 ms      6.31           0.00%
     ```

     The cap is a **quality** setting here, not a performance one: its entire
     range is worth 22% of the render, and the cheap end costs **44% of the frame
     its correct colour**. The reason is the *shape* of the distribution —
     iteration counts cluster around six with a thin tail, so almost every sample
     ends on the early-out and the cap only ever affects the few that would have
     gone on longest. The mandelbrot is the opposite because its interior pixels
     all run to the cap. **The same knob means opposite things in two effects, so
     "this is the expensive one" does not transfer.**

   - **OKLab is not scaled to 0-100, and I set a threshold of 30 against a
     quantity whose entire useful range is 0.6.** Every colour-separation
     assertion in the first draft failed while looking like it was testing
     separation. The numbers: about 0.02 is a just-noticeable difference, 0.1 is
     comfortably different, 0.3 is different categories, 0.5 is about as far apart
     as two colours get (saturated red vs green measures 0.52). A categorical
     palette wants its closest pair above ~0.15. This is now in
     `perceptual_distance`'s doc comment, because it is a thing the next reader
     will get wrong in exactly the same way.

   And the reason the wheel is OKLab rather than HSV: **HSV holds *value* fixed,
   which is not perceptual lightness.** At S=0.85, V=0.95 the six HSV sector
   anchors come out with OKLab lightnesses spread over 0.47 — nearly ten times a
   just-noticeable difference, and *larger than the separation between adjacent
   hues*. So a set of HSV colours told apart only by hue is really being told
   apart by brightness, and when the effect is also using brightness for
   something (here: the iteration count) the two channels fight over the same
   signal. Constant L and constant C fixes it by construction: minimum pairwise
   distance 0.282/0.211/0.172 for degrees 3/4/5, against 0.475/0.121/0.181 for
   HSV. **Note that HSV is *better* at three and worse at four** — 90° apart lands
   on azure and violet — so "the HSV wheel is fine" is not a thing to assume.

   Still on the list, unchanged: `starfield`, `flow`, and Gray-Scott.

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
effect that wants per-dot colour has to keep a parallel per-cell colour array.
`solarsystem` does that, and pairs it with a depth buffer -- without one, the first
thing drawn wins every contested cell, and since it draws the sun first, a planet
crossing in front of it would vanish for the length of the conjunction. `dvd` needs
only the colour array: a flat single-colour silhouette has nothing to occlude, and
the second colour `quadrant` gave it was buying nothing but a black background
painted behind every edge cell.

There is also a shared `GlyphRamp` in the same directory, and four effects draw
their characters from it: `cube`, `donut`, `plasma` and `terrain`. (`life` used
to and no longer does — a live cell is one constant character now, so it has no
ramp to draw from.) (`crab`
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

**`DEFAULT_SEED` now means "unset", not 42.** This is the single most important
thing to know about seeding in this crate, and it is recent. `Config::randomise_seeds`
gives every effect whose seed is still at `DEFAULT_SEED` its own distinct draw at
startup, so an unseeded run is different every launch — which is what a screensaver
is for. `--seed <N>` still pins every effect. There are fifteen seeded effects now;
`cube`, `donut` and `plasma` gained seeds in this round and previously had none, so
"all the effects are random" was not true without them.

The shortcut is real and named here so nobody has to rediscover it: the honest
shape is `Option<u64>` on fifteen option structs, and it was rejected because it
pushes "was this set?" into every effect's constructor, where an effect rebuilt on
a terminal resize would draw a *new* seed and change the picture under the user.
Resolution has to happen once, on the config, before anything is built. The cost is
that a config pinning `seed = 42` is the same as one that omits it, and both mean
"give me something different".

That in turn **dissolves** the `--print-config` trap documented elsewhere in this
file. A generated config used to pin whatever the defaults were when it was
generated, which is how a 60x-too-slow donut rotation survived a default change in
this project. For seeds that is now harmless, because 42 pins nothing.

Determinism is done: every effect that uses randomness now carries a `seed` option
and a seeded `StdRng`, `--seed <N>` overrides all of them, and
`tests/effect_contracts.rs` asserts both reproducibility and seed sensitivity for
every effect. Because they became comparable, that suite also found that five of
them advanced by a fixed step per rendered frame — `maze`, `boids`, `crab`,
`pipes` and `constellation` (now `solarsystem`) ignored the speed keys —
which is fixed too.

Not worth doing soon: further performance work at ordinary sizes. Nothing is
dropping frames at 200x50 — the worst effect there uses about 10% of a 60 fps
budget, and `frame_times` now passes its own 2 ms check at that size outright.
At 400x200, which is an eight-times-heavier terminal than that budget assumes,
only `mandelbrot` and `life` are over: `life` on its `update x4` worst case at
high `--speed`, and `mandelbrot` on render, which is item 1 above.

`plasma` is the one to watch. It has been over and under again inside one round —
2.05 ms from the glyph remap, 1.99 ms after the field rewrite — and both numbers
are the *sum* of two effects that move in opposite directions. Its render is
dominated by sine count and its encode by how much of the screen changes, so
making the field more structured raises the first and lowers the second. Read both
columns before concluding anything from either.

Two numbers in the frame table went *up* in the earlier audit, and both are
correct. `mandelbrot` went from 31 KB to 215 KB of escape sequences per frame at
400x200, and `matrix` from 63 KB to 67 KB. Each was previously cheap because it
was broken: the mandelbrot was showing a flat wash with no detail to change, and
the matrix drop tails were all one saturated colour because a truncating cast
destroyed the fade. A byte count is only meaningful next to what is on screen.

Two numbers then went sharply *down*, and both are worth understanding rather
than just noting. `terrain` fell from 554 KB to 11 KB because the height-field
rewrite leaves the sky as a space that matches the cleared cell, so unchanged sky
drops out of the diff entirely — the effect got *more* structured and 50x cheaper
to emit. `dvd` is 596 bytes at 400x200 for the same reason in braille: a cell with
no raised dot is not written. **Not writing a cell is the cheapest rendering
optimisation there is**, and both of these found it by accident rather than by
looking for it. Check whether an effect is writing cells that did not change before
optimising anything it computes.

**And that terrain number then went back up, and then all the way back down
again.** Giving the ground a 2D grain took it from 11 KB to 24 KB at 400x200 and
519 changed cells a frame to 1,127, because a body whose glyph was a pure function
of depth barely changed between frames and a picture that changes 519 cells a
second is a picture not doing anything. That was round two, and the user called
the result "still shit" — correctly, and for a reason no byte count would have
shown. The effect had been a *cutaway* the whole time, which is not what anyone
means by terrain.

Round three deleted the grain entirely and added a second ridge. Back to 526
changed cells a frame, and the render's 54,000 noise samples a frame down to 800
— two per column, which is the number the height-field rewrite bought and the
grain spent. **The lesson is not that grain is expensive.** It is that a
measurement of efficiency is not a measurement of whether the thing works, and
two rounds of making this effect measurably busier moved it further from what was
asked for. The frame table cannot tell you whether an effect reads.

The remaining 526 is not 77, and that is deliberate: the near fill's shading is
measured from each column's *own* surface, so a one-row surface move re-shades the
whole column. Shading by row instead would be four times cheaper and would paint a
hilltop the same colour as a valley floor, which reads as a mistake. Both numbers
are at the call site.

**The other half of that lesson is not to trust a byte count in the other
direction either.** Do not assume one moved because of your change: plasma's went from 466 KB to 476 KB *this
round* and I attributed a six-fold increase to the new glyph calibration before
measuring the pre-round binary. The old effect was already the second-largest
emitter in the crate, because a field peaked in the middle was already rewriting
most of the screen every frame — the histogram being lopsided did not make the
cells static. **`git show <commit>~1:src/<effect>/effect.rs` and measure.** It
takes a minute and it is the difference between optimising the right thing.

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

### A mirrored ramp is not a brightness ramp

Nearly every ramp in `palette::presets` is **mirrored** — it climbs to a peak and
comes back down — so that `sample_wrapped` has no visible seam. `Palette::sample`
returns `stops[len - 1]` at `t = 1.0`, and on a mirrored ramp that is the
*second-darkest* stop, because the mirroring is symmetric. So any effect that
takes a preset whole and treats it as a brightness scale puts its **top of scale
in near-black**, with the ramp's brightest colour sitting unused in the middle
where the value that reaches it is a coincidence.

Four effects were built on that. `ants` with `MAGMA` measured out as:

| flips | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 |
|---|---|---|---|---|---|---|---|---|
| rgb | 93,23,124 | 200,75,115 | 253,170,129 | **253,239,186** | 253,235,185 | 253,159,119 | 192,65,118 | **81,18,124** |

It peaks at `t = 0.5` and falls back to dark. `physarum` and `ripple` did it with
`OCEAN` (top of scale `0,10,50`, a near-black navy) and `flyover` with `DEPTH`
(`16,8,40`). `mandelbrot` and `donut` are correct and are the reason the bug
survived so long: they use the *cycling* sampler on purpose.

`Palette::truncated_at_peak` is the fix — cut the ramp at its brightest stop, by
Rec. 709 luma rather than by channel maximum, because `DEPTH` peaks in blue and a
channel max would find the brick red at its end. Truncate **before** `expand`, or
the ascending half's stops end up spread over the first half of the quantised
table and the descending half's over the second.

**A byte count cannot predict which way this moves.** Measured at 400x200:

| effect | before | after | |
|---|---|---|---|
| flyover | 350,808 | 207,428 | **−41%** |
| physarum | 74,814 | 74,362 | −0.6% |
| ants | 1,105 | 1,097 | flat |
| ripple | 73,083 | 85,733 | **+24%** |

Ripple got *more* expensive: its top of scale is now a pale colour that genuinely
differs from its neighbour, so more cells count as changed. This is the
bandwidth note below in its purest form — **how many colours a frame uses is a
bandwidth setting, not a quality one** — and it is why a fixed palette is never
obviously the cheap one.

### The corner of the DVD logo is a float accident

`ColorChange::Corner` is documented as the easter egg the reference's README jokes
about, "it could hit the corner if you look at it long enough", and
`DvdOptions::color_change` works out that this is about every 95 seconds at 80x24.
**That figure is wrong by an order of magnitude.** A corner is defined as both
axes reversing on the same frame, the bounce periods are `2*max_x/vx` and
`2*max_y/vy`, and with the shipped 30-cell wordmark those are 8.3 and 5.7 seconds —
a ratio of 25:17 whose least common alignment is 142 seconds away, *and* float
accumulation means the two walls are then crossed on adjacent frames rather than
the same one. Measured over 1,000 seconds of real simulation:

```text
terminal   x-hits  y-hits  corners
80x24        240     350       1
200x50        70     139       0
400x200       32      31       0
```

So the corner is not a rare easter egg, it is **effectively unreachable**, and
`ColorChange::Corner` ships a mode that a user will never see fire. Two things
were tried to fix it by construction and both failed, which is worth knowing
because both were confident:

- **Locking the periods into a rational ratio** by solving `vy` for
  `T_x/T_y = (n+1)/n`. Larger `n`, which should be *closer* to alignment, made
  corners monotonically **rarer** — n=4 gave one per 17s at 80x24, n=32 and n=64
  gave none in 1,000s. Approximate commensurability is not commensurability.
- **A proximity window**, calling a corner any two wall hits within N frames.
  This does work and it is a real option: at 80x24, slack 3 gives one per 100s and
  slack 10 one per 34s. It does *not* rescue a large terminal — 400x200 is still
  zero at slack 10, because the two periods differ by 2.66s per cycle so the
  phase walks through a full cycle every ~25 minutes.

What *does* produce corners reliably is a small logo on a large terminal, where
`logo_height` is one cell and the two periods come out equal (66.3s each,
measured) so every bounce is a corner. `dvd`'s test suite uses that as its
corner-rich fixture, and the reason is written down at the fixture rather than
left as a magic screen size.

The wave rides on this. `DvdOptions::corner_wave` is independent of
`color_change` — `Never` still throws waves — because tying it to
`ColorChange::Corner` would mean most configurations never see it. And a wave is
drawn by **walking the ring parametrically**, not by scanning the annulus's
bounding box: at radius 200 the box is the whole 400x200 screen, which is 80,000
`hypot`s to draw about 1,250 cells, and the cost grows with the square of the
radius. One sample per cell of *arc* is O(circumference) and the ring stays
connected at any radius.

### Physarum does not converge, and "stable" is satisfiable by nothing

`DECAY = 0.90` is a steady state statistically and never a fixed point: the
diffusion erodes veins the agents keep rebuilding, slightly elsewhere each time.
Churn — the fraction of marked cells whose marked-state changed in 30 steps — at
4,000 steps, seed 3:

| decay | 60x20 | 200x50 | 400x200 |
|---|---|---|---|
| 0.90 | 0.35 | 0.41 | 1.40 |
| 0.96 | 0.25 | 0.25 | 0.55 |
| 0.99 | 0.03 | 0.07 | 0.14 |

0.35 is a picture redrawing a third of itself twice a second, which is not
stability and does not look like it. Raising decay fixes it and costs coverage —
at 0.99 permanently the 60x20 board is 46% covered, past the 25% that
`the_agents_build_veins_rather_than_a_slab` rejects. So the anneal is a *phase*,
not a new default: grow at the configured decay, ramp to 0.995 over 2,500 steps,
hold, fade, re-seed. Measured, that reaches churn 0.067–0.109 across board sizes
and keeps coverage under the slab threshold. `CHURN_SETTLED` is 0.15 because that
is inside the measured gap between 0.11 and 0.35 — **the threshold is a
measurement, not a round number.**

Two things about where this lives, both learned by getting them wrong:

- **The cycle is in `advance`, not in `step`.** It was in `step` first, and it
  broke three tests that drive `step` directly — including the edge-ratio
  assertion, whose ratio fell to 0.10 because the anneal had thickened the field
  into the very slab that test exists to reject. `step` is `pub` and is what
  `examples/physarum_sweep.rs` drives, so it has to stay the model those were
  written to measure. The effect's *behaviour over time* is the frame path's
  business.
- **Convergence has two ways to be satisfied by nothing, and both fired.** An
  *empty* field has zero churn, so a churn-only detector declares victory the
  instant the agents stop depositing and then holds a blank screen for ever —
  hence the coverage floor. And a field that has **not started** is the same trap
  from the other side: the five blobs `seed_trail` lays are stationary and
  fully covered, so the detector fired on the first check at step 30 and held
  five blobs for four seconds before growing anything. Hence
  `age_steps >= ANNEAL_STEPS` before any verdict, with both incidents written at
  the constants.

The trail value above which a cell counts as carrying trail is `TRAIL_FLOOR`,
absolute at 3.0, and it is the *same* floor the shape tests use. That is
deliberate: "settled" has to be measured on the same set of cells "is a vein" is,
or the effect can satisfy one and not the other.

### Fire's ramp was miscalibrated before it was misordered

The hand-written nine-arm `match` was not monotonic in ink — it ended
`'@' -> '%'`, and a per-cent sign is sparser than an at-sign, so the hottest part
of a flame was drawn in a *lighter* mark than the part below it. `FLAME`
(`" ▁▂▃▄▅▆▇█"`) fixes that, and the lower eighth blocks are the right instrument:
eight steps instead of three, one-eighth increments of the same bar, anchored to
the bottom of the cell so a row's *height* is its value.

That required widening `is_ambiguous_or_narrow` from `█` alone to the whole
`U+2581..=U+2588` span. The old arm allowed the top of `BLOCKS` and rejected the
seven characters directly below it, which was an oversight in the range list
rather than a decision about them — all eight are East_Asian_Width = Ambiguous,
the class the function already accepts. `no_preset_contains_a_character_that_
would_shear_a_grid` is what notices when a preset reaches past the end of a range.

Then the *calibration* turned out to matter more than the ordering. The glyph was
indexed on the bitmap's full 0..255 while the colour was indexed on 0..`HOTTEST`,
and the bitmap's distribution is heavily skewed to the cold end. Measured on a
settled fire at 400x200: **75.7% of all cells are intensity 0–9 and 0.01% reach
190.** So a 9-entry ramp spread over 0..255 put its first step at 16 and its last
at 240 — a quarter of the *visible* flame drew as a single `▁`, and `▇` and `█`
were never drawn at all on any terminal. One scale for both fixed it: the hottest
cell in a settled fire measures 193, above `HOTTEST` at 187, so the top of the
ramp is now reachable. **`flyover` has the same skewed-input problem and is
already noted in this file as the most expensive effect in the crate.**

The leading space in `FLAME` looks like the bug the glyph ramp documents — "a
ramp for a filled region must not begin with a space, because the sparsest step
lands on the row at the top of the region". A flame is not a filled region:
intensity 0 is the gap between two tongues of flame and the empty air above them,
and it is the majority of the frame. Said so at the call site, because the rule
looks like it applies and does not.

### Tests that pass for the wrong reason

A second pass, driven by watching the effects run rather than reading them,
produced five tests that passed *against the bug they were written to catch*. That
is a distinct failure from a missing test and it is worse, because it reports
coverage that is not being provided. Four patterns, all from real incidents:

**A test that stopped testing its subject.** `the_ground_gets_denser_and_darker_
with_depth` drove the ramp *function* rather than the render, so after the glyph
stopped being a function of depth it went on checking that a ramp is ordered —
which was never in doubt. Meanwhile the crab's shadow test asked for shadows on the
bottom row, which on a sloping seabed is exactly where the sand is not, so it had
stopped checking anything at all without failing. After changing what a field
encodes, ask what each test that mentions it is still measuring.

**A confounded metric.** The obvious way to ask "does the ground body have
horizontal structure" is to count how many adjacent cells in one *row* differ. Two
cells in the same row sit at different depths wherever the surface is uneven, so
they differed with no structure at all — the metric measured the silhouette. The
version that works compares at *constant depth*. Then it failed again, because it
took the surface from a fresh effect at offset zero while rendering at offset 1.5,
so each column was sampled at a different depth. When a measurement passes against
the code it was written to reject, suspect the sampling, not the threshold.

**A proxy that the defect can satisfy.** `the_picture_changes_on_almost_every_frame`
asserted the DVD logo changes on most frames, as a proxy for "moves smoothly". A
proxy like that is satisfiable by moving a whole cell at a time on rare frames,
which is precisely the staircase it was written to catch. Assert the thing
directly: no single frame moves the logo more than 0.2 of a cell. Where a proxy is
unavoidable, say in the test's doc which defect it cannot see.

**A search over the wrong range.** The terrain scroll test cross-correlates for the
shift at which the ground matches itself, and first searched `0..=4` and reported a
best of zero. That zero was the edge of its own range, not a fact about the
picture: the offset is *added* to the column, so the same material is found further
left. Check that a measured optimum is in the interior of the range you searched.

A sixth kind is a **derived table with no test against its source**. `plasma`'s
glyph remap was a binary search over its boundary table, and replacing it with a
lookup meant building a 65,536-entry index table and a sixteen-entry segment
table at compile time. The two existing tests on that function both still pass
against a table quantised so coarsely that it flattens the middle of the curve —
a flattened curve is still monotone and still reaches both ends, which is all
they check. Deriving something at compile time does not make it a constant like
any other; it makes it a second place the same thing is written down. Compare it
against the code it replaced, with the tolerance being its own resolution.

Two of the five also came from tests that could not see the thing they claimed to.
The precedence between `--seed` and `--random` happens after argument parsing, so
the only reachable test was the whole binary; `resolve_seed` is a separate function
precisely so the decision is testable at all. And the orrery's ghosting needed
*three* frames, not two, because frame 2's diff is still correct — a two-frame
comparison passed against the bug.

### Inferring state from a measurement

The crab's collision response gated the turn-away on `position.1 < ground`. That is
a float comparison against a *sampled* value, and the sample moves. On a flat
seabed the ground never changed under a crab, so it was free. With a slope, a
walking crab sits a hundredth of a row above or below its own ground depending on
which way it is walking, so it read as airborne — and a crab walking downhill could
never turn away from a neighbour. The collision response was silently dead for half
the colony, which is the same class of bug that had already been chased once in
that file.

It is an explicit `airborne` flag now. **When the state is known, store it rather
than re-deriving it from a measurement**, and be suspicious of any `if a < b` whose
right-hand side is a function of time or position.

Two more from the same year, and the second is the worse of the two because it
looked right while being wrong.

**An event rare enough that it happens ten times in twenty thousand frames is not
something to ask about by measuring something else.** The DVD wave's tests needed
to know whether a wave had been thrown on a given frame, and the obvious way to
find out was to look for a wave with age zero in the list. But `step` ages the
waves *after* it pushes, so a wave thrown on a given frame is already one step old
by the time `step` returns — the probe found nothing, and when the counting was
reworked to count *grew by one* frames it reported ten disagreements where the
truth was ten corners and zero disagreements. `Dvd` now stores
`corner_this_frame`. `Physarum` stores `last_churn` for the same reason: it costs
a full pass over the field to measure, so a test that wanted to know how close the
model was to settling would otherwise reimplement the measurement — and a
reimplementation is a second place the definition lives.

**A probe read after the call it was supposed to precede is a probe that cannot
fail.** The same test counted wave-list growth with `let before = len` written
*after* `update()`, so `grew` was identically `false` and the assertion it fed
was comparing `true` against `false` on every corner. It produced a plausible
disagreement count rather than a compile error. When a "before" value is captured,
check that it is captured *before*.

### The crab's sprite width caps its relief

`crab` is asked to have "eight rows of seabed", and it has them. It does not *show*
eight rows on an 80-column terminal, and the reason is worth knowing before anyone
tunes `seabed_amplitude` or `seabed_period` again.

The sprite is **fifteen columns wide and four tall** (`RIGHT_POSES`, `sprite()`).
For it to look planted rather than floating at one end, the ground can rise at
most about **one row across the sprite's own width** — a slope of 1/15, about 0.067
rows per column. A sinusoid of amplitude `A` and period `P` has a steepest slope
of `2πA/P`, so for `A = 4` that is **P ≥ 375 cells**, which is longer than a wide
terminal is.

So the shipped values are amplitude 21.0 and period 500, which measure 8 rows of
relief and 0.92 rows of drift across the sprite. The trade is forced: **tall relief
or frequent hills, not both.** The colony does climb — measured, row 19 to row
13.8 over forty-five seconds, against 15.1 before — but an 80-column window shows
two fifths of one bank, so the ground rises about a row at a time.

If the crabs still read as bottom-heavy, **the lever is the sprite's width, not the
period.** A smaller crab allows a shorter period, which shows more bank at once.
That is a redesign of the art rather than a tuning change, which is why it was not
done unasked.

### The noise does not reach ±1

Worth stating once, because it was wrong in three separate places in one session
and each time it cost a test failure to discover. `terrain::noise`'s octave noise
was assumed to span the full `-1.0..=1.0` range. Measured over four thousand
samples at three different periods, it spans **0.79**. So anything dividing by
1.0 to "normalise" it puts every value inside the middle four fifths and leaves
the ends of the scale unreachable — quietly, with no error.

It bit three times:

- `terrain`'s `relief` is in rows per noise period, and an amplitude of 3.0
  moved the surface 1.2 rows, which rounds to *one* row on a flat stretch and is
  invisible. 7.0 gave the intended three rows. Both figures have since moved with
  the effect's redesign, which is the other half of the lesson: these constants
  are calibrated against a *measurement*, so a redesign invalidates them and
  nothing but a test notices.
- The crab's `seabed_amplitude` first shipped at 3.0 and its own test measured
  **one row of relief** across eighty columns. Same arithmetic, same cause. It is
  now 21.0, and the period moved with it — see the note on the crab's sprite
  width.
- The terrain body's grain divided by `NOISE_PRACTICAL_RANGE` for exactly this
  reason. That constant and the grain it served are both gone now; the bullet is
  left because the third instance is what makes the pattern worth writing down.

The lesson is not "remember the number", it is that a normalisation constant
derived from a spec sheet rather than a measurement is a guess wearing the
costume of a fact. Measure it, and name the constant so the measurement is
visible at the call site.

## Architecture
See `specs/overview.md` for the project architecture and technical overview.
