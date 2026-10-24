//! Writing the collection files.
//!
//! `tasks.siddran`, `calendar.siddran` and `settings.siddran` are each one JSON
//! document, so any change rewrites the whole file — which is exactly why the
//! row ordering below matters, and why rows keep their raw backing (a row we did
//! not touch must come back out byte-identical).

use crate::error::Result;
use crate::json;
use crate::row::Row;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::path::Path;

use super::atomic;

/// `byId` (localStore.js:215) — lexicographic on the **string** form of the id,
/// so "1000" sorts before "999". Sorting numerically would rewrite the file.
/// Stable ordering is what keeps an edit to one row a one-line diff.
fn by_id(mut rows: Vec<Map<String, Value>>) -> Vec<Map<String, Value>> {
    rows.sort_by_key(|r| id_string(r.get("id")));
    rows
}

/// `byCompletion` (localStore.js:217-218) — completions have no id of their own.
fn by_completion(mut rows: Vec<Map<String, Value>>) -> Vec<Map<String, Value>> {
    rows.sort_by_key(|r| (id_string(r.get("daily_task_id")), id_string(r.get("date"))));
    rows
}

fn id_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn raws<T: DeserializeOwned>(rows: &[Row<T>]) -> Vec<Map<String, Value>> {
    rows.iter().map(|r| r.raw.clone()).collect()
}

fn arr(rows: Vec<Map<String, Value>>) -> Value {
    Value::Array(rows.into_iter().map(Value::Object).collect())
}

/// Rebuild `tasks.siddran` from the four collections, in the desktop's key order.
pub fn tasks_file<A, B, C, D>(
    path: &Path,
    tasks: &[Row<A>],
    dailies: &[Row<B>],
    completions: &[Row<C>],
    projects: &[Row<D>],
) -> Result<()>
where
    A: DeserializeOwned,
    B: DeserializeOwned,
    C: DeserializeOwned,
    D: DeserializeOwned,
{
    let mut root = Map::new();
    root.insert("tasks".into(), arr(by_id(raws(tasks))));
    root.insert("dailies".into(), arr(by_id(raws(dailies))));
    root.insert("completions".into(), arr(by_completion(raws(completions))));
    // Project subtasks are deliberately NOT sorted — they are append-only and
    // already diff-stable, and the desktop leaves them in insertion order.
    root.insert("projects".into(), arr(by_id(raws(projects))));
    atomic::write(path, json::to_string(&Value::Object(root)).as_bytes())
}

pub fn calendar_file<A, B>(path: &Path, events: &[Row<A>], schedules: &[Row<B>]) -> Result<()>
where
    A: DeserializeOwned,
    B: DeserializeOwned,
{
    let mut root = Map::new();
    root.insert("events".into(), arr(by_id(raws(events))));
    root.insert("schedules".into(), arr(by_id(raws(schedules))));
    atomic::write(path, json::to_string(&Value::Object(root)).as_bytes())
}

pub fn settings_file(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    let mut root = Map::new();
    root.insert("settings".into(), Value::Object(settings.clone()));
    atomic::write(path, json::to_string(&Value::Object(root)).as_bytes())
}

/// The notebook sidecar. Note `order` is written as `null` when absent — unlike
/// a note's frontmatter, where the key is omitted entirely (localStore.js:164).
pub fn notebook_meta(path: &Path, meta: &Map<String, Value>) -> Result<()> {
    const KEYS: [&str; 7] = [
        "id",
        "color",
        "tags",
        "is_favorite",
        "order",
        "created_at",
        "updated_at",
    ];
    let mut out = Map::new();
    for k in KEYS {
        out.insert(k.into(), meta.get(k).cloned().unwrap_or(Value::Null));
    }
    atomic::write(path, json::to_string(&Value::Object(out)).as_bytes())
}
