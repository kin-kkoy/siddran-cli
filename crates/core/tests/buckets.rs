//! Bucket semantics. These decide what the tasks view shows, so the edge cases
//! deserve pinning rather than being rediscovered from a screenshot.

use serde_json::json;
use siddran_core::logic::deadline;
use siddran_core::model::{Daily, Task};
use siddran_core::query::{self, Bucket};
use siddran_core::Row;

fn task(title: &str, due: Option<&str>, done: bool) -> Row<Task> {
    Row::from_value(&json!({
        "id": title, "title": title, "description": "", "priority": "normal",
        "due_date": due, "is_completed": done,
        "created_at": "2026-01-01T00:00:00.000Z", "updated_at": "2026-01-01T00:00:00.000Z"
    }))
    .unwrap()
}

/// Local midnight on a day `offset` from today, which is how a date-only
/// deadline is stored.
fn day_at(offset: i64) -> String {
    let d = deadline::today()
        .checked_add(jiff::Span::new().days(offset))
        .unwrap();
    let (iso, _) = deadline::from_picker_value(&d.to_string()).unwrap();
    iso
}

#[test]
fn today_includes_overdue_work() {
    // A deadline that slipped is still something to do today. A "today" list
    // that quietly drops it is how things get missed.
    let rows = vec![
        task("today", Some(&day_at(0)), false),
        task("late", Some(&day_at(-3)), false),
        task("tomorrow", Some(&day_at(1)), false),
        task("undated", None, false),
        task("done-late", Some(&day_at(-3)), true),
    ];
    let got: Vec<&str> = query::bucket_tasks(&rows, Bucket::Today)
        .iter()
        .map(|r| r.v.title.as_str())
        .collect();
    // Sorted by deadline, so the oldest slippage comes first.
    assert_eq!(got, vec!["late", "today"]);
}

#[test]
fn the_other_buckets_partition_as_expected() {
    let rows = vec![
        task("today", Some(&day_at(0)), false),
        task("late", Some(&day_at(-3)), false),
        task("soon", Some(&day_at(2)), false),
        task("later", Some(&day_at(9)), false),
        task("undated", None, false),
        task("finished", None, true),
    ];
    let names = |b| -> Vec<String> {
        query::bucket_tasks(&rows, b)
            .iter()
            .map(|r| r.v.title.clone())
            .collect()
    };
    assert_eq!(names(Bucket::Overdue), vec!["late"]);
    // Upcoming holds future deadlines *and* undated work — soonest first, with
    // the undated tail last, because a date outranks an intention.
    assert_eq!(names(Bucket::Upcoming), vec!["soon", "later", "undated"]);
    assert_eq!(names(Bucket::Completed), vec!["finished"]);
    // `All` is every standalone task, open above finished.
    let all = names(Bucket::All);
    assert_eq!(all.len(), 6);
    assert_eq!(all.last().unwrap(), "finished");
    // Dailies and projects are not tasks, so those buckets yield none here.
    assert!(names(Bucket::Dailies).is_empty());
    assert!(names(Bucket::Projects).is_empty());

    // A completed task is never overdue, however old its deadline.
    let rows = vec![task("old-but-done", Some(&day_at(-9)), true)];
    assert!(query::bucket_tasks(&rows, Bucket::Overdue).is_empty());
    assert!(query::bucket_tasks(&rows, Bucket::Today).is_empty());
    // ...but it is still in All and in Completed.
    assert_eq!(query::bucket_tasks(&rows, Bucket::All).len(), 1);
    assert_eq!(query::bucket_tasks(&rows, Bucket::Completed).len(), 1);
}

#[test]
fn the_bucket_order_puts_what_is_due_first_and_the_record_last() {
    // The order is deliberate, so it is pinned: Today and Projects are what you
    // reach for constantly; Completed is a record, not a queue.
    let names: Vec<&str> = Bucket::ALL.iter().map(|b| b.name()).collect();
    assert_eq!(
        names,
        vec!["Today", "Projects", "Overdue", "Upcoming", "All tasks", "Dailies", "Completed"]
    );
    assert_eq!(Bucket::ALL[0], Bucket::Today);
    assert_eq!(*Bucket::ALL.last().unwrap(), Bucket::Completed);
}

