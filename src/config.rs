use crate::common::DEFAULT_SEED;
use crate::{
    ants::AntsOptions,
    blank::BlankOptions,
    boids::BoidsOptions,
    crab::CrabOptions,
    cube::CubeOptions,
    donut::DonutOptions,
    dvd::DvdOptions,
    error::{ConfigError, Result, TermzzzError},
    fire::FireOptions,
    flyover::FlyoverOptions,
    ink::AsciiFieldOptions,
    life::ConwayLifeOptions,
    mandelbrot::MandelbrotOptions,
    maze::MazeOptions,
    physarum::PhysarumOptions,
    pipes::PipesOptions,
    plasma::PlasmaOptions,
    playlist::PlaylistOptions,
    rain::digital_rain::DigitalRainOptions,
    ripple::RippleOptions,
    rule30::Rule30Options,
    sandpile::SandpileOptions,
    solarsystem::SolarSystemOptions,
    terrain::TerrainOptions,
    wireworld::WireworldOptions,
};
use crossterm::style::Color;
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
    /// Whether to reduce the frame rate when the terminal does not have focus.
    ///
    /// A screensaver nobody is looking at should not be costing a core, so this
    /// drops to [`idle_fps`](Self::idle_fps) instead. It throttles rather than
    /// stops, and it always has on paper: the report was "when I put it on my
    /// second monitor and work on my first, it stops", and the code froze the
    /// *simulation* rather than the frame rate. Freezing is invisible twice over
    /// -- no effect renders from a wall clock, so an un-updated frame diffs to
    /// nothing and the loop wrote **zero bytes** four times a second.
    ///
    /// Renamed from `pause_when_unfocused`, which never described it: nothing
    /// was ever paused, the process ran throughout. The old name is still
    /// accepted so existing configs keep working.
    #[serde(alias = "pause_when_unfocused")]
    pub throttle_when_unfocused: bool,
    /// Frames per second while unfocused, when
    /// [`throttle_when_unfocused`](Self::throttle_when_unfocused) is set.
    ///
    /// Not zero. Stopping outright is the cheapest option and the most dangerous
    /// one, for the reason above.
    ///
    /// **Twenty, and the number is a threshold rather than a taste.** The loop
    /// clamps a frame's delta to `MAX_FRAME_DELTA`, 50 ms, so the effect
    /// advances at most 50 ms per drawn frame. At `N` frames a second that is
    /// `N * 50 ms` of simulation per second, which reaches real time at
    /// **20 fps** and is a fraction of it below that. So this knob quietly
    /// controls the *speed* as well as the smoothness, and the two cannot be
    /// pulled apart without sub-stepping: at the old default of 4, an unfocused
    /// effect would run at a fifth speed as well as at a quarter frame rate.
    ///
    /// 20 fps of a screensaver is smooth enough to read as motion and is a third
    /// of the render and encode work. Below 20 you are choosing to spend less
    /// and see less.
    pub idle_fps: f32,
    /// The terminal's own background colour, for the whole session.
    ///
    /// Emitted once as a single escape sequence when the session starts and
    /// reset on exit, rather than painted cell by cell. That distinction is the
    /// whole reason this is a global option and not a per-effect one: painting
    /// every cell would double the byte volume of every sparse effect, and eight
    /// of them would each need their own copy of the same code.
    ///
    /// `reset` -- the default -- leaves the terminal exactly as the user
    /// configured it. Setting it matters on terminals with a tinted profile: an
    /// effect that draws its dark end near black looks broken against a
    /// greyish-blue background, and the cheapest fix is to make the background
    /// actually black for the duration.
    ///
    /// Accepts crossterm's own colour spellings, which are *not* the same as
    /// the ones `Color::from_str` takes. The serde form wants
    /// `"dark_grey"`, not `"darkgrey"`, and `"rgb_(12,12,20)"`, not
    /// `"rgb(12,12,20)"`; `"#0c0c14"` and the plain names (`"black"`,
    /// `"white"`, `"reset"`) are the same either way. Listed here because the
    /// mismatch is invisible until a config fails to load, and the resulting
    /// error message quotes a list of the accepted forms rather than the ones
    /// most people would try.
    ///
    /// Round-trips: `--print-config` writes these back in the `rgb_(r,g,b)`
    /// form, so a generated config is stable under a second `--print-config`.
    pub background: Color,
    /// The terminal's own foreground colour, for the whole session.
    ///
    /// The same mechanism as [`background`](Self::background) and the same
    /// `reset` default. Separate because a user who pins the background to
    /// black usually wants the foreground pinned too, and the two are chosen
    /// together.
    pub foreground: Color,
}

