use crate::buffer::Cell;
use crate::runtime::{
    CrosstermInput, FrameContext, InputEvent, InputSource, InputState, Key,
    KeyPhase,
};
use crate::session::SessionColors;
use crossterm::{QueueableCommand, cursor, style, terminal};
use std::{
    io::{BufWriter, Result, Write},
    time::{Duration, Instant},
};

pub const MIN_EFFECT_SIZE: u16 = 6;
pub const MIN_SPEED: f32 = 0.05;
pub const MAX_SPEED: f32 = 8.0;
pub const SPEED_STEP: f32 = 0.1;

/// Bounds on the unfocused frame rate.
///
/// Zero would mean "stop drawing", which is the cheapest option and the most
/// dangerous one: a terminal that never reports a focus change would then leave
/// the program frozen with no way to wake it. So the slowest it will go is one
/// frame a second, which is slow enough to cost almost nothing and fast enough
/// that a missing focus event degrades to a quiet screensaver rather than a hung
/// one.
pub const MIN_IDLE_FPS: f32 = 1.0;
pub const MAX_IDLE_FPS: f32 = 30.0;

/// The frames per second an unfocused run uses when the config does not say.
///
/// One number in two places, and they disagreed: the config's default was 20 and
/// `RuntimeOptions::default` said 4, and 4 is the value the config's own
/// documentation identifies as a bug -- at that rate the frame-delta clamp costs
/// an unfocused effect four fifths of its *speed* as well as three quarters of
/// its frame rate. Every run from `main` passes the config through
/// `with_focus_policy`, so this default only reaches library users -- but a
/// library user should not get the worse of the two.
pub const DEFAULT_IDLE_FPS: f32 = 20.0;

/// A float that came from a config file, put inside its bounds.
///
/// **`f32::clamp` does not remove `NaN`,** which is the whole reason this is a
/// function rather than a `clamp` call. `Ord::clamp` is
/// `if self < min { min } else if self > max { max } else { self }`, and for
/// `NaN` *both* comparisons are false, so it returns `NaN` unchanged. (`f32::max`
/// and `f32::min` do discard `NaN`; `clamp` does not. That difference is the
/// bug.)
///
/// A config can produce `NaN`, because TOML 1.0 defines `nan` and `inf` as float
/// literals and the `toml` crate parses them. Both of these were reachable and
/// both failed quietly:
///
/// * `idle_fps = nan` reached `Duration::from_secs_f64`, which panics on a `NaN`
///   argument -- *after* the session had already put the terminal into raw mode
///   on the alternate screen with the cursor hidden.
/// * `speed = nan` reached `TickClock::advance`, whose accumulator went `NaN`
///   and stayed there. `accumulator + f32::EPSILON >= QUANTUM` is false for
///   `NaN` on every frame, so `advance` returned zero steps for ever. The effect
///   was never stepped, the loop kept running at 60 Hz, and `write_cells` was
///   handed an empty diff every frame. The result was a completely silent frozen
///   screensaver burning a core: no error, no output, nothing to show that
///   anything had gone wrong.
///
/// A non-finite value is not a request for an extreme one -- nobody types `nan`
/// meaning "as fast as possible" -- so it is treated as absent and the default
/// is used. The command line already refused these: `--speed nan` is rejected by
/// an `is_finite` check in `parse_args_from`. Only the config file let them
/// through, which is how two entry points ended up disagreeing about a value
/// that means the same thing in both.
pub fn bounded_f32(value: f32, default: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

/// The most a single frame's delta is allowed to claim.
///
/// A stall -- a resize, the terminal doing something expensive, the process
/// being descheduled -- must not teleport a time-integrating effect forward, so
/// the delta is clamped. Three frames at 60 Hz: long enough that ordinary jitter
/// never reaches it, short enough that a hitch is a brief pause rather than a
/// visible jump. It used to be a hundred milliseconds, which is six frames of
/// motion delivered in one.
const MAX_FRAME_DELTA: Duration = Duration::from_millis(50);

/// How long check mode waits for input.
///
/// Check mode has no frame loop to pace it, so its poll blocks. The screensaver
/// loop passes [`Duration::ZERO`] instead -- see the comment there.
const CHECK_MODE_POLL: Duration = Duration::from_millis(10);

/// The generator every effect draws its randomness from.
///
/// This is deliberately *not* `ThreadRng`. `ThreadRng` cannot be seeded, so an
/// effect using it can neither be reproduced from a config value nor compared
/// against a second instance — which is why `tests/effect_contracts.rs` used to
/// report nine effects as having an unverifiable timebase. `StdRng` is seedable
/// and, for the handful of draws an effect makes per frame, costs nothing
/// measurable beside the per-cell work that dominates the frame.
pub type EffectRng = rand::rngs::StdRng;

/// The default seed, shared by every effect that has a `seed` field.
///
/// A fixed value keeps `termzzz --print-config` stable and makes a default run
/// reproducible, which is what lets the contract suite compare two instances. It
/// is not meant to be interesting; `--seed` is how you pick a different one.
pub const DEFAULT_SEED: u64 = 42;

/// Seeds a generator for one effect.
///
/// `salt` keeps two effects configured with the same `seed` from replaying an
/// identical sequence of numbers, which is not a bug but makes the effects look
/// implausibly synchronised — the same colour, the same glyph, the same instant
/// in both.
pub fn seeded_rng(seed: u64, salt: &str) -> EffectRng {
    use rand::SeedableRng;

    // FNV-1a rather than `DefaultHasher`: the standard library makes no promise
    // that `DefaultHasher`'s output is stable across releases, and a seed that
    // silently changes meaning between Rust versions would make every
    // "reproducible" run a lie.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in salt.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    EffectRng::seed_from_u64(seed ^ hash)
}

pub fn normalize_effect_size(size: (u16, u16)) -> (u16, u16) {
    (size.0.max(MIN_EFFECT_SIZE), size.1.max(MIN_EFFECT_SIZE))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RuntimeOptions {
    pub speed: f32,
    /// Whether to reduce the frame rate while the terminal window is unfocused.
    pub throttle_when_unfocused: bool,
    /// Frames per second while unfocused. Clamped by [`MIN_IDLE_FPS`] and
    /// [`MAX_IDLE_FPS`], because zero means "stop" and a large value means "do
    /// nothing", and both are worse than the plain reading of the number.
    pub idle_fps: f32,
    /// The colours the session installed for the whole run.
    ///
    /// Carried here rather than read from a global because the output path has
    /// to re-assert them after every SGR reset, and SGR 0 clears the background.
    /// See [`write_cells`]. Defaults to pinning nothing, which is the pre-existing
    /// behaviour and the overwhelmingly common one.
    pub colors: SessionColors,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            speed: 1.0,
            throttle_when_unfocused: true,
            idle_fps: DEFAULT_IDLE_FPS,
            colors: SessionColors::default(),
        }
    }
}

impl RuntimeOptions {
    pub fn new(speed: f32) -> Self {
        Self {
            speed: bounded_f32(speed, 1.0, MIN_SPEED, MAX_SPEED),
            ..Self::default()
        }
    }

    /// Sets what to do while the terminal is unfocused.
    pub fn with_focus_policy(
        mut self,
        throttle_when_unfocused: bool,
        idle_fps: f32,
    ) -> Self {
        self.throttle_when_unfocused = throttle_when_unfocused;
        self.idle_fps =
            bounded_f32(idle_fps, DEFAULT_IDLE_FPS, MIN_IDLE_FPS, MAX_IDLE_FPS);
        self
    }

