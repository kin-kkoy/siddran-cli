//! The filters the desktop exposes as query params (localStore.js:491-499),
//! reproduced so the CLI and the TUI ask the same questions of the data.

use crate::logic::deadline;
use crate::model::{Daily, Task};
use crate::row::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskFilter {
    /// Everything, completed included.
    All,
    /// Incomplete and due today.
    Today,
    /// Incomplete with a due date in the past.
    Overdue,
    /// Has a due date.
    Dated,
    /// Incomplete with no due date.
    Undated,
    /// Incomplete — the default view.
    Open,
}

pub fn tasks(rows: &[Row<Task>], f: TaskFilter) -> Vec<&Row<Task>> {
    rows.iter()
        .filter(|r| {
            let t = &r.v;
            match f {
                TaskFilter::All => true,
                TaskFilter::Open => !t.is_completed,
                TaskFilter::Dated => t.due_date.is_some(),
                TaskFilter::Undated => t.due_date.is_none() && !t.is_completed,
                TaskFilter::Today => {
                    !t.is_completed && t.due_date.as_deref().is_some_and(deadline::is_today)
                }
                TaskFilter::Overdue => t
                    .due_date
                    .as_deref()
                    .is_some_and(|d| deadline::is_overdue(d, t.is_completed)),
            }
        })
        .collect()
}

/// A daily with a `recurrence` never expires; one without is ephemeral and the
/// desktop purges it 24h after creation. We only *display*, so an expired
/// ephemeral daily is hidden rather than deleted.
pub fn active_dailies(rows: &[Row<Daily>]) -> Vec<&Row<Daily>> {
    let now = jiff::Timestamp::now();
    rows.iter()
        .filter(|r| {
            if r.v.recurrence.is_some() {
                return true;
            }
            match r.v.expires_at.as_deref().and_then(|e| e.parse().ok()) {
                Some(exp) => now < exp,
                None => true,
            }
        })
        .collect()
}

// ── task-view buckets ───────────────────────────────────────────────

/// The left column of the tasks view.
///
/// Ordered by how often you actually reach for them, not alphabetically or by
/// data type: what is due now, then the work you are carrying, then the things
/// that have slipped or are coming, then the whole list, then habits — and
/// `Completed` last, because it is a record rather than a queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Today,
    Projects,
    Overdue,
    /// Future deadlines *and* undated work. An undated task is something you
    /// intend to do but have not dated — which is "upcoming", not a category of
    /// its own; splitting them just meant two lists to check instead of one.
    Upcoming,
    /// Every standalone task, completed included. Not dailies, not project
    /// subtasks — those are separate things that happen to also be tickable.
    All,
    Dailies,
    Completed,
}

impl Bucket {
    pub const ALL: [Bucket; 7] = [
        Bucket::Today,
        Bucket::Projects,
        Bucket::Overdue,
        Bucket::Upcoming,
        Bucket::All,
        Bucket::Dailies,
        Bucket::Completed,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Bucket::Today => "Today",
            Bucket::Projects => "Projects",
            Bucket::Overdue => "Overdue",
            Bucket::Upcoming => "Upcoming",
            Bucket::All => "All tasks",
            Bucket::Dailies => "Dailies",
            Bucket::Completed => "Completed",
        }
    }

    /// Whether this bucket lists tasks (as opposed to dailies or projects).
    pub fn is_tasks(self) -> bool {
        !matches!(self, Bucket::Dailies | Bucket::Projects)
    }
}

/// Tasks in a bucket, in the order they should be shown.
///
/// `Today` deliberately includes overdue work: a deadline that slipped is still
/// something to do today, and a "today" list that quietly omits it is how things
/// get missed. `Overdue` exists separately for when you want only the slippage.
pub fn bucket_tasks(rows: &[Row<Task>], b: Bucket) -> Vec<&Row<Task>> {
    let mut out: Vec<&Row<Task>> = rows
        .iter()
        .filter(|r| {
            let t = &r.v;
            match b {
                Bucket::Today => {
                    !t.is_completed
                        && t.due_date.as_deref().is_some_and(|d| {
                            deadline::is_today(d) || deadline::is_overdue(d, false)
                        })
                }
                Bucket::Overdue => t
                    .due_date
                    .as_deref()
                    .is_some_and(|d| deadline::is_overdue(d, t.is_completed)),
                Bucket::Upcoming => {
                    !t.is_completed
                        && match t.due_date.as_deref() {
                            None => true,
                            Some(d) => deadline::due_day(d)
                                .is_some_and(|day| day > deadline::today()),
                        }
                }
                Bucket::All => true,
                Bucket::Completed => t.is_completed,
                Bucket::Projects | Bucket::Dailies => false,
            }
        })
        .collect();

    match b {
        // Dated buckets read best in deadline order.
        Bucket::Today | Bucket::Overdue => {
            out.sort_by(|a, c| a.v.due_date.cmp(&c.v.due_date));
        }
        // Upcoming mixes dated and undated: the dated ones first, soonest at the
        // top, then the undated tail — a date is a commitment and outranks an
        // intention.
        Bucket::Upcoming => out.sort_by(|a, c| match (&a.v.due_date, &c.v.due_date) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }),
        // `All` puts open work above the finished, then newest-updated first.
        Bucket::All => out.sort_by(|a, c| {
            a.v.is_completed
                .cmp(&c.v.is_completed)
                .then(c.v.updated_at.cmp(&a.v.updated_at))
        }),
        _ => {}
    }
    out
}

/// Was this daily ticked off on `date` (`YYYY-MM-DD`)?
pub fn completed_on(completions: &[Row<Completion>], daily: &crate::Id, date: &str) -> bool {
    completions
        .iter()
        .any(|c| &c.v.daily_task_id == daily && c.v.date == date)
}

/// The last `days` days, oldest first, as `(date, done, fires)`.
///
/// `fires` matters: a weekday-only habit should not look like a miss on Sunday,
/// so the caller can render "not due" differently from "not done".
pub fn daily_history(
    completions: &[Row<Completion>],
    daily: &Row<Daily>,
    days: usize,
) -> Vec<(String, bool, bool)> {
    let rule = crate::logic::recurrence::parse(daily.v.recurrence.as_deref());
    let today = deadline::today();
    let mut out = Vec::with_capacity(days);

    for back in (0..days).rev() {
        let Some(day) = today.checked_sub(jiff::Span::new().days(back as i64)).ok() else {
            continue;
        };
        let iso = day.to_string();
        let fires = rule.as_ref().is_some_and(|r| crate::logic::recurrence::matches(r, day));
        let done = completed_on(completions, &daily.v.id, &iso);
        out.push((iso, done, fires));
    }
    out
}

use crate::model::Completion;
