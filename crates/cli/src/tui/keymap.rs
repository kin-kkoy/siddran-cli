//! Key handling. Every branch returns an `Effect`, and only `Redraw` repaints —
//! that gate is what keeps an idle instance at 0% CPU.

use super::app::{App, Feature, Mode, Pending, PromptKind};
use super::term::{Guard, Tui};
use super::{Effect, Frame, Palette};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use siddran_core::bag::{notes, write};
use siddran_core::logic::deadline;
use siddran_core::logic::calendar;
use siddran_core::query::Bucket;

pub fn handle(app: &mut App, k: KeyEvent, guard: &Guard, term: &mut Tui) -> Effect {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);

    // A flash is transient chrome; any key dismisses it and then acts normally.
    if matches!(app.mode, Mode::Flash(..)) {
        app.mode = Mode::Normal;
    }
    // A ringing alarm is dismissed by the first key, which is then swallowed —
    // otherwise the key that silenced it would also do something you did not
    // intend to a note you were not looking at.
    if app.ringing.is_some() {
        app.ringing = None;
        return Effect::Redraw;
    }

    match &mut app.mode {
        // Deleting is the one thing that asks first, and the answer must be an
        // explicit y — any other key cancels, so a stray keypress can never
        // delete anything.
        Mode::Confirm { action, .. } => {
            let action = action.clone();
            app.mode = Mode::Normal;
            return match k.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => do_delete(app, action),
                _ => app.flash("cancelled", false),
            };
        }

        Mode::Help => {
            return match k.code {
                KeyCode::Char('q') | KeyCode::Esc | KeyCode::F(1) => {
                    app.mode = Mode::Normal;
                    Effect::Redraw
                }
                _ => Effect::None,
            }
        }

        Mode::Filter(q) => {
            return match k.code {
                KeyCode::Esc => {
                    app.mode = Mode::Normal;
                    app.wm().filter = None;
                    app.wm().item = 0;
                    Effect::Redraw
                }
                // Confirming *keeps* the filter and moves into the results.
                KeyCode::Enter => {
                    let q = q.clone();
                    app.mode = Mode::Normal;
                    app.wm().filter = (!q.is_empty()).then_some(q);
                    app.wm().item = 0;
                    app.wm().col = 2;
                    Effect::Redraw
                }
                KeyCode::Backspace => {
                    q.pop();
                    app.wm().item = 0;
                    Effect::Redraw
                }
                KeyCode::Char('u') if ctrl => {
                    q.clear();
                    app.wm().item = 0;
                    Effect::Redraw
                }
                KeyCode::Char(c) if !ctrl => {
                    q.push(c);
                    app.wm().item = 0;
                    Effect::Redraw
                }
                _ => Effect::None,
            }
        }

        Mode::Prompt { what, buf, key } => {
            let what = *what;
            let key = key.clone();
            return match k.code {
                KeyCode::Esc => {
                    app.mode = Mode::Normal;
                    app.flash("cancelled", false)
                }
                KeyCode::Backspace => {
                    buf.pop();
                    Effect::Redraw
                }
                // Readline's line-editing keys. A prompt prefilled with the
                // current value is only useful if there is a quick way to
                // replace it — otherwise typing just appends to it.
                KeyCode::Char('u') if ctrl => {
                    buf.clear();
                    Effect::Redraw
                }
                KeyCode::Char('w') if ctrl => {
                    while buf.ends_with(' ') {
                        buf.pop();
                    }
                    while !buf.is_empty() && !buf.ends_with(' ') {
                        buf.pop();
                    }
                    Effect::Redraw
                }
                KeyCode::Char(c) if !ctrl => {
                    buf.push(c);
                    Effect::Redraw
                }
                KeyCode::Enter => {
                    let value = buf.clone();
                    app.mode = Mode::Normal;
                    match what {
                        PromptKind::Setting => commit_setting(app, &key, value.trim()),
                        _ => commit_prompt(app, what, &value),
                    }
                }
                _ => Effect::None,
            };
        }

        Mode::Normal | Mode::Flash(..) => {}
    }

    // `g` is a prefix: `gg` is top, `g`+letter jumps to a feature.
    if app.pending_g {
        app.pending_g = false;
        return match k.code {
            KeyCode::Char('g') => {
                if app.w().col == 1 {
                    app.wm().source = 0;
                    app.wm().group = 0;
                } else {
                    app.wm().item = 0
                }
                app.scroll = 0;
                Effect::Redraw
            }
            KeyCode::Char('n') => set_feature(app, Feature::Notes),
            KeyCode::Char('t') => set_feature(app, Feature::Tasks),
            KeyCode::Char('c') => set_feature(app, Feature::Calendar),
            KeyCode::Char('s') => set_feature(app, Feature::Settings),
            _ => app.flash("g then n / t / c / s for notes, tasks, calendar, settings", true),
        };
    }

    // The month grid is two-dimensional, so in that column the movement keys
    // walk *dates*, not list rows: h/l and the horizontal arrows step a day,
    // j/k and the vertical arrows step a week. `Enter` leaves the grid for the
    // agenda, and `h` there comes back. Treating the grid as a list — which is
    // what the first version did — made it impossible to move sideways at all.
    if app.w().feature == Feature::Calendar {
        let in_grid = app.w().col == 1;
        match k.code {
            // Month, from anywhere. Both cases, so neither needs Shift held.
            KeyCode::Char('<') | KeyCode::Char(',') => {
                app.shift_month(-1);
                return Effect::Redraw;
            }
            KeyCode::Char('>') => {
                app.shift_month(1);
                return Effect::Redraw;
            }
            KeyCode::Char('.') => {
                app.set_day(deadline::today());
                return app.flash("today", false);
            }
            // A day step works anywhere, including from the agenda.
            KeyCode::Char('J') => return step_day(app, 1),
            KeyCode::Char('K') => return step_day(app, -1),

            KeyCode::Char('h') | KeyCode::Left if in_grid => return step_day(app, -1),
            KeyCode::Char('l') | KeyCode::Right if in_grid => return step_day(app, 1),
            KeyCode::Char('j') | KeyCode::Down if in_grid => return step_day(app, 7),
            KeyCode::Char('k') | KeyCode::Up if in_grid => return step_day(app, -7),

            KeyCode::Enter if in_grid => {
                if app.day_items().is_empty() {
                    return app.flash("nothing scheduled that day", true);
                }
                app.wm().col = 2;
                app.wm().item = 0;
                return Effect::Redraw;
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc if !in_grid => {
                app.wm().col = 1;
                return Effect::Redraw;
            }
            // In the agenda, Enter and Space act on the *calendar item* under
            // the cursor. Without this they fell through to the tasks handler,
            // which toggles whatever happens to be selected in the tasks
            // view — pressing Enter here would silently complete an unrelated
            // task.
            KeyCode::Enter | KeyCode::Char(' ') if !in_grid => return toggle_calendar_item(app),
            _ => {}
        }
    }

    match k.code {
        // Workspaces on Alt+1..4. Not Ctrl: classic terminals cannot encode
        // Ctrl with a digit, so Ctrl+1 arrives as a plain `1` and would look
        // like a feature switch. Alt sends ESC+digit, which works everywhere.
        KeyCode::Char(c @ '1'..='4') if alt || ctrl => {
            let i = c as usize - '1' as usize;
            if i == app.ws {
                return app.flash(format!("already on workspace {}", i + 1), false);
            }
            app.ws = i;
            app.scroll = 0;
            app.flash(format!("workspace {} — {}", i + 1, app.w().feature.name().to_lowercase()), false)
        }
        KeyCode::Char('[') => cycle_ws(app, -1),
        KeyCode::Char(']') => cycle_ws(app, 1),

        // Features on the bare digits.
        KeyCode::Char('1') => set_feature(app, Feature::Notes),
        KeyCode::Char('2') => set_feature(app, Feature::Tasks),
        KeyCode::Char('3') => set_feature(app, Feature::Calendar),
        KeyCode::Char('4') => set_feature(app, Feature::Settings),
        KeyCode::Tab => {
            let next = match app.w().feature {
                Feature::Notes => Feature::Tasks,
                Feature::Tasks => Feature::Calendar,
                Feature::Calendar => Feature::Settings,
                Feature::Settings => Feature::Notes,
            };
            set_feature(app, next)
        }

        KeyCode::Char('g') => {
            app.pending_g = true;
            Effect::Redraw
        }
        KeyCode::Char('G') => {
            let notes_view = app.w().feature == Feature::Notes;
            if app.w().col == 1 {
                let n = if notes_view { app.sources().len() } else { Bucket::ALL.len() };
                if notes_view { app.wm().source = n - 1 } else { app.wm().group = n - 1 }
            } else {
                let n = if notes_view { app.notes().len() } else { app.item_count() };
                app.wm().item = n.saturating_sub(1);
            }
            app.scroll = 0;
            Effect::Redraw
        }

        // In a project, the right-hand column is the subtask list, so the
        // movement keys select rather than scroll.
        KeyCode::Char('j') | KeyCode::Down
            if app.w().col == 3 && app.subtask_count() > 0 =>
        {
            let n = app.subtask_count();
            let next = (app.w().sub + 1).min(n - 1);
            app.wm().sub = next;
            Effect::Redraw
        }
        KeyCode::Char('k') | KeyCode::Up if app.w().col == 3 && app.subtask_count() > 0 => {
            let prev = app.w().sub.saturating_sub(1);
            app.wm().sub = prev;
            Effect::Redraw
        }

        // With the cursor in the right-hand column, the note is a document to
        // read, so the movement keys scroll it.
        KeyCode::Char('j') | KeyCode::Down if app.w().col == 3 => {
            app.scroll_by(1);
            Effect::Redraw
        }
        KeyCode::Char('k') | KeyCode::Up if app.w().col == 3 => {
            app.scroll_by(-1);
            Effect::Redraw
        }
        KeyCode::PageDown | KeyCode::Char('d') if app.w().col == 3 => {
            app.scroll_by(15);
            Effect::Redraw
        }
        KeyCode::PageUp | KeyCode::Char('u') if app.w().col == 3 => {
            app.scroll_by(-15);
            Effect::Redraw
        }
        KeyCode::Home if app.w().col == 3 => {
            app.scroll = 0;
            Effect::Redraw
        }

        KeyCode::Char('j') | KeyCode::Down => move_sel(app, 1),
        KeyCode::Char('k') | KeyCode::Up => move_sel(app, -1),
        KeyCode::Char('h') | KeyCode::Left => {
            let c = app.w().col;
            app.wm().col = c.saturating_sub(1).max(1);
            Effect::Redraw
        }
        KeyCode::Char('l') | KeyCode::Right => {
            let c = app.w().col;
            let entering_subtasks = c == 2 && app.bucket() == Bucket::Projects;
            if entering_subtasks && app.subtask_count() == 0 {
                return app.flash("this project has no subtasks", true);
            }
            app.wm().col = (c + 1).min(3);
            if entering_subtasks {
                app.wm().sub = 0;
            }
            Effect::Redraw
        }

        // Scroll the preview without leaving the list.
        KeyCode::Char('e') if ctrl => {
            app.scroll_by(1);
            Effect::Redraw
        }
        KeyCode::Char('y') if ctrl => {
            app.scroll_by(-1);
            Effect::Redraw
        }

        KeyCode::Char('/') => {
            let current = app.w().filter.clone().unwrap_or_default();
            app.mode = Mode::Filter(current);
            app.wm().item = 0;
            Effect::Redraw
        }
        // Esc in normal mode is "show me everything again".
        KeyCode::Esc => {
            if app.w().filter.is_some() {
                app.wm().filter = None;
                app.wm().item = 0;
                return app.flash("filter cleared", false);
            }
            app.mode = Mode::Normal;
            Effect::Redraw
        }

        KeyCode::Enter => match app.w().feature {
            Feature::Notes => open_in_editor(app, guard, term),
            Feature::Settings => edit_setting(app),
            _ => toggle_done(app),
        },

        KeyCode::Char('r') if app.w().feature == Feature::Notes => start_prompt(app, PromptKind::Rename),
        KeyCode::Char('t') if app.w().feature == Feature::Notes => start_prompt(app, PromptKind::Tags),
        KeyCode::Char(' ') => match app.w().feature {
            Feature::Notes => toggle_favourite(app),
            Feature::Settings => edit_setting(app),
            _ => toggle_done(app),
        },
        KeyCode::Char('p') if app.w().feature == Feature::Tasks => cycle_priority(app),

        // Create. What gets made depends on where you are, which is the only
        // sensible reading of "add one of these".
        KeyCode::Char('a') => start_create(app),
        // Delete is `D`, not `d`: `d` pages the preview, and a destructive
        // action earns the deliberate shift.
        KeyCode::Char('D') => start_delete(app),

        KeyCode::Char('R') => {
            app.reload();
            app.flash("reloaded from disk", false)
        }
        KeyCode::Char('P') => {
            let p = if app.palette == Palette::Ember { Palette::Yazi } else { Palette::Ember };
            app.set_palette(p);
            app.flash(if p == Palette::Ember { "palette: siddran ember" } else { "palette: yazi" }, false)
        }
        KeyCode::Char('F') => {
            let f = match app.theme.frame {
                Frame::Rounded => Frame::Square,
                Frame::Square => Frame::Flat,
                Frame::Flat => Frame::Rounded,
            };
            app.set_frame(f);
            let _ = crate::config::set("frame", f.name());
            app.flash(format!("frame: {}", f.name()), false)
        }

        KeyCode::Char('?') | KeyCode::F(1) => {
            app.mode = Mode::Help;
            Effect::Redraw
        }
        KeyCode::Char('q') => Effect::Quit,
        _ => Effect::None,
    }
}

