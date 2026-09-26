use std::process::Command;
use std::time::Duration;

use termzzz::ascii::{AsciiField, AsciiFieldOptions, GlyphPalette};
use termzzz::buffer::Cell;
use termzzz::common::{
    self, TerminalEffect, TickClock, run_loop_with_source_and_size,
};
use termzzz::config::Config;
use termzzz::registry::{AnyEffect, EffectId};
use termzzz::runtime::{
    FrameContext, InputEvent, InputSource, InputState, Key, KeyPhase,
    PointerButton, PointerPhase,
};

#[test]
fn input_state_tracks_key_and_pointer_interaction() {
    let mut state = InputState::default();

    state.apply(InputEvent::Key {
        key: Key::Char('r'),
        phase: KeyPhase::Pressed,
    });
    assert!(state.is_key_pressed(Key::Char('r')));

    state.apply(InputEvent::Pointer {
        position: (3, 4),
        phase: PointerPhase::Pressed,
        button: PointerButton::Left,
    });
    state.apply(InputEvent::Pointer {
        position: (5, 6),
        phase: PointerPhase::Moved,
        button: PointerButton::Left,
    });

    assert_eq!(state.pointer().position, (5, 6));
    assert_eq!(state.pointer().previous, Some((3, 4)));
    assert_eq!(state.pointer().delta, (2.0, 2.0));
    assert!(state.pointer().pressed);

    state.apply(InputEvent::Pointer {
        position: (5, 6),
        phase: PointerPhase::Released,
        button: PointerButton::Left,
    });
    assert!(!state.pointer().pressed);
}

#[test]
fn frame_context_carries_runtime_state() {
    let input = InputState::default();
    let context = FrameContext::new(
        (80, 24),
        7,
        Duration::from_millis(350),
        Duration::from_millis(16),
        input.clone(),
    );

    assert_eq!(context.size, (80, 24));
    assert_eq!(context.frame, 7);
    assert_eq!(context.elapsed, Duration::from_millis(350));
    assert_eq!(context.delta, Duration::from_millis(16));
    assert_eq!(context.input.pointer().position, (0, 0));
}

#[test]
fn input_state_bounds_held_keys_per_frame() {
    let mut state = InputState::default();
    state.apply(InputEvent::Key {
        key: Key::Char('a'),
        phase: KeyPhase::Pressed,
    });

    state.begin_frame();

    assert!(!state.is_key_pressed(Key::Char('a')));
}

#[test]
fn ascii_field_is_deterministic_and_bounds_safe() {
    let options = AsciiFieldOptions {
        seed: 42,
        ..Default::default()
    };
    let mut first = AsciiField::new(options.clone(), (20, 10));
    let mut second = AsciiField::new(options, (20, 10));

    let first_frame = first.get_diff();
    let second_frame = second.get_diff();

    assert_eq!(first_frame, second_frame);
    assert!(!first_frame.is_empty());
    assert!(first_frame.iter().all(|(x, y, _)| *x < 20 && *y < 10));
}

#[test]
fn ascii_field_accepts_interactive_input() {
    let options = AsciiFieldOptions {
        seed: 7,
        ..Default::default()
    };
    let mut field = AsciiField::new(options, (20, 10));

    field.handle_input(&InputEvent::Pointer {
        position: (5, 5),
        phase: PointerPhase::Pressed,
        button: PointerButton::Left,
    });
    field.update();

    assert!(field.get_diff().iter().all(|(x, y, _)| *x < 20 && *y < 10));
}

#[test]
fn ascii_field_reseeds_on_r() {
    let options = AsciiFieldOptions {
        seed: 9,
        ..Default::default()
    };
    let mut field = AsciiField::new(options, (20, 10));

    let first = field.get_diff();
    field.handle_input(&InputEvent::Key {
        key: Key::Char('r'),
        phase: KeyPhase::Pressed,
    });
    let second = field.get_diff();
    field.handle_input(&InputEvent::Key {
        key: Key::Char('r'),
        phase: KeyPhase::Pressed,
    });
    let third = field.get_diff();

    assert_ne!(first, second);
    assert!(!third.is_empty());
}

struct ScriptedInput {
    events: Vec<InputEvent>,
}

impl InputSource for ScriptedInput {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<Vec<InputEvent>> {
        if self.events.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![self.events.remove(0)])
        }
    }
}

struct BatchInput {
    events: Option<Vec<InputEvent>>,
}

impl BatchInput {
    fn with(events: Vec<InputEvent>) -> Self {
        Self {
            events: Some(events),
        }
    }
}

impl InputSource for BatchInput {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<Vec<InputEvent>> {
        Ok(self.events.take().unwrap_or_default())
    }
}

