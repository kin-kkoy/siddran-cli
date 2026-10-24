use crate::cmd::Ctx;
use crate::out::{self, Mode};
use clap::Subcommand;
use serde_json::json;
use siddran_core::logic::{deadline, priority, recurrence};
use siddran_core::query::{self, TaskFilter};
use siddran_core::bag::write;
use siddran_core::Row;

#[derive(Subcommand)]
pub enum Cmd {
    /// Add a task
    Add {
        title: String,
        /// low | normal | high
        #[arg(long, default_value = "normal")]
        priority: String,
        /// today | tomorrow | YYYY-MM-DD | YYYY-MM-DDTHH:MM
        #[arg(long)]
        due: Option<String>,
    },
    /// Mark a task complete
    Done { what: String },
    /// Mark a task not complete
    Undone { what: String },
    /// List tasks. Defaults to everything still open.
    Ls {
        /// Due today
        #[arg(long)]
        today: bool,
        /// Due before today
        #[arg(long)]
        overdue: bool,
        /// Has no due date
        #[arg(long)]
        undated: bool,
        /// Include completed
        #[arg(long)]
        all: bool,
    },
    /// Show one task in full
    Show { what: String },
}

#[derive(Subcommand)]
pub enum DailyCmd {
    /// Tick a recurring daily off for today
    Done { what: String },
    /// Untick today's completion
    Undone { what: String },
    /// List dailies — recurring ones, plus ephemerals that have not expired
    Ls {
        /// Include expired ephemerals
        #[arg(long)]
        all: bool,
    },
}

impl DailyCmd {
    pub fn mutates(&self) -> bool {
        matches!(self, DailyCmd::Done { .. } | DailyCmd::Undone { .. })
    }
}

#[derive(Subcommand)]
pub enum ProjectCmd {
    /// Tick a project subtask off
    Done { project: String, subtask: String },
    /// List projects with derived priority and progress
    Ls,
    /// Show a project's subtasks
    Show { what: String },
}

impl Cmd {
    pub fn mutates(&self) -> bool {
        matches!(self, Cmd::Add { .. } | Cmd::Done { .. } | Cmd::Undone { .. })
    }
}

pub fn run(cmd: Cmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    let rows = bag.tasks();
    match cmd {
        Cmd::Add {
            title,
            priority,
            due,
        } => {
            if !matches!(priority.as_str(), "low" | "normal" | "high") {
                eprintln!("priority must be low, normal or high");
                return false;
            }
            let mut rows = rows;
            let id = siddran_core::ids::uuid_v4();
            let mut row = Row::<siddran_core::model::Task>::new_task(&id, &title, &priority);
            if let Some(d) = due.as_deref() {
                match deadline::from_picker_value(d) {
                    Some((iso, all_day)) => row.set_due(Some(&iso), all_day),
                    None => {
                        eprintln!("could not read a date from {d:?} — try today, tomorrow, 2026-09-20, or 2026-09-20T14:30");
                        return false;
                    }
                }
            }
            rows.push(row);
            save_tasks(ctx, &rows, &bag.dailies(), &bag.completions(), &bag.projects())
                .map(|_| {
                    println!("added {id}");
                    true
                })
                .unwrap_or(false)
        }

        Cmd::Ls {
            today,
            overdue,
            undated,
            all,
        } => {
            let f = if today {
                TaskFilter::Today
            } else if overdue {
                TaskFilter::Overdue
            } else if undated {
                TaskFilter::Undated
            } else if all {
                TaskFilter::All
            } else {
                TaskFilter::Open
            };
            let sel = query::tasks(&rows, f);

            if mode == Mode::Json {
                let v: Vec<_> = sel.iter().map(|r| json!(r.raw)).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                return true;
            }
            if sel.is_empty() {
                out::empty(
                    mode,
                    match f {
                        TaskFilter::Today => "nothing due today.",
                        TaskFilter::Overdue => "nothing overdue.",
                        TaskFilter::Undated => "every task has a due date.",
                        TaskFilter::All => "this Bag has no tasks.",
                        TaskFilter::Open => "no open tasks — try --all to include completed.",
                        TaskFilter::Dated => "no tasks have a due date.",
                    },
                );
                return true;
            }
            let out_rows: Vec<Vec<String>> = sel
                .iter()
                .map(|r| {
                    let t = &r.v;
                    vec![
                        if t.is_completed { "✓" } else { "☐" }.to_owned(),
                        prio_glyph(&t.priority).to_owned(),
                        out::truncate(&t.title, 54),
                        t.due_date
                            .as_deref()
                            .map(|d| deadline::human(d, t.due_all_day))
                            .unwrap_or_else(|| "—".into()),
                        t.id.to_string(),
                    ]
                })
                .collect();
            out::rows(mode, &out_rows);
            true
        }

        Cmd::Done { what } => set_done(ctx, rows, &what, true),
        Cmd::Undone { what } => set_done(ctx, rows, &what, false),

        Cmd::Show { what } => {
            let Some(r) = rows.iter().find(|r| super::matches(&r.v.id, &r.v.title, &what)) else {
                eprintln!("no task matching {what:?}");
                return false;
            };
            if mode == Mode::Json {
                println!("{}", serde_json::to_string_pretty(&r.raw).unwrap());
                return true;
            }
            let t = &r.v;
            println!("{}", t.title);
            println!("  id         {}", t.id);
            println!("  status     {}", if t.is_completed { "complete" } else { "open" });
            println!("  priority   {}", t.priority);
            match t.due_date.as_deref() {
                Some(d) => {
                    println!("  due        {}  ({d})", deadline::human(d, t.due_all_day));
                    // Worth surfacing: a legacy row has no flag at all, and that
                    // absence is what makes it date-only.
                    println!(
                        "  all day    {}",
                        match t.due_all_day {
                            Some(b) => b.to_string(),
                            None => format!("(absent — legacy, reads as {})", deadline::is_all_day(d, None)),
                        }
                    );
                }
                None => println!("  due        —"),
            }
            if let Some(r) = &t.remind_at {
                println!("  remind     {r}");
            }
            println!("  updated    {}", t.updated_at);
            if let Some(d) = t.description.as_deref().filter(|d| !d.is_empty()) {
                println!("\n{d}");
            }
            true
        }
    }
}

