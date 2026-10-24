//! Deadline semantics — the subtlest thing in the Bag. See `utils/deadline.js`.
//!
//! `due_date` is one ISO timestamp with nowhere to put "no time", so the
//! distinction lives in `due_all_day`. A date-only deadline is stored at **local
//! midnight**. Crucially, rows written before that flag existed have **no
//! `due_all_day` key at all**, and for those local midnight *is* the "no time"
//! signal — `allDay == null && hours == 0 && minutes == 0`.
//!
//! Every comparison here normalises to the start of the local day, exactly as
//! the JS does, so the stored hour never decides anything: the flag does.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

/// Parse a stored ISO-8601 instant (`2026-08-23T01:00:00.000Z`) into the
/// system-local zone. All day arithmetic in Siddran is local-time.
///
/// This depends on jiff's `tz-system` feature. Without it `TimeZone::system()`
/// silently returns UTC and every deadline reads as if it were in UTC — which
/// is wrong by a whole day near midnight, with no error anywhere. That is not
/// hypothetical: it shipped in the first build of this file. `tests/timezone.rs`
/// exists to make sure it cannot come back.
pub fn to_local(iso: &str) -> Option<Zoned> {
    to_zone(iso, &jiff::tz::TimeZone::system())
}

/// The same conversion against an explicit zone, so behaviour can be tested
/// without depending on where the test happens to run.
pub fn to_zone(iso: &str, tz: &jiff::tz::TimeZone) -> Option<Zoned> {
    iso.parse::<Timestamp>().ok().map(|t| t.to_zoned(tz.clone()))
}

/// `new Date().toISOString()` — always exactly three fractional digits and a
/// literal `Z`, e.g. `2026-08-11T07:40:42.349Z`.
///
/// Hand-written on purpose: no crate's default `Display` matches this, and a
/// timestamp that differs in precision from every other row in the file is both
/// an ugly diff and a subtle inequality when strings are compared (the Bag
/// compares `updated_at` as text in places).
pub fn now_iso() -> String {
    iso_ms(Timestamp::now())
}

pub fn iso_ms(t: Timestamp) -> String {
    let z = t.to_zoned(jiff::tz::TimeZone::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        z.year(),
        z.month(),
        z.day(),
        z.hour(),
        z.minute(),
        z.second(),
        z.millisecond()
    )
}

pub fn today() -> Date {
    Zoned::now().date()
}

/// Is this deadline date-only? True when the flag says so, and also when the
/// flag is absent and the timestamp sits at local midnight (the legacy signal).
pub fn is_all_day(due_date: &str, flag: Option<bool>) -> bool {
    if let Some(f) = flag {
        return f;
    }
    match to_local(due_date) {
        Some(z) => z.hour() == 0 && z.minute() == 0,
        None => false,
    }
}

/// The local calendar day a deadline falls on.
pub fn due_day(due_date: &str) -> Option<Date> {
    to_local(due_date).map(|z| z.date())
}

/// Overdue is decided by day, never by hour — a task due "today" is not overdue
/// at 14:00 just because it was stored at local midnight.
pub fn is_overdue(due_date: &str, is_completed: bool) -> bool {
    if is_completed {
        return false;
    }
    match due_day(due_date) {
        Some(d) => d < today(),
        None => false,
    }
}

pub fn is_today(due_date: &str) -> bool {
    due_day(due_date) == Some(today())
}

/// `today`, `tomorrow`, `in 3d`, `overdue 2d`, or an absolute date further out.
/// Appends the time only when the deadline actually carries one.
pub fn human(due_date: &str, flag: Option<bool>) -> String {
    let Some(z) = to_local(due_date) else {
        return due_date.to_owned();
    };
    let day = z.date();
    let delta = day.since(today()).map(|s| s.get_days()).unwrap_or(0);

    let base = match delta {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        -1 => "yesterday".to_owned(),
        d if d < 0 => format!("overdue {}d", -d),
        d if d <= 7 => format!("in {d}d"),
        _ => day.strftime("%b %-d").to_string(),
    };

    if is_all_day(due_date, flag) {
        base
    } else {
        format!("{base} {:02}:{:02}", z.hour(), z.minute())
    }
}

/// `fromPickerValue` (deadline.js:29-43), plus the words a CLI wants.
///
/// `today | tomorrow | yesterday | YYYY-MM-DD | YYYY-MM-DDTHH:MM` ->
/// `(iso, all_day)`. A value with no time is stored at **local midnight** with
/// the flag set — storing 23:59 for "end of Tuesday" reads better in the
/// abstract and would make the calendar draw every dateless task as a
/// late-night appointment, which is why the desktop rejected it.
pub fn from_picker_value(v: &str) -> Option<(String, bool)> {
    use jiff::civil::{date, Time};

    let t0 = today();
    let (d, t) = match v {
        "today" => (t0, None),
        "tomorrow" => (t0.tomorrow().ok()?, None),
        "yesterday" => (t0.yesterday().ok()?, None),
        _ => {
            let (dpart, tpart) = match v.split_once('T') {
                Some((d, t)) => (d, Some(t)),
                None => (v, None),
            };
            let p: Vec<&str> = dpart.split('-').collect();
            if p.len() != 3 {
                return None;
            }
            let d = date(p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?);
            let t = match tpart {
                Some(t) => {
                    let hm: Vec<&str> = t.split(':').collect();
                    if hm.len() < 2 {
                        return None;
                    }
                    Some(Time::new(hm[0].parse().ok()?, hm[1].parse().ok()?, 0, 0).ok()?)
                }
                None => None,
            };
            (d, t)
        }
    };

    let all_day = t.is_none();
    let zoned = d
        .to_datetime(t.unwrap_or(Time::midnight()))
        .to_zoned(jiff::tz::TimeZone::system())
        .ok()?;
    Some((iso_ms(zoned.timestamp()), all_day))
}

/// Seconds until the next local midnight.
///
/// The TUI needs this because "today" and "overdue" are date-dependent: a
/// session left open overnight would otherwise keep showing yesterday's answer.
/// It is also the only routine timer in the app — one wakeup a day.
pub fn seconds_to_midnight() -> Option<u64> {
    let now = Zoned::now();
    let mid = today()
        .tomorrow()
        .ok()?
        .to_datetime(jiff::civil::Time::midnight())
        .to_zoned(now.time_zone().clone())
        .ok()?;
    let secs = mid.timestamp().as_second() - now.timestamp().as_second();
    (secs > 0).then_some(secs as u64)
}

/// 23:59:59.999 local today — the expiry an ephemeral daily is created with
/// (localStore.js writes end-of-day, not now+24h).
pub fn end_of_today() -> String {
    let t = jiff::civil::Time::new(23, 59, 59, 999_000_000).unwrap_or(jiff::civil::Time::midnight());
    today()
        .to_datetime(t)
        .to_zoned(jiff::tz::TimeZone::system())
        .map(|z| iso_ms(z.timestamp()))
        .unwrap_or_default()
}