/// An input source that records the timeout it was asked to wait for, and
/// faithfully blocks for it, so a test can tell whether the loop is willing to
/// spend frame budget on waiting.
struct RecordingInput {
    timeouts: Vec<Duration>,
    /// Set to hand over one event per poll, so a test can check that polling
    /// without a timeout still delivers input.
    pending: Vec<InputEvent>,
    /// Polls remaining before quitting, so the loop terminates.
    quit_after: Option<usize>,
    polls: usize,
}

impl RecordingInput {
    fn new() -> Self {
        Self {
            timeouts: Vec::new(),
            pending: Vec::new(),
            quit_after: None,
            polls: 0,
        }
    }
}

impl InputSource for RecordingInput {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<Vec<InputEvent>> {
        self.timeouts.push(timeout);
        self.polls += 1;
        if let Some(remaining) = self.quit_after
            && self.polls > remaining
        {
            return Ok(vec![InputEvent::Quit]);
        }
        // A real poll blocks for up to `timeout`. Reproducing that is the point:
        // the cost only exists if the source is asked to wait.
        if !timeout.is_zero() {
            std::thread::sleep(timeout);
        }
        Ok(if self.pending.is_empty() {
            Vec::new()
        } else {
            vec![self.pending.remove(0)]
        })
    }
}

/// The screensaver loop must never ask the input source to block.
///
/// It used to ask for up to ten milliseconds on every frame. That is most of a
/// 60 Hz frame budget spent asleep, and because the block sat *inside* the
/// measured frame, the delta handed to the effect claimed ten milliseconds had
/// passed while the program did nothing at all. The visible cost was that a
/// heavy effect could not fit in what was left of the frame, plus up to ten
/// milliseconds of extra latency on every keystroke.
#[test]
fn the_frame_loop_never_blocks_waiting_for_input() {
    let mut source = RecordingInput::new();
    source.quit_after = Some(8);
    let mut effect = ResizeEffect {
        resets: 0,
        last_size: None,
    };

    termzzz::common::run_loop_with_source_and_size_and_target(
        &mut Vec::new(),
        &mut termzzz::common::SingleEffect::new(&mut effect),
        None,
        &mut source,
        (20, 6),
        termzzz::common::RuntimeOptions::default(),
    )
    .expect("the loop runs without a terminal");

    assert!(
        !source.timeouts.is_empty(),
        "the loop never polled for input, so this test proved nothing"
    );
    assert!(
        source.timeouts.iter().all(|timeout| timeout.is_zero()),
        "the loop asked the source to block for {:?}. Every poll should be \
         non-blocking: the loop already runs sixty times a second, so the worst a \
         keystroke waits is one frame, which is better than ten milliseconds plus \
         a frame",
        source.timeouts
    );
}

/// Polling without a timeout must not cost input delivery.
///
/// The obvious risk of making the poll non-blocking is that events arriving while
/// the loop is asleep are missed. They are not: they sit in the terminal's queue
/// and the next poll drains them. This is the test for that.
#[test]
fn non_blocking_polls_still_deliver_input() {
    struct Counting {
        keys: usize,
    }

    impl TerminalEffect for Counting {
        fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
            Vec::new()
        }

        fn update(&mut self) {}

        fn update_size(&mut self, _width: u16, _height: u16) {}

        fn reset(&mut self) {}

        fn handle_input(&mut self, _event: &InputEvent) {
            self.keys += 1;
        }
    }

    let mut source = RecordingInput::new();
    for _ in 0..3 {
        source.pending.push(InputEvent::Key {
            key: Key::Char('z'),
            phase: KeyPhase::Pressed,
        });
    }
    source.quit_after = Some(8);
    let mut effect = Counting { keys: 0 };

    termzzz::common::run_loop_with_source_and_size_and_target(
        &mut Vec::new(),
        &mut termzzz::common::SingleEffect::new(&mut effect),
        None,
        &mut source,
        (20, 6),
        termzzz::common::RuntimeOptions::default(),
    )
    .expect("the loop runs without a terminal");

    assert_eq!(
        effect.keys, 3,
        "three keypresses were queued and the effect saw {} of them; a \
         non-blocking poll must still drain what is waiting",
        effect.keys
    );
}

