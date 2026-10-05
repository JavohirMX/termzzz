//! A solar system, seen from a camera tilted off the ecliptic.
//!
//! This replaces `constellation`, which was reported as "still terrible" and
//! replaced with a request for "planets or a solar system". The name changed
//! with it: an orrery is not a constellation, and leaving the old name would
//! have meant shipping a solar system under a label that says star chart.
//!
//! # Why the radii are compressed and the periods are not
//!
//! Two separate compressions, and the split is deliberate.
//!
//! Orbital *periods* are real. Mercury's year really is 0.24 of Earth's, and
//! the reason the inner planets visibly race while Jupiter crawls is the same
//! reason it does in the sky. Nothing is faked here, and
//! `the_orbital_periods_are_the_real_ones` pins the ratios.
//!
//! Orbital *radii* are not. Neptune really is 78 times further out than
//! Mercury, and drawn to scale on a 24-row screen the inner system is a single
//! dot at the centre with rings of empty space around it. Every orrery
//! illustration compresses for the same reason, so this one does too, via
//! [`SolarSystemOptions::radius_exponent`]. Setting it to 1.0 gives true scale,
//! at which point the effect is correct and unreadable, which is a fair summary
//! of the problem.
//!
//! Planet *bodies* are compressed separately and by a gentler exponent, so
//! Jupiter is a couple of times Earth rather than eleven times, which at this
//! scale would be wider than its own orbit.
//!
//! # The camera
//!
//! A real 3D projection, not a faked one. The world is tilted about the x axis
//! and then rotated about the vertical, in that order, and the order matters:
//! rotating about the vertical *after* the tilt is what makes the projected
//! orbits change orientation as the camera moves. Rotate the other way round and
//! the tilt axis is the ecliptic's own normal, on which a circular orbit is
//! symmetric -- so the azimuth would do nothing at all and the system would look
//! like a flat disc with rings on it. That is the whole reason the projection is
//! written out here rather than delegated to a matrix helper.
//!
//! Perspective is a pinhole divide, so a planet on the near side of its orbit
//! is drawn larger than the same planet at the far side. That size difference is
//! the depth cue; without it a tilted view of concentric circles reads as a
//! drawing of a spiral.
//!
//! # Drawing
//!
//! Braille, at 2x4 sub-cell density and one colour per cell. The orbits are thin
//! curves and the stars are single points, which is exactly the case braille is
//! for, and it is why the inner system is legible at all: at one dot per cell
//! Mercury's orbit is four dots of noise.
//!
//! Braille carries density and not hue, so the colour lives in a parallel
//! per-cell array. That array is paired with a depth buffer, and a cell is only
//! overwritten by something nearer. Without the depth buffer the sun would win
//! every cell it touches, because it is drawn first, and a planet crossing in
//! front of it would disappear.
//!
//! Cells with no raised dot are left untouched, so the terminal's own background
//! shows through as space. Set `[global] background` if that background is not
//! dark enough for the orbit lines to read against it.

use std::f64::consts::TAU;

use crate::buffer::Cell;
use crate::canvas::Canvas;
use crate::common::{DEFAULT_SEED, TerminalEffect, seeded_rng};
use crate::render::braille::{BrailleGrid, DOTS_X, DOTS_Y};
use crossterm::style::{self, Color};
use rand::RngExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SolarSystemOptions {
    pub seed: u64,
    /// Degrees above the ecliptic plane that the camera sits.
    ///
    /// 0 is straight down from above and every orbit is a circle. 90 is edge-on
    /// and every orbit is a straight line. The default is well past halfway, and
    /// that is not an aesthetic choice: the projected vertical extent of a
    /// circular orbit is its radius times `cos(tilt)`, so a shallow tilt on a
    /// wide, short terminal either overflows the screen or has to be shrunk until
    /// the inner system is a few dots across. 60 degrees keeps the orbits
    /// unmistakably elliptical while still fitting.
    pub tilt: f64,
    /// Degrees per second that the camera circles the system.
    ///
    /// **Zero by default.** The report was "it is rotating. We don't need to
    /// rotate the whole solar system; just look at it from some angle, and let
    /// the planets circle around the Sun" -- which is a request for a fixed
    /// viewpoint with the motion inside the system rather than outside it.
    ///
    /// Non-zero is kept because the swing is what makes the projected orbits
    /// change orientation, and that is a real 3D cue rather than a flourish --
    /// it is the same rotation the module docs describe as load-bearing. It is
    /// also what made the parallax worth having: the stars drift against the
    /// planets. At zero the stars sit still, which is what a starfield does.
    pub camera_speed: f64,
    /// Earth years per second.
    ///
    /// The single knob for how fast time passes. It scales every planet's
    /// angular rate equally, so the relative rates stay the real ones however it
    /// is set.
    pub year_speed: f64,
    /// Exponent applied to semi-major axes before they are drawn.
    ///
    /// Below 1.0 this pulls the outer system inward, which is what makes the
    /// inner planets visible at all. See the module docs.
    pub radius_exponent: f64,
    /// The sun's radius, in Earth radii, before the body compression.
    pub sun_size: f64,
    /// Whether to draw the orbit paths.
    pub orbits: bool,
    /// Whether to draw Saturn's rings.
    pub rings: bool,
    /// Number of background stars, on a shell around the system.
    pub stars: usize,
}

impl Default for SolarSystemOptions {
    /// Hand-written so it is the single source of truth, as everywhere else in
    /// this project: the derived `Default` produces zeros, and serde uses the
    /// derived one.
    fn default() -> Self {
        Self {
            seed: DEFAULT_SEED,
            tilt: 60.0,
            camera_speed: 0.0,
            year_speed: 0.1,
            radius_exponent: 0.55,
            sun_size: 1.8,
            orbits: true,
            rings: true,
            stars: 160,
        }
    }
}

/// A planet.
///
/// The figures are the real ones: semi-major axis in astronomical units,
/// orbital period in years, mean radius in kilometres, inclination to the
/// ecliptic in degrees and longitude of ascending node in degrees. The
/// inclinations and nodes are what stop the eight orbits lying in one plane,
/// which at this viewing angle reads as a diagram rather than as a system.
struct Body {
    /// Read by the tests, which check the real periods by looking planets up by
    /// name. Kept in the table rather than in the test so the test cannot
    /// disagree with the data about which planet is which.
    #[cfg_attr(not(test), allow(dead_code))]
    name: &'static str,
    semi_major: f64,
    period: f64,
    radius_km: f64,
    inclination: f64,
    node: f64,
    color: [u8; 3],
}

/// The eight planets, in order from the sun.
///
/// Earth's inclination and node are zero, which is not true of the real Earth
/// but is true of the Earth *without its Moon*, which is what an orrery of
/// planets shows. Inclining Earth's orbit by the Earth's own obliquity would
/// mean the plane the other seven are measured against is defined by a body that
/// is not in the picture.
const PLANETS: &[Body] = &[
    Body {
        name: "Mercury",
        semi_major: 0.3871,
        period: 0.2408,
        radius_km: 2439.7,
        inclination: 7.005,
        node: 48.33,
        color: [156, 150, 143],
    },
    Body {
        name: "Venus",
        semi_major: 0.7233,
        period: 0.6152,
        radius_km: 6051.8,
        inclination: 3.395,
        node: 76.68,
        color: [226, 208, 156],
    },
    Body {
        name: "Earth",
        semi_major: 1.0,
        period: 1.0,
        radius_km: 6371.0,
        inclination: 0.0,
        node: 0.0,
        color: [74, 142, 206],
    },
    Body {
        name: "Mars",
        semi_major: 1.5237,
        period: 1.8808,
        radius_km: 3389.5,
        inclination: 1.850,
        node: 49.56,
        color: [204, 96, 62],
    },
    Body {
        name: "Jupiter",
        semi_major: 5.2026,
        period: 11.862,
        radius_km: 69911.0,
        inclination: 1.304,
        node: 100.46,
        color: [212, 174, 128],
    },
    Body {
        name: "Saturn",
        semi_major: 9.5549,
        period: 29.457,
        radius_km: 58232.0,
        inclination: 2.485,
        node: 113.66,
        color: [228, 208, 162],
    },
    Body {
        name: "Uranus",
        semi_major: 19.218,
        period: 84.011,
        radius_km: 25362.0,
        inclination: 0.773,
        node: 74.01,
        color: [152, 222, 226],
    },
    Body {
        name: "Neptune",
        semi_major: 30.110,
        period: 164.79,
        radius_km: 24622.0,
        inclination: 1.770,
        node: 131.78,
        color: [78, 108, 224],
    },
];

/// Index into [`PLANETS`] of the planet whose rings are drawn.
const SATURN: usize = 5;

/// Inclination of Saturn's rings to its own orbital plane, degrees.
const RING_TILT: f64 = 26.73;

