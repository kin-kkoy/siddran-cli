//! The tools: notes, tasks, calendar, the desktop's live state. Sandbox tools
//! live in `sandbox.rs`.
//!
//! Each write goes through `writing`, which is the whole concurrency story: lock,
//! fresh read, change, atomic write, unlock. Each handler returns JSON the model
//! reads, or an error string written for the model to act on ("matches 3 notes:
//! …" rather than "ambiguous").

use serde_json::{json, Value};
use siddran_core::bag::{lock, notes, write, NoteFile};
use siddran_core::logic::{calendar, deadline, priority, recurrence};
use siddran_core::model::{Daily, Event, Project, Task};
use siddran_core::query::{self, TaskFilter};
use siddran_core::{Bag, Row};
use std::path::Path;
use std::time::Duration;

pub type R = Result<Value, String>;

// ── plumbing ────────────────────────────────────────────────────────

pub fn reading(root: &Path) -> Result<Bag, String> {
    Bag::open(root).map_err(|e| e.to_string())
}

/// One write: lock (waiting out anyone mid-write), read fresh, change, release.
pub fn writing<T>(root: &Path, f: impl FnOnce(&Bag) -> Result<T, String>) -> Result<T, String> {
    let _g = lock::acquire_wait(root, "mcp", Duration::from_secs(5)).map_err(|e| e.to_string())?;
    let bag = Bag::open(root).map_err(|e| e.to_string())?;
    f(&bag)
}

pub fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty())
}
pub fn need<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    s(a, k).ok_or_else(|| format!("`{k}` is required"))
}
fn flag(a: &Value, k: &str) -> Option<bool> {
    a.get(k).and_then(Value::as_bool)
}
fn strings(a: &Value, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(v)) => v.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        Some(Value::String(x)) if !x.trim().is_empty() => vec![x.clone()],
        _ => Vec::new(),
    }
}

/// Find one thing by id, then exact title, then unique prefix, then unique
/// substring (all case-insensitive). Ambiguity is an error that names the
/// candidates, so the model can retry with an id instead of guessing.
pub fn pick<'a, T>(kind: &str, needle: &str, items: &'a [T], id: impl Fn(&T) -> String, title: impl Fn(&T) -> String) -> Result<&'a T, String> {
    let n = needle.trim().to_lowercase();
    if let Some(x) = items.iter().find(|x| id(x) == needle.trim()) {
        return Ok(x);
    }
    for test in [
        &(|t: &str| t == n) as &dyn Fn(&str) -> bool,
        &|t: &str| t.starts_with(&n),
        &|t: &str| t.contains(&n),
    ] {
        let hits: Vec<&T> = items.iter().filter(|x| test(&title(x).to_lowercase())).collect();
        match hits.len() {
            0 => continue,
            1 => return Ok(hits[0]),
            k => {
                let list: Vec<String> = hits.iter().take(8).map(|x| format!("{} — {}", id(x), title(x))).collect();
                return Err(format!("{needle:?} matches {k} {kind}s; use an id:\n  {}", list.join("\n  ")));
            }
        }
    }
    Err(format!("no {kind} matching {needle:?}"))
}

fn save_tasks(bag: &Bag, tasks: &[Row<Task>], dailies: &[Row<Daily>], completions: &[Row<siddran_core::model::Completion>], projects: &[Row<Project>]) -> Result<(), String> {
    write::tasks_file(&bag.root.join("tasks.siddran"), tasks, dailies, completions, projects).map_err(|e| e.to_string())
}

fn when(v: &str) -> Result<(String, bool), String> {
    deadline::from_picker_value(v.trim())
        .ok_or_else(|| format!("could not read a date from {v:?} — use today, tomorrow, YYYY-MM-DD, or YYYY-MM-DDTHH:MM"))
}

// ── definitions ─────────────────────────────────────────────────────

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": props, "required": required },
    })
}