/// Tick off the calendar item under the cursor. Events are not completable —
/// a calendar block is a span of time, not a to-do.
fn toggle_calendar_item(app: &mut App) -> Effect {
    let items = app.day_items();
    let Some(it) = items.get(app.w().item.min(items.len().saturating_sub(1))) else {
        return app.flash("nothing selected", true);
    };
    match it.kind {
        calendar::Kind::Event => app.flash("a calendar block has nothing to complete", true),
        calendar::Kind::Task => {
            let id = it.id.clone();
            let mut tasks = app.rows.tasks.clone();
            let Some(i) = tasks.iter().position(|r| r.v.id.to_string() == id) else {
                return app.flash("task not found", true);
            };
            let done = !tasks[i].v.is_completed;
            let title = tasks[i].v.title.clone();
            tasks[i].set_completed(done);
            match save_tasks(app, &tasks) {
                Ok(()) => {
                    app.reload();
                    app.flash(format!("{} {title}", if done { "✓" } else { "☐" }), false)
                }
                Err(e) => app.flash(e, true),
            }
        }
        calendar::Kind::Daily => {
            // Completions are per *day*, and the day in question is the one
            // being looked at — not today.
            let (id, title, day) = (it.id.clone(), it.title.clone(), app.day().to_string());
            let mut completions = app.rows.completions.clone();
            let existing = completions
                .iter()
                .position(|c| c.v.daily_task_id.to_string() == id && c.v.date == day);
            let done = existing.is_none();
            match existing {
                Some(k) => {
                    completions.remove(k);
                }
                None => {
                    let row = serde_json::json!({ "daily_task_id": id, "date": day });
                    match siddran_core::Row::from_value(&row) {
                        Some(r) => completions.push(r),
                        None => return app.flash("could not build the completion row", true),
                    }
                }
            }
            if let Err(e) = guard_write(app) {
                return app.flash(e, true);
            }
            let res = write::tasks_file(
                &app.root.join("tasks.siddran"),
                &app.rows.tasks,
                &app.rows.dailies,
                &completions,
                &app.rows.projects,
            );
            match res {
                Ok(()) => {
                    app.reload();
                    app.flash(format!("{} {title} for {day}", if done { "✓" } else { "☐" }), false)
                }
                Err(e) => app.flash(format!("{e}"), true),
            }
        }
    }
}

