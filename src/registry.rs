use crate::ascii::AsciiField;
use crate::blank::Blank;
use crate::boids::Boids;
use crate::buffer::Cell;
use crate::common::TerminalEffect;
use crate::config::Config;
use crate::constellation::Constellation;
use crate::crab::Crab;
use crate::cube::Cube;
use crate::donut::Donut;
use crate::dvd::Dvd;
use crate::fire::Fire;
use crate::life::ConwayLife;
use crate::maze::Maze;
use crate::pipes::Pipes;
use crate::plasma::Plasma;
use crate::rain::digital_rain::DigitalRain;
use crate::runtime::{FrameContext, InputEvent};
use crate::terrain::Terrain;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum EffectId {
    Matrix,
    Life,
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
    Constellation,
    Ascii,
    Terrain,
}

impl EffectId {
    pub const ALL: &'static [Self] = &[
        Self::Matrix,
        Self::Life,
        Self::Maze,
        Self::Boids,
        Self::Blank,
        Self::Cube,
        Self::Crab,
        Self::Donut,
        Self::Dvd,
        Self::Pipes,
        Self::Plasma,
        Self::Fire,
        Self::Constellation,
        Self::Ascii,
        Self::Terrain,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matrix => "matrix",
            Self::Life => "life",
            Self::Maze => "maze",
            Self::Boids => "boids",
            Self::Blank => "blank",
            Self::Cube => "cube",
            Self::Crab => "crab",
            Self::Donut => "donut",
            Self::Dvd => "dvd",
            Self::Pipes => "pipes",
            Self::Plasma => "plasma",
            Self::Fire => "fire",
            Self::Constellation => "constellation",
            Self::Ascii => "ascii",
            Self::Terrain => "terrain",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Matrix => "Matrix digital rain",
            Self::Life => "Conway's Game of Life",
            Self::Maze => "Maze generation",
            Self::Boids => "Boids flocking simulation",
            Self::Blank => "Blank screen",
            Self::Cube => "3D cube rotation",
            Self::Crab => "ASCII crab animation",
            Self::Donut => "3D donut rotation",
            Self::Dvd => "Bouncing DVD logo",
            Self::Pipes => "Pipe maze animation",
            Self::Plasma => "Plasma effect",
            Self::Fire => "Fire simulation",
            Self::Constellation => "Drifting stars and dotted connections",
            Self::Ascii => "Interactive generative ASCII field",
            Self::Terrain => "Terrain generation",
        }
    }

    pub fn default_duration(self) -> f32 {
        match self {
            Self::Blank => 2.0,
            Self::Terrain => 4.0,
            Self::Fire => 12.0,
            Self::Matrix | Self::Crab | Self::Pipes | Self::Plasma => 15.0,
            Self::Donut | Self::Cube => 18.0,
            Self::Dvd => 12.0,
            Self::Maze
            | Self::Life
            | Self::Boids
            | Self::Constellation
            | Self::Ascii => 20.0,
        }
    }

    pub fn needs_mouse(self) -> bool {
        matches!(self, Self::Ascii)
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
        Self::ALL
            .iter()
            .copied()
            .find(|id| id.as_str() == value)
            .ok_or_else(|| format!("Unknown effect: {value}"))
    }
}

pub enum AnyEffect {
    Matrix(DigitalRain),
    Life(ConwayLife),
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
    Constellation(Constellation),
    Ascii(AsciiField),
    Terrain(Terrain),
}

impl AnyEffect {
    pub fn id(&self) -> EffectId {
        match self {
            Self::Matrix(_) => EffectId::Matrix,
            Self::Life(_) => EffectId::Life,
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
            Self::Constellation(_) => EffectId::Constellation,
            Self::Ascii(_) => EffectId::Ascii,
            Self::Terrain(_) => EffectId::Terrain,
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
            EffectId::Constellation => Self::Constellation(Constellation::new(
                config.get_constellation_options(),
                screen_size,
            )),
            EffectId::Ascii => Self::Ascii(AsciiField::new(
                config.get_ascii_options(),
                screen_size,
            )),
            EffectId::Terrain => Self::Terrain(Terrain::new(
                config.get_terrain_options(),
                screen_size,
            )),
        }
    }
}

