use crate::cmd::Ctx;
use crate::out::{self, Mode};
use clap::Subcommand;

#[derive(Subcommand)]
pub enum Cmd {
    /// List every setting
    Ls,
    /// Print one setting's value
    Get { key: String },
}

pub fn run(cmd: Cmd, ctx: &Ctx) -> bool {
    let (bag, mode) = (&ctx.bag, ctx.mode);
    let Some(map) = bag.settings_map() else {
        eprintln!("this Bag has no settings.siddran");
        return false;
    };
    match cmd {
        Cmd::Ls => {
            if mode == Mode::Json {
                println!("{}", serde_json::to_string_pretty(map).unwrap());
                return true;
            }
            // Deliberately shown as-is and unparsed. These are the desktop's
            // keys; the TUI preserves them untouched rather than interpreting
            // them, so it can never corrupt one it does not understand.
            let rows: Vec<Vec<String>> = map
                .iter()
                .map(|(k, v)| vec![k.clone(), out::truncate(&scalar(v), 40)])
                .collect();
            out::rows(mode, &rows);
            true
        }
        Cmd::Get { key } => match map.get(&key) {
            Some(v) => {
                println!("{}", scalar(v));
                true
            }
            None => {
                eprintln!("no setting named {key:?}");
                false
            }
        },
    }
}

/// Strings unquoted, everything else as JSON — so `$(siddran settings get theme)`
/// gives `default`, not `"default"`.
fn scalar(v: &serde_json::Value) -> String {
    match v.as_str() {
        Some(s) => s.to_owned(),
        None => v.to_string(),
    }
}
