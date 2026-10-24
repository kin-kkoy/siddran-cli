//! Writes that cannot leave a half-file behind.
//!
//! The desktop truncates and writes (`main.rs:51-57`), so a crash mid-write can
//! leave a zero-length note. We write a temp file beside the target, fsync it,
//! then rename over — rename within a filesystem is atomic, so a reader sees
//! either the old file or the new one, never a partial one. The resulting bytes
//! are identical, so there is no divergence to manage.

use crate::error::{io, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(io(parent))?;

    // The temp name carries the pid so two processes cannot collide, and starts
    // with `.siddran-tmp-` so it is recognisable if one is ever orphaned. It is
    // never a `.md` or `.comments.json`, so nothing that scans the notes tree
    // will mistake it for content.
    let tmp = parent.join(format!(
        ".siddran-tmp-{}-{}",
        std::process::id(),
        path.file_name().and_then(|s| s.to_str()).unwrap_or("x")
    ));

    let res = (|| -> Result<()> {
        let mut f = File::create(&tmp).map_err(io(&tmp))?;
        f.write_all(bytes).map_err(io(&tmp))?;
        f.sync_all().map_err(io(&tmp))?;
        drop(f);
        fs::rename(&tmp, path).map_err(io(path))
    })();

    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

/// Durability for the *directory entry* — a rename is atomic but not
/// necessarily persisted until the directory is synced. Called once per flush
/// rather than per file: 44 notes should not mean 44 directory syncs.
pub fn sync_dir(dir: &Path) -> Result<()> {
    if let Ok(f) = File::open(dir) {
        let _ = f.sync_all();
    }
    Ok(())
}
