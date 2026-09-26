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
- `host.rs`: The running effect, the cycle order, and the `n`/`p` transition
- `render/`: Sub-cell renderers -- braille, half-block, dithering, palettes, wipe
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

Effects advance on `context.delta` and draw in `get_diff`, and the split matters.
An effect that does its simulation inside `get_diff` is stepping once per rendered
frame, so its speed follows the terminal's refresh rate rather than elapsed time
and the global speed keys only change how often it is drawn. Effects that produce
work at a rate rather than a per-frame count — the maze carving a cell, the pipes
growing a segment — accumulate elapsed time and spend it in whole steps. Both
halves are covered by a contract test that renders every effect at 60fps and at
20fps and requires the results to differ.

Randomness comes from `common::EffectRng`, a seeded `StdRng`, never from
`ThreadRng`, which cannot be seeded and would make an effect impossible to
reproduce. Each effect exposes a `seed` option and `--seed` overrides all of them
at once. `common::seeded_rng` folds a per-effect salt into the seed so two
effects configured identically do not replay the same sequence.

Output encoding is the one place that talks to the terminal, and it is built from
crossterm's individual commands rather than from `PrintStyledContent`. That is
not a style choice. `PrintStyledContent` writes the colours, then the attributes,
then the glyph, and then a full reset whenever an attribute is attached -- which
it always is, because attaching one puts a bit in a non-empty set. So every
styled glyph was followed by `\x1b[0m`, and any attempt to skip re-stating the
style for a run of identically styled cells was wrong from the second cell
onwards: those cells were emitted as bare glyphs and painted in the terminal's
default foreground, which is white on a dark profile. `Attribute::Reset` was
worse, because it is SGR 0 and so cleared the colour that had just been set,
before the glyph was drawn -- and the mandelbrot tags every cell `Reset`, so it
came out entirely white.

So the encoder tracks `(foreground, background, attributes)` itself and emits a
cursor move only on a position break, a colour only when it differs, and a reset
only when the attributes change -- with the colours restated after that reset,
never before. The tracked style is forgotten between frames, and each frame ends
by resetting, so what the terminal is left in matches what the next frame assumes.
`Cell::attr` is read as additive only: `Reset` and `NormalIntensity` both mean
"nothing is turned on". The background is on the wire, which is what lets the
half-block glyph carry two colours in one cell.

The tests in `common.rs` include a small terminal model, written from the SGR
semantics rather than from the encoder, and the output is replayed through it.
That is the only kind of test that could have caught the above. The old suite
counted glyphs and cursor moves, and the broken encoder really was emitting every
glyph -- just in the wrong colour, which no count can see.

Two cells of one style now cost one style change rather than one per cell, and a
colour change costs one colour sequence and no reset. Measured at 400x200, that
plus the effect-side palette work took plasma from 824 KB to 483 KB per frame and
fire from 153 KB to 106 KB. Two effects went the other way -- mandelbrot from
31 KB to 215 KB, and matrix from 63 KB to 67 KB -- because they were previously
cheap by being broken: the mandelbrot was showing a flat wash with no detail to
change, and the matrix drop tails were all one saturated colour.

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

## Focus

The loop asks the terminal for focus events (`EnableFocusChange` on enter,
`DisableFocusChange` on restore) because focus reporting is off by default, and
without it there is no way to know whether anyone is watching. The events used to
be folded into `InputEvent::Ignored` and discarded.

Focus is latched in `InputState` rather than read per frame. A window stays
focused across every frame in which no focus event arrives, so a per-frame
reading would be wrong almost always. `InputState` has a hand-written `Default`
because the derive would give `focused` the value `false`, and a screensaver
that believes nobody is watching throttles itself.

While unfocused the loop drops to `idle_fps` and **freezes the simulation**
rather than feeding it a coarser delta. At four frames a second a real delta is
250 ms, which the frame-delta clamp would cut to 50, so the effect would run at a
fifth of its speed and stay there, and a playlist would stall rather than keep
time. Freezing also means the elapsed time is simply discarded: on regaining
focus the loop resets its frame clock, so the first frame back carries one frame
of delta instead of the whole absence. That is what stops the effect jumping.

Throttling rather than stopping is deliberate. A terminal that never reports a
focus change would, under a hard pause, leave the program frozen with no way to
wake it. Throttled, the same failure degrades to a quiet screensaver. The rate is
therefore clamped to `MIN_IDLE_FPS..=MAX_IDLE_FPS`, and a config of `0` does not
mean "stop".

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

## Frame Pacing

