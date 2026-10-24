//! Terminal lifecycle.
//!
//! The rule: whatever happens — clean exit, panic, SIGTERM, a hand-off to an
//! editor — the terminal must come back. A TUI that leaves raw mode on is a
//! terminal the user has to close.

use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub struct Guard;

impl Guard {
    pub fn enter() -> io::Result<(Tui, Guard)> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, crossterm::cursor::Hide)?;

        // A panic in raw mode would otherwise print the message into a screen
        // that cannot scroll, on a terminal with no echo. Restore first, then
        // let the default hook print normally.
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore();
            default(info);
        }));

        Ok((Terminal::new(CrosstermBackend::new(io::stdout()))?, Guard))
    }

    /// Step out of the alternate screen so an external program can own the
    /// terminal, then step back. This is what makes `$EDITOR` hand-off work —
    /// and the reason the event loop is single-threaded: a background reader
    /// thread would already be blocked inside `read()` and would swallow the
    /// first keystroke meant for the editor.
    pub fn suspended<T>(&self, term: &mut Tui, f: impl FnOnce() -> T) -> io::Result<T> {
        restore()?;
        let out = f();
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, crossterm::cursor::Hide)?;
        term.clear()?;
        Ok(out)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = restore();
    }
}

pub fn restore() -> io::Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show)?;
    disable_raw_mode()
}