#[test]
fn today_is_decided_by_day_not_by_hour() {
    // A task due at 09:00 today is still "today" at 14:00 — overdue is a
    // question about the date, never the clock.
    let (iso, _) = deadline::from_picker_value(&format!("{}T09:00", deadline::today())).unwrap();
    let rows = vec![task("nine am", Some(&iso), false)];
    assert_eq!(query::bucket_tasks(&rows, Bucket::Today).len(), 1);
    assert!(query::bucket_tasks(&rows, Bucket::Overdue).is_empty());
}

fn daily(title: &str, rec: Option<&str>) -> Row<Daily> {
    Row::from_value(&json!({
        "id": title, "title": title, "priority": "normal", "is_completed": false,
        "created_at": "2026-01-01T00:00:00.000Z", "updated_at": "2026-01-01T00:00:00.000Z",
        "expires_at": "2026-01-01T00:00:00.000Z", "recurrence": rec, "time": null
    }))
    .unwrap()
}

#[test]
fn daily_history_separates_not_due_from_not_done() {
    let d = daily("weekday habit", Some("weekdays"));

    // Tick the most recent weekday, whenever the test happens to run.
    let mut day = deadline::today();
    while matches!(
        day.weekday(),
        jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
    ) {
        day = day.yesterday().unwrap();
    }
    let ticked = day.to_string();
    let completions = vec![Row::from_value(&json!({
        "daily_task_id": "weekday habit", "date": ticked
    }))
    .unwrap()];

    let hist = query::daily_history(&completions, &d, 14);
    assert_eq!(hist.len(), 14);
    assert_eq!(hist.last().unwrap().0, deadline::today().to_string(), "history ends today");

    for (iso, done, fires) in &hist {
        let date: jiff::civil::Date = iso.parse().unwrap();
        let weekend = matches!(
            date.weekday(),
            jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
        );
        // `fires` follows the rule, so a weekend is "not due" rather than a miss.
        assert_eq!(*fires, !weekend, "{iso} fires={fires}");
        // `done` follows the completions, and is deliberately independent of
        // `fires` — a habit ticked on a day it was not due is real data, and
        // hiding it would be a lie about what is on disk.
        assert_eq!(*done, *iso == ticked, "{iso} done={done}");
    }
}

#[test]
fn a_completion_on_a_non_firing_day_still_shows() {
    let d = daily("weekday habit", Some("weekdays"));
    // Find a Sunday in the last fortnight and tick it.
    let mut day = deadline::today();
    for _ in 0..14 {
        if day.weekday() == jiff::civil::Weekday::Sunday {
            break;
        }
        day = day.yesterday().unwrap();
    }
    let sunday = day.to_string();
    let completions =
        vec![Row::from_value(&json!({"daily_task_id": "weekday habit", "date": sunday})).unwrap()];

    let hist = query::daily_history(&completions, &d, 15);
    let entry = hist.iter().find(|(i, _, _)| *i == sunday).expect("in range");
    assert!(entry.1, "the completion must be reported");
    assert!(!entry.2, "...even though the rule does not fire that day");
}

#[test]
fn completions_are_identified_by_the_pair_not_an_id() {
    // (daily_task_id, date) is the identity — there is no id column.
    let a = Row::from_value(&json!({"daily_task_id": "x", "date": "2026-09-12"})).unwrap();
    let b = Row::from_value(&json!({"daily_task_id": "y", "date": "2026-09-12"})).unwrap();
    let rows = vec![a, b];
    assert!(query::completed_on(&rows, &siddran_core::Id::Str("x".into()), "2026-09-12"));
    assert!(!query::completed_on(&rows, &siddran_core::Id::Str("x".into()), "2026-09-11"));
    assert!(!query::completed_on(&rows, &siddran_core::Id::Str("z".into()), "2026-09-12"));
}
