use crate::ids::Id;
use serde::Deserialize;

/// Note every `Option` here: `#[serde(default)]` makes an *absent* key `None`,
/// and an explicit `null` also deserializes to `None`. That conflation is
/// correct for reading — `deadline.js:58` tests `allDay == null`, which is true
/// for both — but it is why writing must go through `Row::raw`, not this struct.
#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: Id,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "normal")]
    pub priority: String,
    #[serde(default)]
    pub due_date: Option<String>,
    #[serde(default)]
    pub due_all_day: Option<bool>,
    #[serde(default)]
    pub remind_at: Option<String>,
    #[serde(default)]
    pub is_completed: bool,
    #[serde(default)]
    pub order: Option<i64>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Daily {
    pub id: Id,
    #[serde(default)]
    pub title: String,
    #[serde(default = "normal")]
    pub priority: String,
    #[serde(default)]
    pub is_completed: bool,
    #[serde(default)]
    pub expires_at: Option<String>,
    /// `'every-day' | 'weekdays' | 'weekends' | '{"mask":[7 bools]}'`, or absent
    /// for an ephemeral daily that expires at end of day.
    #[serde(default)]
    pub recurrence: Option<String>,
    /// `'HH:MM'`
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// Completions have no id of their own — `(daily_task_id, date)` is the
/// identity (localStore.js:216-218).
#[derive(Debug, Clone, Deserialize)]
pub struct Completion {
    pub daily_task_id: Id,
    pub date: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    pub id: Id,
    #[serde(default)]
    pub title: String,
    /// Stored, but *derived* from the subtasks — always prefer
    /// `logic::priority::bucket` over trusting this field.
    #[serde(default = "normal")]
    pub priority: String,
    #[serde(default)]
    pub is_completed: bool,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub tasks: Vec<ProjectTask>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectTask {
    pub id: Id,
    #[serde(default)]
    pub title: String,
    #[serde(default = "normal")]
    pub priority: String,
    #[serde(default)]
    pub is_completed: bool,
}

fn normal() -> String {
    "normal".to_owned()
}

use crate::row::Row;
use serde_json::{json, Map, Value};

impl Row<Task> {
    pub fn set_completed(&mut self, done: bool) {
        self.v.is_completed = done;
        self.set("is_completed", done);
        self.touch();
    }

    pub fn set_title(&mut self, title: &str) {
        self.v.title = title.to_owned();
        self.set("title", title);
        self.touch();
    }

    pub fn set_priority(&mut self, p: &str) {
        self.v.priority = p.to_owned();
        self.set("priority", p);
        self.touch();
    }

    /// Setting a deadline always writes *both* keys, because on a row that never
    /// had `due_all_day` the flag's absence is itself meaningful — leaving it
    /// absent while changing the date would hand the desktop a legacy-looking
    /// row with a modern date.
    pub fn set_due(&mut self, iso: Option<&str>, all_day: bool) {
        self.v.due_date = iso.map(str::to_owned);
        self.v.due_all_day = Some(all_day);
        self.set("due_date", iso);
        self.set("due_all_day", all_day);
        self.touch();
    }

    /// A new task, with the desktop's key order and defaults
    /// (localStore.js:503). Key order is copied from the JS object literal so a
    /// row we create is indistinguishable from one the desktop created.
    pub fn new_task(id: &str, title: &str, priority: &str) -> Row<Task> {
        let now = crate::logic::deadline::now_iso();
        let raw: Map<String, Value> = json!({
            "id": id,
            "title": title,
            "description": "",
            "priority": priority,
            "due_date": Value::Null,
            "due_all_day": false,
            "remind_at": Value::Null,
            "is_completed": false,
            "order": Value::Null,
            "created_at": now,
            "updated_at": now,
        })
        .as_object()
        .cloned()
        .expect("literal is an object");
        Row::from_value(&Value::Object(raw)).expect("template deserializes")
    }
}

impl Row<Daily> {
    /// A new daily, in the desktop's key order (localStore.js:529). A daily with
    /// no recurrence is ephemeral and expires at the end of the local day.
    pub fn new_daily(id: &str, title: &str, recurrence: Option<&str>) -> Row<Daily> {
        let now = crate::logic::deadline::now_iso();
        let expires = crate::logic::deadline::end_of_today();
        let raw = json!({
            "id": id,
            "title": title,
            "priority": "normal",
            "is_completed": false,
            "created_at": now,
            "updated_at": now,
            "expires_at": expires,
            "recurrence": recurrence,
            "time": Value::Null,
        });
        Row::from_value(&raw).expect("template deserializes")
    }

    pub fn set_completed(&mut self, done: bool) {
        self.v.is_completed = done;
        self.set("is_completed", done);
        self.touch();
    }
}

impl Row<Project> {
    /// A new project (localStore.js:551). `tasks` starts empty, which makes the
    /// derived priority `normal` — the documented behaviour for no open subtasks.
    pub fn new_project(id: &str, title: &str) -> Row<Project> {
        let now = crate::logic::deadline::now_iso();
        let raw = json!({
            "id": id,
            "title": title,
            "priority": "normal",
            "is_completed": false,
            "color": Value::Null,
            "created_at": now,
            "updated_at": now,
            "tasks": [],
        });
        Row::from_value(&raw).expect("template deserializes")
    }

    /// Append a subtask, keeping insertion order — the desktop leaves
    /// `p.tasks` unsorted on purpose, since append-only is already diff-stable.
    pub fn add_subtask(&mut self, id: &str, title: &str) {
        let now = crate::logic::deadline::now_iso();
        let sub = json!({
            "id": id,
            "project_id": self.v.id,
            "title": title,
            "priority": "normal",
            "is_completed": false,
            "created_at": now,
            "updated_at": now,
        });
        if let Some(Value::Array(arr)) = self.raw.get_mut("tasks") {
            arr.push(sub.clone());
        }
        if let Ok(st) = serde_json::from_value(sub) {
            self.v.tasks.push(st);
        }
        self.touch();
    }

    pub fn remove_subtask(&mut self, id: &crate::ids::Id) -> bool {
        let Some(i) = self.v.tasks.iter().position(|t| &t.id == id) else {
            return false;
        };
        self.v.tasks.remove(i);
        if let Some(Value::Array(arr)) = self.raw.get_mut("tasks") {
            if i < arr.len() {
                arr.remove(i);
            }
        }
        let bucket = crate::logic::priority::bucket(&self.v.tasks);
        self.v.priority = bucket.to_owned();
        self.set("priority", bucket);
        self.touch();
        true
    }

    /// Toggle one subtask. The project's stored `priority` is recomputed to
    /// match, since the desktop derives it on write.
    pub fn set_subtask_completed(&mut self, subtask: &crate::ids::Id, done: bool) -> bool {
        let Some(idx) = self.v.tasks.iter().position(|t| &t.id == subtask) else {
            return false;
        };
        self.v.tasks[idx].is_completed = done;

        if let Some(Value::Array(arr)) = self.raw.get_mut("tasks") {
            if let Some(Value::Object(o)) = arr.get_mut(idx) {
                o.insert("is_completed".into(), json!(done));
                o.insert("updated_at".into(), json!(crate::logic::deadline::now_iso()));
            }
        }
        let bucket = crate::logic::priority::bucket(&self.v.tasks);
        self.v.priority = bucket.to_owned();
        self.set("priority", bucket);
        self.touch();
        true
    }
}