/// Work inside the frame budget must not lengthen the run.
///
/// The loop used to sleep for "whatever is left of the last frame", measured
/// before the sleep itself. An oversleep therefore could not be repaid: it was
/// measured, discarded, and the next frame slept its own remainder on top of it.
/// A frame that consistently overran its slot drifted further and further from
/// the cadence, and the delta every time-integrating effect was handed drifted
/// with it. That is visible as stutter in anything that integrates `delta`.
#[test]
fn a_frames_cost_does_not_compound_into_the_next_one() {
    /// Burns a fixed amount of wall clock per frame, which is the case the old
    /// pacing got wrong: a constant overshoot, frame after frame.
    struct SlowEffect {
        cost: Duration,
        deltas: Vec<Duration>,
    }

    impl TerminalEffect for SlowEffect {
        fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
            let until = std::time::Instant::now() + self.cost;
            while std::time::Instant::now() < until {
                std::hint::spin_loop();
            }
            Vec::new()
        }

        fn update(&mut self) {}

        fn update_size(&mut self, _width: u16, _height: u16) {}

        fn reset(&mut self) {}

        fn update_with_context(
            &mut self,
            context: &termzzz::runtime::FrameContext,
        ) {
            self.deltas.push(context.delta);
        }
    }

    let frame = Duration::from_secs_f64(1.0 / 60.0);
    let frames = 24usize;

    let run = |cost: Duration| {
        let mut effect = SlowEffect {
            cost,
            deltas: Vec::new(),
        };
        let started = std::time::Instant::now();
        termzzz::common::run_loop_with_source_and_size(
            &mut Vec::new(),
            &mut effect,
            Some(frames),
            &mut BatchInput::with(Vec::new()),
            (20, 6),
        )
        .expect("the loop runs without a terminal");
        (started.elapsed(), effect.deltas)
    };

    // A third of the budget, spent inside the frame. The loop should absorb it:
    // the frame is still scheduled at a fixed cadence, so the run takes the same
    // wall clock and the effect sees the same steady delta.
    let (slow_elapsed, slow_deltas) = run(frame / 3);
    let (fast_elapsed, fast_deltas) = run(Duration::ZERO);

    let ideal = frame * frames as u32;
    assert!(
        fast_elapsed >= ideal / 2 && fast_elapsed < ideal * 2,
        "{frames} frames with no work took {fast_elapsed:?}, which is not near \
         the {ideal:?} a 60 Hz cadence asks for"
    );

    let drift = slow_elapsed
        .checked_sub(fast_elapsed)
        .unwrap_or(Duration::ZERO);
    assert!(
        drift < ideal / 4,
        "spending {:?} per frame inside a {frame:?} budget stretched {frames} \
         frames by {drift:?}; the cost should be absorbed, not compounded",
        frame / 3
    );

    // And the deltas the effect was handed stayed near the frame time instead of
    // creeping, which is what a drifting loop looks like from inside an effect.
    let mean = |deltas: &[Duration]| -> Duration {
        let total: Duration = deltas.iter().sum();
        total / deltas.len() as u32
    };
    let slow_mean = mean(&slow_deltas);
    let fast_mean = mean(&fast_deltas);
    assert!(
        slow_mean < frame * 2,
        "with a third of the budget spent per frame the mean delta was \
         {slow_mean:?}, so the loop is drifting rather than holding its cadence"
    );
    assert!(
        slow_mean < fast_mean + frame,
        "the mean delta grew from {fast_mean:?} to {slow_mean:?} once each frame \
         had work to do, so the per-frame cost is being added on top of the wait \
         rather than absorbed by it"
    );
}

/// An input source that can report a focus change partway through, and that
/// stops the loop on a deadline.
///
/// The stop is wall-clock rather than a poll count on purpose: these tests exist
/// to measure frame rates, and a poll budget at four frames a second takes fifty
/// seconds to spend.
struct FocusInput {
    pending: Vec<InputEvent>,
    /// Stop once this many polls have happened.
    quit_after: Option<usize>,
    /// Stop once this much wall clock has passed, measured from construction.
    stop_after: Option<Duration>,
    started: std::time::Instant,
    polls: usize,
}

impl FocusInput {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
            quit_after: None,
            stop_after: None,
            started: std::time::Instant::now(),
            polls: 0,
        }
    }

    /// Hands over one event per poll, then stops after `seconds` of wall clock.
    fn scripted(pending: Vec<InputEvent>, seconds: f64) -> Self {
        Self {
            pending,
            quit_after: None,
            stop_after: Some(Duration::from_secs_f64(seconds)),
            started: std::time::Instant::now(),
            polls: 0,
        }
    }
}

impl InputSource for FocusInput {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<Vec<InputEvent>> {
        self.polls += 1;
        if let Some(remaining) = self.quit_after
            && self.polls > remaining
        {
            return Ok(vec![InputEvent::Quit]);
        }
        if let Some(limit) = self.stop_after
            && self.polls > 1
            && self.started.elapsed() >= limit
        {
            return Ok(vec![InputEvent::Quit]);
        }
        Ok(if self.pending.is_empty() {
            Vec::new()
        } else {
            vec![self.pending.remove(0)]
        })
    }
}

/// Records the delta of every frame it is asked to advance.
struct FocusEffect {
    deltas: Vec<Duration>,
}

impl TerminalEffect for FocusEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        Vec::new()
    }

    fn update(&mut self) {}

    fn update_size(&mut self, _width: u16, _height: u16) {}

    fn reset(&mut self) {}

    fn update_with_context(&mut self, context: &termzzz::runtime::FrameContext) {
        self.deltas.push(context.delta);
    }
}

