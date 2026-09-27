use crossterm::terminal;
use std::process;

use termzzz::{
    check, common,
    config::Config,
    error,
    host::EffectHost,
    playlist::{Playlist, PlaylistEntry},
    registry::EffectId,
    session::{SessionColors, TerminalSession, install_panic_hook},
};

/// The terminal colours this run should install, taken from the config.
///
/// A function rather than a `Config` method so that the "read the two global
/// colour fields" step exists in exactly one place. Every entry point -- single
/// effect, playlist, and `--check` -- has to agree on this, and the failure mode
/// when they do not is a check run that looks nothing like the real run.
fn session_colors(config: &Config) -> SessionColors {
    SessionColors {
        background: config.global.background,
        foreground: config.global.foreground,
    }
}

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
    seed: Option<u64>,
    random_seed: bool,
}

/// A seed for `--random`, drawn from the operating system.
///
/// The obvious implementation is `SystemTime::now()` as a nanosecond count, and
/// it is the wrong one twice over. It is a *clock*, so two runs started in the
/// same millisecond get the same picture, and on a fast machine that is not
/// hypothetical. And it increases monotonically, so consecutive runs get
/// *adjacent* seeds, and a field built from those looks like the same picture
/// nudged -- the opposite of what someone asking for "something different each
/// time" wants. `random_seeds_differ_and_are_not_merely_increasing` checks the
/// correlation rather than only the equality, because equality is the weaker
/// claim and a clock passes it.
///
/// `rand::rng` is seeded from the OS and panics if that fails, which is
/// acceptable here and worth stating rather than shrugging at: every target this
/// builds for has a system entropy source, it cannot fail in a way a user could
/// act on, and a screensaver refusing to start over four random bytes would be a
/// worse outcome than a repeated picture.
///
/// Deliberately *not* how the effects get their randomness. Those take a seeded
/// generator so that `--seed` reproduces a run; this is a one-shot choice of that
/// seed, made before any effect exists.
fn random_seed() -> u64 {
    use rand::RngExt;
    rand::rng().random::<u64>()
}

/// The seed this run should use, and whether to tell the user we picked.
///
/// Split out from `main` so the precedence is testable without a terminal. The
/// first version of `--random` put this inline in `main`, where the only
/// reachable test was the argument parser -- and the parser cannot see the
/// decision, because it happens after parsing. A test that only proves `--seed`
/// was stored somewhere proves nothing about which one wins.
fn resolve_seed(explicit: Option<u64>, random: bool) -> (Option<u64>, bool) {
    match (explicit, random) {
        (Some(seed), true) => (Some(seed), true),
        (Some(seed), false) => (Some(seed), false),
        (None, true) => (Some(random_seed()), false),
        (None, false) => (None, false),
    }
}

