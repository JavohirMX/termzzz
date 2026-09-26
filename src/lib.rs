//! Terminal-based screensavers and visual effects.
//!
//! Modules:
//!
//! | Module   | Description                              |
//! |----------|------------------------------------------|
//! | `ink`    | Interactive generative field you pour ink into |
//! | `blank`  | Blank screen — no-op placeholder         |
//! | `boids`  | Boids flocking simulation                |
//! | `buffer` | Terminal cell buffer for colored output  |
//! | `canvas` | Double-buffered drawing surface and diff-and-commit |
//! | `check`  | Terminal event checking (input, resize)  |
//! | `common` | Shared traits and types (TerminalEffect) |
//! | `config` | TOML configuration loading and options     |
//! | `constellation` | Drifting stars and dotted connections |
//! | `crab`   | ASCII crab walking animation             |
//! | `cube`   | 3D rotating cube in ASCII                |
//! | `donut`  | 3D rotating donut in ASCII               |
//! | `dvd`    | Bouncing DVD logo                        |
//! | `error`  | Error types for the crate                |
//! | `fire`   | Fire simulation effect                   |
//! | `host`   | The running effect, and swapping it        |
//! | `life`   | Conway's Game of Life                    |
//! | `maze`   | Maze generation and animation            |
//! | `pipes`  | Pipe maze animation                      |
//! | `plasma` | Plasma color wave effect                 |
//! | `playlist` | Timed playlist with blank-wipe transitions |
//! | `rain`   | Matrix-style digital rain                |
//! | `render` | Sub-cell renderers: braille, half-block, dithering |
//! | `registry`| The effect table: ids, names, durations |
//! | `runtime`| Runtime input and frame context          |
//! | `session`| Terminal setup, teardown and panic safety |
//! | `terrain`| Terrain generation — scrolling landscape |
//!
//! The list of effects, their names, help text and default playlist durations
//! is [`registry::EFFECT_SPECS`]. It is not repeated here or in the docs,
//! because a second copy is a second thing to forget to update.

pub mod blank;
pub mod boids;
pub mod buffer;
pub mod canvas;
pub mod check;
pub mod common;
pub mod config;
pub mod constellation;
pub mod crab;
pub mod cube;
pub mod donut;
pub mod dvd;
pub mod error;
pub mod fire;
pub mod host;
pub mod ink;
pub mod life;
pub mod mandelbrot;
pub mod maze;
pub mod pipes;
pub mod plasma;
pub mod playlist;
pub mod rain;
pub mod registry;
pub mod render;
pub mod runtime;
pub mod session;
pub mod terrain;