/// Runs the loop over `effect` with a focus policy.
///
/// The `SingleEffect` has to be dropped before the caller can read `effect`,
/// which is why this is a function rather than four copies of the same block.
fn run_with_focus_policy(
    effect: &mut FocusEffect,
    source: &mut FocusInput,
    options: termzzz::common::RuntimeOptions,
) -> std::io::Result<f64> {
    let mut target = termzzz::common::SingleEffect::new(effect);
    termzzz::common::run_loop_with_source_and_size_and_options(
        &mut Vec::new(),
        &mut target,
        None,
        source,
        (40, 12),
        options,
    )
}

/// Runs for a fixed number of seconds and reports what the loop did.
struct Measured {
    seconds: f64,
    frames: usize,
    deltas: Vec<Duration>,
}

fn measure(pending: Vec<InputEvent>, policy: bool, idle_fps: f32) -> Measured {
    let mut source = FocusInput::scripted(pending, 2.0);
    let mut effect = FocusEffect { deltas: Vec::new() };

    let started = std::time::Instant::now();
    run_with_focus_policy(
        &mut effect,
        &mut source,
        termzzz::common::RuntimeOptions::default()
            .with_focus_policy(policy, idle_fps),
    )
    .expect("the loop runs without a terminal");

    Measured {
        seconds: started.elapsed().as_secs_f64(),
        frames: source.polls,
        deltas: effect.deltas,
    }
}

/// An unfocused terminal must cost dramatically less.
///
/// This is the whole point of the feature, so it is asserted as a rate rather
/// than as an implementation detail: whatever the loop does internally, an
/// unfocused window has to run at a small fraction of the focused rate.
#[test]
fn an_unfocused_terminal_runs_at_a_fraction_of_the_frame_rate() {
    let focused = measure(Vec::new(), true, 4.0);
    let idle = measure(vec![InputEvent::FocusLost], true, 4.0);

    assert!(
        focused.frames > 20,
        "only {} focused frames in two seconds, so this is not measuring \
         anything",
        focused.frames
    );
    assert!(
        idle.frames >= 4,
        "only {} unfocused frames in two seconds; the throttle is slower than \
         expected or the deadline is not being honoured",
        idle.frames
    );

    let focused_fps = focused.frames as f64 / focused.seconds;
    let idle_fps = idle.frames as f64 / idle.seconds;

    assert!(
        focused_fps > 40.0,
        "the focused loop only managed {focused_fps:.1} fps"
    );
    assert!(
        idle_fps < focused_fps / 5.0,
        "unfocused ran at {idle_fps:.1} fps against {focused_fps:.1} focused, \
         which is not the saving it is supposed to be"
    );
    assert!(
        (2.0..=7.0).contains(&idle_fps),
        "unfocused ran at {idle_fps:.1} fps, nowhere near the configured 4"
    );
}

/// The frames it does draw cost nothing to simulate.
///
/// The simulation is frozen rather than fed a coarser delta. At four frames a
/// second a real delta is 250 ms, which the frame-delta clamp would cut to 50,
/// so the effect would run at a fifth of its speed and stay there.
#[test]
fn an_unfocused_effect_is_frozen_rather_than_slowed() {
    let idle = measure(vec![InputEvent::FocusLost], true, 4.0);
    assert!(
        idle.deltas.is_empty(),
        "the effect was advanced {} times while unfocused, each with a delta of \
         {:?}; it should not be advanced at all",
        idle.deltas.len(),
        idle.deltas.first()
    );

    let focused = measure(Vec::new(), true, 4.0);
    assert!(
        focused.deltas.len() > 20,
        "a focused run only advanced the effect {} times, so the freeze is not \
         being distinguished from a broken loop",
        focused.deltas.len()
    );
}

/// The rate has to be the configured one, not merely a smaller one.
#[test]
fn the_unfocused_frame_rate_is_the_configured_one() {
    let slow = measure(vec![InputEvent::FocusLost], true, 2.0);
    let fast = measure(vec![InputEvent::FocusLost], true, 8.0);

    let slow_fps = slow.frames as f64 / slow.seconds;
    let fast_fps = fast.frames as f64 / fast.seconds;

    assert!(
        (1.0..=3.5).contains(&slow_fps),
        "asked for 2 fps unfocused and got {slow_fps:.1}"
    );
    assert!(
        fast_fps > slow_fps * 1.5,
        "asked for 8 fps and got {fast_fps:.1}, against {slow_fps:.1} at 2 fps, \
         so the configured rate is not what is being used"
    );
}

/// Turning the policy off has to restore the full rate.
///
/// Otherwise there is no way to run at full speed on a terminal that never
/// reports focus changes, which is the situation the policy exists to survive.
#[test]
fn the_focus_policy_can_be_turned_off() {
    let measured = measure(vec![InputEvent::FocusLost], false, 4.0);
    let fps = measured.frames as f64 / measured.seconds;

    assert!(
        fps > 40.0,
        "with the policy off an unfocused terminal still ran at {fps:.1} fps"
    );
    assert!(
        measured.deltas.len() > 20,
        "with the policy off the effect was advanced only {} times, so the \
         simulation is frozen even though the policy is off",
        measured.deltas.len()
    );
}