pub fn definitions() -> Value {
    let mut v = vec![
        tool("siddran_overview", "Start here. Today's agenda, overdue tasks, notebooks, recently edited notes, sandboxes, and what the user has open in the desktop app right now.", json!({}), &[]),
        tool("app_context", "What the user is looking at in the Siddran desktop app right now (page, open note, open board), or that it is not running.", json!({}), &[]),
        tool("app_open", "Show something in the Siddran desktop app: a note, a sandbox board, the tasks page, or a calendar day.", json!({
            "note": { "type": "string", "description": "Note id or title" },
            "sandbox": { "type": "string", "description": "Board id or title" },
            "page": { "type": "string", "enum": ["notes", "tasks", "calendar", "sandboxes"] },
            "date": { "type": "string", "description": "YYYY-MM-DD, with page=calendar" },
        }), &[]),

        tool("notes_search", "Find notes by words in the title, body or tags. Empty query lists recent notes.", json!({
            "query": { "type": "string" },
            "notebook": { "type": "string", "description": "Only this notebook" },
            "tag": { "type": "string" },
            "limit": { "type": "integer", "default": 20 },
        }), &[]),
        tool("note_read", "Read a note's full markdown body and metadata.", json!({ "note": { "type": "string", "description": "Note id or title" } }), &["note"]),
        tool("note_create", "Create a note. The notebook is created if it does not exist.", json!({
            "title": { "type": "string" },
            "body": { "type": "string", "description": "Markdown" },
            "notebook": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" } },
        }), &["title"]),
        tool("note_edit", "Change part of a note. Prefer the targeted forms — `replacements` (exact find/replace, like a code edit), `insert_after` / `insert_before` an exact anchor, `section` (rewrite everything under one heading), `append`, `prepend` — over `body`: the user may be typing in the same note, and a targeted edit merges with theirs where a full rewrite cannot. Applied in that order.", json!({
            "note": { "type": "string", "description": "Note id or title" },
            "replacements": { "type": "array", "items": { "type": "object", "properties": {
                "find": { "type": "string", "description": "Exact text currently in the note; must occur once unless all=true" },
                "replace": { "type": "string" },
                "all": { "type": "boolean" },
            }, "required": ["find", "replace"] } },
            "insert_after": { "type": "array", "items": { "type": "object", "properties": {
                "anchor": { "type": "string", "description": "Exact text currently in the note (must occur once)" },
                "text": { "type": "string", "description": "Markdown to put on the line(s) after the line the anchor ends on" },
            }, "required": ["anchor", "text"] } },
            "insert_before": { "type": "array", "items": { "type": "object", "properties": {
                "anchor": { "type": "string" }, "text": { "type": "string", "description": "Markdown to put on the line(s) before the line the anchor starts on" },
            }, "required": ["anchor", "text"] } },
            "section": { "type": "object", "properties": {
                "heading": { "type": "string", "description": "Heading text, without the #s" },
                "body": { "type": "string", "description": "New content under that heading, up to the next heading of the same or higher level" },
            }, "required": ["heading", "body"] },
            "append": { "type": "string", "description": "Markdown added at the end" },
            "prepend": { "type": "string", "description": "Markdown added at the start" },
            "body": { "type": "string", "description": "Replace the entire body (last resort)" },
        }), &["note"]),
        tool("note_update", "Rename a note, move it between notebooks, or change its tags / favourite flag.", json!({
            "note": { "type": "string" },
            "title": { "type": "string" },
            "notebook": { "type": "string", "description": "Target notebook; empty string for none" },
            "tags": { "type": "array", "items": { "type": "string" } },
            "favorite": { "type": "boolean" },
        }), &["note"]),
        tool("note_delete", "Move a note to the Bag's trash (.siddran-trash/, recoverable).", json!({ "note": { "type": "string" } }), &["note"]),

        tool("tasks_list", "List tasks.", json!({
            "filter": { "type": "string", "enum": ["open", "today", "overdue", "upcoming", "undated", "completed", "all"], "default": "open" },
            "query": { "type": "string", "description": "Only tasks whose title contains this" },
        }), &[]),
        tool("task_add", "Add a task.", json!({
            "title": { "type": "string" },
            "due": { "type": "string", "description": "today | tomorrow | YYYY-MM-DD | YYYY-MM-DDTHH:MM" },
            "priority": { "type": "string", "enum": ["low", "normal", "high"] },
            "description": { "type": "string" },
        }), &["title"]),
        tool("task_update", "Change a task: title, due date (empty string clears it), priority, description, or completed.", json!({
            "task": { "type": "string", "description": "Task id or title" },
            "title": { "type": "string" },
            "due": { "type": "string" },
            "priority": { "type": "string", "enum": ["low", "normal", "high"] },
            "description": { "type": "string" },
            "completed": { "type": "boolean" },
        }), &["task"]),
        tool("task_delete", "Delete a task.", json!({ "task": { "type": "string" } }), &["task"]),
        tool("dailies_list", "List daily tasks and habits, with whether each is done today.", json!({}), &[]),
        tool("daily_add", "Add a daily task. With `repeat` it is a habit; without, a one-off for today.", json!({
            "title": { "type": "string" },
            "repeat": { "description": "every-day | weekdays | weekends | a list of days like [\"mon\",\"thu\"]" },
            "time": { "type": "string", "description": "HH:MM" },
        }), &["title"]),
        tool("daily_check", "Tick a daily off (or untick it) for today or a given date.", json!({
            "daily": { "type": "string" },
            "done": { "type": "boolean", "default": true },
            "date": { "type": "string", "description": "YYYY-MM-DD, default today" },
        }), &["daily"]),
        tool("projects_list", "List projects (bundles of subtasks) with progress.", json!({}), &[]),
        tool("project_add", "Create a project with subtasks.", json!({
            "title": { "type": "string" },
            "subtasks": { "type": "array", "items": { "type": "string" } },
        }), &["title"]),
        tool("project_update", "Add, complete, reopen or remove a project's subtasks, or rename / complete the project.", json!({
            "project": { "type": "string" },
            "title": { "type": "string" },
            "completed": { "type": "boolean" },
            "add": { "type": "array", "items": { "type": "string" } },
            "complete": { "type": "array", "items": { "type": "string" }, "description": "Subtask ids or titles" },
            "reopen": { "type": "array", "items": { "type": "string" } },
            "remove": { "type": "array", "items": { "type": "string" } },
        }), &["project"]),

        tool("agenda", "What is on the calendar: events, dated tasks and habits, day by day.", json!({
            "from": { "type": "string", "description": "YYYY-MM-DD, default today" },
            "days": { "type": "integer", "default": 7 },
        }), &[]),
        tool("event_add", "Add a calendar event. A date without a time is all-day.", json!({
            "title": { "type": "string" },
            "start": { "type": "string", "description": "YYYY-MM-DD or YYYY-MM-DDTHH:MM (local)" },
            "end": { "type": "string", "description": "Default: one hour after a timed start; the same day for all-day" },
            "description": { "type": "string" },
            "color": { "type": "string", "description": "#rrggbb" },
        }), &["title", "start"]),
        tool("event_update", "Change a calendar event.", json!({
            "event": { "type": "string", "description": "Event id or title" },
            "title": { "type": "string" },
            "start": { "type": "string" },
            "end": { "type": "string" },
            "description": { "type": "string" },
            "color": { "type": "string" },
        }), &["event"]),
        tool("event_delete", "Delete a calendar event.", json!({ "event": { "type": "string" } }), &["event"]),
    ];
    v.extend(super::sandbox::definitions());
    Value::Array(v)
}