pub fn run_daily(cmd: DailyCmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    let rows = bag.dailies();
    let all = match cmd {
        DailyCmd::Done { what } => return set_daily_done(ctx, &rows, &what, true),
        DailyCmd::Undone { what } => return set_daily_done(ctx, &rows, &what, false),
        DailyCmd::Ls { all } => all,
    };
    let sel: Vec<_> = if all {
        rows.iter().collect()
    } else {
        query::active_dailies(&rows)
    };

    if mode == Mode::Json {
        let v: Vec<_> = sel.iter().map(|r| json!(r.raw)).collect();
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return true;
    }

    if sel.is_empty() {
        out::empty(
            mode,
            if rows.is_empty() {
                "this Bag has no dailies."
            } else {
                // The desktop purges these 24h after creation; we only hide them.
                "no active dailies — the rest are expired ephemerals. Try --all."
            },
        );
        return true;
    }
    let completions = bag.completions();
    let today = deadline::today().to_string();
    let out_rows: Vec<Vec<String>> = sel
        .iter()
        .map(|r| {
            let d = &r.v;
            let done_today = completions
                .iter()
                .any(|c| c.v.daily_task_id == d.id && c.v.date == today);
            let rule = recurrence::parse(d.recurrence.as_deref());
            vec![
                if done_today || d.is_completed { "✓" } else { "☐" }.to_owned(),
                prio_glyph(&d.priority).to_owned(),
                out::truncate(&d.title, 44),
                rule.as_ref()
                    .map(recurrence::label)
                    .unwrap_or_else(|| "once".into()),
                d.time.clone().unwrap_or_else(|| "—".into()),
            ]
        })
        .collect();
    out::rows(mode, &out_rows);
    true
}

impl ProjectCmd {
    pub fn mutates(&self) -> bool {
        matches!(self, ProjectCmd::Done { .. })
    }
}

pub fn run_project(cmd: ProjectCmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    let rows = bag.projects();
    match cmd {
        ProjectCmd::Done { project, subtask } => {
            let mut rows = rows;
            let Some(i) = rows.iter().position(|r| super::matches(&r.v.id, &r.v.title, &project)) else {
                eprintln!("no project matching {project:?}");
                return false;
            };
            let sub = rows[i]
                .v
                .tasks
                .iter()
                .find(|t| super::matches(&t.id, &t.title, &subtask))
                .map(|t| (t.id.clone(), t.title.clone()));
            let Some((id, title)) = sub else {
                eprintln!("no subtask matching {subtask:?} in {:?}", rows[i].v.title);
                return false;
            };
            rows[i].set_subtask_completed(&id, true);
            let bucket = siddran_core::logic::priority::bucket(&rows[i].v.tasks);
            match save_tasks(ctx, &bag.tasks(), &bag.dailies(), &bag.completions(), &rows) {
                Ok(_) => {
                    println!("✓ {title}   (project priority is now {bucket})");
                    true
                }
                Err(_) => false,
            }
        }

        ProjectCmd::Ls => {
            if mode == Mode::Json {
                let v: Vec<_> = rows.iter().map(|r| json!(r.raw)).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                return true;
            }
            if rows.is_empty() {
                out::empty(mode, "this Bag has no projects.");
                return true;
            }
            let out_rows: Vec<Vec<String>> = rows
                .iter()
                .map(|r| {
                    let p = &r.v;
                    let (done, total) = priority::progress(&p.tasks);
                    vec![
                        if p.is_completed { "✓" } else { "☐" }.to_owned(),
                        out::truncate(&p.title, 40),
                        // Derived, not the stored field — the stored one can be stale.
                        priority::bucket(&p.tasks).to_owned(),
                        format!("{done}/{total}"),
                    ]
                })
                .collect();
            out::rows(mode, &out_rows);
            true
        }
        ProjectCmd::Show { what } => {
            let Some(r) = rows.iter().find(|r| super::matches(&r.v.id, &r.v.title, &what)) else {
                eprintln!("no project matching {what:?}");
                return false;
            };
            if mode == Mode::Json {
                println!("{}", serde_json::to_string_pretty(&r.raw).unwrap());
                return true;
            }
            let p = &r.v;
            let (done, total) = priority::progress(&p.tasks);
            println!("{}  ({done}/{total}, {})", p.title, priority::bucket(&p.tasks));
            let out_rows: Vec<Vec<String>> = p
                .tasks
                .iter()
                .map(|t| {
                    vec![
                        format!("  {}", if t.is_completed { "✓" } else { "☐" }),
                        prio_glyph(&t.priority).to_owned(),
                        out::truncate(&t.title, 54),
                    ]
                })
                .collect();
            out::rows(mode, &out_rows);
            true
        }
    }
}