/// Regaining focus must not teleport the simulation forward.
///
/// The wall clock runs on while the simulation is frozen, so the first focused
/// frame is handed the whole absence as one delta unless the loop resets its
/// frame clock. The effect would visibly jump, which is the artefact this
/// feature would otherwise introduce.
#[test]
fn returning_to_focus_does_not_jump_the_simulation_forward() {
    // Lose focus, wait out a few throttled frames, get it back, then let the
    // deadline end the run while focused.
    let mut source = FocusInput::new();
    source.pending = vec![
        InputEvent::FocusLost,
        InputEvent::FocusGained,
        InputEvent::FocusGained,
    ];
    source.stop_after = Some(Duration::from_millis(1500));
    let mut effect = FocusEffect { deltas: Vec::new() };

    run_with_focus_policy(
        &mut effect,
        &mut source,
        termzzz::common::RuntimeOptions::default().with_focus_policy(true, 4.0),
    )
    .expect("the loop runs without a terminal");

    assert!(
        !effect.deltas.is_empty(),
        "the effect was never advanced, so nothing regained focus; the test is \
         not exercising the resume path"
    );

    // Every delta after the resume is one frame, not the whole absence. The
    // absence is bounded by the clamp either way, so what matters is that it is
    // not the *whole* stretch and that the resumed frames are frame-sized.
    let longest = effect.deltas.iter().copied().max().unwrap();
    assert!(
        longest <= Duration::from_millis(60),
        "a frame was handed a delta of {longest:?} after focus came back, so the \
         absence was paid out in one lump rather than discarded"
    );
}

/// Quitting has to work while unfocused.
///
/// The loop polls as rarely as once a second at its slowest, so this is the one
/// interaction that must not be lost. A user who alt-tabs away and then closes
/// the window should not have to come back to quit.
#[test]
fn quitting_works_while_unfocused() {
    let mut source = FocusInput::new();
    // The first poll reports the focus loss, the second reports the quit -- so
    // the quit arrives while unfocused, at the slowest rate allowed.
    source.pending = vec![InputEvent::FocusLost];
    source.quit_after = Some(1);
    let mut effect = FocusEffect { deltas: Vec::new() };

    let started = std::time::Instant::now();
    let result = run_with_focus_policy(
        &mut effect,
        &mut source,
        termzzz::common::RuntimeOptions::default().with_focus_policy(true, 1.0),
    );

    assert!(result.is_ok(), "the loop returned an error while unfocused");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "quitting took {:?} while unfocused, so the key was effectively lost at \
         the slowest frame rate",
        started.elapsed()
    );
}

/// The unfocused rate is clamped, because a config of `0` would mean "stop".
///
/// Stopping is the cheapest option and the most dangerous one: a terminal that
/// never reports a focus change would leave the program frozen with no way back.
#[test]
fn the_unfocused_rate_is_clamped_away_from_stopping() {
    for (requested, expected_floor) in
        [(0.0f32, 1.0f32), (-5.0, 1.0), (0.001, 1.0), (1000.0, 30.0)]
    {
        let options = termzzz::common::RuntimeOptions::default()
            .with_focus_policy(true, requested);
        assert!(
            (termzzz::common::MIN_IDLE_FPS..=termzzz::common::MAX_IDLE_FPS)
                .contains(&options.idle_fps),
            "asked for {requested} fps unfocused and got {}",
            options.idle_fps
        );
        assert!(
            options.idle_fps >= expected_floor,
            "asked for {requested} fps and got {}, which is below the floor",
            options.idle_fps
        );
    }
}

struct ResizeEffect {
    resets: usize,
    last_size: Option<(u16, u16)>,
}

impl TerminalEffect for ResizeEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        Vec::new()
    }

    fn update(&mut self) {}

    fn update_size(&mut self, width: u16, height: u16) {
        self.last_size = Some((width, height));
    }

    fn reset(&mut self) {
        self.resets += 1;
    }
}

#[test]
fn runtime_loop_coalesces_resize_events() {
    let mut effect = ResizeEffect {
        resets: 0,
        last_size: None,
    };
    let mut input = BatchInput {
        events: Some(vec![
            InputEvent::Resize { size: (12, 8) },
            InputEvent::Resize { size: (20, 10) },
        ]),
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(1),
        &mut input,
        (30, 12),
    )
    .unwrap();

    assert_eq!(effect.resets, 1);
    assert_eq!(effect.last_size, Some((20, 10)));
}

struct RecordingEffect {
    frames: usize,
    last_size: Option<(u16, u16)>,
    context_size: Option<(u16, u16)>,
    context_frame: Option<u64>,
}

