// use super::rain_options::DigitalRainOptions;
use crate::common::EffectRng;
use crate::rain::digital_rain::DigitalRainOptions;
use rand::{
    self, Rng, RngExt,
    distr::{Distribution, StandardUniform},
    seq::IndexedRandom,
};
use std::sync::LazyLock;
use std::time::Duration;

/// The character pool, in a fixed order.
///
/// This was a `HashMap<&str, &str>` flattened into a `Vec<char>`, and that made
/// `--seed` a lie. `HashMap` iterates in a per-process order, so a uniform index
/// drawn from the flattened vector landed on a different character in every
/// process: two runs with the same seed produced different rain. The determinism
/// contract could not see it, because both instances it compares live in one
/// process and share the same already-initialised `LazyLock`.
///
/// A map was never the right shape here. The labels were never looked up -- the
/// map existed to be iterated -- and four static sets do not need one.
///
/// Wide characters are deliberately absent. `:` and `・` and `日` were all tried
/// and removed: they occupy two columns in most terminals, which shears the grid.
/// The katakana are halfwidth (East_Asian_Width = Halfwidth), so they are one
/// column each and are safe.
const CHARACTER_SETS: &[&str] = &[
    "012345789",                        // digits
    r#":."=*+-<>"#,                     // punctuation
    "ﾊﾐﾋｰｳｼﾅﾓﾆｻﾜﾂｵﾘｱﾎﾃﾏｹﾒｴｶｷﾑﾕﾗｾﾈｽﾀﾇﾍ", // katakana
    "¦çﾘｸ",                             // other
];

/// Every character a drop can be made of, in [`CHARACTER_SETS`] order.
static CHARACTERS: LazyLock<Vec<char>> =
    LazyLock::new(|| CHARACTER_SETS.concat().chars().collect());

/// How a drop is coloured, which is also how it fades.
///
/// `Debug` is here so a failing assertion about a drop's colours can name the
/// style it was about rather than printing an opaque field number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RainDropStyle {
    Front,
    Middle,
    Back,
    Fading,
    Gradient,
}

impl RainDropStyle {
    /// Every variant, so a test can assert a property of all of them at once
    /// rather than of whichever one it remembered to list.
    pub const ALL: &'static [RainDropStyle] = &[
        RainDropStyle::Front,
        RainDropStyle::Middle,
        RainDropStyle::Back,
        RainDropStyle::Fading,
        RainDropStyle::Gradient,
    ];
}

pub struct RainDrop {
    pub _drop_id: usize,
    pub body: Vec<char>,
    pub style: RainDropStyle,
    pub fx: u16,
    pub fy: f32,
    pub max_length: usize,
    pub speed: u16,
}

impl Distribution<RainDropStyle> for StandardUniform {
    /// Choose from range
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> RainDropStyle {
        match rng.random_range(1..=100) {
            1..=10 => RainDropStyle::Front,
            11..=20 => RainDropStyle::Middle,
            21..=40 => RainDropStyle::Back,
            41..=50 => RainDropStyle::Fading,
            _ => RainDropStyle::Gradient,
        }
    }
}

/// Set of operations to make drain drop moving and growing
impl RainDrop {
    /// Create new rain drop with sane random defaults
    pub fn new(
        screen_size: (u16, u16),
        options: &DigitalRainOptions,
        drop_id: usize,
        rng: &mut EffectRng,
    ) -> Self {
        // Drawn from the generator we were handed, not from the ambient thread
        // RNG. Using `rand::random()` here meant a caller seeding `rng` to get a
        // reproducible drop still got a random style.
        let style: RainDropStyle = rng.random();
        let fx: u16 = rng.random_range(0..screen_size.0);
        let fy: f32 = rng.random_range(0..screen_size.1 / 4) as f32;
        // At most `2h/3`. The bound is load-bearing twice over: a drop that long
        // covers most of a terminal, and it is what keeps a drop inside the
        // colour ramp, which is built at `3h/2` entries (see
        // `DigitalRain::build_ramp`). `reset` below uses a tighter `h/2` bound.
        //
        // It stopped being a guarantee for a while, because a drop created after
        // a resize is sized from the *new* height while the ramp was still sized
        // from the one the effect was built at -- so growing the terminal could
        // index a drop straight past the end of the ramp. Both sides follow the
        // current height now.
        let max_length: usize =
            rng.random_range(4..=(2 * screen_size.1 / 3)) as usize;

        let speed: u16 =
            rng.random_range(options.get_min_speed()..=options.get_max_speed());

        let init_length = rng.random_range(1..max_length / 2);
        let mut body: Vec<char> = vec![*CHARACTERS.choose(rng).unwrap()];
        for _ in 1..init_length {
            body.push(*CHARACTERS.choose(rng).unwrap());
        }

        Self::from_values(drop_id, body, style, fx, fy, max_length, speed)
    }

