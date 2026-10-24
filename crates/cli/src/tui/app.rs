//! App state and the reducer over it.

use super::term::{Guard, Tui};
use super::theme::{Frame, Palette, Theme};
use super::Effect;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::Matcher;
use siddran_core::md;
use siddran_core::jiff;
use siddran_core::logic::{calendar, deadline};
use siddran_core::query::Bucket;
use siddran_core::Bag;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Notes,
    Tasks,
    Calendar,
    Settings,
}

impl Feature {
    pub fn name(self) -> &'static str {
        match self {
            Feature::Notes => "NOTES",
            Feature::Tasks => "TASKS",
            Feature::Calendar => "CALENDAR",
            Feature::Settings => "SETTINGS",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Feature::Notes => 0,
            Feature::Tasks => 1,
            Feature::Calendar => 2,
            Feature::Settings => 3,
        }
    }
}

/// One workspace: its own feature and its own position, so switching away and
/// back lands exactly where you left.
#[derive(Clone)]
pub struct Workspace {
    pub feature: Feature,
    pub col: usize,
    pub source: usize,
    pub item: usize,
    pub group: usize,
    /// Projects: which subtask is selected inside the open project.
    pub sub: usize,
    /// An applied filter, which outlives the typing of it.
    ///
    /// It lived inside `Mode::Filter` at first, which meant confirming a filter
    /// with Enter *discarded* it — the list snapped back to everything while the
    /// selection index stayed put, so you silently landed on a different note.
    pub filter: Option<String>,
    /// Calendar: the month being shown, and the selected day within it.
    pub month: Option<(i16, i8)>,
    pub day: Option<jiff::civil::Date>,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace {
            feature: Feature::Notes,
            col: 1,
            source: 0,
            item: 0,
            group: 0,
            sub: 0,
            filter: None,
            month: None,
            day: None,
        }
    }
}

/// A left column entry: "All notes", "Unfiled", then the notebooks.
pub enum Source {
    All,
    Unfiled,
    Notebook(String),
}

pub enum Mode {
    Normal,
    /// A yes/no question that must be answered before anything else happens.
    /// Deleting is the only thing that asks.
    Confirm { question: String, action: Pending },
    /// `/` — incremental filter over the current column.
    Filter(String),
    /// A one-line prompt in the status bar (rename, tags, a setting).
    Prompt { what: PromptKind, buf: String, key: String },
    Help,
    /// A message that clears itself.
    Flash(String, bool),
}

/// What a confirmation will do if answered yes. Deleting is the only thing that
/// asks, so every variant is a deletion — the name says what, not whether.
#[derive(Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum Pending {
    DeleteNote(std::path::PathBuf),
    DeleteTask(siddran_core::Id),
    DeleteDaily(siddran_core::Id),
    DeleteProject(siddran_core::Id),
    DeleteSubtask(siddran_core::Id, siddran_core::Id),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Rename,
    Tags,
    /// Creating something; what, depends on the view it was started from.
    NewNote,
    NewNotebook,
    NewTask,
    NewDaily,
    NewProject,
    NewSubtask,
    /// Editing a terminal setting; the key is carried alongside.
    Setting,
}

/// Parsed rows, held so a draw does not re-deserialize them.
///
/// `Bag` keeps the raw JSON as the source of truth for writing, and its
/// accessors parse on demand — right for a one-shot command, wrong for a render
/// loop that asks three times per frame. On the real Bag (300 events) caching
/// these took a calendar keypress from 3.0 ms to 0.3 ms.
#[derive(Clone)]
pub struct Rows {
    pub tasks: Vec<siddran_core::Row<siddran_core::model::Task>>,
    pub dailies: Vec<siddran_core::Row<siddran_core::model::Daily>>,
    pub completions: Vec<siddran_core::Row<siddran_core::model::Completion>>,
    pub projects: Vec<siddran_core::Row<siddran_core::model::Project>>,
    pub events: Vec<siddran_core::Row<siddran_core::model::Event>>,
    pub schedules: Vec<siddran_core::Row<siddran_core::model::Schedule>>,
}

