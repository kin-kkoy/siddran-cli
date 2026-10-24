//! Deletions go to `.siddran-trash/` rather than straight to `unlink`.
//!
//! `reconcileNotes` is the only destructive code in either app, and a keep-set
//! bug there silently deletes notes. This crate does not run a global reconcile
//! (see `notes.rs`), but deletion is still deletion: until this has lived on a
//! real Bag for a while, a removed file should be recoverable.
//!
//! The trash sits inside the Bag so it travels with it, and is skipped by
//! everything that scans the Bag.

use crate::error::{io, Result};
use std::path::{Path, PathBuf};

pub const DIR: &str = ".siddran-trash";

pub fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}

/// Moves `path` into the Bag's trash, keeping enough of its original location to
/// be recognisable, and never overwriting an earlier casualty.
pub fn remove(root: &Path, path: &Path) -> Result<PathBuf> {
    let trash = dir(root);
    std::fs::create_dir_all(&trash).map_err(io(&trash))?;

    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "__");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut target = trash.join(format!("{stamp}-{rel}"));
    let mut n = 1;
    while target.exists() {
        target = trash.join(format!("{stamp}-{n}-{rel}"));
        n += 1;
    }

    // A rename across filesystems fails; a Bag is one tree, but fall back to
    // copy+unlink so an odd mount cannot turn "delete" into "error".
    if std::fs::rename(path, &target).is_err() {
        std::fs::copy(path, &target).map_err(io(path))?;
        std::fs::remove_file(path).map_err(io(path))?;
    }
    Ok(target)
}
