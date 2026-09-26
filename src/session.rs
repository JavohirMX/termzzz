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
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::{
    cursor,
    event::{
        DisableFocusChange, DisableMouseCapture, EnableFocusChange,
        EnableMouseCapture,
    },
    execute,
    terminal::{
        self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
    },
};

/// Whether a session currently owns the terminal. The panic hook has no access
/// to the session value, so it reads this instead.
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Puts the terminal into raw mode on the alternate screen and hides the
/// cursor, restoring everything on drop.
pub struct TerminalSession {
    stdout: Stdout,
    mouse_capture: bool,
    active: bool,
}

impl TerminalSession {
    pub fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout();
        terminal::enable_raw_mode()?;

        // Focus reporting is off by default, so without this the terminal never
        // sends `FocusGained`/`FocusLost` and the frame loop cannot tell whether
        // anyone is watching. A screensaver nobody is watching should not be
        // spending a core on it.
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            cursor::Hide,
            Clear(ClearType::All),
            EnableFocusChange
        ) {
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
}
