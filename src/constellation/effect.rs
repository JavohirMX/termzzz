use crate::buffer::{Buffer, Cell};
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, EffectRng, TerminalEffect, seeded_rng};
use crate::render::palette::Palette;
use crossterm::style;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::LazyLock;

/// Sparse to dense, and *ordered*: a star's glyph is its brightness, so `✦`
/// really is the brightest thing on the screen. It used to be drawn from
/// `rng.random_range(0..4)` at construction and never changed, so a `✦` was no
/// brighter than a `○` and the glyph carried no information at all.
const STAR_GLYPHS: [char; 4] = ['○', '◦', '*', '✦'];

/// The colour a star starts from, per hue: dark enough that the dimmest star on
/// screen is a suggestion rather than a point of light.
const DIM: [(u8, u8, u8); 4] =
    [(33, 43, 78), (48, 30, 68), (26, 58, 68), (53, 38, 78)];

/// Where a star's hue tops out before it starts going white.
const BRIGHT: [(u8, u8, u8); 4] = [
    (110, 150, 240),
    (170, 110, 230),
    (90, 210, 230),
    (190, 150, 255),
];

/// The white a star fades to at the very top of its twinkle, so the brightest
/// moments in the sky are not all the same colour.
const WHITE: (u8, u8, u8) = (238, 243, 255);

/// One ramp per hue, running dim to bright to white.
///
/// Through [`Palette`] rather than a hand-rolled lerp, which is what the
/// previous `lerp_color`/`as_rgb` pair was: twenty duplicated lines and an
/// `as_rgb` that answered white for any colour that was not `Rgb`, so a named
/// colour would have quietly come out as a white star.
static HUES: LazyLock<[Palette; 4]> = LazyLock::new(|| {
    std::array::from_fn(|hue| {
        Palette::new(vec![
            style::Color::Rgb {
                r: DIM[hue].0,
                g: DIM[hue].1,
                b: DIM[hue].2,
            },
            style::Color::Rgb {
                r: BRIGHT[hue].0,
                g: BRIGHT[hue].1,
                b: BRIGHT[hue].2,
            },
            style::Color::Rgb {
                r: WHITE.0,
                g: WHITE.1,
                b: WHITE.2,
            },
        ])
    })
});

/// Cells per star, so the density of the sky is the same at every terminal size.
const CELLS_PER_STAR: f64 = 900.0;
/// The floor, for the sizes where the density on its own would ask for fewer
/// stars than a sky needs. Bounded by the area in practice -- see
/// [`Constellation::star_count`].
const MIN_STARS: usize = 25;
/// The ceiling, so a very large terminal does not turn back into a mesh.
const MAX_STARS: usize = 220;

/// How often the connection graph is rebuilt, in seconds.
///
/// The graph used to be recomputed from scratch every frame with no hysteresis,
/// which is why it churned: a line appeared the instant two stars came inside
/// the connect distance and vanished the instant they left, and because the
/// budget was shared between both endpoints, one line appearing could cascade
/// through the greedy pass and take others with it. Measured over 120 frames at
/// 80x24, with the stars moving 0.025 of a cell per frame, the drawn
/// connections changed 12.7 cells per frame. The interval is the larger half of
/// the fix; without it there is no frame at which the graph is not a fresh
/// answer to a question the stars have not finished asking.
const EDGE_REBUILD_SECONDS: f64 = 0.5;

/// How far past the connect distance a line survives before it is dropped.
///
/// Without it, a rebuild still flickers: two stars either side of the threshold
/// swap the line on and off. A line now outlives the distance that made it by
/// 60%, which at 80x24 and the default drift is about a third of a second of
/// extra life and at 400x200 closer to two -- in both cases enough that a
/// rebuild does not immediately undo the one before it.
const EDGE_HYSTERESIS: f64 = 1.6;