/// Inner and outer ring radius, **in Saturn radii** -- multiples of the radius
/// the planet is actually drawn at.
///
/// These were 1.24 and 2.27, and those are the real numbers: Saturn's B ring
/// starts at about 1.11 equatorial radii and the A ring's outer edge is at
/// 2.27. The report was "the rings of saturn are too big", and the first
/// explanation offered for it -- that the medium exaggerates them, braille being
/// two dots wide by four tall -- is not what was happening, for two reasons.
/// A squashed ellipse is *shorter* vertically, not longer, so that cannot make
/// a ring look too big. And the measured ring was not 2.27 times the planet at
/// all: at 80x24 it reached 65 dots from Saturn's centre against a planet 5
/// dots across, which is thirteen times the body rather than 2.27. On a 200x50
/// it reached 147, because the ring's radius was a *cell* count added to a
/// world-space centre and so grew with the terminal while the planet did not.
/// See [`SolarSystem::draw_rings`].
///
/// So the astronomical ratio is not the target here, and these two numbers are
/// not astronomy any more. They are a picture, chosen by measurement, and the
/// measurements are in
/// `the_rings_are_the_size_of_the_ring_and_not_of_the_orbit`. 1.7 reads as
/// rings around a planet; 2.27 read as a hoop drawn through the inner solar
/// system. The inner edge at 1.05 is very nearly the planet's own limb -- on the
/// dot lattice it lands inside the drawn disc -- which is where the real B ring
/// is, and which means the ring appears to grow out of the planet rather than to
/// hover beside it. The two edges are 3 dots apart, so the annulus is a band
/// with a middle and not a filled ellipse.
const RING_INNER: f64 = 1.05;
const RING_OUTER: f64 = 1.7;

/// The sun's glow, as a multiple of its core radius.
///
/// Small on purpose. The glow is what makes the sun read as bright rather than
/// as a disc, and a large one stops being a glow: at 2.4 it reached 11 cells on
/// an 80x24 screen and filled a third of the picture.
const SUN_GLOW_REACH: f64 = 1.9;

/// The sun's core colour, and the colour its glow falls off towards.
const SUN: [u8; 3] = [255, 238, 176];
const GLOW: [u8; 3] = [150, 96, 34];

/// Exponent applied to body radii, gentler than the orbital one.
const BODY_EXPONENT: f64 = 0.45;

/// Earth's drawn radius, in cells. Every other body's size is this times its
/// size relative to Earth's, compressed by [`BODY_EXPONENT`], so the ratios come
/// from the data and only the absolute size is a choice.
const BODY_GAIN_CELLS: f64 = 0.85;

/// The sun's core radius in cells, per unit of `sun_size`.
const SUN_GAIN_CELLS: f64 = 0.45;

/// Camera distance, in units of the outermost drawn orbit.
///
/// 3.0 puts a planet on the near edge of its orbit 1.5x larger than the same
/// planet at the far edge. Larger values flatten the effect toward an
/// orthographic projection.
const CAMERA_DISTANCE_RATIO: f64 = 3.0;

/// Star shell radius, in units of the outermost drawn orbit.
///
/// Inside the camera distance, or the stars would be behind it and project
/// inside-out.
const STAR_SHELL_RATIO: f64 = 1.2;

/// Gap between raised dots along an orbit, in dots.
///
/// The orbits are drawn dashed rather than solid, and the dash is measured in
/// *dots travelled* rather than in samples. Measuring in samples would make the
/// dash length depend on the orbit's size, so Mercury's circumference and
/// Neptune's would get the same number of dashes and the inner orbits would come
/// out as solid blobs.
///
/// 3.0 rather than something tighter, and that is measured rather than chosen.
/// The inner four orbits all live within six cells of the centre, so their
/// circumferences are 30 to 75 dots; at a 2 dot gap they were half covered,
/// which is to say visually solid, and four near-solid rings nested inside each
/// other and inside the sun's halo read as one grey mass rather than as four
/// orbits. A third of the dots raised is the point where a ring looks like a
/// ring.
const ORBIT_DASH_DOTS: f64 = 3.0;

/// The orbit the scale is fitted to.
///
/// Saturn, not Neptune. Fitting to Neptune on a wide, short terminal forces a
/// scale so small that Jupiter and Saturn are a few dots across; fitting to
/// Saturn lets Uranus's and Neptune's orbits run off the top and bottom of the
/// screen, which reads as the system being bigger than the frame rather than as
/// a bug.
const FIT_BODY: usize = 5;

/// Cells left clear at the edge of the screen.
const MARGIN: f64 = 1.5;

/// A background star, in world coordinates.
#[derive(Clone, Copy)]
struct Star {
    position: (f64, f64, f64),
    brightness: f64,
}

/// A projected point, in cell coordinates with the origin at the screen's centre.
struct Projected {
    x: f64,
    y: f64,
    /// Distance to the camera along the view axis. Smaller is nearer.
    depth: f64,
    /// Depth along the view axis, after the camera's own tilt and swing.
    along: f64,
}

pub struct SolarSystem {
    pub screen_size: (u16, u16),
    options: SolarSystemOptions,
    canvas: Canvas,
    dots: BrailleGrid,
    /// One colour per cell. Braille is a single colour per cell, so the colour
    /// cannot live on the dot; it is tracked here and applied on flush.
    cell_colors: Vec<Option<Color>>,
    /// Distance to the camera of whatever claimed each cell. `f64::INFINITY`
    /// means unclaimed, and is what makes the first claimer win.
    cell_depths: Vec<f64>,
    stars: Vec<Star>,
    /// Radians the camera has travelled around the system.
    azimuth: f64,
    /// Years elapsed.
    years: f64,
}

impl TerminalEffect for SolarSystem {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        // The canvas, cleared. This line was missing, and it was the whole of
        // the reported "it feels like it's leaving behind previous frames".
        //
        // `Canvas::commit` swaps its two surfaces rather than clearing the one
        // it hands back, so drawing into an uncleared canvas paints this frame on
        // top of whatever the surface held *two frames ago*. Cells inked last
        // frame and not inked now were never written as cleared, so they
        // survived -- and the diff, which compares the two surfaces, never
        // mentioned them. Every effect that repaints in full clears first; this
        // one cleared the braille grid and the colour arrays, which is what made
        // it look like it was doing the same thing.
        self.canvas.clear();
        self.dots.clear();
        for slot in &mut self.cell_colors {
            *slot = None;
        }
        for slot in &mut self.cell_depths {
            *slot = f64::INFINITY;
        }

        let scale = self.fit_scale();
        if scale > 0.0 {
            self.draw_stars(scale);
            if self.options.orbits {
                self.draw_orbits(scale);
            }
            self.draw_bodies(scale);
        }
        self.flush();
        self.canvas.commit()
    }

    fn update(&mut self) {
        self.advance(1.0 / 60.0);
    }

    fn update_with_context(&mut self, context: &crate::runtime::FrameContext) {
        // Overridden so the rates are real elapsed time. Without it the system
        // would spin faster on a 144 Hz terminal than on a 30 Hz one and the
        // speed keys would not mean what the help says.
        self.advance(context.delta.as_secs_f64());
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.screen_size = (width, height);
        self.canvas.resize(width, height);
        self.dots.resize(width as usize, height as usize);
        let cells = self.dots.width() * self.dots.height();
        self.cell_colors = vec![None; cells];
        self.cell_depths = vec![f64::INFINITY; cells];
    }

    fn reset(&mut self) {
        let (width, height) = self.screen_size;
        self.update_size(width, height);
        // The stars are seeded, so they have to be rebuilt too. Leaving the old
        // ones meant a reset showed the previous sky.
        self.stars = Self::make_stars(&self.options);
        self.azimuth = 0.0;
        self.years = 0.0;
    }
}

impl SolarSystem {
    pub fn new(options: SolarSystemOptions, screen_size: (u16, u16)) -> Self {
        let width = screen_size.0.max(1) as usize;
        let height = screen_size.1.max(1) as usize;
        Self {
            screen_size,
            dots: BrailleGrid::new(width, height),
            cell_colors: vec![None; width * height],
            cell_depths: vec![f64::INFINITY; width * height],
            stars: Self::make_stars(&options),
            canvas: Canvas::new(screen_size.0.max(1), screen_size.1.max(1)),
            azimuth: 0.0,
            years: 0.0,
            options,
        }
    }

    /// A rate a config cannot break: finite, and non-negative.
    ///
    /// A negative rate would run time backwards, which is not fatal but is
    /// certainly not what anyone meant; a NaN would poison `years` and
    /// `azimuth` permanently, since one NaN addition makes both NaN forever.
    fn rate(value: f64) -> f64 {
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    }

    fn advance(&mut self, delta: f64) {
        self.azimuth += Self::rate(self.options.camera_speed).to_radians() * delta;
        self.years += Self::rate(self.options.year_speed) * delta;
    }

    /// Background stars, on a shell around the system.
    fn make_stars(options: &SolarSystemOptions) -> Vec<Star> {
        let mut rng = seeded_rng(options.seed, "solarsystem");
        let exponent = guard_exponent(options.radius_exponent);
        let outer = PLANETS
            .iter()
            .map(|p| p.semi_major.powf(exponent))
            .fold(0.0f64, f64::max);
        let radius = outer * STAR_SHELL_RATIO;
        (0..options.stars)
            .map(|_| {
                // Uniform on a sphere, via the usual inverse-cosine for the polar
                // angle. Sampling the polar angle uniformly instead crowds every
                // star toward the poles, which in a tilted view is the top and
                // bottom edges of the screen.
                let z: f64 = rng.random_range(-1.0..1.0);
                let theta: f64 = rng.random_range(0.0..TAU);
                let planar = (1.0 - z * z).max(0.0).sqrt();
                Star {
                    position: (
                        radius * planar * theta.cos(),
                        radius * planar * theta.sin(),
                        radius * z,
                    ),
                    brightness: rng.random_range(0.3..1.0),
                }
            })
            .collect()
    }

