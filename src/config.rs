use crate::{
    ascii::{AsciiFieldOptions, AsciiFieldOptionsBuilder},
    blank::{BlankOptions, BlankOptionsBuilder},
    boids::{BoidsOptions, BoidsOptionsBuilder},
    constellation::{ConstellationOptions, ConstellationOptionsBuilder},
    crab::{CrabOptions, CrabOptionsBuilder},
    cube::{CubeOptions, CubeOptionsBuilder},
    donut::{DonutOptions, DonutOptionsBuilder},
    dvd::{DvdOptions, DvdOptionsBuilder},
    error::{ConfigError, Result, TermzzzError},
    fire::{FireOptions, FireOptionsBuilder},
    life::{ConwayLifeOptions, ConwayLifeOptionsBuilder},
    maze::{MazeOptions, MazeOptionsBuilder},
    pipes::{PipesOptions, PipesOptionsBuilder},
    plasma::{PlasmaOptions, PlasmaOptionsBuilder},
    playlist::{PlaylistOptions, PlaylistOptionsBuilder},
    rain::digital_rain::{DigitalRainOptions, DigitalRainOptionsBuilder},
    terrain::{TerrainOptions, TerrainOptionsBuilder},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| {
            directories::BaseDirs::new()
                .unwrap()
                .home_dir()
                .join("AppData/Roaming")
                .display()
                .to_string()
        });
        PathBuf::from(appdata).join("termzzz.toml")
    }
    #[cfg(not(target_os = "windows"))]
    {
        directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .join(".config/termzzz.toml")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalOptions {
    pub speed: f32,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self { speed: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub global: GlobalOptions,
    #[serde(default)]
    pub matrix: DigitalRainOptions,
    #[serde(default)]
    pub life: ConwayLifeOptions,
    #[serde(default)]
    pub maze: MazeOptions,
    #[serde(default)]
    pub boids: BoidsOptions,
    #[serde(default)]
    pub ascii: AsciiFieldOptions,
    #[serde(default)]
    pub blank: BlankOptions,
    #[serde(default)]
    pub cube: CubeOptions,
    #[serde(default)]
    pub crab: CrabOptions,
    #[serde(default)]
    pub donut: DonutOptions,
    #[serde(default)]
    pub dvd: DvdOptions,
    #[serde(default)]
    pub pipes: PipesOptions,
    #[serde(default)]
    pub plasma: PlasmaOptions,
    #[serde(default)]
    pub fire: FireOptions,
    #[serde(default)]
    pub terrain: TerrainOptions,
    #[serde(default)]
    pub constellation: ConstellationOptions,
    #[serde(default)]
    pub playlist: PlaylistOptions,
}

impl Config {
    /// Print default config as TOML to stdout (for piping into a file).
    pub fn print_default_config() -> Result<()> {
        let contents = toml::to_string_pretty(&Config::default())
            .map_err(|e| TermzzzError::Config(ConfigError::SerializeFormat(e)))?;
        println!("{}", contents);
        Ok(())
    }

    /// Load config from platform path. Returns the config and a status message.
    /// If no config file exists, returns default config in memory (does NOT write to disk).
    pub fn load() -> Result<(Self, String)> {
        let path = config_path();
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            let config = toml::from_str(&contents).map_err(|e| {
                TermzzzError::Config(ConfigError::DeserializeFormat(e))
            })?;
            Ok((config, format!("Loaded config from {}", path.display())))
        } else {
            Ok((
                Config::default(),
                format!("No config found at {}, using defaults", path.display()),
            ))
        }
    }
}

impl Config {
    pub fn get_global_options(&self) -> GlobalOptions {
        self.global.clone()
    }

    pub fn get_matrix_options(
        &self,
        screen_size: (u16, u16),
    ) -> DigitalRainOptions {
        let mut options = self.matrix.clone();
        let (w, h) = screen_size;
        let area = w as f32 * h as f32;
        options.drops_range = {
            let min = (area / 160.0 * options.drops_coeff) as u16;
            let max = (area / 80.0 * options.drops_coeff) as u16;
            (min.max(10), max.max(20))
        };
        options.speed_range = {
            let min = ((h as f32 / 20.0 * options.speed_coeff) as u16).max(2);
            let max = ((h as f32 / 10.0 * options.speed_coeff) as u16).max(16);
            (min, max)
        };
        options
    }

    pub fn get_life_options(&self, screen_size: (u16, u16)) -> ConwayLifeOptions {
        let mut options = self.life.clone();
        let (w, h) = screen_size;
        options.initial_cells =
            (w as f32 * h as f32 * 0.15 * options.cells_coeff) as u32;
        options
    }

    pub fn get_maze_options(&self, _screen_size: (u16, u16)) -> MazeOptions {
        self.maze.clone()
    }

    pub fn get_boids_options(&self, screen_size: (u16, u16)) -> BoidsOptions {
        let mut options = self.boids.clone();
        options.screen_size = screen_size;
        let (w, h) = screen_size;
        options.boid_count = ((w as f32 * h as f32 * 0.5 * options.boid_coeff)
            as u16)
            .clamp(50, 300);
        options
    }

    pub fn get_ascii_options(&self) -> AsciiFieldOptions {
        self.ascii.clone()
    }

    pub fn get_blank_options(&self) -> BlankOptions {
        self.blank.clone()
    }

    pub fn get_cube_options(&self) -> CubeOptions {
        self.cube.clone()
    }

    pub fn get_crab_options(&self, screen_size: (u16, u16)) -> CrabOptions {
        let mut options = self.crab.clone();
        let screen_area = screen_size.0 as f32 * screen_size.1 as f32;
        options.crab_count =
            (screen_area / 800.0 * options.crab_coeff).clamp(3.0, 15.0) as u16;
        options
    }

    pub fn get_donut_options(&self, screen_size: (u16, u16)) -> DonutOptions {
        let mut options = self.donut.clone();
        let min_dim = screen_size.0.min(screen_size.1) as f32;
        options.k1 = min_dim * 0.8 * options.k1_coeff;
        options
    }

    pub fn get_dvd_options(&self) -> DvdOptions {
        self.dvd.clone()
    }

    pub fn get_pipes_options(&self) -> PipesOptions {
        self.pipes.clone()
    }

    pub fn get_plasma_options(&self) -> PlasmaOptions {
        self.plasma.clone()
    }

    pub fn get_fire_options(&self) -> FireOptions {
        self.fire.clone()
    }

    pub fn get_terrain_options(&self) -> TerrainOptions {
        self.terrain.clone()
    }

    pub fn get_constellation_options(&self) -> ConstellationOptions {
        self.constellation.clone()
    }

    pub fn get_playlist_options(&self) -> PlaylistOptions {
        self.playlist.clone()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            global: GlobalOptions::default(),
            matrix: DigitalRainOptionsBuilder::default().build().unwrap(),
            life: ConwayLifeOptionsBuilder::default().build().unwrap(),
            maze: MazeOptionsBuilder::default().build().unwrap(),
            boids: BoidsOptionsBuilder::default().build().unwrap(),
            ascii: AsciiFieldOptionsBuilder::default().build().unwrap(),
            blank: BlankOptionsBuilder::default().build().unwrap(),
            cube: CubeOptionsBuilder::default().build().unwrap(),
            crab: CrabOptionsBuilder::default().build().unwrap(),
            donut: DonutOptionsBuilder::default().build().unwrap(),
            dvd: DvdOptionsBuilder::default().build().unwrap(),
            pipes: PipesOptionsBuilder::default().build().unwrap(),
            plasma: PlasmaOptionsBuilder::default().build().unwrap(),
            fire: FireOptionsBuilder::default().build().unwrap(),
            terrain: TerrainOptionsBuilder::default().build().unwrap(),
            constellation: ConstellationOptionsBuilder::default().build().unwrap(),
            playlist: PlaylistOptionsBuilder::default()
                .transition(0.6_f32)
                .build()
                .unwrap(),
        }
    }
}
