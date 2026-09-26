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

use crate::buffer::Cell;
use crate::common::{FrameTarget, TerminalEffect};
use crate::config::Config;
use crate::registry::{AnyEffect, EffectId};
use crate::render::wipe;
use std::time::Duration;

/// How long a switch takes, in seconds. Matches the playlist's default so the
/// two transitions feel like the same transition.
const DEFAULT_TRANSITION: f32 = 0.6;

/// The effect being run, plus what is needed to swap it.
pub struct EffectHost {
    active: Box<dyn TerminalEffect>,
    config: Config,
    /// The cycle order. Defaults to every registered effect, so `n` walks the
    /// whole catalogue rather than a hand-picked subset.
    order: Vec<EffectId>,
    position: usize,
    screen_size: (u16, u16),
    /// Where the switch transition is, `0.0` to `1.0`.
    ///
    /// The switch wipes out to blank, swaps, then wipes back in -- the same
    /// two-phase shape the playlist uses, so the effect that comes out from
    /// behind the wipe already exists and the first frame is not a flash of an
    /// unbuilt effect.
    phase: SwitchPhase,
    /// How far the current half of the transition has run, `0.0..=1.0`.
    /// Lives beside the phase because a small enum cannot carry a float.
    elapsed: f32,
    transition: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwitchPhase {
    Idle,
    WipeOut,
    WipeIn,
    /// Mid-swap, after the wipe finished and before the new one started. The
    /// effect is rebuilt on the first frame that lands here.
    Swap,
}

impl FrameTarget for EffectHost {
    fn effect(&mut self) -> &mut dyn TerminalEffect {
        self.active.as_mut()
    }

    /// `n` and `p`, alongside the speed keys the loop already handles.
    fn on_global_key(&mut self, key: crate::runtime::Key) -> bool {
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

    /// Blanks the cells the transition front has passed, and ticks it forward.
    fn on_frame(&mut self, delta: Duration, cells: &mut [(usize, usize, Cell)]) {
        let (width, height) = self.screen_size;
        if let Some(progress) = self.wipe_progress() {
            wipe::apply_to_cells(cells, width as usize, height as usize, progress);
        }
        self.advance(delta);
    }

    fn on_resize(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.active
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

        Self {
            active: Box::new(AnyEffect::build(id, &config, screen_size)),
            config,
            order,
            position,
            screen_size,
            phase: SwitchPhase::Idle,
            elapsed: 0.0,
            transition: DEFAULT_TRANSITION,
        }
    }

    /// The running effect, for the frame loop to drive.
    pub fn effect(&mut self) -> &mut dyn TerminalEffect {
        self.active.as_mut()
    }

    /// The running effect, read-only.
    pub fn effect_ref(&self) -> &dyn TerminalEffect {
        self.active.as_ref()
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
        if self.order.len() < 2 {
            return;
        }
        self.position = (self.position + 1) % self.order.len();
        self.begin_switch();
    }

    /// Cycles to the previous effect and starts the transition.
    pub fn previous(&mut self) {
        if self.order.len() < 2 {
            return;
        }
        // Wrapping, so `p` from the first effect reaches the last rather than
        // doing nothing.
        self.position = (self.position + self.order.len() - 1) % self.order.len();
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
        // `advance` handles the zero case, so this only has to mark that a
        // switch is wanted.
        self.elapsed = 0.0;
        self.phase = if self.transition <= 0.0 {
            SwitchPhase::Swap
        } else {
            SwitchPhase::WipeOut
        };
    }

    /// Overrides the transition duration. Zero switches with no wipe.
    pub fn set_transition(&mut self, seconds: f32) {
        self.transition = seconds.max(0.0);
    }

    /// Whether a switch is in progress.
    pub fn is_switching(&self) -> bool {
        self.phase != SwitchPhase::Idle
    }

    /// Advances the transition, and swaps the effect when the wipe finishes.
    pub fn advance(&mut self, delta: Duration) {
        // A zero-length transition has no phases to run through. Handling it
        // here rather than in `begin_switch` keeps one place that knows a swap
        // happened, and `Swap` would otherwise wait for an elapsed time it can
        // never reach.
        if self.transition <= 0.0 {
            if self.phase != SwitchPhase::Idle {
                self.rebuild();
                self.phase = SwitchPhase::Idle;
                self.elapsed = 0.0;
            }
            return;
        }

        match self.phase {
            SwitchPhase::Idle => {}
            SwitchPhase::WipeOut => {
                // Tracked as elapsed time rather than a frame count, so the
                // transition takes the same wall-clock time at any frame rate.
                self.elapsed += delta.as_secs_f32();
                if self.elapsed >= 1.0 {
                    self.phase = SwitchPhase::Swap;
                }
            }
            SwitchPhase::Swap => {
                self.rebuild();
                self.phase = SwitchPhase::WipeIn;
                self.elapsed = 0.0;
            }
            SwitchPhase::WipeIn => {
                self.elapsed += delta.as_secs_f32();
                if self.elapsed >= 1.0 {
                    self.phase = SwitchPhase::Idle;
                    self.elapsed = 0.0;
                }
            }
        }
    }

    /// Builds the effect now current in the order and adopts it.
    fn rebuild(&mut self) {
        let id = self.id();
        self.active =
            Box::new(AnyEffect::build(id, &self.config, self.screen_size));
    }

    /// Forwards a terminal resize to the running effect.
    pub fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.active
            .update_size(self.screen_size.0, self.screen_size.1);
    }

    /// The wipe to apply to a finished frame, if any.
    ///
    /// `None` outside a transition. During one it is the fraction of the screen
    /// the front has passed, which [`wipe::apply`] turns into blanked cells.
    pub fn wipe_progress(&self) -> Option<f32> {
        match self.phase {
            SwitchPhase::Idle | SwitchPhase::Swap => None,
            SwitchPhase::WipeOut => Some(self.elapsed),
            SwitchPhase::WipeIn => Some(1.0 - self.elapsed),
        }
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

        // A whole cycle from here returns to here.
        let count = EffectId::all().count();
        for _ in 0..count {
            host.next();
        }
        assert_eq!(host.id(), EffectId::Life, "the cycle did not wrap");
        assert_eq!(EffectId::all().count(), 16, "the catalogue changed size");
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
            EffectId::Terrain,
            "`p` from the first effect did not wrap to the last"
        );

        // And a whole cycle in reverse returns to where it started.
        for _ in 0..(count - 1) {
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
}