fn prio_glyph(p: &str) -> &'static str {
    match p {
        "low" | "very_low" | "quite_low" => "▁",
        "high" | "very_high" | "quite_high" => "█",
        _ => "▄",
    }
}


// ── writes ──────────────────────────────────────────────────────────

fn set_done(ctx: &Ctx, mut rows: Vec<Row<siddran_core::model::Task>>, what: &str, done: bool) -> bool {
    let Some(i) = rows.iter().position(|r| super::matches(&r.v.id, &r.v.title, what)) else {
        eprintln!("no task matching {what:?}");
        return false;
    };
    if rows[i].v.is_completed == done {
        println!("{} is already {}", rows[i].v.title, if done { "done" } else { "open" });
        return true;
    }
    let title = rows[i].v.title.clone();
    rows[i].set_completed(done);

    let bag = &ctx.bag;
    match save_tasks(ctx, &rows, &bag.dailies(), &bag.completions(), &bag.projects()) {
        Ok(_) => {
            println!("{} {}", if done { "✓" } else { "☐" }, title);
            true
        }
        Err(_) => false,
    }
}

/// Today's completion for a recurring daily. Identity is `(daily_task_id, date)`
/// — completions have no id of their own (localStore.js:216-218).
fn set_daily_done(
    ctx: &Ctx,
    dailies: &[Row<siddran_core::model::Daily>],
    what: &str,
    done: bool,
) -> bool {
    let Some(d) = dailies.iter().find(|r| super::matches(&r.v.id, &r.v.title, what)) else {
        eprintln!("no daily matching {what:?}");
        return false;
    };
    if d.v.recurrence.is_none() {
        // Matches the desktop: only recurring dailies take per-day completions.
        eprintln!(
            "{:?} is a one-off daily, not recurring — it has no per-day completions",
            d.v.title
        );
        return false;
    }

    let today = siddran_core::logic::deadline::today().to_string();
    let bag = &ctx.bag;
    let mut completions = bag.completions();
    let existing = completions
        .iter()
        .position(|c| c.v.daily_task_id == d.v.id && c.v.date == today);

    match (done, existing) {
        (true, Some(_)) => {
            println!("{} is already done today", d.v.title);
            return true;
        }
        (false, None) => {
            println!("{} was not marked done today", d.v.title);
            return true;
        }
        (true, None) => {
            let row = serde_json::json!({ "daily_task_id": d.v.id, "date": today });
            completions.push(Row::from_value(&row).expect("literal deserializes"));
        }
        (false, Some(i)) => {
            completions.remove(i);
        }
    }

    match save_tasks(ctx, &bag.tasks(), dailies, &completions, &bag.projects()) {
        Ok(_) => {
            println!("{} {} for {today}", if done { "✓" } else { "☐" }, d.v.title);
            true
        }
        Err(_) => false,
    }
}

fn save_tasks(
    ctx: &Ctx,
    tasks: &[Row<siddran_core::model::Task>],
    dailies: &[Row<siddran_core::model::Daily>],
    completions: &[Row<siddran_core::model::Completion>],
    projects: &[Row<siddran_core::model::Project>],
) -> Result<(), ()> {
    if let Err(e) = ctx.before_write() {
        eprintln!("error: {e}");
        return Err(());
    }
    write::tasks_file(&ctx.root.join("tasks.siddran"), tasks, dailies, completions, projects)
        .map_err(|e| eprintln!("error: {e}"))
}
