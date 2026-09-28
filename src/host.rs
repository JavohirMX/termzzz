//! The effect that is currently running, and the ability to change it.
//!
//! Switching the running effect needs three things the frame loop deliberately
//! does not have: somewhere to put a replacement effect, the configuration to
//! build one from, and an order to cycle through. [`EffectHost`] holds all
//! three, and the loop drives it.
//!
//! It exists because the loop takes `&mut dyn TerminalEffect` -- a borrow. An
//! effect cannot replace itself through a borrow it does not own, and the loop
//! has no business knowing what an effect is made of.
//!
//! The transition lives here rather than in the loop because a wipe needs the
//! whole screen and the loop only ever holds a *diff*. Wiping a diff blanks the
//! cells the effect happened to redraw this frame, which is not the same thing
//! as wiping the screen: the effect has already committed those cells against
//! its own canvas, so it will not send them again, and the terminal is left
//! holding pixels that nothing will ever correct. So the running effect is
//! wrapped in [`Running`], which accumulates the diff into a surface of its
//! own, wipes a copy of that, and hands the loop the difference against the
//! frame the terminal is already showing.

use crate::buffer::{Buffer, Cell};
use crate::common::{FrameTarget, TerminalEffect};
use crate::config::Config;
use crate::registry::{AnyEffect, EffectId};
use crate::render::wipe;
use crate::runtime::{FrameContext, InputEvent};
use std::time::Duration;

/// How long a switch takes, in seconds: the wipe out and the wipe in together.
///
/// Matches the playlist's default, though the playlist spends its number on
/// each half of its transition, so the two are not quite the same length.
const DEFAULT_TRANSITION: f32 = 0.6;

/// The effect being run, plus what is needed to swap it.
pub struct EffectHost {
    config: Config,
    /// The cycle order. Defaults to every registered effect, so `n` walks the
    /// whole catalogue rather than a hand-picked subset.
    order: Vec<EffectId>,
    position: usize,
    screen_size: (u16, u16),
    running: Running,
}

/// The running effect, the surface its frame is built on, and the transition
/// between one effect and the next.
///
/// This is a `TerminalEffect` rather than a plain field because the transition
/// has to replace the diff the frame loop writes, and the loop only ever asks a
/// target for that through `effect()`.
struct Running {
    effect: Box<dyn TerminalEffect>,
    /// The frame being built: the frame the terminal is showing, plus every
    /// change the effect has reported since.
    ///
    /// This is a plain pair of buffers rather than a `Canvas` because the host
    /// accumulates *differences*, and `Canvas` is built for an effect that
    /// redraws its whole surface every frame. `Canvas::commit` swaps its two
    /// surfaces, so what it hands back to draw on next is the frame from before
    /// the one it just emitted -- one commit too old to accumulate onto. Done
    /// that way the screen alternates between the current frame and the one
    /// before it, and every cell the effect did not redraw blinks out and back.
    frame: Buffer,
    /// The frame the terminal is showing. The diff is against this, and it is
    /// what makes a wipe able to blank a cell the effect did not redraw: the
    /// cell is on the screen whether or not it is in this frame's diff.
    shown: Buffer,
    /// A wiped copy of `frame`, allocated only while a transition is running.
    ///
    /// The wipe is applied to this rather than to `frame` so the undimmed frame
    /// survives underneath and the wipe can be taken back as the front retreats,
    /// instead of being baked in permanently. `clone_from` reuses the
    /// allocation, so a transition frame costs no memory after the first.
    wiped: Option<Buffer>,
    /// Where the switch transition is.
    ///
    /// The switch wipes out to blank, swaps, then wipes back in -- the same
    /// two-phase shape the playlist uses, so the effect that comes out from
    /// behind the wipe already exists and the first frame is not a flash of an
    /// unbuilt effect.
    phase: SwitchPhase,
    /// How far the current half of the transition has run. Lives beside the
    /// phase because a small enum cannot carry it.
    elapsed: Duration,
    transition: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwitchPhase {
    Idle,
    WipeOut,
    WipeIn,
    /// The wipe finished and the replacement has not been built yet. Only
    /// reachable with a zero-length transition, which has no wipe to wait for.
    Swap,
}

impl TerminalEffect for Running {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        let delta = self.effect.get_diff();
        self.present(delta)
    }

    fn get_diff_with_context(
        &mut self,
        context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        let delta = self.effect.get_diff_with_context(context);
        self.present(delta)
    }

    fn update(&mut self) {
        self.effect.update();
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.effect.update_with_context(context);
    }

