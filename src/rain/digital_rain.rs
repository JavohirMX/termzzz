use super::draw::{pick_color, pick_style};
use super::gradient;
use super::rain_drop::RainDrop;
use crate::buffer::{Buffer, Cell};
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};

use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DigitalRainOptions {
    /// Derived from the terminal size by the config layer, so not persisted.
    #[serde(skip)]
    pub drops_range: (u16, u16),
    /// Derived from the terminal size by the config layer, so not persisted.
    #[serde(skip)]
    pub speed_range: (u16, u16),
    pub drops_coeff: f32,
    pub speed_coeff: f32,
    /// Seed for drop placement, style, length and speed.
    pub seed: u64,
}

impl Default for DigitalRainOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file that
    /// omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            drops_range: (10, 20),
            speed_range: (2, 16),
            drops_coeff: 1.0,
            speed_coeff: 1.0,
            seed: DEFAULT_SEED,
        }
    }
}

pub struct DigitalRain {
    pub screen_size: (u16, u16),
    options: DigitalRainOptions,
    /// The colour ramp the `Back` drops fade along. See [`DigitalRain::build_ramp`].
    ramp: Vec<gradient::Color>,
    rain_drops: Vec<RainDrop>,
    buffer: Buffer,
    rng: EffectRng,
}

impl TerminalEffect for DigitalRain {
    /// Calculate difference between current frame and previous frame
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // NOTE: this still allocates a full-screen `Buffer` per frame, which is
        // the pattern `Canvas` exists to remove -- `new` builds one, `update_size`
        // throws it away and builds another, and every effect in this crate holds
        // a `Canvas` instead. Not converted here because matrix was explicitly
        // out of scope for the colour work, and the change is not only a
        // substitution: `Canvas::commit` swaps the two surfaces rather than
        // assigning, so the baseline semantics after `update_size` have to be
        // re-derived rather than copied. Worth its own pass, not a drive-by.
        let mut curr_buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);

        // fill current buffer
        // first draw drops with bigger fy
        Self::fill_buffer(&self.rain_drops, &mut curr_buffer, &self.ramp);

        let diff = self.buffer.diff(&curr_buffer);
        self.buffer = curr_buffer;
        diff
    }

    /// Update each rain drop position
    fn update(&mut self) {
        self.update_rain(Duration::from_secs_f64(1.0 / 60.0));
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.update_rain(context.delta);
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width.max(1), height.max(1));
        let area = self.screen_size.0 as f32 * self.screen_size.1 as f32;
        self.options.drops_range = (
            ((area / 160.0 * self.options.drops_coeff) as u16).max(10),
            ((area / 80.0 * self.options.drops_coeff) as u16).max(20),
        );
        self.options.speed_range = (
            ((self.screen_size.1 as f32 / 20.0 * self.options.speed_coeff) as u16)
                .max(2),
            ((self.screen_size.1 as f32 / 10.0 * self.options.speed_coeff) as u16)
                .max(16),
        );

        // The ramp is sized from the height and drops are sized from the height
        // too, so the two have to be rebuilt together. Leaving the ramp at the
        // height the effect was constructed at is what made a grow panic: a new
        // drop could be up to `2h/3` long against a ramp built for the old,
        // shorter height.
        self.ramp = Self::build_ramp(self.screen_size.1);

        // The baseline frame has to follow the screen, or it silently stops
        // describing it. `Buffer::diff` walks `0..min(len)`, so a baseline left
        // at the old size compares only the first `old_len` cells of the new
        // frame and everything past them is never reported -- which on a grow
        // means the added rows and columns are never painted at all.
        //
        // Rebuilt blank rather than resized, because nothing about a frame drawn
        // for the old dimensions can be trusted for the new ones. That makes the
        // next `get_diff` report the whole screen, which is what a resize needs.
        self.buffer =
            Buffer::new(self.screen_size.0 as usize, self.screen_size.1 as usize);
    }

    fn reset(&mut self) {
        let new_effect = DigitalRain::new(self.options.clone(), self.screen_size);
        *self = new_effect;
    }
}