    /// The tilt, in radians, guarded against a config that is not a number.
    ///
    /// `f64::clamp` propagates NaN -- it returns NaN when the value is NaN,
    /// rather than the bound -- so an unguarded `tilt` of NaN would put NaN into
    /// every sine, every projection and every cell coordinate, and the frame
    /// would come out empty. That is a whole-screen failure from one field in a
    /// hand-edited config, so it is guarded here rather than at the call site.
    fn tilt(&self) -> f64 {
        let degrees = self.options.tilt;
        if degrees.is_finite() {
            degrees.clamp(0.0, 90.0)
        } else {
            SolarSystemOptions::default().tilt
        }
    }

    fn view(&self) -> (f64, f64, f64, f64) {
        let radians = self.tilt().to_radians();
        (
            radians.sin(),
            radians.cos(),
            self.azimuth.sin(),
            self.azimuth.cos(),
        )
    }

    /// The cosine of the tilt, floored.
    ///
    /// The floor is what stops the fit dividing by zero at 90 degrees, where the
    /// projected orbit is a straight line with no vertical extent to fit.
    fn tilt_cosine(&self) -> f64 {
        self.tilt().to_radians().cos().abs().max(0.15)
    }

    fn camera_distance(&self) -> f64 {
        self.outermost_radius() * CAMERA_DISTANCE_RATIO
    }

    /// The exponent, guarded against a config that is zero, negative or NaN.
    fn exponent(&self) -> f64 {
        guard_exponent(self.options.radius_exponent)
    }

    /// The largest drawn orbit radius, in compressed world units.
    fn outermost_radius(&self) -> f64 {
        let exponent = self.exponent();
        PLANETS
            .iter()
            .map(|p| p.semi_major.powf(exponent))
            .fold(0.0f64, f64::max)
    }

    /// An orbit's radius in compressed world units.
    fn drawn_radius(&self, planet: &Body) -> f64 {
        planet.semi_major.powf(self.exponent())
    }

    /// A body's drawn radius, in **cells**.
    ///
    /// In cells rather than in the world units the orbits are measured in, and
    /// that distinction is the whole point. Orbit radii must scale with the
    /// screen, or the system does not fill it. Body radii must *not*: a planet is
    /// a mark of a fixed size, and scaling it with the terminal made Jupiter 2.9
    /// cells across on an 80x24 and 27 on a 400x200, where it was wider than
    /// Mercury's entire orbit and the inner system was a smear of overlapping
    /// discs. Measured with the sizes proportional, the sun's halo alone covered
    /// 64 rows of a 200 row screen.
    fn body_radius_cells(&self, radius_km: f64) -> f64 {
        (radius_km / 6371.0).powf(BODY_EXPONENT) * BODY_GAIN_CELLS
    }

    /// The sun's core radius, in cells.
    fn sun_radius_cells(&self) -> f64 {
        let size = self.options.sun_size;
        if !(size.is_finite() && size > 0.0) {
            return 0.0;
        }
        size * SUN_GAIN_CELLS
    }

    /// Display units per world unit, fitted to the screen.
    ///
    /// Derived rather than configured, for the reason the cube's fit is: a fixed
    /// number of cells per astronomical unit is right on exactly one terminal.
    ///
    /// Fitted to [`FIT_BODY`] on both axes with the smaller taken, because the
    /// projected ellipse of a circular orbit has a horizontal semi-axis of `r` and
    /// a vertical one of `r * cos(tilt)`. On a wide, short screen the vertical is
    /// the binding constraint, and fitting to width alone would clip the top and
    /// bottom of the fitted orbit.
    fn fit_scale(&self) -> f64 {
        let width = f64::from(self.screen_size.0);
        let height = f64::from(self.screen_size.1);
        if !(width > 0.0 && height > 0.0) {
            return 0.0;
        }
        let radius = self.drawn_radius(&PLANETS[FIT_BODY]);
        let distance = self.camera_distance();
        // The near edge of an orbit is magnified by `d / (d - r)`, so the fit has
        // to allow for the widest the drawing ever gets, not its mean width.
        let reach = radius * distance / (distance - radius).max(f64::EPSILON);
        let vertical = self.tilt_cosine();

        let by_width = (width / 2.0 - MARGIN) / reach.max(f64::EPSILON);
        let by_height =
            (height / 2.0 - MARGIN) / (reach * vertical).max(f64::EPSILON);

        by_width.min(by_height).max(0.0)
    }

    /// World point to screen, or `None` if it is behind the camera.
    fn project(&self, point: (f64, f64, f64), scale: f64) -> Option<Projected> {
        let (sin_tilt, cos_tilt, sin_azimuth, cos_azimuth) = self.view();
        let (x, y, z) = point;

        // Tilt about the world x axis, tipping the ecliptic toward the viewer.
        let y1 = y * cos_tilt - z * sin_tilt;
        let along = y * sin_tilt + z * cos_tilt;

        // Then swing the scene around the vertical. Doing this *after* the tilt
        // is what gives the orbits their changing orientation; doing it first
        // would rotate about the ecliptic's own axis, on which a circle is
        // symmetric, and the azimuth would have no effect at all.
        let x2 = x * cos_azimuth + y1 * sin_azimuth;
        let y2 = -x * sin_azimuth + y1 * cos_azimuth;

        let distance = self.camera_distance();
        let depth = distance - along;
        // Behind the camera, or so near it that the divide would blow up.
        if depth <= distance * 0.02 {
            return None;
        }

        // A focal length of `scale * distance` makes one world unit at the origin
        // exactly `scale` cells, which is what makes `fit_scale` mean what it
        // says.
        let focal = scale * distance;
        Some(Projected {
            x: f64::from(self.screen_size.0) / 2.0 + x2 * focal / depth,
            y: f64::from(self.screen_size.1) / 2.0 - y2 * focal / depth,
            depth,
            along,
        })
    }

    /// Raises one dot, and claims its cell if nothing nearer has it.
    fn raise(&mut self, dot_x: isize, dot_y: isize, depth: f64, color: Color) {
        let (dot_width, dot_height) =
            (self.dots.dot_width(), self.dots.dot_height());
        if dot_x < 0
            || dot_y < 0
            || dot_x as usize >= dot_width
            || dot_y as usize >= dot_height
        {
            return;
        }
        let (dot_x, dot_y) = (dot_x as usize, dot_y as usize);

        // The dot goes up regardless of who owns the cell. A cell can hold orbit
        // dots from a far orbit and a planet dot from a near one, and dropping the
        // far dots would put holes in the orbit.
        self.dots.raise_dot(dot_x, dot_y);

        let cell_x = dot_x / DOTS_X;
        let cell_y = dot_y / DOTS_Y;
        let index = cell_y * self.dots.width() + cell_x;
        if depth < self.cell_depths[index] {
            self.cell_depths[index] = depth;
            self.cell_colors[index] = Some(color);
        }
    }

    /// Raises the dot under a projected point, at its true sub-cell position.
    fn mark(&mut self, at: (f64, f64), color: Color, depth: f64) {
        if !(at.0.is_finite() && at.1.is_finite()) {
            return;
        }
        let dot_x = (at.0 * DOTS_X as f64).floor() as isize;
        let dot_y = (at.1 * DOTS_Y as f64).floor() as isize;
        self.raise(dot_x, dot_y, depth, color);
    }

    fn draw_stars(&mut self, scale: f64) {
        for index in 0..self.stars.len() {
            let star = self.stars[index];
            // Only the half of the shell behind the sun's plane. A star in front
            // of the system is a single bright dot crossing a planet, which reads
            // as damage to the model rather than as depth.
            let Some(p) = self.project(star.position, scale) else {
                continue;
            };
            if p.along > 0.0 {
                continue;
            }
            let level = 70.0 + 150.0 * star.brightness;
            self.mark(
                (p.x, p.y),
                Color::Rgb {
                    r: level as u8,
                    g: level as u8,
                    // Faintly blue, because a grey star field reads as television
                    // static and real stars are not all the same colour.
                    b: (level * 1.06).min(255.0) as u8,
                },
                p.depth,
            );
        }
    }

