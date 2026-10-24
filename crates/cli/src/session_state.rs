//! "Continue where I left off", for the terminal.
//!
//! The desktop has a `rememberNoteState` toggle in `settings.siddran`. That flag
//! is the one setting in that file with a meaning the terminal can honour, so it
//! does: when it is on, the TUI restores each workspace's feature and selection
//! on the next launch.
//!
//! The *flag* lives in the desktop's file and is never written here. The
//! remembered position lives in ours — it is session state, not configuration,
//! and the desktop has no use for it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Spot {
    pub feature: String,
    /// Notes are remembered by path, not index: an index means something
    /// different the moment a note is added or renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub source: usize,
    #[serde(default)]
    pub group: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Remembered {
    pub active: usize,
    pub spaces: Vec<Spot>,
}

fn path() -> Option<PathBuf> {
    crate::config::path().map(|p| p.with_file_name("session.json"))
}

/// Keyed by Bag, so two Bags remember separate positions.
fn all() -> BTreeMap<String, Remembered> {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn load(bag: &Path) -> Option<Remembered> {
    all().get(&bag.display().to_string()).cloned()
}

/// Best-effort: failing to remember a position is not worth an error on quit.
pub fn save(bag: &Path, state: &Remembered) {
    let Some(p) = path() else { return };
    let mut map = all();
    map.insert(bag.display().to_string(), state.clone());
    if let Ok(text) = serde_json::to_string_pretty(&map) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, text);
    }
}