impl TerminalEffect for RecordingEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        Vec::new()
    }

    fn update(&mut self) {
        self.frames += 1;
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.last_size = Some((width, height));
    }

    fn reset(&mut self) {}

    fn get_diff_with_context(
        &mut self,
        context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        self.context_size = Some(context.size);
        self.context_frame = Some(context.frame);
        Vec::new()
    }
}

#[test]
fn runtime_loop_uses_context_and_resize_events() {
    let mut effect = RecordingEffect {
        frames: 0,
        last_size: None,
        context_size: None,
        context_frame: None,
    };
    let mut input = ScriptedInput {
        events: vec![InputEvent::Resize { size: (12, 8) }],
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(2),
        &mut input,
        (20, 10),
    )
    .unwrap();

    assert_eq!(effect.frames, 2);
    assert_eq!(effect.last_size, Some((12, 8)));
    assert_eq!(effect.context_size, Some((12, 8)));
    assert_eq!(effect.context_frame, Some(1));
}

#[test]
fn runtime_loop_clamps_small_resize_for_effects() {
    let mut effect = RecordingEffect {
        frames: 0,
        last_size: None,
        context_size: None,
        context_frame: None,
    };
    let mut input = ScriptedInput {
        events: vec![InputEvent::Resize { size: (1, 1) }],
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(1),
        &mut input,
        (10, 10),
    )
    .unwrap();

    assert_eq!(effect.last_size, Some((6, 6)));
}

#[test]
fn runtime_loop_exits_before_rendering_after_quit() {
    let mut effect = RecordingEffect {
        frames: 0,
        last_size: None,
        context_size: None,
        context_frame: None,
    };
    let mut input = ScriptedInput {
        events: vec![InputEvent::Quit],
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(3),
        &mut input,
        (20, 10),
    )
    .unwrap();

    assert_eq!(effect.frames, 0);
    assert!(output.is_empty());
}

#[test]
fn package_name_is_termzzz() {
    assert_eq!(env!("CARGO_PKG_NAME"), "termzzz");
}

#[test]
fn cli_help_uses_termzzz_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_termzzz"))
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("termzzz [EFFECT] [OPTIONS]"));
    assert!(stdout.contains("~/.config/termzzz.toml"));
}

#[test]
fn unknown_effect_reports_the_name_exactly_once() {
    let output = Command::new(env!("CARGO_BIN_EXE_termzzz"))
        .arg("notaneffect")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The rejected name appears once, and the message is not doubled up by the
    // caller prefixing a parse error that already explains itself.
    assert!(
        stdout.contains("Unknown effect: notaneffect"),
        "got {stdout}"
    );
    assert_eq!(
        stdout.matches("notaneffect").count(),
        1,
        "the rejected name was reported more than once: {stdout}"
    );
    assert!(
        !stdout.contains("Unknown screen saver: Unknown effect"),
        "the parse error was prefixed again: {stdout}"
    );
}

#[test]
fn partial_ascii_config_uses_defaults() {
    let config: termzzz::config::Config =
        toml::from_str("[ascii]\ntime_scale = 2.0\n").unwrap();

    assert_eq!(config.ascii.time_scale, 2.0);
    assert_eq!(config.ascii.seed, 42);
}

#[test]
fn global_speed_defaults_to_one_and_is_configurable() {
    assert_eq!(Config::default().global.speed, 1.0);

    let config: Config = toml::from_str("[global]\nspeed = 0.4\n").unwrap();
    assert_eq!(config.global.speed, 0.4);
    assert_eq!(config.get_ascii_options().seed, 42);
}

#[test]
fn glyph_palette_rejects_wide_and_control_characters() {
    let palette = GlyphPalette::new("😀\n x", Vec::new());

    assert_eq!(palette.sample(0.0).symbol, ' ');
    assert_eq!(palette.sample(1.0).symbol, 'x');
}

#[test]
fn check_mode_rejects_unknown_effects() {
    assert!(
        termzzz::check::run_test_for_effect("unknown", 1, &Config::default(), 1.0)
            .is_err()
    );
}

#[test]
fn registry_builds_every_effect() {
    let config = Config::default();

    for id in EffectId::all() {
        let mut effect = AnyEffect::build(id, &config, (20, 10));
        assert_eq!(effect.id(), id);
        let _ = effect.get_diff();
    }
}

#[test]
fn registry_ids_round_trip_through_strings() {
    for id in EffectId::all() {
        let text = id.as_str();
        assert_eq!(text.parse::<EffectId>().unwrap(), id);
    }
    assert!("nope".parse::<EffectId>().is_err());
}

#[test]
fn unknown_effect_errors_name_the_rejected_value() {
    let error = "nope".parse::<EffectId>().unwrap_err();

    assert_eq!(error, "Unknown effect: nope");
}