/// Move the calendar selection by `days`, following the month along with it.
fn step_day(app: &mut App, days: i32) -> Effect {
    let d = calendar::shift_day(app.day(), days);
    app.set_day(d);
    Effect::Redraw
}

fn cycle_ws(app: &mut App, d: i32) -> Effect {
    app.ws = ((app.ws as i32 + d).rem_euclid(4)) as usize;
    app.scroll = 0;
    app.flash(format!("workspace {} — {}", app.ws + 1, app.w().feature.name().to_lowercase()), false)
}

fn set_feature(app: &mut App, f: Feature) -> Effect {
    if app.w().feature == f {
        return Effect::None;
    }
    app.wm().feature = f;
    app.wm().col = app.w().col.min(3);
    app.scroll = 0;
    Effect::Redraw
}

fn move_sel(app: &mut App, d: i32) -> Effect {
    let col = app.w().col;
    let notes_view = app.w().feature == Feature::Notes;

    if col == 1 {
        let n = match app.w().feature {
            Feature::Notes => app.sources().len(),
            Feature::Settings => super::settings::Section::ALL.len(),
            _ => Bucket::ALL.len(),
        } as i32;
        if app.w().feature == Feature::Calendar {
            return Effect::None; // handled above, by date
        }
        let cur = if notes_view { app.w().source } else { app.w().group } as i32;
        let next = (cur + d).clamp(0, (n - 1).max(0)) as usize;
        if next as i32 == cur {
            return Effect::None;
        }
        if notes_view {
            app.wm().source = next;
        } else {
            app.wm().group = next;
        }
        app.wm().item = 0;
    } else {
        let n = match app.w().feature {
            Feature::Notes => app.notes().len(),
            Feature::Tasks => app.item_count(),
            Feature::Calendar => app.day_items().len(),
            Feature::Settings => app.entries().len(),
        } as i32;
        let cur = app.w().item as i32;
        let next = (cur + d).clamp(0, (n - 1).max(0)) as usize;
        if next == app.w().item {
            return Effect::None;
        }
        app.wm().item = next;
        // A different project has a different subtask list; carrying the old
        // index over would leave the cursor on an unrelated row.
        app.wm().sub = 0;
    }
    app.scroll = 0;
    Effect::Redraw
}