impl Rows {
    fn of(bag: &Bag) -> Rows {
        Rows {
            tasks: bag.tasks(),
            dailies: bag.dailies(),
            completions: bag.completions(),
            projects: bag.projects(),
            events: bag.events(),
            schedules: bag.schedules(),
        }
    }
}

pub struct App {
    pub bag: Bag,
    pub rows: Rows,
    pub root: PathBuf,
    /// Holds no lock while idle — see `check_unchanged` and `end_key`.
    session: crate::session::Session,
    /// The Bag lock, held only while one keypress writes. Released by the
    /// event loop once the key is handled (`end_key`).
    write_lock: std::cell::RefCell<Option<siddran_core::bag::lock::Guard>>,
    pub theme: Theme,
    pub palette: Palette,

    pub ws: usize,
    pub spaces: [Workspace; 4],

    pub mode: Mode,
    pub pending_g: bool,
    /// Preview scroll, reset when the selection changes.
    pub scroll: u16,

    /// Rendered preview, cached by note path. Moving down a list of 44 notes
    /// must not re-parse markdown 44 times.
    cache: Option<(PathBuf, Vec<md::Line>)>,
    matcher: Matcher,

    pub editor: String,
    flash_until: Option<Instant>,
    /// Next local midnight — "today" and "overdue" are date-dependent, so the
    /// display goes stale without it. One wakeup a day.
    midnight: Option<Instant>,

    /// The next deadline to ring, and the keys already rung this session.
    ///
    /// One wakeup per actual deadline, not one per second — which is what lets
    /// the loop keep its 0%-idle guarantee while still having alarms.
    alarm_at: Option<Instant>,
    fired: std::collections::HashSet<String>,
    /// When the last sweep ran. `None` means the next one is the first.
    swept_at: Option<jiff::Timestamp>,
    pub ringing: Option<String>,
}

impl App {
    pub fn new(
        bag: Bag,
        root: PathBuf,
        session: crate::session::Session,
        palette: Palette,
        frame: Frame,
        editor: String,
    ) -> App {
        App {
            rows: Rows::of(&bag),
            bag,
            root,
            session,
            write_lock: std::cell::RefCell::new(None),
            theme: Theme::new(palette, frame),
            palette,
            ws: 0,
            spaces: [
                Workspace::default(),
                Workspace { feature: Feature::Tasks, ..Default::default() },
                Workspace::default(),
                Workspace { feature: Feature::Tasks, ..Default::default() },
            ],
            mode: Mode::Normal,
            pending_g: false,
            scroll: 0,
            cache: None,
            matcher: Matcher::new(nucleo_matcher::Config::DEFAULT),
            editor,
            flash_until: None,
            midnight: None,
            alarm_at: None,
            fired: std::collections::HashSet::new(),
            swept_at: None,
            ringing: None,
        }
    }

    pub fn w(&self) -> &Workspace {
        &self.spaces[self.ws]
    }
    pub fn wm(&mut self) -> &mut Workspace {
        let i = self.ws;
        &mut self.spaces[i]
    }

    // ── sources (left column) ───────────────────────────────────────

    pub fn sources(&self) -> Vec<Source> {
        let mut v = vec![Source::All, Source::Unfiled];
        v.extend(self.bag.notebooks.iter().map(|(n, _)| Source::Notebook(n.clone())));
        v
    }

    pub fn source_label(&self, s: &Source) -> (String, usize) {
        match s {
            Source::All => ("All notes".into(), self.bag.notes.len()),
            Source::Unfiled => ("Unfiled".into(), self.bag.notes_in(None).len()),
            Source::Notebook(n) => (n.clone(), self.bag.notes_in(Some(n)).len()),
        }
    }

