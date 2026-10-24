use crate::cmd::Ctx;
use crate::out::{self, Mode};
use clap::Subcommand;
use serde_json::json;
use siddran_core::bag::notes;
use siddran_core::logic::deadline;
use siddran_core::Bag;

#[derive(Subcommand)]
pub enum Cmd {
    /// List notes
    Ls {
        /// Only notes in this notebook
        #[arg(long, value_name = "NAME")]
        notebook: Option<String>,
        /// Only notes carrying this tag
        #[arg(long, value_name = "TAG")]
        tag: Option<String>,
        /// Only favourites
        #[arg(long)]
        fav: bool,
        /// Only unfiled notes
        #[arg(long)]
        unfiled: bool,
    },
    /// Print a note's markdown to stdout
    Cat {
        /// Note id or title (prefix match)
        what: String,
        /// Include the frontmatter block
        #[arg(long)]
        raw: bool,
    },
    /// Print the path to a note's file
    Path { what: String },
    /// Create a note
    New {
        title: String,
        /// Put it in this notebook (must already exist)
        #[arg(long, value_name = "NAME")]
        notebook: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Body text, or `-` to read stdin
        #[arg(long)]
        body: Option<String>,
    },
    /// Change a note's title, renaming its file too
    Rename { what: String, title: String },
    /// Move a note to another notebook, or out of one
    Mv {
        what: String,
        /// Target notebook; omit for unfiled
        #[arg(long, value_name = "NAME")]
        notebook: Option<String>,
    },
    /// Toggle a note's favourite flag
    Fav { what: String },
    /// Move a note to .siddran-trash/
    Rm {
        what: String,
        /// Skip the confirmation
        #[arg(long)]
        yes: bool,
    },
}

impl Cmd {
    pub fn mutates(&self) -> bool {
        matches!(
            self,
            Cmd::New { .. } | Cmd::Rename { .. } | Cmd::Mv { .. } | Cmd::Fav { .. } | Cmd::Rm { .. }
        )
    }
}

#[derive(Subcommand)]
pub enum NotebookCmd {
    /// List notebooks
    Ls,
}

