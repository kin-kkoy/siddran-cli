//! The calendar is a projection, never a second copy of the data. These pin the
//! grid shape and the merge rules.

use jiff::civil::{date, Weekday};
use serde_json::json;
use siddran_core::logic::calendar::{self, Kind};
use siddran_core::logic::deadline;
use siddran_core::model::{Completion, Daily, Event, Task};
use siddran_core::Row;

fn row<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Row<T> {
    Row::from_value(&v).expect("fixture deserializes")
}

#[test]
fn the_grid_is_always_42_monday_first_cells() {
    for (y, m) in [(2026, 1), (2026, 2), (2026, 9), (2027, 12)] {
        let g = calendar::month_grid(y, m);
        assert_eq!(g.len(), 42, "{y}-{m} grid is not six weeks");
        assert_eq!(g[0].weekday(), Weekday::Monday, "{y}-{m} does not start on Monday");
        // Contiguous days, no gaps.
        for w in g.windows(2) {
            assert_eq!(w[1], w[0].tomorrow().unwrap());
        }
        // The 1st is in the first week.
        assert!(g[..7].contains(&date(y, m, 1)), "{y}-{m} 1st is not in row one");
    }
}

#[test]
fn february_2026_starts_on_the_right_monday() {
    // 1 Feb 2026 is a Sunday, so the grid must back up six days to Mon 26 Jan.
    assert_eq!(date(2026, 2, 1).weekday(), Weekday::Sunday);
    assert_eq!(calendar::month_grid_start(2026, 2), date(2026, 1, 26));
}

/// A UTC instant for a local date+time, which is how the Bag stores things.
fn at(day: &str, time: Option<&str>) -> String {
    let v = match time {
        Some(t) => format!("{day}T{t}"),
        None => day.to_owned(),
    };
    deadline::from_picker_value(&v).unwrap().0
}

#[test]
fn a_day_merges_events_tasks_and_dailies_all_day_first() {
    let day = date(2026, 9, 14); // a Monday
    let events: Vec<Row<Event>> = vec![
        row(json!({"id": 1, "title": "Standup", "start_at": at("2026-09-14", Some("09:30")),
                   "end_at": at("2026-09-14", Some("09:45")), "all_day": false, "color": "#5a9cf0"})),
        row(json!({"id": 2, "title": "Public holiday", "start_at": at("2026-09-14", None),
                   "end_at": null, "all_day": true})),
    ];
    let tasks: Vec<Row<Task>> = vec![
        row(json!({"id": "t1", "title": "Ship the thing", "due_date": at("2026-09-14", Some("17:00")),
                   "due_all_day": false, "is_completed": false})),
        row(json!({"id": "t2", "title": "Date-only task", "due_date": at("2026-09-14", None),
                   "due_all_day": true, "is_completed": false})),
        row(json!({"id": "t3", "title": "Another day", "due_date": at("2026-09-15", None),
                   "due_all_day": true, "is_completed": false})),
    ];
    let dailies: Vec<Row<Daily>> = vec![
        row(json!({"id": "d1", "title": "Morning walk", "recurrence": "weekdays", "time": "07:30",
                   "is_completed": false, "expires_at": null})),
        row(json!({"id": "d2", "title": "Weekend only", "recurrence": "weekends", "time": null,
                   "is_completed": false, "expires_at": null})),
    ];
    let completions: Vec<Row<Completion>> =
        vec![row(json!({"daily_task_id": "d1", "date": "2026-09-14"}))];

    let items = calendar::day_items(day, &events, &tasks, &dailies, &completions);
    let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();

    // All-day first, then chronological. The weekend daily and the next day's
    // task are both absent.
    assert_eq!(
        titles,
        vec!["Public holiday", "Date-only task", "Morning walk", "Standup", "Ship the thing"]
    );

    let walk = items.iter().find(|i| i.title == "Morning walk").unwrap();
    assert_eq!(walk.kind, Kind::Daily);
    assert!(walk.done, "today's completion should be reflected");

    let standup = items.iter().find(|i| i.title == "Standup").unwrap();
    assert_eq!(standup.time.as_deref(), Some("09:30"));
    assert_eq!(standup.end.as_deref(), Some("09:45"));
}

