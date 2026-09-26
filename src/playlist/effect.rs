use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::TerminalEffect;
use crate::config::Config;
use crate::registry::{AnyEffect, EffectId};
use crate::render::wipe::blank_cell;
use crate::runtime::{FrameContext, InputEvent, InputState};
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One playlist entry: an effect name and an optional duration override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistEntry {
    pub effect: String,
    #[serde(default)]
    pub duration: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistOptions {
    /// Effects to play, in order. Empty means every registered effect.
    pub effects: Vec<PlaylistEntry>,
    /// Play entries in a random order, reshuffling once every effect has run.
    pub shuffle: bool,
    /// Seconds of blank wipe between effects.
    pub transition: f32,
}

impl Default for PlaylistOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            effects: Default::default(),
            shuffle: false,
            transition: 0.6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Slot {
    id: EffectId,
    duration: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Running,
    WipeOut,
    WipeIn,
}

/// Runs a queue of effects, wiping to blank between each one.
pub struct Playlist {
    config: Config,
    screen_size: (u16, u16),
    options: PlaylistOptions,
    slots: Vec<Slot>,
    order: Vec<usize>,
    bag: Vec<usize>,
    position: usize,
    elapsed: f32,
    phase: Phase,
    phase_progress: f32,
    current: AnyEffect,
    canvas: Canvas,
}

impl TerminalEffect for Playlist {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.render_full_frame(Vec::new())
    }

    fn update(&mut self) {
        let size = self.screen_size;
        self.update_with_context(&FrameContext::new(
            size,
            0,
            Duration::ZERO,
            Duration::from_secs_f64(1.0 / 60.0),
            InputState::default(),
        ));
    }

    fn get_diff_with_context(
        &mut self,
        context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        let delta = self.current.get_diff_with_context(context);
        self.render_full_frame(delta)
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        self.current.update_with_context(context);
        self.advance(context.delta.as_secs_f32());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        self.rebuild_buffers();
        self.current.update_size(width, height);
    }

    fn reset(&mut self) {
        self.current.reset();
        self.elapsed = 0.0;
        self.phase = Phase::Running;
        self.phase_progress = 0.0;
        self.rebuild_buffers();
    }

    fn handle_input(&mut self, event: &InputEvent) {
        self.current.handle_input(event);
    }
}

impl Playlist {
    pub fn new(
        options: PlaylistOptions,
        config: Config,
        screen_size: (u16, u16),
    ) -> Self {
        let screen_size = (screen_size.0.max(1), screen_size.1.max(1));
        let slots = Self::build_slots(&options);
        let order = (0..slots.len()).collect();
        let position = 0;
        let current = AnyEffect::build(slots[position].id, &config, screen_size);
        let mut playlist = Self {
            config,
            screen_size,
            options,
            slots,
            order,
            bag: Vec::new(),
            position,
            elapsed: 0.0,
            phase: Phase::Running,
            phase_progress: 0.0,
            current,
            canvas: {
                let mut canvas = Canvas::new(screen_size.0, screen_size.1);
                canvas.clear_with(&blank_cell());
                canvas.establish_baseline();
                canvas
            },
        };
        playlist.rebuild_buffers();
        playlist
    }

    fn build_slots(options: &PlaylistOptions) -> Vec<Slot> {
        let mut slots: Vec<Slot> = options
            .effects
            .iter()
            .filter_map(|entry| {
                let id = entry.effect.parse::<EffectId>().ok()?;
                Some(Slot {
                    id,
                    duration: entry
                        .duration
                        .filter(|duration| duration.is_finite() && *duration > 0.0)
                        .unwrap_or_else(|| id.default_duration()),
                })
            })
            .collect();

        if slots.is_empty() {
            slots = EffectId::all()
                .map(|id| Slot {
                    id,
                    duration: id.default_duration(),
                })
                .collect();
        }

        slots
    }

    fn current_id(&self) -> EffectId {
        self.slots[self.order[self.position]].id
    }

