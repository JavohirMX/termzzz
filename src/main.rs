use crossterm::{self, cursor, execute, terminal};
use std::{io, process};

use termzzz::{
    ascii::AsciiField, blank::Blank, boids::Boids, check, common, config::Config,
    constellation::Constellation, crab::Crab, cube::Cube, donut::Donut, dvd::Dvd,
    error, fire::Fire, life::ConwayLife, maze::Maze, pipes::Pipes, plasma::Plasma,
    rain::digital_rain::DigitalRain, terrain::Terrain,
};

#[derive(Debug)]
struct AppArgs {
    screen_saver: String,
    check: bool,
    effect: Option<String>,
    frames: Option<usize>,
}

/// Guard to drop out alternate screen in case of errors
struct TerminalGuard {
    stdout: io::Stdout,
}

impl TerminalGuard {
    fn new() -> Result<Self, io::Error> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;
        if let Err(error) = execute!(
            stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            terminal::Clear(terminal::ClearType::All)
        ) {
            let _ = execute!(
                stdout,
                cursor::Show,
                terminal::Clear(terminal::ClearType::All),
                terminal::LeaveAlternateScreen,
            );
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }

        Ok(Self { stdout })
    }

    // Get mutable access to the stdout
    fn get_stdout(&mut self) -> &mut io::Stdout {
        &mut self.stdout
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            self.stdout,
            cursor::Show,
            terminal::Clear(terminal::ClearType::All),
            terminal::LeaveAlternateScreen,
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn main() -> Result<(), error::TermzzzError> {
    env_logger::init();

    let args = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing args: {}", e);
            process::exit(1);
        }
    };

    if args.check {
        let effect = check_effect(&args);
        let frames = args.frames.unwrap_or(1);
        return check::run_test_for_effect(&effect, frames);
    }

    let (config, config_status) = Config::load()?;
    let size = common::normalize_effect_size(terminal::size()?);

    let fps = {
        let mut guard = TerminalGuard::new()?;
        let mut effect = build_effect(&args.screen_saver, &config, size)?;

        common::run_loop(guard.get_stdout(), &mut effect, None)?
    };

    println!("{}", config_status);
    println!("Frames per second: {:.1}", fps);
    Ok(())
}

enum AnyEffect {
    Matrix(DigitalRain),
    Life(ConwayLife),
    Maze(Maze),
    Boids(Boids),
    Blank(Blank),
    Cube(Cube),
    Crab(Crab),
    Donut(Donut),
    Pipes(Pipes),
    Plasma(Plasma),
    Fire(Fire),
    Constellation(Constellation),
    Terrain(Terrain),
    Ascii(AsciiField),
    Dvd(Dvd),
}

fn build_effect(
    name: &str,
    config: &Config,
    size: (u16, u16),
) -> Result<AnyEffect, error::TermzzzError> {
    let unknown = || error::TermzzzError::UnsupportedEffect(name.to_string());

    Ok(match name {
        "matrix" => AnyEffect::Matrix(DigitalRain::new(
            config.get_matrix_options(size),
            size,
        )),
        "life" => {
            AnyEffect::Life(ConwayLife::new(config.get_life_options(size), size))
        }
        "maze" => AnyEffect::Maze(Maze::new(config.get_maze_options(size), size)),
        "boids" => AnyEffect::Boids(Boids::new(config.get_boids_options(size))),
        "blank" => AnyEffect::Blank(Blank::new(config.get_blank_options(), size)),
        "cube" => AnyEffect::Cube(Cube::new(config.get_cube_options(), size)),
        "crab" => AnyEffect::Crab(Crab::new(config.get_crab_options(size), size)),
        "donut" => {
            AnyEffect::Donut(Donut::new(config.get_donut_options(size), size))
        }
        "pipes" => AnyEffect::Pipes(Pipes::new(config.get_pipes_options(), size)),
        "plasma" => {
            AnyEffect::Plasma(Plasma::new(config.get_plasma_options(), size))
        }
        "fire" => AnyEffect::Fire(Fire::new(config.get_fire_options(), size)),
        "constellation" => AnyEffect::Constellation(Constellation::new(
            config.get_constellation_options(),
            size,
        )),
        "terrain" => {
            AnyEffect::Terrain(Terrain::new(config.get_terrain_options(), size))
        }
        "dvd" => AnyEffect::Dvd(Dvd::new(config.get_dvd_options(), size)),
        "ascii" => {
            AnyEffect::Ascii(AsciiField::new(config.get_ascii_options(), size))
        }
        _ => return Err(unknown()),
    })
}