#[test]
fn a_multi_day_event_appears_on_every_day_it_spans() {
    let events: Vec<Row<Event>> = vec![row(json!({
        "id": 9, "title": "Conference", "all_day": true,
        "start_at": at("2026-09-14", None), "end_at": at("2026-09-16", None)
    }))];
    let (t, d, c) = (vec![], vec![], vec![]);
    for day in 14..=16 {
        let items = calendar::day_items(date(2026, 9, day), &events, &t, &d, &c);
        assert_eq!(items.len(), 1, "missing on the {day}th");
    }
    assert!(calendar::day_items(date(2026, 9, 13), &events, &t, &d, &c).is_empty());
    assert!(calendar::day_items(date(2026, 9, 17), &events, &t, &d, &c).is_empty());
}

#[test]
fn a_legacy_task_with_no_flag_reads_as_all_day() {
    // No `due_all_day` key at all, stored at local midnight — the pre-flag
    // signal for a date-level deadline (deadline.js:58).
    let tasks: Vec<Row<Task>> = vec![row(json!({
        "id": 1625, "title": "Legacy", "due_date": at("2026-09-14", None), "is_completed": false
    }))];
    assert!(tasks[0].raw.get("due_all_day").is_none(), "precondition");

    let items = calendar::day_items(date(2026, 9, 14), &[], &tasks, &[], &[]);
    assert_eq!(items.len(), 1);
    assert!(items[0].time.is_none(), "should render as all-day, not as a midnight appointment");
}

#[test]
fn an_ephemeral_daily_never_appears() {
    // Only *recurring* dailies expand onto the calendar; a one-off has no rule
    // and belongs to the day it was made, which the calendar does not own.
    let dailies: Vec<Row<Daily>> = vec![row(json!({
        "id": "x", "title": "One-off", "recurrence": null, "time": null,
        "is_completed": false, "expires_at": null
    }))];
    for day in 10..20 {
        assert!(calendar::day_items(date(2026, 9, day), &[], &[], &dailies, &[]).is_empty());
    }
}

#[test]
fn real_calendar_data_projects_without_panicking() {
    let Ok(p) = std::env::var("SIDDRAN_TEST_BAG") else { return };
    let bag = siddran_core::Bag::open(&p).unwrap();
    let (e, t, d, c) = (bag.events(), bag.tasks(), bag.dailies(), bag.completions());
    let counts = calendar::month_counts(2026, 7, &e, &t);
    let seen: usize = counts.iter().map(|c| c.0).sum();
    let owed = counts.iter().filter(|c| c.1).count();
    eprintln!("July 2026 grid: {seen} blocks/deadlines, {owed} day(s) with something owed");

    // The fast path still agrees with the slow one on real data — with dailies
    // excluded, which is what the grid marker counts.
    for (i, day) in calendar::month_grid(2026, 7).iter().enumerate() {
        assert_eq!(
            counts[i].0,
            calendar::day_items(*day, &e, &t, &[], &c).len(),
            "cell {i} ({day})"
        );
    }
    let _ = &d;
}

#[test]
fn paging_months_clamps_the_day_instead_of_failing() {
    // 31 Jan -> February has no 31st. Landing on the 28th is the useful answer;
    // failing, or silently jumping to the 1st, is not.
    let (y, m, d) = calendar::shift_month(2026, 1, 31, 1);
    assert_eq!((y, m), (2026, 2));
    assert_eq!(d, date(2026, 2, 28));

    // 2028 is a leap year, so the same move lands on the 29th.
    assert_eq!(calendar::shift_month(2028, 1, 31, 1).2, date(2028, 2, 29));

    // Year boundaries in both directions.
    assert_eq!(calendar::shift_month(2026, 12, 15, 1).2, date(2027, 1, 15));
    assert_eq!(calendar::shift_month(2026, 1, 15, -1).2, date(2025, 12, 15));

    // And a day step crosses months and years on its own.
    assert_eq!(calendar::shift_day(date(2026, 12, 31), 1), date(2027, 1, 1));
    assert_eq!(calendar::shift_day(date(2026, 3, 1), -1), date(2026, 2, 28));
}

