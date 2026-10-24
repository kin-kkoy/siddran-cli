//! Port of `bucketPriority` (localStore.js:367-373).
//!
//! A project's own `priority` field is stored, but it is *derived* from its
//! incomplete subtasks. Always compute it rather than trusting the stored value,
//! which can be stale if a subtask changed elsewhere.

use crate::model::ProjectTask;

fn weight(p: &str) -> f64 {
    match p {
        "low" => 1.0,
        "high" => 3.0,
        _ => 2.0, // `PRIO_W[t.priority] || 2` — anything unrecognised is normal
    }
}

/// One of `very_low quite_low low normal high quite_high very_high`.
pub fn bucket(tasks: &[ProjectTask]) -> &'static str {
    let active: Vec<&ProjectTask> = tasks.iter().filter(|t| !t.is_completed).collect();
    if active.is_empty() {
        return "normal";
    }
    let avg: f64 = active.iter().map(|t| weight(&t.priority)).sum::<f64>() / active.len() as f64;
    if avg <= 1.2 {
        "very_low"
    } else if avg <= 1.5 {
        "quite_low"
    } else if avg <= 1.8 {
        "low"
    } else if avg <= 2.2 {
        "normal"
    } else if avg <= 2.5 {
        "high"
    } else if avg <= 2.8 {
        "quite_high"
    } else {
        "very_high"
    }
}

/// `(done, total)` over all subtasks.
pub fn progress(tasks: &[ProjectTask]) -> (usize, usize) {
    (tasks.iter().filter(|t| t.is_completed).count(), tasks.len())
}
