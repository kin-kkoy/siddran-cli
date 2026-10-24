//! Timezone handling, pinned.
//!
//! Everything on disk is UTC with a `Z`, but *all* day arithmetic in Siddran is
//! local: `isoDate` (calendarDates.js:12-18), the overdue filter,
//! `recurrenceMatches`, `allDayFireTime`. Get the UTC↔local boundary wrong and
//! every deadline shifts by the offset — near midnight, by a whole day — with no
//! error raised anywhere.
//!
//! This is its own test binary so `TZ` can be set before jiff resolves the
//! system zone. Do not merge it into another test file.

use jiff::tz::TimeZone;
use siddran_core::logic::deadline;

#[test]
fn system_zone_is_actually_detected() {
    // Must run before anything else touches the system zone.
    std::env::set_var("TZ", "Asia/Manila");

    let utc_midnight = "2026-08-23T00:00:00.000Z";
    let sys = deadline::to_local(utc_midnight).expect("parses");

    // In Manila (+0800) that instant is 08:00 on the same day. If jiff's
    // `tz-system` feature is missing, `TimeZone::system()` falls back to UTC and
    // this reads 00:00 — the exact bug this test exists for.
    assert_eq!(
        sys.hour(),
        8,
        "system zone resolved to UTC, not $TZ — is jiff's `tz-system` feature enabled?"
    );
}

#[test]
fn utc_to_local_crosses_the_day_boundary() {
    let manila = TimeZone::get("Asia/Manila").expect("tzdb is readable");

    // 23:00 UTC is already the *next* day in Manila. A deadline stored here
    // belongs to the 24th locally, and the overdue/today filters must agree.
    let z = deadline::to_zone("2026-08-23T23:00:00.000Z", &manila).unwrap();
    assert_eq!(z.date().to_string(), "2026-08-24");
    assert_eq!(z.hour(), 7);

    // ...and 16:00 UTC is local midnight on the following day, which is exactly
    // how a date-only deadline is stored for a +0800 user.
    let z = deadline::to_zone("2026-08-23T16:00:00.000Z", &manila).unwrap();
    assert_eq!(z.date().to_string(), "2026-08-24");
    assert_eq!((z.hour(), z.minute()), (0, 0));
}

#[test]
fn legacy_rows_without_the_flag_read_as_date_only_at_local_midnight() {
    // deadline.js:58 — `allDay == null && hours === 0 && minutes === 0`.
    // A row with no `due_all_day` key at local midnight is date-only...
    let midnight_local_for_plus8 = "2026-08-23T16:00:00.000Z";
    assert!(deadline::is_all_day(midnight_local_for_plus8, None));

    // ...but the same row at any other time is not, and an explicit flag always
    // wins so a deliberately chosen 00:00 survives a round trip.
    assert!(!deadline::is_all_day("2026-08-23T01:00:00.000Z", None));
    assert!(!deadline::is_all_day(midnight_local_for_plus8, Some(false)));
    assert!(deadline::is_all_day("2026-08-23T01:00:00.000Z", Some(true)));
}

#[test]
fn recurrence_masks_are_sunday_indexed() {
    use jiff::civil::date;
    use siddran_core::logic::recurrence::{self, Rule};

    // JS `getDay()` puts Sunday at 0, and the stored mask follows it.
    let sunday = date(2026, 9, 13);
    let monday = date(2026, 9, 14);
    let saturday = date(2026, 9, 12);

    let mut mask = [false; 7];
    mask[0] = true; // Sunday only
    let rule = Rule::Mask(mask);
    assert!(recurrence::matches(&rule, sunday));
    assert!(!recurrence::matches(&rule, monday));

    assert!(recurrence::matches(&Rule::Weekdays, monday));
    assert!(!recurrence::matches(&Rule::Weekdays, sunday));
    assert!(recurrence::matches(&Rule::Weekends, saturday));
    assert!(recurrence::matches(&Rule::Weekends, sunday));

    assert_eq!(recurrence::parse(Some("every-day")), Some(Rule::EveryDay));
    assert_eq!(recurrence::parse(Some(r#"{"mask":[false,true,true,true,true,true,false]}"#)),
               Some(Rule::Mask([false,true,true,true,true,true,false])));
    assert_eq!(recurrence::parse(Some("nonsense")), None);
    assert_eq!(recurrence::parse(None), None);
}

#[test]
fn project_priority_is_derived_from_incomplete_subtasks() {
    use serde_json::json;
    use siddran_core::logic::priority;
    use siddran_core::model::ProjectTask;

    let mk = |p: &str, done: bool| -> ProjectTask {
        serde_json::from_value(json!({"id":"x","title":"t","priority":p,"is_completed":done})).unwrap()
    };

    // localStore.js:367-373 — completed subtasks are excluded entirely, and an
    // empty set is 'normal' rather than an error.
    assert_eq!(priority::bucket(&[]), "normal");
    assert_eq!(priority::bucket(&[mk("high", true)]), "normal");
    assert_eq!(priority::bucket(&[mk("low", false)]), "very_low");
    assert_eq!(priority::bucket(&[mk("high", false)]), "very_high");
    assert_eq!(priority::bucket(&[mk("low", false), mk("high", false)]), "normal");
    // An unrecognised priority weighs as normal (`PRIO_W[p] || 2`).
    assert_eq!(priority::bucket(&[mk("bogus", false)]), "normal");
    assert_eq!(priority::progress(&[mk("low", true), mk("low", false)]), (1, 2));
}

#[test]
fn timestamps_match_javascript_to_iso_string() {
    use jiff::Timestamp;
    use siddran_core::logic::deadline::iso_ms;

    // JS emits exactly three fractional digits, always, with a literal Z.
    let t: Timestamp = "2026-08-11T07:40:42.349Z".parse().unwrap();
    assert_eq!(iso_ms(t), "2026-08-11T07:40:42.349Z");

    // Zero milliseconds are still printed as .000 — not elided.
    let t: Timestamp = "2026-08-23T01:00:00Z".parse().unwrap();
    assert_eq!(iso_ms(t), "2026-08-23T01:00:00.000Z");

    // Sub-millisecond precision is truncated, as JS Date has none.
    let t: Timestamp = "2026-08-23T01:00:00.123456Z".parse().unwrap();
    assert_eq!(iso_ms(t), "2026-08-23T01:00:00.123Z");

    // And a generated stamp must be parseable back — the round trip the Bag
    // relies on when it compares updated_at values.
    let now = siddran_core::logic::deadline::now_iso();
    assert!(now.parse::<Timestamp>().is_ok(), "generated {now} does not parse");
    assert_eq!(now.len(), 24, "{now} is not the canonical 24-char form");
}