    fn update_size(&mut self, width: u16, height: u16) {
        let (width, height) = (width.max(1) as usize, height.max(1) as usize);
        if (self.frame.width, self.frame.height) != (width, height) {
            // Both frames describe dimensions that no longer exist, and a resize
            // is a full repaint, so neither of them can be trusted. The wiped
            // copy is in the old dimensions too.
            self.frame = Buffer::new(width, height);
            self.shown = Buffer::new(width, height);
            self.wiped = None;
        }
        self.effect.update_size(width as u16, height as u16);
    }

    fn reset(&mut self) {
        self.effect.reset();
    }

    fn handle_input(&mut self, event: &InputEvent) {
        self.effect.handle_input(event);
    }
}

impl Running {
    fn new(
        effect: Box<dyn TerminalEffect>,
        screen_size: (u16, u16),
        transition: f32,
    ) -> Self {
        let (width, height) = (screen_size.0 as usize, screen_size.1 as usize);
        Self {
            effect,
            frame: Buffer::new(width, height),
            // The session clears the terminal before the loop starts, so blank
            // is the truth about what it is showing.
            shown: Buffer::new(width, height),
            wiped: None,
            phase: SwitchPhase::Idle,
            elapsed: Duration::ZERO,
            transition,
        }
    }

    /// Builds the frame to write to the terminal: the effect's changes on top of
    /// the whole screen, with the wipe applied to a copy of the result.
    ///
    /// Returns the difference against the frame the terminal is showing, which
    /// is also recorded as shown, so the two cannot drift apart.
    fn present(
        &mut self,
        delta: Vec<(usize, usize, Cell)>,
    ) -> Vec<(usize, usize, Cell)> {
        let (width, height) = (self.frame.width, self.frame.height);
        for (x, y, cell) in delta {
            // `Buffer::set` only debug-asserts its bounds, and the output path
            // skips rather than clamps, so an out-of-range coordinate from an
            // effect must not reach either.
            if x < width && y < height {
                self.frame.set(x, y, cell);
            }
        }

        let outgoing = match self.wipe_progress() {
            None => &self.frame,
            Some(progress) => {
                let wiped = self.wiped.get_or_insert_with(|| self.frame.clone());
                wiped.clone_from(&self.frame);
                wipe::apply(wiped, width, height, progress);
                wiped
            }
        };

        let cells = self.shown.diff(outgoing);
        for (x, y, cell) in &cells {
            self.shown.set(*x, *y, *cell);
        }
        cells
    }

    /// Takes a replacement effect.
    ///
    /// The frame being built is blanked but the frame the terminal is showing is
    /// not: that one is the truth about the screen, and the frame before this
    /// one is the last of the outgoing effect. A replacement's first frame
    /// covers every cell it draws and nothing at all for the cells it leaves
    /// blank, so a frame still carrying the outgoing effect would show it
    /// through underneath -- and keep showing it, if the replacement never
    /// redraws those cells.
    fn adopt(&mut self, effect: Box<dyn TerminalEffect>) {
        self.effect = effect;
        self.frame.fill_with(&wipe::blank_cell());
    }

    /// Ends the transition, releasing the copy the wipe was using.
    fn finish(&mut self) {
        self.phase = SwitchPhase::Idle;
        self.elapsed = Duration::ZERO;
        self.wiped = None;
    }

    /// How long one half of the transition runs for.
    ///
    /// The configured duration is the whole switch and the two halves are half
    /// of it each, so a transition of `0.4` is a switch that takes 0.4 seconds.
    ///
    /// Quantised to a microsecond. The configured value is an `f32`, so halving
    /// it lands a few nanoseconds either side of the intended time, and that is
    /// enough to decide the comparison below one frame late -- which is the
    /// whole difference between one transition taking two lengths at two
    /// different frame rates.
    fn half(&self) -> Duration {
        let seconds = f64::from(self.transition) * 0.5;
        Duration::from_micros((seconds * 1_000_000.0) as u64)
    }

    /// How far the current half of the transition has run, `0.0..=1.0`.
    fn progress(&self) -> f32 {
        if self.transition <= 0.0 {
            // Unreachable through `begin_switch`, which picks `Swap` for a
            // zero-length transition, but a `set_transition` part-way through a
            // wipe can produce it. Hiding nothing beats dividing by zero.
            return 0.0;
        }
        // The ratio of two durations, and clamped because one long frame can
        // take the accumulator past the end of the half.
        let ratio = self.elapsed.as_secs_f64() / self.half().as_secs_f64();
        (ratio as f32).clamp(0.0, 1.0)
    }

    /// Whether the current half has run out.
    fn half_elapsed(&self) -> bool {
        self.transition > 0.0 && self.elapsed >= self.half()
    }