fn start_prompt(app: &mut App, what: PromptKind) -> Effect {
    let Some(i) = app.selected_note() else {
        return app.flash("nothing selected", true);
    };
    let buf = match what {
        PromptKind::Rename => app.bag.notes[i].note.title.clone(),
        PromptKind::Tags => app.bag.notes[i].note.tags.clone(),
        // These open their prompts elsewhere, with an empty buffer.
        _ => return Effect::None,
    };
    app.mode = Mode::Prompt { what, buf, key: String::new() };
    Effect::Redraw
}

// ── settings ────────────────────────────────────────────────────────

/// Cycle a choice, or open a prompt for free text. Read-only keys say why.
fn edit_setting(app: &mut App) -> Effect {
    use super::settings::Kind;
    let entries = app.entries();
    let Some(e) = entries.get(app.w().item.min(entries.len().saturating_sub(1))) else {
        return app.flash("nothing selected", true);
    };
    let (key, value, kind) = (e.key.clone(), e.value.clone(), e.kind);

    match kind {
        Kind::ReadOnly => app.flash(
            if app.section() == super::settings::Section::Terminal {
                "shown for reference — not editable here"
            } else {
                "read-only: this belongs to the desktop app"
            },
            true,
        ),
        Kind::Choice(opts) => {
            let i = opts.iter().position(|o| *o == value).unwrap_or(0);
            let next = opts[(i + 1) % opts.len()];
            apply_setting(app, &key, next);
            match crate::config::set(&key, next) {
                Ok(_) => app.flash(format!("{key} = {next}   (saved)"), false),
                Err(e) => app.flash(format!("changed, but not saved: {e}"), true),
            }
        }
        Kind::Text => {
            app.mode = Mode::Prompt {
                what: PromptKind::Setting,
                buf: value,
                key,
            };
            Effect::Redraw
        }
    }
}

fn commit_setting(app: &mut App, key: &str, value: &str) -> Effect {
    if value.is_empty() {
        return app.flash("a setting cannot be blank", true);
    }
    apply_setting(app, key, value);
    match crate::config::set(key, value) {
        Ok(p) => app.flash(format!("{key} = {value}   (saved to {})", p.display()), false),
        Err(e) => app.flash(format!("changed, but not saved: {e}"), true),
    }
}

/// Apply a setting to the running session. `bag` is the exception: reopening a
/// different Bag mid-session would mean dropping the lock and re-reading
/// everything, so it takes effect on the next start.
fn apply_setting(app: &mut App, key: &str, value: &str) {
    match key {
        "palette" => app.set_palette(if value == "yazi" { Palette::Yazi } else { Palette::Ember }),
        "frame" => app.set_frame(Frame::parse(value)),
        "editor" => app.editor = value.to_owned(),
        // Both change when alarms ring, so the schedule has to be recomputed.
        "alarm_time" | "notify" => app.rearm_alarms(),
        _ => {}
    }
}

fn commit_prompt(app: &mut App, what: PromptKind, value: &str) -> Effect {
    let Some(i) = app.selected_note() else {
        return app.flash("nothing selected", true);
    };
    let nf = &app.bag.notes[i];
    let mut note = nf.note.clone();
    note.updated = deadline::now_iso();
    let (from, notebook) = (nf.path.clone(), nf.notebook.clone());

    let res = match what {
        PromptKind::NewNote => return create_note(app, value),
        PromptKind::NewNotebook => return create_notebook(app, value),
        PromptKind::NewTask => return create_task(app, value),
        PromptKind::NewDaily => return create_daily(app, value),
        PromptKind::NewProject => return create_project(app, value),
        PromptKind::NewSubtask => return create_subtask(app, value),
        PromptKind::Setting => return Effect::None,
        PromptKind::Rename => {
            if value.trim().is_empty() {
                return app.flash("a title cannot be empty", true);
            }
            note.title = value.trim().to_owned();
            let to = notes::target_path(&app.bag, note.id.as_deref(), &note.title, notebook.as_deref());
            notes::save_moving(&note, &from, &to, &app.root).map(|_| ())
        }
        PromptKind::Tags => {
            note.tags = value.trim().to_owned();
            notes::save(&note, &from)
        }
    };

    match res {
        Ok(()) => {
            app.reload();
            app.flash(
                match what {
                    PromptKind::Rename => "renamed — the file moved too",
                    _ => "tags written to frontmatter",
                },
                false,
            )
        }
        Err(e) => app.flash(format!("{e}"), true),
    }
}

