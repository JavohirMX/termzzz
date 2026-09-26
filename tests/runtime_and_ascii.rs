use std::process::Command;
use std::time::Duration;

use termzzz::ascii::{AsciiField, AsciiFieldOptionsBuilder, GlyphPalette};
use termzzz::buffer::Cell;
use termzzz::common::{TerminalEffect, TickClock, run_loop_with_source_and_size};
use termzzz::config::Config;
use termzzz::registry::{AnyEffect, EffectId};
use termzzz::runtime::{
    FrameContext, InputEvent, InputSource, InputState, Key, KeyPhase,
    PointerButton, PointerPhase,
};

#[test]
fn input_state_tracks_key_and_pointer_interaction() {
    let mut state = InputState::default();

    state.apply(InputEvent::Key {
        key: Key::Char('r'),
        phase: KeyPhase::Pressed,
    });
    assert!(state.is_key_pressed(Key::Char('r')));

    state.apply(InputEvent::Pointer {
        position: (3, 4),
        phase: PointerPhase::Pressed,
        button: PointerButton::Left,
    });
    state.apply(InputEvent::Pointer {
        position: (5, 6),
        phase: PointerPhase::Moved,
        button: PointerButton::Left,
    });

    assert_eq!(state.pointer().position, (5, 6));
    assert_eq!(state.pointer().previous, Some((3, 4)));
    assert_eq!(state.pointer().delta, (2.0, 2.0));
    assert!(state.pointer().pressed);

    state.apply(InputEvent::Pointer {
        position: (5, 6),
        phase: PointerPhase::Released,
        button: PointerButton::Left,
    });
    assert!(!state.pointer().pressed);
}

#[test]
fn frame_context_carries_runtime_state() {
    let input = InputState::default();
    let context = FrameContext::new(
        (80, 24),
        7,
        Duration::from_millis(350),
        Duration::from_millis(16),
        input.clone(),
    );

    assert_eq!(context.size, (80, 24));
    assert_eq!(context.frame, 7);
    assert_eq!(context.elapsed, Duration::from_millis(350));
    assert_eq!(context.delta, Duration::from_millis(16));
    assert_eq!(context.input.pointer().position, (0, 0));
}

#[test]
fn input_state_bounds_held_keys_per_frame() {
    let mut state = InputState::default();
    state.apply(InputEvent::Key {
        key: Key::Char('a'),
        phase: KeyPhase::Pressed,
    });

    state.begin_frame();

    assert!(!state.is_key_pressed(Key::Char('a')));
}

#[test]
fn ascii_field_is_deterministic_and_bounds_safe() {
    let options = AsciiFieldOptionsBuilder::default()
        .seed(42u64)
        .build()
        .unwrap();
    let mut first = AsciiField::new(options.clone(), (20, 10));
    let mut second = AsciiField::new(options, (20, 10));

    let first_frame = first.get_diff();
    let second_frame = second.get_diff();

    assert_eq!(first_frame, second_frame);
    assert!(!first_frame.is_empty());
    assert!(first_frame.iter().all(|(x, y, _)| *x < 20 && *y < 10));
}

#[test]
fn ascii_field_accepts_interactive_input() {
    let options = AsciiFieldOptionsBuilder::default()
        .seed(7u64)
        .build()
        .unwrap();
    let mut field = AsciiField::new(options, (20, 10));

    field.handle_input(&InputEvent::Pointer {
        position: (5, 5),
        phase: PointerPhase::Pressed,
        button: PointerButton::Left,
    });
    field.update();

    assert!(field.get_diff().iter().all(|(x, y, _)| *x < 20 && *y < 10));
}

#[test]
fn ascii_field_reseeds_on_r() {
    let options = AsciiFieldOptionsBuilder::default()
        .seed(9u64)
        .build()
        .unwrap();
    let mut field = AsciiField::new(options, (20, 10));

    let first = field.get_diff();
    field.handle_input(&InputEvent::Key {
        key: Key::Char('r'),
        phase: KeyPhase::Pressed,
    });
    let second = field.get_diff();
    field.handle_input(&InputEvent::Key {
        key: Key::Char('r'),
        phase: KeyPhase::Pressed,
    });
    let third = field.get_diff();

    assert_ne!(first, second);
    assert!(!third.is_empty());
}

struct ScriptedInput {
    events: Vec<InputEvent>,
}