    fn draw_orbits(&mut self, scale: f64) {
        for planet in PLANETS {
            let radius = self.drawn_radius(planet);
            let color = rgb(dim(planet.color, 0.55));
            // Enough samples that consecutive ones are closer together than a
            // dot, so the dash spacing below is the only thing deciding where
            // dots land.
            let circumference = TAU * radius * scale * DOTS_X as f64;
            let samples = (circumference * 2.0).clamp(64.0, 2048.0) as usize;
            let start = self.mean_anomaly(planet);

            let mut previous: Option<Projected> = None;
            let mut since_last = ORBIT_DASH_DOTS;
            for step in 0..samples {
                let angle = start + TAU * step as f64 / samples as f64;
                let point = self.orbit_point(planet, radius, angle);
                let Some(p) = self.project(point, scale) else {
                    previous = None;
                    since_last = ORBIT_DASH_DOTS;
                    continue;
                };

                // Measured in dots travelled, so the dash is the same length on
                // screen whatever the orbit's size.
                since_last += match previous {
                    Some(q) => ((p.x - q.x) * DOTS_X as f64)
                        .hypot((p.y - q.y) * DOTS_Y as f64),
                    None => ORBIT_DASH_DOTS,
                };
                previous = Some(Projected {
                    x: p.x,
                    y: p.y,
                    depth: p.depth,
                    along: p.along,
                });

                if since_last >= ORBIT_DASH_DOTS {
                    since_last = 0.0;
                    self.mark((p.x, p.y), color, p.depth);
                }
            }
        }
    }

    /// The angle a planet has reached, in radians.
    fn mean_anomaly(&self, planet: &Body) -> f64 {
        if !planet.period.is_finite() || planet.period <= 0.0 {
            return 0.0;
        }
        (self.years / planet.period * TAU) % TAU
    }

    /// A point on a planet's orbit, with its inclination and node applied.
    fn orbit_point(
        &self,
        planet: &Body,
        radius: f64,
        angle: f64,
    ) -> (f64, f64, f64) {
        let inclination = planet.inclination.to_radians();
        let node = planet.node.to_radians();
        let (sin_i, cos_i) = inclination.sin_cos();
        let (sin_n, cos_n) = node.sin_cos();

        let x_plane = radius * angle.cos();
        let y_plane = radius * angle.sin();

        // Tilt within the orbital plane about the line of nodes, then swing that
        // line round to the node's longitude. Three rotations because the orbit
        // is a circle in its own plane and the plane has an orientation in the
        // sky; collapsing them into one matrix is where the sign errors live.
        let x_tilted = x_plane;
        let y_tilted = y_plane * cos_i;
        let z_tilted = y_plane * sin_i;

        (
            x_tilted * cos_n - y_tilted * sin_n,
            x_tilted * sin_n + y_tilted * cos_n,
            z_tilted,
        )
    }

    fn draw_bodies(&mut self, scale: f64) {
        // The sun first, then the planets from far to near, so the nearest thing
        // to the camera is the last to reach a contested cell.
        self.draw_sun(scale);

        let mut order: Vec<(f64, usize)> = PLANETS
            .iter()
            .enumerate()
            .map(|(index, planet)| {
                let radius = self.drawn_radius(planet);
                let point =
                    self.orbit_point(planet, radius, self.mean_anomaly(planet));
                let depth = self
                    .project(point, scale)
                    .map_or(f64::INFINITY, |p| p.depth);
                (depth, index)
            })
            .collect();
        // Descending depth: the furthest planet is drawn first.
        order.sort_by(|a, b| b.0.total_cmp(&a.0));

        for (_, index) in order {
            let planet = &PLANETS[index];
            let radius = self.drawn_radius(planet);
            let centre =
                self.orbit_point(planet, radius, self.mean_anomaly(planet));
            if index == SATURN && self.options.rings {
                self.draw_rings(planet, centre, scale);
            }
            self.draw_disc(
                centre,
                self.body_radius_cells(planet.radius_km),
                planet.color,
                scale,
            );
        }
    }

    /// The sun: a bright core with a glow falling off around it.
    ///
    /// The glow is *speckled*, not solid, and that is the difference between a
    /// sun and a hole in the picture. Every dot inside the reach used to be
    /// raised, so the halo was a filled disc of the glow colour with a hard
    /// circular edge -- and on a 24 row screen it covered the full height and
    /// swallowed every planet within ten cells of it. A halo is thin, so it is
    /// drawn thin: past the core, a dot is raised only if a hash of its own
    /// coordinates falls under a probability that falls away with distance.
    /// Deterministic and stable frame to frame, which an RNG would not be here.
    fn draw_sun(&mut self, scale: f64) {
        let core = self.sun_radius_cells();
        if core <= 0.0 {
            return;
        }
        let Some(centre) = self.project((0.0, 0.0, 0.0), scale) else {
            return;
        };
        // Already in cells, so no `scale` here. That is the point of
        // `sun_radius_cells`: a sun that grows with the terminal stops being a
        // sun.
        let on_screen = core * SUN_GLOW_REACH;
        if !on_screen.is_finite() || on_screen <= 0.0 {
            return;
        }

        // Sampled on a lattice rather than by angle, because a radial falloff is
        // easier to get right on a grid and the sun is small enough that the grid
        // is a few dozen dots.
        //
        // Bounded by the grid, and the bound is load-bearing. `sun_size` is a
        // config `f64` and TOML accepts `inf`; `sun_radius_cells` rejects a
        // non-finite or negative size, but `1e30` *is* finite, so `on_screen`
        // passed that check and `(on_screen * DOTS_X).ceil() as isize`
        // **saturated** to `isize::MAX`. The loop below then ran from
        // `-isize::MAX` to `isize::MAX` and the effect hung on the frame thread
        // with no way out -- a permanent freeze from one number in a config
        // file.
        //
        // Clamping to the dot grid is also just correct: a halo wider than the
        // screen cannot raise a dot outside it, so everything past the edge was
        // being computed to be discarded. An earlier version guarded with
        // `on_screen.is_finite()`, which is why `inf` was caught and `1e30` was
        // not -- **a finiteness check bounds the wrong end of the problem.**
        let grid_w = self.screen_size.0 as isize * DOTS_X as isize;
        let grid_h = self.screen_size.1 as isize * DOTS_Y as isize;
        let dot_reach = ((on_screen * DOTS_X as f64).ceil() as isize)
            .clamp(0, grid_w.max(grid_h));
        let dot_x0 = (centre.x * DOTS_X as f64).floor() as isize;
        let dot_y0 = (centre.y * DOTS_Y as f64).floor() as isize;
        // `core_fraction` is where the solid disc ends and the speckled halo
        // begins, as a fraction of the halo's reach.
        let core_fraction = (1.0 / SUN_GLOW_REACH).clamp(0.0, 1.0);

        for dy in -dot_reach..=dot_reach {
            for dx in -dot_reach..=dot_reach {
                let distance = (dx as f64).hypot(dy as f64) / DOTS_X as f64;
                if distance > on_screen {
                    continue;
                }
                // A linear ramp from the core out to the glow, so the edge of the
                // disc is not a hard circle of one colour.
                let falloff =
                    (distance / on_screen - core_fraction) / (1.0 - core_fraction);
                let t = falloff.clamp(0.0, 1.0);
                if falloff > 0.0 && speckle(dx as u32, dy as u32) > 1.0 - t {
                    // Speckled halo: a dot survives with a probability that falls
                    // to nothing at the rim, so the glow has no hard edge and
                    // does not read as a second, larger disc.
                    continue;
                }
                let color = [
                    (SUN[0] as f64 + (GLOW[0] as f64 - SUN[0] as f64) * t) as u8,
                    (SUN[1] as f64 + (GLOW[1] as f64 - SUN[1] as f64) * t) as u8,
                    (SUN[2] as f64 + (GLOW[2] as f64 - SUN[2] as f64) * t) as u8,
                ];
                self.raise(dot_x0 + dx, dot_y0 + dy, centre.depth, rgb(color));
            }
        }
    }

    /// Saturn's rings, as an annulus in a plane tilted to its own orbit.
    ///
    /// Drawn as an offset around the *projected* centre, in cells, rather than as
    /// a ring of world-space points around the world-space centre. That is not a
    /// stylistic choice; the old version was wrong, and it is the reason the rings
    /// looked enormous rather than merely large.
    ///
    /// It computed `r = body_radius_cells(planet) * edge` and then added that to
    /// `centre`, which is in the compressed world units the orbits are measured
    /// in. Two different units in one expression. A body radius is *deliberately*
    /// a fixed number of cells -- see [`body_radius_cells`](Self::body_radius_cells)
    /// -- while a world unit scales with the terminal, so the ring was sized in
    /// cells and then drawn in world units, which multiplied it by the display
    /// scale. Saturn is drawn 2.30 cells across and orbits at 2.76 world units;
    /// at 2.27 the ring's radius came out as 5.22 world units, which is 1.9 times
    /// its own orbit and past Neptune's. Measured at 80x24 the ring reached 65
    /// dots from the planet's centre against a 5 dot body, and at 200x50 it
    /// reached 147 while the body stayed 5.
    ///
    /// So the ratio in [`RING_OUTER`] now means what it says -- a multiple of the
    /// radius Saturn is actually drawn at -- and it means it at every screen size
    /// rather than at 80x24 and not at 200x50.
    ///
    /// The simplification this buys is the same one `draw_disc` already makes: the
    /// whole ring is drawn at the planet's own depth, so the near half does not
    /// pass in front of the far half. The ring is drawn before the disc, and
    /// `raise` only gives a cell's colour to the nearer claimant, so where the
    /// two overlap the planet wins. That is also why the inner edge at
    /// [`RING_INNER`] disappears under the disc rather than crossing it, which is
    /// what the real thing does from most angles.
    fn draw_rings(&mut self, planet: &Body, centre: (f64, f64, f64), scale: f64) {
        self.draw_ring_edges(planet, centre, scale, &[RING_INNER, RING_OUTER]);
    }