/// Process digital rain effect.
/// Note that all processing done implying coordinates started from 0, 0
/// and width / height is actual number of columns and rows
impl DigitalRain {
    // Initialize screensaver
    pub fn new(options: DigitalRainOptions, screen_size: (u16, u16)) -> Self {
        let mut rng = seeded_rng(options.seed, "matrix");
        let mut rain_drops: Vec<RainDrop> = vec![];
        let mut buffer: Buffer =
            Buffer::new(screen_size.0 as usize, screen_size.1 as usize);
        for rain_drop_id in 1..=options.get_min_drops_number() {
            rain_drops.push(RainDrop::new(
                screen_size,
                &options,
                rain_drop_id as usize,
                &mut rng,
            ));
        }

        // fill the ramp the Back drops fade along
        let ramp = Self::build_ramp(screen_size.1);
        Self::sort_drops(&mut rain_drops);

        Self::fill_buffer(&rain_drops, &mut buffer, &ramp);

        Self {
            screen_size,
            options,
            ramp,
            rain_drops,
            buffer,
            rng,
        }
    }

    /// The colour ramp the `Back` drops fade along.
    ///
    /// Deliberately one ramp, not three. There used to be three, built with
    /// three different head colours and step counts, and `pick_color` read only
    /// `gradients[2]` -- the other two were allocated per construction and never
    /// indexed by anything. One of them started at pure `(255, 255, 255)`, so
    /// the dead set was also a dead *white*.
    ///
    /// A free function of the height rather than a constant because the length
    /// is what makes it line up with the drops: a drop is at most `2h/3` cells
    /// (see `RainDrop::new`) and this is `3h/2`, so the ramp is always longer
    /// than the drops that index it, at any height. That relationship was the
    /// only thing making `ramp[pos]` safe, and it stopped holding as soon as
    /// the terminal grew past the height the effect was built at. `update_size`
    /// rebuilds the ramp and `draw::ramp_at` clamps as well.
    pub(crate) fn build_ramp(height: u16) -> Vec<gradient::Color> {
        let height = height as usize;
        gradient::two_step_color_gradient(
            // Was `rgb(200, 200, 200)` -- a light grey at 78% luminance, and the
            // ramp is indexed by *absolute* drop position rather than by a
            // fraction of the drop. So the pale stretch at the top of every back
            // drop grew with the terminal: about 20 cells at 200 rows, against 2
            // at 50. Those are non-head cells rendering as near-white, which is
            // the other half of "the gray lines flash white".
            //
            // The back layer now starts as a dim green and recedes to near-black,
            // so it reads as distance rather than as brightness.
            gradient::Color { r: 0, g: 80, b: 0 },
            gradient::Color { r: 0, g: 110, b: 0 },
            gradient::Color { r: 0, g: 25, b: 0 },
            height / 2,
            3 * height / 2,
        )
    }

    /// Orders drops slowest-first, so the fast ones are drawn last and end up on
    /// top.
    ///
    /// This belongs in `update` rather than in the draw. Sorting is idempotent,
    /// so re-sorting every frame produced the same order every frame while
    /// making the draw path -- which is supposed to be a pure function of the
    /// simulation -- a mutation. It also runs exactly as often as it needs to:
    /// the order only changes when a drop is added, and that happens in
    /// `update`.
    fn sort_drops(rain_drops: &mut [RainDrop]) {
        rain_drops.sort_by(|a, b| a.speed.partial_cmp(&b.speed).unwrap());
    }

    fn update_rain(&mut self, delta: Duration) {
        for rain_drop in self.rain_drops.iter_mut() {
            rain_drop.update(self.screen_size, &self.options, delta, &mut self.rng);
        }
        self.add_one();
        // After `add_one`, because a newly added drop is appended and has to be
        // slotted into the order too.
        Self::sort_drops(&mut self.rain_drops);
    }

    pub fn fill_buffer(
        rain_drops: &[RainDrop],
        buffer: &mut Buffer,
        ramp: &[gradient::Color],
    ) {
        for rain_drop in rain_drops.iter().rev() {
            let points = rain_drop.to_points_vec();
            let len = points.len();
            for (index, (x, y, character)) in points.iter().enumerate() {
                let (width, height) = buffer.get_size();
                if *x < width as u16 && *y < height as u16 {
                    buffer.set(
                        *x as usize,
                        *y as usize,
                        Cell::new(
                            *character,
                            pick_color(&rain_drop.style, index, len, ramp),
                            pick_style(&rain_drop.style, index),
                        ),
                    );
                };
            }
        }
    }