    /// Create new worm from values
    #[inline(always)]
    pub fn from_values(
        _drop_id: usize,
        body: Vec<char>,
        style: RainDropStyle,
        fx: u16,
        fy: f32,
        max_length: usize,
        speed: u16,
    ) -> Self {
        Self {
            _drop_id,
            body,
            style,
            fx,
            fy,
            max_length,
            speed,
        }
    }

    /// Convert float into screen coordinates
    #[inline]
    pub fn to_point(&self) -> (u16, u16) {
        let x = self.fx;
        let y = self.fy.round() as u16;
        (x, y)
    }

    /// Receive vector of coordinates of RainDrop body
    pub fn to_points_vec(&self) -> Vec<(u16, u16, char)> {
        let mut points = vec![];
        let (head_x, head_y) = self.to_point();
        for (index, character) in self.body.iter().enumerate() {
            let yy = head_y as i16 - index as i16;
            if yy >= 0 {
                points.push((head_x, yy as u16, *character));
            } else {
                break;
            };
        }
        points
    }

    /// Reset worm to the sane defaults
    fn reset(
        &mut self,
        screen_size: (u16, u16),
        options: &DigitalRainOptions,
        rng: &mut EffectRng,
    ) {
        self.body.clear();
        self.body.insert(0, *CHARACTERS.choose(rng).unwrap());
        self.style = rng.random();
        self.fy = 0.0;
        self.fx = rng.random_range(0..screen_size.0);
        self.speed =
            rng.random_range(options.get_min_speed()..=options.get_max_speed());
        self.max_length =
            rng.random_range(screen_size.1 / 4 + 1..=(screen_size.1 / 2)) as usize;
    }

    /// Grow condition
    fn grow_condition(&self) -> bool {
        self.speed > 8
    }

    /// Grow up matrix worm characters array
    fn grow(&mut self, head_y: u16, rng: &mut EffectRng) {
        if self.body.len() >= self.max_length {
            self.body.truncate(self.max_length);
            return;
        };

        match self.grow_condition() {
            true => {
                // grow drop body to the number of cells passed during update
                let delta: i16 = head_y as i16 - self.fy.round() as i16;
                if delta > 0 {
                    for _ in 0..delta as usize {
                        self.body.insert(0, *CHARACTERS.choose(rng).unwrap());
                    }
                };
            }
            false => {
                // grow only to one character if position changed
                let delta: i16 = head_y as i16 - self.fy.round() as i16;
                if delta > 0 {
                    self.body.insert(0, *CHARACTERS.choose(rng).unwrap());
                };
            }
        };

        self.body.truncate(self.max_length);
    }

