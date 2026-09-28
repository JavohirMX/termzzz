//! Terminal-based screensavers and visual effects.
//!
//! Modules:
//!
//! | Module   | Description                              |
//! |----------|------------------------------------------|
//! | `ink`    | Interactive generative field you pour ink into |
//! | `ants`   | Langton's ants sharing one board         |
//! | `blank`  | Blank screen — no-op placeholder         |
//! | `boids`  | Boids flocking simulation                |
//! | `buffer` | Terminal cell buffer for colored output  |
//! | `canvas` | Double-buffered drawing surface and diff-and-commit |
//! | `check`  | Terminal event checking (input, resize)  |
//! | `common` | Shared traits and types (TerminalEffect) |
//! | `config` | TOML configuration loading and options     |
//! | `solarsystem` | A 3D orrery: planets on tilted orbits, real periods |
//! | `crab`   | ASCII crab walking animation             |
//! | `cube`   | 3D rotating cube in ASCII                |
//! | `donut`  | 3D rotating donut in ASCII               |
//! | `dvd`    | Bouncing DVD logo                        |
//! | `error`  | Error types for the crate                |
//! | `fire`   | Fire simulation effect                   |
//! | `flyover`| First-person flight over a fractal height field |
//! | `host`   | The running effect, and swapping it        |
//! | `life`   | Conway's Game of Life                    |
//! | `mandelbrot` | Escape-time Mandelbrot set, zooming   |
//! | `maze`   | Maze generation and animation            |
//! | `physarum`| Slime-mould agents building a transport network |
//! | `pipes`  | Pipe maze animation                      |
//! | `plasma` | Plasma color wave effect                 |
//! | `playlist` | Timed playlist with blank-wipe transitions |
//! | `rain`   | Matrix-style digital rain                |
//! | `render` | Sub-cell renderers: braille, half-block, dithering |
//! | `registry`| The effect table: ids, names, durations |
//! | `ripple` | Interfering waves from a few point sources |
//! | `runtime`| Runtime input and frame context          |
//! | `session`| Terminal setup, teardown and panic safety |
//! | `terrain`| Terrain generation — scrolling landscape |
//!
//! The list of effects, their names, help text and default playlist durations
//! is [`registry::EFFECT_SPECS`]. It is not repeated here or in the docs,
//! because a second copy is a second thing to forget to update.

pub mod ants;
pub mod blank;
pub mod boids;
pub mod buffer;
pub mod canvas;
pub mod check;
pub mod common;
pub mod config;
pub mod crab;
pub mod cube;
pub mod donut;
pub mod dvd;
pub mod error;
pub mod fire;
pub mod flyover;
pub mod host;
pub mod ink;
pub mod life;
pub mod mandelbrot;
pub mod maze;
pub mod physarum;
pub mod pipes;
pub mod plasma;
pub mod playlist;
pub mod rain;
pub mod registry;
pub mod render;
pub mod ripple;
pub mod runtime;
pub mod sandpile;
pub mod session;
pub mod solarsystem;
pub mod terrain;