    /// Add one more worm with decent chance
    pub fn add_one(&mut self) {
        if self.rain_drops.len() >= self.options.get_max_drops_number() as usize {
            return;
        };
        // Uses the carried generator rather than a fresh thread-local one, so the
        // sequence of additions is part of the reproducible run.
        let roll = self.rng.random_range(0.0..=1.0);
        if roll <= 0.3 {
            self.rain_drops.push(RainDrop::new(
                self.screen_size,
                &self.options,
                self.rain_drops.len() + 1,
                &mut self.rng,
            ));
        };
    }
}

impl DigitalRainOptions {
    #[inline]
    pub fn get_min_drops_number(&self) -> u16 {
        self.drops_range.0
    }

    #[inline]
    pub fn get_max_drops_number(&self) -> u16 {
        self.drops_range.1
    }

    #[inline]
    pub fn get_min_speed(&self) -> u16 {
        self.speed_range.0
    }

    #[inline]
    pub fn get_max_speed(&self) -> u16 {
        self.speed_range.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn get_sane_default_options() -> DigitalRainOptions {
        DigitalRainOptions {
            drops_range: (20, 30),
            speed_range: (10, 20),
            ..Default::default()
        }
    }

    #[test]
    fn create_new() {
        let foo = DigitalRain::new(get_sane_default_options(), (100, 100));
        assert_eq!(foo.rain_drops.len(), 20);
    }

    #[test]
    fn resize_recomputes_drop_and_speed_ranges() {
        let options = DigitalRainOptions {
            drops_range: (50, 60),
            speed_range: (3, 4),
            drops_coeff: 1.0,
            speed_coeff: 1.0,
            seed: DEFAULT_SEED,
        };
        let mut rain = DigitalRain::new(options, (80, 40));

        rain.update_size(10, 10);

        assert_eq!(rain.options.drops_range, (10, 20));
        assert_eq!(rain.options.speed_range, (2, 16));
    }

    #[test]
    fn no_diff() {
        let mut foo = DigitalRain::new(get_sane_default_options(), (100, 100));
        let q = foo.get_diff();
        assert!(q.is_empty());
    }

    #[test]
    fn same_diff_and_update() {
        let mut foo = DigitalRain::new(get_sane_default_options(), (100, 100));
        let mut q = Vec::new();
        for _ in 0..60 {
            foo.update();
            q = foo.get_diff();
            if !q.is_empty() {
                break;
            }
        }
        assert!(!q.is_empty());
    }

    #[test]
    fn quantum_update_moves_less_than_the_legacy_step() {
        let options = get_sane_default_options();
        let mut quick = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c'],
            crate::rain::rain_drop::RainDropStyle::Back,
            10,
            10.0,
            20,
            10,
        );
        let mut tick = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c'],
            crate::rain::rain_drop::RainDropStyle::Back,
            10,
            10.0,
            20,
            10,
        );

        quick.update(
            (100, 100),
            &options,
            Duration::from_millis(50),
            &mut seeded_rng(1, "rain_test"),
        );
        tick.update(
            (100, 100),
            &options,
            Duration::from_secs_f64(1.0 / 60.0),
            &mut seeded_rng(1, "rain_test"),
        );