#[test]
fn month_counts_matches_day_items() {
    // The fast path and the obvious path must agree — that equivalence is the
    // only thing making the optimisation safe.
    let events: Vec<Row<Event>> = vec![
        row(json!({"id": 1, "title": "One day", "start_at": at("2026-09-14", Some("09:00")),
                   "end_at": at("2026-09-14", Some("10:00")), "all_day": false})),
        row(json!({"id": 2, "title": "Spans a week", "start_at": at("2026-09-16", None),
                   "end_at": at("2026-09-22", None), "all_day": true})),
        // Starts before the grid and ends after it — the clamping case.
        row(json!({"id": 3, "title": "Very long", "start_at": at("2026-01-01", None),
                   "end_at": at("2026-12-31", None), "all_day": true})),
        // Entirely outside the grid.
        row(json!({"id": 4, "title": "Elsewhere", "start_at": at("2025-03-03", None),
                   "end_at": null, "all_day": true})),
    ];
    let tasks: Vec<Row<Task>> = vec![
        row(json!({"id": "t1", "title": "Due", "due_date": at("2026-09-30", None),
                   "due_all_day": true, "is_completed": false})),
        row(json!({"id": "t2", "title": "Outside", "due_date": at("2024-01-01", None),
                   "due_all_day": true, "is_completed": false})),
    ];
    let dailies: Vec<Row<Daily>> = vec![
        row(json!({"id": "d1", "title": "Weekdays", "recurrence": "weekdays", "time": "07:30",
                   "is_completed": false, "expires_at": null})),
        row(json!({"id": "d2", "title": "One-off", "recurrence": null, "time": null,
                   "is_completed": false, "expires_at": null})),
    ];

    for (y, m) in [(2026, 9), (2026, 2), (2027, 1)] {
        let fast = calendar::month_counts(y, m, &events, &tasks);
        for (i, day) in calendar::month_grid(y, m).iter().enumerate() {
            // Dailies are excluded from the grid marker on purpose, so compare
            // against a projection with none.
            let slow = calendar::day_items(*day, &events, &tasks, &[], &[]).len();
            assert_eq!(fast[i].0, slow, "{y}-{m} cell {i} ({day})");
        }
    }
    let _ = &dailies;
}

#[test]
fn the_month_marker_ignores_dailies_and_flags_deadlines() {
    // A habit that fires daily would mark all 42 cells, saturating the signal.
    let dailies: Vec<Row<Daily>> = vec![row(json!({
        "id": "d", "title": "Every day", "recurrence": "every-day", "time": null,
        "is_completed": false, "expires_at": null
    }))];
    let counts = calendar::month_counts(2026, 9, &[], &[]);
    assert!(counts.iter().all(|c| c.0 == 0), "dailies leaked into the grid marker");
    let _ = dailies;

    // An open deadline flags its day; a completed one does not.
    let tasks: Vec<Row<Task>> = vec![
        row(json!({"id": "a", "title": "Owed", "due_date": at("2026-09-14", None),
                   "due_all_day": true, "is_completed": false})),
        row(json!({"id": "b", "title": "Done", "due_date": at("2026-09-15", None),
                   "due_all_day": true, "is_completed": true})),
    ];
    let counts = calendar::month_counts(2026, 9, &[], &tasks);
    let grid = calendar::month_grid(2026, 9);
    let at_day = |d: u8| counts[grid.iter().position(|g| g.day() == d as i8 && g.month() == 9).unwrap()];
    assert_eq!(at_day(14), (1, true), "an open deadline should flag its day");
    assert_eq!(at_day(15), (1, false), "a completed one should not");
}