impl TerminalEffect for AnyEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        match self {
            Self::Matrix(effect) => effect.get_diff(),
            Self::Life(effect) => effect.get_diff(),
            Self::Maze(effect) => effect.get_diff(),
            Self::Boids(effect) => effect.get_diff(),
            Self::Blank(effect) => effect.get_diff(),
            Self::Cube(effect) => effect.get_diff(),
            Self::Crab(effect) => effect.get_diff(),
            Self::Donut(effect) => effect.get_diff(),
            Self::Dvd(effect) => effect.get_diff(),
            Self::Pipes(effect) => effect.get_diff(),
            Self::Plasma(effect) => effect.get_diff(),
            Self::Fire(effect) => effect.get_diff(),
            Self::Constellation(effect) => effect.get_diff(),
            Self::Ascii(effect) => effect.get_diff(),
            Self::Terrain(effect) => effect.get_diff(),
        }
    }

    fn update(&mut self) {
        match self {
            Self::Matrix(effect) => effect.update(),
            Self::Life(effect) => effect.update(),
            Self::Maze(effect) => effect.update(),
            Self::Boids(effect) => effect.update(),
            Self::Blank(effect) => effect.update(),
            Self::Cube(effect) => effect.update(),
            Self::Crab(effect) => effect.update(),
            Self::Donut(effect) => effect.update(),
            Self::Dvd(effect) => effect.update(),
            Self::Pipes(effect) => effect.update(),
            Self::Plasma(effect) => effect.update(),
            Self::Fire(effect) => effect.update(),
            Self::Constellation(effect) => effect.update(),
            Self::Ascii(effect) => effect.update(),
            Self::Terrain(effect) => effect.update(),
        }
    }

    fn update_size(&mut self, width: u16, height: u16) {
        match self {
            Self::Matrix(effect) => effect.update_size(width, height),
            Self::Life(effect) => effect.update_size(width, height),
            Self::Maze(effect) => effect.update_size(width, height),
            Self::Boids(effect) => effect.update_size(width, height),
            Self::Blank(effect) => effect.update_size(width, height),
            Self::Cube(effect) => effect.update_size(width, height),
            Self::Crab(effect) => effect.update_size(width, height),
            Self::Donut(effect) => effect.update_size(width, height),
            Self::Dvd(effect) => effect.update_size(width, height),
            Self::Pipes(effect) => effect.update_size(width, height),
            Self::Plasma(effect) => effect.update_size(width, height),
            Self::Fire(effect) => effect.update_size(width, height),
            Self::Constellation(effect) => effect.update_size(width, height),
            Self::Ascii(effect) => effect.update_size(width, height),
            Self::Terrain(effect) => effect.update_size(width, height),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Matrix(effect) => effect.reset(),
            Self::Life(effect) => effect.reset(),
            Self::Maze(effect) => effect.reset(),
            Self::Boids(effect) => effect.reset(),
            Self::Blank(effect) => effect.reset(),
            Self::Cube(effect) => effect.reset(),
            Self::Crab(effect) => effect.reset(),
            Self::Donut(effect) => effect.reset(),
            Self::Dvd(effect) => effect.reset(),
            Self::Pipes(effect) => effect.reset(),
            Self::Plasma(effect) => effect.reset(),
            Self::Fire(effect) => effect.reset(),
            Self::Constellation(effect) => effect.reset(),
            Self::Ascii(effect) => effect.reset(),
            Self::Terrain(effect) => effect.reset(),
        }
    }

    fn handle_input(&mut self, event: &InputEvent) {
        match self {
            Self::Matrix(effect) => effect.handle_input(event),
            Self::Life(effect) => effect.handle_input(event),
            Self::Maze(effect) => effect.handle_input(event),
            Self::Boids(effect) => effect.handle_input(event),
            Self::Blank(effect) => effect.handle_input(event),
            Self::Cube(effect) => effect.handle_input(event),
            Self::Crab(effect) => effect.handle_input(event),
            Self::Donut(effect) => effect.handle_input(event),
            Self::Dvd(effect) => effect.handle_input(event),
            Self::Pipes(effect) => effect.handle_input(event),
            Self::Plasma(effect) => effect.handle_input(event),
            Self::Fire(effect) => effect.handle_input(event),
            Self::Constellation(effect) => effect.handle_input(event),
            Self::Ascii(effect) => effect.handle_input(event),
            Self::Terrain(effect) => effect.handle_input(event),
        }
    }

    fn get_diff_with_context(
        &mut self,
        context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        match self {
            Self::Matrix(effect) => effect.get_diff_with_context(context),
            Self::Life(effect) => effect.get_diff_with_context(context),
            Self::Maze(effect) => effect.get_diff_with_context(context),
            Self::Boids(effect) => effect.get_diff_with_context(context),
            Self::Blank(effect) => effect.get_diff_with_context(context),
            Self::Cube(effect) => effect.get_diff_with_context(context),
            Self::Crab(effect) => effect.get_diff_with_context(context),
            Self::Donut(effect) => effect.get_diff_with_context(context),
            Self::Dvd(effect) => effect.get_diff_with_context(context),
            Self::Pipes(effect) => effect.get_diff_with_context(context),
            Self::Plasma(effect) => effect.get_diff_with_context(context),
            Self::Fire(effect) => effect.get_diff_with_context(context),
            Self::Constellation(effect) => effect.get_diff_with_context(context),
            Self::Ascii(effect) => effect.get_diff_with_context(context),
            Self::Terrain(effect) => effect.get_diff_with_context(context),
        }
    }

    fn update_with_context(&mut self, context: &FrameContext) {
        match self {
            Self::Matrix(effect) => effect.update_with_context(context),
            Self::Life(effect) => effect.update_with_context(context),
            Self::Maze(effect) => effect.update_with_context(context),
            Self::Boids(effect) => effect.update_with_context(context),
            Self::Blank(effect) => effect.update_with_context(context),
            Self::Cube(effect) => effect.update_with_context(context),
            Self::Crab(effect) => effect.update_with_context(context),
            Self::Donut(effect) => effect.update_with_context(context),
            Self::Dvd(effect) => effect.update_with_context(context),
            Self::Pipes(effect) => effect.update_with_context(context),
            Self::Plasma(effect) => effect.update_with_context(context),
            Self::Fire(effect) => effect.update_with_context(context),
            Self::Constellation(effect) => effect.update_with_context(context),
            Self::Ascii(effect) => effect.update_with_context(context),
            Self::Terrain(effect) => effect.update_with_context(context),
        }
    }
}
