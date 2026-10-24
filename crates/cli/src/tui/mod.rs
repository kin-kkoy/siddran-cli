//! The TUI: single-threaded, event-driven, and idle at ~0% CPU.
//!
//! The loop blocks in `crossterm::event::poll`, which on Unix waits in
//! `epoll_wait` over stdin plus crossterm's SIGWINCH pipe. While waiting the
//! thread is off the run queue entirely.
//!
//! Measured, not assumed — on the real Bag, one thread, 20 s idle:
//!   CPU 0.000 s (0.000% of a core) · RSS 5.8 MB · 0 context switches in 10 s
//!   `/proc/<pid>/wchan` reads `__x64_sys_epoll_wait`
//! Re-check this whenever the loop changes; it is the requirement the whole
//! project exists for.
//!
//! Two things would break that, and both are avoided deliberately:
//!
//! 1. **Polling with a short timeout.** Nearly every ratatui example is written
//!    as `poll(16ms)` then an unconditional `draw`, which is 62 wakeups a second
//!    forever. Here a redraw happens only when a message says state changed.
//! 2. **Background threads.** There is no work slow enough to need one —
//!    hydrating the real Bag is ~2 ms and rendering a note is microseconds — so
//!    there is nothing to wait on but the user. That is also what makes editor
//!    hand-off safe (see `term::Guard::suspended`).
//!
//! Timers are coalesced into a single next-deadline. With nothing pending the
//! loop blocks for a bounded "forever"; the only routine wakeup is local
//! midnight, because "today" and "overdue" are date-dependent.

mod app;
mod keymap;
mod render;
mod settings;
mod term;
mod theme;

pub use app::App;
pub use theme::{Frame, Palette};

use crossterm::event::{self, Event};
use std::time::{Duration, Instant};

/// What a key press did, which is also what decides whether we repaint.
pub enum Effect {
    None,
    Redraw,
    Quit,
}

pub fn run(app: &mut App) -> std::io::Result<()> {
    let (mut term, guard) = term::Guard::enter()?;
    term.draw(|f| render::draw(f, app))?;

    loop {
        // A bounded "forever". Waking once a day costs nothing and means a
        // session left open overnight still says "today" correctly.
        let timeout = app
            .next_deadline()
            .map(|d| d.saturating_duration_since(Instant::now()))
            .unwrap_or(Duration::from_secs(86_400));

        let effect = match event::poll(timeout) {
            Ok(true) => match event::read()? {
                // Windows sends key-release events too; only presses act.
                Event::Key(k) if k.kind == event::KeyEventKind::Press => {
                    // Act on the Bag as it is now, not as it was at the last
                    // keypress: the desktop app or an AI may have written since.
                    let moved = app.refresh_if_moved();
                    let e = app.on_key(k, &guard, &mut term);
                    app.end_key();
                    match e {
                        Effect::None if moved => Effect::Redraw,
                        e => e,
                    }
                }
                Event::Resize(_, _) => Effect::Redraw,
                _ => Effect::None,
            },
            // The deadline fired rather than a key arriving.
            Ok(false) => app.on_deadline(),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Effect::None,
            Err(e) => return Err(e),
        };

        match effect {
            Effect::Redraw => {
                term.draw(|f| render::draw(f, app))?;
            }
            Effect::Quit => break,
            Effect::None => {}
        }
    }

    drop(guard);
    Ok(())
}