    /// Records the colours the session installed, so the encoder can put them
    /// back after each reset.
    pub fn with_colors(mut self, colors: SessionColors) -> Self {
        self.colors = colors;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TickClock {
    accumulator: f32,
}

impl TickClock {
    pub const QUANTUM: f32 = 1.0 / 60.0;
    pub const MAX_STEPS: usize = 4;

    pub fn advance(&mut self, delta: Duration, speed: f32) -> usize {
        // Bounded rather than clamped, and this is the backstop that matters.
        // Once the accumulator is `NaN` it is poisoned for good: the loop's test
        // is `accumulator + EPSILON >= QUANTUM`, which is *false* for `NaN` on
        // every frame, so a single bad speed turns "return some steps" into
        // "return zero steps" permanently and silently. The accumulator is a
        // `f32` field with no way to recover it, so the value that reaches it
        // has to be finite. This method is public, so it cannot rely on every
        // caller having gone through `RuntimeOptions` first.
        self.accumulator +=
            delta.as_secs_f32() * bounded_f32(speed, 1.0, MIN_SPEED, MAX_SPEED);
        let mut steps = 0;
        while self.accumulator + f32::EPSILON >= Self::QUANTUM
            && steps < Self::MAX_STEPS
        {
            self.accumulator -= Self::QUANTUM;
            steps += 1;
        }
        if steps == Self::MAX_STEPS {
            self.accumulator = 0.0;
        }
        steps
    }
}

/// What the frame loop drives.
///
/// The loop needs four things from whatever it is running, and only one of them
/// -- the effect -- varies per frame. The rest exist because the program can
/// swap the running effect mid-session: a key to press, a transition to apply to
/// the frame, and somewhere to put a resize.
///
/// This trait is why there is one copy of the loop rather than two. A second
/// loop for the swappable case would be ninety lines that drift apart from the
/// first within a month, and the timing and input handling in particular must
/// not differ between them.
pub trait FrameTarget {
    /// The effect to advance and draw this frame.
    fn effect(&mut self) -> &mut dyn TerminalEffect;

    /// A key the loop would not otherwise act on. Returning true consumes it, so
    /// it does not also reach the effect.
    ///
    /// The phase is passed through because "the user is holding `n` down" and
    /// "the user pressed `n` twice" are indistinguishable without it. A target
    /// that acts on a keypress -- switching effects, say -- needs to tell them
    /// apart, because a held key arrives as a stream of repeats and acting on
    /// each one is what turns a transition into a strobe.
    fn on_global_key(&mut self, _key: Key, _phase: KeyPhase) -> bool {
        false
    }

    /// Called after the frame is produced, before the cells are written.
    ///
    /// The canonical use is the transition between effects, which has to reach
    /// the cells rather than the effect: the wipe is a property of the finished
    /// frame.
    fn on_frame(&mut self, _delta: Duration, _cells: &mut [(usize, usize, Cell)]) {}

    /// Called when the terminal resizes, instead of `update_size` on the effect
    /// directly, so a target with its own size to track can.
    fn on_resize(&mut self, _width: u16, _height: u16) {}
}

/// A [`FrameTarget`] over a single effect that cannot be swapped.
///
/// What tests, benchmarks and check mode drive: the same loop, without the
/// ability to change what is running.
pub struct SingleEffect<'a> {
    effect: &'a mut dyn TerminalEffect,
}

impl<'a> SingleEffect<'a> {
    pub fn new(effect: &'a mut dyn TerminalEffect) -> Self {
        Self { effect }
    }
}

impl FrameTarget for SingleEffect<'_> {
    fn effect(&mut self) -> &mut dyn TerminalEffect {
        self.effect
    }

    /// Forwards to the effect. The trait's default is a no-op, which is right
    /// for a target that tracks its own size and wrong here -- a bare effect has
    /// no size but the caller's, and `update_size` is how it hears about a
    /// resize at all.
    fn on_resize(&mut self, width: u16, height: u16) {
        self.effect.update_size(width, height);
    }
}

pub trait TerminalEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)>;
    fn update(&mut self);
    fn update_size(&mut self, width: u16, height: u16);
    fn reset(&mut self);

    fn handle_input(&mut self, _event: &InputEvent) {}

    fn get_diff_with_context(
        &mut self,
        _context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        self.get_diff()
    }

    fn update_with_context(&mut self, _context: &FrameContext) {
        self.update();
    }
}

/// The attributes a cell turns *on*, as something comparable.
///
/// crossterm's `Attribute::Reset` is SGR 0, which clears the colours as well as
/// the attributes. A cell tagged `Reset` therefore used to have its colour wiped
/// between the colour being set and the glyph being drawn, and rendered in
/// whatever the terminal's default foreground happened to be -- usually white.
/// The mandelbrot tags *every* cell `Reset`, so the whole effect, including its
/// black interior, came out white.
///
/// [`Cell::attr`] is read here as additive only. An attribute is either turned on
/// or it is not; `Reset` and `NormalIntensity` both mean "nothing is on". An
/// attribute change is applied with a full reset followed by the attributes that
/// should be on, and the colours are restated *after* that, never before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct AttributesOn {
    bold: bool,
}

impl AttributesOn {
    fn of(attr: style::Attribute) -> Self {
        Self {
            bold: matches!(attr, style::Attribute::Bold),
        }
    }
}

/// The style the encoder believes the terminal is currently in.
///
/// Tracked so that a run of identically styled cells costs one escape sequence
/// rather than one per cell. The previous implementation of this tracked
/// `(colour, attribute)` and skipped the sequence when they matched, which was
/// wrong in three separate ways; see [`write_cells`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveStyle {
    fg: style::Color,
    bg: style::Color,
    attributes: AttributesOn,
}

impl Default for ActiveStyle {
    /// The state a terminal is in after an SGR reset, which is also the state
    /// this function leaves it in at the end of every frame.
    fn default() -> Self {
        Self {
            fg: style::Color::Reset,
            bg: style::Color::Reset,
            attributes: AttributesOn::default(),
        }
    }
}

/// The style the terminal is in at the start of a frame.
///
/// Not [`ActiveStyle::default`] when the session pins its colours, because
/// `session.rs` set them and nothing has reset them since. Identical to
/// `ActiveStyle::default` when it pins nothing, which is the default and the
/// overwhelmingly common case.
fn session_style(colors: SessionColors) -> ActiveStyle {
    ActiveStyle {
        fg: colors.foreground,
        bg: colors.background,
        attributes: AttributesOn::default(),
    }
}

/// A cell colour, with `Reset` read as "whatever the session pinned".
///
/// The identity function when nothing is pinned, which is what keeps this from
/// changing a single byte of the default path.
fn resolve(cell: style::Color, session: style::Color) -> style::Color {
    if cell == style::Color::Reset {
        session
    } else {
        cell
    }
}

/// Puts the session's colours back after an SGR 0 took them away.
///
/// Emits nothing at all in the default case, so the common run is unaffected.
fn reassert_session_colors<W: Write>(
    out: &mut W,
    colors: SessionColors,
) -> Result<()> {
    if colors.is_default() {
        return Ok(());
    }
    out.queue(style::SetForegroundColor(colors.foreground))?;
    out.queue(style::SetBackgroundColor(colors.background))?;
    Ok(())
}