pub fn call(root: &Path, name: &str, a: &Value) -> R {
    match name {
        "siddran_overview" => overview(root),
        "app_context" => app_context(root),
        "app_open" => app_open(root, a),
        "notes_search" => notes_search(root, a),
        "note_read" => note_read(root, a),
        "note_create" => note_create(root, a),
        "note_edit" => note_edit(root, a),
        "note_update" => note_update(root, a),
        "note_delete" => note_delete(root, a),
        "tasks_list" => tasks_list(root, a),
        "task_add" => task_add(root, a),
        "task_update" => task_update(root, a),
        "task_delete" => task_delete(root, a),
        "dailies_list" => dailies_list(root),
        "daily_add" => daily_add(root, a),
        "daily_check" => daily_check(root, a),
        "projects_list" => projects_list(root),
        "project_add" => project_add(root, a),
        "project_update" => project_update(root, a),
        "agenda" => agenda(root, a),
        "event_add" => event_add(root, a),
        "event_update" => event_update(root, a),
        "event_delete" => event_delete(root, a),
        other => super::sandbox::call(root, other, a).unwrap_or_else(|| Err(format!("no tool named {other:?}"))),
    }
}

// ── overview / app ──────────────────────────────────────────────────

fn overview(root: &Path) -> R {
    let bag = reading(root)?;
    let today = deadline::today();
    let tasks = bag.tasks();
    let items = calendar::day_items(today, &bag.events(), &tasks, &bag.dailies(), &bag.completions());
    let mut recent: Vec<&NoteFile> = bag.notes.iter().collect();
    recent.sort_by(|a, b| b.note.updated.cmp(&a.note.updated));
    let boards = siddran_core::bag::sandbox::list(root).unwrap_or_default();
    Ok(json!({
        "today": today.to_string(),
        "agenda_today": items.iter().map(item_json).collect::<Vec<_>>(),
        "overdue": query::tasks(&tasks, TaskFilter::Overdue).into_iter().map(task_json).collect::<Vec<_>>(),
        "open_tasks": query::tasks(&tasks, TaskFilter::Open).len(),
        "notes": bag.notes.len(),
        "notebooks": bag.notebooks.iter().map(|(n, _)| json!({ "name": n, "notes": bag.notes_in(Some(n)).len() })).collect::<Vec<_>>(),
        "recent_notes": recent.iter().take(8).map(|n| note_brief(n)).collect::<Vec<_>>(),
        "sandboxes": boards.iter().take(8).map(|b| json!({ "id": b.id(), "title": b.title(), "items": b.items.len() })).collect::<Vec<_>>(),
        "desktop": app_state(root),
    }))
}

fn app_state(root: &Path) -> Value {
    match siddran_core::presence::read(root) {
        Some(p) => p,
        None => json!({ "running": false }),
    }
}

fn app_context(root: &Path) -> R {
    Ok(app_state(root))
}

fn app_open(root: &Path, a: &Value) -> R {
    if siddran_core::presence::read(root).is_none() {
        return Err("the Siddran desktop app is not running with this Bag open".into());
    }
    let bag = reading(root)?;
    let req = if let Some(n) = s(a, "note") {
        let nf = find_note(&bag, n)?;
        json!({ "action": "open", "note": nf.note.id })
    } else if let Some(b) = s(a, "sandbox") {
        let boards = siddran_core::bag::sandbox::list(root).map_err(|e| e.to_string())?;
        let b = pick("sandbox", b, &boards, |x| x.id().to_owned(), |x| x.title().to_owned())?;
        json!({ "action": "open", "sandbox": b.id() })
    } else if let Some(p) = s(a, "page") {
        json!({ "action": "open", "page": p, "date": s(a, "date") })
    } else {
        return Err("say what to open: note, sandbox, or page".into());
    };
    let mut req = req;
    req["at"] = json!(deadline::now_iso());
    siddran_core::presence::request(root, &req).map_err(|e| e.to_string())?;
    Ok(json!({ "requested": req }))
}

// ── notes ───────────────────────────────────────────────────────────

pub fn find_note<'a>(bag: &'a Bag, what: &str) -> Result<&'a NoteFile, String> {
    pick("note", what, &bag.notes, |n| n.note.id.clone().unwrap_or_default(), |n| n.note.title.clone())
}

fn note_brief(n: &NoteFile) -> Value {
    json!({ "id": n.note.id, "title": n.note.title, "notebook": n.notebook, "tags": n.note.tags, "updated": n.note.updated })
}

fn notes_search(root: &Path, a: &Value) -> R {
    let bag = reading(root)?;
    let q = s(a, "query").unwrap_or("").to_lowercase();
    let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 200) as usize;
    let mut hits: Vec<(u8, &NoteFile)> = bag
        .notes
        .iter()
        .filter(|n| s(a, "notebook").is_none_or(|nb| n.notebook.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(nb))))
        .filter(|n| s(a, "tag").is_none_or(|t| n.note.tags.split(',').any(|x| x.trim().eq_ignore_ascii_case(t))))
        .filter_map(|n| {
            if q.is_empty() {
                return Some((2, n));
            }
            if n.note.title.to_lowercase().contains(&q) {
                Some((0, n))
            } else if n.note.tags.to_lowercase().contains(&q) || n.note.body.to_lowercase().contains(&q) {
                Some((1, n))
            } else {
                None
            }
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.note.updated.cmp(&a.1.note.updated)));
    Ok(Value::Array(
        hits.iter()
            .take(limit)
            .map(|(_, n)| {
                let mut v = note_brief(n);
                v["snippet"] = json!(snippet(&n.note.body, &q));
                v
            })
            .collect(),
    ))
}

