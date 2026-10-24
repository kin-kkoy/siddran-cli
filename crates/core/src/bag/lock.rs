//! An advisory lock, so two Siddran processes never write the same Bag.
//!
//! `O_EXCL` creation is the primitive — it either creates the file or tells us
//! someone else did. The payload is readable JSON on purpose: an fcntl/flock
//! lock is invisible to a human, invisible to the desktop app either way, and
//! unreliable on network filesystems. "Advisory" should mean something you can
//! look at.
//!
//! **Held per write, never per session** (since 2026-10-24). The desktop app
//! now takes this same file around each flush, and `siddran mcp` around each
//! tool call. A lock held for as long as a program is *open* would lock every
//! other writer out for that whole time — so everyone holds it only for the
//! read-check-write of one change, and waits briefly (`acquire_wait`) when
//! someone else is mid-write, rather than failing.

use crate::error::{io, Result};
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const NAME: &str = ".siddran-lock";

#[derive(Debug, Serialize, Deserialize)]
pub struct Holder {
    pub pid: u32,
    pub host: String,
    pub started_at: String,
    pub mode: String,
}

#[derive(Debug)]
pub enum LockError {
    /// Someone else holds it, and they look alive.
    Held(Holder),
    Io(crate::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Held(h) => write!(
                f,
                "this Bag is already open (pid {} on {}, since {}). \
                 Close it, or run `siddran bag unlock --force` if that process is gone.",
                h.pid, h.host, h.started_at
            ),
            LockError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Releases the lock on drop — including on panic, which is the case that
/// matters: a crashed process must not leave the Bag permanently unopenable.
#[derive(Debug)]
pub struct Guard {
    path: PathBuf,
    ours: bool,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.ours {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn acquire(root: &Path, mode: &str) -> std::result::Result<Guard, LockError> {
    let path = root.join(NAME);

    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut f) => {
            let holder = Holder {
                pid: std::process::id(),
                host: hostname(),
                started_at: crate::logic::deadline::now_iso(),
                mode: mode.to_owned(),
            };
            let body = serde_json::to_string_pretty(&holder).unwrap_or_default();
            f.write_all(body.as_bytes())
                .map_err(|e| LockError::Io(crate::Error::Io { path: path.clone(), source: e }))?;
            Ok(Guard { path, ours: true })
        }

        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let holder: Option<Holder> = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());

            match holder {
                // A lock from this host whose process is gone is stale: reclaim
                // it. A lock from *another* host is never reclaimed
                // automatically — we cannot see that machine's process table.
                Some(h) if h.host == hostname() && !pid_alive(h.pid) => {
                    let _ = std::fs::remove_file(&path);
                    acquire(root, mode)
                }
                Some(h) => Err(LockError::Held(h)),
                // Unreadable payload: treat as held rather than stomping it.
                None => Err(LockError::Held(Holder {
                    pid: 0,
                    host: "unknown".into(),
                    started_at: "unknown".into(),
                    mode: "unknown".into(),
                })),
            }
        }

        Err(e) => Err(LockError::Io(crate::Error::Io { path, source: e })),
    }
}

/// `acquire`, but wait up to `wait` for a holder that is mid-write to finish.
/// Every Siddran writer holds the lock for milliseconds, so a short wait turns
/// "the Bag is busy" from an error into a pause nobody notices.
pub fn acquire_wait(root: &Path, mode: &str, wait: std::time::Duration) -> std::result::Result<Guard, LockError> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match acquire(root, mode) {
            Err(LockError::Held(h)) if std::time::Instant::now() < deadline => {
                let _ = h;
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other,
        }
    }
}

/// Remove a lock regardless of who holds it. Only for `bag unlock --force`.
pub fn force_release(root: &Path) -> Result<bool> {
    let path = root.join(NAME);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path).map_err(io(&path))?;
    Ok(true)
}

pub fn read(root: &Path) -> Option<Holder> {
    serde_json::from_str(&std::fs::read_to_string(root.join(NAME)).ok()?).ok()
}

fn pid_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "unknown".into())
}
