use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, TerminalEffect};
use crate::render::braille::{BrailleGrid, DOTS_X, DOTS_Y};
use crate::render::dither::Dither;
use crate::render::glyph_ramp::{self, GlyphRamp};
use crate::render::palette::Palette;
use crossterm::style::{Attribute, Color};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Represents a 3D point in space
#[derive(Clone, Copy, Debug)]
struct Point3D {
    x: f32,
    y: f32,
    z: f32,
}

/// Represents a 2D point for screen coordinates
#[derive(Clone, Copy, Debug)]
struct Point2D {
    x: f32,
    y: f32,
}

/// Represents a 3D edge connecting two vertices
struct Edge {
    v1: usize,
    v2: usize,
    /// The two faces this edge is shared by.
    ///
    /// An edge is the *intersection* of two face planes, so these are the two
    /// `z` values that decide how deep it is: the nearer of the two, which is
    /// the `max`. That is a fact about the geometry rather than about
    /// visibility -- an edge at the near corner of a face-on cube is the front
    /// of the cube, and one at the far corner of the back face is the back of
    /// it, and both are the intersection of exactly the planes that put them
    /// there.
    ///
    /// They used to be the whole of the hidden-line removal as well: a convex
    /// solid hides an edge exactly when *both* of its faces point away, so one
    /// boolean per face was enough to cull the three far edges, with no depth
    /// sort and no z-buffer. That culling is gone -- see
    /// [`Cube::render`] -- and the faces are kept for the depth and for the
    /// corner markers, both of which still need them.
    faces: [usize; 2],
}

/// One of the cube's six faces, as the four corners around it.
///
/// The order is counter-clockwise seen from outside, so the cross product of
/// the first two spans gives the outward normal without any face having to be
/// special-cased.
struct Face {
    corners: [usize; 4],
}

/// The six faces, indexed the way [`EDGES`] refers to them.
///
/// 0 front (z-)   1 back (z+)   2 right (x+)
/// 3 left (x-)    4 top (y+)    5 bottom (y-)
///
/// A constant rather than a field on the effect. It never changes, so holding
/// it on the effect bought nothing except two borrows per frame that the
/// renderer then had to copy out of before it could write -- and a `Vec` of
/// topology allocated per effect instance for sixteen bytes of content.
const FACES: [Face; 6] = [
    Face {
        corners: [0, 3, 2, 1],
    },
    Face {
        corners: [4, 5, 6, 7],
    },
    Face {
        corners: [1, 2, 6, 5],
    },
    Face {
        corners: [0, 4, 7, 3],
    },
    Face {
        corners: [3, 7, 6, 2],
    },
    Face {
        corners: [0, 1, 5, 4],
    },
];

