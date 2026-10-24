//! The calendar is a **view** over data that already exists — events, dated
//! tasks, and recurring dailies projected onto days. It is never a second copy,
//! which is why nothing here stores anything.
//!
//! Ports `calendarDates.js` (the grid) and the projection in `useCalendar.js`.
//! All of it is local-time: the Bag stores UTC, but a day is a local day, and
//! using UTC would shift every event across the midnight boundary.

use crate::logic::{deadline, recurrence};
use crate::model::{Completion, Daily, Event, Task};
use crate::row::Row;
use jiff::civil::{date, Date, Weekday};

pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];

/// Monday of the week containing the 1st — the first cell of the grid.
pub fn month_grid_start(year: i16, month: i8) -> Date {
    let first = date(year, month, 1);
    // JS: `(first.getDay() + 6) % 7` days back to Monday.
    let back = match first.weekday() {
        Weekday::Monday => 0,
        Weekday::Tuesday => 1,
        Weekday::Wednesday => 2,
        Weekday::Thursday => 3,
        Weekday::Friday => 4,
        Weekday::Saturday => 5,
        Weekday::Sunday => 6,
    };
    first
        .checked_sub(jiff::Span::new().days(back))
        .unwrap_or(first)
}

/// The 42 dates (six weeks) of a Monday-start month grid. Always 42, so the
/// grid never changes height as you page through months.
pub fn month_grid(year: i16, month: i8) -> Vec<Date> {
    let start = month_grid_start(year, month);
    (0..42)
        .filter_map(|i| start.checked_add(jiff::Span::new().days(i)).ok())
        .collect()
}

/// What sort of thing is on a day. Kept separate from the row so the caller can
/// colour and sort by source without re-deriving it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Event,
    Task,
    Daily,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub id: String,
    pub title: String,
    /// Events and tasks both carry one; it is the single most useful thing the
    /// detail pane can show, and was previously read from disk and discarded.
    pub description: Option<String>,
    /// Set when an event was stamped by a schedule rather than placed by hand.
    pub schedule_id: Option<String>,
    /// `None` for an all-day item, which is what sorts it to the top.
    pub time: Option<String>,
    pub end: Option<String>,
    pub done: bool,
    pub color: Option<String>,
    pub ref_type: Option<String>,
}

/// Everything happening on one local day, all-day items first, then by time.
pub fn day_items(
    day: Date,
    events: &[Row<Event>],
    tasks: &[Row<Task>],
    dailies: &[Row<Daily>],
    completions: &[Row<Completion>],
) -> Vec<Item> {
    let iso = day.to_string();
    let mut out: Vec<Item> = Vec::new();

    // 1) Events — placed on every local day they span, inclusive.
    for e in events {
        let Some(start) = deadline::due_day(&e.v.start_at) else { continue };
        let end = e
            .v
            .end_at
            .as_deref()
            .and_then(deadline::due_day)
            .unwrap_or(start);
        if day < start || day > end {
            continue;
        }
        out.push(Item {
            kind: Kind::Event,
            id: e.v.id.to_string(),
            title: e.v.title.clone(),
            description: e.v.description.clone().filter(|d| !d.is_empty()),
            schedule_id: e.v.schedule_id.as_ref().map(|s| s.to_string()),
            time: (!e.v.all_day).then(|| hhmm(&e.v.start_at)).flatten(),
            end: e.v.end_at.as_deref().and_then(hhmm),
            done: false,
            color: e.v.color.clone(),
            ref_type: e.v.ref_type.clone(),
        });
    }

    // 2) Tasks — on their due date's local day.
    for t in tasks {
        let Some(due) = t.v.due_date.as_deref() else { continue };
        if deadline::due_day(due) != Some(day) {
            continue;
        }
        // `due_all_day` is authoritative where it exists; local midnight is the
        // older signal, kept for rows written before the flag existed.
        let all_day = deadline::is_all_day(due, t.v.due_all_day);
        out.push(Item {
            kind: Kind::Task,
            id: t.v.id.to_string(),
            title: t.v.title.clone(),
            description: t.v.description.clone().filter(|d| !d.is_empty()),
            schedule_id: None,
            time: (!all_day).then(|| hhmm(due)).flatten(),
            end: None,
            done: t.v.is_completed,
            color: None,
            ref_type: None,
        });
    }

    // 3) Recurring dailies — expanded into virtual instances for this day.
    for d in dailies {
        let Some(rule) = recurrence::parse(d.v.recurrence.as_deref()) else { continue };
        if !recurrence::matches(&rule, day) {
            continue;
        }
        out.push(Item {
            kind: Kind::Daily,
            id: d.v.id.to_string(),
            title: d.v.title.clone(),
            description: None,
            schedule_id: None,
            time: d.v.time.clone(),
            end: None,
            done: crate::query::completed_on(completions, &d.v.id, &iso),
            color: None,
            ref_type: None,
        });
    }

    // All-day first, then chronological. Stable within a time so equal rows keep
    // their source order.
    out.sort_by(|a, b| match (&a.time, &b.time) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => x.cmp(y),
    });
    out
}