/// Placement attempts per star before one is accepted wherever it landed.
///
/// Bounded, because a crowded screen still has to get its stars: the
/// alternative to accepting a close placement is a smaller sky, and a smaller
/// sky is the bug this is here to fix.
const PLACEMENT_TRIES: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConstellationOptions {
    /// How many stars to place, or zero for "as many as the area calls for",
    /// which is the default.
    ///
    /// The count used to be a flat 65 at every size. That is a degenerate sky
    /// in both directions: at 6x6, the smallest window the runtime will ask
    /// for, 65 stars competed for 36 cells and 29 of them were silently
    /// overwritten by later draws, while at 400x200 the same 65 stars left a
    /// screen that looked half empty.
    ///
    /// A non-zero value is an exact count, for a user who wants one. Note the
    /// trap in `--print-config`: it writes every default to disk, so a config
    /// generated before this was the default carries the old 65 and pins it.
    pub star_count: usize,
    /// The distance within which two stars are drawn connected, as a fraction
    /// of the screen diagonal.
    ///
    /// Was 0.18, which at 80x24 is 15 cells. With 65 stars in 1920 cells that
    /// is 24 expected neighbours per star against a budget of four, so the
    /// greedy pass took about 130 edges and a measured 19% of the screen came
    /// out covered in dots: a mesh, not a constellation. At 0.06 the expected
    /// count is about one neighbour per star, which is what the budget below can
    /// actually draw, and the same screen comes out at 1.5%.
    pub connect_radius: f64,
    /// The most lines one star may be an endpoint of.
    ///
    /// Was 4. One is enough for the effect to read as a set of named figures
    /// rather than as a graph, and it is what makes the greedy pass below
    /// stable: with a budget of one, a star's nearest neighbour is the only
    /// candidate that can ever take it, so the graph is close to a function of
    /// geometry alone.
    pub max_connections: usize,
    pub twinkle: bool,
    /// Cells per second. Was 0.3 to 1.5, which put a star's crossing of an
    /// 80-column screen at 53 to 267 seconds -- the name says drifting, and at
    /// that rate the only thing visibly changing is the twinkle.
    pub min_speed: f64,
    pub max_speed: f64,
    /// Seed for the star field. Fixes positions, speeds, magnitudes, hues and
    /// twinkle phases, so the whole sky is reproducible.
    pub seed: u64,
}

impl Default for ConstellationOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            star_count: 0,
            connect_radius: 0.06,
            max_connections: 1,
            twinkle: true,
            min_speed: 3.0,
            max_speed: 15.0,
            seed: DEFAULT_SEED,
        }
    }
}

#[derive(Clone, Debug)]
struct Star {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    twinkle: f64,
    twinkle_freq: f64,
    /// How bright this star gets, fixed for its lifetime.
    ///
    /// Without it the twinkle took every star to full brightness at its peak
    /// and the sky had no hierarchy: the field of magnitudes that makes a
    /// constellation look like a constellation was only in the colours, which
    /// a monochrome terminal throws away.
    magnitude: f64,
    /// Which of [`HUES`] this star is drawn from.
    hue: usize,
}

/// A connection between two stars, ordered so a pair compares equal whichever
/// end found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    low: usize,
    high: usize,
}

impl Edge {
    fn new(a: usize, b: usize) -> Self {
        if a <= b {
            Self { low: a, high: b }
        } else {
            Self { low: b, high: a }
        }
    }
}

pub struct Constellation {
    screen_size: (u16, u16),
    options: ConstellationOptions,
    canvas: Canvas,
    stars: Vec<Star>,
    connect_dist: f64,
    /// The current graph, kept between frames rather than rebuilt per frame.
    edges: Vec<Edge>,
    /// Seconds until the graph is next reconsidered.
    edge_timer: f64,
}

impl TerminalEffect for Constellation {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // Destructured so the canvas can be borrowed while the rest of the
        // effect is read; a `&self` draw method could not also hold
        // `&mut self.canvas`.
        let Self {
            stars,
            edges,
            connect_dist,
            options,
            screen_size,
            canvas,
            ..
        } = self;

        canvas.clear();
        Self::draw_connections(
            stars,
            edges,
            *connect_dist,
            *screen_size,
            canvas.surface_mut(),
        );
        Self::draw_stars(stars, options, *screen_size, canvas.surface_mut());
        canvas.commit()
    }

    fn update(&mut self) {
        // Nominal frame time, so an effect driven through the plain `update`
        // path still moves at the rate it was tuned for.
        self.step(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Capped so a stall does not teleport every star across the screen.
        self.step(context.delta.as_secs_f64().min(0.1));
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.reset();
    }

    fn reset(&mut self) {
        self.canvas
            .resize(self.screen_size.0.max(1), self.screen_size.1.max(1));
        self.connect_dist = Self::calc_connect_dist(
            self.screen_size.0,
            self.screen_size.1,
            self.options.connect_radius,
        );

        let spacing = Self::min_spacing(self.screen_size.0, self.screen_size.1);
        let count = self.star_count();
        self.stars.clear();
        self.stars.reserve(count);
        let mut rng = seeded_rng(self.options.seed, "constellation");
        for _ in 0..count {
            self.stars.push(Self::random_star(
                &self.screen_size,
                spacing,
                &self.stars,
                &self.options,
                &mut rng,
            ));
        }

        // After the stars, not before: the graph is a function of where they
        // are, so the first frame is a constellation rather than an empty sky
        // that fills in a second later.
        self.edges.clear();
        self.rebuild_edges();
    }
}