    /// Notes in a source, unfiltered. Shared by `notes()` and by the
    /// save/restore path, which must not see a live filter.
    pub fn notes_for(&self, srcs: &[Source], sel: usize) -> Vec<usize> {
        let Some(src) = srcs.get(sel) else { return Vec::new() };
        self.bag
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| match src {
                Source::All => true,
                Source::Unfiled => n.notebook.is_none(),
                Source::Notebook(nb) => n.notebook.as_deref() == Some(nb.as_str()),
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Notes in the selected source, after the filter.
    pub fn notes(&mut self) -> Vec<usize> {
        let srcs = self.sources();
        let sel = self.w().source.min(srcs.len().saturating_sub(1));
        let mut idx = self.notes_for(&srcs, sel);

        let live = match &self.mode {
            Mode::Filter(q) => Some(q.clone()),
            _ => self.w().filter.clone(),
        };
        if let Some(q) = live {
            if !q.is_empty() {
                // Matched synchronously on every keystroke: at 44 notes this is
                // tens of microseconds, which is why no worker thread exists.
                let pat = Pattern::parse(&q, CaseMatching::Ignore, Normalization::Smart);
                let mut scored: Vec<(u32, usize)> = idx
                    .iter()
                    .filter_map(|&i| {
                        let mut buf = Vec::new();
                        let hay = nucleo_matcher::Utf32Str::new(&self.bag.notes[i].note.title, &mut buf);
                        pat.score(hay, &mut self.matcher).map(|s| (s, i))
                    })
                    .collect();
                scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
                idx = scored.into_iter().map(|(_, i)| i).collect();
            }
        }
        idx
    }

    // ── tasks ───────────────────────────────────────────────────────

    pub fn bucket(&self) -> Bucket {
        Bucket::ALL[self.w().group.min(Bucket::ALL.len() - 1)]
    }

    /// Count for the left column. Recomputed per draw — at this data size it is
    /// microseconds, and caching it would mean invalidating on every write.
    pub fn bucket_count(&self, b: Bucket) -> usize {
        match b {
            Bucket::Dailies => siddran_core::query::active_dailies(&self.rows.dailies).len(),
            Bucket::Projects => self.rows.projects.len(),
            _ => siddran_core::query::bucket_tasks(&self.rows.tasks, b).len(),
        }
    }

    /// How many rows the current bucket holds, for clamping the selection.
    pub fn item_count(&self) -> usize {
        self.bucket_count(self.bucket())
    }

    /// The project under the cursor, when the Projects bucket is open.
    pub fn selected_project(&self) -> Option<&siddran_core::Row<siddran_core::model::Project>> {
        if self.bucket() != Bucket::Projects {
            return None;
        }
        self.rows
            .projects
            .get(self.w().item.min(self.rows.projects.len().saturating_sub(1)))
    }

    pub fn subtask_count(&self) -> usize {
        self.selected_project().map(|p| p.v.tasks.len()).unwrap_or(0)
    }

    // ── calendar ────────────────────────────────────────────────────

    /// The month on show, defaulting to the one containing today.
    pub fn month(&self) -> (i16, i8) {
        self.w().month.unwrap_or_else(|| {
            let t = deadline::today();
            (t.year(), t.month())
        })
    }

    /// The selected day, defaulting to today when it is in the month on show,
    /// else the 1st — so paging to another month lands somewhere sensible
    /// instead of nowhere.
    pub fn day(&self) -> jiff::civil::Date {
        let (y, m) = self.month();
        self.w().day.unwrap_or_else(|| {
            let t = deadline::today();
            if (t.year(), t.month()) == (y, m) {
                t
            } else {
                jiff::civil::date(y, m, 1)
            }
        })
    }

    pub fn shift_month(&mut self, by: i32) {
        let (y, m) = self.month();
        let (ny, nm, day) = calendar::shift_month(y, m, self.day().day(), by);
        self.wm().month = Some((ny, nm));
        self.wm().day = Some(day);
        self.scroll = 0;
    }

    pub fn set_day(&mut self, d: jiff::civil::Date) {
        self.wm().month = Some((d.year(), d.month()));
        self.wm().day = Some(d);
        // The agenda is a different list on a different day — keeping the old
        // index would leave the cursor pointing at whatever happened to be in
        // that slot, or past the end entirely.
        self.wm().item = 0;
        self.scroll = 0;
    }

    /// Scroll the preview, clamped so it cannot run off the end of the note.
    /// The bound is the unwrapped line count, which is a lower bound on the
    /// wrapped height — erring towards letting you scroll slightly too far
    /// rather than cutting the last lines off.
    pub fn scroll_by(&mut self, delta: i32) {
        let max = self.preview().len().saturating_sub(1) as i32;
        let next = (self.scroll as i32 + delta).clamp(0, max.max(0));
        self.scroll = next as u16;
    }

    pub fn day_items(&self) -> Vec<calendar::Item> {
        calendar::day_items(
            self.day(),
            &self.rows.events,
            &self.rows.tasks,
            &self.rows.dailies,
            &self.rows.completions,
        )
    }

    // ── settings ────────────────────────────────────────────────────

    /// The desktop's `rememberNoteState`, which the terminal honours as
    /// "restore each workspace's position on next launch".
    pub fn remember_state(&self) -> bool {
        self.bag
            .settings_map()
            .and_then(|m| m.get("rememberNoteState"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Capture where each workspace is, for the next launch.
    pub fn snapshot(&self) -> crate::session_state::Remembered {
        crate::session_state::Remembered {
            active: self.ws,
            spaces: self
                .spaces
                .iter()
                .map(|w| crate::session_state::Spot {
                    feature: match w.feature {
                        Feature::Notes => "notes",
                        Feature::Tasks => "tasks",
                        Feature::Calendar => "calendar",
                        Feature::Settings => "settings",
                    }
                    .to_owned(),
                    note: (w.feature == Feature::Notes)
                        .then(|| {
                            let srcs = self.sources();
                            let sel = w.source.min(srcs.len().saturating_sub(1));
                            self.notes_for(&srcs, sel)
                                .get(w.item)
                                .map(|i| self.bag.notes[*i].path.display().to_string())
                        })
                        .flatten(),
                    source: w.source,
                    group: w.group,
                })
                .collect(),
        }
    }

    /// Put each workspace back where it was. A note that has since been renamed
    /// or deleted simply does not match, and that workspace opens at the top —
    /// which is the right outcome, not an error.
    pub fn restore(&mut self, r: &crate::session_state::Remembered) {
        for (i, spot) in r.spaces.iter().take(self.spaces.len()).enumerate() {
            let feature = match spot.feature.as_str() {
                "tasks" => Feature::Tasks,
                "calendar" => Feature::Calendar,
                "settings" => Feature::Settings,
                _ => Feature::Notes,
            };
            self.spaces[i].feature = feature;
            self.spaces[i].group = spot.group;
            self.spaces[i].source = spot.source;
            self.spaces[i].item = 0;

            if let Some(want) = &spot.note {
                let srcs = self.sources();
                let sel = spot.source.min(srcs.len().saturating_sub(1));
                if let Some(pos) = self
                    .notes_for(&srcs, sel)
                    .iter()
                    .position(|n| self.bag.notes[*n].path.display().to_string() == *want)
                {
                    self.spaces[i].item = pos;
                }
            }
        }
        self.ws = r.active.min(self.spaces.len() - 1);
    }

    pub fn section(&self) -> super::settings::Section {
        use super::settings::Section;
        Section::ALL[self.w().group.min(Section::ALL.len() - 1)]
    }

    pub fn entries(&self) -> Vec<super::settings::Entry> {
        use super::settings::Section;
        if self.section() == Section::Terminal {
            super::settings::terminal_entries(
                &self.root,
                &self.editor,
                if self.palette == Palette::Yazi { "yazi" } else { "ember" },
                self.theme.frame.name(),
            )
        } else {
            super::settings::shared_entries(self.bag.settings_map())
        }
    }

    pub fn selected_note(&mut self) -> Option<usize> {
        let n = self.notes();
        n.get(self.w().item.min(n.len().saturating_sub(1))).copied()
    }

    /// Rendered preview for the selection, cached by path.
    pub fn preview(&mut self) -> &[md::Line] {
        let Some(i) = self.selected_note() else {
            self.cache = None;
            return &[];
        };
        let path = self.bag.notes[i].path.clone();
        let hit = matches!(&self.cache, Some((p, _)) if *p == path);
        if !hit {
            let lines = md::render(&self.bag.notes[i].note.body);
            self.cache = Some((path, lines));
        }
        self.cache.as_ref().map(|(_, l)| l.as_slice()).unwrap_or(&[])
    }

    // ── timers ──────────────────────────────────────────────────────

    /// The single coalesced deadline. `None` means block indefinitely.
    pub fn next_deadline(&mut self) -> Option<Instant> {
        if self.midnight.is_none() {
            self.midnight = next_midnight();
        }
        [self.flash_until, self.midnight, self.alarm_at]
            .into_iter()
            .flatten()
            .min()
    }

    /// Sweep the deadlines: ring anything due, arm the next one.
    ///
    /// Called on open and after every change to tasks or dailies, because both
    /// can move a deadline or disarm one.
    pub fn rearm_alarms(&mut self) {
        use siddran_core::logic::alarms;
        let all = alarms::triggers(
            &self.rows.tasks,
            &self.rows.dailies,
            &self.rows.completions,
            self.alarm_hour(),
        );
        let now = jiff::Timestamp::now();
        let (ring, stale) = alarms::partition(all.clone(), now, |k| self.fired.contains(k));

        // Stale ones are marked fired without a sound, so a two-week-old
        // deadline neither screams now nor on the next sweep.
        for t in stale {
            self.fired.insert(t.key);
        }

        // Only the *first* sweep announces what was already due — that is the
        // "you were away, here is what you missed" case, and it is the point of
        // the grace window. Later sweeps run because something was written, and
        // a deadline that was already in the past when you typed it did not
        // just become due: creating a task due today should not set an alarm off
        // in your face. So after the first sweep, only ring what crossed its
        // moment since the last one.
        let ring: Vec<_> = match self.swept_at {
            None => ring,
            Some(since) => {
                let (fresh, already): (Vec<_>, Vec<_>) =
                    ring.into_iter().partition(|t| t.at > since);
                for t in already {
                    self.fired.insert(t.key);
                }
                fresh
            }
        };
        self.swept_at = Some(now);
        if let Some(first) = ring.first() {
            self.ringing = Some(match first.tier {
                alarms::Tier::Reminder => format!("⏰ reminder — {}", first.title),
                alarms::Tier::Alarm => format!("⏰ DUE NOW — {}", first.title),
            });
            self.notify(&first.title, first.tier);
            for t in ring {
                self.fired.insert(t.key);
            }
        }

        self.alarm_at = alarms::next_after(&all, now).map(|at| {
            let secs = (at.as_second() - now.as_second()).max(0) as u64;
            Instant::now() + std::time::Duration::from_secs(secs)
        });
    }

    /// When a date-only deadline rings. Configurable; 09:00 by default, because
    /// the stored time for such a deadline is local midnight and ringing then
    /// helps nobody.
    fn alarm_hour(&self) -> jiff::civil::Time {
        crate::config::get("alarm_time")
            .and_then(|v| {
                let (h, m) = v.split_once(':')?;
                jiff::civil::Time::new(h.parse().ok()?, m.parse().ok()?, 0, 0).ok()
            })
            .unwrap_or(jiff::civil::Time::constant(9, 0, 0, 0))
    }

    /// Hand the notification to whatever the user has. A desktop notification
    /// outlives the terminal being buried behind other windows, which a status
    /// line does not — but shelling out is opt-in, never a surprise.
    fn notify(&self, title: &str, tier: siddran_core::logic::alarms::Tier) {
        use siddran_core::logic::alarms::Tier;
        let Some(cmd) = crate::config::get("notify") else { return };
        if cmd.trim().is_empty() {
            return;
        }
        let head = match tier {
            Tier::Reminder => "Siddran — reminder",
            Tier::Alarm => "Siddran — due now",
        };
        // Spawned, never waited on: a hung notifier must not freeze the UI.
        let _ = std::process::Command::new(&cmd)
            .arg(head)
            .arg(title)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    pub fn on_deadline(&mut self) -> Effect {
        let now = Instant::now();
        let mut redraw = false;
        if matches!(self.flash_until, Some(t) if t <= now) {
            self.flash_until = None;
            if matches!(self.mode, Mode::Flash(..)) {
                self.mode = Mode::Normal;
            }
            redraw = true;
        }
        if matches!(self.midnight, Some(t) if t <= now) {
            self.midnight = next_midnight();
            redraw = true;
        }
        if matches!(self.alarm_at, Some(t) if t <= now) {
            self.rearm_alarms();
            redraw = true;
        }
        if redraw { Effect::Redraw } else { Effect::None }
    }

    pub fn flash(&mut self, msg: impl Into<String>, warn: bool) -> Effect {
        self.mode = Mode::Flash(msg.into(), warn);
        self.flash_until = Some(Instant::now() + std::time::Duration::from_secs(3));
        Effect::Redraw
    }

    pub fn on_key(&mut self, k: crossterm::event::KeyEvent, guard: &Guard, term: &mut Tui) -> Effect {
        super::keymap::handle(self, k, guard, term)
    }

    /// Re-read the Bag and re-stamp it. Called after every write, and by `R`.
    pub fn reload(&mut self) {
        if let Ok(b) = Bag::open(&self.root) {
            self.rows = Rows::of(&b);
            self.bag = b;
            self.cache = None;
            // Without this, the file we just wrote would look like somebody
            // else's change and the next write would be refused.
            self.session.refresh(&self.root, &self.bag);
            // A write can move a deadline or tick something off, so the sweep
            // has to run again.
            self.rearm_alarms();
        }
    }

    /// Accept a change we caused ourselves — see `Session::restamp`.
    pub fn restamp(&mut self, path: &std::path::Path) {
        self.session.restamp(path);
    }

    /// Guard before writing: take the Bag lock (waiting out anyone mid-write),
    /// then make sure nothing moved since we last read. The keypress already
    /// reloaded if it had (`refresh_if_moved`), so a refusal here means a write
    /// landed in the microseconds between — rare, and safe to retry.
    pub fn check_unchanged(&self) -> Result<(), String> {
        if self.write_lock.borrow().is_none() {
            let g = self.session.lock_for_write(&self.root)?;
            *self.write_lock.borrow_mut() = Some(g);
        }
        self.session.check_unchanged()
    }

    /// The key that wrote is done: let the other writers in.
    pub fn end_key(&mut self) {
        self.write_lock.borrow_mut().take();
    }

    /// Someone else — the desktop app, `siddran mcp`, an editor — changed the
    /// Bag since we read it. Reload before acting on a stale picture. Returns
    /// whether it did, so the caller can repaint.
    pub fn refresh_if_moved(&mut self) -> bool {
        if self.session.moved() {
            self.reload();
            true
        } else {
            false
        }
    }

    pub fn set_palette(&mut self, p: Palette) {
        self.palette = p;
        self.theme = Theme::new(p, self.theme.frame);
    }

    pub fn set_frame(&mut self, f: Frame) {
        self.theme = Theme::new(self.palette, f);
    }
}

fn next_midnight() -> Option<Instant> {
    siddran_core::logic::deadline::seconds_to_midnight()
        .map(|s| Instant::now() + std::time::Duration::from_secs(s))
}