/// What is on each of the 42 grid cells: `(count, any_deadline)`.
///
/// **Recurring dailies are deliberately excluded.** A habit that fires every day
/// puts something on all 42 cells, which saturates the marker and tells you
/// nothing about which day to look at — the whole point of a month overview is
/// to show where the *unevenly distributed* things are. The agenda still lists
/// them for the selected day.
///
/// `any_deadline` is tracked separately so a day you owe something on can be
/// marked differently from a day that is merely busy.
///
/// Calling `day_items` per cell is the obvious implementation and is what this
/// replaces: that is 42 × every event, with a `Vec<Item>` and its strings
/// allocated and thrown away each time. On the real Bag (300 events) it cost
/// **6.8 ms per keypress**; this is one pass over each collection and costs
/// **0.1 ms**. Same answer — `month_counts_matches_day_items` proves it.
pub fn month_counts(
    year: i16,
    month: i8,
    events: &[Row<Event>],
    tasks: &[Row<Task>],
) -> [(usize, bool); 42] {
    let grid = month_grid(year, month);
    let (first, last) = (grid[0], grid[41]);
    let mut counts = [(0usize, false); 42];

    // Index into the grid, or None when the date is outside it.
    let slot = |d: Date| -> Option<usize> {
        if d < first || d > last {
            return None;
        }
        d.since(first).ok().map(|s| s.get_days() as usize).filter(|i| *i < 42)
    };

    for e in events {
        let Some(start) = deadline::due_day(&e.v.start_at) else { continue };
        let end = e.v.end_at.as_deref().and_then(deadline::due_day).unwrap_or(start);
        // Clamp the span to the grid rather than walking it from the start,
        // so a year-long block costs the same as a one-day one.
        let from = start.max(first);
        let to = end.min(last);
        let (Some(a), Some(b)) = (slot(from), slot(to)) else { continue };
        if a <= b {
            for c in counts.iter_mut().take(b + 1).skip(a) {
                c.0 += 1;
            }
        }
    }

    for t in tasks {
        let Some(due) = t.v.due_date.as_deref().and_then(deadline::due_day) else { continue };
        if let Some(i) = slot(due) {
            counts[i].0 += 1;
            // A completed task is no longer something owed.
            counts[i].1 |= !t.v.is_completed;
        }
    }

    counts
}

/// `HH:MM` in local time for a stored UTC instant.
fn hhmm(iso: &str) -> Option<String> {
    deadline::to_local(iso).map(|z| format!("{:02}:{:02}", z.hour(), z.minute()))
}

/// Step the shown month by `by` months, carrying the selected day-of-month and
/// clamping it into the new month.
///
/// Clamping matters: paging from the 31st into a 30-day month must land on the
/// 30th rather than failing or silently jumping to the 1st.
pub fn shift_month(year: i16, month: i8, day_of_month: i8, by: i32) -> (i16, i8, Date) {
    let first = date(year, month, 1);
    let moved = first
        .checked_add(jiff::Span::new().months(by as i64))
        .unwrap_or(first);
    let last = moved.last_of_month().day();
    let d = date(moved.year(), moved.month(), day_of_month.min(last));
    (moved.year(), moved.month(), d)
}

/// Move the selection by `days`, following the month along with it.
pub fn shift_day(from: Date, days: i32) -> Date {
    from.checked_add(jiff::Span::new().days(days as i64)).unwrap_or(from)
}