impl Constellation {
    pub fn new(options: ConstellationOptions, screen_size: (u16, u16)) -> Self {
        let mut effect = Self {
            screen_size,
            options,
            canvas: Canvas::new(screen_size.0, screen_size.1),
            stars: Vec::new(),
            connect_dist: 0.0,
            edges: Vec::new(),
            edge_timer: 0.0,
        };

        effect.reset();
        effect
    }

    fn step(&mut self, dt: f64) {
        let width = self.screen_size.0 as f64;
        let height = self.screen_size.1 as f64;

        for star in &mut self.stars {
            star.x += star.vx * dt;
            star.y += star.vy * dt;

            if self.options.twinkle {
                star.twinkle += star.twinkle_freq * dt;
            }

            if star.x < 0.0 {
                star.x = -star.x;
                star.vx = -star.vx;
            } else if star.x >= width {
                star.x = 2.0 * width - star.x - 0.01;
                star.vx = -star.vx;
            }

            if star.y < 0.0 {
                star.y = -star.y;
                star.vy = -star.vy;
            } else if star.y >= height {
                star.y = 2.0 * height - star.y - 0.01;
                star.vy = -star.vy;
            }
        }

        self.edge_timer -= dt;
        if self.edge_timer <= 0.0 {
            self.rebuild_edges();
        }
    }