    /// The wipe to apply to a finished frame, if any.
    ///
    /// `None` outside a transition. During one it is the fraction of the screen
    /// the front has passed, which [`wipe::apply`] turns into blanked cells.
    pub fn wipe_progress(&self) -> Option<f32> {
        match self.phase {
            SwitchPhase::Idle => None,
            // Only reachable with a zero-length transition, where the frame in
            // hand came from the effect being replaced and the replacement has
            // not been built yet. Blanking it is the hard cut that "no
            // transition" means; leaving it alone drew the outgoing effect at
            // full brightness for a frame.
            SwitchPhase::Swap => Some(1.0),
            SwitchPhase::WipeOut => Some(self.progress()),
            SwitchPhase::WipeIn => Some(1.0 - self.progress()),
        }
    }

    /// Whether a switch is in progress.
    fn is_switching(&self) -> bool {
        self.phase != SwitchPhase::Idle
    }

    /// Whether a wipe is running, which is when a further request would restart
    /// it.
    fn wipe_in_flight(&self) -> bool {
        matches!(self.phase, SwitchPhase::WipeOut | SwitchPhase::WipeIn)
    }

    /// Marks a switch as wanted. `advance` does the rest.
    fn begin_switch(&mut self) {
        self.elapsed = Duration::ZERO;
        self.phase = if self.transition <= 0.0 {
            SwitchPhase::Swap
        } else {
            SwitchPhase::WipeOut
        };
    }
}

impl FrameTarget for EffectHost {
    fn effect(&mut self) -> &mut dyn TerminalEffect {
        &mut self.running
    }

    /// `n` and `p`, alongside the speed keys the loop already handles.
    ///
    /// Repeats are dropped here. A held key arrives as a stream of
    /// `KeyPhase::Repeated` events, and acting on each one is what turned a
    /// transition into a strobe: every repeat restarted the wipe, so it never got
    /// past its first frame and never hid anything, while the effect underneath
    /// was replaced each time and drawn at full brightness over the last.
    ///
    /// Consuming a repeat still returns true, so the effect does not see it
    /// either -- a key the host has swallowed must not leak through as an
    /// effect-level input.
    fn on_global_key(
        &mut self,
        key: crate::runtime::Key,
        phase: crate::runtime::KeyPhase,
    ) -> bool {
        if phase == crate::runtime::KeyPhase::Repeated {
            return matches!(
                key,
                crate::runtime::Key::Char('n') | crate::runtime::Key::Char('p')
            );
        }
        match key {
            crate::runtime::Key::Char('n') => {
                self.next();
                true
            }
            crate::runtime::Key::Char('p') => {
                self.previous();
                true
            }
            _ => false,
        }
    }

    /// Ticks the transition on.
    ///
    /// The wipe itself is applied where the frame is produced, in
    /// [`Running::present`], because the loop hands this the effect's diff and a
    /// diff is not enough to wipe: the cells the effect did not redraw this
    /// frame are on the screen and have to be blanked too.
    fn on_frame(&mut self, delta: Duration, _cells: &mut [(usize, usize, Cell)]) {
        self.advance(delta);
    }

    fn on_resize(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.running
            .update_size(self.screen_size.0, self.screen_size.1);
    }
}

impl EffectHost {
    /// A host running `id`, cycling through every registered effect.
    pub fn new(id: EffectId, config: Config, screen_size: (u16, u16)) -> Self {
        let order: Vec<EffectId> = EffectId::all().collect();
        let position = order
            .iter()
            .position(|candidate| *candidate == id)
            .unwrap_or(0);
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let effect = Box::new(AnyEffect::build(id, &config, screen_size));

        Self {
            config,
            order,
            position,
            screen_size,
            running: Running::new(effect, screen_size, DEFAULT_TRANSITION),
        }
    }

    /// The running effect, for the frame loop to drive.
    pub fn effect(&mut self) -> &mut dyn TerminalEffect {
        &mut self.running
    }

    /// The running effect, read-only.
    pub fn effect_ref(&self) -> &dyn TerminalEffect {
        &self.running
    }

    /// Which effect is running.
    pub fn id(&self) -> EffectId {
        self.order[self.position]
    }

    /// Replaces the cycle order. An empty or unknown list is ignored, because a
    /// host with nothing to switch to cannot honour `n` and would panic on it.
    pub fn set_order(&mut self, order: Vec<EffectId>) {
        if order.is_empty() {
            return;
        }
        let current = self.id();
        self.order = order;
        self.position = self
            .order
            .iter()
            .position(|candidate| *candidate == current)
            .unwrap_or(0);
    }

