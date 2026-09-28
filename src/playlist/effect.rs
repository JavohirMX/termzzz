use crate::buffer::{Buffer, Cell};
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
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
#[serde(default)]
pub struct PlaylistOptions {
    /// Effects to play, in order. Empty means every registered effect.
    pub effects: Vec<PlaylistEntry>,
    /// Play entries in a random order, reshuffling once every effect has run.
    pub shuffle: bool,
    /// Seconds of blank wipe between effects.
    pub transition: f32,
    /// Seed for the shuffle order.
    ///
    /// Present because `--seed N` promises a reproducible run, and it was not
    /// one: the bag drew from `rand::rng()`, which is OS entropy, so
    /// `--seed 1234 --shuffle` gave a reproducible *picture* in a random
    /// *order*. The same `DEFAULT_SEED` convention as every effect, so a
    /// config pinning 42 means "shuffle it differently each launch" and
    /// `--seed 42` pins the order too.
    pub seed: u64,
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
            seed: DEFAULT_SEED,
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
    /// Held rather than re-seeded per refill -- see [`Playlist::refill`].
    bag_rng: EffectRng,
    position: usize,
    elapsed: f32,
    phase: Phase,
    phase_progress: f32,
    current: AnyEffect,
    /// The frame being built: the frame the terminal is showing plus every
    /// change the running effect has reported since.
    ///
    /// This was a `Canvas`, which is built for an effect that redraws its whole
    /// surface every frame. `Canvas::commit` swaps its two surfaces, so what it
    /// hands back to draw on next is the frame from *before* the one it just
    /// emitted -- one commit too old to accumulate a diff onto. A playlist
    /// accumulates diffs, so every cell the effect did not redraw was reported
    /// as going blank and then redrawn on the frame after: the screen flickered
    /// between the frame and a mostly empty one, sixty times a second.
    frame: Buffer,
    /// The frame the terminal is showing. The diff is against this, and it is
    /// what lets a wipe blank a cell the effect did not redraw this frame: the
    /// cell is on the screen whether or not it is in the diff.
    shown: Buffer,
    /// A wiped copy of `frame`, allocated only while a transition is running.
    ///
    /// The wipe goes on this rather than on `frame` so the undimmed frame
    /// survives underneath and the wipe can be taken back as the front retreats,
    /// instead of being baked in permanently. `clone_from` reuses the
    /// allocation, so a transition frame costs no memory after the first.
    wiped: Option<Buffer>,
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
        let order: Vec<usize> = (0..slots.len()).collect();
        // The opening effect is *drawn*, not assumed to be the first entry.
        //
        // It used to be `let position = 0`, and that is the whole of
        // "every --shuffle starts from matrix": the bag is only consulted by
        // `next_index`, which handles transitions, so the run always opened on
        // slot 0. With no `--playlist`, `build_slots` falls back to
        // `EffectId::all()`, whose first entry is `matrix` -- so a bare
        // `--shuffle` looked like it did nothing at all.
        //
        // Drawn from the bag rather than by shuffling `order`, so the opening
        // effect comes from the same distribution as every later one and the
        // "never the same effect twice running" rule holds from the first
        // transition instead of starting one step in.
        let mut bag_rng = seeded_rng(options.seed, "playlist.shuffle");
        let mut bag: Vec<usize> = Vec::new();
        let position = if options.shuffle && order.len() > 1 {
            bag = Self::refill(&order, &mut bag_rng);
            bag.remove(0)
        } else {
            0
        };
        let current =
            AnyEffect::build(slots[order[position]].id, &config, screen_size);
        let (width, height) = (screen_size.0 as usize, screen_size.1 as usize);
        let mut playlist = Self {
            config,
            screen_size,
            options,
            slots,
            order,
            bag,
            bag_rng,
            position,
            elapsed: 0.0,
            phase: Phase::Running,
            phase_progress: 0.0,
            current,
            frame: Buffer::new(width, height),
            shown: Buffer::new(width, height),
            wiped: None,
        };
        playlist.rebuild_buffers();
        playlist
    }

    fn build_slots(options: &PlaylistOptions) -> Vec<Slot> {
        let mut unresolvable: Vec<&str> = Vec::new();
        let mut slots: Vec<Slot> = options
            .effects
            .iter()
            .filter_map(|entry| {
                let Ok(id) = entry.effect.parse::<EffectId>() else {
                    unresolvable.push(entry.effect.as_str());
                    return None;
                };
                Some(Slot {
                    id,
                    duration: entry
                        .duration
                        .filter(|duration| duration.is_finite() && *duration > 0.0)
                        .unwrap_or_else(|| id.default_duration()),
                })
            })
            .collect();

        // A name this code cannot resolve used to be dropped in silence, which
        // shortens the playlist with no indication that anything was wrong. That
        // is how a renamed effect takes a user's playlist apart without them
        // finding out until the run looks wrong: `effect = "ascii"` in a file
        // written before the rename to `ink` is now a name that parses to nothing,
        // and the entry simply vanishes.
        //
        // Not an error -- a playlist naming an effect that does not exist is a
        // config mistake, not a crash, and the rest of the playlist is still
        // perfectly playable. But it is reported, because a silent shortening is
        // the worst of the three outcomes.
        if !unresolvable.is_empty() {
            eprintln!(
                "termzzz: ignoring {} playlist entr{} this build cannot resolve: {}",
                unresolvable.len(),
                if unresolvable.len() == 1 { "y" } else { "ies" },
                unresolvable.join(", ")
            );
            let known: Vec<&str> = EffectId::all().map(|id| id.as_str()).collect();
            eprintln!(
                "termzzz: the effects this build knows are: {}",
                known.join(", ")
            );
        }

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

    /// Resizes both frames, and starts a new effect from a blank screen.
    ///
    /// Only the frame being built is blanked. `shown` is the record of what the
    /// terminal is showing, so blanking it would be a lie the next diff cannot
    /// correct: the last frame of a wipe-out is a fraction short of blank, and
    /// declaring it blank meant the leftover was never erased. What the terminal
    /// really shows is the next diff's business, and it erases it.
    ///
    /// It used to blank the surface and then establish the baseline, which is a
    /// commit, which swaps the two -- so the blank became the baseline and the
    /// surface being drawn into kept the outgoing effect's frame. Every swap
    /// then re-emitted that frame, and an effect whose diff is always empty,
    /// like `blank` or `terrain`, repainted the previous effect's image
    /// underneath itself for its whole slot.
    fn rebuild_buffers(&mut self) {
        let (width, height) =
            (self.screen_size.0 as usize, self.screen_size.1 as usize);
        if (self.frame.width, self.frame.height) != (width, height) {
            // Neither frame describes dimensions that no longer exist, and a
            // resize is a full repaint, so neither can be trusted. The terminal
            // is in an unknown state too, which is the one case where `shown`
            // does get blanked.
            self.frame = Buffer::new(width, height);
            self.shown = Buffer::new(width, height);
            self.wiped = None;
            return;
        }
        // This effect's blank is `Color::Reset`, not the `Cell::default()` black
        // a buffer clears to, so it is filled explicitly.
        self.frame.fill_with(&blank_cell());
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
                    // The transition is over, so the copy the wipe was using can
                    // go. A playlist holds this between transitions rather than
                    // for the whole session.
                    self.wiped = None;
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

    /// A fresh, shuffled bag of every index in `order`, drawn from the
    /// playlist's own generator.
    ///
    /// That generator is *held* rather than re-seeded per refill, and the
    /// distinction is load-bearing: reseeding from the same value would produce
    /// the same permutation every round, so a long shuffled playlist would cycle
    /// through one order forever. Holding it means successive refills differ
    /// while the whole sequence is still reproducible from the seed.
    ///
    /// The bag used to draw from `rand::rng()`, which is OS entropy, so
    /// `--seed 1234 --shuffle` was not the reproducible run the flag promises:
    /// the effects looked the same and arrived in a different order each time.
    ///
    /// One call site, used by both the opening draw and every refill, so the
    /// opening effect is drawn the same way as every later one.
    fn refill(order: &[usize], rng: &mut EffectRng) -> Vec<usize> {
        let mut bag = order.to_vec();
        bag.shuffle(rng);
        bag
    }

    /// Sequential order steps forward; shuffle order draws from a reshuffling bag.
    fn next_index(&mut self) -> usize {
        if !self.options.shuffle || self.order.len() < 2 {
            return (self.position + 1) % self.order.len();
        }

        if self.bag.is_empty() {
            self.bag = Self::refill(&self.order, &mut self.bag_rng);
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
        let (width, height) =
            (self.screen_size.0 as usize, self.screen_size.1 as usize);
        if (self.frame.width, self.frame.height) != (width, height) {
            self.rebuild_buffers();
        }

        // The frame accumulates: each effect's diff lands on top of the frame it
        // is building, which is what lets an effect that draws nothing this
        // frame keep the one before it. `Buffer::set` only debug-asserts its
        // bounds, so a coordinate from outside the screen is dropped rather than
        // written past the end.
        for (x, y, cell) in delta {
            if x < width && y < height {
                self.frame.set(x, y, cell);
            }
        }

        let wipe = match self.phase {
            Phase::Running => 0.0,
            Phase::WipeOut => self.phase_progress,
            Phase::WipeIn => 1.0 - self.phase_progress,
        };
        let outgoing = if wipe > 0.0 {
            // Wiped on a copy, so the undimmed frame underneath survives and the
            // wipe can be taken back rather than being baked in permanently --
            // which is also what lets the wipe-in reveal an effect that has
            // nothing new to draw.
            let wiped = self.wiped.get_or_insert_with(|| self.frame.clone());
            wiped.clone_from(&self.frame);
            crate::render::wipe::apply(wiped, width, height, wipe);
            wiped
        } else {
            &self.frame
        };

        let cells = self.shown.diff(outgoing);
        for (x, y, cell) in &cells {
            self.shown.set(*x, *y, *cell);
        }
        cells
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
        playlist_seeded(entries, transition, shuffle, DEFAULT_SEED)
    }

    /// [`playlist_with`] with the seed spelled out, for the tests where the
    /// order itself is the subject.
    fn playlist_seeded(
        entries: &[(&str, Option<f32>)],
        transition: f32,
        shuffle: bool,
        seed: u64,
    ) -> Playlist {
        let options = PlaylistOptions {
            seed,
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

    /// The opening effect is drawn, not assumed to be the first entry.
    ///
    /// This is the whole of "every `--shuffle` starts from matrix".
    /// `Playlist::new` hardcoded `let position = 0`, and the bag is only
    /// consulted for *transitions*, so a run always opened on slot 0. With no
    /// `--playlist` that slot is `EffectId::all()[0]`, which is `matrix` — so a
    /// bare `--shuffle` looked like it did nothing at all.
    ///
    /// Asserted as a distribution rather than as "not matrix", because "not
    /// matrix" is satisfiable by an order that is biased some other way. Two
    /// hundred seeds, and every effect has to turn up.
    #[test]
    fn a_shuffled_playlist_opens_on_a_drawn_effect() {
        let entries: Vec<(&str, Option<f32>)> =
            EffectId::all().map(|id| (id.as_str(), None)).collect();
        let mut firsts: std::collections::BTreeMap<String, usize> =
            Default::default();
        for seed in 0..200u64 {
            let playlist = playlist_seeded(&entries, 0.6, true, seed);
            *firsts
                .entry(playlist.current_id().as_str().to_string())
                .or_default() += 1;
        }

        assert_eq!(
            firsts.len(),
            entries.len(),
            "across 200 seeds only {} of the {} effects ever opened the playlist: \
             {:?}",
            firsts.len(),
            entries.len(),
            firsts
        );
        // And not one of them is doing all the work, which is what a shuffled
        // *order* with a fixed *first* entry would look like from further away.
        let busiest = firsts.values().copied().max().unwrap_or(0);
        assert!(
            busiest < 40,
            "one effect opened {busiest} of 200 shuffled playlists, so the order \
             is random but the opening effect is not"
        );
    }

    /// `--seed N` pins the order, not only the pictures.
    ///
    /// The bag drew from `rand::rng()` — OS entropy — so `--seed 1234
    /// --shuffle` was not the reproducible run the flag promises: the effects
    /// looked the same and arrived in a different order. The generator is now
    /// held rather than re-seeded per refill, because re-seeding from the same
    /// value would repeat one permutation forever.
    #[test]
    fn a_seeded_shuffle_reproduces_its_whole_order() {
        let entries: Vec<(&str, Option<f32>)> =
            EffectId::all().map(|id| (id.as_str(), None)).collect();
        let order_of = |seed: u64| -> Vec<String> {
            let mut playlist = playlist_seeded(&entries, 0.0, true, seed);
            let mut seen = vec![playlist.current_id().as_str().to_string()];
            for _ in 0..entries.len() {
                playlist.step_to_next();
                seen.push(playlist.current_id().as_str().to_string());
            }
            seen
        };
        assert_eq!(
            order_of(7),
            order_of(7),
            "two runs at the same seed produced different orders, so --seed does \
             not pin the playlist"
        );
        assert_ne!(
            order_of(7),
            order_of(8),
            "seeds 7 and 8 produced the same order, so the seed is not reaching \
             the shuffle at all"
        );
    }

    /// Successive refills differ, so a long shuffled playlist does not cycle
    /// through one order forever.
    ///
    /// This is the failure a *held* generator was chosen over a per-refill
    /// re-seed, and it is invisible in a test that only looks at the first few
    /// entries.
    #[test]
    fn a_long_shuffled_playlist_does_not_repeat_one_order() {
        let entries: Vec<(&str, Option<f32>)> =
            EffectId::all().map(|id| (id.as_str(), None)).collect();
        let count = entries.len();
        let mut playlist = playlist_seeded(&entries, 0.0, true, 3);
        let mut orders: std::collections::BTreeSet<Vec<usize>> = Default::default();
        for _ in 0..6 {
            let mut round: Vec<usize> = Vec::new();
            for _ in 0..=count {
                round.push(playlist.position);
                playlist.step_to_next();
            }
            orders.insert(round);
        }
        assert!(
            orders.len() >= 3,
            "six full rounds of a shuffled playlist produced only {} distinct \
             orders, so it is cycling rather than shuffling",
            orders.len()
        );
    }

    /// A playlist too short to shuffle still works.
    ///
    /// The opening draw is new code on a path that used to be a constant, and a
    /// single-entry list is the case where an off-by-one in it would panic
    /// rather than merely look wrong.
    #[test]
    fn a_single_effect_playlist_still_opens_on_that_effect() {
        for shuffle in [false, true] {
            let mut playlist = playlist_with(&[("matrix", None)], 0.0, shuffle);
            assert_eq!(playlist.current_id().as_str(), "matrix");
            playlist.step_to_next();
            assert_eq!(
                playlist.current_id().as_str(),
                "matrix",
                "a one-entry playlist moved to something else"
            );
        }
    }

    /// The opening effect is not repeated by the first transition.
    ///
    /// The bag's own rule is "keep drawing until something other than the
    /// current effect comes up". It used to start one entry late, so the run
    /// opened on slot 0 and could then draw slot 0 again immediately.
    #[test]
    fn the_first_transition_does_not_repeat_the_opening_effect() {
        let entries: Vec<(&str, Option<f32>)> =
            EffectId::all().map(|id| (id.as_str(), None)).collect();
        for seed in 0..100u64 {
            let mut playlist = playlist_seeded(&entries, 0.0, true, seed);
            let first = playlist.current_id();
            playlist.step_to_next();
            assert_ne!(
                playlist.current_id(),
                first,
                "seed {seed} opened on {} and then stayed on it",
                first.as_str()
            );
        }
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

    /// One frame of the real loop: the effect is advanced, then asked for its
    /// diff, and the result lands on `screen`.
    fn tick(
        playlist: &mut Playlist,
        frame: u64,
        screen: &mut Buffer,
    ) -> Vec<(usize, usize, Cell)> {
        let context = FrameContext::new(
            (40, 12),
            frame,
            Duration::ZERO,
            Duration::from_secs_f64(1.0 / 60.0),
            InputState::default(),
        );
        playlist.update_with_context(&context);
        let cells = playlist.get_diff_with_context(&context);
        for (x, y, cell) in &cells {
            screen.set(*x, *y, *cell);
        }
        cells
    }

    /// How much of the screen is showing something other than a space.
    fn lit(buffer: &Buffer) -> usize {
        (0..buffer.height)
            .flat_map(|y| (0..buffer.width).map(move |x| (x, y)))
            .filter(|(x, y)| buffer.get(*x, *y).symbol != ' ')
            .count()
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
        assert_eq!(playlist.frame.get_size(), (20, 8));
        assert_eq!(playlist.shown.get_size(), (20, 8));
    }

    /// The accumulated frame is the frame, not a frame behind it.
    ///
    /// The playlist accumulates the running effect's *diff*, so the base it
    /// accumulates onto has to be the frame the terminal is showing. It was a
    /// `Canvas`, whose commit swaps its two surfaces and so hands back the frame
    /// from before the one it just emitted -- one commit too old. Every cell the
    /// effect did not redraw that frame was therefore reported as going blank
    /// and redrawn on the frame after, and the screen flickered between the
    /// frame and an almost empty one, sixty times a second. The bare effect
    /// emits about fifteen times as many cells as this did.
    #[test]
    fn the_accumulated_frame_keeps_up_with_the_terminal() {
        let mut playlist = playlist_with(&[("life", Some(20.0))], 0.6, false);
        let mut screen = Buffer::new(40, 12);

        // What a bare effect draws in the same number of frames, for scale.
        let mut bare =
            AnyEffect::build(EffectId::Life, &Config::default(), (40, 12));
        let mut bare_screen = Buffer::new(40, 12);
        let context = FrameContext::new(
            (40, 12),
            0,
            Duration::ZERO,
            Duration::from_secs_f64(1.0 / 60.0),
            InputState::default(),
        );
        let mut bare_cells = 0;
        for _ in 0..60 {
            bare.update_with_context(&context);
            let diff = bare.get_diff_with_context(&context);
            bare_cells += diff.len();
            for (x, y, cell) in &diff {
                bare_screen.set(*x, *y, *cell);
            }
        }

        let mut cells = 0;
        let mut frame = 0;
        while frame < 60 {
            cells += tick(&mut playlist, frame, &mut screen).len();
            frame += 1;
        }

        assert!(
            cells * 4 > bare_cells,
            "the playlist emitted {cells} cells where the effect on its own \
             emits {bare_cells}, so it is not keeping up with the screen"
        );
        // And the two agree about what the screen looks like, which is the part
        // that actually matters: the playlist is not losing the frame.
        assert_eq!(screen.buffer, bare_screen.buffer);
    }

    #[test]
    fn input_is_forwarded_to_the_active_effect() {
        let mut playlist = playlist_with(&[("ink", Some(5.0))], 0.6, false);
        let is_paused = |playlist: &mut Playlist| match &playlist.current {
            AnyEffect::Ink(field) => field.paused,
            _ => false,
        };
        assert!(!is_paused(&mut playlist));

        playlist.handle_input(&InputEvent::Key {
            key: crate::runtime::Key::Space,
            phase: crate::runtime::KeyPhase::Pressed,
        });

        assert!(is_paused(&mut playlist));
    }

    /// A static effect is still revealed by the wipe.
    ///
    /// `terrain` draws one frame and then reports nothing at all for the rest of
    /// its slot, so the wipe-in has to reveal the frame it already has rather
    /// than wait for changes that never come. That only works if the
    /// accumulation survives the wipe, which is what applying the wipe to a copy
    /// is for -- and it did not: the wipe was baked into the surface the next
    /// frame accumulated onto, so the static effect's one and only frame was
    /// destroyed by the first frame of its own wipe-in and the slot was blank.
    #[test]
    fn a_static_effect_is_revealed_by_the_wipe_in() {
        let mut playlist = playlist_with(
            &[("dvd", Some(0.05)), ("terrain", Some(5.0))],
            0.6,
            false,
        );
        let mut screen = Buffer::new(40, 12);
        let mut frame = 0;

        while playlist.current_id() != EffectId::Terrain {
            tick(&mut playlist, frame, &mut screen);
            frame += 1;
            assert!(frame < 1000, "the playlist never reached the second slot");
        }

        // From the swap onwards, the most the screen ever holds. Terrain fills
        // most of the screen, so a slot that shows less than a third of it never
        // revealed the effect.
        let mut fullest = 0;
        for _ in 0..200 {
            tick(&mut playlist, frame, &mut screen);
            fullest = fullest.max(lit(&screen));
            frame += 1;
        }

        assert!(
            fullest > 40 * 12 / 3,
            "a static effect's frame was never revealed: the screen held at \
             most {fullest} of {} cells",
            40 * 12
        );
    }

    /// A rebuilt playlist starts from a blank screen, not from the frame the
    /// outgoing effect was leaving.
    ///
    /// `rebuild_buffers` blanked the surface and established the baseline, and
    /// establishing a baseline is a commit, and a commit swaps the canvas's two
    /// surfaces -- so the frame on screen became the blank while the surface
    /// being drawn into kept the outgoing effect. `render_full_frame` then
    /// accumulated onto that, so every swap re-emitted the outgoing frame, and
    /// an effect whose diff is always empty painted the previous effect's image
    /// underneath itself for its whole slot. `blank` is that effect.
    #[test]
    fn a_rebuilt_effect_does_not_re_emit_the_outgoing_frame() {
        let mut playlist =
            playlist_with(&[("dvd", Some(0.05)), ("blank", Some(5.0))], 0.6, false);
        let mut screen = Buffer::new(40, 12);

        // Run the first slot out and the wipe through into the second, keeping
        // the most the screen ever had on it: by the time the swap lands the
        // screen is blank again, which is the point.
        let mut frame = 0;
        let mut fullest = 0;
        while playlist.current_id() != EffectId::Blank {
            tick(&mut playlist, frame, &mut screen);
            frame += 1;
            fullest = fullest.max(lit(&screen));
            assert!(frame < 1000, "the playlist never reached the second slot");
        }
        assert!(fullest > 0, "nothing was ever drawn to carry over");

        // The surface the new effect's frame is built on is blank. The effect's
        // first frame covers every cell it draws and nothing at all for the
        // cells it leaves blank, so an accumulation still holding the outgoing
        // effect shows it through underneath for the whole slot.
        assert_eq!(
            lit(&playlist.frame),
            0,
            "the outgoing effect is still in the frame the new effect builds on"
        );

        // And from here the playlist is showing `blank`, which draws nothing at
        // all, so everything it reports has to be blank.
        for _ in 0..120 {
            for (x, y, cell) in tick(&mut playlist, frame, &mut screen) {
                assert_eq!(
                    cell.symbol, ' ',
                    "the outgoing effect's frame was re-emitted at ({x},{y})"
                );
            }
            frame += 1;
        }

        assert_eq!(
            lit(&screen),
            0,
            "the previous effect stayed on screen through the whole slot"
        );
    }
}
