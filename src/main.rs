use crossterm::{self, cursor, execute, terminal};
use std::{io, process};

use termzzz::{
    check, common,
    config::Config,
    error,
    registry::{AnyEffect, EffectId},
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

    let (config, config_status) = Config::load()?;

    if args.check {
        let effect = check_effect(&args);
        let frames = args.frames.unwrap_or(1);
        return check::run_test_for_effect(&effect, frames, &config);
    }
    let size = common::normalize_effect_size(terminal::size()?);

    let effect_id = match args.screen_saver.parse::<EffectId>() {
        Ok(effect_id) => effect_id,
        Err(error) => {
            println!("Unknown screen saver: {error}");
            print_help();
            return Ok(());
        }
    };

    let fps = {
        let mut guard = TerminalGuard::new()?;
        let mut effect = AnyEffect::build(effect_id, &config, size);

        common::run_loop(guard.get_stdout(), &mut effect, None)?
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
    println!("        --print-config       Print default config as TOML to stdout");
    println!();
    println!("CONFIG:");
    println!("    Config file (optional): ~/.config/termzzz.toml");
    println!(
        "    Generate one with:      termzzz --print-config > ~/.config/termzzz.toml"
    );
    println!();
    println!("EXAMPLES:");
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
}