#[test]
fn dvd_is_registered_with_a_distinct_name() {
    let dvd = EffectId::Dvd.as_str();
    assert_eq!(dvd, "dvd");
    assert!(EffectId::all().any(|id| id == EffectId::Dvd));
    assert!(!EffectId::Dvd.needs_mouse());
    assert!(EffectId::Ascii.needs_mouse());
}

#[test]
fn playlist_config_parses_effects_and_transitions() {
    let config: Config = toml::from_str(
        r#"
[playlist]
shuffle = true
transition = 1.2

[[playlist.effects]]
effect = "matrix"
duration = 4.0

[[playlist.effects]]
effect = "dvd"
"#,
    )
    .unwrap();

    let options = config.get_playlist_options();
    assert!(options.shuffle);
    assert_eq!(options.transition, 1.2);
    assert_eq!(options.effects.len(), 2);
    assert_eq!(options.effects[0].effect, "matrix");
    assert_eq!(options.effects[0].duration, Some(4.0));
    assert_eq!(options.effects[1].duration, None);
}

#[test]
fn playlist_defaults_to_a_short_transition() {
    let options = Config::default().get_playlist_options();
    assert_eq!(options.transition, 0.6);
    assert!(!options.shuffle);
    assert!(options.effects.is_empty());
}

/// The default logo is a block letter, not three characters on a line.
///
/// Was `"DVD"` -- a text label rather than a logo, and a one-row slab stepping a
/// whole cell at a time is far more obviously a staircase than a taller one. The
/// name of this test kept its meaning; the assertion follows the effect.
#[test]
fn dvd_options_default_to_a_block_letter_logo() {
    let options = Config::default().get_dvd_options();
    assert!(
        options.logo.contains('\n'),
        "the default logo is still one line: {:?}",
        options.logo
    );
    assert!(
        options.logo.lines().count() >= 5,
        "the default logo is {} rows tall",
        options.logo.lines().count()
    );
    let widest = options.logo.lines().map(str::len).max().unwrap_or(0);
    assert!(
        widest >= 15,
        "the default logo is {widest} cells wide, which is a label"
    );
    // A multi-row logo only reaches the screen if the parser splits on newlines,
    // which it always did -- but nothing asserted that until the default started
    // relying on it.
    let rendered = toml::to_string_pretty(&Config::default()).unwrap();
    assert!(
        rendered.contains("[dvd]"),
        "the dvd section is missing from the rendered config"
    );
    assert!(options.corner_color_change);
    // And the speed has to be high enough that the drawn position actually
    // changes between frames, which is the whole "not a staircase" requirement.
    assert!(
        options.speed >= 20.0,
        "the default speed is {}, which is slow enough for the logo to sit still \
         for several frames between steps",
        options.speed
    );
    assert!(
        options.slope > 1.0,
        "the default slope is {}, which draws equal cell deltas and so a line at \
         50 to 63 degrees rather than a diagonal",
        options.slope
    );
}

#[test]
fn default_config_serializes_with_global_and_playlist_sections() {
    let rendered = toml::to_string_pretty(&Config::default()).unwrap();

    assert!(rendered.contains("[global]"));
    assert!(rendered.contains("[playlist]"));
    assert!(rendered.contains("[dvd]"));
}

#[test]
fn speed_clock_scales_simulation_ticks() {
    let mut normal = TickClock::default();
    let mut slow = TickClock::default();
    let delta = Duration::from_millis(50);

    assert_eq!(normal.advance(delta, 1.0), 3);
    assert_eq!(slow.advance(delta, 0.5), 1);
}

#[test]
fn legacy_effect_defaults_use_calmer_pacing() {
    let config = Config::default();

    // Plasma's `color_speed` was 20 here, which with its `time_scale` of 0.5
    // moved the palette ten entries a second -- about a sixth of a whole lap per
    // frame, so a sixth of the screen changed colour every frame. That measured
    // 824 KB of escape sequences per frame at 400x200, or 49 MB/s, and was the
    // single largest source of the lag. The invariant behind the new value is
    // pinned in `plasma`'s own tests, where the reasoning lives; this is here to
    // catch a config default quietly reverting it.
    assert_eq!(
        config.get_plasma_options().color_speed,
        termzzz::plasma::PlasmaOptions::default().color_speed
    );
    assert!(
        config.get_plasma_options().color_speed < 20.0,
        "plasma's palette speed went back to 20, which repaints a sixth of the \
         screen every frame"
    );
    assert_eq!(
        config.get_life_options((20, 10)).generations_per_second,
        8.0
    );
    // The donut's rotation speed was 0.022 here. That value is radians per
    // *frame* from when the effect advanced a fixed step per rendered frame, and
    // it is multiplied by the frame delta in seconds, so it was being applied
    // 60x too slowly: one revolution every 4 minutes 46 seconds, and 12.6
    // degrees of turn in ten seconds of watching. It did not look like a slow
    // animation, it looked like a broken effect. The relationship is pinned in
    // `donut`'s own tests; this catches the config default reverting it.
    let donut = config.get_donut_options((20, 10));
    assert!(
        donut.rotation_speed_a > 0.5,
        "the donut's rotation speed is back at {}, which is one revolution every \
         {} seconds",
        donut.rotation_speed_a,
        std::f32::consts::TAU / donut.rotation_speed_a
    );
    assert_eq!(config.get_pipes_options().num_lines, 3);
    assert_eq!(config.get_cube_options().rotation_speed_x, 0.25);
    assert_eq!(config.get_crab_options((20, 10)).movement_speed, 3.0);
}