/// A line of context around the first match, or the opening of the note.
fn snippet(body: &str, q: &str) -> String {
    let lower = body.to_lowercase();
    let at = if q.is_empty() { 0 } else { lower.find(q).unwrap_or(0) };
    // Byte offsets from the lowercase copy can split a char in the original;
    // walk to boundaries rather than slice blindly.
    let mut start = at.saturating_sub(60).min(body.len());
    while !body.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + 120).min(body.len());
    while !body.is_char_boundary(end) {
        end += 1;
    }
    body[start..end].split_whitespace().collect::<Vec<_>>().join(" ")
}

fn note_read(root: &Path, a: &Value) -> R {
    let bag = reading(root)?;
    let n = find_note(&bag, need(a, "note")?)?;
    Ok(json!({
        "id": n.note.id, "title": n.note.title, "notebook": n.notebook, "tags": n.note.tags,
        "favorite": n.note.favorite, "updated": n.note.updated, "body": n.note.body,
    }))
}

/// The notebook's folder name, creating it (with the desktop's sidecar) if new.
/// Returns the id the next note should take, which must clear the notebook's.
fn ensure_notebook(bag: &Bag, name: &str) -> Result<(String, i64), String> {
    let folder = siddran_core::bag::layout::safe_name(name);
    let next = notes::next_note_id(bag);
    if bag.notebooks.iter().any(|(n, _)| *n == folder) {
        return Ok((folder, next));
    }
    let now = deadline::now_iso();
    let meta = json!({ "id": next, "color": null, "tags": "", "is_favorite": false, "order": null, "created_at": now, "updated_at": now });
    let path = bag.root.join("notes").join(&folder).join(".siddran-notebook.json");
    write::notebook_meta(&path, meta.as_object().unwrap()).map_err(|e| e.to_string())?;
    Ok((folder, next + 1))
}

fn tags_of(a: &Value) -> Option<String> {
    a.get("tags")?;
    Some(strings(a, "tags").iter().map(|t| t.trim().trim_start_matches('#')).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(","))
}

fn note_create(root: &Path, a: &Value) -> R {
    let title = need(a, "title")?.to_owned();
    writing(root, |bag| {
        let (notebook, id) = match s(a, "notebook") {
            Some(nb) => {
                let (f, id) = ensure_notebook(bag, nb)?;
                (Some(f), id)
            }
            None => (None, notes::next_note_id(bag)),
        };
        let id = id.to_string();
        let now = deadline::now_iso();
        let mut body = a.get("body").and_then(Value::as_str).unwrap_or("").to_owned();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        let note = siddran_core::Note {
            id: Some(id.clone()),
            title: title.clone(),
            tags: tags_of(a).unwrap_or_default(),
            favorite: false,
            color: String::new(),
            order: None,
            created: now.clone(),
            updated: now,
            body,
            had_frontmatter: true,
        };
        let path = notes::target_path(bag, Some(&id), &title, notebook.as_deref());
        if path.exists() {
            return Err(format!("{} already exists", path.display()));
        }
        notes::save(&note, &path).map_err(|e| e.to_string())?;
        Ok(json!({ "id": id, "title": title, "notebook": notebook }))
    })
}

fn note_edit(root: &Path, a: &Value) -> R {
    let what = need(a, "note")?;
    writing(root, |bag| {
        let nf = find_note(bag, what)?;
        let mut body = nf.note.body.clone();
        if let Some(b) = a.get("body").and_then(Value::as_str) {
            body = b.to_owned();
        }
        if let Some(Value::Array(reps)) = a.get("replacements") {
            for (i, r) in reps.iter().enumerate() {
                let find = r.get("find").and_then(Value::as_str).unwrap_or("");
                let repl = r.get("replace").and_then(Value::as_str).unwrap_or("");
                if find.is_empty() {
                    return Err(format!("replacement {i}: `find` is empty"));
                }
                let count = body.matches(find).count();
                let all = r.get("all").and_then(Value::as_bool).unwrap_or(false);
                match count {
                    0 => return Err(format!("replacement {i}: {find:?} is not in the note (read it again — it may have changed)")),
                    1 => body = body.replacen(find, repl, 1),
                    k if all => {
                        let _ = k;
                        body = body.replace(find, repl)
                    }
                    k => return Err(format!("replacement {i}: {find:?} occurs {k} times; include more surrounding text, or set all=true")),
                }
            }
        }
        for (key, after) in [("insert_after", true), ("insert_before", false)] {
            for (i, ins) in a.get(key).and_then(Value::as_array).into_iter().flatten().enumerate() {
                let anchor = ins.get("anchor").and_then(Value::as_str).unwrap_or("");
                let text = ins.get("text").and_then(Value::as_str).unwrap_or("");
                body = insert_at_anchor(&body, anchor, text, after).map_err(|e| format!("{key} {i}: {e}"))?;
            }
        }
        if let Some(sec) = a.get("section") {
            let heading = sec.get("heading").and_then(Value::as_str).unwrap_or("");
            let text = sec.get("body").and_then(Value::as_str).unwrap_or("");
            body = replace_section(&body, heading, text)?;
        }
        if let Some(p) = s(a, "prepend") {
            body = format!("{}\n{}", p.trim_end_matches('\n'), body);
        }
        if let Some(t) = a.get("append").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
            if !body.is_empty() && !body.ends_with('\n') {
                body.push('\n');
            }
            body.push_str(t);
            if !t.ends_with('\n') {
                body.push('\n');
            }
        }
        if body == nf.note.body {
            return Ok(json!({ "id": nf.note.id, "changed": false }));
        }
        let mut note = nf.note.clone();
        note.body = body;
        note.updated = deadline::now_iso();
        notes::save(&note, &nf.path).map_err(|e| e.to_string())?;
        Ok(json!({ "id": note.id, "changed": true, "length": note.body.len() }))
    })
}

