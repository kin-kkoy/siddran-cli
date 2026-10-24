//! Writing notes — deliberately **not** a port of `reconcileNotes`.
//!
//! The desktop's reconcile builds a keep-set from every note in memory and then
//! deletes any `.md` in `notes/` that is not in it, plus any *directory* that is
//! not a notebook folder — recursively (localStore.js:194-206). That is correct
//! there, because its in-memory db is the whole truth and it syncs the entire
//! tree on every flush.
//!
//! It is the wrong shape for this crate. A CLI mutates one thing and exits, so a
//! global keep-set buys nothing and risks everything: any bug in it silently
//! deletes notes, and it would also delete anything a user or another tool had
//! put in `notes/` that we failed to model. Targeted operations reach the same
//! end state — write this file, remove that one — and **cannot mass-delete by
//! construction**.
//!
//! The one thing given up is cleaning orphans left by a desktop crash. That is
//! reported by `siddran bag check` rather than fixed silently, and the desktop
//! reconciles them itself next time it runs.

use crate::error::Result;
use crate::model::Note;
use std::path::{Path, PathBuf};

use super::{atomic, layout::safe_name, trash, Bag, NoteFile};

/// Where a note belongs, given its title and notebook.
///
/// Collision rule from localStore.js:174: the first note to claim
/// `<title>.md` keeps it, and a later one with the same title becomes
/// `<title>-<id>.md`. "Already claimed" means *by a different note* — rewriting
/// a note in place must not rename it.
pub fn target_path(bag: &Bag, note_id: Option<&str>, title: &str, notebook: Option<&str>) -> PathBuf {
    let dir = match notebook {
        Some(nb) => bag.root.join("notes").join(safe_name(nb)),
        None => bag.root.join("notes"),
    };
    let base = safe_name(title);
    let plain = dir.join(format!("{base}.md"));

    let claimed_by_other = bag.notes.iter().any(|n| {
        n.path == plain && n.note.id.as_deref() != note_id
    });

    if claimed_by_other {
        // A note with no id cannot disambiguate this way; fall back to the
        // plain name and let the caller's own uniqueness check catch it.
        match note_id {
            Some(id) => dir.join(format!("{base}-{id}.md")),
            None => plain,
        }
    } else {
        plain
    }
}

/// Next numeric note id: `max(existing numeric ids, 1000) + 1`
/// (localStore.js:143-147). Notes must keep numeric ids — `mdToNote` does
/// `Number(fm.id)`, so a UUID in note frontmatter would come back as `NaN` on
/// the desktop side. Only tasks/dailies/projects/events use UUIDs.
pub fn next_note_id(bag: &Bag) -> i64 {
    let mut max = 1000i64;
    for n in &bag.notes {
        if let Some(v) = n.note.id.as_deref().and_then(|s| s.parse::<i64>().ok()) {
            max = max.max(v);
        }
    }
    for (_, meta) in &bag.notebooks {
        if let Some(v) = meta.get("id").and_then(|v| v.as_i64()) {
            max = max.max(v);
        }
    }
    max + 1
}

/// Write one note to its own path. Used for content edits, where the filename
/// does not change.
pub fn save(note: &Note, path: &Path) -> Result<()> {
    atomic::write(path, note.to_md().as_bytes())
}

/// Write a note to a new path and remove the old one — a rename or a move
/// between notebooks. The comment sidecar travels with it.
pub fn save_moving(note: &Note, from: &Path, to: &Path, root: &Path) -> Result<()> {
    atomic::write(to, note.to_md().as_bytes())?;
    if from != to {
        move_sidecar(from, to, root)?;
        // Only after the new file is safely on disk.
        if from.exists() {
            trash::remove(root, from)?;
        }
    }
    Ok(())
}

/// Trash a note and its sidecar. Returns where they went.
pub fn delete(nf: &NoteFile, root: &Path) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    let sidecar = sidecar_of(&nf.path);
    if sidecar.exists() {
        moved.push(trash::remove(root, &sidecar)?);
    }
    moved.push(trash::remove(root, &nf.path)?);
    Ok(moved)
}

/// `<note>.md` -> `<note>.comments.json` (localStore.js:181).
pub fn sidecar_of(note_path: &Path) -> PathBuf {
    let s = note_path.to_string_lossy();
    PathBuf::from(match s.strip_suffix(".md") {
        Some(stem) => format!("{stem}.comments.json"),
        None => s.into_owned(),
    })
}

fn move_sidecar(from: &Path, to: &Path, root: &Path) -> Result<()> {
    let (a, b) = (sidecar_of(from), sidecar_of(to));
    if a.exists() && a != b {
        let bytes = std::fs::read(&a).map_err(crate::error::io(&a))?;
        atomic::write(&b, &bytes)?;
        trash::remove(root, &a)?;
    }
    Ok(())
}
