use crate::{
    blank::BlankOptions,
    boids::BoidsOptions,
    constellation::ConstellationOptions,
    crab::CrabOptions,
    cube::CubeOptions,
    donut::DonutOptions,
    dvd::DvdOptions,
    error::{ConfigError, Result, TermzzzError},
    fire::FireOptions,
    ink::AsciiFieldOptions,
    life::ConwayLifeOptions,
    mandelbrot::MandelbrotOptions,
    maze::MazeOptions,
    pipes::PipesOptions,
    plasma::PlasmaOptions,
    playlist::PlaylistOptions,
    rain::digital_rain::DigitalRainOptions,
    terrain::TerrainOptions,
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
    /// Whether to slow down when the terminal window does not have focus.
    ///
    /// A screensaver nobody is looking at should not be costing a core. It
    /// throttles rather than stops, deliberately: a terminal that never reports
    /// a focus change would otherwise leave the program frozen with no way to
    /// wake it, whereas this degrades to a quietly slow screensaver.
    pub pause_when_unfocused: bool,
    /// Frames per second while unfocused, when `pause_when_unfocused` is set.
    ///
    /// Not zero. Stopping outright is the cheapest option and the most dangerous
    /// one, for the reason above. Four is a fifteenth of the work and still
    /// visibly alive if you alt-tab back and watch for a moment.
    pub idle_fps: f32,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self {
            speed: 1.0,
            pause_when_unfocused: true,
            idle_fps: 4.0,
        }
    }
}

/// Every effect's tunables.
///
/// The container-level `#[serde(default)]` is load-bearing. It makes a missing
/// section inherit from [`Config::default`], which is built from each effect's
/// option defaults. A *field*-level `#[serde(default)]` would instead fall back
/// to that field type's derived `Default`, and those disagree with the real
/// defaults, so a partial user config would silently zero out every section the
/// user did not mention.
///
/// Every effect's own options struct carries the same attribute, and it is
/// needed for the other half of the same problem: a *present* section that names
/// only some of its keys. `Config`'s attribute covers a missing section but not a
/// missing key inside one that is there, so without it a user who wrote
///
/// ```toml
/// [plasma]
/// time_scale = 2.0
/// ```
///
/// gets a deserialisation error at startup rather than the other plasma settings
/// at their defaults. That matters more than it sounds, because `--print-config`
/// writes every key out, so a generated config is pinned to whatever the defaults
/// were when it was generated -- which means "add a new knob" and "add a key
/// nobody's file has yet" are the same situation. `omitting_any_single_key_keeps
/// _its_real_default` in `tests/effect_contracts.rs` is the guard.
///
/// Deliberately *not* `deny_unknown_fields`. A stale key from a removed setting
/// would then be a hard startup failure, and an ignored key is a much better
/// failure than a refusal to launch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub global: GlobalOptions,
    pub matrix: DigitalRainOptions,
    pub life: ConwayLifeOptions,
    pub mandelbrot: MandelbrotOptions,
    pub maze: MazeOptions,
    pub boids: BoidsOptions,
    pub ink: AsciiFieldOptions,
    pub blank: BlankOptions,
    pub cube: CubeOptions,
    pub crab: CrabOptions,
    pub donut: DonutOptions,
    pub dvd: DvdOptions,
    pub pipes: PipesOptions,
    pub plasma: PlasmaOptions,
    pub fire: FireOptions,
    pub terrain: TerrainOptions,
    pub constellation: ConstellationOptions,
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

    pub fn get_mandelbrot_options(&self) -> MandelbrotOptions {
        self.mandelbrot.clone()
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

    pub fn get_ink_options(&self) -> AsciiFieldOptions {
        self.ink.clone()
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

    /// Overrides the seed of every section that has one.
    ///
    /// Spelled out field by field on purpose: a `--seed` that quietly skipped a
    /// section would leave that effect unreproducible while appearing to work,
    /// which is the same class of silent gap as an effect missing from the
    /// registry.
    ///
    /// The list is hand-maintained and that is a real risk, so it is not left to
    /// review. `seeded_effects_are_reproducible_and_seed_sensitive` overrides
    /// with two different values and requires every seed-sensitive effect to
    /// render differently, so a section left off this list fails the test suite
    /// rather than quietly doing nothing. Adding mandelbrot without adding it
    /// here is exactly how that was found.
    pub fn override_seed(&mut self, seed: u64) {
        self.mandelbrot.seed = seed;
        self.matrix.seed = seed;
        self.life.seed = seed;
        self.maze.seed = seed;
        self.boids.seed = seed;
        self.ink.seed = seed;
        self.crab.seed = seed;
        self.dvd.seed = seed;
        self.pipes.seed = seed;
        self.fire.seed = seed;
        self.terrain.seed = seed;
        self.constellation.seed = seed;
    }
}

impl Default for Config {
    /// Every section's own `Default` is now the single source of truth, so this
    /// is just the field-by-field assembly. It used to spell out sixteen builder
    /// calls, which meant the defaults lived in two places per struct: the
    /// builder, and the derived `Default` that serde actually used.
    fn default() -> Self {
        Self {
            global: GlobalOptions::default(),
            matrix: DigitalRainOptions::default(),
            life: ConwayLifeOptions::default(),
            mandelbrot: MandelbrotOptions::default(),
            maze: MazeOptions::default(),
            boids: BoidsOptions::default(),
            ink: AsciiFieldOptions::default(),
            blank: BlankOptions::default(),
            cube: CubeOptions::default(),
            crab: CrabOptions::default(),
            donut: DonutOptions::default(),
            dvd: DvdOptions::default(),
            pipes: PipesOptions::default(),
            plasma: PlasmaOptions::default(),
            fire: FireOptions::default(),
            terrain: TerrainOptions::default(),
            constellation: ConstellationOptions::default(),
            playlist: PlaylistOptions::default(),
        }
    }
}
