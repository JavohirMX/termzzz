use crossterm::{self, cursor, execute, terminal};
use std::{io, process};

use termzzz::{
    check, common,
    config::Config,
    error,
    playlist::{Playlist, PlaylistEntry},
    registry::{AnyEffect, EffectId},
};

#[derive(Debug)]
struct AppArgs {
    screen_saver: String,
    check: bool,
    effect: Option<String>,
    frames: Option<usize>,
    speed: Option<f32>,
    playlist: Option<Vec<String>>,
    shuffle: bool,
    transition: Option<f32>,
}

/// Guard to drop out alternate screen in case of errors
struct TerminalGuard {
    stdout: io::Stdout,
    mouse_capture: bool,
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

        Ok(Self {
            stdout,
            mouse_capture: false,
        })
    }

    // Get mutable access to the stdout
    fn get_stdout(&mut self) -> &mut io::Stdout {
        &mut self.stdout
    }

    fn enable_mouse(&mut self) -> io::Result<()> {
        self.mouse_capture = true;
        execute!(self.stdout, crossterm::event::EnableMouseCapture)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.mouse_capture {
            let _ = execute!(self.stdout, crossterm::event::DisableMouseCapture);
        }
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

    let (config, config_status) = Config::load()?;
    let speed = args
        .speed
        .unwrap_or(config.global.speed)
        .clamp(common::MIN_SPEED, common::MAX_SPEED);

    if args.check {
        let effect = check_effect(&args);
        let frames = args.frames.unwrap_or(1);
        if let Err(error) =
            check::run_test_for_effect(&effect, frames, &config, speed)
        {
            eprintln!("Error: {error}");
            process::exit(1);
        }
        return Ok(());
    }

    if args.playlist.is_some() || args.shuffle {
        let mut options = config.get_playlist_options();
        if let Some(names) = &args.playlist {
            options.effects = names
                .iter()
                .map(|name| PlaylistEntry {
                    effect: name.clone(),
                    duration: None,
                })
                .collect();
        }
        if args.shuffle {
            options.shuffle = true;
        }
        if let Some(transition) = args.transition {
            options.transition = transition;
        }

        let needs_mouse = options
            .effects
            .iter()
            .filter_map(|entry| entry.effect.parse::<EffectId>().ok())
            .any(|id| id.needs_mouse());

        let fps = {
            let mut guard = TerminalGuard::new()?;
            let (width, height) = common::normalize_effect_size(terminal::size()?);
            let mut effect =
                Playlist::new(options, config.clone(), (width, height));

            if needs_mouse && let Err(error) = guard.enable_mouse() {
                eprintln!("Mouse capture unavailable: {}", error);
            }

            common::run_loop_with_options(
                guard.get_stdout(),
                &mut effect,
                None,
                common::RuntimeOptions::new(speed),
            )?
        };

        println!("{}", config_status);
        println!("Frames per second: {:.1}", fps);
        return Ok(());
    }

    let effect_id = match args.screen_saver.parse::<EffectId>() {
        Ok(effect_id) => effect_id,
        Err(name) => {
            println!("Unknown screen saver: {name}");
            print_help();
            return Ok(());
        }
    };

    let fps = {
        let mut guard = TerminalGuard::new()?;
        let (width, height) = common::normalize_effect_size(terminal::size()?);
        let mut effect = AnyEffect::build(effect_id, &config, (width, height));

        if effect_id.needs_mouse()
            && let Err(error) = guard.enable_mouse()
        {
            eprintln!("Mouse capture unavailable: {}", error);
        }

        common::run_loop_with_options(
            guard.get_stdout(),
            &mut effect,
            None,
            common::RuntimeOptions::new(speed),
        )?
    };

    println!("{}", config_status);
    println!("Frames per second: {:.1}", fps);
    Ok(())
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
    let mut speed = None;
    let mut playlist = None;
    let mut shuffle = false;
    let mut transition = None;

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
            "--speed" => {
                let speed_str = args
                    .next()
                    .ok_or_else(|| "--speed requires a value".to_string())?;
                let value = speed_str
                    .parse::<f32>()
                    .map_err(|_| "--speed must be a number".to_string())?;
                if !value.is_finite()
                    || !(common::MIN_SPEED..=common::MAX_SPEED).contains(&value)
                {
                    return Err(format!(
                        "--speed must be between {} and {}",
                        common::MIN_SPEED,
                        common::MAX_SPEED
                    ));
                }
                speed = Some(value);
            }
            "--playlist" => {
                let value = args.next().ok_or_else(|| {
                    "--playlist requires a comma-separated list of effects"
                        .to_string()
                })?;
                playlist = Some(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_string)
                        .collect(),
                );
                if playlist.as_ref().is_some_and(Vec::is_empty) {
                    return Err(
                        "--playlist requires at least one effect".to_string()
                    );
                }
            }
            "--shuffle" => {
                shuffle = true;
            }
            "--transition" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--transition requires a value".to_string())?;
                let parsed = value
                    .parse::<f32>()
                    .map_err(|_| "--transition must be a number".to_string())?;
                if !parsed.is_finite() || parsed < 0.0 {
                    return Err("--transition must be zero or greater".to_string());
                }
                transition = Some(parsed);
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
        speed,
        playlist,
        shuffle,
        transition,
    })
}