/// Writes changed cells to the terminal.
///
/// Cells outside `size` are skipped: a diff is compared against the previous
/// frame, and an effect that has not yet caught up with a resize can report
/// coordinates derived from the old dimensions.
///
/// # Why this does not use `PrintStyledContent`
///
/// It used to. crossterm's `PrintStyledContent` writes the foreground, then the
/// attributes, then the glyph, and then -- because `write_cells` always attached
/// an attribute, which puts a bit in a non-empty set -- a full `ResetColor`. So
/// every styled glyph was followed by `\x1b[0m`, and the style cache that decided
/// whether to re-state the colour was wrong from the second cell of every run
/// onwards. Those cells were emitted as bare glyphs and rendered in the
/// terminal's *default* foreground, which is white on a dark profile. That is
/// what "the colours show but not fully and clean, with a lot of white mixed in"
/// was.
///
/// So the escape sequences are built here instead, from crossterm's individual
/// commands, which keeps crossterm's well-tested colour encoding and lets this
/// function control what is emitted and when:
///
/// * a cursor move only when the cell is not the one immediately after the
///   previous one, so a run along a row costs one move rather than one per cell;
/// * a foreground colour only when it differs from the one in effect;
/// * a background colour only when it differs, which is what makes the
///   half-block glyph `▀` able to carry two colours per cell at all;
/// * a reset plus the wanted attributes only when the attributes change.
///
/// The tracked style is deliberately forgotten between calls, so a frame can
/// never inherit a stale idea of what the terminal is set to, and each frame
/// ends by resetting, so what the terminal is left in matches what the next
/// frame assumes.
///
/// This is a free function rather than an inline loop so the output path can be
/// measured and tested without running a whole frame loop.
///
/// # The session's own colours
///
/// `colors` is the pair [`crate::session::TerminalSession`] installed for the
/// whole session, and it is here because **SGR 0 clears the background.** The
/// session pins the background with one `SetBackgroundColor` at startup, and
/// this function emits `ResetColor` three times per frame -- the first-cell
/// baseline, an attribute change mid-run, and the end-of-frame reset. Every one
/// of them took the pinned background with it, on frame one and on every frame
/// after, so `global.background` was a setting that did nothing.
///
/// So a cell carrying `Color::Reset` in its background now means *the session's
/// background* rather than *the terminal's default*, and the session colours are
/// re-asserted after each reset. That distinction is invisible in the default
/// case and is the whole of the fix: with `colors` at its default, `Reset`
/// resolves to `Reset`, the re-assert emits nothing, and the bytes on the wire
/// are identical to what they were before. `the_default_session_costs_nothing_
/// extra` is the test that holds that line.
///
/// The attribute-change case is the same defect one level down. `ResetColor`
/// there also clears the colours, so the comparison against `previous` has to be
/// against a terminal that is no longer wearing them -- see the comment inside.
pub fn write_cells<W: Write>(
    out: &mut W,
    size: (u16, u16),
    cells: &[(usize, usize, Cell)],
    colors: SessionColors,
) -> Result<()> {
    let (max_x, max_y) = (size.0 as usize, size.1 as usize);

    // Where the terminal cursor sits once everything written so far has been
    // drawn, and the style it is currently in.
    let mut cursor: Option<(u16, u16)> = None;
    let mut active: Option<ActiveStyle> = None;

    for &(x, y, cell) in cells {
        if x >= max_x || y >= max_y {
            continue;
        }
        let column = x as u16;
        let row = y as u16;

        if cursor != Some((column, row)) {
            out.queue(cursor::MoveTo(column, row))?;
        }

        let wanted = ActiveStyle {
            fg: resolve(cell.color, colors.foreground),
            bg: resolve(cell.bg, colors.background),
            attributes: AttributesOn::of(cell.attr),
        };

        if active.is_none() {
            // Establish the baseline rather than assume it. Entering the
            // alternate screen does not reset SGR, so the terminal may still be
            // wearing whatever style the shell left it in, and a first cell that
            // asks for the default colours would otherwise inherit them. Once
            // per frame, so it costs nothing.
            out.queue(style::ResetColor)?;
            // ...and put the session's own colours back, which SGR 0 just took
            // with it. Two sequences a frame, and only when they are pinned.
            reassert_session_colors(out, colors)?;
        }

        if active != Some(wanted) {
            // Start from what is in effect, not from a hardcoded default: the
            // first cell of a frame may be continuing the terminal's real state
            // rather than the reset state, and a diff can begin mid-screen.
            let mut previous = active.unwrap_or_else(|| session_style(colors));

            if previous.attributes != wanted.attributes {
                // SGR has no cheap "turn bold off and leave the colour alone",
                // so an attribute change costs a reset plus the attributes that
                // should be on. The colours are restated below, after it, which
                // is the whole point: `Attribute::Reset` used to land *before* the
                // glyph and erase the colour with it.
                out.queue(style::ResetColor)?;
                if wanted.attributes.bold {
                    out.queue(style::SetAttribute(style::Attribute::Bold))?;
                }
                // SGR 0 cleared the colours too, so from here on the terminal is
                // *not* wearing `previous`'s colours -- it is wearing whatever
                // `reassert_session_colors` just put back, which is the session's
                // own. Without this the two comparisons below compare against a
                // state the terminal left several sequences ago: a bold cell
                // followed by a non-bold cell of the same colour compared equal,
                // emitted nothing, and drew that cell -- and every cell after it,
                // because `active` went on to record a style the terminal did
                // not have -- in whatever the terminal's default foreground
                // happened to be.
                previous = session_style(colors);
                reassert_session_colors(out, colors)?;
            }
            if previous.fg != wanted.fg {
                out.queue(style::SetForegroundColor(wanted.fg))?;
            }
            if previous.bg != wanted.bg {
                out.queue(style::SetBackgroundColor(wanted.bg))?;
            }

            active = Some(wanted);
        }

        out.queue(style::Print(cell.symbol))?;

        cursor = Some((column + 1, row));
    }

    // Leave the terminal in the state `session_style` describes, so the next
    // frame's assumption holds and the shell the user returns to is not left
    // wearing whatever colour the last cell happened to be. Once per frame, so
    // it costs nothing. The re-assert is the same three-sites-one-defect as the
    // baseline above: without it the session's background is gone by the time
    // the first cell of the next frame is drawn.
    if active.is_some() {
        out.queue(style::ResetColor)?;
        reassert_session_colors(out, colors)?;
    }

    Ok(())
}

/// Runs a [`FrameTarget`] against an injected source and a known size.
///
/// The seam the integration tests drive a host through: `run_loop_with_target`
/// needs a real terminal, and these do not.
pub fn run_loop_with_source_and_size_and_target<W, S, T>(
    stdout: &mut W,
    target: &mut T,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
    S: InputSource,
    T: FrameTarget + ?Sized,
{
    run_loop_with_source_and_size_and_options(
        stdout,
        target,
        iterations,
        source,
        initial_size,
        options,
    )
}

/// Polls input once, without drawing. Used by check mode.
///
/// Check mode has no frame loop pacing it, so the poll is allowed to block:
/// without a timeout this would spin rather than wait.
pub fn process_input<TE>(effect: &mut TE) -> Result<bool>
where
    TE: TerminalEffect,
{
    let mut size = terminal::size()?;
    let mut input = InputState::default();
    input.set_size(size);
    let mut source = CrosstermInput;
    let mut speed = 1.0;
    let mut target = SingleEffect::new(effect);
    process_runtime_events(
        &mut target,
        &mut source,
        &mut input,
        &mut size,
        &mut speed,
        CHECK_MODE_POLL,
    )
}

/// Runs a bare effect against the real terminal.
pub fn run_loop<W>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
) -> Result<f64>
where
    W: Write,
{
    let mut source = CrosstermInput;
    let size = terminal::size()?;
    run_loop_with_source_and_size(stdout, effect, iterations, &mut source, size)
}

/// Runs a bare effect against the real terminal, with a speed multiplier.
pub fn run_loop_with_options<W>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
{
    let mut source = CrosstermInput;
    let size = terminal::size()?;
    let mut target = SingleEffect::new(effect);
    run_loop_with_source_and_size_and_options(
        stdout,
        &mut target,
        iterations,
        &mut source,
        size,
        options,
    )
}

/// Runs a bare effect against an injected input source.
pub fn run_loop_with_source<W, S>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    source: &mut S,
) -> Result<f64>
where
    W: Write,
    S: InputSource,
{
    let size = terminal::size()?;
    run_loop_with_source_and_size(stdout, effect, iterations, source, size)
}

