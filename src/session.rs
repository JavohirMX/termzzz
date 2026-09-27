//! Terminal setup and teardown, shared by the screensaver loop and check mode.
//!
//! The two entry points used to carry their own near-identical copy of this
//! logic, which is how they drifted apart. They are one concern, so they are
//! one type.
//!
//! # Panics
//!
//! The release profile sets `panic = "abort"`, which means [`Drop`] does not run
//! when a panic unwinds the stack. Anything relying on `Drop` to leave the
//! terminal usable therefore does not run at all, and the user is left with a
//! terminal in raw mode inside the alternate screen, where Ctrl-C is not
//! interpreted as a signal and there is no shell prompt to return to.
//!
//! [`install_panic_hook`] closes that hole: it performs the same restore
//! sequence from inside the panic hook, before the process aborts. It is
//! installed before the first session is entered.

use std::io::{self, Stdout};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::{
    cursor,
    event::{
        DisableFocusChange, DisableMouseCapture, EnableFocusChange,
        EnableMouseCapture,
    },
    execute,
    style::{Color, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
    },
};

/// Whether a session currently owns the terminal. The panic hook has no access
/// to the session value, so it reads this instead.
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);

/// The colours the live session installed, so the panic hook can put them back.
///
/// The hook runs from inside a panic, where nothing is guaranteed to be
/// initialised and a poisoned lock is a real possibility, so a failure to read
/// this degrades to "reset both colours" rather than to a second panic. Resetting
/// is always safe: it cannot make the terminal worse than not having tried.
static SESSION_COLORS: Mutex<Option<SessionColors>> = Mutex::new(None);

fn remember_colors(colors: Option<SessionColors>) {
    if let Ok(mut slot) = SESSION_COLORS.lock() {
        *slot = colors;
    }
}

fn take_remembered_colors() -> Option<SessionColors> {
    match SESSION_COLORS.lock() {
        Ok(mut slot) => slot.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

/// The terminal colours a session should install, and put back on the way out.
///
/// Grouped into one type because they are always installed and restored
/// together, and because a `restore` that reset the background but forgot the
/// foreground would leave the terminal half-pinned. `Color::Reset` on both is
/// the default and means "leave the terminal alone".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionColors {
    pub background: Color,
    pub foreground: Color,
}

impl Default for SessionColors {
    fn default() -> Self {
        Self {
            background: Color::Reset,
            foreground: Color::Reset,
        }
    }
}

impl SessionColors {
    /// Whether either end is pinned, and so whether there is anything to emit.
    ///
    /// Checked so the overwhelmingly common default emits *nothing at all*,
    /// rather than a redundant pair of resets on every session start.
    pub fn is_default(&self) -> bool {
        self.background == Color::Reset && self.foreground == Color::Reset
    }
}

/// Puts the terminal into raw mode on the alternate screen and hides the
/// cursor, restoring everything on drop.
pub struct TerminalSession {
    stdout: Stdout,
    mouse_capture: bool,
    active: bool,
}

impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        Self::enter_with(SessionColors::default())
    }

    /// Enters a session, optionally pinning the terminal's own colours.
    ///
    /// The colours are set *after* entering the alternate screen and clearing,
    /// because both of those repaint every cell and a colour set before them
    /// would be undone by the clear.
    pub fn enter_with(colors: SessionColors) -> io::Result<Self> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;

        // Focus reporting is off by default, so without this the terminal never
        // sends `FocusGained`/`FocusLost` and the frame loop cannot tell whether
        // anyone is watching. A screensaver nobody is watching should not be
        // spending a core on it.
        let entered = execute!(
            stdout,
            EnterAlternateScreen,
            cursor::Hide,
            Clear(ClearType::All),
            EnableFocusChange
        );
        if let Err(error) = entered.and_then(|()| {
            if colors.is_default() {
                return Ok(());
            }
            execute!(
                stdout,
                SetForegroundColor(colors.foreground),
                SetBackgroundColor(colors.background),
            )
        }) {
            // Undo the half-applied state before reporting the failure, or the
            // caller is left with raw mode enabled and no way back.
            let _ = execute!(
                stdout,
                cursor::Show,
                Clear(ClearType::All),
                DisableFocusChange,
                LeaveAlternateScreen,
            );
            let _ = terminal::disable_raw_mode();
            return Err(error);
        }

        SESSION_ACTIVE.store(true, Ordering::SeqCst);
        remember_colors((!colors.is_default()).then_some(colors));

        Ok(Self {
            stdout,
            mouse_capture: false,
            active: true,
        })
    }

    pub fn stdout(&mut self) -> &mut Stdout {
        &mut self.stdout
    }

    /// Enables mouse reporting, which the interactive effects need. Failure is
    /// not fatal: the effect still runs, it just cannot see the pointer.
    pub fn enable_mouse(&mut self) -> io::Result<()> {
        self.mouse_capture = true;
        execute!(self.stdout, EnableMouseCapture)
    }

    /// Returns the terminal to a usable state. Idempotent, and safe to call
    /// from both the panic hook and `Drop`.
    pub fn restore(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        SESSION_ACTIVE.store(false, Ordering::SeqCst);

        if self.mouse_capture {
            let _ = execute!(self.stdout, DisableMouseCapture);
        }
        // Ahead of leaving the alternate screen, and ahead of the clear: both
        // repaint, and a repaint would otherwise re-derive its background from
        // whatever the profile says and quietly undo the pin. Read from the
        // static rather than the field so this and the panic hook cannot
        // disagree about what needs resetting.
        if take_remembered_colors().is_some() {
            let _ = execute!(
                self.stdout,
                SetForegroundColor(Color::Reset),
                SetBackgroundColor(Color::Reset),
            );
        }
        let _ = execute!(
            self.stdout,
            cursor::Show,
            Clear(ClearType::All),
            DisableFocusChange,
            LeaveAlternateScreen,
        );
        let _ = terminal::disable_raw_mode();
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.restore();
    }
}