fn toggle_favourite(app: &mut App) -> Effect {
    let Some(i) = app.selected_note() else {
        return app.flash("nothing selected", true);
    };
    let mut note = app.bag.notes[i].note.clone();
    note.favorite = !note.favorite;
    note.updated = deadline::now_iso();
    let path = app.bag.notes[i].path.clone();
    match notes::save(&note, &path) {
        Ok(()) => {
            let on = note.favorite;
            app.reload();
            app.flash(if on { "★ favourited" } else { "○ unfavourited" }, false)
        }
        Err(e) => app.flash(format!("{e}"), true),
    }
}

/// Hand the note body to the configured editor.
///
/// The TUI steps out of the alternate screen entirely: once the editor starts it
/// owns the terminal, and nothing can be drawn over it. That is why the hint
/// lives in the status line *before* launch rather than as an overlay during.
fn open_in_editor(app: &mut App, guard: &Guard, term: &mut Tui) -> Effect {
    let Some(i) = app.selected_note() else {
        return app.flash("nothing selected", true);
    };
    let path = app.bag.notes[i].path.clone();
    let notebook = app.bag.notes[i].notebook.clone();
    let editor = app.editor.clone();

    // Remember the file as it stands, so we can tell whether the editor
    // actually changed anything.
    let before = std::fs::read_to_string(&path).unwrap_or_default();

    let status = guard.suspended(term, || {
        std::process::Command::new(&editor).arg(&path).status()
    });

    match status {
        Ok(Ok(_)) => {
            let after = std::fs::read_to_string(&path).unwrap_or_default();
            // The editor's write is ours — we launched it — so the staleness
            // guard must not read it as somebody else's.
            app.restamp(&path);
            if after != before {
                if let Err(e) = touch_after_edit(app, &path, notebook.as_deref(), &after) {
                    app.reload();
                    return app.flash(e, true);
                }
            }
            // The preview cache is keyed by path, so a reload is what refreshes it.
            app.reload();
            Effect::Redraw
        }
        Ok(Err(e)) => app.flash(format!("could not run {editor:?}: {e}"), true),
        Err(e) => app.flash(format!("terminal: {e}"), true),
    }
}

/// Bring a note's metadata back in line after an external edit.
///
/// The editor writes the file directly and knows nothing about frontmatter, so
/// after it exits two things can be wrong:
///
/// 1. `updated` still says whatever it said before, even though the body
///    changed — and the desktop sorts notes by that field, so the note quietly
///    sinks down a list it should have risen to the top of.
/// 2. If the title line was edited, the filename no longer matches the title.
///    The desktop renames the file on its next flush; leaving the two
///    disagreeing until then is worse than doing it now.
///
/// Both are the same operation: re-parse what the editor left, stamp it, and
/// save it where its title says it belongs.
fn touch_after_edit(
    app: &mut App,
    path: &std::path::Path,
    notebook: Option<&str>,
    contents: &str,
) -> Result<(), String> {
    let fallback = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled");
    let mut note = siddran_core::Note::from_md(contents, fallback);
    note.updated = deadline::now_iso();

    guard_write(app)?;
    let to = notes::target_path(&app.bag, note.id.as_deref(), &note.title, notebook);
    if to == path {
        notes::save(&note, path).map_err(|e| e.to_string())
    } else {
        notes::save_moving(&note, path, &to, &app.root).map_err(|e| e.to_string())
    }
}


// ── task writes ─────────────────────────────────────────────────────

/// Toggle whatever is selected: a task, today's tick on a daily, or a project's
/// first open subtask.
fn toggle_done(app: &mut App) -> Effect {
    let bucket = app.bucket();
    let i = app.w().item;

    let res = match bucket {
        Bucket::Dailies => toggle_daily(app, i),
        Bucket::Projects => {
            // In the project list, step into the subtasks; in the subtask list,
            // tick the one under the cursor.
            if app.w().col < 3 {
                if app.subtask_count() == 0 {
                    return app.flash("this project has no subtasks", true);
                }
                app.wm().col = 3;
                app.wm().sub = 0;
                return Effect::Redraw;
            }
            toggle_subtask(app)
        }
        _ => toggle_task(app, bucket, i),
    };

    match res {
        Ok(msg) => {
            app.reload();
            app.flash(msg, false)
        }
        Err(e) => app.flash(e, true),
    }
}

fn toggle_subtask(app: &mut App) -> Result<String, String> {
    let (pid, sid, title, now_done) = {
        let p = app.selected_project().ok_or("no project selected")?;
        let st = p
            .v
            .tasks
            .get(app.w().sub.min(p.v.tasks.len().saturating_sub(1)))
            .ok_or("no subtask selected")?;
        (p.v.id.clone(), st.id.clone(), st.title.clone(), !st.is_completed)
    };

    let mut projects = app.rows.projects.clone();
    let pi = projects.iter().position(|r| r.v.id == pid).ok_or("project vanished")?;
    projects[pi].set_subtask_completed(&sid, now_done);
    // The project's stored priority is derived from its subtasks, so it moves
    // with them — `set_subtask_completed` recomputes it.
    let bucket = siddran_core::logic::priority::bucket(&projects[pi].v.tasks);

    guard_write(app)?;
    write::tasks_file(
        &app.root.join("tasks.siddran"),
        &app.rows.tasks,
        &app.rows.dailies,
        &app.rows.completions,
        &projects,
    )
    .map_err(|e| e.to_string())?;
    Ok(format!(
        "{} {title}   (project priority is now {bucket})",
        if now_done { "✓" } else { "☐" }
    ))
}