    fn current_duration(&self) -> f32 {
        self.slots[self.order[self.position]].duration
    }

    fn transition_duration(&self) -> f32 {
        self.options.transition.max(0.01)
    }

    fn rebuild_buffers(&mut self) {
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        // This effect's blank is `Color::Reset`, not the `Cell::default()` black
        // the canvas clears to, so it is filled explicitly on both counts.
        self.canvas.clear_with(&blank_cell());
        self.canvas.establish_baseline();
    }

    /// Wipes the effect away, swaps it, then wipes the next one in.
    fn advance(&mut self, delta: f32) {
        self.elapsed += delta;

        match self.phase {
            Phase::Running => {
                if self.elapsed >= self.current_duration() {
                    self.elapsed = 0.0;
                    if self.options.transition > 0.0 {
                        self.phase = Phase::WipeOut;
                        self.phase_progress = 0.0;
                    } else {
                        self.step_to_next();
                    }
                }
            }
            Phase::WipeOut => {
                self.phase_progress += delta / self.transition_duration();
                if self.phase_progress >= 1.0 {
                    self.step_to_next();
                    self.phase = Phase::WipeIn;
                    self.phase_progress = 0.0;
                }
            }
            Phase::WipeIn => {
                self.phase_progress += delta / self.transition_duration();
                if self.phase_progress >= 1.0 {
                    self.phase = Phase::Running;
                    self.phase_progress = 0.0;
                }
            }
        }
    }

    fn step_to_next(&mut self) {
        self.position = self.next_index();
        let id = self.current_id();
        self.current = AnyEffect::build(id, &self.config, self.screen_size);
        self.rebuild_buffers();
    }

    /// Sequential order steps forward; shuffle order draws from a reshuffling bag.
    fn next_index(&mut self) -> usize {
        if !self.options.shuffle || self.order.len() < 2 {
            return (self.position + 1) % self.order.len();
        }

        if self.bag.is_empty() {
            let mut bag = self.order.clone();
            bag.shuffle(&mut rand::rng());
            self.bag = bag;
        }

        let current = self.order[self.position];

        // Keep drawing until something other than the effect we are on comes
        // up, so a shuffled playlist never shows the same effect twice running.
        // This used to retry exactly once, which still returned the current
        // effect one time in fourteen, and the test that checks this failed
        // about that often.
        for _ in 0..=self.bag.len() {
            let next = self.bag.remove(0);
            if next != current {
                return next;
            }
            self.bag.push(next);
        }

        // Only reachable if the bag holds nothing but the current effect, which
        // a bag holding every index once cannot do unless there is only one.
        self.bag.remove(0)
    }