/// Runs a bare effect against an injected source and a known terminal size.
pub fn run_loop_with_source_and_size<W, S>(
    stdout: &mut W,
    effect: &mut dyn TerminalEffect,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
) -> Result<f64>
where
    W: Write,
    S: InputSource,
{
    let mut target = SingleEffect::new(effect);
    run_loop_with_source_and_size_and_options(
        stdout,
        &mut target,
        iterations,
        source,
        initial_size,
        RuntimeOptions::default(),
    )
}

/// Runs a [`FrameTarget`] against the real terminal.
///
/// This is how the program gets an effect it can swap mid-session. Every other
/// entry point above takes a bare effect for tests, benchmarks and check mode
/// and wraps it in a [`SingleEffect`], so there is still only one loop.
pub fn run_loop_with_target<W, T>(
    stdout: &mut W,
    target: &mut T,
    iterations: Option<usize>,
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
    T: FrameTarget + ?Sized,
{
    let mut source = CrosstermInput;
    let size = terminal::size()?;
    run_loop_with_source_and_size_and_options(
        stdout,
        target,
        iterations,
        &mut source,
        size,
        options,
    )
}

/// The one frame loop. Every other `run_loop*` is a convenience wrapper over it.
///
/// Takes a trait object rather than a generic. A generic would monomorphise the
/// entire loop once per effect type -- fifteen copies of the timing, input and
/// encoding code -- and would forbid holding effects of different types at once,
/// which is what swapping the running effect needs.
pub fn run_loop_with_source_and_size_and_options<W, S, T>(
    stdout: &mut W,
    target: &mut T,
    iterations: Option<usize>,
    source: &mut S,
    initial_size: (u16, u16),
    options: RuntimeOptions,
) -> Result<f64>
where
    W: Write,
    S: InputSource,
    T: FrameTarget + ?Sized,
{
    if iterations == Some(0) {
        return Ok(0.0);
    }

    let mut size = (initial_size.0.max(1), initial_size.1.max(1));
    let mut input = InputState::default();
    input.set_size(size);
    let effect_size = normalize_effect_size(size);
    if effect_size != size {
        target.on_resize(effect_size.0, effect_size.1);
        target.effect().reset();
    }
    let started_at = Instant::now();
    let mut previous_frame = started_at;
    let mut frame = 0u64;
    let mut is_running = true;
    let mut frames_per_second = 0.0;
    let mut speed = bounded_f32(options.speed, 1.0, MIN_SPEED, MAX_SPEED);
    let mut tick_clock = TickClock::default();
    let active_frame_duration = Duration::from_secs_f64(1.0 / 60.0);
    let idle_frame_duration = Duration::from_secs_f64(
        1.0 / f64::from(bounded_f32(
            options.idle_fps,
            DEFAULT_IDLE_FPS,
            MIN_IDLE_FPS,
            MAX_IDLE_FPS,
        )),
    );
    // When the next frame is due, advanced by a fixed step each time rather than
    // measured from the end of the last one. Sleeping for "the remainder of the
    // last frame" cannot repay an oversleep, so the period drifts and the delta
    // handed to every time-integrating effect jitters with it.
    let mut next_frame_at = started_at;
    let mut buffered_stdout = BufWriter::new(stdout);

    while is_running {
        let frame_started_at = Instant::now();
        let delta = frame_started_at
            .saturating_duration_since(previous_frame)
            .min(MAX_FRAME_DELTA);
        let elapsed = frame_started_at.duration_since(started_at);
        previous_frame = frame_started_at;
        input.begin_frame();

        // Non-blocking. This used to wait up to ten milliseconds for input on
        // every frame, which is most of a 60 Hz frame budget spent asleep, and
        // it was inside the measured delta, so effects were being told ten
        // milliseconds had passed while the program did nothing. Polling without
        // a timeout costs nothing -- the loop already runs sixty times a second,
        // so the worst a keystroke now waits is one frame, which is better than
        // the ten milliseconds plus a frame it used to wait.
        if !process_runtime_events(
            target,
            source,
            &mut input,
            &mut size,
            &mut speed,
            Duration::ZERO,
        )? {
            break;
        }

        // Resolved after the poll, because a focus change arrives as an event.
        //
        // There used to be a resume branch here that reset `previous_frame` and
        // `next_frame_at` on regaining focus, to stop the wall-clock time spent
        // away being handed to the effect as one delta. It was doing nothing,
        // twice over: the delta is clamped to `MAX_FRAME_DELTA` at the top of the
        // loop, so a long absence could never have been handed over in one piece
        // anyway, and with the freeze gone the simulation advances while away, so
        // there is no absence left to discard. Resetting the clock on resume
        // would now *drop* a frame's worth of real time for no reason.
        let focused = !options.throttle_when_unfocused || input.is_focused();

        let context = FrameContext::new(size, frame, elapsed, delta, input.clone());
        let mut diff = target.effect().get_diff_with_context(&context);

        // Before anything is written, so a transition can reach the cells.
        target.on_frame(delta, &mut diff);

        write_cells(&mut buffered_stdout, size, &diff, options.colors)?;
        buffered_stdout.flush()?;

        // The simulation advances whether or not the window has focus.
        //
        // It used to be inside `if focused`, and the report was "when I put it
        // on my second monitor and work on my first, it stops". It was worse
        // than a slowdown, because **no bytes reached the terminal at all**:
        // every effect renders from state that only `update` advances, so an
        // un-updated frame re-renders byte-identically, `Canvas::commit` diffs
        // it to nothing, and `write_cells` writes an empty frame four times a
        // second. The loop was spinning to produce silence.
        //
        // The justification for freezing was that at four frames a second a real
        // delta is 250 ms, which `MAX_FRAME_DELTA` clamps to 50, so an effect
        // would run at a fifth of its speed. That is true and it is a *consequence
        // of `idle_fps`*, not a reason to stop: a frozen screen is worse than a
        // slow one, and `pause_when_unfocused` is now the knob that says which
        // of the two you would rather have.
        if (speed - 1.0).abs() < f32::EPSILON {
            target.effect().update_with_context(&context);
        } else {
            let steps = tick_clock.advance(delta, speed);
            let mut tick_context = context.clone();
            tick_context.delta = Duration::from_secs_f64(TickClock::QUANTUM as f64);
            for _ in 0..steps {
                target.effect().update_with_context(&tick_context);
            }
        }

        // Fixed-cadence pacing. A frame that overruns its slot pushes the next
        // one out rather than compounding the overshoot, and a frame that
        // finishes early gives the difference back here instead of losing it.
        // The step is whichever cadence focus calls for; the accumulator does not
        // care that it changes.
        next_frame_at += if focused {
            active_frame_duration
        } else {
            idle_frame_duration
        };
        let now = Instant::now();
        if next_frame_at <= now {
            // Too far behind to repay. Resynchronise rather than accumulate a
            // debt the loop would spend the rest of the session trying to
            // settle, which is what makes a loop like this run flat out after
            // one stall.
            next_frame_at = now;
        } else {
            std::thread::sleep(next_frame_at - now);
        }

        let measured_duration =
            frame_started_at.elapsed().as_secs_f64().max(f64::EPSILON);
        frames_per_second = (frames_per_second + 1.0 / measured_duration) / 2.0;

        frame += 1;
        if iterations.is_some_and(|limit| frame as usize >= limit) {
            is_running = false;
        }
    }

    Ok(frames_per_second)
}

fn process_runtime_events<S, T>(
    target: &mut T,
    source: &mut S,
    input: &mut InputState,
    size: &mut (u16, u16),
    speed: &mut f32,
    poll_timeout: Duration,
) -> Result<bool>
where
    S: InputSource,
    T: FrameTarget + ?Sized,
{
    let events = source.poll(poll_timeout)?;
    if events.iter().any(is_quit_event) {
        return Ok(false);
    }

    let last_resize = events.iter().rev().find_map(|event| match event {
        InputEvent::Resize { size } => Some(*size),
        _ => None,
    });
    if let Some(new_size) = last_resize {
        *size = (new_size.0.max(1), new_size.1.max(1));
        input.set_size(*size);
        let effect_size = normalize_effect_size(*size);
        target.on_resize(effect_size.0, effect_size.1);
        target.effect().reset();
    }

    for event in events {
        if matches!(event, InputEvent::Resize { .. }) {
            continue;
        }
        input.apply(event);
        if let InputEvent::Key { key, phase } = event
            && matches!(phase, KeyPhase::Pressed | KeyPhase::Repeated)
        {
            match key {
                Key::Char('+') | Key::Char('=') => {
                    *speed = (*speed + SPEED_STEP).min(MAX_SPEED);
                    continue;
                }
                Key::Char('-') => {
                    *speed = (*speed - SPEED_STEP).max(MIN_SPEED);
                    continue;
                }
                key if target.on_global_key(key, phase) => continue,
                _ => {}
            }
        }
        if !matches!(event, InputEvent::Ignored) {
            target.effect().handle_input(&event);
        }
    }
    Ok(true)
}

fn is_quit_event(event: &InputEvent) -> bool {
    match event {
        InputEvent::Quit => true,
        InputEvent::Key { key, phase } => {
            matches!(phase, KeyPhase::Pressed | KeyPhase::Repeated)
                && matches!(key, Key::Char('q') | Key::Escape | Key::CtrlC)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::style::Color;

    fn cell(symbol: char) -> Cell {
        Cell::new(symbol, Color::Red, style::Attribute::Bold)
    }

    /// Counts cursor moves, which are the CSI sequences ending in `H`.
    fn count_moves(out: &str) -> usize {
        out.split("\u{1b}[").filter(|seq| seq.contains('H')).count()
    }

    /// Counts full SGR resets, which is the number that has to stay small.
    ///
    /// An attribute change legitimately costs one, because SGR has no cheap
    /// "turn bold off and leave the colour alone". A *colour* change must not:
    /// the encoder sets the new colour directly instead.
    fn count_resets(out: &str) -> usize {
        out.matches("\u{1b}[0m").count()
    }

    fn encoded(size: (u16, u16), cells: &[(usize, usize, Cell)]) -> String {
        encoded_with(size, cells, SessionColors::default())
    }

    /// [`encoded`] against a session that pins its own colours, which is the
    /// case the pin is about and the one most of these tests do not exercise.
    fn encoded_with(
        size: (u16, u16),
        cells: &[(usize, usize, Cell)],
        colors: SessionColors,
    ) -> String {
        let mut out: Vec<u8> = Vec::new();
        write_cells(&mut out, size, cells, colors)
            .expect("writing to a Vec cannot fail");
        String::from_utf8(out).expect("crossterm emits utf8")
    }

    /// What one cell of the modelled terminal ended up looking like.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Painted {
        symbol: char,
        fg: Color,
        bg: Color,
        bold: bool,
    }

    /// A terminal, modelled well enough to check what the encoder did.
    ///
    /// This is deliberately written from the SGR semantics rather than from the
    /// encoder, so the two can disagree. That is the point: the encoder used to
    /// believe a style stayed in effect after a styled glyph, and every test that
    /// only counted glyphs and cursor moves was happy with that, because the
    /// glyph really was emitted -- in the wrong colour, and no test looked.
    ///
    /// It understands the subset of SGR the encoder emits, and panics on anything
    /// else, so an unexpected escape shows up as a failure rather than as a
    /// silently ignored one.
    struct Terminal {
        width: usize,
        height: usize,
        painted: Vec<Painted>,
        cursor: (usize, usize),
        fg: Color,
        bg: Color,
        bold: bool,
    }

    impl Terminal {
        fn new(width: usize, height: usize) -> Self {
            let blank = Painted {
                symbol: ' ',
                fg: Color::Reset,
                bg: Color::Reset,
                bold: false,
            };
            Self {
                width,
                height,
                painted: vec![blank; width * height],
                cursor: (0, 0),
                fg: Color::Reset,
                bg: Color::Reset,
                bold: false,
            }
        }

        fn replay(&mut self, out: &str) {
            let mut chars = out.chars().peekable();
            while let Some(c) = chars.next() {
                if c != '\u{1b}' {
                    self.draw(c);
                    continue;
                }
                assert_eq!(chars.next(), Some('['), "expected a CSI sequence");
                let mut sequence = String::new();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        self.control(&sequence, c);
                        break;
                    }
                    sequence.push(c);
                }
            }
        }

        fn control(&mut self, sequence: &str, final_byte: char) {
            let parts: Vec<&str> = sequence.split(';').collect();
            let numbers: Vec<u8> = parts
                .iter()
                .map(|part| part.parse::<u8>().unwrap_or(0))
                .collect();

            match final_byte {
                // Cursor position, one-based, which is what crossterm emits.
                'H' => {
                    let row =
                        numbers.first().copied().unwrap_or(1).max(1) as usize - 1;
                    let column =
                        numbers.get(1).copied().unwrap_or(1).max(1) as usize - 1;
                    self.cursor = (row, column);
                }
                'm' => self.select_graphic_rendition(&numbers),
                other => panic!("the encoder emitted an unhandled escape: {other}"),
            }
        }

        fn select_graphic_rendition(&mut self, numbers: &[u8]) {
            let mut index = 0;
            while index < numbers.len() {
                match numbers[index] {
                    0 => {
                        self.fg = Color::Reset;
                        self.bg = Color::Reset;
                        self.bold = false;
                        index += 1;
                    }
                    1 => {
                        self.bold = true;
                        index += 1;
                    }
                    22 => {
                        self.bold = false;
                        index += 1;
                    }
                    39 => {
                        self.fg = Color::Reset;
                        index += 1;
                    }
                    49 => {
                        self.bg = Color::Reset;
                        index += 1;
                    }
                    38 | 48 => {
                        // crossterm encodes a colour as `38;2;r;g;b` (24-bit) or
                        // `38;5;n` (256-colour index). Only the 24-bit form is
                        // expected from the effects, which all use truecolor, so
                        // anything else is a surprise worth failing on.
                        let target = numbers[index];
                        let color = match numbers.get(index + 1).copied() {
                            Some(2) => Color::Rgb {
                                r: numbers[index + 2],
                                g: numbers[index + 3],
                                b: numbers[index + 4],
                            },
                            Some(5) => from_ansi_index(numbers[index + 2]),
                            other => panic!(
                                "unexpected colour encoding {other:?} in {numbers:?}"
                            ),
                        };
                        if target == 38 {
                            self.fg = color;
                        } else {
                            self.bg = color;
                        }
                        index += if numbers[index + 1] == 2 { 5 } else { 3 };
                    }
                    other => panic!("unhandled SGR parameter {other}"),
                }
            }
        }

        fn draw(&mut self, symbol: char) {
            let (row, column) = self.cursor;
            if row < self.height && column < self.width {
                self.painted[row * self.width + column] = Painted {
                    symbol,
                    fg: self.fg,
                    bg: self.bg,
                    bold: self.bold,
                };
            }
            self.cursor = (row, column + 1);
        }

        fn at(&self, x: usize, y: usize) -> Painted {
            self.painted[y * self.width + x]
        }

        /// A terminal already sitting in a session's colours.
        ///
        /// `session.rs` emits the pinned pair once at startup, so a frame does
        /// not begin from a reset state -- it begins from these. Modelling the
        /// start any other way is what let the pin be defeated by the encoder's
        /// own SGR 0 without a single test noticing.
        fn in_session(width: usize, height: usize, colors: SessionColors) -> Self {
            let mut terminal = Self::new(width, height);
            terminal.fg = colors.foreground;
            terminal.bg = colors.background;
            terminal
        }
    }

    /// The colour a 256-colour SGR index names, in crossterm's own mapping.
    ///
    /// That mapping is not the obvious one -- `Red` is 9 and `DarkRed` is 1 --
    /// so it is transcribed rather than derived. A model that guessed here
    /// would agree with a wrong encoder, which is the one thing this must not
    /// do.
    fn from_ansi_index(index: u8) -> Color {
        const TABLE: [Color; 16] = [
            Color::Black,       // 0
            Color::DarkRed,     // 1
            Color::DarkGreen,   // 2
            Color::DarkYellow,  // 3
            Color::DarkBlue,    // 4
            Color::DarkMagenta, // 5
            Color::DarkCyan,    // 6
            Color::Grey,        // 7
            Color::DarkGrey,    // 8
            Color::Red,         // 9
            Color::Green,       // 10
            Color::Yellow,      // 11
            Color::Blue,        // 12
            Color::Magenta,     // 13
            Color::Cyan,        // 14
            Color::White,       // 15
        ];
        TABLE
            .get(index as usize)
            .copied()
            .unwrap_or(Color::AnsiValue(index))
    }

    /// Replays the encoder's output and returns what the terminal would show.
    fn painted(size: (u16, u16), cells: &[(usize, usize, Cell)]) -> Terminal {
        let out = encoded(size, cells);
        let mut terminal = Terminal::new(size.0 as usize, size.1 as usize);
        terminal.replay(&out);
        terminal
    }

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb { r, g, b }
    }

    /// The bug this whole model exists for.
    ///
    /// The encoder used to skip the colour whenever it matched the previous
    /// cell's, on the assumption that the style was still in effect. crossterm
    /// resets it after every styled glyph, so the second cell onwards of any run
    /// was drawn in the terminal's default foreground -- white, on a dark
    /// profile. The glyph was always present, which is why every existing test
    /// passed.
    #[test]
    fn every_cell_of_a_uniform_run_lands_in_its_own_colour() {
        let run: Vec<(usize, usize, Cell)> = (0..8)
            .map(|x| {
                (
                    x,
                    0,
                    Cell::new('#', rgb(200, 30, 30), style::Attribute::Bold),
                )
            })
            .collect();

        let terminal = painted((20, 4), &run);

        for x in 0..8 {
            let cell = terminal.at(x, 0);
            assert_eq!(cell.symbol, '#', "cell {x} has no glyph");
            assert_eq!(
                cell.fg,
                rgb(200, 30, 30),
                "cell {x} was painted {:?}, not the colour it asked for -- the \
                 style did not survive from the previous cell",
                cell.fg
            );
            assert!(cell.bold, "cell {x} lost its bold");
        }
    }

    /// The same, for a run with no attributes at all, which is the shape the
    /// mandelbrot produces.
    #[test]
    fn a_run_of_attribute_reset_cells_keeps_its_colour() {
        let run: Vec<(usize, usize, Cell)> = (0..8)
            .map(|x| {
                (
                    x,
                    0,
                    Cell::new('x', rgb(10, 90, 200), style::Attribute::Reset),
                )
            })
            .collect();

        let terminal = painted((20, 4), &run);

        for x in 0..8 {
            assert_eq!(
                terminal.at(x, 0).fg,
                rgb(10, 90, 200),
                "cell {x} lost its colour: `Attribute::Reset` is SGR 0, which \
                 clears the colours too, so it has to be applied before the \
                 colour rather than instead of it"
            );
        }
    }

    /// `Attribute::Reset` after a bold cell has to turn the bold off *and* leave
    /// the new colour intact.
    #[test]
    fn leaving_bold_does_not_take_the_colour_with_it() {
        let cells = [
            (
                0usize,
                0usize,
                Cell::new('a', rgb(1, 2, 3), style::Attribute::Bold),
            ),
            (1, 0, Cell::new('b', rgb(4, 5, 6), style::Attribute::Reset)),
            (2, 0, Cell::new('c', rgb(7, 8, 9), style::Attribute::Reset)),
        ];

        let terminal = painted((10, 3), &cells);

        assert!(terminal.at(0, 0).bold, "the bold cell lost its bold");
        assert!(!terminal.at(1, 0).bold, "bold survived into a plain cell");
        assert_eq!(terminal.at(1, 0).fg, rgb(4, 5, 6));
        assert_eq!(terminal.at(2, 0).fg, rgb(7, 8, 9));
    }

    /// The half-block glyph is the only reason `Cell` carries a background, and
    /// it is how the sub-cell renderers get two colours into one cell.
    #[test]
    fn the_background_colour_reaches_the_terminal() {
        let cells = [
            (
                0usize,
                0usize,
                Cell::with_bg(
                    crate::render::halfblock::UPPER,
                    rgb(255, 0, 0),
                    rgb(0, 0, 255),
                    style::Attribute::Reset,
                ),
            ),
            (
                1,
                0,
                Cell::with_bg(
                    crate::render::halfblock::UPPER,
                    rgb(255, 0, 0),
                    rgb(0, 255, 0),
                    style::Attribute::Reset,
                ),
            ),
            (
                2,
                0,
                Cell::with_bg(
                    crate::render::halfblock::UPPER,
                    rgb(255, 0, 0),
                    rgb(0, 255, 0),
                    style::Attribute::Reset,
                ),
            ),
        ];

        let terminal = painted((10, 3), &cells);

        assert_eq!(terminal.at(0, 0).fg, rgb(255, 0, 0));
        assert_eq!(terminal.at(0, 0).bg, rgb(0, 0, 255));
        // The third cell repeats the second's background exactly, so it catches
        // an encoder that decides the background has not changed and skips it.
        assert_eq!(
            terminal.at(1, 0).bg,
            rgb(0, 255, 0),
            "a changed background was dropped"
        );
        assert_eq!(terminal.at(2, 0).bg, rgb(0, 255, 0));
    }

    /// A cell that carries no background must actively clear one that a previous
    /// cell set, or a half-block field bleeds its bottom colour sideways.
    #[test]
    fn a_cell_with_no_background_clears_one_that_was_set() {
        let cells = [
            (
                0usize,
                0usize,
                Cell::with_bg(
                    'a',
                    rgb(9, 9, 9),
                    rgb(1, 1, 1),
                    style::Attribute::Reset,
                ),
            ),
            (1, 0, Cell::new('b', rgb(9, 9, 9), style::Attribute::Reset)),
        ];

        let terminal = painted((10, 3), &cells);

        assert_eq!(terminal.at(1, 0).bg, Color::Reset, "a background leaked");
    }

    /// A session that pins its background keeps it, across a whole frame.
    ///
    /// This is the test for `global.background` doing anything at all. The pin is
    /// one `SetBackgroundColor` at startup, and the encoder emits SGR 0 three
    /// times a frame -- the first-cell baseline, an attribute change, and the
    /// end-of-frame reset -- and SGR 0 clears the background along with
    /// everything else. So the pin survived until the first frame and no longer,
    /// for every effect that writes a cell.
    ///
    /// The first cell of the frame is a plain coloured glyph with no background,
    /// which is what almost every effect emits for most of its cells, and
    /// `solarsystem` emits it for the *whole screen*.
    #[test]
    fn a_pinned_background_survives_a_frame() {
        let pinned = SessionColors {
            background: Color::Rgb { r: 0, g: 0, b: 0 },
            foreground: Color::Rgb {
                r: 0xcc,
                g: 0xcc,
                b: 0xdd,
            },
        };
        let cells = [
            (
                0usize,
                0usize,
                Cell::new('*', rgb(255, 255, 255), style::Attribute::Reset),
            ),
            // An attribute change partway through, because that is the third SGR 0
            // site and it is the one a single-cell frame cannot reach.
            (
                1,
                0,
                Cell::new('*', rgb(255, 255, 255), style::Attribute::Bold),
            ),
            (
                2,
                0,
                Cell::new('*', rgb(255, 255, 255), style::Attribute::Reset),
            ),
            // And a cell that paints its own background, which must still win.
            (
                3,
                0,
                Cell::with_bg(
                    'x',
                    rgb(1, 2, 3),
                    rgb(200, 30, 30),
                    style::Attribute::Reset,
                ),
            ),
        ];

        let mut out: Vec<u8> = Vec::new();
        write_cells(&mut out, (4, 1), &cells, pinned)
            .expect("writing to a Vec cannot fail");
        let encoded = String::from_utf8(out).expect("crossterm emits utf8");

        let mut terminal = Terminal::in_session(4, 1, pinned);
        terminal.replay(&encoded);

        for x in 0..3 {
            assert_eq!(
                terminal.at(x, 0).bg,
                pinned.background,
                "the pinned background was gone at ({x}, 0), so a cell that asked \
                 for no background of its own was drawn in the terminal's default"
            );
        }
        assert_eq!(
            terminal.at(3, 0).bg,
            rgb(200, 30, 30),
            "a cell's own background was overridden by the session pin"
        );
    }

    /// The default session is byte-for-byte what it always was.
    ///
    /// The whole fix is a reinterpretation of `Color::Reset` and a re-assert
    /// after each reset. With nothing pinned, `Reset` resolves to `Reset` and the
    /// re-assert emits nothing -- so if this test ever fails, the change has
    /// started costing bytes on the overwhelmingly common run, and every encoder
    /// test below is measuring something the user does not get.
    #[test]
    fn the_default_session_costs_nothing_extra() {
        // No cell carries a background of its own, so a default session has
        // nothing to re-assert and nothing to resolve. That is what makes this a
        // byte-level comparison against the pre-existing output rather than a
        // test of the new behaviour.
        let cells = [
            (
                0usize,
                0usize,
                Cell::new('a', Color::Red, style::Attribute::Bold),
            ),
            (1, 0, Cell::new('b', Color::Red, style::Attribute::Reset)),
            (2, 0, Cell::new('c', Color::Blue, style::Attribute::Reset)),
        ];

        let mut out: Vec<u8> = Vec::new();
        write_cells(&mut out, (3, 1), &cells, SessionColors::default())
            .expect("writing to a Vec cannot fail");
        let encoded = String::from_utf8(out).expect("crossterm emits utf8");

        // The re-assert would emit SGR 39 and 49. With nothing pinned it must
        // emit neither, which is the whole of the no-op claim.
        assert!(
            !encoded.contains("39m") && !encoded.contains("49m"),
            "a default session emitted a colour reset, so the re-assert is not a \
             no-op: {encoded:?}"
        );
        // The first-cell baseline, two attribute changes (bold on, bold off),
        // and the end-of-frame reset. Exactly what it emitted before.
        assert_eq!(
            count_resets(&encoded),
            4,
            "expected the baseline, both attribute changes and the end-of-frame \
             reset and nothing else: {encoded:?}"
        );
    }

    /// A colour survives an attribute change that does not change it.
    ///
    /// The third SGR 0 site, and the one that was wrong on its own terms rather
    /// than only in combination with a session pin. SGR has no cheap "turn bold
    /// off and leave the colour alone", so an attribute change costs a reset --
    /// and a reset takes the colour with it. The encoder then compared the
    /// wanted colour against the *pre-reset* one, found them equal, emitted
    /// nothing, and drew the cell in whatever the terminal's default foreground
    /// happened to be. `active` went on to record a style the terminal did not
    /// have, so every cell after it in the run was wrong too.
    ///
    /// The existing test for this (`leaving_bold_does_not_take_the_colour_with_it`)
    /// used three *different* colours, so the colour was re-emitted for an
    /// unrelated reason and the assertion passed against the defect. This one
    /// holds the colour fixed across the boundary, which is the only shape that
    /// reaches it.
    #[test]
    fn a_colour_is_not_dropped_when_only_the_attribute_changes() {
        let ink = Color::Rgb { r: 1, g: 2, b: 3 };
        let cells = [
            (0usize, 0usize, Cell::new('x', ink, style::Attribute::Bold)),
            (1, 0, Cell::new('y', ink, style::Attribute::Reset)),
            (2, 0, Cell::new('z', ink, style::Attribute::Reset)),
        ];

        let mut out: Vec<u8> = Vec::new();
        write_cells(&mut out, (3, 1), &cells, SessionColors::default())
            .expect("writing to a Vec cannot fail");
        let encoded = String::from_utf8(out).expect("crossterm emits utf8");

        let mut terminal = Terminal::new(3, 1);
        terminal.replay(&encoded);

        for x in 0..3 {
            assert_eq!(
                terminal.at(x, 0).fg,
                ink,
                "cell {x} lost its colour across the attribute change, so it was \
                 drawn in the terminal's default foreground"
            );
        }
    }

    /// Every kind of cell the crate can produce, in one go, checked against the
    /// diff that produced it.
    #[test]
    fn every_cell_of_a_mixed_frame_lands_exactly_as_specified() {
        let styles = [
            style::Attribute::Reset,
            style::Attribute::Bold,
            style::Attribute::NormalIntensity,
        ];
        let colors = [
            Color::Reset,
            Color::Red,
            Color::White,
            Color::DarkGreen,
            rgb(1, 2, 3),
            rgb(250, 240, 230),
        ];

        let cells: Vec<(usize, usize, Cell)> = (0..60)
            .map(|i| {
                let x = i % 10;
                let y = i / 10;
                let attr = styles[i % styles.len()];
                let color = colors[i % colors.len()];
                // Every third cell carries a background, so runs of identical
                // foregrounds are interrupted by background changes and back.
                let cell = if i % 3 == 0 {
                    Cell::with_bg(
                        crate::render::halfblock::UPPER,
                        color,
                        colors[(i + 2) % colors.len()],
                        attr,
                    )
                } else {
                    Cell::new('#', color, attr)
                };
                (x, y, cell)
            })
            .collect();

        let terminal = painted((10, 6), &cells);

        for (x, y, cell) in &cells {
            let painted_cell = terminal.at(*x, *y);
            assert_eq!(painted_cell.symbol, cell.symbol, "glyph at ({x},{y})");
            assert_eq!(painted_cell.fg, cell.color, "foreground at ({x},{y})");
            assert_eq!(painted_cell.bg, cell.bg, "background at ({x},{y})");
            assert_eq!(
                painted_cell.bold,
                matches!(cell.attr, style::Attribute::Bold),
                "bold at ({x},{y})"
            );
        }
    }

    /// Cells the diff does not mention must be left exactly as they were, which
    /// is the whole premise of diffing.
    #[test]
    fn untouched_cells_keep_the_style_they_were_drawn_with() {
        let first: Vec<(usize, usize, Cell)> = (0..10)
            .map(|x| (x, 0, Cell::new('#', rgb(3, 4, 5), style::Attribute::Bold)))
            .collect();
        let second: Vec<(usize, usize, Cell)> =
            vec![(4, 0, Cell::new('@', rgb(9, 9, 9), style::Attribute::Bold))];

        let mut out = Vec::new();
        write_cells(&mut out, (20, 3), &first, SessionColors::default())
            .expect("vec write");
        write_cells(&mut out, (20, 3), &second, SessionColors::default())
            .expect("vec write");

        let mut terminal = Terminal::new(20, 3);
        terminal.replay(&String::from_utf8(out).expect("utf8"));

        for x in 0..10 {
            if x == 4 {
                continue;
            }
            assert_eq!(terminal.at(x, 0).symbol, '#', "cell {x} was disturbed");
            assert_eq!(
                terminal.at(x, 0).fg,
                rgb(3, 4, 5),
                "cell {x} was disturbed"
            );
        }
        assert_eq!(terminal.at(4, 0).symbol, '@');
        assert_eq!(terminal.at(4, 0).fg, rgb(9, 9, 9));
    }

    /// The style tracker starts every frame from a clean slate, so a frame can
    /// never inherit a stale idea of what the terminal is in.
    #[test]
    fn each_frame_ends_by_resetting_so_the_next_one_can_assume_it() {
        let out = encoded((10, 3), &[(0, 0, cell('x'))]);
        assert!(
            out.ends_with("\u{1b}[0m"),
            "the frame did not leave the terminal reset, got {out:?}"
        );
    }

    #[test]
    fn an_empty_diff_emits_nothing_at_all() {
        // Not even the trailing reset: there was nothing to reset.
        assert_eq!(encoded((10, 10), &[]), "");
    }

    #[test]
    fn writes_nothing_for_an_empty_diff() {
        assert_eq!(encoded((10, 10), &[]), "");
    }

    #[test]
    fn encodes_a_cursor_move_and_the_styled_glyph() {
        let out = encoded((10, 10), &[(3, 4, cell('x'))]);
        assert!(
            out.contains("\u{1b}[5;4H"),
            "expected a move to row 5 column 4, got {out:?}"
        );
        assert!(out.contains('x'), "expected the glyph, got {out:?}");
    }

    #[test]
    fn drops_cells_outside_the_terminal() {
        // A diff produced against a stale previous frame can report coordinates
        // from the old size. Those must not reach the terminal.
        let cells = [
            (0usize, 0usize, cell('a')),
            (99, 0, cell('b')),
            (0, 99, cell('c')),
        ];
        let out = encoded((10, 10), &cells);

        assert!(out.contains('a'), "the in-bounds cell was dropped");
        assert!(!out.contains('b'), "a cell past the right edge was written");
        assert!(
            !out.contains('c'),
            "a cell past the bottom edge was written"
        );
    }

    #[test]
    fn the_last_row_and_column_are_inside_the_terminal() {
        // Off-by-one guard: index 9 of a 10-wide terminal is the last column,
        // not out of bounds.
        let cells = [(9usize, 9usize, cell('z'))];
        let out = encoded((10, 10), &cells);

        assert!(
            out.contains('z'),
            "the last cell was treated as out of bounds"
        );
    }

    #[test]
    fn encodes_every_cell_in_a_dense_diff() {
        let cells: Vec<(usize, usize, Cell)> =
            (0..100).map(|i| (i % 10, i / 10, cell('#'))).collect();
        let out = encoded((10, 10), &cells);

        assert_eq!(out.matches('#').count(), 100);
    }

    /// A uniform full-screen repaint should cost one style change and one cursor
    /// move per row, and one byte per glyph otherwise.
    ///
    /// This bound used to be satisfied only because the encoder skipped the
    /// colour on every cell after the first, which is the bug. It passes now for
    /// the right reason, and `every_cell_of_a_uniform_run_lands_in_its_own_colour`
    /// is what pins the correctness the byte count no longer implies.
    #[test]
    fn a_full_screen_diff_of_one_style_stays_within_budget() {
        let cells: Vec<(usize, usize, Cell)> = (0..200 * 50)
            .map(|i| (i % 200, i / 200, cell('#')))
            .collect();
        let out = encoded((200, 50), &cells);

        assert_eq!(out.matches('#').count(), 200 * 50);
        assert!(
            out.len() < 200 * 50 * 2,
            "a full-screen repaint of uniformly styled cells encoded to {} bytes, \
             which is more than the two bytes per glyph it needs",
            out.len()
        );
        assert_eq!(
            count_moves(&out),
            50,
            "expected one cursor move per row, got {}",
            count_moves(&out)
        );
    }

    /// Changing only the colour must not cost a reset. A reset clears the
    /// colours, so a stream that leans on one is a stream paying to set a colour
    /// it has just cleared.
    #[test]
    fn a_colour_change_alone_does_not_cost_a_reset() {
        let cells: Vec<(usize, usize, Cell)> = (0..10)
            .map(|x| {
                (
                    x,
                    0,
                    Cell::new('#', rgb(x as u8, 0, 0), style::Attribute::Reset),
                )
            })
            .collect();
        let out = encoded((20, 5), &cells);

        // Two: one to establish the baseline at the start of the frame, one to
        // close it. None at all for the nine colour changes in between.
        assert_eq!(
            count_resets(&out),
            2,
            "colour changes are paying for an SGR reset each, got {out:?}"
        );
    }

    /// The first cell of a frame establishes the style rather than assuming it.
    ///
    /// Entering the alternate screen does not reset SGR, so a terminal can still
    /// be wearing whatever style the shell left it in. A frame whose first cell
    /// wants the default colours has to say so, or it inherits the shell's.
    #[test]
    fn the_first_cell_of_a_frame_does_not_assume_the_terminals_style() {
        // A cell that wants exactly the reset state, so the only way it can come
        // out right is if the frame established the baseline itself.
        let out = encoded(
            (10, 3),
            &[(2, 1, Cell::new('x', Color::Reset, style::Attribute::Reset))],
        );

        let mut terminal = Terminal::new(10, 3);
        terminal.replay(&out);
        let painted_cell = terminal.at(2, 1);

        assert_eq!(painted_cell.symbol, 'x');
        assert_eq!(painted_cell.fg, Color::Reset);
        assert_eq!(painted_cell.bg, Color::Reset);

        // The reset has to land before the glyph, not merely somewhere in the
        // frame: a reset after the glyph would leave it in the shell's colour.
        let reset = out
            .find("\u{1b}[0m")
            .expect("the frame emitted no baseline");
        let glyph = out.find('x').expect("the frame emitted no glyph");
        assert!(
            reset < glyph,
            "the baseline reset came after the glyph, so the glyph was drawn in \
             whatever style the terminal already had: {out:?}"
        );
    }

    #[test]
    fn a_row_of_adjacent_cells_costs_one_cursor_move() {
        let cells: Vec<(usize, usize, Cell)> =
            (0..10).map(|x| (x, 0, cell('#'))).collect();
        let out = encoded((20, 5), &cells);

        assert_eq!(
            count_moves(&out),
            1,
            "expected a single cursor move for ten adjacent cells, got {out:?}"
        );
    }

    #[test]
    fn a_gap_in_a_row_forces_a_new_cursor_move() {
        let cells = [(0usize, 0usize, cell('#')), (5, 0, cell('#'))];
        let out = encoded((20, 5), &cells);

        assert_eq!(
            count_moves(&out),
            2,
            "expected a cursor move per cell across the gap, got {out:?}"
        );
    }

    #[test]
    fn wrapping_to_the_next_row_forces_a_new_cursor_move() {
        // The last column of a row is not followed by the first column of the
        // next one, even though the x coordinate appears to continue.
        let cells = [(4usize, 0usize, cell('#')), (0, 1, cell('#'))];
        let out = encoded((5, 5), &cells);

        assert_eq!(
            count_moves(&out),
            2,
            "expected a cursor move per row, got {out:?}"
        );
    }

    #[test]
    fn a_style_change_is_emitted_even_mid_row() {
        let plain = Cell::new('#', Color::Red, style::Attribute::Bold);
        let other = Cell::new('#', Color::Blue, style::Attribute::Bold);
        let cells = [(0usize, 0usize, plain), (1, 0, other), (2, 0, plain)];
        let out = encoded((10, 5), &cells);

        // Still a single cursor move: only the style changed, not the position.
        assert_eq!(count_moves(&out), 1, "got {out:?}");
        // But every cell re-states its colour, because it alternates.
        assert_eq!(out.matches('#').count(), 3, "got {out:?}");
    }
}
