//! A write session: take the lock, then read, then verify nothing moved before
//! writing.
//!
//! The TUI is the exception: it stays open for hours, so it holds no lock while
//! it sits there — it takes one per write (`lock_for_write`) and, between
//! keypresses, notices when someone else moved the Bag (`moved`) and reloads.
//!
//! Read-only commands deliberately do **not** come through here. If they did,
//! `siddran task ls` would fail whenever a TUI had the Bag open, which is
//! exactly when you would want to run it. Reading a JSON file that may be
//! mid-rename is safe — the writer is atomic, so a reader sees the old file or
//! the new one, never a torn one.
//!
//! The lock is taken *before* the read so the snapshot we validate against is
//! one nobody else could have been writing.

use siddran_core::bag::{lock, stamps::Stamps};
use siddran_core::Bag;
use std::path::Path;

pub struct Session {
    _guard: Option<lock::Guard>,
    stamps: Stamps,
}

/// How long a write waits for someone else's write to finish.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Everything whose change means "the Bag moved": the three row files, every
/// note, and the folders — a directory's mtime moves when a file is added to or
/// removed from it, which is how a note created elsewhere gets noticed.
fn stamp_all(root: &Path, bag: &Bag) -> Stamps {
    let mut stamps = Stamps::default();
    for f in ["tasks.siddran", "calendar.siddran", "settings.siddran"] {
        stamps.record(&root.join(f));
    }
    let notes = root.join("notes");
    stamps.record(&notes);
    if let Ok(rd) = std::fs::read_dir(&notes) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                stamps.record(&e.path());
            }
        }
    }
    for n in &bag.notes {
        stamps.record(&n.path);
    }
    stamps
}

impl Session {
    pub fn begin(root: &Path) -> Result<(Session, Bag), String> {
        Session::begin_as(root, "cli")
    }

    /// `mode` is recorded in the lockfile so a human reading it can tell a
    /// long-lived TUI session from a one-shot command.
    pub fn begin_as(root: &Path, mode: &str) -> Result<(Session, Bag), String> {
        let guard = if mode == "tui" {
            None
        } else {
            Some(lock::acquire_wait(root, mode, LOCK_WAIT).map_err(|e| e.to_string())?)
        };
        let bag = Bag::open(root).map_err(|e| e.to_string())?;
        let stamps = stamp_all(root, &bag);
        Ok((Session { _guard: guard, stamps }, bag))
    }

    /// For a session that holds no lock (the TUI): take it for one write. Drop
    /// the guard as soon as the write is done.
    pub fn lock_for_write(&self, root: &Path) -> Result<lock::Guard, String> {
        lock::acquire_wait(root, "tui", LOCK_WAIT).map_err(|e| e.to_string())
    }

    /// Has anything we read changed on disk since? Costs a `stat` per file.
    pub fn moved(&self) -> bool {
        !self.stamps.changed().is_empty()
    }

    /// Re-record the stamps against the Bag as it now is.
    ///
    /// Essential after *our own* writes: otherwise the file we just wrote looks
    /// like someone else's change and the next write is refused. Called from
    /// `App::reload`, which runs after every TUI write.
    pub fn refresh(&mut self, root: &Path, bag: &Bag) {
        self.stamps = stamp_all(root, bag);
    }

    /// Re-record one file we know changed on our account.
    ///
    /// The editor hand-off writes through a child process, so the guard sees
    /// that write as a foreign change and refuses the metadata update that
    /// should follow it. We launched that editor, so the change is ours.
    pub fn restamp(&mut self, path: &Path) {
        self.stamps.record(path);
    }

    pub fn check_unchanged(&self) -> Result<(), String> {
        let changed = self.stamps.changed();
        if changed.is_empty() {
            return Ok(());
        }
        Err(format!(
            "the Bag changed on disk since it was read — refusing to write over it.\n  {}\nRun the command again; it will read the new state.",
            changed
                .iter()
                .take(5)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n  ")
        ))
    }
}
