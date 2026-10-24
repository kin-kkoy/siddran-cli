//! Finding the Bag, in precedence order:
//!   1. `--bag <path>`
//!   2. `$SIDDRAN_BAG`
//!   3. `bag = "…"` in the config (see `config.rs`)
//!   4. `~/Documents/Siddran`, if it looks like a Bag
//!
//! The desktop remembers its recent Bags in `siddran_recent_bags`, but that
//! lives inside WebKit's localStorage SQLite under
//! `~/.local/share/com.kinkkoy.siddran`. Reading a browser engine's private
//! database would be a fragile dependency for a nicety, so it is deliberately
//! not done — say where the Bag is instead.

use std::path::{Path, PathBuf};

pub fn resolve(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        return Ok(p);
    }
    if let Ok(p) = std::env::var("SIDDRAN_BAG") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    if let Some(v) = crate::config::get("bag") {
        return Ok(PathBuf::from(crate::config::expand_tilde(&v)));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let guess = home.join("Documents/Siddran");
        if looks_like_a_bag(&guess) {
            return Ok(guess);
        }
    }
    Err("no Bag found. Pass --bag <path>, set SIDDRAN_BAG, or put \
         `bag = \"/path/to/Bag\"` in ~/.config/siddran/config.toml"
        .to_owned())
}

pub fn looks_like_a_bag(p: &Path) -> bool {
    p.join("notes").is_dir() || p.join("tasks.siddran").is_file()
}

/// Which editor opens a note body.
///
/// `$EDITOR` and `$VISUAL` are both commonly unset — they are on this machine —
/// so a bare read of them is not enough. The fallback chain prefers something
/// that needs nothing learned: micro behaves like any ordinary editor, whereas
/// landing a first-time user in vi with no way out is a genuinely bad outcome.
pub fn editor() -> String {
    if let Some(e) = crate::config::get("editor") {
        return e;
    }
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return v;
            }
        }
    }
    for candidate in ["micro", "nano", "hx", "helix", "nvim", "vim", "vi"] {
        if which(candidate) {
            return candidate.to_owned();
        }
    }
    "vi".to_owned()
}

/// Where the editor came from, so the settings view can say so rather than
/// showing a value with no explanation for why it is that.
pub fn editor_source() -> &'static str {
    if crate::config::get("editor").is_some() {
        "from your config"
    } else if std::env::var("VISUAL").is_ok_and(|v| !v.trim().is_empty()) {
        "from $VISUAL"
    } else if std::env::var("EDITOR").is_ok_and(|v| !v.trim().is_empty()) {
        "from $EDITOR"
    } else {
        "auto-detected ($EDITOR is unset)"
    }
}

fn which(bin: &str) -> bool {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|d| Path::new(d).join(bin).is_file())
}
