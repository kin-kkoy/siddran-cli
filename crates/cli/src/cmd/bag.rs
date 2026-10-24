use crate::cmd::Ctx;
use crate::out::{self, Mode};
use clap::Subcommand;
use serde_json::json;
use siddran_core::bag::{lock, trash};
use siddran_core::json as sjson;
use std::path::Path;

#[derive(Subcommand)]
pub enum Cmd {
    /// Print the resolved Bag path
    Path,
    /// What this Bag holds
    Status,
    /// Verify every file round-trips byte-for-byte (writes nothing)
    Check,
    /// Release a lock left behind by a process that is gone
    Unlock {
        /// Remove it even if the holder looks alive
        #[arg(long)]
        force: bool,
    },
    /// What is in .siddran-trash/
    Trash,
}

impl Cmd {
    pub fn mutates(&self) -> bool {
        // `unlock` deliberately does NOT take the lock — it exists to remove one.
        false
    }
}

pub fn run(cmd: Cmd, ctx: &Ctx) -> bool {
    let (bag, root, mode) = (&ctx.bag, ctx.root.as_path(), ctx.mode);
    match cmd {
        Cmd::Path => {
            println!("{}", root.display());
            true
        }

        Cmd::Status => {
            let tasks = bag.tasks();
            let open = tasks.iter().filter(|t| !t.v.is_completed).count();
            if mode == Mode::Json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "bag": root, "notes": bag.notes.len(), "notebooks": bag.notebooks.len(),
                        "tasks": tasks.len(), "tasks_open": open, "dailies": bag.dailies().len(),
                        "projects": bag.projects().len(), "events": bag.events().len(),
                        "schedules": bag.schedules().len(),
                    }))
                    .unwrap()
                );
                return true;
            }
            let rows = vec![
                vec!["bag".into(), root.display().to_string()],
                vec![
                    "notes".into(),
                    format!("{} in {} notebook(s)", bag.notes.len(), bag.notebooks.len()),
                ],
                vec!["tasks".into(), format!("{} ({open} open)", tasks.len())],
                vec!["dailies".into(), bag.dailies().len().to_string()],
                vec!["projects".into(), bag.projects().len().to_string()],
                vec![
                    "events".into(),
                    format!("{} in {} schedule(s)", bag.events().len(), bag.schedules().len()),
                ],
            ];
            out::rows(mode, &rows);
            true
        }

        // The Phase 0 contract, exposed as a command: re-serialize everything and
        // report anything whose bytes would change. Nothing is written.
        Cmd::Check => {
            let mut checked = 0usize;
            let mut bad = Vec::new();
            for n in &bag.notes {
                if !n.note.had_frontmatter {
                    continue;
                }
                checked += 1;
                if n.note.to_md() != n.raw {
                    bad.push(n.path.display().to_string());
                }
            }
            let mut check = |p: std::path::PathBuf, v: &serde_json::Value| {
                if let Ok(orig) = std::fs::read_to_string(&p) {
                    checked += 1;
                    if sjson::to_string(v) != orig {
                        bad.push(p.display().to_string());
                    }
                }
            };
            for (name, v) in &bag.notebooks {
                check(root.join("notes").join(name).join(".siddran-notebook.json"), v);
            }
            for (f, v) in [
                ("tasks.siddran", &bag.tasks),
                ("calendar.siddran", &bag.calendar),
                ("settings.siddran", &bag.settings),
            ] {
                if let Some(v) = v {
                    check(root.join(f), v);
                }
            }

            if bad.is_empty() {
                println!("{checked} file(s) checked — byte-identical ✓");
                true
            } else if false { unreachable!() } else {
                println!("{} of {checked} file(s) WOULD BE REWRITTEN:", bad.len());
                for p in &bad {
                    println!("  {p}");
                }
                false
            }
        }

        Cmd::Unlock { force } => match lock::read(root) {
            None => {
                println!("no lock held");
                true
            }
            Some(h) => {
                let alive = Path::new(&format!("/proc/{}", h.pid)).exists();
                if alive && !force {
                    eprintln!(
                        "pid {} on {} still looks alive (since {}). \
                         Close it, or pass --force if you are sure.",
                        h.pid, h.host, h.started_at
                    );
                    return false;
                }
                match lock::force_release(root) {
                    Ok(_) => {
                        println!("released the lock held by pid {}", h.pid);
                        true
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        false
                    }
                }
            }
        },

        Cmd::Trash => {
            let dir = trash::dir(root);
            let Ok(entries) = std::fs::read_dir(&dir) else {
                println!("trash is empty");
                return true;
            };
            let mut rows: Vec<Vec<String>> = Vec::new();
            for e in entries.flatten() {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                rows.push(vec![
                    e.file_name().to_string_lossy().into_owned(),
                    format!("{len} B"),
                ]);
            }
            if rows.is_empty() {
                println!("trash is empty");
            } else {
                rows.sort();
                println!("{}", dir.display());
                out::rows(mode, &rows);
            }
            true
        }
    }
}