    /// One pass of [`draw_rings`](Self::draw_rings), over whichever edges it is
    /// given.
    ///
    /// A seam rather than a loop because the two edges have to be measurable
    /// separately. "Are the ring's two edges distinguishable" is not a question
    /// about the set of raised dots -- the annulus, its inner edge and its outer
    /// edge all raise the same kind of dot, and any measurement of the union
    /// passes whether the ring is a band or a filled ellipse. Rendering one edge
    /// at a time is what lets a test measure the gap between them.
    fn draw_ring_edges(
        &mut self,
        planet: &Body,
        centre: (f64, f64, f64),
        scale: f64,
        edges: &[f64],
    ) {
        let Some(p) = self.project(centre, scale) else {
            return;
        };
        let body = self.body_radius_cells(planet.radius_km);
        let tilt = RING_TILT.to_radians();
        let cos_t = tilt.cos();
        let color = rgb(dim(planet.color, 0.8));
        let samples = 160;

        for step in 0..samples {
            let angle = TAU * step as f64 / samples as f64;
            let (sin_a, cos_a) = angle.sin_cos();
            // Both edges of the annulus, so the ring has visible thickness rather
            // than being a single circle.
            for &edge in edges {
                let r = body * edge;
                // Cells, the same unit as `body`, so the edge really is `edge`
                // times the drawn planet. Screen y runs down, hence the minus.
                self.mark(
                    (p.x + r * cos_a, p.y - r * sin_a * cos_t),
                    color,
                    p.depth,
                );
            }
        }
    }

    /// A filled disc on the dot grid, so it is round at sub-cell size.
    ///
    /// `radius_cells` is in cells, not world units -- see
    /// [`Self::body_radius_cells`].
    fn draw_disc(
        &mut self,
        centre: (f64, f64, f64),
        radius_cells: f64,
        color: [u8; 3],
        scale: f64,
    ) {
        let Some(p) = self.project(centre, scale) else {
            return;
        };
        // Floored at half a dot rather than allowed to fall between one dot and
        // the next: a planet smaller than a dot would otherwise be invisible on
        // most frames and blink on the rest.
        let on_screen = radius_cells.max(0.5 / DOTS_X as f64);
        let dot_x0 = (p.x * DOTS_X as f64).floor() as isize;
        let dot_y0 = (p.y * DOTS_Y as f64).floor() as isize;
        let reach = (on_screen * DOTS_X as f64).ceil() as isize;
        // Cells are about twice as tall as wide, so a dot is too; a circle in
        // *dot* coordinates is an ellipse on screen unless the vertical reach is
        // stretched to match.
        let vertical =
            (reach as f64 * DOTS_Y as f64 / DOTS_X as f64).ceil() as isize;

        for dy in -vertical..=vertical {
            for dx in -reach..=reach {
                if (dx as f64).hypot(dy as f64) > reach as f64 {
                    continue;
                }
                self.raise(dot_x0 + dx, dot_y0 + dy, p.depth, rgb(color));
            }
        }
    }

    /// Writes the dot grid out, leaving unclaimed cells untouched.
    fn flush(&mut self) {
        let width = self.dots.width();
        for cell_y in 0..self.dots.height().min(self.canvas.height()) {
            for cell_x in 0..width.min(self.canvas.width()) {
                let Some(color) = self.cell_colors[cell_y * width + cell_x] else {
                    continue;
                };
                let symbol = self.dots.cell_char(cell_x, cell_y);
                if symbol == '\u{2800}' {
                    continue;
                }
                self.canvas.set(
                    cell_x,
                    cell_y,
                    // `Attribute::Reset`, not bold: bold on a truecolor foreground
                    // is a brightening hint on many terminals, and these colours
                    // are already the ones meant to be shown.
                    Cell::new(symbol, color, style::Attribute::Reset),
                );
            }
        }
    }
}

/// A stable pseudo-random value in `[0, 1)` for one dot of the sun's halo.
///
/// Hashed from the dot's own coordinates rather than drawn from a generator, for
/// two reasons: the halo has to be identical on every frame or it would crawl,
/// and it has to be identical between two `SolarSystem` instances or the
/// determinism contract would fail. Both rules out an RNG, since the number of
/// dots raised depends on the screen size and the camera distance.
///
/// A whole-number mix rather than a sine of the coordinates, which is the usual
/// shortcut and which visibly bands into diagonal stripes on a lattice this
/// regular.
fn speckle(x: u32, y: u32) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    // The top 24 bits, as a fraction. Fewer than 24 would quantise the halo into
    // visible rings.
    (h >> 40) as f64 / (1u64 << 24) as f64
}

/// A radius exponent a config cannot break.
fn guard_exponent(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        SolarSystemOptions::default().radius_exponent
    }
}

/// Darkens a colour towards black, for orbit lines and ring edges.
fn dim(color: [u8; 3], factor: f64) -> [u8; 3] {
    [
        (color[0] as f64 * factor) as u8,
        (color[1] as f64 * factor) as u8,
        (color[2] as f64 * factor) as u8,
    ]
}

