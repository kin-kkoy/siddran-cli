//! Loading a Bag. The whole thing is read at once, deliberately: `reconcileNotes`
//! builds its keep-set from *every* note and deletes any `.md` not in it
//! (localStore.js:194-206), so a partial load would compute a partial keep-set
//! and delete the notes it never read. Eager loading is a correctness
//! requirement here, not a performance choice. The real Bag is ~223 KB of
//! markdown, so the cost is nil.

use crate::error::{io, Error, Result};
use crate::json;
use crate::model::{Completion, Daily, Event, Note, Project, Schedule, Task};
use crate::row::Row;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// One note as found on disk, with the bytes it was read from so fidelity can
/// be asserted without re-reading.
#[derive(Debug, Clone)]
pub struct NoteFile {
    pub path: PathBuf,
    /// `None` for a loose note in `notes/`, else the notebook folder name.
    pub notebook: Option<String>,
    pub note: Note,
    pub raw: String,
}

#[derive(Debug)]
pub struct Bag {
    pub root: PathBuf,
    pub notes: Vec<NoteFile>,
    /// Notebook folder name -> its `.siddran-notebook.json`, kept as a raw
    /// `Value` so keys we do not model survive a round trip untouched.
    pub notebooks: Vec<(String, Value)>,
    pub tasks: Option<Value>,
    pub calendar: Option<Value>,
    pub settings: Option<Value>,
}

impl Bag {
    pub fn open(root: impl Into<PathBuf>) -> Result<Bag> {
        let root = root.into();
        let notes_dir = root.join("notes");
        if !notes_dir.is_dir() && !root.join("tasks.siddran").is_file() {
            return Err(Error::NotABag(root));
        }

        let mut bag = Bag {
            notes: Vec::new(),
            notebooks: Vec::new(),
            tasks: read_opt(&root.join("tasks.siddran"))?,
            calendar: read_opt(&root.join("calendar.siddran"))?,
            settings: read_opt(&root.join("settings.siddran"))?,
            root,
        };

        if notes_dir.is_dir() {
            bag.load_notes(&notes_dir, None)?;
            for entry in read_dir_sorted(&notes_dir)? {
                if entry.is_dir() {
                    let name = file_name(&entry);
                    let meta = entry.join(".siddran-notebook.json");
                    if meta.is_file() {
                        bag.notebooks.push((name.clone(), json::parse(&meta)?));
                    }
                    bag.load_notes(&entry, Some(name))?;
                }
            }
        }
        Ok(bag)
    }

    fn load_notes(&mut self, dir: &Path, notebook: Option<String>) -> Result<()> {
        for path in read_dir_sorted(dir)? {
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).map_err(io(&path))?;
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled");
            self.notes.push(NoteFile {
                note: Note::from_md(&raw, stem),
                notebook: notebook.clone(),
                path,
                raw,
            });
        }
        Ok(())
    }
}

/// Typed views over the raw collections. Parsed on demand rather than at open:
/// the raw `Value` stays the source of truth for writing, and a caller that only
/// wants notes should not pay to deserialize 300 calendar events.
impl Bag {
    pub fn tasks(&self) -> Vec<Row<Task>> {
        self.tasks.as_ref().map(|v| Row::list(v, "tasks")).unwrap_or_default()
    }
    pub fn dailies(&self) -> Vec<Row<Daily>> {
        self.tasks.as_ref().map(|v| Row::list(v, "dailies")).unwrap_or_default()
    }
    pub fn completions(&self) -> Vec<Row<Completion>> {
        self.tasks.as_ref().map(|v| Row::list(v, "completions")).unwrap_or_default()
    }
    pub fn projects(&self) -> Vec<Row<Project>> {
        self.tasks.as_ref().map(|v| Row::list(v, "projects")).unwrap_or_default()
    }
    pub fn events(&self) -> Vec<Row<Event>> {
        self.calendar.as_ref().map(|v| Row::list(v, "events")).unwrap_or_default()
    }
    pub fn schedules(&self) -> Vec<Row<Schedule>> {
        self.calendar.as_ref().map(|v| Row::list(v, "schedules")).unwrap_or_default()
    }

    /// The settings blob is `{ "settings": { … } }`. Returned as a raw map: most
    /// keys are desktop-only visuals the TUI must preserve without interpreting.
    pub fn settings_map(&self) -> Option<&serde_json::Map<String, Value>> {
        self.settings.as_ref()?.get("settings")?.as_object()
    }

    /// Notes in a notebook, or the loose ones when `notebook` is `None`.
    pub fn notes_in(&self, notebook: Option<&str>) -> Vec<&NoteFile> {
        self.notes
            .iter()
            .filter(|n| n.notebook.as_deref() == notebook)
            .collect()
    }
}

fn read_opt(path: &Path) -> Result<Option<Value>> {
    if path.is_file() {
        Ok(Some(json::parse(path)?))
    } else {
        Ok(None)
    }
}

/// `read_dir` order is arbitrary on both apps; sorting makes *our* behaviour
/// deterministic, which matters because filename-collision resolution depends
/// on the order notes are visited.
fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).map_err(io(dir))? {
        out.push(e.map_err(io(dir))?.path());
    }
    out.sort();
    Ok(out)
}

fn file_name(p: &Path) -> String {
    p.file_name().and_then(|s| s.to_str()).unwrap_or_default().to_owned()
}