macro_rules! dispatch {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            AnyEffect::Matrix(effect) => effect.$method($($arg),*),
            AnyEffect::Life(effect) => effect.$method($($arg),*),
            AnyEffect::Maze(effect) => effect.$method($($arg),*),
            AnyEffect::Boids(effect) => effect.$method($($arg),*),
            AnyEffect::Blank(effect) => effect.$method($($arg),*),
            AnyEffect::Cube(effect) => effect.$method($($arg),*),
            AnyEffect::Crab(effect) => effect.$method($($arg),*),
            AnyEffect::Donut(effect) => effect.$method($($arg),*),
            AnyEffect::Pipes(effect) => effect.$method($($arg),*),
            AnyEffect::Plasma(effect) => effect.$method($($arg),*),
            AnyEffect::Fire(effect) => effect.$method($($arg),*),
            AnyEffect::Constellation(effect) => effect.$method($($arg),*),
            AnyEffect::Terrain(effect) => effect.$method($($arg),*),
            AnyEffect::Ascii(effect) => effect.$method($($arg),*),
            AnyEffect::Dvd(effect) => effect.$method($($arg),*),
        }
    };
}

impl common::TerminalEffect for AnyEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, termzzz::buffer::Cell)> {
        dispatch!(self, get_diff)
    }

    fn update(&mut self) {
        dispatch!(self, update)
    }

    fn update_size(&mut self, width: u16, height: u16) {
        dispatch!(self, update_size, width, height)
    }

    fn reset(&mut self) {
        dispatch!(self, reset)
    }
}

fn check_effect(args: &AppArgs) -> String {
    args.effect
        .clone()
        .unwrap_or_else(|| args.screen_saver.clone())
}

fn parse_args() -> Result<AppArgs, String> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I>(mut args: I) -> Result<AppArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut screen_saver = "matrix".to_string();
    let mut check = false;
    let mut effect = None;
    let mut effect_explicit = false;
    let mut frames = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--version" | "-v" => {
                print_version();
                std::process::exit(0);
            }
            "--check" => {
                check = true;
            }
            "--print-config" => {
                if let Err(e) = Config::print_default_config() {
                    eprintln!("Failed to print config: {}", e);
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--effect" => {
                effect = args.next();
                effect_explicit = true;
            }
            "--frames" => {
                if let Some(frame_str) = args.next() {
                    frames = frame_str.parse().ok();
                }
            }
            arg if !arg.starts_with('-') => {
                if check && !effect_explicit {
                    effect = Some(arg.to_string());
                } else if !check {
                    screen_saver = arg.to_string();
                }
            }
            _ => {
                return Err(format!("Unknown argument: {}", arg));
            }
        }
    }

    Ok(AppArgs {
        screen_saver,
        check,
        effect,
        frames,
    })
}

fn print_help() {
    println!("termzzz - Terminal screensavers");
    println!();
    println!("USAGE:");
    println!("    termzzz [EFFECT] [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help              Show help");
    println!("    -v, --version           Show version");
    println!("        --check             Run test mode");
    println!("        --effect <EFFECT>    Effect to test (with --check)");
    println!("        --frames <NUM>       Number of frames to run (with --check)");
    println!("        --print-config       Print default config as TOML to stdout");
    println!();
    println!("CONFIG:");
    println!("    Config file (optional): ~/.config/termzzz.toml");
    println!(
        "    Generate one with:      termzzz --print-config > ~/.config/termzzz.toml"
    );
    println!();
    println!("EXAMPLES:");
    println!("    termzzz matrix            Run Matrix effect");
    println!("    termzzz ascii             Run interactive ASCII field");
    println!("    termzzz dvd               Run bouncing DVD logo");
    println!("    termzzz --check            Test with default effect");
    println!("    termzzz --check life       Test Life effect");
    println!("    termzzz --check --frames 100 life");
    println!("    termzzz --print-config > ~/.config/termzzz.toml");
    println!("    termzzz --version          Show version");
}

fn print_version() {
    println!("termzzz {}", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_mode_uses_positional_effect() {
        let args = parse_args_from(
            ["boids".to_string(), "--check".to_string()].into_iter(),
        )
        .unwrap();

        assert!(args.check);
        assert_eq!(check_effect(&args), "boids");
    }

    #[test]
    fn explicit_check_effect_takes_precedence() {
        let args = parse_args_from(
            [
                "--check".to_string(),
                "--effect".to_string(),
                "life".to_string(),
                "boids".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();

        assert_eq!(check_effect(&args), "life");
    }

    #[test]
    fn build_effect_rejects_unknown_names() {
        assert!(build_effect("nope", &Config::default(), (20, 10)).is_err());
    }
}
