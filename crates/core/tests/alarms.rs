//! Alarm firing rules. The grace window and the disarm-on-completion rule are
//! the two that matter: without them an old deadline screams on every launch,
//! and ticking something off early does not stop it ringing.

use jiff::civil::{time, Time};
use jiff::Timestamp;
use serde_json::json;
use siddran_core::logic::{alarms, deadline};
use siddran_core::model::{Completion, Daily, Task};
use siddran_core::Row;

fn row<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Row<T> {
    Row::from_value(&v).unwrap()
}

fn at_local(day_offset: i64, hhmm: &str) -> String {
    let d = deadline::today()
        .checked_add(jiff::Span::new().days(day_offset))
        .unwrap();
    deadline::from_picker_value(&format!("{d}T{hhmm}")).unwrap().0
}

fn date_only(day_offset: i64) -> String {
    let d = deadline::today()
        .checked_add(jiff::Span::new().days(day_offset))
        .unwrap();
    deadline::from_picker_value(&d.to_string()).unwrap().0
}

const NINE: Time = time(9, 0, 0, 0);

#[test]
fn a_timed_deadline_rings_at_its_time() {
    let tasks: Vec<Row<Task>> = vec![row(json!({
        "id": "t1", "title": "Ship it", "due_date": at_local(0, "17:00"),
        "due_all_day": false, "is_completed": false
    }))];
    let tr = alarms::triggers(&tasks, &[], &[], NINE);
    assert_eq!(tr.len(), 1);
    assert_eq!(tr[0].tier, alarms::Tier::Alarm);
    let z = tr[0].at.to_zoned(jiff::tz::TimeZone::system());
    assert_eq!((z.hour(), z.minute()), (17, 0));
}

#[test]
fn a_date_only_deadline_rings_at_the_chosen_hour_not_midnight() {
    // Stored at local midnight; ringing then helps nobody, so it goes off at the
    // configured hour instead.
    let tasks: Vec<Row<Task>> = vec![row(json!({
        "id": "t1", "title": "All day", "due_date": date_only(0),
        "due_all_day": true, "is_completed": false
    }))];
    let tr = alarms::triggers(&tasks, &[], &[], NINE);
    let z = tr[0].at.to_zoned(jiff::tz::TimeZone::system());
    assert_eq!((z.hour(), z.minute()), (9, 0));
    assert_eq!(z.date(), deadline::today());
}

#[test]
fn a_completed_task_never_rings() {
    let tasks: Vec<Row<Task>> = vec![row(json!({
        "id": "t1", "title": "Done", "due_date": at_local(0, "17:00"),
        "due_all_day": false, "is_completed": true
    }))];
    assert!(alarms::triggers(&tasks, &[], &[], NINE).is_empty());
}

#[test]
fn remind_at_is_a_separate_softer_trigger() {
    let tasks: Vec<Row<Task>> = vec![row(json!({
        "id": "t1", "title": "Ship it", "due_date": at_local(0, "17:00"),
        "due_all_day": false, "remind_at": at_local(0, "16:30"), "is_completed": false
    }))];
    let tr = alarms::triggers(&tasks, &[], &[], NINE);
    assert_eq!(tr.len(), 2);
    assert!(tr.iter().any(|t| t.tier == alarms::Tier::Reminder));
    assert!(tr.iter().any(|t| t.tier == alarms::Tier::Alarm));
}

#[test]
fn a_daily_rings_only_on_days_it_fires_and_only_with_a_time() {
    let timed: Vec<Row<Daily>> = vec![row(json!({
        "id": "d1", "title": "Morning walk", "recurrence": "every-day", "time": "07:30",
        "is_completed": false, "expires_at": null
    }))];
    assert_eq!(alarms::triggers(&[], &timed, &[], NINE).len(), 1);

    // No time of day is no deadline to speak of.
    let untimed: Vec<Row<Daily>> = vec![row(json!({
        "id": "d2", "title": "Read", "recurrence": "every-day", "time": null,
        "is_completed": false, "expires_at": null
    }))];
    assert!(alarms::triggers(&[], &untimed, &[], NINE).is_empty());

    // A rule that does not match today produces nothing.
    let weekend: Vec<Row<Daily>> = vec![row(json!({
        "id": "d3", "title": "Tidy", "recurrence": "weekends", "time": "11:00",
        "is_completed": false, "expires_at": null
    }))];
    let fires = matches!(
        deadline::today().weekday(),
        jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday
    );
    assert_eq!(alarms::triggers(&[], &weekend, &[], NINE).len(), usize::from(fires));
}

#[test]
fn ticking_a_daily_off_early_disarms_it() {
    // Completion is checked at sweep time, not schedule time.
    let dailies: Vec<Row<Daily>> = vec![row(json!({
        "id": "d1", "title": "Morning walk", "recurrence": "every-day", "time": "07:30",
        "is_completed": false, "expires_at": null
    }))];
    assert_eq!(alarms::triggers(&[], &dailies, &[], NINE).len(), 1);

    let done: Vec<Row<Completion>> =
        vec![row(json!({"daily_task_id": "d1", "date": deadline::today().to_string()}))];
    assert!(alarms::triggers(&[], &dailies, &done, NINE).is_empty());
}

#[test]
fn the_grace_window_keeps_old_deadlines_quiet() {
    let now = Timestamp::now();
    let mk = |secs_ago: i64, key: &str| alarms::Trigger {
        key: key.into(),
        at: Timestamp::from_second(now.as_second() - secs_ago).unwrap(),
        tier: alarms::Tier::Alarm,
        title: key.into(),
    };

    let (ring, stale) = alarms::partition(
        vec![
            mk(60, "recent"),                    // a minute ago — ring
            mk(alarms::GRACE + 60, "ancient"),   // older than the window — stale
            alarms::Trigger {                    // still in the future — neither
                key: "future".into(),
                at: Timestamp::from_second(now.as_second() + 600).unwrap(),
                tier: alarms::Tier::Alarm,
                title: "future".into(),
            },
        ],
        now,
        |_| false,
    );
    assert_eq!(ring.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["recent"]);
    assert_eq!(stale.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["ancient"]);
}

#[test]
fn an_alarm_already_fired_does_not_ring_again() {
    let now = Timestamp::now();
    let t = alarms::Trigger {
        key: "k".into(),
        at: Timestamp::from_second(now.as_second() - 60).unwrap(),
        tier: alarms::Tier::Alarm,
        title: "x".into(),
    };
    let (ring, _) = alarms::partition(vec![t.clone()], now, |k| k == "k");
    assert!(ring.is_empty(), "a fired key must stay silent");
    let (ring, _) = alarms::partition(vec![t], now, |_| false);
    assert_eq!(ring.len(), 1);
}

#[test]
fn next_after_is_the_soonest_future_trigger() {
    let now = Timestamp::now();
    let mk = |o: i64| alarms::Trigger {
        key: o.to_string(),
        at: Timestamp::from_second(now.as_second() + o).unwrap(),
        tier: alarms::Tier::Alarm,
        title: String::new(),
    };
    let tr = vec![mk(-100), mk(500), mk(60), mk(900)];
    assert_eq!(
        alarms::next_after(&tr, now).map(|t| t.as_second() - now.as_second()),
        Some(60)
    );
    assert!(alarms::next_after(&[mk(-10)], now).is_none());
}