/// Put `text` on its own line(s) right after (or before) the line holding
/// `anchor`. Whole lines, so an insert never splits a sentence in two.
fn insert_at_anchor(body: &str, anchor: &str, text: &str, after: bool) -> Result<String, String> {
    if anchor.is_empty() {
        return Err("`anchor` is empty".into());
    }
    let hits: Vec<usize> = body.match_indices(anchor).map(|(i, _)| i).collect();
    let at = match hits.as_slice() {
        [] => return Err(format!("{anchor:?} is not in the note (read it again — it may have changed)")),
        [i] => *i,
        many => return Err(format!("{anchor:?} occurs {} times; include more surrounding text", many.len())),
    };
    let cut = if after {
        let end = at + anchor.len();
        body[end..].find('\n').map(|n| end + n + 1).unwrap_or(body.len())
    } else {
        body[..at].rfind('\n').map(|n| n + 1).unwrap_or(0)
    };
    let mut out = String::with_capacity(body.len() + text.len() + 2);
    out.push_str(&body[..cut]);
    if cut > 0 && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(text.trim_end_matches('\n'));
    out.push('\n');
    out.push_str(&body[cut..]);
    Ok(out)
}

fn heading_of(line: &str) -> Option<(usize, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&level) && t[level..].starts_with(' ') {
        Some((level, t[level..].trim()))
    } else {
        None
    }
}

/// Replace what sits under a heading — up to the next heading of the same or a
/// higher level, so subsections of it go too, and the next chapter does not.
fn replace_section(body: &str, heading: &str, text: &str) -> Result<String, String> {
    let lines: Vec<&str> = body.split('\n').collect();
    let want = heading.trim().trim_start_matches('#').trim().to_lowercase();
    let starts: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| heading_of(l).is_some_and(|(_, h)| h.to_lowercase() == want)).map(|(i, _)| i).collect();
    let start = match starts.as_slice() {
        [] => return Err(format!("no heading {heading:?} in the note")),
        [i] => *i,
        many => return Err(format!("{} headings are called {heading:?}; use replacements instead", many.len())),
    };
    let level = heading_of(lines[start]).map(|(l, _)| l).unwrap_or(1);
    let end = lines.iter().enumerate().skip(start + 1).find(|(_, l)| heading_of(l).is_some_and(|(lv, _)| lv <= level)).map(|(i, _)| i).unwrap_or(lines.len());
    let mut out: Vec<String> = lines[..=start].iter().map(|s| s.to_string()).collect();
    out.push(String::new());
    out.extend(text.trim_matches('\n').split('\n').map(str::to_owned));
    if end < lines.len() {
        out.push(String::new());
    }
    out.extend(lines[end..].iter().map(|s| s.to_string()));
    Ok(out.join("\n"))
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    const DOC: &str = "# Trip\n\nIntro line.\n\n## Packing\n- socks\n- charger\n\n### Extras\n- book\n\n## Budget\n100\n";

    #[test]
    fn inserts_after_the_anchor_line() {
        let out = insert_at_anchor(DOC, "socks", "- toothbrush", true).unwrap();
        assert!(out.contains("- socks\n- toothbrush\n- charger"));
    }
    #[test]
    fn inserts_before_the_anchor_line() {
        let out = insert_at_anchor(DOC, "## Budget", "## Dates\nJune", false).unwrap();
        assert!(out.contains("## Dates\nJune\n## Budget"));
    }
    #[test]
    fn refuses_a_missing_or_repeated_anchor() {
        assert!(insert_at_anchor(DOC, "nope", "x", true).is_err());
        assert!(insert_at_anchor(DOC, "- ", "x", true).is_err());
    }
    #[test]
    fn section_replaces_down_to_the_next_peer_heading() {
        let out = replace_section(DOC, "Packing", "- passport").unwrap();
        assert!(out.contains("## Packing\n\n- passport\n\n## Budget\n100"), "{out}");
        assert!(!out.contains("Extras"), "a subsection belongs to its parent");
        assert!(out.starts_with("# Trip\n\nIntro line."));
    }
    #[test]
    fn section_of_the_last_heading_runs_to_the_end() {
        let out = replace_section(DOC, "budget", "250").unwrap();
        assert!(out.ends_with("## Budget\n\n250"), "{out}");
    }
}

fn note_update(root: &Path, a: &Value) -> R {
    let what = need(a, "note")?;
    writing(root, |bag| {
        let nf = find_note(bag, what)?;
        let mut note = nf.note.clone();
        if let Some(t) = s(a, "title") {
            note.title = t.to_owned();
        }
        if let Some(t) = tags_of(a) {
            note.tags = t;
        }
        if let Some(f) = flag(a, "favorite") {
            note.favorite = f;
        }
        let notebook = match a.get("notebook").and_then(Value::as_str) {
            Some(nb) if nb.trim().is_empty() => None,
            Some(nb) => Some(ensure_notebook(bag, nb)?.0),
            None => nf.notebook.clone(),
        };
        note.updated = deadline::now_iso();
        let to = notes::target_path(bag, note.id.as_deref(), &note.title, notebook.as_deref());
        notes::save_moving(&note, &nf.path, &to, &bag.root).map_err(|e| e.to_string())?;
        Ok(json!({ "id": note.id, "title": note.title, "notebook": notebook, "tags": note.tags, "favorite": note.favorite }))
    })
}

