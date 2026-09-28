use crate::ants::Ants;
use crate::blank::Blank;
use crate::boids::Boids;
use crate::buffer::Cell;
use crate::common::TerminalEffect;
use crate::config::Config;
use crate::crab::Crab;
use crate::cube::Cube;
use crate::donut::Donut;
use crate::dvd::Dvd;
use crate::fire::Fire;
use crate::flyover::Flyover;
use crate::ink::AsciiField;
use crate::life::ConwayLife;
use crate::mandelbrot::Mandelbrot;
use crate::maze::Maze;
use crate::newton::Newton;
use crate::physarum::Physarum;
use crate::pipes::Pipes;
use crate::plasma::Plasma;
use crate::rain::digital_rain::DigitalRain;
use crate::ripple::Ripple;
use crate::runtime::{FrameContext, InputEvent};
use crate::solarsystem::SolarSystem;
use crate::terrain::Terrain;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(into = "String", try_from = "String")]
pub enum EffectId {
    Matrix,
    Life,
    Mandelbrot,
    Maze,
    Boids,
    Blank,
    Cube,
    Crab,
    Donut,
    Dvd,
    Pipes,
    Plasma,
    Fire,
    SolarSystem,
    Ink,
    Terrain,
    Ants,
    Flyover,
    Physarum,
    Ripple,
    Newton,
}

/// Everything the CLI needs to know about an effect, in one place.
///
/// This table is the single source of truth. It used to be five separate
/// hand-maintained lists -- the `ALL` array, `as_str`, `description`,
/// `default_duration` and `needs_mouse` -- and forgetting to update the `ALL`
/// array was the dangerous one: the effect still compiled, but became invisible
/// to `--help`, to argument parsing and to playlists, with nothing to say so.
///
/// Adding an effect is now one entry here. A variant with no entry is caught by
/// `every_effect_id_has_a_spec_entry`.
pub struct EffectSpec {
    pub id: EffectId,
    /// Name accepted on the command line and in the config file.
    pub name: &'static str,
    /// One line shown in `--help`.
    pub description: &'static str,
    /// Seconds this effect runs for in a playlist that does not say.
    pub default_duration: f32,
    /// Whether the effect reads the mouse and so needs capture enabled.
    pub needs_mouse: bool,
    /// Key of this effect's section in the config file.
    pub config_section: &'static str,
}

