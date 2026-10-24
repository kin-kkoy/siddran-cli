use crate::cmd::Ctx;
use crate::out::{self, Mode};
use clap::Subcommand;
use serde_json::json;
use siddran_core::logic::deadline;

#[derive(Subcommand)]
pub enum Cmd {
    /// List calendar events
    Ls {
        /// Only today's
        #[arg(long)]
        today: bool,
        /// Only this many days ahead (default 7 when neither flag is given)
        #[arg(long, value_name = "N")]
        days: Option<i64>,
        /// Everything, however far out
        #[arg(long)]
        all: bool,
    },
    /// List schedules and how many blocks each stamped
    Schedules,
}

pub fn run(cmd: Cmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    let rows = bag.events();
    match cmd {
        Cmd::Ls { today, days, all } => {
            let start = deadline::today();
            let span = if all {
                None
            } else if today {
                Some(0)
            } else {
                Some(days.unwrap_or(7))
            };

            let mut sel: Vec<_> = rows
                .iter()
                .filter(|r| {
                    let Some(d) = deadline::due_day(&r.v.start_at) else {
                        return false;
                    };
                    match span {
                        None => true,
                        Some(n) => {
                            let delta = d.since(start).map(|s| s.get_days()).unwrap_or(0);
                            (0..=n).contains(&(delta as i64))
                        }
                    }
                })
                .collect();
            sel.sort_by(|a, b| a.v.start_at.cmp(&b.v.start_at));

            if mode == Mode::Json {
                let v: Vec<_> = sel.iter().map(|r| json!(r.raw)).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                return true;
            }
            if sel.is_empty() {
                let hint = match span {
                    Some(0) => "nothing scheduled today.".to_owned(),
                    Some(n) => format!("nothing scheduled in the next {n} days."),
                    None => "this Bag has no events.".to_owned(),
                };
                out::empty(mode, &hint);
                return true;
            }
            let out_rows: Vec<Vec<String>> = sel
                .iter()
                .map(|r| {
                    let e = &r.v;
                    vec![
                        deadline::due_day(&e.start_at)
                            .map(|d| d.strftime("%a %b %-d").to_string())
                            .unwrap_or_else(|| "—".into()),
                        if e.all_day {
                            "all day".to_owned()
                        } else {
                            time_range(&e.start_at, e.end_at.as_deref())
                        },
                        out::truncate(&e.title, 44),
                        e.ref_type.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            out::rows(mode, &out_rows);
            true
        }

        Cmd::Schedules => {
            let scheds = bag.schedules();
            if mode == Mode::Json {
                let v: Vec<_> = scheds.iter().map(|r| json!(r.raw)).collect();
                println!("{}", serde_json::to_string_pretty(&v).unwrap());
                return true;
            }
            let out_rows: Vec<Vec<String>> = scheds
                .iter()
                .map(|s| {
                    let blocks = rows
                        .iter()
                        .filter(|e| e.v.schedule_id.as_ref() == Some(&s.v.id))
                        .count();
                    vec![
                        s.v.name.clone(),
                        format!("{blocks} blocks"),
                        s.v.color.clone().unwrap_or_else(|| "—".into()),
                    ]
                })
                .collect();
            out::rows(mode, &out_rows);
            true
        }
    }
}

fn time_range(start: &str, end: Option<&str>) -> String {
    let f = |iso: &str| {
        deadline::to_local(iso)
            .map(|z| format!("{:02}:{:02}", z.hour(), z.minute()))
            .unwrap_or_else(|| "??:??".into())
    };
    match end {
        Some(e) => format!("{}–{}", f(start), f(e)),
        None => f(start),
    }
}