impl InputSource for ScriptedInput {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<Vec<InputEvent>> {
        if self.events.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![self.events.remove(0)])
        }
    }
}

struct BatchInput {
    events: Option<Vec<InputEvent>>,
}

impl InputSource for BatchInput {
    fn poll(&mut self, _timeout: Duration) -> std::io::Result<Vec<InputEvent>> {
        Ok(self.events.take().unwrap_or_default())
    }
}

struct ResizeEffect {
    resets: usize,
    last_size: Option<(u16, u16)>,
}

impl TerminalEffect for ResizeEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        Vec::new()
    }

    fn update(&mut self) {}

    fn update_size(&mut self, width: u16, height: u16) {
        self.last_size = Some((width, height));
    }

    fn reset(&mut self) {
        self.resets += 1;
    }
}

#[test]
fn runtime_loop_coalesces_resize_events() {
    let mut effect = ResizeEffect {
        resets: 0,
        last_size: None,
    };
    let mut input = BatchInput {
        events: Some(vec![
            InputEvent::Resize { size: (12, 8) },
            InputEvent::Resize { size: (20, 10) },
        ]),
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(1),
        &mut input,
        (30, 12),
    )
    .unwrap();

    assert_eq!(effect.resets, 1);
    assert_eq!(effect.last_size, Some((20, 10)));
}

struct RecordingEffect {
    frames: usize,
    last_size: Option<(u16, u16)>,
    context_size: Option<(u16, u16)>,
    context_frame: Option<u64>,
}

impl TerminalEffect for RecordingEffect {
    fn get_diff(&mut self) -> Vec<(usize, usize, Cell)> {
        Vec::new()
    }

    fn update(&mut self) {
        self.frames += 1;
    }

    fn update_size(&mut self, width: u16, height: u16) {
        self.last_size = Some((width, height));
    }

    fn reset(&mut self) {}

    fn get_diff_with_context(
        &mut self,
        context: &FrameContext,
    ) -> Vec<(usize, usize, Cell)> {
        self.context_size = Some(context.size);
        self.context_frame = Some(context.frame);
        Vec::new()
    }
}

#[test]
fn runtime_loop_uses_context_and_resize_events() {
    let mut effect = RecordingEffect {
        frames: 0,
        last_size: None,
        context_size: None,
        context_frame: None,
    };
    let mut input = ScriptedInput {
        events: vec![InputEvent::Resize { size: (12, 8) }],
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(2),
        &mut input,
        (20, 10),
    )
    .unwrap();

    assert_eq!(effect.frames, 2);
    assert_eq!(effect.last_size, Some((12, 8)));
    assert_eq!(effect.context_size, Some((12, 8)));
    assert_eq!(effect.context_frame, Some(1));
}

#[test]
fn runtime_loop_clamps_small_resize_for_effects() {
    let mut effect = RecordingEffect {
        frames: 0,
        last_size: None,
        context_size: None,
        context_frame: None,
    };
    let mut input = ScriptedInput {
        events: vec![InputEvent::Resize { size: (1, 1) }],
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(1),
        &mut input,
        (10, 10),
    )
    .unwrap();

    assert_eq!(effect.last_size, Some((6, 6)));
}

#[test]
fn runtime_loop_exits_before_rendering_after_quit() {
    let mut effect = RecordingEffect {
        frames: 0,
        last_size: None,
        context_size: None,
        context_frame: None,
    };
    let mut input = ScriptedInput {
        events: vec![InputEvent::Quit],
    };
    let mut output = Vec::new();

    run_loop_with_source_and_size(
        &mut output,
        &mut effect,
        Some(3),
        &mut input,
        (20, 10),
    )
    .unwrap();

    assert_eq!(effect.frames, 0);
    assert!(output.is_empty());
}

#[test]
fn package_name_is_termzzz() {
    assert_eq!(env!("CARGO_PKG_NAME"), "termzzz");
}

#[test]
fn cli_help_uses_termzzz_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_termzzz"))
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success());
    assert!(stdout.contains("termzzz [EFFECT] [OPTIONS]"));
    assert!(stdout.contains("~/.config/termzzz.toml"));
}

#[test]
fn partial_ascii_config_uses_defaults() {
    let config: termzzz::config::Config =
        toml::from_str("[ascii]\ntime_scale = 2.0\n").unwrap();

    assert_eq!(config.ascii.time_scale, 2.0);
    assert_eq!(config.ascii.seed, 42);
}