pub static EFFECT_SPECS: &[EffectSpec] = &[
    EffectSpec {
        id: EffectId::Matrix,
        name: "matrix",
        description: "Matrix digital rain",
        default_duration: 15.0,
        needs_mouse: false,
        config_section: "matrix",
    },
    EffectSpec {
        id: EffectId::Life,
        name: "life",
        description: "Conway's Game of Life",
        default_duration: 20.0,
        needs_mouse: false,
        config_section: "life",
    },
    EffectSpec {
        id: EffectId::Mandelbrot,
        name: "mandelbrot",
        description: "Escape-time Mandelbrot set, slowly zooming",
        default_duration: 20.0,
        needs_mouse: false,
        config_section: "mandelbrot",
    },
    EffectSpec {
        id: EffectId::Maze,
        name: "maze",
        description: "Maze generation",
        default_duration: 20.0,
        needs_mouse: false,
        config_section: "maze",
    },
    EffectSpec {
        id: EffectId::Boids,
        name: "boids",
        description: "Boids flocking simulation, scattered by clicking",
        default_duration: 20.0,
        // A click drops a shockwave into the flock, so this effect reads the
        // mouse. Not optional bookkeeping: `main.rs` decides whether to enable
        // capture from *the playlist's own effects*, so a playlist of `boids`
        // alone would otherwise never enable it and the effect would be
        // interactive in exactly one of the three ways it can be run.
        needs_mouse: true,
        config_section: "boids",
    },
    EffectSpec {
        id: EffectId::Blank,
        name: "blank",
        description: "Blank screen",
        default_duration: 2.0,
        needs_mouse: false,
        config_section: "blank",
    },
    EffectSpec {
        id: EffectId::Cube,
        name: "cube",
        description: "3D cube rotation",
        default_duration: 18.0,
        needs_mouse: false,
        config_section: "cube",
    },
    EffectSpec {
        id: EffectId::Crab,
        name: "crab",
        description: "ASCII crab animation",
        default_duration: 15.0,
        needs_mouse: false,
        config_section: "crab",
    },
    EffectSpec {
        id: EffectId::Donut,
        name: "donut",
        description: "3D donut rotation",
        default_duration: 18.0,
        needs_mouse: false,
        config_section: "donut",
    },
    EffectSpec {
        id: EffectId::Dvd,
        name: "dvd",
        description: "Bouncing DVD logo",
        default_duration: 12.0,
        needs_mouse: false,
        config_section: "dvd",
    },
    EffectSpec {
        id: EffectId::Pipes,
        name: "pipes",
        description: "Pipe maze animation",
        default_duration: 15.0,
        needs_mouse: false,
        config_section: "pipes",
    },
    EffectSpec {
        id: EffectId::Plasma,
        name: "plasma",
        description: "Plasma color wave effect",
        default_duration: 15.0,
        needs_mouse: false,
        config_section: "plasma",
    },
    EffectSpec {
        id: EffectId::Fire,
        name: "fire",
        description: "Fire simulation",
        default_duration: 12.0,
        needs_mouse: false,
        config_section: "fire",
    },
    EffectSpec {
        id: EffectId::SolarSystem,
        name: "solarsystem",
        description: "A solar system on tilted orbits, real orbital periods",
        default_duration: 20.0,
        needs_mouse: false,
        config_section: "solarsystem",
    },
    EffectSpec {
        id: EffectId::Ink,
        // Was "ascii", which named the medium rather than the effect: it draws
        // characters, but so do eleven others, and the thing you interact with is
        // ink poured into a field. Renamed with no alias, so an old `[ascii]`
        // config section is ignored and an old `effect = "ascii"` playlist entry
        // is dropped -- see `build_slots`, which now reports the latter rather
        // than swallowing it.
        name: "ink",
        description: "Interactive generative field you pour ink into",
        default_duration: 20.0,
        needs_mouse: true,
        config_section: "ink",
    },
    EffectSpec {
        id: EffectId::Terrain,
        name: "terrain",
        description: "Terrain generation",
        default_duration: 4.0,
        needs_mouse: false,
        config_section: "terrain",
    },
    EffectSpec {
        id: EffectId::Ants,
        name: "ants",
        // "Langton's ants" rather than "Langton's ant": the effect is several of
        // them on one board, and the whole point of that is that they read each
        // other's flips and wreck each other's highways. Naming it in the singular
        // would describe a ten-line effect that is not this one.
        description: "Several Langton's ants sharing one board",
        default_duration: 30.0,
        needs_mouse: false,
        config_section: "ants",
    },
    EffectSpec {
        id: EffectId::Flyover,
        name: "flyover",
        description: "First-person flight over a fractal height field",
        default_duration: 25.0,
        needs_mouse: false,
        config_section: "flyover",
    },
    EffectSpec {
        id: EffectId::Physarum,
        name: "physarum",
        description: "Slime-mould agents building a transport network",
        default_duration: 30.0,
        needs_mouse: false,
        config_section: "physarum",
    },
    EffectSpec {
        id: EffectId::Ripple,
        name: "ripple",
        description: "Interfering waves from a few point sources",
        default_duration: 20.0,
        needs_mouse: false,
        config_section: "ripple",
    },
    EffectSpec {
        id: EffectId::Newton,
        // "Newton's method", and the description carries the part that matters.
        // A name alone does not say what is on screen: the mandelbrot is also an
        // iteration on the complex plane, and the two are close enough that
        // "Newton fractal" as a bare name would leave a user who has just watched
        // `mandelbrot` with no reason to expect a different picture. What
        // distinguishes this one is the colour, so that is what the line says.
        name: "newton",
        description: "Newton's method basins, coloured by which root each sample finds",
        default_duration: 25.0,
        needs_mouse: false,
        config_section: "newton",
    },
];