    /// Update rain drops to change position/grow etc
    /// there can be 4 cases:
    /// rain drop vector not yet fully come from top
    /// rain drop vector somewhere in the middle of the screen
    /// rain drop vector reach bottom and need to fade out
    /// raid drop vector tail out of screen rect visibility
    ///
    /// Note that rain drop coordinates can be outside bounds defined
    /// by screen width and height, this should be handled during draw process
    pub fn update(
        &mut self,
        screen_size: (u16, u16),
        options: &DigitalRainOptions,
        dt: Duration,
        rng: &mut EffectRng,
    ) {
        // NOTE: looks like guard, but why i even need it here?
        if self.body.is_empty() {
            self.reset(screen_size, options, rng);
            return;
        }

        // new fy coordinate
        let fy = self.fy + self.speed as f32 * dt.as_secs_f32();

        // calculate head and tail y coordinate
        let head_y = fy.round() as u16;
        let tail_y = fy.round() as i16 - self.body.len() as i16;
        let height = screen_size.1;

        if tail_y <= 0 {
            // not fully come out from top
            self.grow(head_y, rng);
            self.fy = fy;
            return;
        };

        if (head_y <= height) && (tail_y > 0) {
            // somewhere in the middle
            self.grow(head_y, rng);
            self.fy = fy;
            return;
        };

        if (head_y > height) && (tail_y < height as i16) {
            // got to the bottom
            self.fy = fy;
            return;
        };

        // NOTE: need this to reset
        if tail_y as u16 >= height {
            self.reset(screen_size, options, rng);
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{super::digital_rain::DigitalRainOptions, *};
    use crate::common::seeded_rng;

    /// The character pool has to be assembled in a declared order.
    ///
    /// `--seed` is only a promise if a uniform index drawn from this vector lands
    /// on the same character in every process. It used to be built by iterating a
    /// `HashMap`, whose order is per-process, so the same seed produced different
    /// rain on every run and nothing in the test suite could see it -- both
    /// instances the determinism contract compares share one `LazyLock`.
    ///
    /// Reintroducing a map fails here rather than silently unbreaking the seed.
    #[test]
    fn the_character_pool_is_flattened_in_the_declared_order() {
        let expected: Vec<char> = CHARACTER_SETS.concat().chars().collect();

        assert_eq!(
            *CHARACTERS, expected,
            "the pool is not the declared concatenation, so a seeded run is not \
             reproducible across processes"
        );
        assert!(
            CHARACTER_SETS.len() > 1,
            "the sets were merged into one, which loses the documentation of what \
             each group is for"
        );
    }

    /// Every character must be one column wide.
    ///
    /// A double-width glyph occupies two cells, and the drop body is indexed by
    /// cell, so one shears the grid rather than merely looking wrong. The comment
    /// on `CHARACTER_SETS` records `・` (U+30FB) and `日` (U+65E5) as having been
    /// removed for exactly that, so this is the test that keeps them out.
    ///
    /// What remains is ASCII, the halfwidth katakana block, and three Latin-1
    /// supplement characters. The last group is *ambiguous* width rather than
    /// wide: one column in a Latin-configured terminal, two in a CJK-configured
    /// one. That is a real but much smaller risk than the wide characters were,
    /// and the effect has shipped with them, so the assertion is that nothing
    /// genuinely wide has crept back in.
    #[test]
    fn no_character_in_the_pool_is_double_width() {
        for set in CHARACTER_SETS {
            for glyph in set.chars() {
                let ambiguous_or_narrow = glyph.is_ascii()
                    || ('\u{00A0}'..='\u{00FF}').contains(&glyph)
                    || ('\u{FF61}'..='\u{FF9F}').contains(&glyph);
                assert!(
                    ambiguous_or_narrow,
                    "{glyph:?} (U+{:04X}) in {set:?} is double-width, and will \
                     shear the grid",
                    glyph as u32
                );
            }
        }
    }

    fn get_sane_options() -> DigitalRainOptions {
        DigitalRainOptions {
            drops_range: (20, 30),
            speed_range: (10, 20),
            ..Default::default()
        }
    }

    #[test]
    fn create_new_and_reset() {
        let mut rng = seeded_rng(1, "rain_drop_test");
        let mut new_drop =
            RainDrop::new((100, 100), &get_sane_options(), 1, &mut rng);
        assert!(!new_drop.body.is_empty());
        assert!(new_drop.speed > 0);

        new_drop.reset((100, 100), &get_sane_options(), &mut rng);
        assert_eq!(new_drop.fy, 0.0);
        assert_eq!(new_drop._drop_id, 1);
        assert_eq!(new_drop.body.len(), 1);
    }

    #[test]
    fn generate_a_lot_of_drops() {
        let mut rng = seeded_rng(1, "rain_drop_test");
        let mut drops = vec![];
        for index in 1..=1000 {
            drops.push(RainDrop::new(
                (100, 100),
                &get_sane_options(),
                index,
                &mut rng,
            ));
        }
        assert_eq!(drops.len(), 1000);
    }

    #[test]
    fn to_point() {
        let new_drop = RainDrop::from_values(
            1,
            vec!['a'],
            RainDropStyle::Gradient,
            10,
            10.8,
            20,
            10,
        );
        let (x, y) = new_drop.to_point();
        assert_eq!(x, 10);
        assert_eq!(y, 11);
    }

    #[test]
    fn to_point_vec() {
        let new_drop = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c'],
            RainDropStyle::Fading,
            10,
            10.0,
            10,
            8,
        );
        let points = new_drop.to_points_vec();
        assert_eq!(points.len(), 3);
        assert_eq!(points[0], (10, 10, 'a'));
    }

    #[test]
    fn grow() {
        let mut rng = seeded_rng(1, "rain_drop_test");
        let mut new_drop = RainDrop::from_values(
            1,
            vec!['a'],
            RainDropStyle::Front,
            10,
            10.8,
            20,
            10,
        );
        new_drop.grow(10, &mut rng);
        assert_eq!(new_drop.body.len(), 1);
        assert_eq!(new_drop.body.first(), Some(&'a'));

        let mut new_drop = RainDrop::from_values(
            1,
            vec!['b'],
            RainDropStyle::Middle,
            10,
            10.8,
            20,
            4,
        );
        new_drop.grow(12, &mut rng);
        assert_eq!(new_drop.body.len(), 2);
        assert_eq!(new_drop.body.get(1), Some(&'b'));
        new_drop.grow(11, &mut rng);
        assert_eq!(new_drop.body.len(), 2);

        let mut new_drop = RainDrop::from_values(
            1,
            vec!['c'],
            RainDropStyle::Back,
            10,
            10.8,
            3,
            4,
        );
        for _ in 1..10 {
            new_drop.grow(12, &mut rng);
        }
        assert_eq!(new_drop.body.len(), 3);
    }

    #[test]
    fn update() {
        let mut rng = seeded_rng(1, "rain_drop_test");

        // nothing special worm update
        let mut new_drop = RainDrop::from_values(
            1,
            vec!['c'],
            RainDropStyle::Back,
            10,
            10.8,
            3,
            10,
        );
        new_drop.update(
            (100, 100),
            &get_sane_options(),
            Duration::from_millis(1000),
            &mut rng,
        );
        assert_eq!(new_drop.fy.round() as u16, 21);
        assert_eq!(new_drop.body.len(), 3);

        // edge case when body len is 0 (why?)
        let mut new_drop =
            RainDrop::from_values(1, vec![], RainDropStyle::Middle, 10, 10.8, 3, 8);
        new_drop.update(
            (100, 100),
            &get_sane_options(),
            Duration::from_millis(1000),
            &mut rng,
        );
        assert_eq!(new_drop.body.len(), 1);
        assert_eq!(new_drop.fy, 0.0); // should be out of the h bounds and reset

        // when tail_y < 0
        let mut new_drop = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c', 'd'],
            RainDropStyle::Fading,
            10,
            2.0,
            5,
            2,
        );
        new_drop.update(
            (100, 100),
            &get_sane_options(),
            Duration::from_millis(1000),
            &mut rng,
        );
        assert_eq!(new_drop.body.len(), 5);
        assert!((new_drop.fy - new_drop.body.len() as f32) < 0.0);

        // when head_y > screen height
        let mut new_drop = RainDrop::from_values(
            1,
            vec!['a', 'b', 'c', 'd'],
            RainDropStyle::Fading,
            10,
            30.8,
            5,
            2,
        );
        new_drop.update(
            (100, 100),
            &get_sane_options(),
            Duration::from_millis(1000),
            &mut rng,
        );
        assert_eq!(new_drop.body.len(), 5);
        assert!(new_drop.fy > 30.0);

        // when head_y > screen height and body len is 2
        let mut new_drop = RainDrop::from_values(
            1,
            vec!['a', 'b'],
            RainDropStyle::Fading,
            10,
            29.0,
            5,
            2,
        );
        new_drop.update(
            (100, 100),
            &get_sane_options(),
            Duration::from_millis(1000),
            &mut rng,
        );
        assert_eq!(new_drop.body.len(), 3);
        assert_eq!(new_drop.fy, 31.0);
        new_drop.update(
            (100, 100),
            &get_sane_options(),
            Duration::from_millis(1000),
            &mut rng,
        );
        assert_eq!(new_drop.fy, 33.0); // should be reset there
    }

    #[test]
    fn out_of_bounds() {
        let mut rng = seeded_rng(1, "rain_drop_test");
        let mut drops = vec![];
        for i in 1..=10 {
            drops.push(RainDrop::new((100, 100), &get_sane_options(), i, &mut rng));
        }
        assert_eq!(drops.len(), 10);

        for _ in 1..=1000 {
            for drop in drops.iter_mut() {
                drop.update(
                    (100, 100),
                    &get_sane_options(),
                    Duration::from_millis(100),
                    &mut rng,
                )
            }
        }
    }
}