/// Restores the terminal from inside a panic, before the process aborts.
///
/// Installs a hook that runs [`restore_terminal`] and then delegates to the
/// previous hook, so the default panic message is still printed. Call once,
/// before entering a session; calling it twice chains hooks, which is harmless
/// but pointless.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        previous(info);
    }));
}

/// Best-effort terminal restore that does not need the session value.
pub fn restore_terminal() {
    if !SESSION_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }

    let mut stdout = io::stdout();
    // Same ordering and same reason as `TerminalSession::restore`.
    if take_remembered_colors().is_some() {
        let _ = execute!(
            stdout,
            SetForegroundColor(Color::Reset),
            SetBackgroundColor(Color::Reset),
        );
    }
    let _ = execute!(
        stdout,
        DisableMouseCapture,
        DisableFocusChange,
        cursor::Show,
        Clear(ClearType::All),
        LeaveAlternateScreen,
    );
    let _ = terminal::disable_raw_mode();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoring_without_a_session_is_a_no_op() {
        // The panic hook can fire before any session was entered, and after one
        // was already restored. Neither may touch the terminal.
        assert!(!SESSION_ACTIVE.load(Ordering::SeqCst));
        restore_terminal();
        assert!(!SESSION_ACTIVE.load(Ordering::SeqCst));
    }

    #[test]
    fn the_hook_can_be_installed_more_than_once() {
        // Chaining hooks is harmless; what must not happen is a panic while
        // installing one.
        install_panic_hook();
        install_panic_hook();
        restore_terminal();
    }

    /// Serialises the tests that read or write [`SESSION_COLORS`].
    ///
    /// That slot is process-global by necessity -- the panic hook has no access
    /// to a session value -- so the tests that exercise it have to take turns.
    /// Without this they are not merely flaky in the usual sense: both would
    /// pass in isolation and fail together, which is the worst failure mode for
    /// a test.
    static GLOBAL_STATE: Mutex<()> = Mutex::new(());

    fn global_state() -> std::sync::MutexGuard<'static, ()> {
        GLOBAL_STATE.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn the_default_colours_emit_nothing_at_all() {
        // The overwhelmingly common case is a user who never touched the
        // setting, and it must not cost a single byte. A redundant pair of
        // resets is not merely untidy: it repaints, and a repaint gives the
        // terminal a chance to re-derive its background from the profile.
        assert!(SessionColors::default().is_default());
    }

    #[test]
    fn pinning_either_end_counts_as_pinning() {
        // Tested per end because a bug that only inspected the background would
        // pass a test that set both, and the foreground is exactly the field a
        // user is likelier to set on its own.
        let only_background = SessionColors {
            background: Color::Black,
            foreground: Color::Reset,
        };
        let only_foreground = SessionColors {
            background: Color::Reset,
            foreground: Color::Rgb {
                r: 0xcc,
                g: 0xcc,
                b: 0xdd,
            },
        };
        assert!(!only_background.is_default());
        assert!(!only_foreground.is_default());
    }

    #[test]
    fn remembered_colours_are_taken_exactly_once() {
        // `take`, not `get`: both restore paths can run -- `Drop` and then the
        // panic hook, or the reverse -- and a second read that still reported a
        // live pin would emit a redundant reset after the terminal had already
        // been handed back to the user.
        let _guard = global_state();
        remember_colors(Some(SessionColors {
            background: Color::Black,
            foreground: Color::Reset,
        }));
        assert!(take_remembered_colors().is_some());
        assert!(take_remembered_colors().is_none());
    }

    #[test]
    fn forgetting_the_colours_leaves_nothing_to_restore() {
        let _guard = global_state();
        remember_colors(None);
        assert!(take_remembered_colors().is_none());
    }
}