fn main() -> Result<(), error::TermzzzError> {
    env_logger::init();
    // Installed before any session exists. With `panic = "abort"` the usual
    // `Drop`-based restore never runs, so a panic would otherwise leave the
    // terminal in raw mode on the alternate screen with no way out.
    install_panic_hook();

    let args = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing args: {}", e);
            process::exit(1);
        }
    };

    let (mut config, config_status) = Config::load()?;
    // Applied before anything reads an options struct, so it reaches the single
    // effect that runs as well as every entry of a playlist.
    // `--seed` wins over `--random`, and says so rather than quietly ignoring one
    // of them. An explicit seed is a request to reproduce something, and a
    // random one next to it can only be a mistake; picking the seed silently
    // would make `--seed 1234 --random` unreproducible while appearing to honour
    // the seed.
    let (chosen, seed_won) = resolve_seed(args.seed, args.random_seed);
    if seed_won {
        eprintln!("Note: --seed wins over --random, so this run is reproducible.");
    }
    if let Some(seed) = chosen {
        config.override_seed(seed);
    }
    let speed = args
        .speed
        .unwrap_or(config.global.speed)
        .clamp(common::MIN_SPEED, common::MAX_SPEED);

    // Both entry points below share this, so the focus policy is decided once and
    // cannot drift between running a single effect and running a playlist.
    let runtime_options = common::RuntimeOptions::new(speed).with_focus_policy(
        config.global.pause_when_unfocused,
        config.global.idle_fps,
    );

    if args.check {
        let effect = check_effect(&args);
        let frames = args.frames.unwrap_or(1);
        if let Err(error) = check::run_test_for_effect(
            &effect,
            frames,
            &config,
            speed,
            session_colors(&config),
        ) {
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
            let mut session = TerminalSession::enter_with(session_colors(&config))?;
            let (width, height) = common::normalize_effect_size(terminal::size()?);
            let mut effect =
                Playlist::new(options, config.clone(), (width, height));

            if needs_mouse && let Err(error) = session.enable_mouse() {
                eprintln!("Mouse capture unavailable: {}", error);
            }

            common::run_loop_with_options(
                session.stdout(),
                &mut effect,
                None,
                runtime_options,
            )?
        };

        println!("{}", config_status);
        println!("Frames per second: {:.1}", fps);
        return Ok(());
    }

    let effect_id = match args.screen_saver.parse::<EffectId>() {
        Ok(effect_id) => effect_id,
        Err(error) => {
            // The parse error already names the problem and the rejected value,
            // so it is printed on its own rather than behind another prefix,
            // which would read "Unknown screen saver: Unknown effect: foo".
            println!("{error}");
            print_help();
            return Ok(());
        }
    };

    let fps = {
        let mut session = TerminalSession::enter_with(session_colors(&config))?;
        let (width, height) = common::normalize_effect_size(terminal::size()?);
        let mut host = EffectHost::new(effect_id, config.clone(), (width, height));

        // `--transition` used to reach the playlist and nothing else, so the wipe
        // behind `n` and `p` was always the host's own default no matter what
        // was asked for. The playlist still spends its number on each half of a
        // transition; the host spends it on the whole switch, so the two differ
        // by a factor of two on the same flag. That is a deliberate choice in
        // each direction -- see `EffectHost::set_transition` -- but it is worth
        // knowing about before someone reads the flag as meaning one thing.
        if let Some(transition) = args.transition {
            host.set_transition(transition);
        }

        // Mouse capture is enabled for the whole session, not just the effect
        // that happens to be running: `n` can bring up the ASCII field at any
        // moment, and toggling capture mid-session would leave the terminal
        // reporting motion the effect never asked for.
        if EffectId::all().any(|id| id.needs_mouse())
            && let Err(error) = session.enable_mouse()
        {
            eprintln!("Mouse capture unavailable: {}", error);
        }

        common::run_loop_with_target(
            session.stdout(),
            &mut host,
            None,
            runtime_options,
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
    let mut seed = None;
    let mut random_seed = false;

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
            "--seed" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--seed requires a value".to_string())?;
                seed =
                    Some(value.parse::<u64>().map_err(|_| {
                        "--seed must be a whole number".to_string()
                    })?);
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
            "--random" => {
                random_seed = true;
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
        seed,
        random_seed,
    })
}

