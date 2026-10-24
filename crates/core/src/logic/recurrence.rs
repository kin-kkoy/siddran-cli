//! Port of `parseRecurrence` / `recurrenceMatches` (calendarDates.js:108-130).
//!
//! The desktop deliberately keeps these two in one place because "two
//! implementations of recurrence that must agree is the most expensive kind of
//! duplication in this app" — the same reasoning applies across languages, so
//! this file is a direct translation and the tests pin it to the JS behaviour.

use jiff::civil::{Date, Weekday};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    EveryDay,
    Weekdays,
    Weekends,
    /// Index 0 is **Sunday**, matching JS `Date.getDay()`.
    Mask([bool; 7]),
}

pub fn parse(rec: Option<&str>) -> Option<Rule> {
    let s = rec?.trim();
    match s {
        "every-day" => return Some(Rule::EveryDay),
        "weekdays" => return Some(Rule::Weekdays),
        "weekends" => return Some(Rule::Weekends),
        _ => {}
    }
    if s.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(s).ok()?;
        let arr = v.get("mask")?.as_array()?;
        if arr.len() == 7 {
            let mut mask = [false; 7];
            for (i, b) in arr.iter().enumerate() {
                mask[i] = b.as_bool().unwrap_or(false);
            }
            return Some(Rule::Mask(mask));
        }
    }
    None
}

/// JS `getDay()`: Sunday = 0.
fn js_day(d: Date) -> usize {
    match d.weekday() {
        Weekday::Sunday => 0,
        Weekday::Monday => 1,
        Weekday::Tuesday => 2,
        Weekday::Wednesday => 3,
        Weekday::Thursday => 4,
        Weekday::Friday => 5,
        Weekday::Saturday => 6,
    }
}

pub fn matches(rule: &Rule, d: Date) -> bool {
    let day = js_day(d);
    match rule {
        Rule::EveryDay => true,
        Rule::Weekdays => (1..=5).contains(&day),
        Rule::Weekends => day == 0 || day == 6,
        Rule::Mask(m) => m[day],
    }
}

/// `M T W T F S S` with the firing days lit — Monday-first for display, even
/// though the mask itself is Sunday-first.
pub fn label(rule: &Rule) -> String {
    const NAMES: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
    // display index 0..6 = Mon..Sun  ->  js index 1..6,0
    let js_for_display = [1, 2, 3, 4, 5, 6, 0];
    let fires = |js: usize| match rule {
        Rule::EveryDay => true,
        Rule::Weekdays => (1..=5).contains(&js),
        Rule::Weekends => js == 0 || js == 6,
        Rule::Mask(m) => m[js],
    };
    js_for_display
        .iter()
        .enumerate()
        .map(|(i, &js)| {
            if fires(js) {
                NAMES[i].to_owned()
            } else {
                "·".to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("")
}