impl EffectId {
    /// Every registered effect, in the order they appear in `--help` and in a
    /// playlist that does not choose.
    pub fn all() -> impl Iterator<Item = EffectId> {
        EFFECT_SPECS.iter().map(|spec| spec.id)
    }

    pub fn len() -> usize {
        EFFECT_SPECS.len()
    }

    pub fn is_empty() -> bool {
        EFFECT_SPECS.is_empty()
    }

    /// The spec for this effect.
    ///
    /// Panics if a variant has no entry, which is a programming error rather
    /// than a user error. `every_effect_id_has_a_spec_entry` turns that panic
    /// into a test failure.
    pub fn spec(self) -> &'static EffectSpec {
        EFFECT_SPECS
            .iter()
            .find(|spec| spec.id == self)
            .unwrap_or_else(|| panic!("{self:?} has no entry in EFFECT_SPECS"))
    }

    pub fn as_str(self) -> &'static str {
        self.spec().name
    }

    pub fn description(self) -> &'static str {
        self.spec().description
    }

    pub fn default_duration(self) -> f32 {
        self.spec().default_duration
    }

    pub fn needs_mouse(self) -> bool {
        self.spec().needs_mouse
    }

    /// Key of this effect's section in the config file.
    pub fn config_section(self) -> &'static str {
        self.spec().config_section
    }
}

impl From<EffectId> for String {
    fn from(value: EffectId) -> Self {
        value.as_str().to_string()
    }
}

impl TryFrom<String> for EffectId {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl FromStr for EffectId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        EFFECT_SPECS
            .iter()
            .find(|spec| spec.name == value)
            .map(|spec| spec.id)
            .ok_or_else(|| format!("Unknown effect: {value}"))
    }
}

pub enum AnyEffect {
    Matrix(DigitalRain),
    Life(ConwayLife),
    Mandelbrot(Mandelbrot),
    Maze(Maze),
    Boids(Boids),
    Blank(Blank),
    Cube(Cube),
    Crab(Crab),
    Donut(Donut),
    Dvd(Dvd),
    Pipes(Pipes),
    Plasma(Plasma),
    Fire(Fire),
    SolarSystem(SolarSystem),
    Ink(AsciiField),
    Terrain(Terrain),
    Ants(Ants),
    Flyover(Flyover),
    Physarum(Physarum),
    Ripple(Ripple),
    Newton(Newton),
}

impl AnyEffect {
    pub fn id(&self) -> EffectId {
        match self {
            Self::Matrix(_) => EffectId::Matrix,
            Self::Life(_) => EffectId::Life,
            Self::Mandelbrot(_) => EffectId::Mandelbrot,
            Self::Maze(_) => EffectId::Maze,
            Self::Boids(_) => EffectId::Boids,
            Self::Blank(_) => EffectId::Blank,
            Self::Cube(_) => EffectId::Cube,
            Self::Crab(_) => EffectId::Crab,
            Self::Donut(_) => EffectId::Donut,
            Self::Dvd(_) => EffectId::Dvd,
            Self::Pipes(_) => EffectId::Pipes,
            Self::Plasma(_) => EffectId::Plasma,
            Self::Fire(_) => EffectId::Fire,
            Self::SolarSystem(_) => EffectId::SolarSystem,
            Self::Ink(_) => EffectId::Ink,
            Self::Terrain(_) => EffectId::Terrain,
            Self::Ants(_) => EffectId::Ants,
            Self::Flyover(_) => EffectId::Flyover,
            Self::Physarum(_) => EffectId::Physarum,
            Self::Ripple(_) => EffectId::Ripple,
            Self::Newton(_) => EffectId::Newton,
        }
    }