The loop holds a fixed cadence. Each frame is scheduled at `started_at + n *
1/60s` and sleeps until that deadline, rather than sleeping for "whatever is left
of the last frame". The second form cannot repay an oversleep -- the overshoot is
measured, discarded, and the next frame sleeps its own remainder on top of it --
so a frame that consistently overran its slot drifted further and further from the
cadence, and the delta every time-integrating effect was handed drifted with it.
Falling more than a frame behind resynchronises rather than accumulating a debt,
because a loop that tries to settle a debt it can never repay runs flat out for
the rest of the session.

Input is polled without a timeout. It used to ask for up to ten milliseconds on
every frame, which is most of a 60 Hz budget spent asleep, and because the block
sat inside the measured frame the delta handed to the effect claimed ten
milliseconds had passed while the program did nothing at all. The cost of that
was not latency -- the loop already runs sixty times a second, so a keystroke
waited at most one frame plus ten milliseconds -- but that a heavy effect could
not fit in what was left of the frame. A non-blocking poll cannot miss anything:
events that arrive during the sleep sit in the terminal's queue and the next poll
drains them.

`delta` is clamped to `MAX_FRAME_DELTA` so a stall does not teleport a
time-integrating effect forward. It used to be a hundred milliseconds, which is
six frames of motion delivered in one.

## Playlist

`Playlist` owns an ordered list of `Slot` values built from config or `--playlist`, plus a `AnyEffect` for the active entry. It accumulates the active effect's diff into a full-screen buffer, then compares that against the previously shown frame. Between effects a diagonal blank wipe masks the frame in two phases (`WipeOut` then `WipeIn`), which keeps transitions independent of what each effect draws. Sequential mode steps through the list; shuffle mode draws from a reshuffling bag so every effect runs once per round.

## Accumulating a diff versus wiping one

`EffectHost` and `Playlist` both wrap a running effect, and both need to apply a
transition to the *finished frame* rather than to the effect's diff. Neither can
use `Canvas` for this, for two reasons.

The first is that wiping a diff is wrong. The effect has already committed the
un-wiped frame to its own `Canvas`, so blanking cells in the diff leaves the
terminal holding cells the effect believes are painted. Those cells are not in the
next frame's diff, so they are never re-sent, and the screen stays half-stale.
The incoming effect is worse: its first frame is a full repaint against a fresh
canvas, the wipe blanks all of it, and the new canvas's baseline then records it
as already on screen. So: accumulate the delta into a frame buffer, wipe a
*copy*, and commit that. The unwiped frame underneath survives, so the wipe can
be taken back rather than baked in permanently.

The second is that `Canvas::commit` swaps its two surfaces, so what it hands back
to draw on next is the frame from *before* the one it just emitted. That is
correct for an effect that repaints its whole surface every frame, and wrong for
anything that accumulates a diff, which is both of these. Built on `Canvas`, the
playlist flickered sixty times a second, alternating between a frame and a nearly
empty one. Both now hold two full-screen buffers and choose explicitly which one
to draw on next.

The transition is two phases, `WipeOut` then `WipeIn`, and the effect is rebuilt
the moment the wipe-out half completes rather than in a separate `Swap` phase. A
`Swap` phase reported "no wipe" for one frame, and because the rebuild happened
after that frame's cells were produced, that frame was a full-brightness unwiped
frame of the *outgoing* effect -- one full-screen flash per switch, immediately
followed by a blank one.

`transition` means the whole switch for `EffectHost` and each half for
`Playlist`, so one `--transition` flag gives two different durations. Both choices
are deliberate and each is pinned by a test, but it is a real divergence and the
first thing to reconcile if the two are ever meant to feel identical.

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

### Testing the output path

Counting glyphs and cursor moves is not enough to test an encoder, and finding
that out cost a colour bug that every test in the suite was happy with. The
encoder was emitting every glyph it should have -- in the terminal's default
foreground, because the style it had been tracking was reset after each one. A
count cannot see that.

So `common.rs` carries a small terminal model, written from the SGR semantics
rather than from the encoder, and the encoded output is replayed through it and
compared against the diff that produced it. Seven of those tests fail against the
old encoder and pass against the new one. The same rule -- assert the observable
result, not a proxy for it -- is why the frame-cost assertions check the
accumulated terminal state rather than the shape of the transition, and why the
palette tests check monotonic brightness rather than specific constants.

A test that only passes with the fix is worth more than one that passes either
way, and the check that a new test has that property is to run it against the old
code. That is cheap here: the encoder and its model are self-contained enough to
lift into a scratch crate and revert, which is how the seven were confirmed.

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
