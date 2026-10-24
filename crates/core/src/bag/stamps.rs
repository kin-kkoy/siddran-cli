//! Staleness detection: did the Bag change under us since we read it?
//!
//! The lock stops two writers interleaving; this stops a writer acting on a
//! stale read. We record `(len, mtime)` for every file at open, and re-check
//! just before writing. A mismatch aborts the write rather than clobbering
//! whatever the other app just did.
//!
//! It is detection, not prevention — but for a CLI that opens, mutates and exits
//! in milliseconds, the window it cannot cover is correspondingly tiny.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone)]
pub struct Stamps(HashMap<PathBuf, (u64, Option<std::time::SystemTime>)>);

impl Stamps {
    pub fn record(&mut self, path: &Path) {
        if let Ok(m) = std::fs::metadata(path) {
            self.0.insert(path.to_path_buf(), (m.len(), m.modified().ok()));
        }
    }

    /// Paths whose size or mtime differs from what we recorded. A file that has
    /// since been deleted counts as changed.
    pub fn changed(&self) -> Vec<&Path> {
        self.0
            .iter()
            .filter(|(p, (len, mtime))| match std::fs::metadata(p) {
                Ok(m) => m.len() != *len || m.modified().ok() != *mtime,
                Err(_) => true,
            })
            .map(|(p, _)| p.as_path())
            .collect()
    }
}