#[test]
fn global_speed_defaults_to_one_and_is_configurable() {
    assert_eq!(Config::default().global.speed, 1.0);

    let config: Config = toml::from_str("[global]\nspeed = 0.4\n").unwrap();
    assert_eq!(config.global.speed, 0.4);
    assert_eq!(config.get_ascii_options().seed, 42);
}

#[test]
fn glyph_palette_rejects_wide_and_control_characters() {
    let palette = GlyphPalette::new("😀\n x", Vec::new());

    assert_eq!(palette.sample(0.0).symbol, ' ');
    assert_eq!(palette.sample(1.0).symbol, 'x');
}

#[test]
fn check_mode_rejects_unknown_effects() {
    assert!(
        termzzz::check::run_test_for_effect("unknown", 1, &Config::default(), 1.0)
            .is_err()
    );
}

#[test]
fn registry_builds_every_effect() {
    let config = Config::default();

    for id in EffectId::ALL {
        let mut effect = AnyEffect::build(*id, &config, (20, 10));
        assert_eq!(effect.id(), *id);
        let _ = effect.get_diff();
    }
}

#[test]
fn registry_ids_round_trip_through_strings() {
    for id in EffectId::ALL {
        let text = id.as_str();
        assert_eq!(text.parse::<EffectId>().unwrap(), *id);
    }
    assert!("nope".parse::<EffectId>().is_err());
}

#[test]
fn dvd_is_registered_with_a_distinct_name() {
    let dvd = EffectId::Dvd.as_str();
    assert_eq!(dvd, "dvd");
    assert!(EffectId::ALL.contains(&EffectId::Dvd));
    assert!(!EffectId::Dvd.needs_mouse());
    assert!(EffectId::Ascii.needs_mouse());
}

#[test]
fn playlist_config_parses_effects_and_transitions() {
    let config: Config = toml::from_str(
        r#"
[playlist]
shuffle = true
transition = 1.2

[[playlist.effects]]
effect = "matrix"
duration = 4.0

[[playlist.effects]]
effect = "dvd"
"#,
    )
    .unwrap();

    let options = config.get_playlist_options();
    assert!(options.shuffle);
    assert_eq!(options.transition, 1.2);
    assert_eq!(options.effects.len(), 2);
    assert_eq!(options.effects[0].effect, "matrix");
    assert_eq!(options.effects[0].duration, Some(4.0));
    assert_eq!(options.effects[1].duration, None);
}

#[test]
fn playlist_defaults_to_a_short_transition() {
    let options = Config::default().get_playlist_options();
    assert_eq!(options.transition, 0.6);
    assert!(!options.shuffle);
    assert!(options.effects.is_empty());
}

#[test]
fn dvd_options_default_to_the_dvd_logo() {
    let options = Config::default().get_dvd_options();
    assert_eq!(options.logo, "DVD");
    assert!(options.corner_color_change);
}

#[test]
fn default_config_serializes_with_global_and_playlist_sections() {
    let rendered = toml::to_string_pretty(&Config::default()).unwrap();

    assert!(rendered.contains("[global]"));
    assert!(rendered.contains("[playlist]"));
    assert!(rendered.contains("[dvd]"));
}

#[test]
fn speed_clock_scales_simulation_ticks() {
    let mut normal = TickClock::default();
    let mut slow = TickClock::default();
    let delta = Duration::from_millis(50);

    assert_eq!(normal.advance(delta, 1.0), 3);
    assert_eq!(slow.advance(delta, 0.5), 1);
}

#[test]
fn legacy_effect_defaults_use_calmer_pacing() {
    let config = Config::default();

    assert_eq!(config.get_plasma_options().color_speed, 20.0);
    assert_eq!(
        config.get_life_options((20, 10)).generations_per_second,
        8.0
    );
    assert_eq!(config.get_donut_options((20, 10)).rotation_speed_a, 0.022);
    assert_eq!(config.get_pipes_options().num_lines, 3);
    assert_eq!(config.get_cube_options().rotation_speed_x, 0.25);
    assert_eq!(config.get_crab_options((20, 10)).movement_speed, 3.0);
}

#[test]
fn ascii_defaults_include_a_large_glyph_ramp() {
    let options = termzzz::ascii::AsciiFieldOptions::default();

    assert!(options.glyphs.len() > 20);
    assert!(options.pointer_decay > 1.0);
}