    pub fn build(id: EffectId, config: &Config, screen_size: (u16, u16)) -> Self {
        match id {
            EffectId::Matrix => Self::Matrix(DigitalRain::new(
                config.get_matrix_options(screen_size),
                screen_size,
            )),
            EffectId::Life => Self::Life(ConwayLife::new(
                config.get_life_options(screen_size),
                screen_size,
            )),
            EffectId::Mandelbrot => Self::Mandelbrot(Mandelbrot::new(
                config.get_mandelbrot_options(),
                screen_size,
            )),
            EffectId::Maze => Self::Maze(Maze::new(
                config.get_maze_options(screen_size),
                screen_size,
            )),
            EffectId::Boids => {
                Self::Boids(Boids::new(config.get_boids_options(screen_size)))
            }
            EffectId::Blank => {
                Self::Blank(Blank::new(config.get_blank_options(), screen_size))
            }
            EffectId::Cube => {
                Self::Cube(Cube::new(config.get_cube_options(), screen_size))
            }
            EffectId::Crab => Self::Crab(Crab::new(
                config.get_crab_options(screen_size),
                screen_size,
            )),
            EffectId::Donut => Self::Donut(Donut::new(
                config.get_donut_options(screen_size),
                screen_size,
            )),
            EffectId::Dvd => {
                Self::Dvd(Dvd::new(config.get_dvd_options(), screen_size))
            }
            EffectId::Pipes => {
                Self::Pipes(Pipes::new(config.get_pipes_options(), screen_size))
            }
            EffectId::Plasma => {
                Self::Plasma(Plasma::new(config.get_plasma_options(), screen_size))
            }
            EffectId::Fire => {
                Self::Fire(Fire::new(config.get_fire_options(), screen_size))
            }
            EffectId::SolarSystem => Self::SolarSystem(SolarSystem::new(
                config.get_solarsystem_options(),
                screen_size,
            )),
            EffectId::Ink => {
                Self::Ink(AsciiField::new(config.get_ink_options(), screen_size))
            }
            EffectId::Terrain => Self::Terrain(Terrain::new(
                config.get_terrain_options(),
                screen_size,
            )),
            EffectId::Ants => Self::Ants(Ants::new(
                config.get_ants_options(screen_size),
                screen_size,
            )),
            EffectId::Flyover => Self::Flyover(Flyover::new(
                config.get_flyover_options(),
                screen_size,
            )),
            EffectId::Physarum => Self::Physarum(Physarum::new(
                config.get_physarum_options(screen_size),
                screen_size,
            )),
            EffectId::Ripple => {
                Self::Ripple(Ripple::new(config.get_ripple_options(), screen_size))
            }
            EffectId::Newton => {
                Self::Newton(Newton::new(config.get_newton_options(), screen_size))
            }
        }
    }
}

/// Forwards every `TerminalEffect` method to the wrapped effect.
///
/// The six forwarding methods used to be written out by hand, sixteen arms
/// each, which is ninety-six near-identical lines. The variant list below is the
/// only thing that has to be maintained.
///
/// Forgetting a variant is a compile error rather than a silent gap: the
/// `match` inside each method stops being exhaustive, and `AnyEffect` is used as
/// a `TerminalEffect` everywhere.
macro_rules! impl_terminal_effect_for_any {
    ($($variant:ident),+ $(,)?) => {
        impl TerminalEffect for AnyEffect {
            fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
                match self {
                    $(Self::$variant(effect) => effect.get_diff(),)+
                }
            }

            fn update(&mut self) {
                match self {
                    $(Self::$variant(effect) => effect.update(),)+
                }
            }

            fn update_size(&mut self, width: u16, height: u16) {
                match self {
                    $(Self::$variant(effect) => effect.update_size(width, height),)+
                }
            }

            fn reset(&mut self) {
                match self {
                    $(Self::$variant(effect) => effect.reset(),)+
                }
            }

            fn handle_input(&mut self, event: &InputEvent) {
                match self {
                    $(Self::$variant(effect) => effect.handle_input(event),)+
                }
            }

            fn get_diff_with_context(
                &mut self,
                context: &FrameContext,
            ) -> Vec<(usize, usize, Cell)> {
                match self {
                    $(Self::$variant(effect) => effect.get_diff_with_context(context),)+
                }
            }

            fn update_with_context(&mut self, context: &FrameContext) {
                match self {
                    $(Self::$variant(effect) => effect.update_with_context(context),)+
                }
            }
        }
    };
}

