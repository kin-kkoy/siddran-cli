//! The live channel between a running desktop app and anything else on this
//! machine — `siddran mcp` above all.
//!
//! Two small files per Bag, outside the Bag (in `$XDG_RUNTIME_DIR`, a per-user
//! tmpfs): the Bag is a folder people sync between machines, and "what is open
//! on this screen right now" is neither theirs to sync nor anything another
//! machine could act on.
//!
//! - `presence-<key>.json` — written by the desktop whenever what you are
//!   looking at changes: the page, the open note, the open board.
//! - `request-<key>.json` — written by anyone else to ask the desktop to show
//!   something ("open this note"). The desktop watches for it, acts, deletes it.
//!
//! Files rather than a socket or a local HTTP port: nothing listens, so there
//! is nothing for another program — or a web page — to connect to, and the
//! desktop can be closed without anyone noticing anything but a stale file.
//!
//! `<key>` is FNV-1a over the Bag's canonical path. Siddran-Desktop computes the
//! same key (src-tauri/src/presence.rs); change one, change both.

use serde_json::Value;
use std::path::{Path, PathBuf};

pub fn key(bag_root: &Path) -> String {
    let p = std::fs::canonicalize(bag_root).unwrap_or_else(|_| bag_root.to_path_buf());
    let mut h: u64 = 0xcbf29ce484222325;
    for b in p.to_string_lossy().as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir());
    base.join("siddran")
}

pub fn presence_path(bag_root: &Path) -> PathBuf {
    dir().join(format!("presence-{}.json", key(bag_root)))
}

pub fn request_path(bag_root: &Path) -> PathBuf {
    dir().join(format!("request-{}.json", key(bag_root)))
}

/// What the desktop last said it was showing, if it is still running.
pub fn read(bag_root: &Path) -> Option<Value> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(presence_path(bag_root)).ok()?).ok()?;
    let pid = v.get("pid").and_then(Value::as_u64)?;
    // A presence file outlives a crash; a dead pid means nobody is looking.
    Path::new(&format!("/proc/{pid}")).exists().then_some(v)
}

/// Ask the desktop to show something. Atomic, so it never reads half a request.
pub fn request(bag_root: &Path, req: &Value) -> crate::Result<()> {
    let p = request_path(bag_root);
    crate::bag::atomic::write(&p, serde_json::to_string(req).unwrap_or_default().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable_and_path_specific() {
        // Same literal as Siddran-Desktop's src-tauri/src/presence.rs test: the two
        // apps find each other's files only while these agree.
        assert_eq!(key(Path::new("/nonexistent/bag-a")), "6a15c151c3b63ae2");
        assert_ne!(key(Path::new("/nonexistent/bag-a")), key(Path::new("/nonexistent/bag-b")));
    }
}
