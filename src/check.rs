use crate::common::TerminalEffect;
use crate::error::Result;
use crate::runtime::{FrameContext, InputState};
use crate::session::{SessionColors, TerminalSession};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{self, Clear, ClearType},
};
use std::io::Write;
use std::time::Duration;

/// Runs a terminal screensaver effect for a limited number of frames to validate its functionality.
///
/// This function initializes the terminal in alternate screen mode, runs the specified effect
/// for the given number of frames, and waits for user input before restoring the terminal state.
/// Useful for testing and debugging screensaver effects.
///
/// # Arguments
/// * `effect` - The terminal effect implementation to run
/// * `frames` - Number of frames to render before pausing
///
/// # Returns
/// * `Result<(), TermzzzError>` - Success or error with terminal operations
///
/// # Example
/// ```ignore
/// let options = Default::default()
///     .screen_size((80, 40))
///     .build()?;
/// let mut effect = DigitalRain::new(options);
/// test_effect(&mut effect, 100)?;
/// ```
pub fn test_effect<T: TerminalEffect>(
    effect: &mut T,
    frames: usize,
    speed: f32,
    colors: SessionColors,
) -> Result<()> {
    let mut session = TerminalSession::enter_with(colors)?;
    let size = crate::common::normalize_effect_size(terminal::size()?);
    let mut input = InputState::default();
    input.set_size(size);
    let speed = crate::common::bounded_f32(
        speed,
        1.0,
        crate::common::MIN_SPEED,
        crate::common::MAX_SPEED,
    );
    let delta = Duration::from_secs_f64(1.0 / 60.0 * speed as f64);

    // Once, before the loop, for the same reason the real loop does not clear at
    // all: see `render_frames`.
    execute!(session.stdout(), Clear(ClearType::All))?;
    render_frames(
        effect,
        frames,
        size,
        &mut input,
        delta,
        &mut session.stdout(),
        colors,
    )?;

    loop {
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(_) = event::read()?
        {
            break;
        }
    }

    Ok(())
}

/// Draws `frames` frames of `effect` to `out`, and is the whole of what
/// `--check` puts on the screen.
///
/// Split out of [`test_effect`] so it can be driven against a `Vec<u8>` in a
/// test, which is the only way to see the bug below: the caller does the
/// `Clear`, and a test cannot reach a terminal.
///
/// **`get_diff_with_context` returns a DELTA**, against the effect's own
/// committed canvas. So the deltas of successive frames have to be written in
/// order onto one screen. This used to clear the screen on every iteration and
/// then write only that frame's delta, which erased everything the effect had
/// not happened to redraw this frame and never repainted it -- so `--check
/// --frames 100` showed a near-empty screen with a few hundred cells blinking in
/// it, and the more static the effect the emptier it got. `--frames 1` worked
/// by accident, because the first commit is a full repaint, which is why no test
/// caught it: `check_mode_rejects_unknown_effects` is the only test on this path
/// and it never enters the loop.
pub fn render_frames<W: Write>(
    effect: &mut impl TerminalEffect,
    frames: usize,
    size: (u16, u16),
    input: &mut InputState,
    delta: Duration,
    out: &mut W,
    colors: SessionColors,
) -> Result<()> {
    let started_at = std::time::Instant::now();
    for frame in 0..frames {
        // `elapsed` is real elapsed time and `frame` starts at 0, both to match
        // the real loop. Passing `delta` for `elapsed` made it a constant
        // ~16 ms for the whole run, and starting at 1 made frame 1 the second
        // frame. No effect reads either field today, so this is latent -- but a
        // diagnostic that lies about time is a bad thing to hand someone who is
        // debugging a timing bug with it.
        let context = FrameContext::new(
            size,
            frame as u64,
            started_at.elapsed(),
            delta,
            input.clone(),
        );
        let diff = effect.get_diff_with_context(&context);

        // The screensaver loop's encoder, not a second copy of it. This used to
        // open-code `PrintStyledContent` per cell, which is the encoding that
        // drops the background colour entirely and resets the style after every
        // glyph -- so check mode and the real loop showed different colours for
        // the same frame.
        crate::common::write_cells(out, size, &diff, colors)?;
        out.flush()?;
        effect.update_with_context(&context);
    }
    Ok(())
}

/// Run appropriate effect till frame number
pub fn run_test_for_effect(
    effect_name: &str,
    frames: usize,
    config: &crate::config::Config,
    speed: f32,
    colors: SessionColors,
) -> Result<()> {
    let effect_id = effect_name
        .parse::<crate::registry::EffectId>()
        .map_err(crate::error::TermzzzError::UnsupportedEffect)?;
    let size = crate::common::normalize_effect_size(terminal::size()?);
    let mut effect = crate::registry::AnyEffect::build(effect_id, config, size);
    test_effect(&mut effect, frames, speed, colors)
}