impl Default for GlobalOptions {
    fn default() -> Self {
        Self {
            speed: 1.0,
            throttle_when_unfocused: true,
            idle_fps: 20.0,
            // `Reset` is not "no colour", it is "whatever the terminal profile
            // says", which is exactly the pre-existing behaviour. Defaulting to
            // `Black` instead would silently repaint every user's terminal.
            background: Color::Reset,
            foreground: Color::Reset,
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
    pub solarsystem: SolarSystemOptions,
    pub ants: AntsOptions,
    pub flyover: FlyoverOptions,
    pub physarum: PhysarumOptions,
    pub ripple: RippleOptions,
    pub rule30: Rule30Options,
    pub sandpile: SandpileOptions,
    pub wireworld: WireworldOptions,
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

    pub fn get_solarsystem_options(&self) -> SolarSystemOptions {
        self.solarsystem.clone()
    }

    /// The ant count, from the screen area.
    ///
    /// Derived here rather than in the effect for the reason every other effect
    /// does it: a fixed count is wrong at both ends of the range, and the
    /// arithmetic belongs next to the other densities rather than inside a
    /// constructor. Clamped, because ants sharing a cell flip it within a step of
    /// each other and destroy each other's turn decisions -- past the ceiling the
    /// board is noise, which the same picture at a lower density produces free.
    pub fn get_ants_options(&self, screen_size: (u16, u16)) -> AntsOptions {
        let mut options = self.ants.clone();
        let area = screen_size.0 as f32 * screen_size.1 as f32;
        options.ants = (area * crate::ants::ANT_DENSITY * options.ant_coeff).clamp(
            crate::ants::MIN_ANT_COUNT as f32,
            crate::ants::MAX_ANT_COUNT as f32,
        ) as u16;
        options
    }

    pub fn get_flyover_options(&self) -> FlyoverOptions {
        self.flyover.clone()
    }

    /// The agent count, from the size of the trail map.
    ///
    /// The trail map is at half-block row resolution rather than at cell
    /// resolution -- see [`crate::physarum::Physarum::size`] -- so the density is
    /// per *field row* and not per cell. Getting that wrong by a factor of two is
    /// invisible as a number and visible as a field that is either starved or a
    /// solid block.
    pub fn get_physarum_options(&self, screen_size: (u16, u16)) -> PhysarumOptions {
        let mut options = self.physarum.clone();
        let width = screen_size.0 as f32;
        let height =
            screen_size.1 as f32 * crate::render::halfblock::ROWS_PER_CELL as f32;
        options.agents =
            (width * height * crate::physarum::AGENT_DENSITY * options.agent_coeff)
                .clamp(
                    crate::physarum::MIN_AGENT_COUNT as f32,
                    crate::physarum::MAX_AGENT_COUNT as f32,
                ) as u32;
        options
    }

    pub fn get_ripple_options(&self) -> RippleOptions {
        self.ripple.clone()
    }

    pub fn get_rule30_options(&self) -> Rule30Options {
        self.rule30.clone()
    }

    pub fn get_sandpile_options(&self) -> SandpileOptions {
        self.sandpile.clone()
    }

    pub fn get_wireworld_options(&self) -> WireworldOptions {
        self.wireworld.clone()
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
        self.solarsystem.seed = seed;
        self.cube.seed = seed;
        self.donut.seed = seed;
        self.plasma.seed = seed;
        self.ants.seed = seed;
        self.flyover.seed = seed;
        self.physarum.seed = seed;
        self.ripple.seed = seed;
        self.rule30.seed = seed;
        self.sandpile.seed = seed;
        self.wireworld.seed = seed;
        // The playlist order, not just the pictures. Without this,
        // `--seed N --shuffle` reproduced the effects and not their order.
        self.playlist.seed = seed;
    }

    /// Gives every *unpinned* effect its own random seed.
    ///
    /// A seed of [`DEFAULT_SEED`] does not mean "42" any more -- it means
    /// **unset**, and this is the method that gives it a value. The request was
    /// "all the effects should be random and unique every time if no seed is
    /// set", and until now every effect that used randomness started from 42 on
    /// every launch, so a screensaver looked the same every time you started it.
    ///
    /// Each effect gets a *distinct* draw rather than one shared value. Sharing
    /// would be cheaper and perfectly reproducible, but then every effect in a
    /// playlist would be looking at the same underlying numbers, and two effects
    /// that both hash their seed the same way would move in step.
    ///
    /// ## Why `DEFAULT_SEED` means unset
    ///
    /// The alternative is `Option<u64>` on twelve option structs, which is the
    /// honest shape, and it was rejected for a specific reason: it pushes the
    /// "was this set?" question into every effect's constructor, and an effect
    /// rebuilt on a terminal resize would draw a *new* seed and change the
    /// picture under the user. Resolution has to happen once, on the config,
    /// before anything is built.
    ///
    /// The cost of the shortcut is one thing, and it is small: a config that
    /// pins `seed = 42` is indistinguishable from one that omits it, and both
    /// mean "give me something different each launch". `--seed 42` is how you
    /// pin that particular value.
    ///
    /// ## What this fixes about `--print-config`
    ///
    /// `--print-config` writes every default to disk, so a user with a generated
    /// config pins whatever the defaults were when they generated it -- the trap
    /// that let a 60x-too-slow donut rotation survive a default change in this
    /// project. For seeds that trap now dissolves: 42 no longer pins anything,
    /// so a generated config asking for 42 is asking for exactly what it should
    /// be asking for. That is the only reason a shortcut this size is defensible.
    pub fn randomise_seeds(&mut self) {
        use rand::RngExt;
        let draw = || rand::rng().random::<u64>();
        // An effect pinned to anything *other* than the default was chosen on
        // purpose and is left alone, which is what "if no seed is set" means.
        if self.boids.seed == DEFAULT_SEED {
            self.boids.seed = draw();
        }
        if self.crab.seed == DEFAULT_SEED {
            self.crab.seed = draw();
        }
        if self.dvd.seed == DEFAULT_SEED {
            self.dvd.seed = draw();
        }
        if self.fire.seed == DEFAULT_SEED {
            self.fire.seed = draw();
        }
        if self.ink.seed == DEFAULT_SEED {
            self.ink.seed = draw();
        }
        if self.life.seed == DEFAULT_SEED {
            self.life.seed = draw();
        }
        if self.mandelbrot.seed == DEFAULT_SEED {
            self.mandelbrot.seed = draw();
        }
        if self.maze.seed == DEFAULT_SEED {
            self.maze.seed = draw();
        }
        if self.pipes.seed == DEFAULT_SEED {
            self.pipes.seed = draw();
        }
        if self.terrain.seed == DEFAULT_SEED {
            self.terrain.seed = draw();
        }
        if self.solarsystem.seed == DEFAULT_SEED {
            self.solarsystem.seed = draw();
        }
        if self.cube.seed == DEFAULT_SEED {
            self.cube.seed = draw();
        }
        if self.donut.seed == DEFAULT_SEED {
            self.donut.seed = draw();
        }
        if self.plasma.seed == DEFAULT_SEED {
            self.plasma.seed = draw();
        }
        if self.ants.seed == DEFAULT_SEED {
            self.ants.seed = draw();
        }
        if self.flyover.seed == DEFAULT_SEED {
            self.flyover.seed = draw();
        }
        if self.physarum.seed == DEFAULT_SEED {
            self.physarum.seed = draw();
        }
        if self.ripple.seed == DEFAULT_SEED {
            self.ripple.seed = draw();
        }
        if self.rule30.seed == DEFAULT_SEED {
            self.rule30.seed = draw();
        }
        if self.sandpile.seed == DEFAULT_SEED {
            self.sandpile.seed = draw();
        }
        if self.wireworld.seed == DEFAULT_SEED {
            self.wireworld.seed = draw();
        }
        if self.playlist.seed == DEFAULT_SEED {
            self.playlist.seed = draw();
        }
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
            solarsystem: SolarSystemOptions::default(),
            ants: AntsOptions::default(),
            flyover: FlyoverOptions::default(),
            physarum: PhysarumOptions::default(),
            ripple: RippleOptions::default(),
            rule30: Rule30Options::default(),
            sandpile: SandpileOptions::default(),
            wireworld: WireworldOptions::default(),
            playlist: PlaylistOptions::default(),
        }
    }
}