fn toggle_task(app: &mut App, bucket: Bucket, i: usize) -> Result<String, String> {
    let mut tasks = app.bag.tasks();
    let target = {
        let sel = siddran_core::query::bucket_tasks(&tasks, bucket);
        let r = sel.get(i.min(sel.len().saturating_sub(1))).ok_or("nothing selected")?;
        r.v.id.clone()
    };
    let idx = tasks.iter().position(|r| r.v.id == target).ok_or("task vanished")?;
    let now_done = !tasks[idx].v.is_completed;
    let title = tasks[idx].v.title.clone();
    tasks[idx].set_completed(now_done);

    save_tasks(app, &tasks)?;
    Ok(format!("{} {title}", if now_done { "✓" } else { "☐" }))
}

fn toggle_daily(app: &mut App, i: usize) -> Result<String, String> {
    let dailies = app.bag.dailies();
    let act = siddran_core::query::active_dailies(&dailies);
    let d = act.get(i.min(act.len().saturating_sub(1))).ok_or("nothing selected")?;

    // Only recurring dailies take per-day completions, matching the desktop.
    if d.v.recurrence.is_none() {
        return Err(format!("{:?} is a one-off, not recurring", d.v.title));
    }
    let (id, title) = (d.v.id.clone(), d.v.title.clone());
    let today = deadline::today().to_string();

    let mut completions = app.bag.completions();
    let existing = completions
        .iter()
        .position(|c| c.v.daily_task_id == id && c.v.date == today);
    let now_done = existing.is_none();
    match existing {
        Some(k) => {
            completions.remove(k);
        }
        None => {
            let row = serde_json::json!({ "daily_task_id": id, "date": today });
            completions.push(siddran_core::Row::from_value(&row).ok_or("bad row")?);
        }
    }

    let bag = &app.bag;
    guard_write(app)?;
    write::tasks_file(
        &app.root.join("tasks.siddran"),
        &bag.tasks(),
        &dailies,
        &completions,
        &bag.projects(),
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("{} {title} for {today}", if now_done { "✓" } else { "☐" }))
}

/// low -> normal -> high -> low
fn cycle_priority(app: &mut App) -> Effect {
    let bucket = app.bucket();
    if !bucket.is_tasks() {
        return app.flash("priority cycling applies to tasks", true);
    }
    let i = app.w().item;
    let mut tasks = app.bag.tasks();
    let target = {
        let sel = siddran_core::query::bucket_tasks(&tasks, bucket);
        match sel.get(i.min(sel.len().saturating_sub(1))) {
            Some(r) => r.v.id.clone(),
            None => return app.flash("nothing selected", true),
        }
    };
    let Some(idx) = tasks.iter().position(|r| r.v.id == target) else {
        return app.flash("task vanished", true);
    };
    let next = match tasks[idx].v.priority.as_str() {
        "low" => "normal",
        "normal" => "high",
        _ => "low",
    };
    tasks[idx].set_priority(next);
    match save_tasks(app, &tasks) {
        Ok(()) => {
            app.reload();
            app.flash(format!("priority: {next}"), false)
        }
        Err(e) => app.flash(e, true),
    }
}

fn save_tasks(app: &App, tasks: &[siddran_core::Row<siddran_core::model::Task>]) -> Result<(), String> {
    guard_write(app)?;
    let bag = &app.bag;
    write::tasks_file(
        &app.root.join("tasks.siddran"),
        tasks,
        &bag.dailies(),
        &bag.completions(),
        &bag.projects(),
    )
    .map_err(|e| e.to_string())
}

/// Take the Bag lock for this keypress and check nothing moved under us. The
/// event loop releases the lock when the key is done (`App::end_key`).
fn guard_write(app: &App) -> Result<(), String> {
    app.check_unchanged()
}


// ── create ──────────────────────────────────────────────────────────

fn prompt(app: &mut App, what: PromptKind) -> Effect {
    app.mode = Mode::Prompt { what, buf: String::new(), key: String::new() };
    Effect::Redraw
}

/// `a` means "add one of whatever this pane holds".
fn start_create(app: &mut App) -> Effect {
    match app.w().feature {
        Feature::Notes => {
            // In the sources column you are pointing at notebooks, so that is
            // what gets made.
            if app.w().col == 1 {
                prompt(app, PromptKind::NewNotebook)
            } else {
                prompt(app, PromptKind::NewNote)
            }
        }
        Feature::Tasks => match app.bucket() {
            Bucket::Dailies => prompt(app, PromptKind::NewDaily),
            Bucket::Projects => {
                if app.w().col == 3 {
                    prompt(app, PromptKind::NewSubtask)
                } else {
                    prompt(app, PromptKind::NewProject)
                }
            }
            _ => prompt(app, PromptKind::NewTask),
        },
        Feature::Calendar => app.flash("events are created in the desktop app", true),
        Feature::Settings => app.flash("nothing to add here", true),
    }
}