/// `n` and `p` change the running effect, through the real frame loop.
///
/// The unit tests on `EffectHost` cover the state machine; this covers the
/// wiring, which is the part that can be wrong while the host is fine -- a key
/// that never reaches `on_global_key`, a target the loop never drives, or a
/// rebuild that does not happen mid-transition.
#[test]
fn the_frame_loop_switches_effects_on_n_and_p() {
    use termzzz::config::Config;
    use termzzz::host::EffectHost;
    use termzzz::registry::EffectId;
    use termzzz::runtime::{InputEvent, Key, KeyPhase};

    let press = |key: char| InputEvent::Key {
        key: Key::Char(key),
        phase: KeyPhase::Pressed,
    };

    let run = |keys: Vec<InputEvent>| {
        let mut source = BatchInput::with(keys);
        let mut host = EffectHost::new(EffectId::Life, Config::default(), (20, 8));
        // No transition, so the swap lands on the next frame rather than
        // part-way through a wipe. The wipe is covered by the host's own tests.
        host.set_transition(0.0);

        let mut output = Vec::new();
        // A scripted source and no real terminal, so the loop is driven
        // entirely by the injected events.
        // Bounded: the scripted source runs dry after its events, and a loop
        // with no iteration limit would never stop.
        common::run_loop_with_source_and_size_and_target(
            &mut output,
            &mut host,
            Some(6),
            &mut source,
            (20, 8),
            common::RuntimeOptions::default(),
        )
        .expect("the loop runs");
        host.id()
    };

    assert_eq!(
        run(Vec::new()),
        EffectId::Life,
        "`n` was pressed but nothing happened"
    );
    assert_eq!(
        run(vec![press('n')]),
        EffectId::Mandelbrot,
        "`n` did not advance the effect"
    );
    assert_eq!(
        run(vec![press('n'), press('p')]),
        EffectId::Life,
        "`p` did not step back"
    );
    // Life is second in the order, so `p` from it lands on the first, Matrix.
    assert_eq!(
        run(vec![press('p')]),
        EffectId::Matrix,
        "`p` did not step back"
    );
    // And from the first effect it has to wrap round to the last.
    assert_eq!(
        run(vec![press('p'), press('p')]),
        EffectId::Terrain,
        "`p` from the first effect did not wrap to the last"
    );
}

/// A key the loop does not handle must still reach the effect.
///
/// `n` and `p` are consumed by the target. Everything else has to fall through
/// to the effect, or the interactive effect loses its controls the moment a
/// host is introduced. `Space` pauses the ASCII field, which is observable: a
/// paused field stops changing, so the frames after it produce no cells at all.
#[test]
fn unhandled_keys_still_reach_the_effect_through_the_host() {
    use termzzz::config::Config;
    use termzzz::host::EffectHost;
    use termzzz::registry::EffectId;
    use termzzz::runtime::{InputEvent, Key, KeyPhase};

    let frames = |keys: Vec<InputEvent>, count: usize| -> usize {
        let mut source = BatchInput::with(keys);
        let mut host = EffectHost::new(EffectId::Ascii, Config::default(), (20, 8));
        let mut output = Vec::new();
        common::run_loop_with_source_and_size_and_target(
            &mut output,
            &mut host,
            Some(count),
            &mut source,
            (20, 8),
            common::RuntimeOptions::default(),
        )
        .expect("the loop runs");
        output.len()
    };

    let space = || InputEvent::Key {
        key: Key::Space,
        phase: KeyPhase::Pressed,
    };

    // With no key the field animates, so thirty frames write far more than one.
    let one = frames(Vec::new(), 1);
    let thirty = frames(Vec::new(), 30);
    assert!(
        thirty > one,
        "an unpaused field wrote the same for 1 and 30 frames ({one} vs \
         {thirty}), so this test cannot tell the cases apart"
    );

    // Paused, frames after the first write nothing: the opening frame is
    // rendered before the keypress is read, and then the field stops changing.
    // So thirty paused frames cost exactly what one costs.
    let paused_one = frames(vec![space()], 1);
    let paused_thirty = frames(vec![space()], 30);
    assert_eq!(
        paused_thirty, paused_one,
        "Space did not reach the effect: {paused_thirty} bytes over 30 frames \
         against {paused_one} over 1"
    );
}