impl_terminal_effect_for_any!(
    Matrix,
    Life,
    Mandelbrot,
    Maze,
    Boids,
    Blank,
    Cube,
    Crab,
    Donut,
    Dvd,
    Pipes,
    Plasma,
    Fire,
    SolarSystem,
    Ink,
    Terrain,
    Ants,
    Flyover,
    Physarum,
    Ripple,
    Newton,
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every effect id, written out.
    ///
    /// This is the one place a new effect has to be added twice, and that is
    /// deliberate: it is a change detector. Adding an effect without adding it
    /// here fails the build's test run rather than passing quietly.
    const KNOWN_IDS: &[EffectId] = &[
        EffectId::Matrix,
        EffectId::Life,
        EffectId::Mandelbrot,
        EffectId::Maze,
        EffectId::Boids,
        EffectId::Blank,
        EffectId::Cube,
        EffectId::Crab,
        EffectId::Donut,
        EffectId::Dvd,
        EffectId::Pipes,
        EffectId::Plasma,
        EffectId::Fire,
        EffectId::SolarSystem,
        EffectId::Ink,
        EffectId::Terrain,
        EffectId::Ants,
        EffectId::Flyover,
        EffectId::Physarum,
        EffectId::Ripple,
        EffectId::Newton,
    ];

    #[test]
    fn every_effect_id_has_a_spec_entry() {
        // A variant with no entry would panic at runtime, and in the meantime be
        // invisible to `--help`, to argument parsing and to playlists.
        for id in KNOWN_IDS {
            let spec = id.spec();
            assert_eq!(spec.id, *id);
        }
        assert_eq!(
            EFFECT_SPECS.len(),
            KNOWN_IDS.len(),
            "EFFECT_SPECS has {} entries but there are {} effect ids",
            EFFECT_SPECS.len(),
            KNOWN_IDS.len()
        );
    }

    #[test]
    fn the_table_lists_each_effect_exactly_once() {
        let mut seen = HashSet::new();
        for spec in EFFECT_SPECS {
            assert!(
                seen.insert(spec.id),
                "{} appears twice in EFFECT_SPECS",
                spec.id.as_str()
            );
        }
    }

    #[test]
    fn names_and_sections_are_unique_and_well_formed() {
        let mut names = HashSet::new();
        let mut sections = HashSet::new();

        for spec in EFFECT_SPECS {
            assert!(
                names.insert(spec.name),
                "two effects share the name {:?}",
                spec.name
            );
            assert!(
                sections.insert(spec.config_section),
                "two effects share the config section {:?}",
                spec.config_section
            );
            assert!(
                spec.name.chars().all(|c| c.is_ascii_alphanumeric()),
                "{:?} is not a plain lowercase name",
                spec.name
            );
            assert_eq!(
                spec.config_section, spec.name,
                "{} should use its own name as its config section",
                spec.name
            );
        }
    }

    #[test]
    fn every_effect_has_usable_metadata() {
        for spec in EFFECT_SPECS {
            assert!(
                !spec.description.trim().is_empty(),
                "{} has an empty description, which would render a blank help line",
                spec.name
            );
            assert!(
                spec.default_duration.is_finite() && spec.default_duration > 0.0,
                "{} has a non-positive playlist duration ({})",
                spec.name,
                spec.default_duration
            );
        }
    }

    #[test]
    fn names_round_trip_through_parsing() {
        for spec in EFFECT_SPECS {
            let parsed: EffectId = spec.name.parse().expect("parses");
            assert_eq!(parsed, spec.id, "{} did not round-trip", spec.name);
            assert_eq!(parsed.as_str(), spec.name);
        }
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert!("not-an-effect".parse::<EffectId>().is_err());
        assert!("".parse::<EffectId>().is_err());
        // A near miss must not resolve.
        assert!("Matrix".parse::<EffectId>().is_err());
        assert!("dvd ".parse::<EffectId>().is_err());
    }

    #[test]
    fn the_conversion_into_a_string_uses_the_registered_name() {
        for spec in EFFECT_SPECS {
            assert_eq!(String::from(spec.id), spec.name);
            assert_eq!(EffectId::try_from(spec.name.to_string()), Ok(spec.id));
        }
    }
}