    /// Cycles to the next effect and starts the transition.
    pub fn next(&mut self) {
        self.step(1);
    }

    /// Cycles to the previous effect and starts the transition.
    pub fn previous(&mut self) {
        self.step(-1);
    }

    /// Moves `count` places through the order and starts the transition.
    ///
    /// A wipe already in flight swallows the request, so a second press during a
    /// transition does not restart it. That guard is on the wipe rather than on
    /// the key because `step` is a public method with no phase to consult; the
    /// key handler above is where a *repeat* is dropped, and this is where a
    /// second deliberate press is absorbed. Together they mean a held key
    /// advances one effect per transition, each behind a full wipe, and a
    /// zero-length transition has no wipe to wait out so it keeps stepping.
    fn step(&mut self, count: isize) {
        if self.order.len() < 2 || self.running.wipe_in_flight() {
            return;
        }
        let len = self.order.len() as isize;
        let position = (self.position as isize + count).rem_euclid(len);
        self.position = position as usize;
        self.begin_switch();
    }

    /// Jumps straight to an effect, with the same transition.
    pub fn switch_to(&mut self, id: EffectId) {
        match self.order.iter().position(|candidate| *candidate == id) {
            Some(position) if position != self.position => {
                self.position = position;
                self.begin_switch();
            }
            // Already running it: no transition, so a held key does not strobe.
            _ => {}
        }
    }

    fn begin_switch(&mut self) {
        self.running.begin_switch();
    }

    /// Overrides the transition duration. Zero switches with no wipe.
    pub fn set_transition(&mut self, seconds: f32) {
        self.running.transition = seconds.max(0.0);
    }

    /// Whether a switch is in progress.
    pub fn is_switching(&self) -> bool {
        self.running.is_switching()
    }

    /// Advances the transition, and swaps the effect when the wipe finishes.
    pub fn advance(&mut self, delta: Duration) {
        if !self.running.is_switching() {
            return;
        }

        // A zero-length transition has no phases to run through. Handling it
        // here rather than in `begin_switch` keeps one place that knows a swap
        // happened, and `Swap` would otherwise wait for an elapsed time it can
        // never reach.
        if self.running.transition <= 0.0 {
            self.swap();
            return;
        }

        // Tracked as elapsed time rather than a frame count, so the transition
        // takes the same wall-clock time at any frame rate. A `Duration` rather
        // than a float, so the half is compared exactly: accumulating float
        // seconds put a transition that divides evenly into the step length one
        // frame late about half the time, which made the same transition take
        // two different lengths at two different frame rates.
        self.running.elapsed += delta;

        match self.running.phase {
            SwitchPhase::Idle | SwitchPhase::Swap => {}
            SwitchPhase::WipeOut => {
                if self.running.half_elapsed() {
                    // The swap happens here rather than on a phase of its own.
                    // A `Swap` phase is a frame in which the effect has not been
                    // replaced yet, and that frame is drawn before the wipe is
                    // applied -- so the outgoing effect went to the terminal at
                    // full brightness, once per switch, and the frame after it
                    // blanked the whole screen to compensate. Rebuilding now
                    // means the next frame is the new effect at a wipe of 1.0.
                    self.swap();
                }
            }
            SwitchPhase::WipeIn => {
                if self.running.half_elapsed() {
                    self.running.finish();
                }
            }
        }
    }

    /// Builds the effect now current in the order and adopts it.
    fn swap(&mut self) {
        let id = self.id();
        let effect = Box::new(AnyEffect::build(id, &self.config, self.screen_size));
        self.running.adopt(effect);
        self.running.elapsed = Duration::ZERO;
        if self.running.transition <= 0.0 {
            // Nothing left to wipe, so the switch is over the moment it happens.
            self.running.finish();
        } else {
            self.running.phase = SwitchPhase::WipeIn;
        }
    }