pub fn run(cmd: Cmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    match cmd {
        Cmd::Ls {
            notebook,
            tag,
            fav,
            unfiled,
        } => {
            let sel: Vec<_> = bag
                .notes
                .iter()
                .filter(|n| match &notebook {
                    Some(nb) => n.notebook.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(nb)),
                    None => !unfiled || n.notebook.is_none(),
                })
                .filter(|n| !fav || n.note.favorite)
                .filter(|n| match &tag {
                    Some(t) => n
                        .note
                        .tags
                        .split(',')
                        .any(|x| x.trim().eq_ignore_ascii_case(t)),
                    None => true,
                })
                .collect();

            if mode == Mode::Json {
                let v: Vec<_> = sel
                    .iter()
                    .map(|n| {
                        json!({
                            "id": n.note.id, "title": n.note.title, "tags": n.note.tags,
                            "favorite": n.note.favorite, "color": n.note.color,
                            "notebook": n.notebook, "updated": n.note.updated,
                            "path": n.path, "bytes": n.note.body.len(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                return true;
            }

            if sel.is_empty() {
                out::empty(mode, "no notes match.");
                return true;
            }
            let rows: Vec<Vec<String>> = sel
                .iter()
                .map(|n| {
                    vec![
                        if n.note.favorite { "★" } else { " " }.to_owned(),
                        out::truncate(&n.note.title, 52),
                        n.notebook.clone().unwrap_or_else(|| "—".into()),
                        day(&n.note.updated),
                        size(n.note.body.len()),
                    ]
                })
                .collect();
            out::rows(mode, &rows);
            true
        }

        Cmd::Cat { what, raw } => match find(bag, &what) {
            Some(n) => {
                print!("{}", if raw { n.raw.clone() } else { n.note.body.clone() });
                true
            }
            None => {
                eprintln!("no note matching {what:?}");
                false
            }
        },

        Cmd::Path { what } => match find(bag, &what) {
            Some(n) => {
                println!("{}", n.path.display());
                true
            }
            None => {
                eprintln!("no note matching {what:?}");
                false
            }
        },

        Cmd::New {
            title,
            notebook,
            tags,
            body,
        } => new_note(ctx, &title, notebook.as_deref(), tags.as_deref(), body.as_deref()),

        Cmd::Rename { what, title } => with_note(ctx, &what, |ctx, nf| {
            let mut note = nf.note.clone();
            note.title = title.clone();
            note.updated = deadline::now_iso();
            let to = notes::target_path(&ctx.bag, note.id.as_deref(), &title, nf.notebook.as_deref());
            notes::save_moving(&note, &nf.path, &to, &ctx.root)
                .map(|_| println!("{}", to.display()))
        }),

        Cmd::Mv { what, notebook } => with_note(ctx, &what, |ctx, nf| {
            if let Some(nb) = notebook.as_deref() {
                if !ctx.bag.notebooks.iter().any(|(n, _)| n == nb) {
                    // Creating the folder here would also mean writing a
                    // notebook sidecar and inventing an id — a separate verb.
                    return Err(siddran_core::Error::NotABag(ctx.root.join("notes").join(nb)));
                }
            }
            let mut note = nf.note.clone();
            note.updated = deadline::now_iso();
            let to = notes::target_path(&ctx.bag, note.id.as_deref(), &note.title, notebook.as_deref());
            notes::save_moving(&note, &nf.path, &to, &ctx.root)
                .map(|_| println!("{}", to.display()))
        }),

        Cmd::Fav { what } => with_note(ctx, &what, |ctx, nf| {
            let mut note = nf.note.clone();
            note.favorite = !note.favorite;
            note.updated = deadline::now_iso();
            let _ = ctx;
            notes::save(&note, &nf.path).map(|_| {
                println!("{} {}", if note.favorite { "★" } else { "○" }, note.title);
            })
        }),

        Cmd::Rm { what, yes } => {
            let Some(nf) = find(bag, &what) else {
                eprintln!("no note matching {what:?}");
                return false;
            };
            if !yes {
                match confirm(&format!("move \"{}\" to the trash?", nf.note.title)) {
                    Confirm::Yes => {}
                    Confirm::No => {
                        println!("cancelled");
                        return true;
                    }
                    // A script that pipes in has not answered "no" — it has
                    // failed to answer. Exiting 0 would look like success.
                    Confirm::NoTerminal => {
                        eprintln!("refusing to delete without a terminal — pass --yes");
                        return false;
                    }
                }
            }
            if let Err(e) = ctx.before_write() {
                eprintln!("error: {e}");
                return false;
            }
            match notes::delete(nf, &ctx.root) {
                Ok(moved) => {
                    for m in moved {
                        println!("trashed {}", m.display());
                    }
                    true
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    false
                }
            }
        }
    }
}

// ── writes ──────────────────────────────────────────────────────────

/// Find the note, guard the write, run the operation.
fn with_note<F>(ctx: &Ctx, what: &str, f: F) -> bool
where
    F: FnOnce(&Ctx, &siddran_core::bag::NoteFile) -> siddran_core::Result<()>,
{
    let Some(nf) = find(&ctx.bag, what) else {
        eprintln!("no note matching {what:?}");
        return false;
    };
    if let Err(e) = ctx.before_write() {
        eprintln!("error: {e}");
        return false;
    }
    match f(ctx, nf) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

fn new_note(
    ctx: &Ctx,
    title: &str,
    notebook: Option<&str>,
    tags: Option<&str>,
    body: Option<&str>,
) -> bool {
    if let Some(nb) = notebook {
        if !ctx.bag.notebooks.iter().any(|(n, _)| n == nb) {
            eprintln!("no notebook named {nb:?} — `siddran notebook ls` to see them");
            return false;
        }
    }
    let body = match body {
        Some("-") => {
            let mut s = String::new();
            use std::io::Read;
            if std::io::stdin().read_to_string(&mut s).is_err() {
                eprintln!("could not read stdin");
                return false;
            }
            s
        }
        Some(b) => format!("{b}\n"),
        None => String::new(),
    };

    let now = deadline::now_iso();
    // Notes keep NUMERIC ids: mdToNote does `Number(fm.id)`, so a UUID in note
    // frontmatter comes back as NaN on the desktop side.
    let id = notes::next_note_id(&ctx.bag).to_string();
    let note = siddran_core::Note {
        id: Some(id.clone()),
        title: title.to_owned(),
        tags: tags.unwrap_or("").to_owned(),
        favorite: false,
        color: String::new(),
        order: None,
        created: now.clone(),
        updated: now,
        body,
        had_frontmatter: true,
    };

    let path = notes::target_path(&ctx.bag, Some(&id), title, notebook);
    if path.exists() {
        eprintln!("{} already exists", path.display());
        return false;
    }
    if let Err(e) = ctx.before_write() {
        eprintln!("error: {e}");
        return false;
    }
    match notes::save(&note, &path) {
        Ok(()) => {
            println!("{}", path.display());
            true
        }
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

enum Confirm {
    Yes,
    No,
    NoTerminal,
}

/// Deleting is the one thing worth a prompt. A non-interactive caller has not
/// answered "no" — it has failed to answer, which is a different outcome and
/// deserves a different exit code. `--yes` is the explicit opt-in.
fn confirm(question: &str) -> Confirm {
    use std::io::{IsTerminal, Write};
    if !std::io::stdin().is_terminal() {
        return Confirm::NoTerminal;
    }
    print!("{question} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return Confirm::No;
    }
    match line.trim() {
        "y" | "Y" | "yes" => Confirm::Yes,
        _ => Confirm::No,
    }
}

pub fn run_notebook(cmd: NotebookCmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    let NotebookCmd::Ls = cmd;
    if mode == Mode::Json {
        let v: Vec<_> = bag
            .notebooks
            .iter()
            .map(|(name, meta)| {
                json!({ "name": name, "notes": bag.notes_in(Some(name)).len(), "meta": meta })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return true;
    }
    let mut rows: Vec<Vec<String>> = vec![vec![
        "—".into(),
        "(unfiled)".into(),
        bag.notes_in(None).len().to_string(),
    ]];
    for (name, meta) in &bag.notebooks {
        rows.push(vec![
            meta.get("color").and_then(|c| c.as_str()).unwrap_or("—").to_owned(),
            name.clone(),
            bag.notes_in(Some(name)).len().to_string(),
        ]);
    }
    out::rows(mode, &rows);
    true
}

fn find<'a>(bag: &'a Bag, needle: &str) -> Option<&'a siddran_core::bag::NoteFile> {
    let n = needle.to_lowercase();
    bag.notes
        .iter()
        .find(|f| f.note.id.as_deref() == Some(needle) || f.note.title.to_lowercase() == n)
        .or_else(|| {
            bag.notes
                .iter()
                .find(|f| f.note.title.to_lowercase().starts_with(&n))
        })
}

/// `2026-08-26T08:36:16.570Z` -> `08-26`. Enough to scan a list by.
fn day(iso: &str) -> String {
    iso.get(5..10).unwrap_or("—").to_owned()
}

fn size(bytes: usize) -> String {
    if bytes >= 1024 {
        format!("{:.1}k", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}")
    }
}