fn note_delete(root: &Path, a: &Value) -> R {
    let what = need(a, "note")?;
    writing(root, |bag| {
        let nf = find_note(bag, what)?;
        notes::delete(nf, &bag.root).map_err(|e| e.to_string())?;
        Ok(json!({ "trashed": nf.note.title, "recoverable_in": ".siddran-trash/" }))
    })
}

// ── tasks ───────────────────────────────────────────────────────────

fn task_json(r: &Row<Task>) -> Value {
    let t = &r.v;
    json!({
        "id": t.id.to_string(), "title": t.title, "priority": t.priority, "completed": t.is_completed,
        "due": t.due_date, "due_text": t.due_date.as_deref().map(|d| deadline::human(d, t.due_all_day)),
        "description": t.description.as_deref().filter(|d| !d.is_empty()),
    })
}

fn find_task<'a>(rows: &'a [Row<Task>], what: &str) -> Result<usize, String> {
    let hit = pick("task", what, rows, |r| r.v.id.to_string(), |r| r.v.title.clone())?;
    Ok(rows.iter().position(|r| std::ptr::eq(r, hit)).unwrap())
}

fn tasks_list(root: &Path, a: &Value) -> R {
    let bag = reading(root)?;
    let rows = bag.tasks();
    let today = deadline::today();
    let mut sel: Vec<&Row<Task>> = match s(a, "filter").unwrap_or("open") {
        "today" => query::tasks(&rows, TaskFilter::Today),
        "overdue" => query::tasks(&rows, TaskFilter::Overdue),
        "undated" => query::tasks(&rows, TaskFilter::Undated),
        "all" => query::tasks(&rows, TaskFilter::All),
        "completed" => rows.iter().filter(|r| r.v.is_completed).collect(),
        "upcoming" => rows
            .iter()
            .filter(|r| !r.v.is_completed && r.v.due_date.as_deref().and_then(deadline::due_day).is_some_and(|d| d > today))
            .collect(),
        _ => query::tasks(&rows, TaskFilter::Open),
    };
    if let Some(q) = s(a, "query") {
        let q = q.to_lowercase();
        sel.retain(|r| r.v.title.to_lowercase().contains(&q));
    }
    sel.sort_by(|x, y| x.v.due_date.is_none().cmp(&y.v.due_date.is_none()).then(x.v.due_date.cmp(&y.v.due_date)));
    Ok(Value::Array(sel.into_iter().map(task_json).collect()))
}

fn prio(a: &Value) -> Result<Option<&str>, String> {
    match s(a, "priority") {
        None => Ok(None),
        Some(p @ ("low" | "normal" | "high")) => Ok(Some(p)),
        Some(p) => Err(format!("priority {p:?} — use low, normal or high")),
    }
}

fn task_add(root: &Path, a: &Value) -> R {
    let title = need(a, "title")?.to_owned();
    let priority = prio(a)?.unwrap_or("normal").to_owned();
    let due = s(a, "due").map(when).transpose()?;
    writing(root, |bag| {
        let mut rows = bag.tasks();
        let id = siddran_core::ids::uuid_v4();
        let mut row = Row::<Task>::new_task(&id, &title, &priority);
        if let Some((iso, all_day)) = &due {
            row.set_due(Some(iso), *all_day);
        }
        if let Some(d) = s(a, "description") {
            row.v.description = Some(d.to_owned());
            row.set("description", d);
        }
        let out = task_json(&row);
        rows.push(row);
        save_tasks(bag, &rows, &bag.dailies(), &bag.completions(), &bag.projects())?;
        Ok(out)
    })
}

fn task_update(root: &Path, a: &Value) -> R {
    let what = need(a, "task")?;
    let priority = prio(a)?;
    writing(root, |bag| {
        let mut rows = bag.tasks();
        let i = find_task(&rows, what)?;
        let r = &mut rows[i];
        if let Some(t) = s(a, "title") {
            r.set_title(t);
        }
        if let Some(p) = priority {
            r.set_priority(p);
        }
        match a.get("due").and_then(Value::as_str) {
            Some(d) if d.trim().is_empty() => r.set_due(None, false),
            Some(d) => {
                let (iso, all_day) = when(d)?;
                r.set_due(Some(&iso), all_day);
            }
            None => {}
        }
        if let Some(d) = a.get("description").and_then(Value::as_str) {
            r.v.description = Some(d.to_owned());
            r.set("description", d);
            r.touch();
        }
        if let Some(done) = flag(a, "completed") {
            r.set_completed(done);
        }
        let out = task_json(r);
        save_tasks(bag, &rows, &bag.dailies(), &bag.completions(), &bag.projects())?;
        Ok(out)
    })
}

fn task_delete(root: &Path, a: &Value) -> R {
    let what = need(a, "task")?;
    writing(root, |bag| {
        let mut rows = bag.tasks();
        let i = find_task(&rows, what)?;
        let gone = rows.remove(i);
        save_tasks(bag, &rows, &bag.dailies(), &bag.completions(), &bag.projects())?;
        Ok(json!({ "deleted": gone.v.title }))
    })
}

// ── dailies & projects ──────────────────────────────────────────────

fn dailies_list(root: &Path) -> R {
    let bag = reading(root)?;
    let rows = bag.dailies();
    let completions = bag.completions();
    let today = deadline::today().to_string();
    Ok(Value::Array(
        query::active_dailies(&rows)
            .into_iter()
            .map(|r| {
                let d = &r.v;
                let rule = recurrence::parse(d.recurrence.as_deref());
                json!({
                    "id": d.id.to_string(), "title": d.title, "time": d.time,
                    "repeats": rule.as_ref().map(recurrence::label),
                    "done_today": if rule.is_some() { query::completed_on(&completions, &d.id, &today) } else { d.is_completed },
                })
            })
            .collect(),
    ))
}