fn create_note(app: &mut App, title: &str) -> Effect {
    let title = title.trim();
    if title.is_empty() {
        return app.flash("a note needs a title", true);
    }
    // A new note lands in the notebook you were looking at — anything else means
    // filing it twice.
    let srcs = app.sources();
    let notebook = match srcs.get(app.w().source) {
        Some(super::app::Source::Notebook(n)) => Some(n.clone()),
        _ => None,
    };

    let id = notes::next_note_id(&app.bag).to_string();
    let now = deadline::now_iso();
    let note = siddran_core::Note {
        id: Some(id.clone()),
        title: title.to_owned(),
        tags: String::new(),
        favorite: false,
        color: String::new(),
        order: None,
        created: now.clone(),
        updated: now,
        body: String::new(),
        had_frontmatter: true,
    };
    let path = notes::target_path(&app.bag, Some(&id), title, notebook.as_deref());
    if path.exists() {
        return app.flash(format!("{} already exists", path.display()), true);
    }
    if let Err(e) = guard_write(app) {
        return app.flash(e, true);
    }
    match notes::save(&note, &path) {
        Ok(()) => {
            app.reload();
            select_note(app, &path);
            app.flash(format!("created {title}"), false)
        }
        Err(e) => app.flash(format!("{e}"), true),
    }
}

/// Put the cursor on a note we just made, so it is not left pointing at
/// whatever happened to shift into its old slot.
fn select_note(app: &mut App, path: &std::path::Path) {
    let srcs = app.sources();
    let sel = app.w().source.min(srcs.len().saturating_sub(1));
    if let Some(pos) = app
        .notes_for(&srcs, sel)
        .iter()
        .position(|i| app.bag.notes[*i].path == path)
    {
        app.wm().item = pos;
        app.wm().col = 2;
    }
}

fn create_notebook(app: &mut App, name: &str) -> Effect {
    let name = name.trim();
    if name.is_empty() {
        return app.flash("a notebook needs a name", true);
    }
    let folder = app.root.join("notes").join(siddran_core::bag::layout::safe_name(name));
    if folder.exists() {
        return app.flash("a notebook by that name already exists", true);
    }
    if let Err(e) = guard_write(app) {
        return app.flash(e, true);
    }
    if let Err(e) = std::fs::create_dir_all(&folder) {
        return app.flash(format!("{}: {e}", folder.display()), true);
    }
    // The sidecar carries the id and colour the desktop expects; without it the
    // folder is still a notebook, but with an id invented fresh on every load.
    let now = deadline::now_iso();
    let meta = serde_json::json!({
        "id": notes::next_note_id(&app.bag),
        "color": serde_json::Value::Null,
        "tags": "",
        "is_favorite": false,
        "order": serde_json::Value::Null,
        "created_at": now,
        "updated_at": now,
    });
    let map = meta.as_object().cloned().unwrap_or_default();
    match write::notebook_meta(&folder.join(".siddran-notebook.json"), &map) {
        Ok(()) => {
            app.reload();
            app.flash(format!("created notebook {name}"), false)
        }
        Err(e) => app.flash(format!("{e}"), true),
    }
}

fn create_task(app: &mut App, title: &str) -> Effect {
    let title = title.trim();
    if title.is_empty() {
        return app.flash("a task needs a title", true);
    }
    let mut tasks = app.rows.tasks.clone();
    let mut row = siddran_core::Row::<siddran_core::model::Task>::new_task(&siddran_core::ids::uuid_v4(), title, "normal");
    // Added from the Today list, it is due today — that is what the list means.
    if app.bucket() == Bucket::Today {
        if let Some((iso, all_day)) = deadline::from_picker_value("today") {
            row.set_due(Some(&iso), all_day);
        }
    }
    tasks.push(row);
    match save_tasks(app, &tasks) {
        Ok(()) => {
            app.reload();
            app.flash(format!("added {title}"), false)
        }
        Err(e) => app.flash(e, true),
    }
}

fn create_daily(app: &mut App, title: &str) -> Effect {
    let title = title.trim();
    if title.is_empty() {
        return app.flash("a daily needs a title", true);
    }
    // Recurring every day by default: a daily with no recurrence expires tonight,
    // which is almost never what someone typing into the Dailies list wants.
    let row = siddran_core::Row::<siddran_core::model::Daily>::new_daily(&siddran_core::ids::uuid_v4(), title, Some("every-day"));
    let mut dailies = app.rows.dailies.clone();
    dailies.push(row);

    if let Err(e) = guard_write(app) {
        return app.flash(e, true);
    }
    let res = write::tasks_file(
        &app.root.join("tasks.siddran"),
        &app.rows.tasks,
        &dailies,
        &app.rows.completions,
        &app.rows.projects,
    );
    match res {
        Ok(()) => {
            app.reload();
            app.flash(format!("added {title} — repeats every day"), false)
        }
        Err(e) => app.flash(format!("{e}"), true),
    }
}

fn create_project(app: &mut App, title: &str) -> Effect {
    let title = title.trim();
    if title.is_empty() {
        return app.flash("a project needs a title", true);
    }
    let mut projects = app.rows.projects.clone();
    projects.push(siddran_core::Row::<siddran_core::model::Project>::new_project(&siddran_core::ids::uuid_v4(), title));
    save_projects(app, &projects, format!("added {title}"))
}

fn create_subtask(app: &mut App, title: &str) -> Effect {
    let title = title.trim();
    if title.is_empty() {
        return app.flash("a subtask needs a title", true);
    }
    let Some(pid) = app.selected_project().map(|p| p.v.id.clone()) else {
        return app.flash("no project selected", true);
    };
    let mut projects = app.rows.projects.clone();
    let Some(i) = projects.iter().position(|p| p.v.id == pid) else {
        return app.flash("project vanished", true);
    };
    projects[i].add_subtask(&siddran_core::ids::uuid_v4(), title);
    let last = projects[i].v.tasks.len() - 1;
    let out = save_projects(app, &projects, format!("added {title}"));
    app.wm().sub = last;
    out
}