/// The twelve edges, grouped as the old effect's list was and with the two
/// faces each is shared by.
const EDGES: [Edge; 12] = [
    // Front face
    Edge {
        v1: 0,
        v2: 1,
        faces: [0, 5],
    },
    Edge {
        v1: 1,
        v2: 2,
        faces: [0, 2],
    },
    Edge {
        v1: 2,
        v2: 3,
        faces: [0, 4],
    },
    Edge {
        v1: 3,
        v2: 0,
        faces: [0, 3],
    },
    // Back face
    Edge {
        v1: 4,
        v2: 5,
        faces: [1, 5],
    },
    Edge {
        v1: 5,
        v2: 6,
        faces: [1, 2],
    },
    Edge {
        v1: 6,
        v2: 7,
        faces: [1, 4],
    },
    Edge {
        v1: 7,
        v2: 4,
        faces: [1, 3],
    },
    // Connecting edges
    Edge {
        v1: 0,
        v2: 4,
        faces: [3, 5],
    },
    Edge {
        v1: 1,
        v2: 5,
        faces: [2, 5],
    },
    Edge {
        v1: 2,
        v2: 6,
        faces: [2, 4],
    },
    Edge {
        v1: 3,
        v2: 7,
        faces: [3, 4],
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CubeOptions {
    pub cube_size: f32,
    pub rotation_speed_x: f32,
    pub rotation_speed_y: f32,
    pub rotation_speed_z: f32,
    pub distance: f32,
    pub use_braille: bool,
    /// Fraction of the shorter half-axis the cube's bounding sphere spans.
    ///
    /// See [`Cube::projection_scale`]. Below 1.0 so that the extreme vertex
    /// still lands inside the last row and column: the rasterisers address
    /// cells by `floor`, and a vertex exactly on the boundary is a dot at
    /// `4 * height`, which the bounds check drops.
    pub fit: f32,
    /// Fill the faces, or draw the wireframe alone.
    ///
    /// **Off by default**, and the reason is a person watching the effect rather
    /// than a measurement. Filled, the cube is a solid whose three front-facing
    /// faces are dithered fields of raised dots; the edge lines are then no
    /// longer the brightest thing on screen, because the near face's own dither
    /// runs up to the same densities near its own edges, and the silhouette
    /// stops being a silhouette. The wireframe is the version that reads as a
    /// cube from across the room, which is what a screensaver has to do. The
    /// fill is kept because it is a real alternative look and `true` still
    /// renders it; it is simply not what a person gets without asking.
    ///
    /// The default wireframe is an X-ray one: all twelve edges, with the three
    /// behind the solid drawn dimmer rather than culled. `true` fills the three
    /// front-facing faces and leaves the far side's edges drawn, because the
    /// edges are the cube's outline whether or not there is a fill behind them.
    ///
    /// Pinned by `the_default_is_the_wireframe_rather_than_the_filled_cube`.
    pub filled: bool,
    /// Mark the eight corners.
    pub vertex_markers: bool,
    /// Characters the *cell-resolution* face fill is drawn as, sparsest first.
    ///
    /// Only read when `use_braille` is off: the braille renderer expresses a
    /// face's density in raised dots rather than in ink coverage, so it has no
    /// ramp to draw from.
    pub glyphs: String,
    /// Seed for this effect.
    ///
    /// [`DEFAULT_SEED`] does not mean 42 here -- it means **unset**, and
    /// [`Config::randomise_seeds`](crate::config::Config::randomise_seeds)
    /// gives it a fresh value at startup. Which is what makes this effect look
    /// different on every launch, which it did not before it had a seed at all.
    /// `--seed N` pins it.
    pub seed: u64,
}

impl Default for CubeOptions {
    /// Hand-written so it is the single source of truth.
    ///
    /// The builder carried the real defaults while the derived `Default`
    /// produced zeros, and serde used the derived one, so a config file
    /// that omitted a section silently zeroed it.
    fn default() -> Self {
        Self {
            cube_size: DEFAULT_CUBE_SIZE,
            seed: DEFAULT_SEED,
            rotation_speed_x: 0.25,
            rotation_speed_y: 0.35,
            rotation_speed_z: 0.18,
            distance: DEFAULT_DISTANCE,
            use_braille: true,
            fit: DEFAULT_FIT,
            filled: DEFAULT_FILLED,
            vertex_markers: true,
            glyphs: DEFAULT_GLYPHS.to_string(),
        }
    }
}

/// The default `cube_size`, and what an unusable one falls back to.
///
/// Falling back rather than clamping to zero is the `terrain` precedent: both
/// fields are user-facing, both are `f32`, and a hand-edited config can put
/// `NaN` in either, and a `NaN` size propagates into every projected
/// coordinate and then into `as i32`, which reads it as 0 and draws the whole
/// cube in the top-left corner.
const DEFAULT_CUBE_SIZE: f32 = 1.0;

/// The default camera distance, and what an unusable one falls back to.
const DEFAULT_DISTANCE: f32 = 3.5;

/// How much of the camera distance the cube's own half-diagonal may occupy.
///
/// The perspective divides by `distance + z`, and the nearest vertex has
/// `z = -cube_size * sqrt(3)`, so the denominator bottoms out at
/// `distance - cube_size * sqrt(3)`. At the defaults that is 2.5. Let it
/// approach zero and the near face blows up across the screen; let it go
/// negative and the near vertices project *behind* the eye, the winding
/// inverts, and a convex solid stops being convex -- every back-face test in
/// this file inverts with it, so the effect draws inside-out rather than
/// merely too large.
///
/// Clamped rather than rejected, because "as big as this camera can honestly
/// show" is a well-defined answer to a `cube_size` that is too large, and
/// rejecting would leave the effect blank. 0.8 leaves the near face at a
/// quarter of the distance, which is a wide-angle enough view to read as
/// perspective.
const MAX_CAMERA_FILL: f32 = 0.8;

/// Clamp range for `distance`.
///
/// Not a taste decision. `projection_scale` is `half_extent / projected_radius`,
/// and `projected_radius` is roughly `radius / distance`, so the scale grows in
/// proportion to `distance` and the projected coordinate is `world * scale`. At
/// the upper bound the projected coordinate is about 4e4, comfortably inside
/// `f32`; without a bound, a `distance` of 1e38 in a hand-edited config makes
/// the scale `inf` and every coordinate `NaN`, which `as i32` turns into 0 --
/// so the cube vanishes into one corner rather than merely looking wrong.
const MIN_DISTANCE: f32 = 0.1;
const MAX_DISTANCE: f32 = 1000.0;

/// The fraction of the half-axis the cube spans by default.
///
/// Just under a whole half-axis, for the `floor`ing reason on
/// [`CubeOptions::fit`]. 0.95 rather than 0.9 because the fitted value is exact
/// and there is no reason to give up a twentieth of the terminal: the whole
/// reason the old 0.8 fudge was replaced is that a constant which happens to
/// fit at one size is not a fit.
const DEFAULT_FIT: f32 = 0.95;

/// Whether the faces are filled by default.
///
/// A constant rather than a bare `false` so the default has one name, and so the
/// doc comment above can be attached to the value rather than to a literal that a
/// later edit can move. See [`CubeOptions::filled`].
const DEFAULT_FILLED: bool = false;

/// Vertical compression of the projection, as a fraction of the horizontal.
///
/// A terminal cell is taller than it is wide -- DejaVu Sans Mono is about
/// 1 : 1.2, the usual assumption is 1 : 2 -- so an uncorrected cube is a
/// rhombus twice as tall as it is wide. Every sub-cell renderer in this crate
/// assumes the same 1 : 2 and is correspondingly squashed on a real font; this
/// is the same correction, chosen conservatively for the same reason
/// `terrain`'s `CELL_ASPECT` is 1.2 rather than 2.
const Y_SQUASH: f32 = 0.8;

/// How close to edge-on a face may get and still be drawn.
///
/// A fraction of `normal . (eye - centroid)`, which for a convex face is its
/// projected area times the product of two lengths and so is already
/// size-independent and distance-independent. A thousandth puts the cut-off at
/// about 0.03 degrees of tilt, which is far below any face worth drawing and
/// far above the cancellation error in a cross product of two nearly-parallel
/// spans.
const EDGE_ON_EPSILON: f32 = 1.0e-3;

/// The lightest a face's dots ever get, as a fraction of the cell filled.
///
/// A face is filled to a density set by its depth and dithered to that density,
/// so the far face is a sparse field and the near one is solid. Not zero,
/// because a face pointing away should recede rather than vanish: the cube is
/// drawn against the terminal's own background, and a face at 4% density
/// simply reads as noise rather than as a plane.
const MIN_FACE_DENSITY: f32 = 0.22;

/// How far past the farthest front-facing face the depth ramp reaches, as a
/// fraction of the distance on to the farthest face of any kind.
///
/// The reserve the far side of the cube is drawn in. See
/// [`Cube::face_depth_range`] for why the ramp has to reach past the visible
/// faces at all; this is how much it reaches.
///
/// 0.15 is a measured choice between two things pulling opposite ways, and
/// the curve is worth more than the number. The band cannot be too small or
/// the far side is not visibly separate from the near one; it cannot be too
/// large or it eats the ramp the *front* side shades itself with, and a
/// rotating wireframe whose nine near edges are all the same white is a set
/// of lines rather than a solid. Sweeping the 200 rotations of
/// `a_far_edge_is_dimmer_than_the_near_ones` and measuring both, in
/// luminance out of 255:
///
/// ```text
/// band   worst-pair gap: median / 5th pct    near side's own contrast
/// 0.10          72.9 / 34.4                            68.0
/// 0.15          91.6 / 42.3                            49.5
/// 0.25         112.7 / 35.4                            27.4
/// 0.35         115.2 / 29.1                            11.9
/// 0.50          97.7 / 24.6                             5.2
/// ```
///
/// The "worst pair" is the dimmest near-side edge against the brightest
/// far-side edge, which is the pair that would be confused first. That gap
/// peaks in the median around 0.3 and then *falls*, because past that the
/// band is wide enough to swallow the far side's own shading and carry it up
/// the ramp with everything else; its 5th percentile peaks here, at 0.15.
/// Meanwhile the near side's own depth cue halves between 0.10 and 0.25 and
/// is nearly gone by 0.35. So 0.15 is where the tail is at its worst and the
/// front side still has 49.5 of 255 to shade itself with.
///
/// The residual rotations where the gap all but closes are not a bug in the
/// number. They are the poses where a face is within a fraction of a degree
/// of edge-on: a turned-away face that is nearly edge-on really is at almost
/// exactly the distance of a front-facing one that is nearly edge-on, and
/// shading those two differently would be a lie about the geometry. The
/// measurement is written around that rather than pretending it is not there.
const FAR_SIDE_BAND: f32 = 0.15;

/// The corner marker.
///
/// Ambiguous-width but not double-width, so one cell in a Latin-configured
/// terminal, and it carries a colour rather than relying on bold -- see
/// [`VERTEX_COLOUR`].
const VERTEX_GLYPH: char = '\u{25C6}';

/// The edge glyph on the cell-resolution path.
///
/// The braille path has no such thing: an edge there is a run of raised dots at
/// full density, which is the same idea at that resolution.
const ASCII_EDGE_GLYPH: char = '\u{2588}';

/// The colour the corner markers are drawn in.
///
/// `Attribute::Bold` is the obvious way to make eight small glyphs stand out
/// from the edge they sit on, and it is a brightening *hint* that many
/// terminals act on -- so the markers would come out brighter on some machines
/// than others, and the top of a depth ramp would land somewhere
/// unpredictable. The edge ramp already ends at white, so the marker is
/// distinguished by its shape and nothing else.
const VERTEX_COLOUR: Color = Color::Rgb {
    r: 255,
    g: 255,
    b: 255,
};

/// Face colour, from the far plane to the near one.
///
/// The far stop is nearly the background rather than a saturated dark version
/// of the green. A face pointing away is the thing that should recede, and a
/// dark *saturated* green reads as a solid slab that the near faces then have
/// to be brighter than -- which puts a brightness step in the wrong place. The
/// top stop is the near face and it stops short of white, because the edges are
/// drawn from the same range and the two have to stay tellable apart.
const FACE_RAMP: &[Color] = &[
    Color::Rgb {
        r: 10,
        g: 34,
        b: 58,
    },
    Color::Rgb {
        r: 20,
        g: 92,
        b: 128,
    },
    Color::Rgb {
        r: 46,
        g: 190,
        b: 168,
    },
    Color::Rgb {
        r: 190,
        g: 255,
        b: 226,
    },
];

/// Edge colour: the same range, lifted.
///
/// A separate ramp rather than a brightening multiplier applied to the face
/// colour, because the point of an edge is to be the brightest thing on screen
/// *wherever it is*. An edge on the far side of the cube is still the edge, and
/// the cube has no cue other than this to say which lines are its own outline
/// and which are the silhouette of a face.
///
/// The bottom stop is the far side's, and that is the one job it does now that
/// nothing is culled. The ramp is indexed by depth, the depth range reaches past
/// the front-facing faces so the far side has somewhere dimmer to go
/// ([`FAR_SIDE_BAND`]), and this is where it lands: 115 of 255 in luminance,
/// against 255 for an edge on the nearest face. It is not a colour that has to
/// be dim on purpose -- it is where a far edge's depth puts it, and the test
/// that says so measures the drawn frame rather than this array.
const EDGE_RAMP: &[Color] = &[
    Color::Rgb {
        r: 62,
        g: 130,
        b: 176,
    },
    Color::Rgb {
        r: 120,
        g: 210,
        b: 216,
    },
    Color::Rgb {
        r: 236,
        g: 255,
        b: 250,
    },
    Color::Rgb {
        r: 255,
        g: 255,
        b: 255,
    },
];

/// The cell-resolution face glyphs, by default.
///
/// Blocks, for the reason `terrain` uses them: `░▒▓█` are *defined* as
/// quarter, half, three-quarter and full coverage of the cell box, so their
/// order holds by the standard rather than by taste, and it holds at any cell
/// aspect ratio -- which a punctuation ramp does not. `SHADE` would be wrong
/// here for the documented reason that `=` is heavier than `+`.
const DEFAULT_GLYPHS: &str = glyph_ramp::presets::BLOCKS;

/// "No depth recorded" in the per-cell depth arrays.
///
/// The depth is quantised into `1..=255` rather than `0..=255` so that zero is
/// free to mean *absent*. Storing a raw `0..=255` would make the far plane
/// indistinguishable from "nothing here", and a face exactly on the far plane
/// does occur -- it is the face that is culled, and the cells behind it are
/// exactly the cells that must not inherit its depth.
const NO_DEPTH: u8 = 0;

/// The half-open rectangle a frame wrote to.
///
/// The flush reads two palettes and does a per-cell decision, so it walks the
/// cube's own bounding box rather than the whole terminal: at 400x200 the
/// difference is 80,000 cells against the few thousand the cube occupies.
#[derive(Clone, Copy)]
struct Rect {
    left: usize,
    top: usize,
    right: usize,
    bottom: usize,
}

impl Rect {
    fn empty() -> Self {
        Self {
            left: usize::MAX,
            top: usize::MAX,
            right: 0,
            bottom: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    fn grow(&mut self, x: usize, y: usize) {
        self.left = self.left.min(x);
        self.top = self.top.min(y);
        self.right = self.right.max(x + 1);
        self.bottom = self.bottom.max(y + 1);
    }
}

/// The per-cell scratch both renderers draw into.
///
/// One rectangle of per-cell state, and the point of it is that a corner cell
/// is written once. The old renderer built a fresh dot map per *edge* and then
/// `set` the whole cell, so all twelve edges wrote over each other and the last
/// one won. Every vertex is the endpoint of three edges, so all eight corners
/// came out as the end-stub of whichever edge happened to be drawn last, and
/// they are the one part of a cube that reads as a cube.
#[derive(Clone)]
struct Field {
    /// Raised dots from face fills.
    faces: BrailleGrid,
    /// Raised dots from edges, kept apart because a braille cell carries one
    /// colour and the flush has to know which of the two reached it.
    edges: BrailleGrid,
    /// The nearest face over each cell, quantised to `1..=255`.
    face_depth: Vec<u8>,
    /// The nearest edge over each cell, quantised the same way.
    edge_depth: Vec<u8>,
    /// The corner marker at each cell, `NO_DEPTH` for none.
    vertex_depth: Vec<u8>,
    dirty: Rect,
}

impl Field {
    fn new(width: u16, height: u16) -> Self {
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        Self {
            faces: BrailleGrid::new(w, h),
            edges: BrailleGrid::new(w, h),
            face_depth: vec![NO_DEPTH; w * h],
            edge_depth: vec![NO_DEPTH; w * h],
            vertex_depth: vec![NO_DEPTH; w * h],
            dirty: Rect::empty(),
        }
    }

    fn resize(&mut self, width: u16, height: u16) {
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        if (w, h) == (self.faces.width(), self.faces.height()) {
            return;
        }
        *self = Self::new(width, height);
    }

    /// Blanks every cell and restarts the dirty rectangle.
    ///
    /// Wholly rather than just the previous frame's rectangle, and that is a
    /// deliberate trade: the alternative needs the *previous* rectangle to
    /// survive the clear, which means the flush reads a rectangle the render
    /// has already reset. The cost is 400 KB of memset at 400x200, sixty times
    /// a second, which is under 20 microseconds -- and it is what the old
    /// renderer spent its whole budget on, twelve times over, in twelve
    /// `HashMap`s.
    fn clear(&mut self) {
        self.faces.clear();
        self.edges.clear();
        self.face_depth.fill(NO_DEPTH);
        self.edge_depth.fill(NO_DEPTH);
        self.vertex_depth.fill(NO_DEPTH);
        self.dirty = Rect::empty();
    }

    #[inline]
    fn mark_face(&mut self, x: usize, y: usize, depth: u8) {
        let index = y * self.faces.width() + x;
        if depth > self.face_depth[index] {
            self.face_depth[index] = depth;
        }
        self.dirty.grow(x, y);
    }

    /// Records an edge crossing a cell, on the cell-resolution path.
    ///
    /// The braille path has no counterpart: there an edge *is* dots, and they
    /// land in `edges`. This is here so both renderers share one `render` and
    /// one flush, differing only in what the per-cell record means.
    #[inline]
    fn mark_edge_cell(&mut self, x: usize, y: usize, depth: u8) {
        let index = y * self.faces.width() + x;
        if depth > self.edge_depth[index] {
            self.edge_depth[index] = depth;
        }
        self.dirty.grow(x, y);
    }

    #[inline]
    fn mark_vertex(&mut self, x: usize, y: usize, depth: u8) {
        let index = y * self.faces.width() + x;
        // Nearest wins, so the *minimum* quantised depth, unlike the two
        // marks above. Two corners in one cell is possible on a small
        // terminal, and of the two the nearer one is the one in front.
        if self.vertex_depth[index] == NO_DEPTH || depth < self.vertex_depth[index]
        {
            self.vertex_depth[index] = depth;
        }
        self.dirty.grow(x, y);
    }

    /// Raises one dot belonging to an edge.
    #[inline]
    fn raise_edge_dot(&mut self, dot_x: usize, dot_y: usize, depth: u8) {
        let (cx, cy) = (dot_x / DOTS_X, dot_y / DOTS_Y);
        if cx >= self.edges.width() || cy >= self.edges.height() {
            return;
        }
        self.edges.raise_dot(dot_x, dot_y);
        let index = cy * self.edges.width() + cx;
        if depth > self.edge_depth[index] {
            self.edge_depth[index] = depth;
        }
        self.dirty.grow(cx, cy);
    }

    /// Raises one dot belonging to a face fill.
    #[inline]
    fn raise_face_dot(&mut self, dot_x: usize, dot_y: usize, depth: u8) {
        let (cx, cy) = (dot_x / DOTS_X, dot_y / DOTS_Y);
        if cx >= self.faces.width() || cy >= self.faces.height() {
            return;
        }
        self.faces.raise_dot(dot_x, dot_y);
        let index = cy * self.faces.width() + cx;
        if depth > self.face_depth[index] {
            self.face_depth[index] = depth;
        }
        self.dirty.grow(cx, cy);
    }
}

/// Packs a depth in `0.0..=1.0` into the `1..=255` range the field stores.
///
/// The shift rather than a `0` sentinel is what lets `NO_DEPTH` mean "nothing
/// here" without also being a reachable depth. See [`NO_DEPTH`].
#[inline]
fn quantise(depth: f32) -> u8 {
    let depth = if depth.is_nan() {
        0.0
    } else {
        depth.clamp(0.0, 1.0)
    };
    1 + (depth * 254.0).round() as u8
}

/// The inverse of [`quantise`].
#[inline]
fn dequantise(packed: u8) -> f32 {
    if packed == NO_DEPTH {
        return 0.0;
    }
    (packed as f32 - 1.0) / 254.0
}

/// A user-facing `f32` that has to be usable, or a fallback.
///
/// `f32::clamp` propagates `NaN` rather than saturating it, so a `NaN` in a
/// config file would sail through a bounds check and arrive in the projection
/// as a `NaN` coordinate, which `as i32` reads as 0.
#[inline]
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Builds the glyph ramp, never empty and never a character that could shear a
/// cell-indexed grid.
fn glyph_ramp(configured: &str) -> GlyphRamp {
    let glyphs: Vec<char> = configured
        .chars()
        .filter(|glyph| {
            !glyph.is_control() && glyph_ramp::is_ambiguous_or_narrow(*glyph)
        })
        .collect();

    if glyphs.is_empty() {
        GlyphRamp::from_text(DEFAULT_GLYPHS)
    } else {
        GlyphRamp::new(glyphs)
    }
}

pub struct Cube {
    pub screen_size: (u16, u16),
    options: CubeOptions,
    canvas: Canvas,
    vertices: Vec<Point3D>,
    rotation: (f32, f32, f32),
    simulation_time: Duration,
    field: Field,
    ramp: GlyphRamp,
    face_palette: Palette,
    edge_palette: Palette,
    /// The rotated and projected vertices, held across frames.
    ///
    /// Eight `Point3D` and eight `Point2D` a frame is nothing, but it is two
    /// `Vec`s the old code allocated on every one of the sixty frames a
    /// second, and the projected copy is what the rasterisers and every test
    /// read, so it wants to be one addressable buffer rather than a fresh
    /// allocation.
    rotated: Vec<Point3D>,
    projected: Vec<Point2D>,
}

impl TerminalEffect for Cube {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        self.canvas.clear();
        self.render();
        let Self {
            canvas,
            field,
            options,
            ramp,
            face_palette,
            edge_palette,
            ..
        } = self;
        Self::flush(field, options, ramp, face_palette, edge_palette, canvas);
        canvas.commit()
    }

    fn update(&mut self) {
        // The rate the rotation was tuned against. `update_with_context` is
        // the real one.
        self.simulation_time += Duration::from_secs_f64(1.0 / 60.0);
        self.update_rotation();
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        self.simulation_time += context.delta;
        self.update_rotation();
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(self.screen_size.0, self.screen_size.1);
        self.field.resize(self.screen_size.0, self.screen_size.1);
    }

    fn reset(&mut self) {
        *self = Self::new(self.options.clone(), self.screen_size);
    }
}

impl Cube {
    fn update_rotation(&mut self) {
        let elapsed = self.simulation_time.as_secs_f32();
        self.rotation.0 = elapsed * self.options.rotation_speed_x;
        self.rotation.1 = elapsed * self.options.rotation_speed_y;
        self.rotation.2 = elapsed * self.options.rotation_speed_z;
    }

    pub fn new(options: CubeOptions, screen_size: (u16, u16)) -> Self {
        let canvas = Canvas::new(screen_size.0, screen_size.1);

        // The vertices are built from the *clamped* size, not the configured
        // one, so the geometry matches what the fit computation assumed. Two
        // different sizes would still produce a cube -- just not the one the
        // screen was fitted to, and the whole cube would be the wrong size on
        // screen with no way to tell from the output.
        let size = options.effective_cube_size();
        let vertices = vec![
            Point3D {
                x: -size,
                y: -size,
                z: -size,
            }, // 0: front bottom left
            Point3D {
                x: size,
                y: -size,
                z: -size,
            }, // 1: front bottom right
            Point3D {
                x: size,
                y: size,
                z: -size,
            }, // 2: front top right
            Point3D {
                x: -size,
                y: size,
                z: -size,
            }, // 3: front top left
            Point3D {
                x: -size,
                y: -size,
                z: size,
            }, // 4: back bottom left
            Point3D {
                x: size,
                y: -size,
                z: size,
            }, // 5: back bottom right
            Point3D {
                x: size,
                y: size,
                z: size,
            }, // 6: back top right
            Point3D {
                x: -size,
                y: size,
                z: size,
            }, // 7: back top left
        ];

        // The six faces and twelve edges are `FACES` and `EDGES`, which are
        // constants -- see there for the ordering and the adjacency.

        let face_palette = Palette::new(FACE_RAMP.to_vec());
        let edge_palette = Palette::new(EDGE_RAMP.to_vec());

        Self {
            screen_size,
            rotated: vec![
                Point3D {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0
                };
                vertices.len()
            ],
            projected: vec![Point2D { x: 0.0, y: 0.0 }; vertices.len()],
            field: Field::new(screen_size.0, screen_size.1),
            ramp: glyph_ramp(&options.glyphs),
            face_palette,
            edge_palette,
            options,
            canvas,
            vertices,
            rotation: (0.0, 0.0, 0.0),
            simulation_time: Duration::ZERO,
        }
    }

    // Rotate a point using rotation matrices
    fn rotate_point(rotation: (f32, f32, f32), p: Point3D) -> Point3D {
        let (rx, ry, rz) = rotation;

        // X-axis rotation
        let cos_x = rx.cos();
        let sin_x = rx.sin();
        let y1 = p.y * cos_x - p.z * sin_x;
        let z1 = p.y * sin_x + p.z * cos_x;

        // Y-axis rotation
        let cos_y = ry.cos();
        let sin_y = ry.sin();
        let x2 = p.x * cos_y + z1 * sin_y;
        let z2 = -p.x * sin_y + z1 * cos_y;

        // Z-axis rotation
        let cos_z = rz.cos();
        let sin_z = rz.sin();
        let x3 = x2 * cos_z - y1 * sin_z;
        let y3 = x2 * sin_z + y1 * cos_z;

        Point3D {
            x: x3,
            y: y3,
            z: z2,
        }
    }

    /// `cube_size`, clamped so the camera stays in front of the whole cube.
    ///
    /// See [`MAX_CAMERA_FILL`]. This is the only place the clamp lives, so the
    /// vertices, the bounding radius the fit uses, and the projection all
    /// agree on how big the cube is.
    fn effective_cube_size(&self) -> f32 {
        let size = finite_or(self.options.cube_size, DEFAULT_CUBE_SIZE);
        if size <= 0.0 {
            return DEFAULT_CUBE_SIZE;
        }
        let distance = self.effective_distance();
        size.min(MAX_CAMERA_FILL * distance / 3.0f32.sqrt())
    }

    /// `distance`, clamped to a range the projection survives.
    ///
    /// See [`MIN_DISTANCE`] and [`MAX_DISTANCE`].
    fn effective_distance(&self) -> f32 {
        let distance = finite_or(self.options.distance, DEFAULT_DISTANCE);
        if distance <= 0.0 {
            return DEFAULT_DISTANCE;
        }
        distance.clamp(MIN_DISTANCE, MAX_DISTANCE)
    }

    /// The cube's circumradius: every vertex is `sqrt(3)` half-diagonals out.
    fn bounding_radius(&self) -> f32 {
        self.effective_cube_size() * 3.0f32.sqrt()
    }

    /// How far the bounding sphere can reach on screen, per unit of scale.
    ///
    /// A point at radius `r` from the origin, in the plane containing the view
    /// axis, is drawn at `r * sin(theta) / (distance + r * cos(theta))`.
    /// Differentiating that over the ball of radius `R` and setting the
    /// derivative to zero gives `cos(theta) = -R / distance` and a maximum of
    /// `R / sqrt(distance^2 - R^2)`.
    ///
    /// The bound is *reached* rather than approached, which is the part worth
    /// checking before trusting it: the eight vertices are at `sqrt(3)` times
    /// the half-side, so rotating the cube sweeps each of them around a circle
    /// of radius `R`, and every angle on that circle is attained.
    fn projected_radius(radius: f32, distance: f32) -> f32 {
        let inside = (distance * distance - radius * radius).max(0.0);
        if inside <= 0.0 {
            return 0.0;
        }
        radius / inside.sqrt()
    }

    /// The projection scale, in cell units, that fits the cube to the screen.
    ///
    /// Derived, where the old code multiplied the shorter terminal dimension by
    /// a hard-coded `0.8` and hoped. That number is not a fit: the cube's
    /// worst-case projected radius is 0.57 of a half-axis per unit of scale, so
    /// `0.8 * 0.8` happened to leave the default `cube_size` on screen at the
    /// sizes it was tried at -- and clipped off the top and bottom everywhere
    /// else. `cube_size` is a public config field, and `1.4` is enough to push
    /// a vertex past the last row at 80x24, which is where this was measured
    /// from. There is no value of a constant that fixes it, because the right
    /// constant is a function of the terminal's aspect ratio.
    fn projection_scale(&self) -> f32 {
        let radius = self.bounding_radius();
        let distance = self.effective_distance();
        let unit_x = Self::projected_radius(radius, distance);
        if unit_x <= 0.0 {
            return 0.0;
        }
        let unit_y = unit_x * Y_SQUASH;

        let half_width = self.screen_size.0 as f32 * 0.5;
        let half_height = self.screen_size.1 as f32 * 0.5;
        if half_width <= 0.0 || half_height <= 0.0 {
            return 0.0;
        }

        // Whichever axis runs out first sets the scale, so the cube is as large
        // as this terminal can honestly show rather than as large as a constant
        // says. A wide short terminal is limited by its height and a narrow
        // tall one by its width, and neither case is special-cased.
        //
        // A non-positive `fit` falls back rather than being honoured, the same
        // as a non-positive `cube_size`. Clamping it to a small positive
        // number would draw a cube four cells across, which is a broken effect
        // that looks like a rendering failure; and "how much of the screen" is
        // not a question with a useful answer of zero.
        let fit = finite_or(self.options.fit, DEFAULT_FIT);
        let fit = if fit <= 0.0 {
            DEFAULT_FIT
        } else {
            fit.min(1.0)
        };
        (half_width / unit_x).min(half_height / unit_y) * fit
    }

    // Project a 3D point to 2D screen coordinates
    fn project(p: Point3D, scale: f32, size: (u16, u16), distance: f32) -> Point2D {
        let z_factor = 1.0 / (distance + p.z);

        // Calculate screen coordinates
        let width = size.0 as f32;
        let height = size.1 as f32;

        let screen_x = width / 2.0 + p.x * z_factor * scale;
        let screen_y = height / 2.0 + p.y * z_factor * scale * Y_SQUASH;

        Point2D {
            x: screen_x,
            y: screen_y,
        }
    }

    /// The depth range the ramp is normalised against: the near end of the
    /// *visible faces' own planes*, and a far end that reaches past them.
    ///
    /// See the note in [`render`](Self::render) for why the near end is not
    /// the nearest vertex.
    ///
    /// The far end is not the farthest visible face either, and that is the
    /// whole of what makes the X-ray work. The far side of the cube is drawn
    /// too -- there is no hidden-line removal -- so the faces pointing away
    /// have to land somewhere *below* everything turned towards the viewer,
    /// or the far face's outline and the two silhouette edges belonging to the
    /// farthest front-facing face come out at the same depth and therefore at
    /// the same colour. Measured, with the far end pinned to the farthest
    /// visible face, that is not a subtle wash: at *every* rotation the
    /// brightest far-side edge and the dimmest near-side edge came out
    /// identical to within 0.1 of 255, because the farthest visible face is at
    /// the far end of the range by construction and a far edge clamped past it
    /// lands on the same stop.
    ///
    /// So the far end is pushed out to [`FAR_SIDE_BAND`] of the way to the
    /// farthest face of any kind, which puts every hidden edge strictly below
    /// every visible one without a second ramp, a second palette or a boolean
    /// per edge. The near/far cue is still one linear function of `z`; the
    /// only thing that changed is where it stops.
    ///
    /// The degenerate case is a cube viewed exactly down one of its axes, where
    /// one face is visible and there is no depth ordering left to express among
    /// the visible faces. A zero-width range would divide by zero, so the
    /// fallback is a nominal width of two circumradii placed so the visible
    /// plane sits at the *top* of it, which draws that face solid and bright.
    ///
    /// Top rather than middle, and that is a measured decision rather than a
    /// tidy one. Centred on the plane puts the lone face at `t = 0.5`, so it
    /// is drawn at 61% dot density -- and a face-on cube then comes out as a
    /// 61% checkerboard, which is the one rotation at which the viewer can see
    /// that the texture is dithering rather than shading anything. The single
    /// visible face is the nearest thing on screen, so drawing it as the
    /// nearest thing is both the honest reading and the readable one.
    fn face_depth_range(&self, visible: &[bool; FACES.len()]) -> (f32, f32) {
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        // The farthest face of *any* kind, which the far end is measured out
        // towards. It is the far side's own ceiling rather than an arbitrary
        // width, so the band the far side gets is a fraction of the real
        // distance there is to cover however the cube happens to be turned.
        let mut farthest = f32::NEG_INFINITY;
        for (index, face) in FACES.iter().enumerate() {
            let z = self.face_centroid_z(face);
            if !z.is_finite() {
                continue;
            }
            farthest = farthest.max(z);
            if !visible[index] {
                continue;
            }
            low = low.min(z);
            high = high.max(z);
        }

        // A convex solid always has at least one front-facing face, so `low`
        // and `high` are finite here; the guard below is for a `NaN` rotation
        // from a hand-edited config, not for a real pose.
        if low.is_finite() && high.is_finite() {
            // `-inf` (or `NaN`, if both ends went at once) collapses to zero
            // rather than poisoning the sum below.
            let beyond = (farthest - high).max(0.0);
            let far = high + FAR_SIDE_BAND * beyond;
            if far.is_finite() && far - low > 0.0 {
                return (low, far);
            }
        }

        let near = if low.is_finite() { low } else { 0.0 };
        let width =
            (1.0 + FAR_SIDE_BAND) * 2.0 * self.bounding_radius().max(1.0e-3);
        (near, near + width)
    }

    /// How deep a rotated `z` sits, as `0.0` at the far end of the range and
    /// `1.0` at the near one.
    #[inline]
    fn depth_of(&self, z: f32, range: (f32, f32)) -> f32 {
        if !z.is_finite() {
            return 0.0;
        }
        let span = range.1 - range.0;
        let span = if span > 0.0 { span } else { 1.0 };
        ((range.1 - z) / span).clamp(0.0, 1.0)
    }

    /// Whether a face is turned towards the viewer.
    ///
    /// The cross product of the first two spans is the face's outward normal,
    /// because [`FACES`] lists every face's corners counter-clockwise seen from
    /// outside -- verified for all six by hand, and pinned by
    /// `the_face_winding_is_outward_at_every_corner` below.
    ///
    /// A convex face is visible exactly when the eye is on its *outer* side, so
    /// the test is `normal . (eye - centroid) > 0`. The eye is at
    /// `(0, 0, -distance)`, not at the origin: the projection is
    /// `1 / (distance + z)`, so a point's distance from the eye runs from
    /// `distance` at `z = 0` towards zero as `z` goes negative, and the
    /// vanishing point is `-z`. Using the origin instead makes the test
    /// `normal . centroid < 0`, which is *also* a correct visibility test for
    /// an orthographic camera and differs from the perspective one exactly
    /// where it matters here: it keeps faces that the eye is behind.
    ///
    /// Three-dimensional rather than a projected winding sign, because of the
    /// degenerate case. A face exactly edge-on has a cross product of
    /// numerically zero, and the *sign* of zero is cancellation noise, so a
    /// sign test makes the face flicker in and out of existence as the cube
    /// turns. Hence the epsilon, scaled by the two vectors' lengths so it means
    /// the same thing at any cube size or camera distance.
    fn face_is_front_facing(&self, face: &Face) -> bool {
        let c = &self.rotated;
        let (a, b, d, e) = (
            c[face.corners[0]],
            c[face.corners[1]],
            c[face.corners[2]],
            c[face.corners[3]],
        );
        let u = Point3D {
            x: b.x - a.x,
            y: b.y - a.y,
            z: b.z - a.z,
        };
        let v = Point3D {
            x: d.x - a.x,
            y: d.y - a.y,
            z: d.z - a.z,
        };
        let normal = Point3D {
            x: u.y * v.z - u.z * v.y,
            y: u.z * v.x - u.x * v.z,
            z: u.x * v.y - u.y * v.x,
        };
        let centroid = Point3D {
            x: (a.x + b.x + d.x + e.x) * 0.25,
            y: (a.y + b.y + d.y + e.y) * 0.25,
            z: (a.z + b.z + d.z + e.z) * 0.25,
        };
        let to_eye = Point3D {
            x: -centroid.x,
            y: -centroid.y,
            z: -self.effective_distance() - centroid.z,
        };
        let dot = normal.x * to_eye.x + normal.y * to_eye.y + normal.z * to_eye.z;
        let scale = Self::length(&normal) * Self::length(&to_eye);

        dot > scale * EDGE_ON_EPSILON
    }

    fn length(p: &Point3D) -> f32 {
        (p.x * p.x + p.y * p.y + p.z * p.z).sqrt()
    }

    /// The horizontal span of a projected convex quad at one scanline.
    ///
    /// `None` when the row misses it. A convex quad meets any horizontal line
    /// in at most one segment, so bracketing it with the quad's edge crossings
    /// is exact -- and doing it per row rather than per dot is the difference
    /// between four divisions per row and four per dot, which at 400x200 is
    /// the difference between about 600k tests a frame and two and a half
    /// million.
    fn row_span(quad: &[Point2D; 4], y: f32) -> Option<(f32, f32)> {
        let mut low = f32::INFINITY;
        let mut high = f32::NEG_INFINITY;
        let mut crossings = 0;
        for index in 0..4 {
            let a = quad[index];
            let b = quad[(index + 1) % 4];
            // Half-open on the upper end, so a vertex exactly on the row counts
            // once rather than twice and a horizontal edge never appears to
            // cross it at all.
            if (a.y <= y) == (b.y <= y) {
                continue;
            }
            let t = (y - a.y) / (b.y - a.y);
            let x = a.x + t * (b.x - a.x);
            if x < low {
                low = x;
            }
            if x > high {
                high = x;
            }
            crossings += 1;
        }
        if crossings < 2 {
            return None;
        }
        Some((low, high))
    }

    /// Fills a projected quad with dots, at a density set by its depth.
    ///
    /// The density is dithered rather than thresholded, which is what lets a
    /// face carry a gradient at all: one colour per cell means the *density*
    /// is the only continuous quantity available, and a hard threshold would
    /// put a visible edge between a face and its neighbour one step away.
    ///
    /// Dots are OR-ed into the shared field rather than written over it, and
    /// the depth is combined with a maximum. Together those two are the whole
    /// reason a filled cube comes out as one solid with visible planes rather
    /// than as whichever face was drawn last: the three visible faces are
    /// disjoint in depth, and the nearest one both wins the cell's colour and
    /// contributes the most dots.
    fn fill_face_dots(&mut self, quad: [Point2D; 4], depth: u8) {
        let density =
            MIN_FACE_DENSITY + (1.0 - MIN_FACE_DENSITY) * dequantise(depth);

        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &quad {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }

        let dot_width = self.field.faces.dot_width() as i32;
        let dot_height = self.field.faces.dot_height() as i32;
        // A dot at integer `n` is drawn over roughly `n - 0.5 .. n + 0.5`, so
        // the dot lattice samples at `n + 0.5` and a dot is inside the quad
        // when `n + 0.5` divided by the dots-per-cell lands inside it. Getting
        // this wrong is the same class of mistake as confusing cell and dot
        // coordinates anywhere else in this crate, and it fails quietly: the
        // fill is drawn, just clipped to a fraction of the face near the
        // top-left corner.
        let first_y = ((min_y * DOTS_Y as f32 - 0.5).ceil() as i32).max(0);
        let last_y =
            ((max_y * DOTS_Y as f32 - 0.5).floor() as i32).min(dot_height - 1);

        for dot_y in first_y..=last_y {
            let y = (dot_y as f32 + 0.5) / DOTS_Y as f32;
            let Some((low, high)) = Self::row_span(&quad, y) else {
                continue;
            };
            let first_x = ((low * DOTS_X as f32 - 0.5).ceil() as i32).max(0);
            let last_x =
                ((high * DOTS_X as f32 - 0.5).floor() as i32).min(dot_width - 1);
            for dot_x in first_x..=last_x {
                let dither = Dither::Bayer4.at(dot_x as usize, dot_y as usize);
                if density > dither {
                    self.field.raise_face_dot(
                        dot_x as usize,
                        dot_y as usize,
                        depth,
                    );
                }
            }
        }
    }

    /// Marks the cells a projected quad covers, at cell resolution.
    ///
    /// The `use_braille = false` path's face fill, and it dithers at *cell*
    /// granularity rather than not at all. Without the dither the ramp glyph
    /// alone is the only carrier of depth, and a five-step ramp between `░` and
    /// `█` gives a near face as one solid block of `█` and a far face as
    /// nothing at all -- two tones, no gradient, and a cube that reads as a
    /// black blob at some rotations. With it, a far face is a sparse field of
    /// `░` and a near one is dense `█`, and the two-tone field has a gradient
    /// across it that the glyph ramp alone could not express.
    fn fill_face_cells(&mut self, quad: [Point2D; 4], depth: u8) {
        let density =
            MIN_FACE_DENSITY + (1.0 - MIN_FACE_DENSITY) * dequantise(depth);
        let width = self.field.faces.width() as i32;
        let height = self.field.faces.height() as i32;

        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &quad {
            min_y = min_y.min(p.y);
            max_y = max_y.max(p.y);
        }
        let first_y = ((min_y - 0.5).ceil() as i32).max(0);
        let last_y = ((max_y - 0.5).floor() as i32).min(height - 1);

        for cell_y in first_y..=last_y {
            let y = cell_y as f32 + 0.5;
            let Some((low, high)) = Self::row_span(&quad, y) else {
                continue;
            };
            let first_x = ((low - 0.5).ceil() as i32).max(0);
            let last_x = ((high - 0.5).floor() as i32).min(width - 1);
            for cell_x in first_x..=last_x {
                if density > Dither::Bayer4.at(cell_x as usize, cell_y as usize) {
                    self.field
                        .mark_face(cell_x as usize, cell_y as usize, depth);
                }
            }
        }
    }

    /// Rasterises a line between two projected points, in dot space.
    ///
    /// A plain Bresenham, as before. What changed is that every dot goes into
    /// the one shared field, so a line crossing another line keeps both, and a
    /// line arriving at a corner adds to what is already there.
    fn draw_edge_dots(&mut self, from: Point2D, to: Point2D, depth: u8) {
        // Scale to braille resolution (2x4 dots per cell)
        let mut x0 = (from.x * DOTS_X as f32) as i32;
        let mut y0 = (from.y * DOTS_Y as f32) as i32;
        let x1 = (to.x * DOTS_X as f32) as i32;
        let y1 = (to.y * DOTS_Y as f32) as i32;

        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            if x0 >= 0 && y0 >= 0 {
                self.field.raise_edge_dot(x0 as usize, y0 as usize, depth);
            }

            if x0 == x1 && y0 == y1 {
                break;
            }

            let e2 = 2 * err;
            if e2 >= dy {
                if x0 == x1 {
                    break;
                }
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                if y0 == y1 {
                    break;
                }
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Rasterises a line between two projected points, at cell resolution.
    fn draw_edge_cells(&mut self, from: Point2D, to: Point2D, depth: u8) {
        let width = self.field.faces.width() as i32;
        let height = self.field.faces.height() as i32;

        let mut x0 = from.x as i32;
        let mut y0 = from.y as i32;
        let x1 = to.x as i32;
        let y1 = to.y as i32;

        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            if x0 >= 0 && x0 < width && y0 >= 0 && y0 < height {
                self.field.mark_edge_cell(x0 as usize, y0 as usize, depth);
            }

            if x0 == x1 && y0 == y1 {
                break;
            }

            let e2 = 2 * err;
            if e2 >= dy {
                if x0 == x1 {
                    break;
                }
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                if y0 == y1 {
                    break;
                }
                err += dx;
                y0 += sy;
            }
        }
    }

    /// Draws one frame into the field.
    fn render(&mut self) {
        self.field.clear();

        // Read the frame's scalars out first. Every draw helper takes
        // `&mut self` for the field, so holding a borrow of `self.vertices` or
        // `self.rotated` across a call is a conflict, and copying eight
        // `f32`s is the cheapest way out of one.
        let scale = self.projection_scale();
        let size = self.screen_size;
        let distance = self.effective_distance();
        let rotation = self.rotation;
        let braille = self.options.use_braille;
        let filled = self.options.filled;
        let markers = self.options.vertex_markers;

        for index in 0..self.vertices.len() {
            self.rotated[index] =
                Self::rotate_point(rotation, self.vertices[index]);
        }
        for index in 0..self.rotated.len() {
            self.projected[index] =
                Self::project(self.rotated[index], scale, size, distance);
        }

        // Which faces are turned towards the viewer, decided before anything is
        // drawn because two later steps need it: the range the ramp is
        // normalised against, and the corner markers.
        //
        // It is *not* a cull list. The far side of the cube is drawn too --
        // the effect is an X-ray wireframe, so a cube is twelve lines and not
        // nine, and the three lines behind the solid are dimmer rather than
        // absent. See [`FAR_SIDE_BAND`].
        //
        // No sort. Painter's algorithm would order the faces by depth and let
        // each overwrite the last, which needs the faces to be disjoint; they
        // are, but so is a maximum on depth, and a maximum does not care what
        // order the faces arrive in -- which matters here because the braille
        // path cannot overwrite at all. A braille cell is one glyph, so its
        // dots only OR together, and OR is commutative.
        let mut visible = [false; FACES.len()];
        for (index, face) in FACES.iter().enumerate() {
            visible[index] = self.face_is_front_facing(face);
        }
        let range = self.face_depth_range(&visible);

        // One depth per face, and the whole depth model hangs off it. An edge
        // takes the depth of the *nearest* face it bounds rather than the mean
        // of its own endpoints, and a corner marker the depth of the nearest
        // face meeting there.
        //
        // That is a deliberate departure from "shade by `z`", and it is what
        // makes the cue legible. An edge runs from one vertex to another, and
        // the two are at very different `z` -- a cube seen face-on has an edge
        // from the nearest vertex to the farthest, so the mean of its endpoints
        // is dead centre and the edge is drawn as though it were halfway back
        // into the cube. The edge does not *belong* to a depth; it belongs to
        // the surfaces that meet along it, and there are only two of them.
        //
        // Normalising over the visible faces' planes is the other half of it.
        // Over the whole vertex range the three visible faces never separate:
        // a face is visible only when its outward normal has `n.z <
        // -size / distance`, which for the defaults is `n.z < -0.29`, so a
        // face's centroid is never nearer than `z = -0.29 * size` against a
        // nearest vertex at `-1.73 * size`. The whole visible cube lives in the
        // near 42% of the ramp and the dark end of every stop is dead on
        // screen. Over the faces' own planes, three visible faces land at 0, a
        // half and 1 -- the full ramp, at every rotation.
        //
        // Every face gets a depth, not only the front-facing ones, and that is
        // what lets the far side be drawn: an edge takes the `max` over the two
        // it is shared by, so a far edge with two turned-away faces still has
        // a depth to be drawn at. It lands below every front-facing face
        // because the range reaches past them -- see
        // [`face_depth_range`](Self::face_depth_range) -- so no edge has to be
        // told apart from any other by a flag of its own.
        let mut face_depth = [NO_DEPTH; FACES.len()];
        for (index, face) in FACES.iter().enumerate() {
            face_depth[index] =
                quantise(self.depth_of(self.face_centroid_z(face), range));
        }

        for (index, face) in FACES.iter().enumerate() {
            if !visible[index] || !filled {
                continue;
            }
            let corners = face.corners;
            let quad = [
                self.projected[corners[0]],
                self.projected[corners[1]],
                self.projected[corners[2]],
                self.projected[corners[3]],
            ];
            if braille {
                self.fill_face_dots(quad, face_depth[index]);
            } else {
                self.fill_face_cells(quad, face_depth[index]);
            }
        }

        // All twelve edges, always.
        //
        // This is the change: the far side is drawn, not culled, and the only
        // thing telling it apart from the near side is how deep it is. The
        // depth is the `max` over the two faces the edge is shared by, which
        // for a far edge is the nearer of two turned-away planes and so lands
        // in the band [`FAR_SIDE_BAND`] reserves.
        //
        // Collected first so the loop below is free to mutate the field.
        let drawn: Vec<(Point2D, Point2D, u8)> = EDGES
            .iter()
            .map(|edge| {
                let mut depth = NO_DEPTH;
                for face in edge.faces {
                    depth = depth.max(face_depth[face]);
                }
                (edge, depth)
            })
            .map(|(edge, depth)| {
                (self.projected[edge.v1], self.projected[edge.v2], depth)
            })
            .collect();
        for (from, to, depth) in drawn {
            if braille {
                self.draw_edge_dots(from, to, depth);
            } else {
                self.draw_edge_cells(from, to, depth);
            }
        }

        if !markers {
            return;
        }
        let width = self.field.faces.width();
        let height = self.field.faces.height();
        for index in 0..self.projected.len() {
            // A corner is marked only if a *front-facing* face meets at it,
            // which is a narrower question than whether anything was drawn
            // there -- now all twelve edges are, so everything is. A cube
            // hides one corner from any viewpoint, and the three edges at that
            // corner are behind the solid; marking it would put a white
            // diamond in the middle of the near face at a face-on rotation,
            // which reads as a hole rather than as a vertex. The far corner is
            // not lost by that: the three lines meeting at it are drawn, and
            // they are where the X-ray read comes from.
            //
            // The marker is also drawn in one flat white rather than at its own
            // depth -- see [`VERTEX_COLOUR`] -- so marking a far corner would
            // make it the *brightest* thing on screen while the edges around it
            // recede, which is exactly backwards.
            let mut depth = NO_DEPTH;
            for edge in EDGES.iter() {
                if edge.v1 != index && edge.v2 != index {
                    continue;
                }
                for face in edge.faces {
                    if !visible[face] {
                        continue;
                    }
                    depth = depth.max(face_depth[face]);
                }
            }
            if depth == NO_DEPTH {
                continue;
            }

            let point = self.projected[index];
            let (dot_x, dot_y) = (
                (point.x * DOTS_X as f32) as i32,
                (point.y * DOTS_Y as f32) as i32,
            );
            if dot_x < 0 || dot_y < 0 {
                continue;
            }
            let (cell_x, cell_y) =
                (dot_x as usize / DOTS_X, dot_y as usize / DOTS_Y);
            if cell_x >= width || cell_y >= height {
                continue;
            }
            self.field.mark_vertex(cell_x, cell_y, depth);
        }
    }

    /// The rotated `z` of a face's centroid.
    fn face_centroid_z(&self, face: &Face) -> f32 {
        face.corners.iter().map(|c| self.rotated[*c].z).sum::<f32>() * 0.25
    }

    /// Writes the field out as cells.
    ///
    /// An associated function rather than a method because it reads five of the
    /// struct's fields and writes the sixth, and a `&mut self` would be one
    /// borrow chain that has to be split anyway.
    fn flush(
        field: &Field,
        options: &CubeOptions,
        ramp: &GlyphRamp,
        face_palette: &Palette,
        edge_palette: &Palette,
        canvas: &mut Canvas,
    ) {
        if field.dirty.is_empty() {
            return;
        }
        let rect = field.dirty;
        let width = field.faces.width();
        let (canvas_width, canvas_height) = (canvas.width(), canvas.height());
        let surface = canvas.surface_mut();

        for y in rect.top..rect.bottom.min(canvas_height) {
            for x in rect.left..rect.right.min(canvas_width) {
                let index = y * width + x;

                // The marker first. It overwrites the cell rather than adding
                // to it, which is the point: `◆` has to be a solid diamond to
                // read as a corner, and a braille pattern with a diamond in the
                // middle of it reads as a smudge. What is lost is the nearest
                // ring of dots around the corner, which the three edges
                // meeting there immediately redraw in the neighbouring cells.
                if field.vertex_depth[index] != NO_DEPTH {
                    surface.set(
                        x,
                        y,
                        Cell::new(VERTEX_GLYPH, VERTEX_COLOUR, Attribute::Reset),
                    );
                    continue;
                }

                if options.use_braille {
                    let edge_bits = field.edges.cell_bits(x, y);
                    let bits = field.faces.cell_bits(x, y) | edge_bits;
                    if bits == 0 {
                        continue;
                    }
                    let (depth, palette) = if edge_bits != 0 {
                        (field.edge_depth[index], edge_palette)
                    } else {
                        (field.face_depth[index], face_palette)
                    };
                    let Some(symbol) = char::from_u32(0x2800 + bits as u32) else {
                        continue;
                    };
                    // `Attribute::Reset`, not `Attribute::Bold`. Every cell was
                    // bold before, and bold on a truecolor foreground is a
                    // rendering hint that many terminals act on by brightening
                    // the colour -- which corrupts a ramp whose entire job is
                    // precise brightness. The cube had one colour and so had no
                    // ramp to spoil, which is exactly why the bug stayed
                    // invisible until the ramp arrived.
                    surface.set(
                        x,
                        y,
                        Cell::new(
                            symbol,
                            palette.sample(dequantise(depth)),
                            Attribute::Reset,
                        ),
                    );
                    continue;
                }

                let (symbol, depth, palette) =
                    if field.edge_depth[index] != NO_DEPTH {
                        (ASCII_EDGE_GLYPH, field.edge_depth[index], edge_palette)
                    } else if field.face_depth[index] != NO_DEPTH {
                        (
                            ramp.sample(dequantise(field.face_depth[index])),
                            field.face_depth[index],
                            face_palette,
                        )
                    } else {
                        continue;
                    };
                surface.set(
                    x,
                    y,
                    Cell::new(
                        symbol,
                        palette.sample(dequantise(depth)),
                        Attribute::Reset,
                    ),
                );
            }
        }
    }
}

impl CubeOptions {
    /// [`Cube::effective_cube_size`], as a method on the options, so
    /// `Cube::new` can ask without an instance to hand.
    fn effective_cube_size(&self) -> f32 {
        let size = finite_or(self.cube_size, DEFAULT_CUBE_SIZE);
        if size <= 0.0 {
            return DEFAULT_CUBE_SIZE;
        }
        let distance = finite_or(self.distance, DEFAULT_DISTANCE);
        if !distance.is_finite() || distance <= 0.0 {
            return size;
        }
        size.min(
            MAX_CAMERA_FILL * distance.clamp(MIN_DISTANCE, MAX_DISTANCE)
                / 3.0f32.sqrt(),
        )
    }
}

impl Cube {
    /// The projected position of a vertex, at the current rotation and scale.
    ///
    /// The same two steps `render` performs, wrapped so the tests do not have
    /// to thread the scale and the camera distance through every assertion --
    /// and so a test cannot quietly use a *different* scale from the one the
    /// frame was drawn with, which would make every geometry assertion here a
    /// measurement of the test rather than of the effect.
    #[cfg(test)]
    fn project_vertex(&self, vertex: Point3D) -> Point2D {
        Self::project(
            Self::rotate_point(self.rotation, vertex),
            self.projection_scale(),
            self.screen_size,
            self.effective_distance(),
        )
    }

    /// `(depth, relative luminance)` for every cell the last frame wrote that
    /// came from a face or an edge.
    ///
    /// Corner markers are deliberately left out. They are drawn in one
    /// constant colour, so including them makes the extreme depths of any frame
    /// read as the same luminance and the measurement stops being about the
    /// ramp -- the extremes are *always* markers, since a marker takes the
    /// depth of the nearest face meeting at its corner, which is by definition
    /// the extreme. `the_corner_markers_are_white` covers them separately.
    ///
    /// The measure the depth shading has to move, and it is a *pairing* rather
    /// than two separate counts. A frame with a dozen distinct colours proves
    /// nothing about whether they are ordered by distance, and counting
    /// colours is exactly the assertion that would have passed against any
    /// confetti -- it is the pairing that says the ramp is doing a job.
    #[cfg(test)]
    fn depth_and_luminance(&self) -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        if self.field.dirty.is_empty() {
            return out;
        }
        let rect = self.field.dirty;
        let width = self.field.faces.width();

        for y in rect.top..rect.bottom {
            for x in rect.left..rect.right {
                let index = y * width + x;
                if self.field.vertex_depth[index] != NO_DEPTH {
                    continue;
                }
                if self.options.use_braille {
                    if self.field.edges.cell_bits(x, y) == 0
                        && self.field.faces.cell_bits(x, y) == 0
                    {
                        continue;
                    }
                    let from_edge = self.field.edges.cell_bits(x, y) != 0;
                    let (depth, palette) = if from_edge {
                        (self.field.edge_depth[index], &self.edge_palette)
                    } else {
                        (self.field.face_depth[index], &self.face_palette)
                    };
                    out.push((
                        dequantise(depth),
                        Self::luminance(palette.sample(dequantise(depth))),
                    ));
                } else if self.field.edge_depth[index] != NO_DEPTH {
                    out.push((
                        dequantise(self.field.edge_depth[index]),
                        Self::luminance(
                            self.edge_palette
                                .sample(dequantise(self.field.edge_depth[index])),
                        ),
                    ));
                } else if self.field.face_depth[index] != NO_DEPTH {
                    let depth = dequantise(self.field.face_depth[index]);
                    out.push((
                        depth,
                        Self::luminance(self.face_palette.sample(depth)),
                    ));
                }
            }
        }
        out
    }

    /// Rec. 601 luminance, 0 to 255.
    #[cfg(test)]
    fn luminance(colour: Color) -> f32 {
        match colour {
            Color::Rgb { r, g, b } => {
                0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
            }
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A rotation at which all six faces are turned away from edge-on.
    ///
    /// Most of the tests here want a generic rotation, because at one of the
    /// special ones the cube is face-on or edge-on and half the geometry
    /// degenerates. Axis-aligned rotations are the measure-zero case that the
    /// epsilon in `face_is_front_facing` exists for, so they are not evidence of
    /// anything.
    const GENERIC: (f32, f32, f32) = (0.41, 0.73, 0.19);

    fn placed(cube_size: f32, size: (u16, u16)) -> Cube {
        let options = CubeOptions {
            cube_size,
            ..Default::default()
        };
        Cube::new(options, size)
    }

    /// `placed`, with the faces filled.
    ///
    /// The tests that are *about* the fill have to ask for it now that the
    /// default is the wireframe, and that is the point of the default having
    /// moved rather than a nuisance: a test that inherited `filled: true` from
    /// the default was measuring a picture nobody sees. Naming it in the helper
    /// also means the tests that want the default look -- the geometry ones, the
    /// markers, the glyph widths -- keep inheriting whatever the default is, so
    /// a future change to it is exercised rather than frozen here.
    fn filled(cube_size: f32, size: (u16, u16)) -> Cube {
        Cube::new(
            CubeOptions {
                cube_size,
                filled: true,
                ..Default::default()
            },
            size,
        )
    }

    /// Runs one frame at a rotation, returning the first frame's full diff.
    fn frame(
        cube: &mut Cube,
        rotation: (f32, f32, f32),
    ) -> Vec<(usize, usize, Cell)> {
        cube.rotation = rotation;
        cube.get_diff()
    }

    /// The dot pattern of each cell a first-frame diff contains.
    fn dot_patterns(
        diff: &[(usize, usize, Cell)],
    ) -> std::collections::HashMap<(usize, usize), u8> {
        diff.iter()
            .filter(|(_, _, cell)| cell.symbol != ' ')
            .map(|(x, y, cell)| {
                let bits = (cell.symbol as u32).saturating_sub(0x2800) as u8;
                ((*x, *y), bits)
            })
            .collect()
    }

    /// The dots one edge raises, computed independently of the renderer.
    ///
    /// A reference rather than a call into the field, because the property
    /// under test *is* that the field is the union of the edges: reading the
    /// answer out of the field and comparing it with itself would pass against
    /// any implementation, including the one that overwrites corners.
    fn edge_dots(
        projected: &[Point2D],
        edge: &Edge,
        size: (u16, u16),
    ) -> HashSet<(i32, i32, u8)> {
        let mut x0 = (projected[edge.v1].x * DOTS_X as f32) as i32;
        let mut y0 = (projected[edge.v1].y * DOTS_Y as f32) as i32;
        let x1 = (projected[edge.v2].x * DOTS_X as f32) as i32;
        let y1 = (projected[edge.v2].y * DOTS_Y as f32) as i32;

        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        let width = size.0 as i32 * DOTS_X as i32;
        let height = size.1 as i32 * DOTS_Y as i32;
        let mut out = HashSet::new();
        loop {
            if x0 >= 0 && x0 < width && y0 >= 0 && y0 < height {
                out.insert((
                    x0 / DOTS_X as i32,
                    y0 / DOTS_Y as i32,
                    1u8 << ((y0 as usize % DOTS_Y) * DOTS_X
                        + (x0 as usize % DOTS_X)),
                ));
            }
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                if x0 == x1 {
                    break;
                }
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                if y0 == y1 {
                    break;
                }
                err += dx;
                y0 += sy;
            }
        }
        out
    }

    /// The cell a projected point falls in, in the renderer's own arithmetic.
    fn cell_of(point: Point2D) -> (usize, usize) {
        let dot_x = (point.x * DOTS_X as f32) as i32;
        let dot_y = (point.y * DOTS_Y as f32) as i32;
        (
            dot_x.max(0) as usize / DOTS_X,
            dot_y.max(0) as usize / DOTS_Y,
        )
    }

    /// The luminance each edge's own cells were drawn in, tagged with which
    /// side of the cube the edge is on.
    ///
    /// Only cells no *other* edge reaches. A cell two edges both cross carries
    /// one colour for both of them, and reading it as either edge's would be
    /// measuring the pair rather than the edge. The three far edges all meet at
    /// the far corner, so each of them gives up a handful of cells there and
    /// keeps the rest; the corner markers are skipped for the same reason, being
    /// a flat white that says nothing about the edge underneath.
    ///
    /// An edge is drawn at one depth and so in one colour, which is asserted
    /// here rather than assumed: a cell whose depth is a `max` over every edge
    /// reaching it would show up as a spread, and a spread would mean the
    /// shading is per-cell rather than per-edge.
    ///
    /// Which side an edge is on comes from the *geometry* -- both the faces it
    /// is shared by turned away, or not -- and not from anything the renderer
    /// computed, so this is a measurement of the output rather than a
    /// restatement of the shading code. An edge with no usable cell of its own
    /// is dropped rather than guessed at.
    fn edge_luminances(
        cube: &Cube,
        diff: &[(usize, usize, Cell)],
    ) -> Vec<(bool, f32)> {
        let by_cell: HashMap<(usize, usize), Cell> =
            diff.iter().map(|(x, y, cell)| ((*x, *y), *cell)).collect();
        let projected: Vec<Point2D> = cube
            .vertices
            .iter()
            .map(|v| cube.project_vertex(*v))
            .collect();
        let per_edge: Vec<HashSet<(i32, i32)>> = EDGES
            .iter()
            .map(|edge| {
                edge_dots(&projected, edge, cube.screen_size)
                    .into_iter()
                    .map(|(x, y, _)| (x, y))
                    .collect()
            })
            .collect();
        let mut reachers: HashMap<(i32, i32), usize> = HashMap::new();
        for cells in &per_edge {
            for cell in cells {
                *reachers.entry(*cell).or_insert(0) += 1;
            }
        }
        let front: Vec<bool> = FACES
            .iter()
            .map(|face| cube.face_is_front_facing(face))
            .collect();

        let mut out = Vec::new();
        for (index, cells) in per_edge.iter().enumerate() {
            let edge = &EDGES[index];
            let far_side = !front[edge.faces[0]] && !front[edge.faces[1]];
            let mut lums: Vec<f32> = Vec::new();
            for (x, y) in cells {
                if *x < 0 || *y < 0 || reachers[&(*x, *y)] != 1 {
                    continue;
                }
                let Some(cell) = by_cell.get(&(*x as usize, *y as usize)) else {
                    continue;
                };
                if cell.symbol == VERTEX_GLYPH {
                    continue;
                }
                lums.push(Cube::luminance(cell.color));
            }
            if lums.is_empty() {
                continue;
            }
            let low = lums.iter().copied().fold(f32::INFINITY, f32::min);
            let high = lums.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert_eq!(
                low,
                high,
                "edge {} of EDGES was drawn across {:.1} of luminance at one \
                 rotation, so it is not one colour",
                index,
                high - low
            );
            out.push((far_side, (low + high) * 0.5));
        }
        out
    }

    /// The corners have to carry the dots of *every* edge that meets there.
    ///
    /// The old renderer built a fresh dot map per edge and then `set` the whole
    /// cell, so a corner cell -- the endpoint of three edges -- was written
    /// three times and only the last write survived. All eight corners came out
    /// as the end-stub of whichever edge was drawn last, which is why a solid
    /// read as a flat wireframe: the one part of a cube that says "cube" was
    /// being thrown away, once per corner, sixty times a second.
    ///
    /// The reference is computed here rather than read out of the field, so
    /// this is a statement about the geometry and not a tautology.
    ///
    /// All three incident edges, not only the ones a front-facing face reaches.
    /// That used to be a filter here, and it was a filter because three of the
    /// twelve were not drawn at all; now all twelve are, and a corner is the
    /// union of all three of its edges whether they are in front of the solid or
    /// behind it. Dropping the filter makes the assertion strictly harder --
    /// a bigger union to have lost something from.
    ///
    /// `vertex_markers` and `filled` are both off, and necessarily so. The
    /// marker deliberately occupies the corner cell and the face fill adds its
    /// own dots to it, so in the default configuration there is nothing in a
    /// corner cell but the marker to inspect.
    #[test]
    fn every_corner_carries_the_union_of_its_edges() {
        let options = CubeOptions {
            vertex_markers: false,
            filled: false,
            ..Default::default()
        };
        let mut cube = Cube::new(options, (200, 50));
        let diff = frame(&mut cube, GENERIC);
        let drawn = dot_patterns(&diff);

        // The projection the renderer used, at the scale it chose.
        let projected: Vec<Point2D> = cube
            .vertices
            .iter()
            .map(|v| cube.project_vertex(*v))
            .collect();

        let mut corners_checked = 0;
        let mut full_junctions = 0;
        for vertex in 0..projected.len() {
            let cell = cell_of(projected[vertex]);
            let incident: Vec<&Edge> = EDGES
                .iter()
                .filter(|edge| edge.v1 == vertex || edge.v2 == vertex)
                .collect();
            if incident.len() == 3 {
                full_junctions += 1;
            }
            // Every corner of a cube has exactly three edges meeting at it, and
            // all three are drawn, so there is no corner here with fewer than
            // two edges to merge.
            assert_eq!(
                incident.len(),
                3,
                "vertex {vertex} has {} edges, which is not a cube",
                incident.len()
            );

            let mut union = 0u8;
            let mut per_edge = Vec::new();
            for edge in &incident {
                let mut bits = 0u8;
                for (cx, cy, bit) in edge_dots(&projected, edge, cube.screen_size) {
                    if (cx, cy) == (cell.0 as i32, cell.1 as i32) {
                        bits |= bit;
                    }
                }
                per_edge.push(bits);
                union |= bits;
            }

            if union == 0 {
                // The corner is off screen at this rotation. Not a failure of
                // the merge, and the fit test covers the fit.
                continue;
            }
            if !per_edge
                .iter()
                .any(|bits| bits.count_ones() < union.count_ones())
            {
                // The three edges happen to raise the same dots in this cell,
                // so there is nothing here for an overwrite to lose. Happens at
                // a silhouette corner, where two edges leave within a couple of
                // dots of each other.
                continue;
            }
            let got = drawn.get(&cell).copied().unwrap_or(0);
            // A superset rather than an equality: a fourth line can cross a
            // corner cell without meeting at the corner -- one of the far edges
            // passes through the middle of the frame at a generic rotation --
            // and those dots are legitimately there too. What must hold is that
            // nothing from the incident edges was *lost*, which is exactly what
            // the whole-cell overwrite broke.
            assert_eq!(
                got & union,
                union,
                "vertex {vertex} at {cell:?}: its {} edges raise {union:#010b} \
                 into the cell and the cell was drawn {got:#010b}, so {:#010b} \
                 was overwritten away",
                incident.len(),
                union & !got
            );
            corners_checked += 1;
        }
        assert_eq!(
            full_junctions, 8,
            "a cube has three edges at each of its eight corners"
        );
        assert!(
            corners_checked >= 6,
            "only {corners_checked} corners were on screen, so the merge \
             is barely tested"
        );
    }

    /// The cube is fitted to the terminal, so no part of it is ever off screen.
    ///
    /// The old scale was `min(width, height) * 0.8`, a constant with nothing to
    /// do with the cube or the terminal's shape. At the default `cube_size`
    /// that happened to leave the cube inside 80x24 and 200x50 -- the fudge was
    /// comfortable there -- which is exactly why it survived. `cube_size` is a
    /// public config field and 1.4 is enough to push a vertex past the last
    /// row at 80x24, at which point the rasterisers' bounds check silently
    /// drops the overflow and the cube loses its top and bottom corners.
    ///
    /// Measured from that case, not asserted from theory: at cube_size 1.4 and
    /// 80x24 the worst-case projected radius is 1.03 of the half-height against
    /// the 0.80 the old constant produced, and the first vertex goes off screen
    /// around t = 0.44 seconds of rotation.
    #[test]
    fn the_projection_is_fitted_to_the_screen() {
        for size in [
            (80u16, 24u16),
            (200, 50),
            (12, 6),
            (6, 40),
            (400, 200),
            (3, 3),
            (1, 1),
        ] {
            for cube_size in [1.0f32, 1.4, 1.8, 2.5, 8.0] {
                let mut cube = placed(cube_size, size);
                for step in 0..90 {
                    let t = step as f32 * 0.11;
                    cube.rotation = (t * 0.25, t * 0.35, t * 0.18);
                    for (index, vertex) in cube.vertices.iter().enumerate() {
                        let p = cube.project_vertex(*vertex);
                        assert!(
                            p.x >= 0.0
                                && p.y >= 0.0
                                && p.x < size.0 as f32
                                && p.y < size.1 as f32,
                            "{}x{} at cube_size {cube_size} put vertex {index} at \
                             ({:.2}, {:.2}), off screen, at t = {t:.2}",
                            size.0,
                            size.1,
                            p.x,
                            p.y
                        );
                    }
                }
            }
        }
    }

    /// And it is a *fit* rather than a fudge: the cube uses the terminal.
    ///
    /// The measure is the vertical extent of the projected cube as a fraction
    /// of the half-height, which is the limiting axis at every wide terminal.
    /// The old `0.8 * 0.8` put it at 0.73, which is a cube floating in the
    /// middle of the screen with a fifth of it unused top and bottom. The old
    /// code's 0.73 was measured, not guessed, and the fitted value is 0.95 by
    /// construction -- so the threshold sits between them and is about the fit
    /// rather than about any particular constant.
    #[test]
    fn the_cube_fills_the_limiting_half_axis() {
        for size in [(80u16, 24u16), (200, 50), (400, 200)] {
            let mut cube = placed(1.0, size);
            let half_height = size.1 as f32 * 0.5;
            let mut extent = 0.0f32;
            for step in 0..300 {
                let t = step as f32 * 0.021;
                cube.rotation = (t, t * 1.4, t * 0.7);
                for vertex in cube.vertices.iter() {
                    let p = cube.project_vertex(*vertex);
                    extent = extent.max((p.y - half_height).abs());
                }
            }
            let fraction = extent / half_height;
            assert!(
                fraction > 0.90,
                "{}x{}: the cube reaches only {:.0}% of the way to the top and \
                 bottom of the screen, against 73% for the old fixed 0.8",
                size.0,
                size.1,
                fraction * 100.0
            );
        }
    }

    /// Nearer geometry is brighter than farther geometry, measured on
    /// luminance.
    ///
    /// The single highest-impact change available to this effect, and the one
    /// whose absence is why it read as flat: a single `Color::Green` carries no
    /// depth cue at all, and foreshortening cannot supply one for a shape that
    /// is symmetric in all three axes.
    ///
    /// Measured as a *pairing* -- depth against luminance for the same cell --
    /// rather than as a count of distinct colours. A frame with a dozen
    /// distinct colours proves nothing about whether they are ordered by
    /// distance, and a count is exactly the assertion that would have passed
    /// against any confetti.
    ///
    /// Swept rather than measured at one rotation, and the sweep is not
    /// decoration. At a rotation where the cube happens to be edge-on, all three
    /// visible faces are at nearly the same depth and a two-band test is
    /// measuring nothing: at (0.41, 0.73, 0.19) the three face depths come out
    /// 1.00, 0.96 and 0.00, with 319 of 322 drawn cells in the top decile. That
    /// is geometrically honest -- those two faces really are the same distance
    /// from the eye and really are near-mirror images across the view axis --
    /// but it means any single-rotation threshold is a threshold on the
    /// rotation. Aggregating over a sweep is the only version of this that
    /// measures the ramp rather than the cube's pose.
    ///
    /// `filled = true`, because this is measuring what the *face* ramp does.
    /// With the wireframe the faces contribute nothing and the near/far split
    /// would be carried entirely by the edge ramp, which is a different claim
    /// about a different ramp.
    #[test]
    fn nearer_geometry_is_brighter_than_farther_geometry() {
        let mut near = Vec::new();
        let mut far = Vec::new();
        let mut widest_spread = 0.0f32;
        let mut widest_contrast = 0.0f32;
        let mut measured = 0usize;

        for step in 0..150 {
            let t = step as f32 * 0.041;
            let mut cube = filled(1.0, (80, 24));
            frame(&mut cube, (t, t * 1.4, t * 0.7));

            let pairs = cube.depth_and_luminance();
            measured += pairs.len();
            for (depth, luminance) in &pairs {
                if *depth >= 0.8 {
                    near.push(*luminance);
                } else if *depth <= 0.2 {
                    far.push(*luminance);
                }
            }
            if pairs.is_empty() {
                continue;
            }

            // The frame with the widest depth spread is the one where the ramp
            // has the most to say, so that is where its contrast is read.
            let lo = pairs.iter().map(|(d, _)| *d).fold(1.0f32, f32::min);
            let hi = pairs.iter().map(|(d, _)| *d).fold(0.0f32, f32::max);
            if hi - lo <= widest_spread {
                continue;
            }
            let at = |target: f32| {
                pairs
                    .iter()
                    .filter(|(d, _)| (*d - target).abs() < 0.05)
                    .map(|(_, l)| *l)
                    .next()
                    .unwrap_or(f32::NAN)
            };
            let contrast = at(hi) - at(lo);
            if contrast.is_finite() {
                widest_spread = hi - lo;
                widest_contrast = contrast;
            }
        }

        assert!(
            measured > 20_000,
            "the sweep only produced {measured} cells to measure"
        );
        let mean =
            |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
        assert!(
            near.len() > 200 && far.len() > 200,
            "the sweep produced {} near cells and {} far cells, so the ramp is \
             not being exercised across its range",
            near.len(),
            far.len()
        );
        let (near_mean, far_mean) = (mean(&near), mean(&far));
        assert!(
            near_mean > far_mean + 40.0,
            "cells in the nearest fifth average {near_mean:.1} and cells in the \
             furthest average {far_mean:.1}, so distance does not order the \
             brightness",
        );

        // And at least one frame has to use the whole ramp, or the means above
        // are being carried by a rare rotation rather than by the shading.
        assert!(
            widest_spread > 0.8,
            "the widest depth spread in the sweep was {widest_spread:.2}, so the \
             ramp is never used end to end"
        );
        assert!(
            widest_contrast > 40.0,
            "at the frame with the widest spread the two ends of the ramp differ \
             by {widest_contrast:.1} in luminance, so the ramp is not steep \
             enough to read"
        );
    }

    /// A face-on rotation fills the near face instead of outlining it.
    ///
    /// Turned so the cube is square to the viewer, the near face's square is
    /// the whole silhouette and the far face is entirely inside it. The old
    /// effect drew only the twelve edges, so the interior of the silhouette was
    /// whatever the *hidden* far face's outline happened to leave there: 240
    /// raised dots at 200x50, all of them back-face line. A filled face is
    /// thousands, because the whole area carries dots at the near face's
    /// density.
    ///
    /// The number in that paragraph is no longer what the wireframe leaves there,
    /// and it is worth knowing why. The X-ray draws the far face's outline too,
    /// and it is inside the near one, so the interior of the silhouette now
    /// carries the far square's four edges rather than nothing -- the same
    /// region, and about the same 240 dots, but they are *drawn on purpose* now
    /// rather than showing through. The threshold below is 1000 and the wireframe
    /// is far under it, so the comparison this test exists for still holds; what
    /// changed is the reason the default is the wireframe, which is no longer
    /// "the fill is the ugly one" so much as "the fill buries the silhouette".
    ///
    /// `filled = true` is asked for rather than inherited, because the default is
    /// the wireframe and this test is about the option rather than about the
    /// default. `the_default_is_the_wireframe_rather_than_the_filled_cube` is the
    /// one that measures which of the two a user actually gets.
    #[test]
    fn a_face_on_rotation_fills_the_near_face() {
        let size = (200u16, 50u16);
        let mut cube = filled(1.0, size);
        let diff = frame(&mut cube, (0.0, 0.0, 0.0));
        let drawn = dot_patterns(&diff);

        // The near face is z = -cube_size, and the projection draws the
        // smaller z larger, so its square is the silhouette.
        let corner = cube.project_vertex(cube.vertices[0]);
        let half_x = (size.0 as f32 / 2.0 - corner.x).abs();
        let half_y = (size.1 as f32 / 2.0 - corner.y).abs();

        let mut interior_dots = 0usize;
        for ((x, y), bits) in &drawn {
            let dx = (*x as f32 + 0.5 - size.0 as f32 / 2.0).abs();
            let dy = (*y as f32 + 0.5 - size.1 as f32 / 2.0).abs();
            if dx < half_x - 1.5 && dy < half_y - 1.5 {
                interior_dots += bits.count_ones() as usize;
            }
        }
        assert!(
            interior_dots > 1000,
            "only {interior_dots} dots inside the near face at a face-on \
             rotation, against 240 for the unfilled wireframe, so the face is \
             not filled"
        );
    }

    /// The drawn edge ink is exactly the union of the twelve edges' dots.
    ///
    /// Stated as an exact equality rather than as "the far edges are there",
    /// because the looser form is not testable. The far edges project across
    /// cells that a near edge also crosses -- at a face-on rotation, the edge
    /// from the near bottom-left corner to the back bottom-left corner runs
    /// along the front face's own bottom edge -- so those cells carry edge ink
    /// from both, and a per-cell assertion would have to carve out exceptions
    /// until it tested nothing. Equality has no exceptions: the field must be
    /// precisely the twelve edges' union, which is one assertion covering
    /// over-drawing (the old bug, and now the way a leftover cull would show
    /// up) and under-drawing (the other way to get this wrong) at once.
    ///
    /// All twelve, and the count is asserted rather than assumed. The three far
    /// ones used to be culled here, which is what made this test the
    /// hidden-line-removal test; it is now the test that the far side is drawn,
    /// and it would catch a stray `filter` as surely as the old one caught its
    /// absence.
    ///
    /// The reference sets come from `edge_dots`, computed here.
    #[test]
    fn the_drawn_edge_ink_is_exactly_the_twelve_edges_union() {
        for rotation in [(0.0f32, 0.0f32, 0.0f32), GENERIC, (0.41, 0.9, 0.55)] {
            let mut cube = placed(1.0, (200, 50));
            frame(&mut cube, rotation);

            let projected: Vec<Point2D> = cube
                .vertices
                .iter()
                .map(|v| cube.project_vertex(*v))
                .collect();

            // What the union of the twelve edges should be, per cell.
            let mut expected: std::collections::HashMap<(usize, usize), u8> =
                std::collections::HashMap::new();
            let mut edges_with_ink = 0;
            for edge in EDGES.iter() {
                let mut raised = false;
                for (cx, cy, bit) in edge_dots(&projected, edge, cube.screen_size) {
                    if cx < 0 || cy < 0 {
                        continue;
                    }
                    raised = true;
                    *expected.entry((cx as usize, cy as usize)).or_insert(0) |= bit;
                }
                edges_with_ink += usize::from(raised);
            }
            assert_eq!(
                edges_with_ink, 12,
                "at {rotation:?} only {edges_with_ink} of the twelve edges put \
                 ink on the screen, so one is not being drawn"
            );

            let mut inked = 0;
            for y in 0..cube.field.edges.height() {
                for x in 0..cube.field.edges.width() {
                    let got = cube.field.edges.cell_bits(x, y);
                    let want = expected.get(&(x, y)).copied().unwrap_or(0);
                    if got != want {
                        let what = if got & !want == 0 {
                            "dots were drawn that no edge raises"
                        } else {
                            "dots from an edge were lost"
                        };
                        panic!(
                            "cell ({x}, {y}) at {rotation:?} has edge pattern {got:#010b}, the twelve edges put {want:#010b} there -- {what}"
                        );
                    }
                    inked += usize::from(got != 0);
                }
            }
            assert!(
                inked > 100,
                "at {rotation:?} only {inked} cells carry edge ink"
            );
        }
    }

    /// A face-on cube shows one face, and still draws all twelve edges.
    ///
    /// The rotation with the most to see through: the far face is entirely
    /// behind the near one, so eight of the twelve edges are behind the solid
    /// and this is where hidden-line removal was doing the most work -- and
    /// where it was most visibly wrong, since a culled far face at this pose is
    /// a cube with nothing behind it and no way to see it is turning.
    ///
    /// Two claims, because they are two different things and either can rot on
    /// its own. The *rule* still says one face is front-facing here, and that
    /// is asserted on the rule rather than on pixels, because the rule is what
    /// the depth range and the corner markers are built on and both still need
    /// it. The *output* then has to draw all twelve edges anyway.
    ///
    /// The bounds and the attribute checks are here because this is the pose
    /// that puts the most geometry in the least room: the far square is
    /// entirely inside the near one, so a dot written past the last row or
    /// column is at the far corner rather than out in empty space. Neither
    /// check is new in kind -- `both_renderers_survive_every_size` and
    /// `no_cell_is_bold` cover them generally -- but they are pinned here
    /// against *this* configuration, so that a change which made the far side
    /// come out bold, or write outside the canvas, could not pass on the
    /// strength of the other two tests.
    #[test]
    fn a_face_on_rotation_still_draws_all_twelve_edges() {
        let size = (200u16, 50u16);
        let mut cube = placed(1.0, size);
        let diff = frame(&mut cube, (0.0, 0.0, 0.0));

        // The rule, first: down one of its own axes, a cube shows one face.
        let faces: Vec<bool> = FACES
            .iter()
            .map(|face| cube.face_is_front_facing(face))
            .collect();
        assert_eq!(
            faces.iter().filter(|f| **f).count(),
            1,
            "a cube viewed down one of its axes shows exactly one face, but {} were treated as front-facing",
            faces.iter().filter(|f| **f).count()
        );
        assert!(faces[0], "and it is the near one, not a far face");

        // Eight of the twelve are behind the solid, and all eight are drawn.
        let far_side = EDGES
            .iter()
            .filter(|edge| !faces[edge.faces[0]] && !faces[edge.faces[1]])
            .count();
        assert_eq!(
            far_side, 8,
            "a cube viewed down one of its axes has eight edges behind the near \
             face, not {far_side}"
        );

        let projected: Vec<Point2D> = cube
            .vertices
            .iter()
            .map(|v| cube.project_vertex(*v))
            .collect();
        let mut with_ink = 0;
        for edge in EDGES.iter() {
            let on_screen = edge_dots(&projected, edge, cube.screen_size)
                .into_iter()
                .any(|(x, y, _)| x >= 0 && y >= 0);
            with_ink += usize::from(on_screen);
        }
        assert_eq!(
            with_ink, 12,
            "at a face-on rotation only {with_ink} of the twelve edges put ink on \
             the screen: the far face is {far_side} of them and all of it has to \
             be there, or the cube does not read as turning"
        );

        // Bold is a brightening hint on many terminals and this frame has more
        // colour in it than any other, so it is the place a stray `Bold` would
        // do the most damage.
        assert!(!diff.is_empty(), "the face-on frame drew nothing");
        for (x, y, cell) in &diff {
            assert_eq!(
                cell.attr,
                Attribute::Reset,
                "cell ({x}, {y}) is bold at a face-on rotation, which brightens \
                 the depth ramp into white on many terminals"
            );
            assert!(
                *x < size.0 as usize && *y < size.1 as usize,
                "cell ({x}, {y}) is outside a {size:?} screen"
            );
        }
    }

    /// A far edge is dimmer than the near ones, at the same rotation.
    ///
    /// The user's own brief, made into a measurement: "it should be a wireframe,
    /// like an X-ray. The other side lines should also be visible", with the far
    /// side asked for dimmer rather than identical so the cube still reads as a
    /// solid turning in space.
    ///
    /// Measured on the drawn frame, not on the depth array. `edge_luminances`
    /// reads the colours out of the diff, one per edge, using only cells that
    /// edge has to itself, and checks each edge's colour to be constant across
    /// its own cells -- so this cannot be satisfied by a cube that merely varies
    /// its brightness from cell to cell, which is the obvious way for a
    /// brightness test to pass against the thing it is about.
    ///
    /// Swept over 200 rotations rather than pinned to one, because the answer is
    /// a property of the shading and any single pose measures the pose. The
    /// statistic is the *worst* pair at each rotation -- the dimmest near-side
    /// edge against the brightest far-side edge, which is the pair that would
    /// fail first if the two sides could be confused. Measured: the tightest gap
    /// over the sweep is 4.9 (at t = 0.820), the median is 91.6, the 5th
    /// percentile is 42.3, and the best is the edge ramp's full 140.1.
    ///
    /// The minimum is asserted only to be positive, and that is a geometric
    /// fact rather than a loose threshold. It is the pose where a face is
    /// within a fraction of a degree of edge-on, and a turned-away face that is
    /// nearly edge-on really is at almost exactly the distance of a
    /// front-facing one that is nearly edge-on. Shading those two differently
    /// would be a lie about the geometry, so the number is asserted to stay
    /// small and the *typical* separation is asserted to be large.
    ///
    /// The last claim is the one the brief asks for by name: one ordinary
    /// rotation, one back edge against one front edge, by a measured amount. A
    /// sweep statistic is easy to satisfy by accident -- a ramp that is merely
    /// *usually* ordered still has a large median -- so the headline number is a
    /// single comparison with the figures written down, and it cannot be
    /// reached at all if the far side is culled, because then there is no far
    /// edge to measure.
    #[test]
    fn a_far_edge_is_dimmer_than_the_near_ones() {
        let mut gaps: Vec<(f32, (f32, f32, f32))> = Vec::new();
        let mut measured = 0usize;
        for step in 0..200 {
            let t = step as f32 * 0.041;
            let mut cube = placed(1.0, (200, 50));
            let diff = frame(&mut cube, (t, t * 1.4, t * 0.7));
            let sides = edge_luminances(&cube, &diff);
            let near: Vec<f32> = sides
                .iter()
                .filter(|(behind, _)| !*behind)
                .map(|(_, l)| *l)
                .collect();
            let far: Vec<f32> = sides
                .iter()
                .filter(|(behind, _)| *behind)
                .map(|(_, l)| *l)
                .collect();

            // Both sides are present at every pose, and how the twelve split is
            // the pose's business -- nine in front and three behind from a
            // general viewpoint, four and eight square to the viewer. An edge
            // on neither side would be a hole in the measurement, though.
            assert!(
                (4..=9).contains(&near.len()) && (3..=8).contains(&far.len()),
                "at t = {t:.3} the sweep found {} near-side and {} far-side \
                 edges, which is not a cube seen from outside",
                near.len(),
                far.len()
            );
            measured += near.len() + far.len();

            let dimmest_near = near.iter().copied().fold(f32::INFINITY, f32::min);
            let brightest_far =
                far.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            gaps.push((
                dimmest_near - brightest_far,
                (dimmest_near, brightest_far, t),
            ));
        }

        // Not all twelve edges are measurable at every pose, and the test says
        // so rather than quietly measuring fewer. A rotation close to edge-on
        // projects two edges onto the same line, and then neither has a cell of
        // its own. 2381 of the 2400 are, and the floor is well under that: what
        // it catches is a change that makes the exclusive cells rarer, which
        // would hollow out the measurement without any single assertion firing.
        assert!(
            measured >= 2300,
            "only {measured} of the 2400 edge measurements in the sweep found a \
             cell to themselves, so the rest were dropped and the gaps above are \
             over a shrinking sample"
        );

        let tightest = gaps
            .iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("the sweep produced rotations");
        assert!(
            tightest.0 > 0.0,
            "at t = {:.3} the brightest far-side edge is drawn at {:.1} against \
             the dimmest near-side edge's {:.1}, so at that pose the far side is \
             not dimmer at all",
            tightest.1.2,
            tightest.1.1,
            tightest.1.0
        );

        let mut sorted: Vec<f32> = gaps.iter().map(|(gap, _)| *gap).collect();
        sorted.sort_by(f32::total_cmp);
        let median = sorted[sorted.len() / 2];
        let fifth = sorted[sorted.len() * 5 / 100];
        assert!(
            median > 80.0,
            "the median gap between the two sides is {median:.1} of 255, so at \
             half of all rotations the far side is not clearly dimmer"
        );
        assert!(
            fifth > 30.0,
            "the 5th-percentile gap is {fifth:.1}, so more than one rotation in \
             twenty puts the far side within a tenth of the way to the near side"
        );

        // And the single comparison, at one ordinary rotation, with the numbers
        // on the record. GENERIC is the rotation the geometry tests use, chosen
        // because it is nowhere near a special pose: three faces turned towards
        // the viewer, well spread in depth, nine edges in front and three
        // behind. The worst pair there is the middle face's edges at 171.8
        // against the far side's darkest ramp stop at 114.9, and the front face's
        // own four edges are at 255 -- so the far side is 56.9 below the dimmest
        // front edge and 140.1 below the brightest one.
        let mut cube = placed(1.0, (200, 50));
        let diff = frame(&mut cube, GENERIC);
        let sides = edge_luminances(&cube, &diff);
        let dimmest_near = sides
            .iter()
            .filter(|(behind, _)| !*behind)
            .map(|(_, l)| *l)
            .fold(f32::INFINITY, f32::min);
        let brightest_far = sides
            .iter()
            .filter(|(behind, _)| *behind)
            .map(|(_, l)| *l)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            dimmest_near - brightest_far > 40.0,
            "at {GENERIC:?} the far side is drawn at {brightest_far:.1} against \
             the near side's {dimmest_near:.1}, which is not a visible difference"
        );
    }

    /// The corner markers are one flat colour, and it is white.
    ///
    /// Separate from `nearer_geometry_is_brighter_than_farther_geometry` because
    /// it is a different claim. The markers are deliberately not depth-shaded,
    /// and `Attribute::Bold` is the obvious way to make eight small glyphs stand
    /// out from the edge they sit on -- it is a brightening *hint* that many
    /// terminals act on, so the markers would come out brighter on some
    /// machines than others and the top of a depth ramp would land somewhere
    /// unpredictable. A colour is the part that can be pinned.
    #[test]
    fn the_corner_markers_are_white() {
        let mut cube = placed(1.0, (80, 24));
        let diff = frame(&mut cube, GENERIC);
        let mut markers = 0;
        for (x, y, cell) in diff {
            if cell.symbol != VERTEX_GLYPH {
                continue;
            }
            assert_eq!(
                cell.color, VERTEX_COLOUR,
                "the marker at ({x}, {y}) is {cell:?} rather than the flat marker colour"
            );
            markers += 1;
        }
        assert_eq!(markers, 7, "a cube hides exactly one corner from any view");
    }

    /// Only the corners that are actually visible are marked.
    ///
    /// Marking a corner behind the solid is worse than not marking it: the far
    /// corners of a cube seen face-on project into the *middle* of the near
    /// face, so a diamond appears in the middle of a flat plane and reads as a
    /// hole rather than as a vertex.
    ///
    /// This is the one piece of hidden-line removal the X-ray kept, and the
    /// reason it has to stay is not the one it was written for. It is no longer
    /// about the *edges* -- those are all drawn, and the far corner's three lines
    /// are exactly the X-ray read. It is about the marker, and the marker is a
    /// flat white rather than a depth-shaded one (see [`VERTEX_COLOUR`]): mark
    /// the far corner and it becomes the *brightest* thing on screen while the
    /// three dim lines meeting at it recede, which reads inside out. A marker
    /// that participated in the depth ramp would be fine either way; a flat one
    /// cannot be.
    ///
    /// Seven of eight at a generic rotation is the right answer, not eight --
    /// a solid cube hides exactly one corner from any viewpoint, which is the
    /// count a photograph of one agrees with. The expected set is computed from
    /// the same back-face test the renderer uses, so this asserts the *rule*
    /// rather than a hard-coded seven, and it is checked at a face-on rotation
    /// too, where a naive implementation marks all eight.
    #[test]
    fn only_the_visible_corners_are_marked() {
        for size in [(80u16, 24u16), (200, 50), (120, 40)] {
            for rotation in [GENERIC, (0.41, 0.9, 0.55), (0.0, 0.0, 0.0)] {
                let mut cube = placed(1.0, size);
                let diff = frame(&mut cube, rotation);
                let by_cell: std::collections::HashMap<(usize, usize), Cell> =
                    diff.iter().map(|(x, y, cell)| ((*x, *y), *cell)).collect();

                let visible: Vec<bool> = FACES
                    .iter()
                    .map(|face| cube.face_is_front_facing(face))
                    .collect();
                let expected: Vec<bool> = (0..cube.vertices.len())
                    .map(|vertex| {
                        EDGES.iter().any(|edge| {
                            (edge.v1 == vertex || edge.v2 == vertex)
                                && (visible[edge.faces[0]]
                                    || visible[edge.faces[1]])
                        })
                    })
                    .collect();
                // Between four and seven: a cube shows seven corners from a
                // general viewpoint and four when it is edge-on, so anything
                // outside that band is a degenerate fixture rather than a
                // result. The per-vertex assertions below are the real ones.
                let visible_corners = expected.iter().filter(|v| **v).count();
                assert!(
                    (4..=7).contains(&visible_corners),
                    "{}x{} at {rotation:?}: {visible_corners} corners are visible, \
                     which is not a cube",
                    size.0,
                    size.1
                );

                for (vertex, corner) in cube.vertices.iter().enumerate() {
                    let cell = cell_of(cube.project_vertex(*corner));
                    let symbol =
                        by_cell.get(&cell).map(|c| c.symbol).unwrap_or(' ');
                    if expected[vertex] {
                        assert_eq!(
                            symbol, VERTEX_GLYPH,
                            "{}x{} at {rotation:?}: visible vertex {vertex} at \
                             {cell:?} drew {symbol:?}",
                            size.0, size.1
                        );
                    } else {
                        assert_ne!(
                            symbol, VERTEX_GLYPH,
                            "{}x{} at {rotation:?}: hidden vertex {vertex} at \
                             {cell:?} was marked, so a diamond is showing through \
                             the solid",
                            size.0, size.1
                        );
                    }
                }
            }
        }
    }

    /// Bold on a truecolor foreground is a brightening hint on many terminals,
    /// and a brightness ramp is exactly what it corrupts.
    ///
    /// This effect had `Attribute::Bold` on every cell and no ramp to spoil it,
    /// so the combination was invisible -- which is the argument for removing
    /// it *before* the ramp, not after.
    #[test]
    fn no_cell_is_bold() {
        for braille in [true, false] {
            let options = CubeOptions {
                use_braille: braille,
                ..Default::default()
            };
            let mut cube = Cube::new(options, (80, 24));
            for (index, rotation) in
                [GENERIC, (1.1, 0.3, 0.9)].into_iter().enumerate()
            {
                let diff = frame(&mut cube, rotation);
                assert!(
                    !diff.is_empty(),
                    "braille={braille} step {index} drew nothing"
                );
                for (x, y, cell) in diff {
                    assert_eq!(
                        cell.attr,
                        Attribute::Reset,
                        "braille={braille}: cell ({x}, {y}) is bold, which \
                         brightens the ramp into white on many terminals"
                    );
                }
            }
        }
    }

    /// Every glyph the cube can put in a cell has to be one cell wide.
    ///
    /// A double-width character in a cell-indexed grid shears every coordinate
    /// after it, and the vertex markers are new art: the right glyph with the
    /// wrong width would shift all eight corners rather than merely look odd.
    #[test]
    fn every_glyph_the_cube_can_draw_is_one_cell_wide() {
        for glyph in [VERTEX_GLYPH, ASCII_EDGE_GLYPH].into_iter().chain(
            (1u8..=255)
                .map(|bits| char::from_u32(0x2800 + bits as u32).unwrap_or('?')),
        ) {
            assert!(
                glyph_ramp::is_ambiguous_or_narrow(glyph)
                    // The braille block is East_Asian_Width = Narrow, so it is
                    // single-width everywhere and the shared predicate does not
                    // cover it.
                    || ('\u{2800}'..='\u{28FF}').contains(&glyph),
                "{glyph:?} (U+{:04X}) could shear a cell-indexed grid",
                glyph as u32
            );
        }
    }

    /// A config file can put anything in a float field, and every one of them
    /// has to leave a drawable cube rather than a blank screen or a `NaN`
    /// coordinate.
    ///
    /// `as i32` on a `NaN` is 0, so a `NaN` that reaches the projection does
    /// not error -- it quietly stacks every vertex into the top-left cell.
    ///
    /// The threshold was 100 and had been measured against the *filled* cube,
    /// which draws thousands of cells at this size. The default is the
    /// wireframe now, and a wireframe cube at 80x24 is 92 cells: twelve edges
    /// plus seven corner markers. 40 is the number that still separates "a cube
    /// is on the screen" from the two failures worth catching here, which are a
    /// blank screen (0 cells) and the `NaN` collapse (a handful, because all
    /// eight vertices land in one cell and the twelve edges become twelve points).

    #[test]
    fn a_degenerate_option_falls_back_rather_than_drawing_nothing() {
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            for field in ["cube_size", "distance", "fit"] {
                let options = match field {
                    "cube_size" => CubeOptions {
                        cube_size: bad,
                        ..Default::default()
                    },
                    "distance" => CubeOptions {
                        distance: bad,
                        ..Default::default()
                    },
                    _ => CubeOptions {
                        fit: bad,
                        ..Default::default()
                    },
                };
                let mut cube = Cube::new(options, (80, 24));
                let diff = frame(&mut cube, GENERIC);

                assert!(
                    diff.len() > 40,
                    "{field} = {bad} drew only {} cells, so the cube is not there",
                    diff.len()
                );
                for (x, y, cell) in &diff {
                    assert!(
                        *x < 80 && *y < 24,
                        "{field} = {bad} produced a cell at ({x}, {y}), which is \
                         off screen"
                    );
                    assert!(
                        cell.symbol.is_ascii()
                            || ('\u{2500}'..='\u{2BFF}').contains(&cell.symbol),
                        "{field} = {bad} produced {cell:?} at ({x}, {y}), which is \
                         not a glyph this effect can draw"
                    );
                }
            }
        }
    }

    /// A `cube_size` that would put the camera inside the cube is clamped, not
    /// honoured.
    ///
    /// The perspective divides by `distance + z` and the nearest vertex is at
    /// `z = -size * sqrt(3)`, so past `distance / sqrt(3)` the denominator
    /// changes sign: the near vertices project behind the eye, and every
    /// back-face test in this file inverts with them, so the effect draws the
    /// cube inside out rather than merely too large. The clamp is the reason
    /// `cube_size = 8.0` in the fit test above is a valid configuration.
    #[test]
    fn a_cube_too_large_for_its_camera_is_clamped() {
        let size = DEFAULT_CUBE_SIZE;
        assert_eq!(CubeOptions::default().effective_cube_size(), size);

        for requested in [1.5f32, 2.0, 3.0, 100.0] {
            let options = CubeOptions {
                cube_size: requested,
                ..Default::default()
            };
            let effective = options.effective_cube_size();
            let limit = MAX_CAMERA_FILL * DEFAULT_DISTANCE / 3.0f32.sqrt();
            assert!(
                effective <= limit,
                "cube_size {requested} stayed at {effective}, past the \
                 {limit} the camera cannot see it from"
            );

            let mut cube = Cube::new(options, (80, 24));
            let diff = frame(&mut cube, GENERIC);
            assert!(!diff.is_empty(), "cube_size {requested} drew nothing");
        }
    }

    /// `--print-config` writes every key to disk, so a generated config is
    /// pinned to whatever the defaults were the day it was generated and every
    /// future knob arrives as "a key the user's file does not have". That is
    /// the normal case, not the exotic one.
    #[test]
    fn the_new_keys_round_trip_through_toml() {
        let options: CubeOptions =
            toml::from_str("fit = 0.8\nfilled = false\nglyphs = \" .:oO@\"\n")
                .expect("three keys parse");
        assert_eq!(options.fit, 0.8);
        assert!(!options.filled);
        assert_eq!(options.glyphs, " .:oO@");
        assert_eq!(
            options.cube_size,
            CubeOptions::default().cube_size,
            "three keys in the section silently reset the others"
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        for key in [
            "cube_size",
            "rotation_speed_x",
            "distance",
            "use_braille",
            "fit",
            "filled",
            "vertex_markers",
            "glyphs",
        ] {
            assert!(
                serialised.contains(key),
                "{key} is missing from the serialised form, so --print-config \
                 would not write it: {serialised}"
            );
        }
    }

    /// A ramp a user's config has emptied out degrades to the documented
    /// default rather than to `SHADE`, which is the one preset here that is
    /// documented as not monotonic in ink.
    #[test]
    fn an_unusable_glyph_config_falls_back_to_the_default_ramp() {
        for configured in ["", "\u{7}\u{1}", "\u{4E2D}"] {
            let ramp = glyph_ramp(configured);
            assert_eq!(
                ramp.glyphs(),
                DEFAULT_GLYPHS.chars().collect::<Vec<char>>(),
                "{configured:?} did not fall back to the documented default"
            );
        }
        assert_eq!(DEFAULT_GLYPHS, glyph_ramp::presets::BLOCKS);
    }

    /// The rotation defaults are a contract with the rest of the crate, so this
    /// pins them from inside as well as from `tests/runtime_and_ascii.rs`.
    ///
    /// `filled` is asserted *false* rather than being left out, which is a real
    /// change rather than an omission. It was `true` from the day the fill was
    /// added and the note on [`CubeOptions::filled`] says why it no longer is;
    /// dropping the line instead would have quietly stopped asserting the field
    /// at all, which is the failure mode the rest of this suite exists to catch.
    #[test]
    fn the_rotation_defaults_are_unchanged() {
        let options = CubeOptions::default();
        assert_eq!(options.rotation_speed_x, 0.25);
        assert_eq!(options.rotation_speed_y, 0.35);
        assert_eq!(options.rotation_speed_z, 0.18);
        assert_eq!(options.distance, DEFAULT_DISTANCE);
        assert!(options.use_braille);
        assert!(
            !options.filled,
            "the faces are filled by default again, which is the look that was \
             asked to be taken off by default"
        );
        assert!(options.vertex_markers);
    }

    /// The default look is the wireframe, and that is asserted on what comes out
    /// of the renderer rather than on the boolean.
    ///
    /// This is the user's own report, made into a measurement: the filled cube
    /// "does not look as good" and "the previous one gave a better feeling when
    /// only the edges were visible". The boolean is the easy half; the half that
    /// can rot is the renderer quietly filling faces anyway -- a face fill that
    /// ignored the option, or a default that was flipped back by a later edit.
    /// Both would leave `options.filled` correct and the picture wrong.
    ///
    /// Measured as dots *inside* the near face at a face-on rotation, because at
    /// any other rotation the interior of the silhouette is partly outside the
    /// far face's outline and the two populations overlap. At that rotation the
    /// near face's square is the whole silhouette, so every dot in the middle of
    /// it is face fill and nothing else can be: 240 dots for the wireframe, and
    /// thousands once the fill is on.
    #[test]
    fn the_default_is_the_wireframe_rather_than_the_filled_cube() {
        assert!(
            !CubeOptions::default().filled,
            "the default still fills faces"
        );

        let size = (200u16, 50u16);
        let interior = |filled: bool| {
            let mut cube = Cube::new(
                CubeOptions {
                    filled,
                    ..Default::default()
                },
                size,
            );
            let diff = frame(&mut cube, (0.0, 0.0, 0.0));
            let drawn = dot_patterns(&diff);

            // The near face is z = -cube_size and the projection draws the
            // smaller z larger, so its square is the silhouette.
            let corner = cube.project_vertex(cube.vertices[0]);
            let half_x = (size.0 as f32 / 2.0 - corner.x).abs();
            let half_y = (size.1 as f32 / 2.0 - corner.y).abs();

            let mut dots = 0usize;
            for ((x, y), bits) in &drawn {
                let dx = (*x as f32 + 0.5 - size.0 as f32 / 2.0).abs();
                let dy = (*y as f32 + 0.5 - size.1 as f32 / 2.0).abs();
                if dx < half_x - 1.5 && dy < half_y - 1.5 {
                    dots += bits.count_ones() as usize;
                }
            }
            dots
        };

        let default_dots = interior(false);
        let filled_dots = interior(true);

        // The fill is still a real look, and it is still reachable, so the two
        // have to be measurably different or this test would pass against a
        // `filled` that does nothing at all.
        assert!(
            filled_dots > default_dots * 10,
            "filling the faces changed the interior from {default_dots} dots to \
             {filled_dots}, so the option is not reaching the renderer"
        );
        assert!(
            default_dots < filled_dots / 10,
            "the *default* frame has {default_dots} dots inside the near face, \
             against {filled_dots} with `filled = true`, so the default is \
             drawing the filled cube"
        );
    }

    /// The wireframe alone has to still be a wireframe, since `filled` is a
    /// config field and a user can turn it off.
    ///
    /// The dot count is the interesting half now, and its threshold is the one
    /// number in this file that moved without being rewritten. It was 250 when
    /// the far side was culled and nine edges were drawn; the X-ray draws twelve,
    /// so the frame carries a third more ink. The threshold is left where it is
    /// deliberately: it is a floor on "this is a wireframe rather than a handful
    /// of dots", not a measurement of how many edges there are, and the count of
    /// edges is asserted exactly where it belongs -- in
    /// `the_drawn_edge_ink_is_exactly_the_twelve_edges_union`. Raising it here to
    /// track the change would make this test about edge count too, and it is
    /// about something else: that `filled = false` is honoured at all.
    #[test]
    fn the_wireframe_alone_draws_edges_and_nothing_else() {
        let options = CubeOptions {
            filled: false,
            ..Default::default()
        };
        let mut cube = Cube::new(options, (200, 50));
        let diff = frame(&mut cube, GENERIC);
        assert!(!diff.is_empty());

        // No face was filled, so every drawn dot is edge ink.
        let width = cube.field.faces.width();
        for y in 0..cube.field.faces.height() {
            for x in 0..width {
                let index = y * width + x;
                assert_eq!(
                    cube.field.face_depth[index], NO_DEPTH,
                    "cell ({x}, {y}) was filled by a face with filled = false"
                );
            }
        }

        // And it is still a wireframe rather than a handful of dots: the twelve
        // edges of a cube covering a fitted 200x50 are thousands of dots.
        let dots: u32 = diff
            .iter()
            .map(|(_, _, cell)| (cell.symbol as u32).saturating_sub(0x2800))
            .map(|bits| bits.count_ones())
            .sum();
        assert!(
            dots > 250,
            "the wireframe drew only {dots} dots, which is not twelve edges"
        );
    }

    /// Both renderers draw at every size, and neither of them indexes outside
    /// its own surface.
    ///
    /// `Buffer::set` only debug-asserts its bounds, so in a release build an
    /// out-of-range write corrupts memory rather than failing. The draw
    /// helpers address the canvas through `Canvas::set`, which drops those, but
    /// the cell arithmetic underneath is exactly the part that has to be right.
    #[test]
    fn both_renderers_survive_every_size() {
        for braille in [true, false] {
            for (width, height) in [
                (1u16, 1u16),
                (2, 3),
                (6, 6),
                (12, 6),
                (6, 12),
                (80, 24),
                (200, 50),
                (400, 200),
            ] {
                let options = CubeOptions {
                    use_braille: braille,
                    ..Default::default()
                };
                let mut cube = Cube::new(options, (width, height));
                for step in 0..24 {
                    let t = step as f32 * 0.13;
                    let diff = frame(&mut cube, (t, t * 1.7, t * 0.6));
                    for (x, y, _) in &diff {
                        assert!(
                            *x < width as usize && *y < height as usize,
                            "braille={braille} at {width}x{height} step {step} \
                             produced a cell at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }
}