    /// The current graph, reconsidered.
    ///
    /// A line that already exists survives while it is within
    /// [`EDGE_HYSTERESIS`] times the connect distance, and then every candidate
    /// pair inside the connect distance is considered nearest-first, taking the
    /// line only if both ends have budget left.
    ///
    /// The previous version walked the stars in index order and gave each one
    /// its own nearest neighbours out of a *shared* budget, so whichever star
    /// had the lower index claimed its four first and the graph carried a
    /// systematic bias toward the front of the array that had nothing to do with
    /// geometry. Sorting the candidate pairs by distance instead lets distance
    /// decide, and the ordering within a tie is the only thing index order
    /// still affects.
    fn rebuild_edges(&mut self) {
        let budget = self.options.max_connections;
        let count = self.stars.len();
        let mut degree = vec![0usize; count];
        let mut edges: BTreeSet<Edge> = BTreeSet::new();

        let claim =
            |edges: &mut BTreeSet<Edge>, degree: &mut Vec<usize>, edge: Edge| {
                if degree[edge.low] < budget && degree[edge.high] < budget {
                    degree[edge.low] += 1;
                    degree[edge.high] += 1;
                    edges.insert(edge);
                }
            };

        for edge in self.edges.iter().copied() {
            // Claimed before any new candidate, and in the order already held
            // rather than nearest-first. A rebuild that dropped a standing line
            // because a nearer pair happened to turn up would be exactly the
            // churn the interval and the hysteresis are here to stop.
            if self.edge_length(edge) <= self.connect_dist * EDGE_HYSTERESIS {
                claim(&mut edges, &mut degree, edge);
            }
        }

        let mut candidates: Vec<(f64, Edge)> = Vec::new();
        for low in 0..count {
            for high in (low + 1)..count {
                let distance = self.star_distance(low, high);
                if distance <= self.connect_dist {
                    candidates.push((distance, Edge::new(low, high)));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, edge) in candidates {
            if !edges.contains(&edge) {
                claim(&mut edges, &mut degree, edge);
            }
        }

        self.edges = edges.into_iter().collect();
        self.edge_timer = EDGE_REBUILD_SECONDS;
    }

    /// How many stars this terminal gets.
    fn star_count(&self) -> usize {
        if self.options.star_count > 0 {
            return self.options.star_count;
        }
        let area = f64::from(self.screen_size.0) * f64::from(self.screen_size.1);
        let by_density = (area / CELLS_PER_STAR).round() as usize;
        // The floor is bounded by the area too, because a floor of 25 is 25
        // stars in 36 cells at 6x6 -- which is the block of stars this
        // replaced.
        by_density.clamp(MIN_STARS.min((area / 8.0) as usize), MAX_STARS)
    }

    fn calc_connect_dist(width: u16, height: u16, radius_factor: f64) -> f64 {
        ((width as f64).powi(2) + (height as f64).powi(2)).sqrt() * radius_factor
    }

    /// The closest two stars may be placed, in cells.
    ///
    /// A constant number of cells would be a different fraction of a 6x6 window
    /// than of a 200x50 one, and stars placed in the same cell are simply lost:
    /// the later `buffer.set` overwrites the earlier one. Capped, because past a
    /// couple of cells the exclusion starts pushing the sky into a lattice
    /// rather than merely keeping it apart.
    fn min_spacing(width: u16, height: u16) -> f64 {
        (f64::from(width.min(height)) * 0.12).clamp(0.0, 2.0)
    }

    fn star_distance(&self, a: usize, b: usize) -> f64 {
        Self::star_distance_between(&self.stars[a], &self.stars[b])
    }

    fn edge_length(&self, edge: Edge) -> f64 {
        self.star_distance(edge.low, edge.high)
    }

    fn random_star(
        screen_size: &(u16, u16),
        spacing: f64,
        placed: &[Star],
        options: &ConstellationOptions,
        rng: &mut EffectRng,
    ) -> Star {
        let speed = rng.random_range(options.min_speed..options.max_speed);
        let angle = rng.random_range(0.0..(std::f64::consts::PI * 2.0));

        // Rejection sampling against the stars already down, rather than a
        // uniform draw. The crab does this too, and for the same reason: two
        // stars in one cell means one of them is never seen, and the drawing
        // order decides which, so the loss is invisible rather than reported.
        //
        // There was a second placement here that spawned stars on a border,
        // behind a `scattered` parameter that every caller passed as `true`, so
        // it was dead code reading as a feature. Deleted rather than wired up:
        // a star on the edge of the screen is a star half off it.
        let mut x = 0.0;
        let mut y = 0.0;
        for _ in 0..PLACEMENT_TRIES {
            x = rng.random_range(0.0..f64::from(screen_size.0));
            y = rng.random_range(0.0..f64::from(screen_size.1));
            let clear = placed.iter().all(|other| {
                let dx = other.x - x;
                let dy = other.y - y;
                (dx * dx + dy * dy).sqrt() >= spacing
            });
            if clear {
                break;
            }
        }

        Star {
            x,
            y,
            vx: angle.cos() * speed,
            vy: angle.sin() * speed,
            twinkle: rng.random_range(0.0..(std::f64::consts::PI * 2.0)),
            twinkle_freq: rng.random_range(0.4..1.2),
            magnitude: rng.random_range(0.25..1.0),
            hue: rng.random_range(0..HUES.len()),
        }
    }

    /// How bright a star is right now, in `0.0..=1.0`.
    fn brightness(star: &Star, twinkle: bool) -> f64 {
        if twinkle {
            star.magnitude * (0.5 + 0.5 * star.twinkle.sin())
        } else {
            star.magnitude
        }
    }

    fn draw_connections(
        stars: &[Star],
        edges: &[Edge],
        connect_dist: f64,
        size: (u16, u16),
        buffer: &mut Buffer,
    ) {
        for edge in edges {
            let (a, b) = (&stars[edge.low], &stars[edge.high]);
            let closeness = (1.0
                - Self::star_distance_between(a, b) / connect_dist)
                .clamp(0.0, 1.0);
            // Half a star's brightness, so a line reads as a link between two
            // points of light rather than as a row of points of its own.
            let color = HUES[a.hue].sample((0.5 * closeness) as f32);

            Self::draw_dotted_line(
                size,
                buffer,
                a.x.round() as i32,
                a.y.round() as i32,
                b.x.round() as i32,
                b.y.round() as i32,
                color,
            );
        }
    }

    fn star_distance_between(a: &Star, b: &Star) -> f64 {
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        (dx * dx + dy * dy).sqrt()
    }

    /// The glyph for a brightness: the ramp, indexed by how bright the star is.
    fn glyph_for(brightness: f64) -> char {
        let rank = ((brightness * (STAR_GLYPHS.len() - 1) as f64).round() as usize)
            .min(STAR_GLYPHS.len() - 1);
        STAR_GLYPHS[rank]
    }

    fn draw_stars(
        stars: &[Star],
        options: &ConstellationOptions,
        size: (u16, u16),
        buffer: &mut Buffer,
    ) {
        for star in stars {
            let brightness = Self::brightness(star, options.twinkle);
            let glyph = Self::glyph_for(brightness);
            let color = HUES[star.hue].sample(brightness as f32);

            let x = star.x.round() as i32;
            let y = star.y.round() as i32;
            if Self::in_bounds(size, x, y) {
                // Not bold. The glyph now encodes the brightness, and bold is a
                // brightening hint on many terminals, so asking for it as well
                // would corrupt the ordering the glyph is there to carry -- and
                // it widens the glyph, which shears a grid indexed by cell.
                buffer.set(
                    x as usize,
                    y as usize,
                    Cell::new(glyph, color, style::Attribute::NormalIntensity),
                );
            }
        }
    }

    fn draw_dotted_line(
        size: (u16, u16),
        buffer: &mut Buffer,
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        color: style::Color,
    ) {
        let dx = x1 - x0;
        let dy = y1 - y0;

        let steps = dx.abs().max(dy.abs());
        if steps < 2 {
            return;
        }

        for i in 1..steps {
            let t = i as f64 / steps as f64;
            let x = x0 + (dx as f64 * t + 0.5) as i32;
            let y = y0 + (dy as f64 * t + 0.5) as i32;

            if Self::in_bounds(size, x, y) {
                buffer.set(
                    x as usize,
                    y as usize,
                    Cell::new('·', color, style::Attribute::NormalIntensity),
                );
            }
        }
    }

    fn in_bounds(size: (u16, u16), x: i32, y: i32) -> bool {
        x >= 0 && x < size.0 as i32 && y >= 0 && y < size.1 as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole visible field, accumulated from the diff.
    ///
    /// `get_diff` reports only what changed, and this effect changes every
    /// drawn cell's colour every frame as the stars twinkle, so a single diff
    /// is a list of changes rather than the picture. Folding the diffs together
    /// reconstructs what the terminal is showing, which is the only thing the
    /// tests below can meaningfully measure.
    struct Field {
        cells: std::collections::BTreeMap<(usize, usize), char>,
    }

    impl Field {
        fn new() -> Self {
            Self {
                cells: Default::default(),
            }
        }

        fn absorb(&mut self, effect: &mut Constellation) {
            for (x, y, cell) in effect.get_diff() {
                self.cells.insert((x, y), cell.symbol);
            }
        }

        fn positions(
            &self,
            symbol: char,
        ) -> std::collections::BTreeSet<(usize, usize)> {
            self.cells
                .iter()
                .filter(|(_, drawn)| **drawn == symbol)
                .map(|(at, _)| *at)
                .collect()
        }
    }

    #[test]
    fn connections_cover_a_small_fraction_of_the_screen() {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            let mut effect =
                Constellation::new(ConstellationOptions::default(), size);
            let mut field = Field::new();
            field.absorb(&mut effect);
            let dots = field.positions('·').len();
            let area = size.0 as usize * size.1 as usize;

            assert!(
                dots <= area / 10,
                "at {size:?} the connections cover {dots} of {area} cells, \
                 {:.1}% of the screen; a constellation is a few lines and a \
                 mesh is not a constellation",
                100.0 * dots as f64 / area as f64
            );
        }
    }

    #[test]
    fn the_star_count_follows_the_terminal() {
        let small = Constellation::new(ConstellationOptions::default(), (80, 24));
        let large = Constellation::new(ConstellationOptions::default(), (400, 200));
        assert!(
            large.stars.len() > small.stars.len(),
            "a 400x200 terminal got {} stars and an 80x24 got {}; the count is \
             flat, so one of them is either empty or over-drawn",
            large.stars.len(),
            small.stars.len()
        );
        assert!(
            small.stars.len() >= 12,
            "an 80x24 terminal only got {} stars",
            small.stars.len()
        );
    }

    #[test]
    fn a_tiny_terminal_shows_stars_without_becoming_a_block() {
        // 6x6 is `MIN_EFFECT_SIZE`, the smallest the runtime will ask for.
        let size = (6u16, 6u16);
        let mut effect = Constellation::new(ConstellationOptions::default(), size);
        let mut field = Field::new();
        field.absorb(&mut effect);

        let lit = field.cells.len();
        let area = size.0 as usize * size.1 as usize;
        assert!(
            lit >= 3,
            "a {size:?} terminal drew {lit} cells, so the sky is empty"
        );
        assert!(
            lit * 2 <= area,
            "a {size:?} terminal drew stars in {lit} of its {area} cells: {} \
             stars were placed and {} of them landed on top of another, so the \
             picture is a block of stars rather than a sky",
            effect.stars.len(),
            effect.stars.len() - lit
        );
    }

    #[test]
    fn the_connection_graph_is_still_between_adjacent_frames() {
        // The speeds are pinned to the *old* ones on purpose. At the current
        // drift a line's endpoints sweep several cells a second, so the drawn
        // dots move even when the graph has not changed at all, and a test on
        // the drawn field would be measuring the drift rather than the graph.
        // Slowing the stars to a tenth of that isolates it.
        let options = ConstellationOptions {
            min_speed: 0.3,
            max_speed: 1.5,
            ..Default::default()
        };
        let mut effect = Constellation::new(options, (80, 24));
        let mut field = Field::new();
        field.absorb(&mut effect);
        let mut previous = field.positions('·');
        let mut changes = 0usize;

        for _ in 0..120 {
            effect.update();
            field.absorb(&mut effect);
            let current = field.positions('·');
            changes += previous.symmetric_difference(&current).count();
            previous = current;
        }

        // The graph used to be recomputed from scratch every frame against a
        // budget shared between both endpoints, so one line appearing could
        // cascade through the greedy pass and take others with it. Measured on
        // this test, that churned 12.7 cells per frame with the stars moving
        // 0.025 of a cell per frame, and no two adjacent frames out of 120 were
        // identical. Persistent edges with hysteresis churn 0.28.
        let per_frame = changes as f64 / 120.0;
        assert!(
            per_frame <= 1.5,
            "the connections changed {changes} cells over 120 frames, \
             {per_frame:.2} per frame, with the stars barely moving"
        );
    }

    #[test]
    fn the_graph_is_not_rebuilt_on_every_frame() {
        // The other half of the same contract, and the precise version of it:
        // between rebuilds the graph is the same set of edges, not merely a
        // similar-looking picture. Without the interval and the hysteresis a
        // rebuild is invisible -- the count and the shapes look the same -- so
        // this is the assertion that actually pins the mechanism.
        let mut effect =
            Constellation::new(ConstellationOptions::default(), (80, 24));
        let mut rebuilds = 0usize;
        let mut previous = effect.edges.clone();

        for _ in 0..120 {
            effect.update();
            if effect.edges != previous {
                rebuilds += 1;
                previous = effect.edges.clone();
            }
        }

        // Two seconds at a half-second interval, give or take a frame.
        assert!(
            rebuilds <= 6,
            "the graph changed {rebuilds} times in 120 frames of two seconds, \
             so it is being recomputed rather than kept"
        );
    }

    #[test]
    fn the_glyph_carries_the_brightness() {
        let mut effect =
            Constellation::new(ConstellationOptions::default(), (80, 24));
        let samples: Vec<(usize, f32)> = effect
            .get_diff()
            .into_iter()
            .filter(|(_, _, cell)| cell.symbol != '·')
            .map(|(_, _, cell)| {
                let (r, g, b) = match cell.color {
                    style::Color::Rgb { r, g, b } => (r, g, b),
                    _ => (0, 0, 0),
                };
                let rank = STAR_GLYPHS
                    .iter()
                    .position(|glyph| *glyph == cell.symbol)
                    .unwrap_or(usize::MAX);
                (
                    rank,
                    (0.299 * f32::from(r)
                        + 0.587 * f32::from(g)
                        + 0.114 * f32::from(b))
                        / 255.0,
                )
            })
            .collect();
        assert!(
            samples.len() >= 8,
            "only {} stars were drawn",
            samples.len()
        );

        let count = samples.len() as f32;
        let mean_rank = samples.iter().map(|(r, _)| *r as f32).sum::<f32>() / count;
        let mean_luma = samples.iter().map(|(_, l)| *l).sum::<f32>() / count;
        let covariance = samples
            .iter()
            .map(|(r, l)| (*r as f32 - mean_rank) * (*l - mean_luma))
            .sum::<f32>();
        let spread_rank = samples
            .iter()
            .map(|(r, _)| (*r as f32 - mean_rank).powi(2))
            .sum::<f32>()
            .sqrt();
        let spread_luma = samples
            .iter()
            .map(|(_, l)| (*l - mean_luma).powi(2))
            .sum::<f32>()
            .sqrt();
        let correlation = covariance / (spread_rank * spread_luma);

        // The glyph used to be drawn from an unseeded-looking `rng.random_range`
        // at construction and never changed, so it carried nothing at all: the
        // measured correlation with the colour it was painted was -0.19, which
        // is what a random pairing looks like. A `✦` is no brighter than a `○`.
        assert!(
            correlation > 0.7,
            "the glyph and the brightness it is painted in correlate at \
             {correlation:.2}, so the glyph is not carrying the value"
        );
    }

    #[test]
    fn the_drift_is_fast_enough_to_see() {
        let options = ConstellationOptions::default();
        // The slowest star has to visibly move, or the effect is animated only
        // in its twinkle. It used to drift at 0.3 cells/s, which is a fifth of
        // a cell every second: a star takes 267 seconds to cross 80 columns, so
        // the only thing on screen that changed was the brightness.
        assert!(
            options.min_speed >= 1.0,
            "min_speed is {} cells/s, so the slowest star in the sky moves a \
             fifth of a cell a second",
            options.min_speed
        );
        // And the fastest has to stay a drift rather than a streak.
        let crossing = 200.0 / options.max_speed;
        assert!(
            (2.0..=40.0).contains(&crossing),
            "max_speed is {} cells/s, so a star takes {crossing:.0}s to cross \
             a 200-column terminal",
            options.max_speed
        );
    }

    #[test]
    fn no_star_is_placed_on_the_screen_edge() {
        // There used to be a second placement branch that spawned stars on a
        // border, reachable only through a parameter nothing ever passed, so it
        // was dead code that read as a feature. It is gone; this is what is
        // left, and a star on an edge is how you would know it had come back.
        for size in [(80u16, 24u16), (40, 12)] {
            let effect = Constellation::new(ConstellationOptions::default(), size);
            for star in &effect.stars {
                assert!(
                    star.x > 0.0
                        && star.x < f64::from(size.0)
                        && star.y > 0.0
                        && star.y < f64::from(size.1),
                    "a star was placed at ({:.2}, {:.2}) on a {size:?} screen",
                    star.x,
                    star.y
                );
            }
        }
    }

    #[test]
    fn stars_start_apart_from_each_other() {
        let effect = Constellation::new(ConstellationOptions::default(), (80, 24));
        let spacing = Constellation::min_spacing(80, 24);
        let mut closest = f64::MAX;
        for (index, star) in effect.stars.iter().enumerate() {
            for other in &effect.stars[index + 1..] {
                let dx = star.x - other.x;
                let dy = star.y - other.y;
                closest = closest.min((dx * dx + dy * dy).sqrt());
            }
        }
        assert!(
            closest >= spacing * 0.5,
            "two stars start {:.2} cells apart at 80x24, where the placement \
             asks for {spacing:.2}",
            closest
        );
    }

    #[test]
    fn the_seed_still_changes_the_sky() {
        // `tests/effect_contracts.rs` iterates every effect for this, and the
        // constellation is not on its seed-insensitive list, so losing the seed
        // here fails there. This is the local half of that.
        let sky = |seed: u64| {
            let options = ConstellationOptions {
                seed,
                ..Default::default()
            };
            let effect = Constellation::new(options, (80, 24));
            (effect.stars[0].x, effect.stars[0].y)
        };
        let first = sky(42);
        let second = sky(42);
        let other = sky(43);
        assert_eq!(first, second, "the same seed moved a star");
        assert!(
            (first.0 - other.0).abs() > 1e-9 || (first.1 - other.1).abs() > 1e-9,
            "a different seed produced the same first star"
        );
    }

    #[test]
    fn creates_constellation_effect() {
        let options = ConstellationOptions::default();
        let effect = Constellation::new(options, (80, 24));
        assert_eq!(effect.screen_size, (80, 24));
    }
}