        assert!(tick.fy < quick.fy);
    }

    #[test]
    fn the_ramp_is_always_longer_than_the_longest_drop_it_is_indexed_by() {
        // The relationship `draw::ramp_at` no longer has to rely on: a drop is
        // at most `2h/3` cells, and the ramp is `3h/2`.
        for height in [6u16, 24, 50, 200, 400] {
            let ramp = DigitalRain::build_ramp(height);
            let longest_drop = (2 * height / 3) as usize;
            assert!(
                ramp.len() > longest_drop,
                "height {height}: a {longest_drop}-cell drop indexes a {}-cell ramp",
                ramp.len()
            );
        }
    }

    #[test]
    fn the_first_frame_after_a_resize_is_a_full_repaint() {
        let mut rain = DigitalRain::new(get_sane_default_options(), (40, 20));
        for _ in 0..200 {
            rain.update();
            rain.get_diff();
        }

        rain.update_size(80, 40);
        let diff = rain.get_diff();

        // What this frame actually contains.
        let mut frame = Buffer::new(80, 40);
        DigitalRain::fill_buffer(&rain.rain_drops, &mut frame, &rain.ramp);
        let lit: HashSet<(usize, usize)> = frame
            .buffer
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.symbol != ' ')
            .map(|(i, _)| frame.pos_of(i))
            .collect();
        assert!(!lit.is_empty(), "the fixture drew nothing");

        // Nothing may be reported outside the new screen either.
        assert!(
            diff.iter().all(|(x, y, _)| *x < 80 && *y < 40),
            "a resize reported cells off the new screen"
        );

        // A frame that describes dimensions which no longer exist cannot be
        // trusted for any of them, so every drawn cell has to be reported.
        //
        // Before the fix the baseline was still 40x20, and `Buffer::diff` walks
        // `0..min(len)`, so it compared the first 800 cells of the new frame
        // against the first 800 of the old one -- a sheared comparison, since
        // the two disagree about what a row is. That reported some cells that
        // had not changed and missed others that had, and every cell past the
        // old area was compared against nothing at all.
        let reported: HashSet<(usize, usize)> =
            diff.iter().map(|(x, y, _)| (*x, *y)).collect();
        let missing: Vec<(usize, usize)> =
            lit.difference(&reported).copied().collect();

        assert!(
            missing.is_empty(),
            "{} of the {} drawn cells were not reported on the first frame \
             after the resize, e.g. {missing:?}",
            missing.len(),
            lit.len()
        );
    }

    #[test]
    fn update_size_rebuilds_the_ramp_for_the_new_height() {
        // The ramp's length is what keeps it longer than the longest drop, so it
        // has to follow the height the drops are sized from. `draw::ramp_at`
        // clamps as well -- belt and braces, and that is why dropping either
        // half alone is survivable -- but a ramp stuck at the construction height
        // would show a grown terminal the bottom of the old one.
        let mut rain = DigitalRain::new(get_sane_default_options(), (40, 20));
        let before = rain.ramp.len();
        rain.update_size(40, 100);

        assert!(
            rain.ramp.len() > before,
            "the ramp is still {before} entries after growing to 100 rows"
        );
    }

    #[test]
    fn update_size_alone_leaves_a_renderable_effect() {
        // The contract `maze` documents: `update_size` is a public entry point,
        // reached from the runtime's resize path with no `reset` after it.
        let mut rain = DigitalRain::new(get_sane_default_options(), (40, 20));
        rain.update_size(80, 40);

        let diff = rain.get_diff();

        // A fresh baseline cannot describe the old screen, so the frame after a
        // resize has to be a full repaint rather than a partial one.
        assert!(
            diff.iter().any(|(x, y, _)| *x < 40 && *y < 20),
            "nothing was repainted at all after a resize"
        );
    }

    #[test]
    fn the_draw_does_not_reorder_the_drops() {
        // The sort moved from `fill_buffer` to `update` so that the draw is a
        // pure function of the simulation. This pins the ordering the sort exists
        // to provide: it has to be in place before a frame is drawn, without a
        // draw having happened.
        //
        // Honest scope: this does not fail against the *old placement*, because
        // re-sorting an already-sorted list is a no-op and the sort draws no
        // randomness, so moving it was a purity and tidiness change with no
        // behavioural difference. It does fail if the ordering is dropped.
        let mut rain = DigitalRain::new(get_sane_default_options(), (60, 30));

        let speeds: Vec<u16> = rain.rain_drops.iter().map(|d| d.speed).collect();
        assert!(
            speeds.windows(2).all(|w| w[0] <= w[1]),
            "the constructor left the drops unordered: {speeds:?}"
        );

        for _ in 0..200 {
            rain.update();
        }
        let after_updates: Vec<u16> =
            rain.rain_drops.iter().map(|d| d.speed).collect();
        assert!(
            after_updates.windows(2).all(|w| w[0] <= w[1]),
            "a new drop was added out of order and nothing slotted it back in: \
             {after_updates:?}"
        );

        // And drawing does not change the order.
        rain.get_diff();
        let after_draw: Vec<u16> =
            rain.rain_drops.iter().map(|d| d.speed).collect();
        assert_eq!(after_updates, after_draw);
    }

    #[test]
    fn a_long_drop_on_a_grown_screen_does_not_reach_past_the_ramp() {
        // The panic the resize left reachable: after a grow, a new drop can be
        // far longer than a ramp built for the old height, and the ramp was
        // indexed with no bound at all.
        let mut rain = DigitalRain::new(get_sane_default_options(), (40, 20));
        rain.update_size(200, 200);

        for _ in 0..600 {
            rain.update();
            rain.get_diff();
        }

        let longest = rain
            .rain_drops
            .iter()
            .map(|d| d.body.len())
            .max()
            .unwrap_or(0);
        assert!(
            longest > 0,
            "the fixture produced no drops to check against the ramp"
        );
    }
}