fn print_help() {
    println!("termzzz - Terminal screensavers");
    println!();
    println!("USAGE:");
    println!("    termzzz [EFFECT] [OPTIONS]");
    println!();
    println!("EFFECTS:");
    for effect in EffectId::all() {
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
    println!(
        "        --seed <N>           Seed the random effects, for a reproducible run"
    );
    println!("        --playlist <LIST>    Play effects in order, comma separated");
    println!("        --shuffle            Play the playlist in random order");
    println!(
        "        --random             Seed this run at random, so it looks different \
         every time"
    );
    println!("        --transition <SECS>  Seconds of blank wipe between effects");
    println!("        --print-config       Print default config as TOML to stdout");
    println!();
    println!("KEYS:");
    println!("    q, Esc, Ctrl+C        Quit");
    println!("    + / -                 Global animation speed");
    println!("    n / p                 Next / previous effect, behind a wipe");
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
            ["--speed".to_string(), "0.5".to_string(), "ink".to_string()]
                .into_iter(),
        )
        .unwrap();

        assert_eq!(args.speed, Some(0.5));
    }

    #[test]
    fn parses_seed_override() {
        let args = parse_args_from(
            [
                "--seed".to_string(),
                "1234".to_string(),
                "matrix".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();

        assert_eq!(args.seed, Some(1234));
    }

    /// `--random` is its own flag rather than a value for `--seed`, and the two
    /// are separate booleans in `AppArgs` so that "neither given" and "both
    /// given" are distinguishable. That distinction is the whole of the
    /// interaction: `--seed 1234 --random` has to keep the 1234.
    #[test]
    fn random_is_separate_from_seed_and_from_playlist_shuffle() {
        let neither = parse_args_from(["matrix".to_string()].into_iter()).unwrap();
        assert!(!neither.random_seed, "--random defaulted to on");
        assert_eq!(neither.seed, None);

        let random = parse_args_from(["--random".to_string()].into_iter()).unwrap();
        assert!(random.random_seed);
        assert_eq!(random.seed, None, "--random invented a seed at parse time");

        // `--shuffle` is the *playlist* order and has been for a while. A flag
        // that reseeds every effect cannot share a name with one that reorders a
        // list, and the collision was found by trying to add it.
        let shuffled =
            parse_args_from(["--shuffle".to_string()].into_iter()).unwrap();
        assert!(shuffled.shuffle, "--shuffle stopped meaning playlist order");
        assert!(
            !shuffled.random_seed,
            "--shuffle started reseeding the effects, which is a different \
             feature with a different meaning"
        );

        let both = parse_args_from(
            [
                "--seed".to_string(),
                "1234".to_string(),
                "--random".to_string(),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(both.seed, Some(1234));
        assert!(both.random_seed);
    }

    /// Which of `--seed` and `--random` wins, which is the actual decision.
    ///
    /// Deliberately a test of [`resolve_seed`] rather than of the parser, because
    /// the parser cannot see this: it happens after parsing, in `main`, where the
    /// only reachable test would be the whole binary. So the first version of this
    /// feature had a test proving `--random` was stored in `AppArgs` and nothing
    /// at all proving the two seeds do not both apply.
    #[test]
    fn an_explicit_seed_beats_the_random_one_and_says_so() {
        assert_eq!(resolve_seed(Some(1234), true), (Some(1234), true));
        assert_eq!(resolve_seed(Some(1234), false), (Some(1234), false));
        assert_eq!(resolve_seed(None, false), (None, false));

        // Only `--random` reaches the OS, so the seed is present and no note is
        // printed -- there is no conflict to report.
        let (seed, noted) = resolve_seed(None, true);
        assert!(seed.is_some(), "--random did not produce a seed");
        assert!(!noted, "--random alone announced a conflict with --seed");
    }

    /// A seed drawn from the OS is a seed, not a clock.
    ///
    /// Two runs must not agree, and more to the point they must not be
    /// *ordered*: `SystemTime::now()` in nanoseconds increases monotonically, so
    /// consecutive runs get adjacent seeds and a field built from them looks
    /// like the same picture nudged. The property that catches that is
    /// correlation between consecutive draws, not just equality.
    #[test]
    fn random_seeds_differ_and_are_not_merely_increasing() {
        let draws: Vec<u64> = (0..8).map(|_| random_seed()).collect();
        let unique: std::collections::HashSet<u64> =
            draws.iter().copied().collect();
        assert!(
            unique.len() >= 7,
            "eight draws from the OS produced only {} distinct values: {draws:?}",
            unique.len()
        );

        let increasing = draws.windows(2).filter(|w| w[1] > w[0]).count();
        assert!(
            increasing < 7,
            "{} of 7 consecutive pairs increased, which is what a monotonically \
             increasing clock would give: {draws:?}",
            increasing
        );
    }

    /// The `#[test]` here was eaten by a scripted insertion and the function sat
    /// in the test module as dead code for a whole commit, passing clippy's
    /// reachability and running nothing. `cargo test --bin termzzz` reported one
    /// fewer test than it should have and nobody compared the number, because
    /// "tests pass" and "the right tests ran" are different claims.
    #[test]
    fn seed_defaults_to_none_and_rejects_nonsense() {
        let untouched =
            parse_args_from(["matrix".to_string()].into_iter()).unwrap();
        assert_eq!(untouched.seed, None);

        // A seed is a whole number, so a negative or fractional value has to be
        // refused rather than silently truncated to something else.
        for bad in ["-1", "1.5", "abc", ""] {
            assert!(
                parse_args_from(
                    ["--seed".to_string(), bad.to_string()].into_iter()
                )
                .is_err(),
                "--seed {bad:?} was accepted"
            );
        }

        assert!(
            parse_args_from(["--seed".to_string()].into_iter()).is_err(),
            "--seed with no value was accepted"
        );
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