fn print_help() {
    println!("termzzz - Terminal screensavers");
    println!();
    println!("USAGE:");
    println!("    termzzz [EFFECT] [OPTIONS]");
    println!();
    println!("EFFECTS:");
    for effect in EffectId::ALL {
        println!("    {:<14}{}", effect.as_str(), effect.description());
    }
    println!();
    println!("OPTIONS:");
    println!("    -h, --help              Show help");
    println!("    -v, --version           Show version");
    println!("        --check             Run test mode");
    println!("        --effect <EFFECT>    Effect to test (with --check)");
    println!("        --frames <NUM>       Number of frames to run (with --check)");
    println!("        --speed <MULT>       Global speed multiplier (default 1.0)");
    println!("        --playlist <LIST>    Play effects in order, comma separated");
    println!("        --shuffle            Play the playlist in random order");
    println!("        --transition <SECS>  Seconds of blank wipe between effects");
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
    println!("    termzzz --check            Test with default effect");
    println!("    termzzz --check life       Test Life effect");
    println!("    termzzz --check --frames 100 life");
    println!("    termzzz plasma --speed 0.5");
    println!("    termzzz --playlist matrix,dvd,plasma");
    println!("    termzzz --shuffle");
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
    fn parses_speed_override() {
        let args = parse_args_from(
            [
                "--speed".to_string(),
                "0.5".to_string(),
                "ascii".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();

        assert_eq!(args.speed, Some(0.5));
    }

    #[test]
    fn parses_playlist_list() {
        let args = parse_args_from(
            ["--playlist".to_string(), "matrix, dvd ,plasma".to_string()]
                .into_iter(),
        )
        .unwrap();

        assert_eq!(
            args.playlist,
            Some(vec![
                "matrix".to_string(),
                "dvd".to_string(),
                "plasma".to_string()
            ])
        );
        assert!(!args.shuffle);
    }

    #[test]
    fn rejects_empty_playlist() {
        assert!(
            parse_args_from(
                ["--playlist".to_string(), " , ".to_string()].into_iter()
            )
            .is_err()
        );
    }

    #[test]
    fn playlist_requires_a_value() {
        assert!(parse_args_from(["--playlist".to_string()].into_iter()).is_err());
    }

    #[test]
    fn parses_shuffle_and_transition() {
        let args = parse_args_from(
            [
                "--shuffle".to_string(),
                "--transition".to_string(),
                "1.5".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();

        assert!(args.shuffle);
        assert_eq!(args.transition, Some(1.5));
    }

    #[test]
    fn rejects_negative_transition() {
        assert!(
            parse_args_from(
                ["--transition".to_string(), "-1".to_string()].into_iter()
            )
            .is_err()
        );
    }

    #[test]
    fn speed_must_stay_in_range() {
        assert!(
            parse_args_from(["--speed".to_string(), "99".to_string()].into_iter())
                .is_err()
        );
        assert!(
            parse_args_from(["--speed".to_string(), "0".to_string()].into_iter())
                .is_err()
        );
    }
}