fn save_projects(app: &mut App, projects: &[siddran_core::Row<siddran_core::model::Project>], msg: String) -> Effect {
    if let Err(e) = guard_write(app) {
        return app.flash(e, true);
    }
    let res = write::tasks_file(
        &app.root.join("tasks.siddran"),
        &app.rows.tasks,
        &app.rows.dailies,
        &app.rows.completions,
        projects,
    );
    match res {
        Ok(()) => {
            app.reload();
            app.flash(msg, false)
        }
        Err(e) => app.flash(format!("{e}"), true),
    }
}

// ── delete ──────────────────────────────────────────────────────────

fn start_delete(app: &mut App) -> Effect {
    let (question, action) = match app.w().feature {
        Feature::Notes => {
            let Some(i) = app.selected_note() else {
                return app.flash("nothing selected", true);
            };
            let nf = &app.bag.notes[i];
            (
                format!("delete \"{}\"?", nf.note.title),
                Pending::DeleteNote(nf.path.clone()),
            )
        }
        Feature::Tasks => match app.bucket() {
            Bucket::Dailies => {
                let act = siddran_core::query::active_dailies(&app.rows.dailies);
                let Some(d) = act.get(app.w().item.min(act.len().saturating_sub(1))) else {
                    return app.flash("nothing selected", true);
                };
                (format!("delete daily \"{}\"?", d.v.title), Pending::DeleteDaily(d.v.id.clone()))
            }
            Bucket::Projects => {
                if app.w().col == 3 {
                    let Some(p) = app.selected_project() else {
                        return app.flash("nothing selected", true);
                    };
                    let Some(st) = p.v.tasks.get(app.w().sub.min(p.v.tasks.len().saturating_sub(1)))
                    else {
                        return app.flash("nothing selected", true);
                    };
                    (
                        format!("delete subtask \"{}\"?", st.title),
                        Pending::DeleteSubtask(p.v.id.clone(), st.id.clone()),
                    )
                } else {
                    let Some(p) = app.selected_project() else {
                        return app.flash("nothing selected", true);
                    };
                    (
                        format!("delete project \"{}\" and its {} subtask(s)?", p.v.title, p.v.tasks.len()),
                        Pending::DeleteProject(p.v.id.clone()),
                    )
                }
            }
            b => {
                let sel = siddran_core::query::bucket_tasks(&app.rows.tasks, b);
                let Some(r) = sel.get(app.w().item.min(sel.len().saturating_sub(1))) else {
                    return app.flash("nothing selected", true);
                };
                (format!("delete \"{}\"?", r.v.title), Pending::DeleteTask(r.v.id.clone()))
            }
        },
        Feature::Calendar => return app.flash("events are deleted in the desktop app", true),
        Feature::Settings => return app.flash("nothing to delete here", true),
    };

    app.mode = Mode::Confirm { question, action };
    Effect::Redraw
}

fn do_delete(app: &mut App, action: Pending) -> Effect {
    if let Err(e) = guard_write(app) {
        return app.flash(e, true);
    }
    match action {
        Pending::DeleteNote(path) => {
            let Some(nf) = app.bag.notes.iter().find(|n| n.path == path).cloned() else {
                return app.flash("note vanished", true);
            };
            // Trashed, not unlinked — recoverable from .siddran-trash/.
            match notes::delete(&nf, &app.root) {
                Ok(_) => {
                    app.reload();
                    let n = app.notes().len();
                    app.wm().item = app.w().item.min(n.saturating_sub(1));
                    app.flash(format!("trashed {}", nf.note.title), false)
                }
                Err(e) => app.flash(format!("{e}"), true),
            }
        }
        Pending::DeleteTask(id) => {
            let mut tasks = app.rows.tasks.clone();
            tasks.retain(|r| r.v.id != id);
            match save_tasks(app, &tasks) {
                Ok(()) => {
                    app.reload();
                    clamp_item(app);
                    app.flash("deleted", false)
                }
                Err(e) => app.flash(e, true),
            }
        }
        Pending::DeleteDaily(id) => {
            let mut dailies = app.rows.dailies.clone();
            dailies.retain(|r| r.v.id != id);
            // Its completions go too, or they linger as rows pointing at nothing.
            let mut completions = app.rows.completions.clone();
            completions.retain(|c| c.v.daily_task_id != id);
            let res = write::tasks_file(
                &app.root.join("tasks.siddran"),
                &app.rows.tasks,
                &dailies,
                &completions,
                &app.rows.projects,
            );
            match res {
                Ok(()) => {
                    app.reload();
                    clamp_item(app);
                    app.flash("deleted", false)
                }
                Err(e) => app.flash(format!("{e}"), true),
            }
        }
        Pending::DeleteProject(id) => {
            let mut projects = app.rows.projects.clone();
            projects.retain(|p| p.v.id != id);
            let out = save_projects(app, &projects, "deleted".into());
            clamp_item(app);
            out
        }
        Pending::DeleteSubtask(pid, sid) => {
            let mut projects = app.rows.projects.clone();
            let Some(i) = projects.iter().position(|p| p.v.id == pid) else {
                return app.flash("project vanished", true);
            };
            projects[i].remove_subtask(&sid);
            let n = projects[i].v.tasks.len();
            let out = save_projects(app, &projects, "deleted".into());
            app.wm().sub = app.w().sub.min(n.saturating_sub(1));
            out
        }
    }
}

/// After a delete the list is shorter, so the cursor has to come back in range.
fn clamp_item(app: &mut App) {
    let n = app.item_count();
    app.wm().item = app.w().item.min(n.saturating_sub(1));
}