/// `every-day`, `weekdays`, `weekends`, or a day list → the stored string.
fn repeat_rule(v: &Value) -> Result<Option<String>, String> {
    const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
    match v {
        Value::Null => Ok(None),
        Value::String(s) => match s.trim().to_lowercase().as_str() {
            "" | "none" | "once" => Ok(None),
            "every-day" | "daily" | "every day" | "everyday" => Ok(Some("every-day".into())),
            "weekdays" => Ok(Some("weekdays".into())),
            "weekends" => Ok(Some("weekends".into())),
            other => repeat_rule(&Value::Array(other.split([',', ' ']).filter(|x| !x.is_empty()).map(|x| json!(x)).collect())),
        },
        Value::Array(list) => {
            let mut mask = [false; 7];
            for d in list {
                let d = d.as_str().unwrap_or("").trim().to_lowercase();
                let i = DAYS.iter().position(|x| d.starts_with(x)).ok_or_else(|| format!("{d:?} is not a day — use mon, tue, …"))?;
                mask[i] = true;
            }
            Ok(Some(json!({ "mask": mask }).to_string()))
        }
        _ => Err("repeat must be a string or a list of days".into()),
    }
}

fn daily_add(root: &Path, a: &Value) -> R {
    let title = need(a, "title")?.to_owned();
    let rule = repeat_rule(a.get("repeat").unwrap_or(&Value::Null))?;
    writing(root, |bag| {
        let mut rows = bag.dailies();
        let id = siddran_core::ids::uuid_v4();
        let mut row = Row::<Daily>::new_daily(&id, &title, rule.as_deref());
        if rule.is_some() {
            // A habit never expires (localStore.js: expires_at null when recurring).
            row.set("expires_at", Value::Null);
        }
        if let Some(t) = s(a, "time") {
            row.set("time", t);
        }
        rows.push(row);
        save_tasks(bag, &bag.tasks(), &rows, &bag.completions(), &bag.projects())?;
        Ok(json!({ "id": id, "title": title, "repeat": rule }))
    })
}

fn daily_check(root: &Path, a: &Value) -> R {
    let what = need(a, "daily")?;
    let done = flag(a, "done").unwrap_or(true);
    let date = s(a, "date").map(str::to_owned).unwrap_or_else(|| deadline::today().to_string());
    writing(root, |bag| {
        let mut dailies = bag.dailies();
        let hit = pick("daily", what, &dailies, |r| r.v.id.to_string(), |r| r.v.title.clone())?;
        let i = dailies.iter().position(|r| std::ptr::eq(r, hit)).unwrap();
        let mut completions = bag.completions();
        if dailies[i].v.recurrence.is_none() {
            dailies[i].set_completed(done);
        } else {
            let id = dailies[i].v.id.clone();
            completions.retain(|c| !(c.v.daily_task_id == id && c.v.date == date));
            if done {
                completions.push(Row::from_value(&json!({ "daily_task_id": id, "date": date })).expect("literal"));
            }
        }
        let title = dailies[i].v.title.clone();
        save_tasks(bag, &bag.tasks(), &dailies, &completions, &bag.projects())?;
        Ok(json!({ "daily": title, "date": date, "done": done }))
    })
}

fn project_json(r: &Row<Project>) -> Value {
    let p = &r.v;
    let (done, total) = priority::progress(&p.tasks);
    json!({
        "id": p.id.to_string(), "title": p.title, "completed": p.is_completed,
        "priority": priority::bucket(&p.tasks), "progress": format!("{done}/{total}"),
        "subtasks": p.tasks.iter().map(|t| json!({ "id": t.id.to_string(), "title": t.title, "done": t.is_completed })).collect::<Vec<_>>(),
    })
}

fn projects_list(root: &Path) -> R {
    let bag = reading(root)?;
    Ok(Value::Array(bag.projects().iter().map(project_json).collect()))
}

fn project_add(root: &Path, a: &Value) -> R {
    let title = need(a, "title")?.to_owned();
    writing(root, |bag| {
        let mut rows = bag.projects();
        let mut p = Row::<Project>::new_project(&siddran_core::ids::uuid_v4(), &title);
        for st in strings(a, "subtasks") {
            p.add_subtask(&siddran_core::ids::uuid_v4(), &st);
        }
        let out = project_json(&p);
        rows.push(p);
        save_tasks(bag, &bag.tasks(), &bag.dailies(), &bag.completions(), &rows)?;
        Ok(out)
    })
}

fn project_update(root: &Path, a: &Value) -> R {
    let what = need(a, "project")?;
    writing(root, |bag| {
        let mut rows = bag.projects();
        let hit = pick("project", what, &rows, |r| r.v.id.to_string(), |r| r.v.title.clone())?;
        let i = rows.iter().position(|r| std::ptr::eq(r, hit)).unwrap();
        let p = &mut rows[i];
        if let Some(t) = s(a, "title") {
            p.v.title = t.to_owned();
            p.set("title", t);
            p.touch();
        }
        for st in strings(a, "add") {
            p.add_subtask(&siddran_core::ids::uuid_v4(), &st);
        }
        let sub = |p: &Row<Project>, w: &str| -> Result<siddran_core::Id, String> {
            pick("subtask", w, &p.v.tasks, |t| t.id.to_string(), |t| t.title.clone()).map(|t| t.id.clone())
        };
        for w in strings(a, "complete") {
            let id = sub(p, &w)?;
            p.set_subtask_completed(&id, true);
        }
        for w in strings(a, "reopen") {
            let id = sub(p, &w)?;
            p.set_subtask_completed(&id, false);
        }
        for w in strings(a, "remove") {
            let id = sub(p, &w)?;
            p.remove_subtask(&id);
        }
        if let Some(c) = flag(a, "completed") {
            p.v.is_completed = c;
            p.set("is_completed", c);
            p.touch();
        }
        let out = project_json(p);
        save_tasks(bag, &bag.tasks(), &bag.dailies(), &bag.completions(), &rows)?;
        Ok(out)
    })
}