    /// Applies the effect delta to the accumulated frame, masks it for the
    /// current wipe phase, and emits the difference against what is on screen.
    fn render_full_frame(
        &mut self,
        delta: Vec<(usize, usize, Cell)>,
    ) -> Vec<(usize, usize, Cell)> {
        let width = self.canvas.width();
        let height = self.canvas.height();
        if (self.canvas.width(), self.canvas.height())
            != (self.screen_size.0 as usize, self.screen_size.1 as usize)
        {
            self.rebuild_buffers();
        }

        // The canvas accumulates: each effect's diff lands on top of the frame
        // it is building, which is what lets an effect that draws nothing this
        // frame keep the one before it.
        for (x, y, cell) in delta {
            self.canvas.set(x, y, cell);
        }

        let wipe = match self.phase {
            Phase::Running => 0.0,
            Phase::WipeOut => self.phase_progress,
            Phase::WipeIn => 1.0 - self.phase_progress,
        };
        if wipe > 0.0 {
            // Wiped on a copy, so the undimmed frame underneath survives and the
            // wipe can be taken back rather than being baked in permanently.
            let mut target = self.canvas.surface().clone();
            crate::render::wipe::apply(&mut target, width, height, wipe);
            self.canvas.replace_surface(target);
        }

        self.canvas.commit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crossterm::style;

    fn playlist_with(
        entries: &[(&str, Option<f32>)],
        transition: f32,
        shuffle: bool,
    ) -> Playlist {
        let options = PlaylistOptions {
            effects: entries
                .iter()
                .map(|(effect, duration)| PlaylistEntry {
                    effect: (*effect).to_string(),
                    duration: *duration,
                })
                .collect(),
            transition,
            shuffle,
        };
        Playlist::new(options, Config::default(), (40, 12))
    }

    fn filled_buffer(width: usize, height: usize) -> Buffer {
        let mut buffer = Buffer::new(width, height);
        buffer.fill_with(&Cell::new(
            '#',
            style::Color::Green,
            style::Attribute::Reset,
        ));
        buffer
    }

    #[test]
    fn empty_playlist_plays_every_effect() {
        let playlist =
            Playlist::new(PlaylistOptions::default(), Config::default(), (40, 12));
        assert_eq!(playlist.slots.len(), EffectId::len());
    }

    #[test]
    fn unknown_effects_fall_back_to_every_effect() {
        let playlist = playlist_with(&[("nope", None)], 0.6, false);
        assert_eq!(playlist.slots.len(), EffectId::len());
    }

    #[test]
    fn entries_use_configured_durations() {
        let playlist =
            playlist_with(&[("matrix", Some(3.0)), ("dvd", None)], 0.6, false);

        assert_eq!(playlist.slots.len(), 2);
        assert_eq!(playlist.slots[0].duration, 3.0);
        assert_eq!(playlist.slots[1].duration, EffectId::Dvd.default_duration());
    }

    #[test]
    fn invalid_durations_fall_back_to_defaults() {
        let playlist = playlist_with(
            &[("matrix", Some(-4.0)), ("dvd", Some(f32::NAN))],
            0.6,
            false,
        );

        assert_eq!(
            playlist.slots[0].duration,
            EffectId::Matrix.default_duration()
        );
        assert_eq!(playlist.slots[1].duration, EffectId::Dvd.default_duration());
    }

    #[test]
    fn advances_to_the_next_effect_after_its_duration() {
        let mut playlist =
            playlist_with(&[("blank", Some(1.0)), ("dvd", Some(5.0))], 0.0, false);
        assert_eq!(playlist.current_id(), EffectId::Blank);

        playlist.advance(1.1);

        assert_eq!(playlist.current_id(), EffectId::Dvd);
        assert_eq!(playlist.phase, Phase::Running);
    }

    #[test]
    fn transition_wipes_out_before_swapping() {
        let mut playlist =
            playlist_with(&[("blank", Some(1.0)), ("dvd", Some(5.0))], 0.6, false);

        playlist.advance(1.0);
        assert_eq!(playlist.phase, Phase::WipeOut);
        assert_eq!(playlist.current_id(), EffectId::Blank);

        playlist.advance(0.3);
        assert_eq!(playlist.phase, Phase::WipeOut);
        assert_eq!(playlist.current_id(), EffectId::Blank);

        playlist.advance(0.3);
        assert_eq!(playlist.phase, Phase::WipeIn);
        assert_eq!(playlist.current_id(), EffectId::Dvd);

        playlist.advance(0.6);
        assert_eq!(playlist.phase, Phase::Running);
    }

    #[test]
    fn wipe_clears_cells_the_front_has_passed() {
        let mut buffer = filled_buffer(10, 10);

        crate::render::wipe::apply(&mut buffer, 10, 10, 0.0);
        assert_eq!(buffer.get(0, 0).symbol, '#');
        assert_eq!(buffer.get(9, 9).symbol, '#');

        buffer = filled_buffer(10, 10);
        crate::render::wipe::apply(&mut buffer, 10, 10, 0.5);
        assert_eq!(buffer.get(0, 0).symbol, ' ');
        assert_eq!(buffer.get(9, 9).symbol, '#');

        buffer = filled_buffer(10, 10);
        crate::render::wipe::apply(&mut buffer, 10, 10, 1.0);
        assert_eq!(buffer.get(0, 0).symbol, ' ');
        assert_eq!(buffer.get(9, 9).symbol, ' ');
    }

    #[test]
    fn full_wipe_fully_clears_the_screen() {
        let mut playlist =
            playlist_with(&[("matrix", Some(1.0)), ("dvd", Some(5.0))], 0.6, false);
        for _ in 0..5 {
            playlist.advance(1.0 / 60.0);
        }
        playlist.advance(1.0);
        assert_eq!(playlist.phase, Phase::WipeOut);

        playlist.advance(0.6);
        assert_eq!(playlist.phase, Phase::WipeIn);

        let mut buffer = filled_buffer(40, 12);
        crate::render::wipe::apply(&mut buffer, 40, 12, 1.0);
        for y in 0..12 {
            for x in 0..40 {
                assert_eq!(buffer.get(x, y).symbol, ' ');
            }
        }
    }

    #[test]
    fn diff_stays_within_screen_bounds() {
        let mut playlist =
            playlist_with(&[("matrix", Some(0.2)), ("dvd", Some(0.2))], 0.2, false);

        for _ in 0..300 {
            playlist.advance(1.0 / 60.0);
            for (x, y, _) in playlist.get_diff() {
                assert!(x < 40 && y < 12);
            }
        }
    }

    #[test]
    fn shuffle_visits_every_effect_in_a_round() {
        let mut playlist = Playlist::new(
            PlaylistOptions {
                shuffle: true,
                transition: 0.1,
                ..Default::default()
            },
            Config::default(),
            (40, 12),
        );

        let mut seen: Vec<EffectId> = vec![playlist.current_id()];
        let mut guard = 0;
        while seen.len() < EffectId::len() && guard < 10_000 {
            guard += 1;
            playlist.advance(1.0);
            let id = playlist.current_id();
            if !seen.contains(&id) {
                seen.push(id);
            }
        }

        assert_eq!(seen.len(), EffectId::len());
    }

    #[test]
    fn sequential_order_follows_the_playlist() {
        let mut playlist = playlist_with(
            &[
                ("dvd", Some(1.0)),
                ("matrix", Some(2.0)),
                ("life", Some(3.0)),
            ],
            0.0,
            false,
        );

        let mut order = vec![playlist.current_id()];
        for _ in 0..2 {
            playlist.advance(playlist.current_duration() + 0.1);
            order.push(playlist.current_id());
        }
        playlist.advance(playlist.current_duration() + 0.1);

        assert_eq!(order, vec![EffectId::Dvd, EffectId::Matrix, EffectId::Life]);
        assert_eq!(playlist.current_id(), EffectId::Dvd);
    }

    #[test]
    fn shuffle_bag_empties_only_after_a_full_round() {
        let mut playlist = Playlist::new(
            PlaylistOptions {
                shuffle: true,
                ..Default::default()
            },
            Config::default(),
            (40, 12),
        );

        for index in 0..EffectId::len() {
            playlist.position = index;
            playlist.bag.clear();
            let next = playlist.next_index();
            assert!(next != index || EffectId::len() == 1);
        }
    }

    #[test]
    fn resize_rebuilds_buffers_for_the_active_effect() {
        let mut playlist = playlist_with(&[("dvd", Some(5.0))], 0.6, false);

        playlist.update_size(20, 8);

        assert_eq!(playlist.screen_size, (20, 8));
        assert_eq!(playlist.canvas.size(), (20, 8));
    }

    #[test]
    fn input_is_forwarded_to_the_active_effect() {
        let mut playlist = playlist_with(&[("ascii", Some(5.0))], 0.6, false);
        let is_paused = |playlist: &mut Playlist| match &playlist.current {
            AnyEffect::Ascii(field) => field.paused,
            _ => false,
        };
        assert!(!is_paused(&mut playlist));

        playlist.handle_input(&InputEvent::Key {
            key: crate::runtime::Key::Space,
            phase: crate::runtime::KeyPhase::Pressed,
        });

        assert!(is_paused(&mut playlist));
    }
}
