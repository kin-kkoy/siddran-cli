//! The terminal's own settings, in `~/.config/siddran/config.toml`.
//!
//! Deliberately separate from `settings.siddran`. That file belongs to the
//! desktop app and is full of keys the terminal has no opinion about
//! (`starCount`, `bookPageHeight`); writing to it would mean round-tripping
//! values we do not model, which is exactly the kind of thing `docs/parity.md`
//! exists to prevent. The TUI shows those read-only and never touches them.
//!
//! The reader is a deliberately tiny `key = "value"` parser rather than a TOML
//! dependency — this is a flat file of five keys. The writer preserves every
//! line it does not recognise, including comments, so hand-edits survive.

use std::path::{Path, PathBuf};

pub fn path() -> Option<PathBuf> {
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(x) if !x.is_empty() => PathBuf::from(x),
        _ => home()?.join(".config"),
    };
    Some(base.join("siddran/config.toml"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn expand_tilde(s: &str) -> String {
    match (s.strip_prefix("~/"), home()) {
        (Some(rest), Some(h)) => h.join(rest).to_string_lossy().into_owned(),
        _ => s.to_owned(),
    }
}

/// One key's value, or `None` when absent or empty.
pub fn get(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path()?).ok()?;
    parse(&text, key)
}

fn parse(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix(key) else { continue };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        let v = rest.trim().trim_matches('"').trim_matches('\'');
        if !v.is_empty() {
            return Some(v.to_owned());
        }
    }
    None
}

/// Set one key, preserving every other line — comments included.
pub fn set(key: &str, value: &str) -> Result<PathBuf, String> {
    let p = path().ok_or("no home directory, so no config path")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }

    let existing = std::fs::read_to_string(&p).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let mut replaced = false;

    for line in existing.lines() {
        let t = line.trim();
        let is_key = !t.starts_with('#')
            && t.strip_prefix(key)
                .and_then(|r| r.trim_start().strip_prefix('='))
                .is_some();
        if is_key && !replaced {
            out.push(format!("{key} = \"{value}\""));
            replaced = true;
        } else if is_key {
            // A duplicate of a key we just wrote would shadow it on read.
            continue;
        } else {
            out.push(line.to_owned());
        }
    }
    if !replaced {
        if out.is_empty() {
            out.push("# Siddran terminal settings".into());
        }
        out.push(format!("{key} = \"{value}\""));
    }

    let body = out.join("\n") + "\n";
    write_atomic(&p, body.as_bytes())?;
    Ok(p)
}

/// Same temp+rename discipline the Bag writer uses: a crash mid-write must not
/// leave a truncated config.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res.map_err(|e| format!("{}: {e}", path.display()))
}
