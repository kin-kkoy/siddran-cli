//! When each thing with a deadline should go off.
//!
//! A direct port of `utils/alarmSchedule.js`, whose header says it was written
//! dependency-free precisely so it could be moved to Rust without dragging
//! anything with it. The rules live here and nowhere else.
//!
//! Two tiers:
//! - `Reminder` — the per-task "remind me at", a gentle nudge.
//! - `Alarm` — the deadline itself.
//!
//! The sweep is **stateless**: it reports every trigger that has a fire time,
//! due or not, and the caller compares against the clock. That is what makes it
//! testable without freezing time.

use crate::logic::{deadline, recurrence};
use crate::model::{Completion, Daily, Task};
use crate::row::Row;
use jiff::civil::Time;
use jiff::Timestamp;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Reminder,
    Alarm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    /// Stable across sweeps, so a fired alarm can be remembered and not repeat.
    pub key: String,
    pub at: Timestamp,
    pub tier: Tier,
    pub title: String,
}

/// The hour a date-only deadline rings at, on the local day it falls on.
///
/// A date-only deadline is stored at local midnight; ringing at midnight helps
/// nobody, so it goes off at a chosen hour instead — a policy about the alarm,
/// not a time silently written onto the task.
pub fn all_day_fire_time(due_iso: &str, at: Time) -> Option<Timestamp> {
    let day = deadline::due_day(due_iso)?;
    day.to_datetime(at)
        .to_zoned(jiff::tz::TimeZone::system())
        .ok()
        .map(|z| z.timestamp())
}

/// A daily's `time` is `HH:MM` local. Built from date parts, never by parsing a
/// `YYYY-MM-DDTHH:MM` string — some engines read that as UTC and shift every
/// alarm by the local offset.
pub fn daily_fire_time(hhmm: &str, day: jiff::civil::Date) -> Option<Timestamp> {
    let (h, m) = hhmm.split_once(':')?;
    let t = Time::new(h.parse().ok()?, m.parse().ok()?, 0, 0).ok()?;
    day.to_datetime(t)
        .to_zoned(jiff::tz::TimeZone::system())
        .ok()
        .map(|z| z.timestamp())
}

/// Every trigger with a fire time at all, whether or not it is due yet.
pub fn triggers(
    tasks: &[Row<Task>],
    dailies: &[Row<Daily>],
    completions: &[Row<Completion>],
    all_day_at: Time,
) -> Vec<Trigger> {
    let mut out = Vec::new();

    for t in tasks {
        if t.v.is_completed {
            continue;
        }
        if let Some(due) = t.v.due_date.as_deref() {
            let at = if deadline::is_all_day(due, t.v.due_all_day) {
                all_day_fire_time(due, all_day_at)
            } else {
                due.parse::<Timestamp>().ok()
            };
            if let Some(at) = at {
                out.push(Trigger {
                    key: format!("t:{}:due:{due}", t.v.id),
                    at,
                    tier: Tier::Alarm,
                    title: t.v.title.clone(),
                });
            }
        }
        if let Some(rem) = t.v.remind_at.as_deref() {
            if let Ok(at) = rem.parse::<Timestamp>() {
                out.push(Trigger {
                    key: format!("t:{}:rem:{rem}", t.v.id),
                    at,
                    tier: Tier::Reminder,
                    title: t.v.title.clone(),
                });
            }
        }
    }

    let today = deadline::today();
    let today_iso = today.to_string();

    for d in dailies {
        let Some(hhmm) = d.v.time.as_deref() else { continue }; // no time is no deadline
        match recurrence::parse(d.v.recurrence.as_deref()) {
            Some(rule) => {
                if !recurrence::matches(&rule, today) {
                    continue;
                }
                // Completion is checked at sweep time, not schedule time, so
                // ticking something off early actually disarms it.
                if crate::query::completed_on(completions, &d.v.id, &today_iso) {
                    continue;
                }
            }
            None => {
                if d.v.is_completed {
                    continue;
                }
                let expired = d
                    .v
                    .expires_at
                    .as_deref()
                    .and_then(|e| e.parse::<Timestamp>().ok())
                    .is_some_and(|e| e < Timestamp::now());
                if expired {
                    continue;
                }
            }
        }
        if let Some(at) = daily_fire_time(hhmm, today) {
            out.push(Trigger {
                key: format!("d:{}:{today_iso}:{hhmm}", d.v.id),
                at,
                tier: Tier::Alarm,
                title: d.v.title.clone(),
            });
        }
    }

    out
}

/// Twelve hours: anything older than this missed its moment while the app was
/// shut, and should not be announced.
pub const GRACE: i64 = 12 * 60 * 60;

/// Split what is due into what should ring and what merely missed its moment.
///
/// Without the grace window a two-week-old deadline screams on every launch —
/// and keeps screaming, since nothing ever marks it handled.
pub fn partition(
    triggers: Vec<Trigger>,
    now: Timestamp,
    already_fired: impl Fn(&str) -> bool,
) -> (Vec<Trigger>, Vec<Trigger>) {
    let mut ring = Vec::new();
    let mut stale = Vec::new();
    for t in triggers {
        if t.at > now || already_fired(&t.key) {
            continue;
        }
        if t.at.as_second() <= now.as_second() - GRACE {
            stale.push(t);
        } else {
            ring.push(t);
        }
    }
    ring.sort_by_key(|t| t.at);
    (ring, stale)
}

/// The soonest trigger still in the future — what the event loop should wake for.
pub fn next_after(triggers: &[Trigger], now: Timestamp) -> Option<Timestamp> {
    triggers.iter().map(|t| t.at).filter(|at| *at > now).min()
}
