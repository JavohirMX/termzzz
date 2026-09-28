use crate::common::TerminalEffect;
use crate::error::Result;
use crate::runtime::{FrameContext, InputState};
use crate::session::{SessionColors, TerminalSession};
use crossterm::{
    cursor,
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

    for frame in 1..=frames {
        let context =
            FrameContext::new(size, frame as u64, delta, delta, input.clone());
        execute!(session.stdout(), Clear(ClearType::All))?;
        let diff = effect.get_diff_with_context(&context);

        // The screensaver loop's encoder, not a second copy of it. This used to
        // open-code `PrintStyledContent` per cell, which is the encoding that
        // drops the background colour entirely and resets the style after every
        // glyph -- so check mode and the real loop showed different colours for
        // the same frame.
        crate::common::write_cells(session.stdout(), size, &diff, colors)?;
        execute!(
            session.stdout(),
            cursor::MoveTo(0, 0),
            crossterm::style::Print(format!("Frame: {}", frame))
        )?;
        session.stdout().flush()?;
        effect.update_with_context(&context);
    }

    loop {
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(_) = event::read()?
        {
            break;
        }
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
