//! The settings model for the TUI.
//!
//! Two kinds of setting live side by side and must not be confused:
//!
//! - **Terminal settings** are ours, in `~/.config/siddran/config.toml`, and are
//!   editable here.
//! - **Desktop settings** come from `settings.siddran`, belong to the desktop
//!   app, and are shown **read-only**. The TUI has no opinion about `starCount`
//!   or `bookPageHeight`, and writing a file whose keys it does not model is how
//!   a peer app corrupts the thing it shares.

use crate::config;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Ours alone, in `~/.config/siddran/config.toml`.
    Terminal,
    /// Desktop settings the terminal actually honours.
    Shared,
}

impl Section {
    pub const ALL: [Section; 2] = [Section::Terminal, Section::Shared];

    pub fn name(self) -> &'static str {
        match self {
            Section::Terminal => "Terminal",
            Section::Shared => "Shared",
        }
    }
}

/// The desktop keys the terminal honours, and what it does with each.
///
/// Deliberately a short allowlist rather than everything in `settings.siddran`.
/// Nineteen of the twenty keys there are desktop-visual — `starCount`,
/// `bookPageHeight`, `noteEditorWidth` — and listing them in a terminal app
/// would be showing you knobs that do nothing here. If the terminal starts
/// honouring another one, it gets added here; until then its absence is honest.
pub const SHARED: [(&str, &str); 1] = [(
    "rememberNoteState",
    "Continue where you left off. When on, each workspace's feature and \
     selection come back on the next launch.",
)];

/// What kind of thing a terminal setting is, which decides how it is edited.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Cycles through fixed choices.
    Choice(&'static [&'static str]),
    /// Free text, entered at a prompt.
    Text,
    /// Shown, but not editable from here.
    ReadOnly,
}

pub struct Entry {
    pub key: String,
    pub value: String,
    pub kind: Kind,
    /// One line saying what it does, and where the value came from.
    pub help: String,
}

const PALETTES: [&str; 2] = ["ember", "yazi"];
const FRAMES: [&str; 3] = ["rounded", "square", "flat"];

/// The terminal's own settings, with their current effective values.
pub fn terminal_entries(bag: &std::path::Path, editor: &str, palette: &str, frame: &str) -> Vec<Entry> {
    vec![
        Entry {
            key: "palette".into(),
            value: palette.into(),
            kind: Kind::Choice(&PALETTES),
            help: "Colour scheme. Siddran's ember, or yazi's blue.".into(),
        },
        Entry {
            key: "frame".into(),
            value: frame.into(),
            kind: Kind::Choice(&FRAMES),
            help: "rounded and square draw a border around the panes; flat runs them to the edges.".into(),
        },
        Entry {
            key: "editor".into(),
            value: editor.into(),
            kind: Kind::Text,
            help: format!("Opens a note body on Enter — {}.", crate::bagpath::editor_source()),
        },
        Entry {
            key: "alarm_time".into(),
            value: config::get("alarm_time").unwrap_or_else(|| "09:00".into()),
            kind: Kind::Text,
            help: "When a date-only deadline rings. Such a deadline is stored at local \
                   midnight, and ringing then helps nobody, so it goes off at this hour \
                   instead — a policy about the alarm, not a time written onto the task."
                .into(),
        },
        Entry {
            key: "notify".into(),
            value: config::get("notify").unwrap_or_else(|| "(off)".into()),
            kind: Kind::Text,
            help: "A command run when an alarm fires, given a title and a body — \
                   `notify-send` is the usual one. Off by default: shelling out should \
                   never be a surprise. A desktop notification outlives the terminal being \
                   buried behind other windows, which the status line does not."
                .into(),
        },
        Entry {
            key: "bag".into(),
            value: config::get("bag").unwrap_or_else(|| bag.display().to_string()),
            kind: Kind::Text,
            help: "Which Bag to open. Takes effect next start; --bag and $SIDDRAN_BAG win over it."
                .into(),
        },
        Entry {
            key: "config file".into(),
            value: config::path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(no home directory)".into()),
            kind: Kind::ReadOnly,
            help: "Where the settings above are stored.".into(),
        },
    ]
}

/// The shared settings, read from `settings.siddran` and shown read-only.
///
/// Read-only because that file belongs to the desktop app: writing keys we do
/// not model is exactly what `docs/parity.md` exists to prevent. Change them in
/// the desktop; the terminal picks the change up on its next start.
pub fn shared_entries(
    settings: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Vec<Entry> {
    SHARED
        .iter()
        .map(|(key, help)| {
            let value = settings
                .and_then(|m| m.get(*key))
                .map(|v| match v.as_str() {
                    Some(s) => s.to_owned(),
                    None => v.to_string(),
                })
                .unwrap_or_else(|| "(not set — treated as off)".into());
            Entry {
                key: (*key).to_owned(),
                value,
                kind: Kind::ReadOnly,
                help: format!("{help}  Set it in the desktop app — this file is the desktop's."),
            }
        })
        .collect()
}