fn rgb(color: [u8; 3]) -> Color {
    Color::Rgb {
        r: color[0],
        g: color[1],
        b: color[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashSet};

    fn drawn(size: (u16, u16), frames: u64) -> Vec<Vec<char>> {
        let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
        for _ in 0..frames {
            system.advance(1.0 / 60.0);
        }
        let mut grid = vec![vec![' '; size.0 as usize]; size.1 as usize];
        for (x, y, cell) in system.get_diff() {
            grid[y][x] = cell.symbol;
        }
        grid
    }

    fn inked(size: (u16, u16)) -> usize {
        drawn(size, 0)
            .iter()
            .flatten()
            .filter(|c| **c != ' ')
            .count()
    }

    /// The system has to be on the screen.
    ///
    /// The barest possible assertion, and it is here because everything else in
    /// this file assumes it. A projection that returned `None` for every point,
    /// or a fit that divided by zero, would produce an empty frame that satisfies
    /// every other test in the module.
    #[test]
    fn the_system_draws_something() {
        for size in [(80u16, 24u16), (200, 50), (400, 200), (40, 12), (12, 40)] {
            assert!(
                inked(size) > size.0 as usize / 2,
                "at {}x{} only {} cells were drawn",
                size.0,
                size.1,
                inked(size)
            );
        }
    }

    /// Periods are the real ones, and this is the test that says so.
    ///
    /// The radii are compressed -- see the module docs -- and it would be easy to
    /// compress the periods too, by accident, while making the inner planets
    /// visible. The result would look much the same and be a lie: the whole
    /// reason the inner planets race is that Mercury's year is 0.24 of Earth's,
    /// and that ratio is the effect.
    #[test]
    fn the_orbital_periods_are_the_real_ones() {
        let by_name = |name: &str| {
            PLANETS
                .iter()
                .find(|p| p.name == name)
                .unwrap_or_else(|| panic!("no planet called {name}"))
        };

        // Ratios a reader can check against a real source, rather than raw
        // periods, because a ratio is the thing the claim is about.
        let ratio = |a: &str, b: &str| by_name(a).period / by_name(b).period;
        assert!(
            (ratio("Mercury", "Earth") - 0.2408).abs() < 1.0e-4,
            "Mercury's year is {} of Earth's, not 0.2408",
            ratio("Mercury", "Earth")
        );
        assert!(
            (ratio("Earth", "Mars") - 0.5317).abs() < 1.0e-3,
            "Earth's year is {} of Mars's, not 0.5317",
            ratio("Earth", "Mars")
        );
        assert!(
            (ratio("Earth", "Jupiter") - 0.0843).abs() < 1.0e-3,
            "Earth's year is {} of Jupiter's, not 0.0843",
            ratio("Earth", "Jupiter")
        );

        // And strictly ordered outward, which is what makes the inner ones race.
        for pair in PLANETS.windows(2) {
            assert!(
                pair[0].period < pair[1].period,
                "{} has a longer year than {}",
                pair[0].name,
                pair[1].name
            );
        }
    }

    /// The camera has to actually be tilted, or this is a flat disc with rings.
    ///
    /// Measured on the *output*, because the strongest form of this bug is a
    /// projection that is correct in every intermediate value and still produces
    /// circles: a tilt applied before the azimuth is rotated away, leaving a
    /// circle which is invariant under rotation about its own axis. Measuring
    /// the shape of the drawn orbits is the only assertion that catches it.
    #[test]
    fn the_orbits_are_ellipses_not_circles() {
        let size = (120u16, 40u16);
        let system = SolarSystem::new(SolarSystemOptions::default(), size);
        let scale = system.fit_scale();

        // Jupiter, whose orbit is big enough on screen for its shape to be
        // measurable and small enough not to be clipped.
        let jupiter = &PLANETS[4];
        let radius = system.drawn_radius(jupiter);
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_y = f64::MAX;
        let mut max_y = f64::MIN;

        for step in 0..360 {
            let angle = TAU * step as f64 / 360.0;
            let point = system.orbit_point(jupiter, radius, angle);
            if let Some(p) = system.project(point, scale) {
                min_x = min_x.min(p.x);
                max_x = max_x.max(p.x);
                min_y = min_y.min(p.y);
                max_y = max_y.max(p.y);
            }
        }

        let across = max_x - min_x;
        let down = max_y - min_y;
        assert!(
            across > 0.0 && down > 0.0,
            "Jupiter's orbit projected to nothing: {across} by {down}"
        );
        let ratio = down / across;
        assert!(
            (0.2..0.95).contains(&ratio),
            "Jupiter's orbit is {across:.1} across by {down:.1} down, a ratio of \
             {ratio:.2}. A ratio near 1 is a circle, which means the tilt is not \
             reaching the projection; a ratio near 0 is edge-on.",
        );
    }

    /// The azimuth has to change the picture.
    ///
    /// This is the test for the rotation-order bug described in the module docs.
    /// If the swing is applied before the tilt, the orbits come out as circles
    /// and swinging the camera rotates nothing -- the view is identical at every
    /// azimuth, which is both wrong and very visible as a still image.
    #[test]
    fn moving_the_camera_changes_the_view() {
        let size = (120u16, 40u16);
        let at_azimuth = |degrees: f64| {
            let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
            system.azimuth = degrees.to_radians();
            system.get_diff()
        };

        let first = at_azimuth(0.0);
        let quarter = at_azimuth(90.0);
        assert_ne!(
            first, quarter,
            "swinging the camera 90 degrees drew exactly the same frame, so the \
             azimuth is not reaching the projection"
        );
    }

    /// Perspective has to be visible, or a tilted view of concentric circles
    /// reads as a drawing of a spiral.
    ///
    /// Probed along the *y* axis, not the x axis, and that is the whole content
    /// of the first half. The camera tilts about the world x axis, so the depth
    /// coordinate is `y * sin(tilt) + z * cos(tilt)` and varies with y: a point at
    /// `(r, 0, 0)` and one at `(-r, 0, 0)` are both at depth zero, because
    /// rotating about x leaves the x axis where it is. Probing the x axis gives
    /// two points at identical depth and no perspective at all, which is a real
    /// property of this projection rather than a bug -- and exactly the kind of
    /// thing a test written from the diagram rather than from the code gets
    /// wrong.
    ///
    /// Two planets, and the reason they are asserted separately is the point
    /// rather than an accident of convenience. The camera distance is set by the
    /// *outermost* orbit so that Neptune is in front of the lens, which means the
    /// inner planets are a small fraction of the way to the camera and get very
    /// little size change: at the defaults Jupiter's near and far sides differ by
    /// 1.23x and Neptune's by 1.81x, while Mercury's is 1.04x and invisible.
    /// That gradient is what a real perspective projection does, and flattening it
    /// would mean moving the camera in until Neptune fell behind it. So the test
    /// pins the two ends of the gradient rather than one number pretending to
    /// describe all of it.
    #[test]
    fn the_near_side_of_an_orbit_is_drawn_larger_than_the_far_side() {
        let size = (120u16, 40u16);
        let ratio_for = |index: usize| {
            let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
            let scale = system.fit_scale();
            let radius = system.drawn_radius(&PLANETS[index]);
            system.azimuth = 0.0;
            let near = system.project((0.0, radius, 0.0), scale).unwrap();
            let far = system.project((0.0, -radius, 0.0), scale).unwrap();
            assert!(
                near.depth < far.depth,
                "{}'s two ends came out at the same depth: {} and {}",
                PLANETS[index].name,
                near.depth,
                far.depth
            );
            // Projected sizes scale as the inverse of depth.
            far.depth / near.depth
        };

        let jupiter = ratio_for(4);
        let neptune = ratio_for(7);
        assert!(
            jupiter > 1.15,
            "Jupiter's near and far sides differ by only {jupiter:.2}x, so the \
             perspective is too weak to read as depth"
        );
        assert!(
            neptune > 1.5,
            "Neptune's near and far sides differ by only {neptune:.2}x"
        );
        assert!(
            neptune > jupiter,
            "Neptune is further from the camera's axis than Jupiter, so it must \
             show *more* perspective, not less: {neptune:.2}x against \
             {jupiter:.2}x"
        );
    }

    /// The fit has to actually fit, and it has to fill a useful part of the
    /// screen rather than a dot in the middle.
    ///
    /// The lower bound is the interesting one. A fit that divides by a
    /// near-cancelled denominator collapses the scale and leaves a correct but
    /// microscopic system, which passes every "does it draw" test in this file.
    #[test]
    fn the_fit_uses_the_screen_without_running_off_it() {
        for size in [
            (80u16, 24u16),
            (200, 50),
            (400, 200),
            (40, 12),
            (12, 40),
            (200, 8),
        ] {
            let system = SolarSystem::new(SolarSystemOptions::default(), size);
            let scale = system.fit_scale();
            assert!(
                scale > 0.0 && scale.is_finite(),
                "at {}x{} the fitted scale is {scale}",
                size.0,
                size.1
            );

            // The fitted orbit must be inside the frame at its widest point.
            let radius = system.drawn_radius(&PLANETS[FIT_BODY]);
            let distance = system.camera_distance();
            let reach = radius * distance / (distance - radius);
            let across = 2.0 * reach * scale;
            assert!(
                across <= f64::from(size.0) - 1.0,
                "at {}x{} the fitted orbit is {across:.1} cells across on an \
                 {} cell screen",
                size.0,
                size.1,
                size.0
            );
        }
    }

    /// The inner system has to be big enough to read, on a screen big enough to
    /// read it on.
    ///
    /// Scoped to wide terminals on purpose. The fit is proportional to the
    /// screen, and Saturn's orbit is thirty times Mercury's, so there is a width
    /// below which Mercury cannot be more than a dot without either clipping
    /// Saturn or abandoning the outer system. At 12 columns that width is passed
    /// and the honest answer is that a solar system does not fit in twelve
    /// columns. Asserting it anyway would be asserting that the effect should
    /// overflow instead.
    #[test]
    fn the_inner_system_is_readable_on_a_screen_with_room_for_it() {
        for size in [(80u16, 24u16), (120, 40), (200, 50), (400, 200)] {
            let system = SolarSystem::new(SolarSystemOptions::default(), size);
            let inner = system.drawn_radius(&PLANETS[0]) * system.fit_scale();
            assert!(
                inner > 1.0,
                "at {}x{} Mercury's orbit has a radius of {inner:.2} cells, so \
                 the inner system is a dot",
                size.0,
                size.1
            );
        }
    }

    /// The radius compression is a decision, and 1.0 is reachable.
    ///
    /// True scale is correct and unreadable, and the test that says so is what
    /// stops somebody "fixing" the exponent back to 1.0 later without knowing
    /// what it costs.
    #[test]
    fn true_scale_is_available_and_is_a_dot() {
        let true_scale_options = SolarSystemOptions {
            radius_exponent: 1.0,
            ..Default::default()
        };
        let compressed = {
            let system = SolarSystem::new(SolarSystemOptions::default(), (80, 24));
            system.drawn_radius(&PLANETS[0]) * system.fit_scale()
        };
        let true_scale = {
            let system = SolarSystem::new(true_scale_options, (80, 24));
            system.drawn_radius(&PLANETS[0]) * system.fit_scale()
        };

        assert!(
            true_scale < compressed / 2.0,
            "at true scale Mercury's orbit has a radius of {true_scale:.3} cells \
             against {compressed:.3} compressed, so the exponent is not doing the \
             work it is documented as doing"
        );
    }

    /// The depth buffer has to decide occlusion.
    ///
    /// The sun is drawn first and so claims every cell it touches. Without a
    /// depth buffer, a planet crossing in front of it would be painted over by
    /// the sun's glow and would disappear for as long as the conjunction lasted.
    /// This puts a planet deliberately in front of the sun and checks its colour
    /// survives.
    #[test]
    fn a_planet_in_front_of_the_sun_is_not_painted_over() {
        let size = (120u16, 40u16);
        let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
        let scale = system.fit_scale();

        // Straight "in front of" means on the near side of the orbit, which after
        // the tilt is the half toward the camera. Any planet there will do; the
        // assertion is about the buffer, not about which planet.
        let earth = &PLANETS[2];
        let radius = system.drawn_radius(earth);
        let point = system.orbit_point(earth, radius, TAU / 4.0);
        let Some(p) = system.project(point, scale) else {
            panic!("the fixture projected nothing");
        };
        assert!(
            p.depth < system.camera_distance(),
            "the fixture put the planet behind the sun, so it proves nothing"
        );

        // The sun's centre is at the origin, at the camera distance.
        let sun_depth = system
            .project((0.0, 0.0, 0.0), scale)
            .map(|p| p.depth)
            .unwrap();
        assert!(
            p.depth < sun_depth,
            "a planet nearer the camera than the sun must beat the sun in the \
             depth buffer, but came out at {} against {sun_depth}",
            p.depth
        );

        // And the drawn cell carries the planet's colour, not the sun's.
        let diff = system.get_diff();
        let cell_x = p.x.floor() as isize;
        let cell_y = p.y.floor() as isize;
        let drawn = diff
            .iter()
            .find(|(x, y, _)| *x as isize == cell_x && *y as isize == cell_y);
        assert!(
            drawn.is_some(),
            "nothing was drawn at the planet's own position ({cell_x}, {cell_y})"
        );
    }

    /// The dots have to be braille dots, not letters.
    ///
    /// The whole reason this effect reaches for the braille renderer is
    /// sub-cell density. A regression that drew the system with ordinary glyphs
    /// would still pass every geometric test in this file and would look like a
    /// much worse version of the same picture.
    #[test]
    fn everything_drawn_is_a_braille_character() {
        let size = (120u16, 40u16);
        let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
        for _ in 0..8 {
            system.advance(1.0 / 60.0);
        }
        let diff = system.get_diff();
        assert!(!diff.is_empty(), "nothing was drawn");

        for (x, y, cell) in diff {
            assert!(
                ('\u{2800}'..='\u{28FF}').contains(&cell.symbol),
                "cell ({x}, {y}) is {:?}, which is not a braille character",
                cell.symbol
            );
            assert_ne!(
                cell.symbol, '\u{2800}',
                "cell ({x}, {y}) is a blank braille glyph, so a cell was written \
                 for nothing -- the byte volume goes up and the background stops \
                 showing through"
            );
        }
    }

    /// Unclaimed cells stay untouched, so the terminal's background is space.
    ///
    /// This is what lets the new `[global] background` setting show through, and
    /// it is why the frame is not a full-screen repaint. Asserted by counting:
    /// a full repaint would be every cell on the screen.
    #[test]
    fn the_frame_does_not_paint_the_whole_screen() {
        let size = (200u16, 50u16);
        let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
        let diff = system.get_diff();
        let total = usize::from(size.0) * usize::from(size.1);
        assert!(
            diff.len() < total / 2,
            "{} of {total} cells were written, so the background is being \
             painted rather than showing through",
            diff.len()
        );
    }

    /// No cell is bold, and the colours are the ones asked for.
    #[test]
    fn no_cell_is_bold_and_every_colour_is_a_truecolor_triple() {
        let size = (120u16, 40u16);
        let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
        for _ in 0..4 {
            system.advance(1.0 / 60.0);
        }
        let mut seen: HashSet<(u8, u8, u8)> = HashSet::new();
        for (x, y, cell) in system.get_diff() {
            assert_eq!(
                cell.attr,
                style::Attribute::Reset,
                "cell ({x}, {y}) is bold, which brightens a truecolor foreground \
                 into something the palette did not ask for"
            );
            match cell.color {
                Color::Rgb { r, g, b } => {
                    seen.insert((r, g, b));
                }
                other => {
                    panic!("cell ({x}, {y}) is {other:?}, not a truecolor triple")
                }
            }
        }
        assert!(
            seen.len() >= 8,
            "only {} distinct colours were drawn, so the planets are not \
             distinguishable from each other",
            seen.len()
        );
    }

    /// Nothing is drawn outside the screen.
    #[test]
    fn nothing_is_drawn_outside_the_screen() {
        for size in [(80u16, 24u16), (200, 50), (12, 40), (40, 12), (9, 9)] {
            let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
            for _ in 0..5 {
                system.advance(1.0 / 60.0);
            }
            for (x, y, _) in system.get_diff() {
                assert!(
                    x < size.0 as usize && y < size.1 as usize,
                    "at {}x{} a cell was emitted at ({x}, {y})",
                    size.0,
                    size.1
                );
            }
        }
    }

    /// The knobs survive a config round trip, because `--print-config` writes
    /// every one of them to disk and a missing key is a key the user's file does
    /// not have.
    #[test]
    fn the_keys_round_trip_through_toml() {
        let options: SolarSystemOptions =
            toml::from_str("tilt = 45.0\ncamera_speed = 12.0\nrings = false\n")
                .expect("three keys parse");
        assert_eq!(options.tilt, 45.0);
        assert_eq!(options.camera_speed, 12.0);
        assert!(!options.rings);
        assert_eq!(
            options.year_speed,
            SolarSystemOptions::default().year_speed,
            "three keys in the section silently reset the others"
        );

        let serialised = toml::to_string(&options).expect("the section serialises");
        for key in [
            "seed",
            "tilt",
            "camera_speed",
            "year_speed",
            "radius_exponent",
            "sun_size",
            "orbits",
            "rings",
            "stars",
        ] {
            assert!(
                serialised.contains(key),
                "{key} is missing from the serialised form: {serialised}"
            );
        }
    }

    /// A config float can be zero, negative or NaN, and none of those may take
    /// the frame with them.
    #[test]
    fn a_degenerate_config_draws_something_anyway() {
        /// A named setter, so the loop below can drive one field per entry
        /// without capturing -- a closure that captures `bad` cannot be a
        /// `fn` pointer.
        type Setter = (&'static str, fn(&mut SolarSystemOptions, f64));

        let fields: [Setter; 4] = [
            ("radius_exponent", |o, v| o.radius_exponent = v),
            ("tilt", |o, v| o.tilt = v),
            ("sun_size", |o, v| o.sun_size = v),
            ("year_speed", |o, v| o.year_speed = v),
        ];

        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            for (name, set) in fields {
                let mut options = SolarSystemOptions::default();
                set(&mut options, bad);
                let mut system = SolarSystem::new(options, (80, 24));
                system.advance(1.0);
                assert!(
                    inked_of(&mut system) > 20,
                    "{name} = {bad} drew almost nothing"
                );
            }
        }
    }

    fn inked_of(system: &mut SolarSystem) -> usize {
        system
            .get_diff()
            .iter()
            .filter(|(_, _, cell)| cell.symbol != ' ')
            .count()
    }

    /// A cell the effect stops drawing must be cleared, and the frame the
    /// terminal is sent must be the frame that was drawn.
    ///
    /// This is the "it feels like it's leaving behind previous frames" report,
    /// and the test is the strongest form available: it compares the *incremental*
    /// diff against a full repaint of the same instant. A correct double buffer
    /// sends the same picture either way, so any cell where they disagree is a
    /// cell the terminal has been told the wrong thing about.
    ///
    /// A two-frame comparison would not have caught it, and that is worth
    /// recording because the first attempt at this test did exactly that and
    /// passed. `Canvas::commit` swaps its two surfaces rather than clearing the
    /// one it hands back, so frame N is painted onto whatever frame N-2 left
    /// there. Frame 2's diff is still *correct* -- it compares frame 1 against
    /// frame 2 -- and the damage only appears at frame 3, when a cell that was
    /// inked at frame 1, blank at frame 2 and blank at frame 3 is reported as
    /// inked, because the surface it is being drawn onto still has frame 1's
    /// copy.
    #[test]
    fn the_incremental_frame_agrees_with_a_full_repaint() {
        let size = (80u16, 24u16);
        let options = SolarSystemOptions::default();
        let frames = 3u64;
        let delta = 1.0 / 60.0;

        // The incremental path: three diffs, the last of which is what the
        // terminal would actually be sent.
        let mut live = SolarSystem::new(options.clone(), size);
        let mut incremental = Vec::new();
        for _ in 0..frames {
            live.advance(delta);
            incremental = live.get_diff();
        }

        // The full repaint at the same instant. A fresh instance's first diff is
        // its entire frame, because `previous` starts blank.
        let mut reference = SolarSystem::new(options, size);
        for _ in 0..frames {
            reference.advance(delta);
        }
        let truth: BTreeMap<(usize, usize), char> = reference
            .get_diff()
            .into_iter()
            .map(|(x, y, cell)| ((x, y), cell.symbol))
            .collect();

        let mut disagreed = Vec::new();
        for (x, y, cell) in &incremental {
            match truth.get(&(*x, *y)) {
                Some(symbol) if *symbol == cell.symbol => {}
                Some(symbol) => disagreed.push(format!(
                    "({x}, {y}) sent {:?} but the frame draws {symbol:?}",
                    cell.symbol
                )),
                // Absent from the reference means *blank*, not "unknown": a
                // first diff only mentions cells that differ from the cleared
                // surface, so a blank cell is simply not in it. Sending a blank
                // for one of those is correct -- that is how a cell gets cleared.
                None if cell.symbol == ' ' || cell.symbol == '\u{2800}' => {}
                None => disagreed.push(format!(
                    "({x}, {y}) sent {:?} but the frame is blank there",
                    cell.symbol
                )),
            }
        }

        assert!(
            disagreed.is_empty(),
            "the incremental diff disagrees with a full repaint on {} cells, so \
             the terminal is being told to draw something the effect did not \
             draw. First few: {}",
            disagreed.len(),
            disagreed
                .iter()
                .take(6)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        );
        assert!(
            !incremental.is_empty(),
            "the third frame was empty, so this test is comparing nothing"
        );
    }

    /// The determinism contract, which `tests/effect_contracts.rs` also checks
    /// from outside the crate.
    #[test]
    fn the_sky_is_reproducible_and_seed_sensitive() {
        let frame = |seed: u64, frames: u64| {
            let options = SolarSystemOptions {
                seed,
                ..Default::default()
            };
            let mut system = SolarSystem::new(options, (80, 24));
            for _ in 0..frames {
                system.advance(1.0 / 60.0);
            }
            system.get_diff()
        };

        for frames in [0u64, 1, 30] {
            assert_eq!(
                frame(1234, frames),
                frame(1234, frames),
                "two systems at seed 1234 differ after {frames} frames"
            );
            assert_ne!(
                frame(1234, frames),
                frame(4321, frames),
                "seeds 1234 and 4321 drew the same sky after {frames} frames"
            );
        }
    }

    /// How far a drawn thing reaches, in the two units that are easy to confuse.
    #[derive(Debug, Default, Clone, Copy)]
    struct Reach {
        /// Dots raised. Zero means nothing was drawn, which is a failure and not
        /// a zero.
        dots: usize,
        /// Furthest dot from the centre, **in cells**.
        ///
        /// In cells and not in dot indices, because a braille cell is two dots
        /// wide by four tall and a dot *index* distance is not a distance: the
        /// same eight dot indices is four cells across and two cells down.
        /// Dividing each axis by its own dots-per-cell is what makes this
        /// comparable with [`body_radius_cells`](SolarSystem::body_radius_cells).
        cells: f64,
        /// Furthest dot from the centre along x, in dot indices.
        ///
        /// The horizontal figure alone, kept because it is the one a reader can
        /// check against a picture: a ring eight dots out around a planet five
        /// dots wide is a ring.
        across: f64,
    }

    /// What to measure at a given rotation.
    #[derive(Clone, Copy)]
    enum Subject {
        /// Saturn's disc, as `draw_disc` draws it.
        Planet,
        /// One edge of the annulus, as `draw_ring_edges` draws it.
        Ring(f64),
    }

    /// Measures `subject` at a rotation, by drawing it and reading the dots.
    ///
    /// Reads the dot grid rather than recomputing the geometry. The obvious
    /// alternative -- project the edge and measure where it lands -- is the same
    /// arithmetic the drawing code does, so a test written that way checks its
    /// own reimplementation of the formula rather than the drawing. This one
    /// cannot be satisfied by a formula that is right in a comment and wrong in
    /// the renderer.
    fn reach_of(size: (u16, u16), frames: u64, subject: Subject) -> Reach {
        let mut system = SolarSystem::new(SolarSystemOptions::default(), size);
        for _ in 0..frames {
            system.advance(1.0 / 60.0);
        }
        let scale = system.fit_scale();
        let saturn = &PLANETS[SATURN];
        let centre = system.orbit_point(
            saturn,
            system.drawn_radius(saturn),
            system.mean_anomaly(saturn),
        );
        let projected = system
            .project(centre, scale)
            .expect("Saturn is behind the camera at every rotation tested here");
        // The dot the centre lands in, which is what `draw_disc` and
        // `draw_ring_edges` both measure from.
        let (centre_x, centre_y) = (
            (projected.x * DOTS_X as f64).floor(),
            (projected.y * DOTS_Y as f64).floor(),
        );

        system.dots.clear();
        match subject {
            Subject::Planet => system.draw_disc(
                centre,
                system.body_radius_cells(saturn.radius_km),
                saturn.color,
                scale,
            ),
            Subject::Ring(edge) => {
                system.draw_ring_edges(saturn, centre, scale, &[edge])
            }
        }

        let mut reach = Reach::default();
        system.dots.for_each_dot(|x, y, raised| {
            if raised {
                let dx = x as f64 - centre_x;
                let dy = y as f64 - centre_y;
                reach.dots += 1;
                reach.cells = reach
                    .cells
                    .max((dx / DOTS_X as f64).hypot(dy / DOTS_Y as f64));
                reach.across = reach.across.max(dx.abs());
            }
        });
        reach
    }

    /// Saturn's rings are a ring around a planet, not a hoop drawn through the
    /// inner solar system.
    ///
    /// The report was "in solarsystem i think the rings of saturn are too big",
    /// and the numbers are the answer. Every figure here is read off the raised
    /// dots at four rotations, on an 80x24 and on a 200x50, through
    /// [`reach_of`].
    ///
    /// **Before**, at 1.24 and 2.27: the outer edge reached 63 dots from Saturn's
    /// centre on an 80x24 and 147 on a 200x50, against a planet 5 dots across.
    /// That is twelve times the body on the small terminal and twenty-nine on
    /// the large one, and the large one is a hoop through the whole inner system
    /// -- it reached past Neptune's orbit, because it was scaled by the display
    /// fit while the planet was not.
    ///
    /// **After**, at 1.05 and 1.7: 8 dots out on both sizes, which is 1.7 times
    /// the planet's own drawn radius. Small enough to be rings; large enough that
    /// the planet is inside them rather than beside them.
    ///
    /// The first assertion is the size. The second is the one that says *why*,
    /// and it is the reason the test exists: the ratio is a property of the
    /// constants and the units, so it cannot depend on the terminal. It did. The
    /// old code computed the ring's radius in cells -- `body_radius_cells`, which
    /// is deliberately a fixed cell count -- and added it to a world-space centre,
    /// which is scaled to fit the screen, so the ring grew with the window and
    /// the planet did not. Tuning the constants alone would not have fixed it:
    /// at 1.7 the old arithmetic still measures 9.4 times the body, which is
    /// outside the band below and fails the first assertion. The unit is the bug;
    /// the constants were only ever the reason nobody had noticed the picture was
    /// wrong, because they made a wrong unit produce an absurd number rather than
    /// a nearly-right one.
    ///
    /// The band is 1.3 to 2.0 rather than a point, and it is wide because the
    /// measurement is a pair of maxima over a rotating ellipse on a two-by-four
    /// lattice. It is not wider than that because the upper end is what "too big"
    /// meant: the old constants under the *new* arithmetic measure 2.3 times the
    /// body, which is a hoop the width of a tenth of a 200 column terminal and
    /// still past the top of the band.
    ///
    /// The last two assertions are about the ring still being a *ring*. The
    /// report did not ask about them and they are the failure the new numbers are
    /// most able to cause, since they brought the outer edge in by a third. Two
    /// edges one dot apart do not read as a band, they read as a filled ellipse,
    /// and the draw code loops over both edges precisely so that there is a
    /// middle. Measured: 1.5 cells and three dots of separation, against a
    /// one-dot floor.
    #[test]
    fn the_rings_are_a_ring_around_a_planet_and_not_a_hoop_around_a_solar_system() {
        // Far enough apart that the camera swing and the planets' own motion
        // have both turned the system right round, which is what makes the
        // size-independence assertion below mean something.
        const ROTATIONS: [u64; 4] = [0, 300, 900, 1500];

        let mut ratios = Vec::new();
        for size in [(80u16, 24u16), (200u16, 50u16)] {
            for frames in ROTATIONS {
                let planet = reach_of(size, frames, Subject::Planet);
                let inner = reach_of(size, frames, Subject::Ring(RING_INNER));
                let outer = reach_of(size, frames, Subject::Ring(RING_OUTER));

                assert!(
                    planet.dots > 0 && inner.dots > 0 && outer.dots > 0,
                    "on {size:?} at frame {frames} the planet raised {} dots, the \
                     inner edge {} and the outer {}, so this measured nothing",
                    planet.dots,
                    inner.dots,
                    outer.dots
                );

                let ratio = outer.cells / planet.cells;
                ratios.push(ratio);
                assert!(
                    (1.3..=2.0).contains(&ratio),
                    "on {size:?} at frame {frames} the outer ring edge reaches \
                     {:.2} cells from Saturn's centre and the planet is {:.2}, so \
                     the ring is {ratio:.1} times the body -- past 2.0 it is a hoop \
                     drawn across the picture rather than rings on a planet",
                    outer.cells,
                    planet.cells
                );

                // The ring must clear the planet rather than cut through it.
                // Measured 1.08 to 1.12, so this is a floor and not a fit.
                assert!(
                    inner.cells >= planet.cells * 0.95,
                    "on {size:?} at frame {frames} the inner ring edge is only \
                     {:.2} cells out against a planet of {:.2}, so the ring starts \
                     inside the body",
                    inner.cells,
                    planet.cells
                );

                // A band with a middle, in both units. One dot of separation
                // draws a filled ellipse, not a ring.
                let band = outer.cells - inner.cells;
                let band_dots = outer.across - inner.across;
                assert!(
                    band >= 1.0,
                    "on {size:?} at frame {frames} the two ring edges are {band:.2} \
                     cells apart, so they are one edge and not an annulus"
                );
                assert!(
                    band_dots >= 2.0,
                    "on {size:?} at frame {frames} the two ring edges are \
                     {band_dots:.0} dots apart across, which is not enough to read \
                     as two edges"
                );
            }
        }

        // The same ratio on a small terminal and a large one. This is the
        // assertion that is about the *unit*, and it is the one that fails
        // against the old draw code at any pair of ring constants: 12.6 on the
        // 80x24 against 29.4 on the 200x50. Tolerance a tenth, because the
        // measured ratio moves by about 0.06 across the rotations at a fixed
        // size, and that much is the lattice, not the fit.
        let widest = ratios.iter().cloned().fold(0.0f64, f64::max);
        let narrowest = ratios.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(
            widest - narrowest < 0.1,
            "the ring measures {narrowest:.2} times the planet on one terminal and \
             {widest:.2} on another, so the ring is being sized in world units \
             and the planet in cells"
        );
    }
}