    /// Forwards a terminal resize to the running effect.
    pub fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.on_resize(self.screen_size.0, self.screen_size.1);
    }

    /// The wipe applied to a finished frame, if any.
    ///
    /// Kept on the host because the loop and the tests both want to know where
    /// the transition is, and neither should have to reach through the effect.
    pub fn wipe_progress(&self) -> Option<f32> {
        self.running.wipe_progress()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn host(id: EffectId) -> EffectHost {
        EffectHost::new(id, Config::default(), (40, 12))
    }

    /// Runs the transition to completion.
    fn settle(host: &mut EffectHost) {
        for _ in 0..200 {
            if !host.is_switching() {
                return;
            }
            host.advance(Duration::from_millis(16));
        }
        panic!("the transition never finished");
    }

    /// One frame of the loop, in the loop's order: the effect draws, the host
    /// wipes what it drew, the cells land on the terminal, and only then does
    /// the transition tick. `terminal` is the screen, so it is what the user
    /// would be looking at.
    fn frame(
        host: &mut EffectHost,
        terminal: &mut Buffer,
        delta: Duration,
    ) -> (Option<f32>, Vec<(usize, usize, Cell)>) {
        let mut cells = host.effect().get_diff();
        let progress = host.wipe_progress();
        host.on_frame(delta, &mut cells);
        for (x, y, cell) in &cells {
            terminal.set(*x, *y, *cell);
        }
        (progress, cells)
    }

    /// How much of the screen is showing something other than a space.
    fn lit(buffer: &Buffer) -> usize {
        (0..buffer.height)
            .flat_map(|y| (0..buffer.width).map(move |x| (x, y)))
            .filter(|(x, y)| buffer.get(*x, *y).symbol != ' ')
            .count()
    }

    /// How many lit cells of `buffer` are still showing exactly what `before`
    /// had there. During a switch this is the outgoing effect, still on screen.
    fn matching(before: &Buffer, buffer: &Buffer) -> usize {
        (0..buffer.height)
            .flat_map(|y| (0..buffer.width).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let cell = buffer.get(*x, *y);
                cell.symbol != ' ' && cell == before.get(*x, *y)
            })
            .count()
    }

    #[test]
    fn a_new_host_runs_what_it_was_asked_for() {
        let host = host(EffectId::Dvd);
        assert_eq!(host.id(), EffectId::Dvd);
    }

    #[test]
    fn an_unknown_start_falls_back_to_the_first_effect() {
        // `unwrap_or(0)` rather than a panic: a bad start should not stop the
        // program, and the registry has no way to name an id that is not there.
        let host = EffectHost::new(EffectId::Terrain, Config::default(), (40, 12));
        assert!(EffectId::all().any(|id| id == host.id()));
    }

    #[test]
    fn next_advances_and_wraps() {
        let mut host = host(EffectId::Matrix);
        let start = host.id();
        host.next();
        assert_ne!(host.id(), start, "`n` did not advance");

        // A whole cycle from here returns to here. Each press has to be a
        // switch the host will accept, so the previous one is settled first.
        let count = EffectId::all().count();
        for _ in 0..count {
            settle(&mut host);
            host.next();
        }
        assert_eq!(host.id(), EffectId::Life, "the cycle did not wrap");
        // A tripwire on the catalogue's size, and deliberately hardcoded: adding an
        // effect should fail here, where the sentence says so, rather than being
        // discovered by a wrap-around test that quietly still passes.
        assert_eq!(EffectId::all().count(), 21, "the catalogue changed size");
    }

    /// The last effect in cycle order, which is what `p` from the first wraps to.
    ///
    /// Read from the registry rather than written out. Three of these tests used
    /// to name the last effect literally, so every effect added to the crate broke
    /// a wrap-around assertion that had nothing to do with wrapping.
    fn last_effect() -> EffectId {
        EffectId::all().last().expect("the catalogue is not empty")
    }

    #[test]
    fn previous_rewinds_and_wraps() {
        let mut host = host(EffectId::Matrix);
        let count = EffectId::all().count();

        // The first effect is first in the order, so `p` from it has to wrap
        // round to the last rather than doing nothing.
        host.previous();
        assert_eq!(
            host.id(),
            last_effect(),
            "`p` from the first effect did not wrap to the last"
        );

        // And a whole cycle in reverse returns to where it started.
        for _ in 0..(count - 1) {
            settle(&mut host);
            host.previous();
        }
        assert_eq!(
            host.id(),
            EffectId::Matrix,
            "the reverse cycle did not wrap"
        );
    }

    #[test]
    fn next_and_previous_are_inverses() {
        let mut host = host(EffectId::Boids);
        let start = host.id();
        host.next();
        // `previous` is ignored while the wipe `next` started is still running,
        // so the two have to be a whole switch apart.
        settle(&mut host);
        host.previous();
        assert_eq!(host.id(), start);
    }

    #[test]
    fn a_switch_wipes_out_then_in() {
        let mut host = host(EffectId::Life);
        host.next();

        assert!(host.is_switching());
        // The wipe starts at zero, so the first transition frame still shows
        // the outgoing effect intact.
        let first = host.wipe_progress().expect("a wipe should be running");
        assert_eq!(first, 0.0, "the wipe started mid-way");

        let mut previous = first;
        for _ in 0..10 {
            host.advance(Duration::from_millis(16));
            let now = host.wipe_progress().expect("the wipe ended early");
            assert!(now >= previous, "the wipe went backwards");
            previous = now;
        }
    }

    #[test]
    fn the_effect_is_rebuilt_mid_transition_not_at_the_end() {
        let mut host = host(EffectId::Life);
        let before = host.id();
        host.next();
        let after = host.id();
        assert_ne!(before, after);

        // `id` reports the new one immediately so the caller can see the
        // request, but the running effect only changes once the wipe finishes.
        let mut swapped_early = false;
        for _ in 0..200 {
            host.advance(Duration::from_millis(16));
            if !swapped_early && host.wipe_progress().is_some() {
                swapped_early = true;
            }
            if !host.is_switching() {
                break;
            }
        }
        assert!(!host.is_switching());
        assert_eq!(host.id(), after);

        // And the running effect is now the new one: it draws something.
        let effect = host.effect();
        let diff = effect.get_diff();
        assert!(
            diff.iter().all(|(x, y, _)| *x < 40 && *y < 12),
            "the rebuilt effect drew outside the canvas"
        );
    }

    #[test]
    fn a_zero_transition_switches_without_a_wipe() {
        let mut host = host(EffectId::Life);
        host.set_transition(0.0);
        let before = host.id();
        host.next();

        host.advance(Duration::from_millis(16));
        assert!(!host.is_switching(), "a zero transition still wiped");
        assert_ne!(host.id(), before);
    }

    #[test]
    fn a_single_effect_order_cannot_be_cycled() {
        // Otherwise `n` would spin forever on one effect.
        let mut host = host(EffectId::Dvd);
        host.set_order(vec![EffectId::Dvd]);
        let before = host.id();
        host.next();
        host.previous();
        assert_eq!(host.id(), before);
    }

    #[test]
    fn an_empty_order_is_ignored_rather_than_accepted() {
        let mut host = host(EffectId::Dvd);
        let before = host.id();
        host.set_order(Vec::new());
        assert_eq!(host.id(), before, "an empty order was accepted");
    }

    #[test]
    fn setting_an_order_keeps_the_current_effect_selected() {
        let mut host = host(EffectId::Fire);
        host.set_order(vec![EffectId::Dvd, EffectId::Fire, EffectId::Life]);
        assert_eq!(host.id(), EffectId::Fire);
        host.next();
        assert_eq!(host.id(), EffectId::Life, "the new order was not used");
    }

    #[test]
    fn switching_to_the_running_effect_does_nothing() {
        // A held key would otherwise strobe the transition.
        let mut host = host(EffectId::Dvd);
        host.switch_to(EffectId::Dvd);
        assert!(!host.is_switching());
    }

    #[test]
    fn the_transition_takes_the_same_time_at_any_frame_rate() {
        let time_at = |step_ms: u64| {
            let mut host = host(EffectId::Life);
            host.set_transition(0.4);
            host.next();
            let mut elapsed = 0.0f32;
            while host.is_switching() {
                host.advance(Duration::from_millis(step_ms));
                elapsed += step_ms as f32 / 1000.0;
                assert!(elapsed < 10.0, "the transition never finished");
            }
            elapsed
        };

        let fast = time_at(8);
        let slow = time_at(40);
        assert!(
            (fast - slow).abs() < 0.05,
            "the transition took {fast}s at 8ms steps and {slow}s at 40ms steps"
        );
    }

    #[test]
    fn a_resize_reaches_the_running_effect() {
        let mut host = host(EffectId::Life);
        host.update_size(20, 8);
        let effect = host.effect();
        let diff = effect.get_diff();
        for (x, y, _) in &diff {
            assert!(*x < 20 && *y < 8, "drew outside the resized canvas");
        }
    }

    #[test]
    fn a_switch_after_a_resize_produces_a_drawable_effect() {
        // The rebuild has to use the current size, not the size the host was
        // built with, or the new effect draws at the wrong dimensions.
        let mut host = host(EffectId::Life);
        host.update_size(24, 8);
        host.next();
        settle(&mut host);

        let effect = host.effect();
        let diff = effect.get_diff();
        for (x, y, _) in &diff {
            assert!(*x < 24 && *y < 8, "drew outside the resized canvas");
        }
    }

    /// The configured duration is the duration that happens.
    ///
    /// The accumulator is scaled by the transition, so `set_transition(0.4)`
    /// makes the switch take about 0.4 seconds. Unscaled it took two seconds
    /// whatever was configured, which is why the default of 0.6 and
    /// `set_transition` were both no-ops.
    #[test]
    fn the_configured_transition_is_the_transition_that_happens() {
        let time_at = |step_ms: u64, transition: f32| {
            let mut host = host(EffectId::Life);
            host.set_transition(transition);
            host.next();
            let mut elapsed = 0.0f32;
            while host.is_switching() {
                host.advance(Duration::from_millis(step_ms));
                elapsed += step_ms as f32 / 1000.0;
                assert!(elapsed < 10.0, "the transition never finished");
            }
            elapsed
        };

        for (transition, step_ms) in [(0.4, 8), (0.4, 16), (0.2, 16), (1.0, 32)] {
            let took = time_at(step_ms, transition);
            assert!(
                (took - transition).abs() < transition * 0.25,
                "a {transition}s transition took {took}s at {step_ms}ms steps"
            );
        }
    }

    /// The progress is a fraction of a screen, whatever the accumulator says.
    ///
    /// It is real elapsed time, so a frame longer than the transition used to
    /// hand the wipe a progress past 1.0, and a wipe-in could go negative --
    /// which `apply` clamps, so nothing panicked, but the value the host
    /// reported was not a fraction of anything.
    #[test]
    fn the_wipe_progress_never_leaves_the_screen() {
        for phase in [SwitchPhase::WipeOut, SwitchPhase::WipeIn, SwitchPhase::Swap]
        {
            let mut host = host(EffectId::Life);
            host.set_transition(0.4);
            host.running.phase = phase;
            // A stall several times the length of the transition.
            host.running.elapsed = Duration::from_secs(9);

            if let Some(progress) = host.wipe_progress() {
                assert!(
                    (0.0..=1.0).contains(&progress),
                    "{phase:?} reported a progress of {progress}"
                );
            }
        }
    }

    /// A held key steps once and leaves the wipe alone.
    ///
    /// `process_runtime_events` sends `KeyPhase::Pressed` and
    /// `KeyPhase::Repeated` to the same handler, so a held `n` lands here once
    /// per key repeat. Every call used to advance the position *and* reset the
    /// elapsed time, so the wipe never got past roughly its first frame, never
    /// hid anything, and each effect was drawn at full brightness straight over
    /// the last one.
    #[test]
    fn a_held_key_steps_once_and_does_not_restart_the_wipe() {
        for press in [
            EffectHost::next as fn(&mut EffectHost),
            EffectHost::previous as fn(&mut EffectHost),
        ] {
            let mut host = host(EffectId::Life);
            press(&mut host);
            host.advance(Duration::from_millis(16));
            let stepped = host.id();
            let elapsed = host.running.elapsed;
            assert!(elapsed > Duration::ZERO, "the wipe never started");

            for _ in 0..20 {
                press(&mut host);
            }

            assert_eq!(host.id(), stepped, "a held key walked the catalogue");
            assert_eq!(
                host.running.elapsed, elapsed,
                "a held key restarted the wipe"
            );
        }
    }

    /// Key repeats still step when there is no wipe to wait for.
    ///
    /// The guard is on the wipe, not on the key, so a zero-length transition has
    /// no wipe to wait out and every press still steps. Which is what the frame
    /// loop's own test of `n` and `p` relies on: it drives a batch of presses
    /// before the first frame, with no transition, and expects `p` twice to wrap
    /// round the catalogue.
    #[test]
    fn presses_still_step_when_there_is_no_wipe_to_wait_for() {
        let press = |host: &mut EffectHost, keys: &[char]| {
            host.set_transition(0.0);
            for key in keys {
                match key {
                    'n' => host.next(),
                    _ => host.previous(),
                }
            }
            host.advance(Duration::from_millis(16));
            host.id()
        };

        assert_eq!(press(&mut host(EffectId::Life), &[]), EffectId::Life);
        assert_eq!(
            press(&mut host(EffectId::Life), &['n']),
            EffectId::Mandelbrot,
            "`n` did not advance"
        );
        assert_eq!(
            press(&mut host(EffectId::Life), &['n', 'p']),
            EffectId::Life,
            "`p` did not step back"
        );
        assert_eq!(
            press(&mut host(EffectId::Life), &['p']),
            EffectId::Matrix,
            "`p` did not step back"
        );
        assert_eq!(
            press(&mut host(EffectId::Life), &['p', 'p']),
            last_effect(),
            "`p` from the first effect did not wrap to the last"
        );
    }

    /// The screen goes blank, and what is left of the outgoing effect is gone.
    ///
    /// This is "previous effects remain on the screen when the next effect
    /// comes", stated as a property of the screen. The effect commits its own
    /// canvas before the host ever sees the diff, so a cell it did not redraw
    /// this frame is not in the diff -- and a wipe applied to the diff alone
    /// cannot blank it. What the wipe misses is not in the next diff either, so
    /// the leftovers survive the whole switch.
    #[test]
    fn the_wipe_out_leaves_nothing_of_the_outgoing_effect_on_the_screen() {
        let (width, height) = (40usize, 12usize);
        let mut host = EffectHost::new(
            EffectId::Donut,
            Config::default(),
            (width as u16, height as u16),
        );
        let mut terminal = Buffer::new(width, height);

        for _ in 0..8 {
            frame(&mut host, &mut terminal, Duration::from_millis(16));
        }
        let outgoing = terminal.clone();
        assert!(lit(&terminal) > 0, "the donut drew nothing to lose");

        host.next();
        let mut emptiest = lit(&terminal);
        for step in 0..60 {
            let (progress, _) =
                frame(&mut host, &mut terminal, Duration::from_millis(16));
            if !host.is_switching() {
                break;
            }
            assert!(
                progress.is_some(),
                "frame {step} of the switch was drawn with no wipe at all"
            );

            if host.running.phase == SwitchPhase::WipeOut {
                // Everything the front has passed is blank. Worked out with the
                // same wipe the host uses, so the two cannot drift.
                let progress = progress.expect("checked above");
                let mut hidden = outgoing.clone();
                wipe::apply(&mut hidden, width, height, progress);
                for y in 0..height {
                    for x in 0..width {
                        if hidden.get(x, y).symbol == ' '
                            && outgoing.get(x, y).symbol != ' '
                        {
                            assert_eq!(
                                terminal.get(x, y).symbol,
                                ' ',
                                "the wipe had passed ({x},{y}) at frame {step} \
                                 and the outgoing effect was still there"
                            );
                        }
                    }
                }
            }
            emptiest = emptiest.min(lit(&terminal));
        }

        assert_eq!(
            emptiest, 0,
            "the screen never went blank, so the wipe never finished: \
             {emptiest} cells were still lit"
        );
    }

    /// The screen and the host's own idea of the screen never drift apart.
    ///
    /// The host emits the difference between the frame the terminal is showing
    /// and the frame it means to show, and records what it emitted as what the
    /// terminal is showing, so the two are the same by construction. The moment
    /// they are not, something has been blanked that will never be repainted --
    /// which is what "previous effects remain on the screen" is.
    #[test]
    fn the_terminal_always_shows_exactly_what_the_host_thinks_it_shows() {
        let (width, height) = (40usize, 12usize);
        let mut host = EffectHost::new(
            EffectId::Donut,
            Config::default(),
            (width as u16, height as u16),
        );
        let mut terminal = Buffer::new(width, height);

        for step in 0..80 {
            if step == 8 {
                host.next();
            }
            frame(&mut host, &mut terminal, Duration::from_millis(16));
            assert_eq!(
                terminal.buffer, host.running.shown.buffer,
                "the screen and the host's record of it disagree at step {step}"
            );
        }
        assert!(!host.is_switching(), "the switch never finished");
    }

    /// No frame of a switch shows the outgoing effect at full brightness.
    ///
    /// The swap used to be a phase of its own, and a phase is a frame: the one
    /// after the wipe finished and before the replacement was built, with no
    /// wipe applied to it at all. So every switch put a full-brightness frame
    /// of the outgoing effect on the screen and then blanked the whole screen
    /// to cover it up. Two full-screen flashes per switch, and for the donut,
    /// which redraws every cell every frame, the flickering the user reported.
    #[test]
    fn no_frame_of_a_switch_draws_the_outgoing_effect_unwiped() {
        let (width, height) = (40usize, 12usize);
        let mut host = EffectHost::new(
            EffectId::Donut,
            Config::default(),
            (width as u16, height as u16),
        );
        let mut terminal = Buffer::new(width, height);
        for _ in 0..8 {
            frame(&mut host, &mut terminal, Duration::from_millis(16));
        }
        let outgoing = terminal.clone();

        host.next();
        let mut visible = matching(&outgoing, &terminal);
        for step in 0..60 {
            let (progress, _) =
                frame(&mut host, &mut terminal, Duration::from_millis(16));
            assert!(
                progress.is_some(),
                "frame {step} of the switch was drawn with no wipe at all"
            );

            if host.running.phase == SwitchPhase::WipeIn {
                assert_eq!(
                    matching(&outgoing, &terminal),
                    0,
                    "the outgoing effect was still on screen once the wipe \
                     had finished"
                );
                break;
            }

            let now = matching(&outgoing, &terminal);
            assert!(
                now <= visible,
                "the outgoing effect came back at frame {step}: {visible} \
                 cells before, {now} after"
            );
            visible = now;
        }
        assert_ne!(
            host.running.phase,
            SwitchPhase::Idle,
            "the switch never finished"
        );
    }
}