// ── calendar ────────────────────────────────────────────────────────

fn item_json(it: &calendar::Item) -> Value {
    json!({
        "kind": format!("{:?}", it.kind).to_lowercase(), "id": it.id, "title": it.title,
        "time": it.time, "end": it.end, "done": it.done,
        "description": it.description,
    })
}

fn agenda(root: &Path, a: &Value) -> R {
    let bag = reading(root)?;
    let from = match s(a, "from") {
        Some(d) => d.parse::<siddran_core::jiff::civil::Date>().map_err(|_| format!("from: {d:?} is not YYYY-MM-DD"))?,
        None => deadline::today(),
    };
    let days = a.get("days").and_then(Value::as_i64).unwrap_or(7).clamp(1, 62) as i32;
    let (events, tasks, dailies, completions) = (bag.events(), bag.tasks(), bag.dailies(), bag.completions());
    let out: Vec<Value> = (0..days)
        .map(|i| {
            let d = calendar::shift_day(from, i);
            json!({ "date": d.to_string(), "weekday": format!("{:?}", d.weekday()), "items": calendar::day_items(d, &events, &tasks, &dailies, &completions).iter().map(item_json).collect::<Vec<_>>() })
        })
        .collect();
    Ok(Value::Array(out))
}

/// start/end strings → (start_iso, end_iso, all_day). A timed event with no end
/// gets an hour, so it draws as a block rather than a sliver.
fn span(start: &str, end: Option<&str>) -> Result<(String, Option<String>, bool), String> {
    let (s_iso, all_day) = when(start)?;
    let e_iso = match end {
        Some(e) => Some(when(e)?.0),
        None if all_day => None,
        None => {
            let t: siddran_core::jiff::Timestamp = s_iso.parse().map_err(|_| "bad start".to_string())?;
            let t = t.checked_add(siddran_core::jiff::SignedDuration::from_hours(1)).map_err(|e| e.to_string())?;
            Some(deadline::iso_ms(t))
        }
    };
    Ok((s_iso, e_iso, all_day))
}

fn event_out(r: &Row<Event>) -> Value {
    json!({ "id": r.v.id.to_string(), "title": r.v.title, "start": r.v.start_at, "end": r.v.end_at, "all_day": r.v.all_day })
}

fn event_add(root: &Path, a: &Value) -> R {
    let title = need(a, "title")?.to_owned();
    let (start, end, all_day) = span(need(a, "start")?, s(a, "end"))?;
    writing(root, |bag| {
        let mut events = bag.events();
        let now = deadline::now_iso();
        // Key order of the desktop's POST /events (localStore.js handleEvents).
        let raw = json!({
            "id": siddran_core::ids::uuid_v4(), "title": title, "description": s(a, "description"),
            "start_at": start, "end_at": end, "all_day": all_day, "color": s(a, "color"),
            "ref_type": null, "ref_id": null, "schedule_id": null, "created_at": now, "updated_at": now,
        });
        let row: Row<Event> = Row::from_value(&raw).ok_or("could not build the event")?;
        let out = event_out(&row);
        events.push(row);
        write::calendar_file(&bag.root.join("calendar.siddran"), &events, &bag.schedules()).map_err(|e| e.to_string())?;
        Ok(out)
    })
}

fn find_event(rows: &[Row<Event>], what: &str) -> Result<usize, String> {
    let hit = pick("event", what, rows, |r| r.v.id.to_string(), |r| r.v.title.clone())?;
    Ok(rows.iter().position(|r| std::ptr::eq(r, hit)).unwrap())
}

fn event_update(root: &Path, a: &Value) -> R {
    let what = need(a, "event")?;
    writing(root, |bag| {
        let mut events = bag.events();
        let i = find_event(&events, what)?;
        let r = &mut events[i];
        if let Some(t) = s(a, "title") {
            r.set("title", t);
        }
        if let Some(d) = a.get("description").and_then(Value::as_str) {
            r.set("description", d);
        }
        if let Some(c) = s(a, "color") {
            r.set("color", c);
        }
        if let Some(st) = s(a, "start") {
            let (si, ei, all_day) = span(st, s(a, "end"))?;
            r.set("start_at", &si);
            r.set("end_at", &ei);
            r.set("all_day", all_day);
        } else if let Some(e) = s(a, "end") {
            r.set("end_at", when(e)?.0);
        }
        r.touch();
        let fresh: Row<Event> = Row::from_value(&Value::Object(r.raw.clone())).ok_or("event no longer parses")?;
        *r = fresh;
        let out = event_out(r);
        write::calendar_file(&bag.root.join("calendar.siddran"), &events, &bag.schedules()).map_err(|e| e.to_string())?;
        Ok(out)
    })
}

fn event_delete(root: &Path, a: &Value) -> R {
    let what = need(a, "event")?;
    writing(root, |bag| {
        let mut events = bag.events();
        let i = find_event(&events, what)?;
        let gone = events.remove(i);
        write::calendar_file(&bag.root.join("calendar.siddran"), &events, &bag.schedules()).map_err(|e| e.to_string())?;
        Ok(json!({ "deleted": gone.v.title }))
    })
}
