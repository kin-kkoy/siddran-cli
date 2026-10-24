//! Drawing. Pure: it reads state and paints, and never mutates anything that
//! would require another paint.

use super::app::{App, Feature, Mode, PromptKind, Source};
use super::settings;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame as TFrame;
use siddran_core::logic::{calendar, deadline, priority, recurrence};
use siddran_core::md;
use siddran_core::query::Bucket;

pub fn draw(f: &mut TFrame, app: &mut App) {
    let area = f.area();
    // No background fill: the terminal's own backdrop shows through.

    let boxed = app.theme.frame.boxed();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),                       // breadcrumb + workspace badges
            Constraint::Min(3),                          // the panes
            Constraint::Length(1),                       // status
        ])
        .margin(if boxed { 1 } else { 0 })
        .split(area);

    crumb(f, app, rows[0]);
    panes(f, app, rows[1]);
    status(f, app, rows[2]);

    if matches!(app.mode, Mode::Help) {
        help(f, app, area);
    }
}

// ── the path line, with workspace badges on the right ───────────────

fn crumb(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = app.theme_copy();
    let mut left = vec![Span::styled(
        format!("{}", app.root.display()),
        Style::default().fg(t.dimmer),
    )];

    if app.w().feature == Feature::Notes {
        let srcs = app.sources();
        if let Some(Source::Notebook(n)) = srcs.get(app.w().source) {
            left.push(Span::styled("/", Style::default().fg(t.dimmer)));
            left.push(Span::styled(n.clone(), Style::default().fg(t.dim)));
        }
        if let Some(i) = app.selected_note() {
            left.push(Span::styled("/", Style::default().fg(t.dimmer)));
            left.push(Span::styled(
                format!("{}.md", app.bag.notes[i].note.title),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
        }
    } else if app.w().feature == Feature::Calendar {
        left.push(Span::styled("/calendar.siddran", Style::default().fg(t.dim)));
        left.push(Span::styled("/", Style::default().fg(t.dimmer)));
        left.push(Span::styled(
            app.day().to_string(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    } else if app.w().feature == Feature::Settings {
        // Terminal settings live in our config; desktop ones in the Bag. The
        // breadcrumb says which file you are looking at, because that is the
        // whole distinction the view is trying to make.
        let sec = app.section();
        left = vec![Span::styled(
            if sec == settings::Section::Terminal {
                crate::config::path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "settings".into())
            } else {
                format!("{}/settings.siddran", app.root.display())
            },
            Style::default().fg(t.dimmer),
        )];
        left.push(Span::styled("/", Style::default().fg(t.dimmer)));
        left.push(Span::styled(
            sec.name().to_lowercase(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    } else {
        left.push(Span::styled("/tasks.siddran", Style::default().fg(t.dim)));
        left.push(Span::styled("/", Style::default().fg(t.dimmer)));
        left.push(Span::styled(
            app.bucket().name().to_lowercase(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }

    // Badges: the active one names what it holds, so a glance says both where
    // you are and what is parked elsewhere.
    let mut right = Vec::new();
    for (i, w) in app.spaces.iter().enumerate() {
        let on = i == app.ws;
        let label = if on {
            format!(" {} {} ", i + 1, w.feature.name().to_lowercase())
        } else {
            format!(" {} ", i + 1)
        };
        right.push(Span::styled(
            label,
            if on {
                Style::default().fg(t.on_accent).bg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.dimmer)
            },
        ));
        right.push(Span::raw(" "));
    }

    let rw: usize = right.iter().map(|s| s.content.chars().count()).sum();
    let total = area.width as usize;

    // Elide the breadcrumb from the LEFT. The tail — the notebook, the note,
    // the date — is the part that changes and the part worth reading; letting
    // the head of a long path push it off the end hides exactly what matters.
    let avail = total.saturating_sub(rw + 1);
    let lw: usize = left.iter().map(|s| s.content.chars().count()).sum();
    if lw > avail && avail > 2 {
        let mut drop = lw - avail + 1;
        while drop > 0 && !left.is_empty() {
            let n = left[0].content.chars().count();
            if n <= drop {
                drop -= n;
                left.remove(0);
            } else {
                let kept: String = left[0].content.chars().skip(drop).collect();
                left[0] = Span::styled(kept, left[0].style);
                drop = 0;
            }
        }
        left.insert(0, Span::styled("…", Style::default().fg(t.dimmer)));
    }

    f.render_widget(Paragraph::new(Line::from(left)), area);
    if rw < total {
        let ra = Rect { x: area.x + (total - rw) as u16, width: rw as u16, ..area };
        f.render_widget(Paragraph::new(Line::from(right)), ra);
    }
}

// ── the three columns ───────────────────────────────────────────────

fn panes(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = &app.theme;
    let boxed = t.frame.boxed();

    let inner = if boxed {
        let b = Block::default()
            .borders(Borders::ALL)
            .border_type(t.frame.border_type())
            .border_style(Style::default().fg(t.line));
        let i = b.inner(area);
        f.render_widget(b, area);
        i
    } else {
        area
    };

    // Each view wants a different split: tasks carry a checkbox, glyph, title
    // and due chip; the calendar's month grid needs a fixed ~24 cells for seven
    // day columns and cannot usefully shrink.
    let widths = match app.w().feature {
        Feature::Notes => [24, 32],
        Feature::Tasks => [17, 45],
        // The month grid is the point of the calendar and the thing you aim at,
        // so it gets the room; the detail pane is the least-read of the three.
        Feature::Calendar => [50, 30],
        // Keys are short; the explanation beside them is the long part.
        Feature::Settings => [18, 34],
    };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(widths[0]),
            Constraint::Percentage(widths[1]),
            Constraint::Min(10),
        ])
        .split(inner);

    let focus = app.w().col;
    let a = focus_frame(f, app, cols[0], focus == 1, true);
    let b = focus_frame(f, app, cols[1], focus == 2, false);
    let c = focus_frame(f, app, cols[2], focus == 3, false);

    // Join each divider to the box with a tee, so the frame reads as one piece
    // rather than a rectangle with three loose lines inside it.
    if boxed {
        for x in [cols[1].x, cols[2].x] {
            for (y, glyph) in [(area.y, "┬"), (area.y + area.height - 1, "┴")] {
                f.render_widget(
                    Paragraph::new(Line::styled(glyph, Style::default().fg(app.theme.line))),
                    Rect { x, y, width: 1, height: 1 },
                );
            }
        }
    }

    match app.w().feature {
        Feature::Notes => {
            sources_col(f, app, a);
            notes_col(f, app, b);
            preview_col(f, app, c);
        }
        Feature::Tasks => tasks_view(f, app, a, b, c),
        Feature::Calendar => calendar_view(f, app, a, b, c),
        Feature::Settings => settings_view(f, app, a, b, c),
    }

}

/// Carves a one-cell gutter off the left of a column and draws the focus
/// indicator in it, returning the rest for content.
///
/// The gutter doubles as the column divider: unfocused it is the separator
/// line, focused it is a solid accent bar. They were two separate things at
/// first, and the divider — drawn later — painted straight over the indicator.
///
/// Without this there is no way to tell which column has the cursor when that
/// column is empty or is the preview, since neither has a selection highlight.
/// Being unable to find the cursor is the most disorienting thing a multi-pane
/// TUI can do.
fn focus_frame(f: &mut TFrame, app: &App, area: Rect, focused: bool, first: bool) -> Rect {
    let t = &app.theme;
    let bar = Rect { width: 1, ..area };
    let (glyph, style) = match (focused, first) {
        (true, _) => ("▐", Style::default().fg(t.accent)),
        (false, true) => (" ", Style::default().fg(t.line)),
        (false, false) => ("│", Style::default().fg(t.line)),
    };
    f.render_widget(
        Paragraph::new(vec![Line::styled(glyph, style); area.height as usize]),
        bar,
    );
    Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area }
}

fn row_style(app: &App, selected: bool, active: bool) -> Style {
    let t = &app.theme;
    match (selected, active) {
        (true, true) => Style::default().fg(t.on_accent).bg(t.accent).add_modifier(Modifier::BOLD),
        (true, false) => Style::default().fg(t.fg).bg(t.sel),
        _ => Style::default().fg(t.dim),
    }
}

fn sources_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let active = app.w().col == 1;
    let sel = app.w().source;
    let srcs = app.sources();
    let labels: Vec<(String, usize)> = srcs.iter().map(|s| app.source_label(s)).collect();

    let lines: Vec<Line> = labels
        .iter()
        .enumerate()
        .skip(scroll_to(sel, area.height as usize, labels.len()))
        .take(area.height as usize)
        .map(|(i, (name, count))| {
            let pad = area.width as usize;
            let c = count.to_string();
            let room = pad.saturating_sub(c.len() + 3);
            Line::styled(
                format!(" {:<room$} {} ", trunc(name, room), c),
                row_style(app, i == sel, active),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

fn notes_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let active = app.w().col >= 2;
    let idx = app.notes();
    let sel = app.w().item.min(idx.len().saturating_sub(1));
    let t_accent = app.theme.accent;

    if idx.is_empty() {
        let msg = if matches!(app.mode, Mode::Filter(_)) { "no matches" } else { "no notes here" };
        f.render_widget(Paragraph::new(empty_marker(app, msg, active)), area);
        return;
    }

    let lines: Vec<Line> = idx
        .iter()
        .enumerate()
        .skip(scroll_to(sel, area.height as usize, idx.len()))
        .take(area.height as usize)
        .map(|(i, &n)| {
            let note = &app.bag.notes[n].note;
            let star = if note.favorite { "★" } else { " " };
            let width = area.width as usize;
            let room = width.saturating_sub(5);
            let base = row_style(app, i == sel, active);
            Line::from(vec![
                Span::styled(format!(" {star} "), if note.favorite && !(i == sel && active) {
                    base.fg(t_accent)
                } else {
                    base
                }),
                Span::styled(format!("{:<room$} ", trunc(&note.title, room)), base),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

fn preview_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let focused = app.w().col == 3;
    let scroll = app.scroll;
    let theme_fg = app.theme.fg;
    let width = area.width.saturating_sub(1) as usize;
    let lines = {
        let t = app.theme_copy();
        app.preview()
            .iter()
            .flat_map(|l| render_line(l, &t, width))
            .collect::<Vec<Line>>()
    };
    if lines.is_empty() {
        f.render_widget(
            Paragraph::new(empty_marker(app, "(empty note)", focused)),
            area,
        );
        return;
    }

    // The note is a document, so it gets a scrollbar when it does not fit —
    // otherwise there is no way to tell there is more below.
    let total = lines.len();
    let height = area.height as usize;
    let text = Rect { width: area.width.saturating_sub(1), ..area };
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).style(Style::default().fg(theme_fg)),
        text,
    );

    if total > height {
        let t = &app.theme;
        let bar = Rect { x: area.x + area.width - 1, width: 1, ..area };
        let span = ((height * height) / total).max(1);
        let max_scroll = total.saturating_sub(height);
        let top = (scroll as usize * (height - span)).checked_div(max_scroll).unwrap_or(0);
        let glyphs: Vec<Line> = (0..height)
            .map(|i| {
                if i >= top && i < top + span {
                    Line::styled("│", Style::default().fg(if focused { t.accent } else { t.dim }))
                } else {
                    Line::styled("│", Style::default().fg(t.dimmer))
                }
            })
            .collect();
        f.render_widget(Paragraph::new(glyphs), bar);
    }
}

fn tasks_view(f: &mut TFrame, app: &mut App, a: Rect, b: Rect, c: Rect) {
    buckets_col(f, app, a);
    items_col(f, app, b);
    detail_col(f, app, c);
}

fn buckets_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let active = app.w().col == 1;
    let sel = app.w().group;
    let lines: Vec<Line> = Bucket::ALL
        .iter()
        .enumerate()
        .map(|(i, b)| {
            let n = app.bucket_count(*b);
            let width = area.width as usize;
            let cnt = n.to_string();
            let room = width.saturating_sub(cnt.len() + 3);
            Line::styled(
                format!(" {:<room$} {} ", trunc(b.name(), room), cnt),
                row_style(app, i == sel, active),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

fn items_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let active = app.w().col >= 2;
    let bucket = app.bucket();
    let t = app.theme_copy();
    let width = area.width as usize;
    let today = deadline::today().to_string();

    let lines: Vec<Line> = match bucket {
        Bucket::Dailies => {
            let active_d = siddran_core::query::active_dailies(&app.rows.dailies);
            let completions = &app.rows.completions;
            let sel = app.w().item.min(active_d.len().saturating_sub(1));
            visible(sel, area.height as usize, active_d.len())
                .map(|i| {
                    let d = active_d[i];
                    let done = siddran_core::query::completed_on(completions, &d.v.id, &today);
                    let rule = recurrence::parse(d.v.recurrence.as_deref());
                    // A recurrence is far easier to read as lit days than as
                    // `{"mask":[…]}`, which is what is actually on disk.
                    let days = rule.as_ref().map(recurrence::label).unwrap_or_else(|| "once".into());
                    // `·` is multi-byte, so pad by character count — `len()`
                    // here misaligns every masked recurrence by two cells.
                    let dw = days.chars().count();
                    let room = width.saturating_sub(dw + 6);
                    let base = row_style(app, i == sel, active);
                    Line::from(vec![
                        Span::styled(format!(" {} ", if done { "✓" } else { "☐" }), base),
                        Span::styled(pad(&trunc(&d.v.title, room), room), base),
                        Span::styled(format!(" {days} "), base),
                    ])
                })
                .collect()
        }
        Bucket::Projects => {
            let projects = &app.rows.projects;
            let sel = app.w().item.min(projects.len().saturating_sub(1));
            visible(sel, area.height as usize, projects.len())
                .map(|i| {
                    let p = &projects[i];
                    let (done, total) = priority::progress(&p.v.tasks);
                    let bucket = bucket_label(priority::bucket(&p.v.tasks));
                    let prog = format!("{done}/{total}");
                    let tail = prog.chars().count() + bucket.chars().count() + 2;
                    let room = width.saturating_sub(tail + 5);
                    let base = row_style(app, i == sel, active);
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", if p.v.is_completed { "✓" } else { "☐" }),
                            base,
                        ),
                        Span::styled(pad(&trunc(&p.v.title, room), room), base),
                        Span::styled(
                            format!(" {bucket} "),
                            if i == sel && active {
                                base
                            } else {
                                base.fg(prio_colour(priority::bucket(&p.v.tasks), &t))
                            },
                        ),
                        Span::styled(format!("{prog} "), base),
                    ])
                })
                .collect()
        }
        _ => {
            let sel_rows = siddran_core::query::bucket_tasks(&app.rows.tasks, bucket);
            let sel = app.w().item.min(sel_rows.len().saturating_sub(1));
            visible(sel, area.height as usize, sel_rows.len())
                .map(|i| {
                    let r = sel_rows[i];
                    let due = r
                        .v
                        .due_date
                        .as_deref()
                        .map(|d| deadline::human(d, r.v.due_all_day))
                        .unwrap_or_default();
                    let overdue = r
                        .v
                        .due_date
                        .as_deref()
                        .is_some_and(|d| deadline::is_overdue(d, r.v.is_completed));
                    let prio = prio_label(&r.v.priority);
                    let base = row_style(app, i == sel, active);
                    let selected = i == sel && active;
                    // Pad by characters, not bytes: `len()` counts UTF-8 bytes
                    // and misaligns every column holding an em-dash or CJK.
                    let tail = due.chars().count() + prio.chars().count() + 1;
                    let room = width.saturating_sub(tail + 5);
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", if r.v.is_completed { "✓" } else { "☐" }),
                            base,
                        ),
                        Span::styled(format!("{} ", pad(&trunc(&r.v.title, room), room)), base),
                        Span::styled(
                            format!("{prio} "),
                            if selected { base } else { base.fg(prio_colour(&r.v.priority, &t)) },
                        ),
                        Span::styled(
                            format!("{due} "),
                            if overdue && !selected { base.fg(t.flame) } else { base },
                        ),
                    ])
                })
                .collect()
        }
    };

    if lines.is_empty() {
        let msg = format!("nothing in {}", bucket.name().to_lowercase());
        f.render_widget(Paragraph::new(empty_marker(app, &msg, active)), area);
        return;
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn detail_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = app.theme_copy();
    let bucket = app.bucket();
    let mut lines: Vec<Line> = Vec::new();
    let kv = |k: &str, v: String, t: &ThemeCopy| {
        Line::from(vec![
            Span::styled(format!(" {k:<10}"), Style::default().fg(t.dim)),
            Span::styled(v, Style::default().fg(t.fg)),
        ])
    };

    match bucket {
        Bucket::Dailies => {
            let act = siddran_core::query::active_dailies(&app.rows.dailies);
            let Some(d) = act.get(app.w().item.min(act.len().saturating_sub(1))) else {
                return;
            };
            let completions = &app.rows.completions;
            lines.push(Line::styled(
                format!(" {}", d.v.title),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));
            let rule = recurrence::parse(d.v.recurrence.as_deref());
            lines.push(kv(
                "repeats",
                rule.as_ref().map(recurrence::label).unwrap_or_else(|| "once, then expires".into()),
                &t,
            ));
            if let Some(time) = &d.v.time {
                lines.push(kv("at", time.clone(), &t));
            }
            lines.push(kv("priority", bucket_label(&d.v.priority).to_owned(), &t));
            lines.push(Line::raw(""));
            lines.push(Line::styled(" last 28 days", Style::default().fg(t.dim)));

            // The heatmap distinguishes three states, not two: a weekday-only
            // habit must not look like it was missed every weekend.
            let hist = siddran_core::query::daily_history(completions, d, 28);
            let mut row: Vec<Span> = vec![Span::raw(" ")];
            for (i, (_, done, fires)) in hist.iter().enumerate() {
                if i > 0 && i % 7 == 0 {
                    lines.push(Line::from(std::mem::take(&mut row)));
                    row.push(Span::raw(" "));
                }
                row.push(match (done, fires) {
                    (true, _) => Span::styled("■ ", Style::default().fg(t.teal)),
                    (false, true) => Span::styled("□ ", Style::default().fg(t.dim)),
                    (false, false) => Span::styled("· ", Style::default().fg(t.dimmer)),
                });
            }
            if !row.is_empty() {
                lines.push(Line::from(row));
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(" ■", Style::default().fg(t.teal)),
                Span::styled(" done   ", Style::default().fg(t.dimmer)),
                Span::styled("□", Style::default().fg(t.dim)),
                Span::styled(" due, not done   ", Style::default().fg(t.dimmer)),
                Span::styled("·", Style::default().fg(t.dimmer)),
                Span::styled(" not due", Style::default().fg(t.dimmer)),
            ]));
        }

        Bucket::Projects => {
            let projects = &app.rows.projects;
            let Some(p) = projects.get(app.w().item.min(projects.len().saturating_sub(1))) else {
                return;
            };
            let (done, total) = priority::progress(&p.v.tasks);
            lines.push(Line::styled(
                format!(" {}", p.v.title),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));
            // Derived, never the stored field — that one goes stale.
            lines.push(kv(
                "priority",
                format!("{} — averaged from the open subtasks", bucket_label(priority::bucket(&p.v.tasks))),
                &t,
            ));
            lines.push(kv("progress", progress_bar(done, total, 14), &t));
            lines.push(Line::raw(""));

            // Subtasks are a real list with their own cursor, so they can be
            // ticked off here. `l` from the project moves focus into it.
            let active = app.w().col == 3;
            let sel = app.w().sub.min(p.v.tasks.len().saturating_sub(1));
            for (i, st) in p.v.tasks.iter().enumerate() {
                let base = row_style(app, i == sel && active, active);
                let fg = if i == sel && active {
                    base
                } else if st.is_completed {
                    Style::default().fg(t.dimmer)
                } else {
                    Style::default().fg(t.fg)
                };
                let prio = prio_label(&st.priority);
                let room = (area.width as usize).saturating_sub(prio.chars().count() + 6);
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", if st.is_completed { "✓" } else { "☐" }), fg),
                    Span::styled(pad(&trunc(&st.title, room), room), fg),
                    Span::styled(
                        format!(" {prio}"),
                        if i == sel && active { fg } else { fg.fg(prio_colour(&st.priority, &t)) },
                    ),
                ]));
            }
            if p.v.tasks.is_empty() {
                lines.push(Line::styled(" (no subtasks)", Style::default().fg(t.dimmer)));
            }
        }

        _ => {
            let rows = siddran_core::query::bucket_tasks(&app.rows.tasks, bucket);
            let Some(r) = rows.get(app.w().item.min(rows.len().saturating_sub(1))) else {
                return;
            };
            lines.push(Line::styled(
                format!(" {}", r.v.title),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::raw(""));
            lines.push(kv(
                "status",
                if r.v.is_completed { "complete".into() } else { "open".into() },
                &t,
            ));
            lines.push(kv("priority", bucket_label(&r.v.priority).to_owned(), &t));
            match r.v.due_date.as_deref() {
                Some(d) => {
                    lines.push(kv("due", deadline::human(d, r.v.due_all_day), &t));
                    // Shown explicitly because the *absence* of this flag is
                    // meaningful on a legacy row — it is what makes the deadline
                    // date-only. Hiding it would hide why the row reads as it does.
                    lines.push(kv(
                        "all day",
                        match r.v.due_all_day {
                            Some(b) => b.to_string(),
                            None => format!("absent — legacy, reads as {}", deadline::is_all_day(d, None)),
                        },
                        &t,
                    ));
                }
                None => lines.push(kv("due", "—".into(), &t)),
            }
            if let Some(rm) = &r.v.remind_at {
                lines.push(kv("remind", rm.clone(), &t));
            }
            if let Some(d) = r.v.description.as_deref().filter(|d| !d.is_empty()) {
                lines.push(Line::raw(""));
                for l in d.lines() {
                    lines.push(Line::styled(format!(" {l}"), Style::default().fg(t.fg)));
                }
            }
        }
    }

    f.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
}

// ── calendar ────────────────────────────────────────────────────────

fn calendar_view(f: &mut TFrame, app: &mut App, a: Rect, b: Rect, c: Rect) {
    month_col(f, app, a);
    agenda_col(f, app, b);
    day_detail_col(f, app, c);
}

/// The month grid, sized to whatever column it is given.
///
/// Cells grow with the column rather than sitting at a fixed ~3 cells and
/// leaving the rest blank — widening the pane should widen the calendar, which
/// is the whole reason for giving it the space. Once a cell has room it carries
/// how much is on that day, not just that *something* is.
fn month_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = app.theme_copy();
    let (y, m) = app.month();
    let sel = app.day();
    let today = deadline::today();
    let active = app.w().col == 1;

    // One pass for the whole grid, not one per cell — see `month_counts`.
    let counts = calendar::month_counts(y, m, &app.rows.events, &app.rows.tasks);

    let usable = area.width.saturating_sub(1) as usize;
    let cell = (usable / 7).clamp(3, 12);
    let pad_left = 1;

    const NAMES: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
    // Fixed sub-fields, not centring: the day number right-aligned in 2 and the
    // activity marker left-aligned in whatever is left. Centring each cell on
    // its own content made the columns jagged, because "1" and "30 ···" are
    // different widths.
    let marker_w = cell.saturating_sub(3);
    let cell_text = |day: String, mark: &str| -> String {
        format!("{day:>2} {mark:<marker_w$}")
    };

    let mut lines = vec![
        Line::styled(
            format!(" {} {}", calendar::MONTHS[(m - 1) as usize], y),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(
            std::iter::once(Span::raw(" ".repeat(pad_left)))
                .chain(NAMES.iter().map(|n| {
                    Span::styled(cell_text((*n).to_owned(), ""), Style::default().fg(t.dimmer))
                }))
                .collect::<Vec<_>>(),
        ),
    ];

    let grid = calendar::month_grid(y, m);
    for (w, week) in grid.chunks(7).enumerate() {
        let mut spans = vec![Span::raw(" ".repeat(pad_left))];
        for (c, d) in week.iter().enumerate() {
            let (n, owed) = counts[w * 7 + c];
            let in_month = d.month() == m;
            let is_sel = *d == sel;

            // Dots say "how busy" at a glance without being read as a number;
            // a `+` past three keeps the field a fixed width. Below ~6 cells
            // there is only room for the day itself.
            let mark = if cell < 6 {
                String::new()
            } else {
                match n {
                    0 => String::new(),
                    1..=3 => "·".repeat(n),
                    _ => "···+".to_owned(),
                }
            };
            let body = cell_text(d.day().to_string(), &mark);
            // Colour carries "you owe something here" without costing a column.
            let mark_colour = if owed { t.flame } else { t.teal };

            let style = if is_sel && active {
                Style::default().fg(t.on_accent).bg(t.accent).add_modifier(Modifier::BOLD)
            } else if is_sel {
                Style::default().fg(t.fg).bg(t.sel)
            } else if *d == today {
                // Today stays legible even when the selection is elsewhere.
                Style::default().fg(t.flame).add_modifier(Modifier::BOLD)
            } else if !in_month {
                Style::default().fg(t.dimmer)
            } else if n > 0 {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.dim)
            };

            // The day number and its marker are styled separately so the marker
            // can carry the deadline colour while the number keeps its own.
            if mark.is_empty() || is_sel {
                spans.push(Span::styled(body, style));
            } else {
                let (num, rest) = body.split_at(3);
                spans.push(Span::styled(num.to_owned(), style));
                spans.push(Span::styled(rest.to_owned(), Style::default().fg(mark_colour)));
            }
        }
        lines.push(Line::from(spans));
        // A blank row between weeks once there is vertical room to spare.
        if cell >= 6 && area.height as usize > 20 {
            lines.push(Line::raw(""));
        }
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(" · ", Style::default().fg(t.teal)),
        Span::styled("blocks   ", Style::default().fg(t.dimmer)),
        Span::styled("· ", Style::default().fg(t.flame)),
        Span::styled("a deadline you owe   ", Style::default().fg(t.dimmer)),
        Span::styled("(dailies are in the agenda)", Style::default().fg(t.dimmer)),
    ]));
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " h j k l  day / week      < >  month      .  today      Enter  agenda",
        Style::default().fg(t.dimmer),
    ));
    f.render_widget(Paragraph::new(lines), area);
}

fn agenda_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = app.theme_copy();
    let active = app.w().col >= 2;
    let items = app.day_items();
    let day = app.day();
    let width = area.width as usize;

    let mut lines = vec![Line::styled(
        format!(" {}", day.strftime("%a %-d %B")),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )];

    if items.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::styled("  nothing scheduled", Style::default().fg(t.dimmer)));
        f.render_widget(Paragraph::new(lines), area);
        return;
    }

    let sel = app.w().item.min(items.len() - 1);
    for (i, it) in items.iter().enumerate() {
        let base = row_style(app, i == sel, active);
        // A left bar coloured by source is how the desktop separates the three
        // kinds at a glance; the same rule applies here.
        let bar_colour = match it.kind {
            calendar::Kind::Event => t.accent,
            calendar::Kind::Task => t.violet,
            calendar::Kind::Daily => t.teal,
        };
        let time = it.time.clone().unwrap_or_else(|| "all day".into());
        let room = width.saturating_sub(time.chars().count() + 5);
        lines.push(Line::from(vec![
            Span::styled("▌", if i == sel && active { base } else { base.fg(bar_colour) }),
            Span::styled(format!("{time:>7} "), base.fg(if i == sel && active { t.on_accent } else { t.dim })),
            Span::styled(
                pad(&trunc(&it.title, room), room),
                if it.done { base.fg(t.dimmer).add_modifier(Modifier::CROSSED_OUT) } else { base },
            ),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

/// The detail pane for the selected agenda item.
///
/// Its job is to show what the agenda column *cannot*: the description, the
/// schedule a block was stamped from, and what a block is linked to — resolved
/// to a real title rather than printed as `ref_type: note`. All of it was
/// already on disk and being discarded, which is what made this column feel
/// like it was repeating the one to its left.
fn day_detail_col(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = app.theme_copy();
    let items = app.day_items();
    let focused = app.w().col == 3;
    if items.is_empty() {
        return;
    }
    let it = items[app.w().item.min(items.len() - 1)].clone();
    let width = area.width.saturating_sub(1) as usize;

    let kv = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!(" {k:<10}"), Style::default().fg(t.dim)),
            Span::styled(v, Style::default().fg(t.fg)),
        ])
    };

    let accent = match it.kind {
        calendar::Kind::Event => t.accent,
        calendar::Kind::Task => t.violet,
        calendar::Kind::Daily => t.teal,
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("▌ ", Style::default().fg(accent)),
            Span::styled(it.title.clone(), Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        ]),
        Line::raw(""),
        kv(
            "kind",
            match it.kind {
                calendar::Kind::Event => "calendar block".into(),
                calendar::Kind::Task => "task deadline".into(),
                calendar::Kind::Daily => "recurring daily".into(),
            },
        ),
        kv(
            "when",
            match (&it.time, &it.end) {
                (Some(s), Some(e)) => format!("{s}–{e}"),
                (Some(s), None) => s.clone(),
                _ => "all day".into(),
            },
        ),
    ];

    // Which schedule stamped this block. A block that belongs to a repeating
    // pattern is a different thing from one you placed by hand, and the
    // difference is otherwise invisible.
    if let Some(sid) = &it.schedule_id {
        let name = app
            .rows
            .schedules
            .iter()
            .find(|s| s.v.id.to_string() == *sid)
            .map(|s| s.v.name.clone())
            .unwrap_or_else(|| format!("schedule {sid}"));
        lines.push(kv("from", name));
    }

    match it.kind {
        calendar::Kind::Task => {
            if let Some(r) = app.rows.tasks.iter().find(|r| r.v.id.to_string() == it.id) {
                lines.push(kv("priority", bucket_label(&r.v.priority).to_owned()));
                lines.push(kv("status", if r.v.is_completed { "done".into() } else { "open".into() }));
                if let Some(rm) = &r.v.remind_at {
                    lines.push(kv("remind", deadline::human(rm, Some(false))));
                }
            }
        }
        calendar::Kind::Daily => {
            if let Some(d) = app.rows.dailies.iter().find(|r| r.v.id.to_string() == it.id) {
                let rule = recurrence::parse(d.v.recurrence.as_deref());
                lines.push(kv(
                    "repeats",
                    rule.as_ref().map(recurrence::label).unwrap_or_else(|| "once".into()),
                ));
                lines.push(kv("done", if it.done { "yes".into() } else { "no".into() }));
                // The same streak the tasks view shows, so the two agree.
                let hist = siddran_core::query::daily_history(&app.rows.completions, d, 14);
                let mut row = vec![Span::styled(" last 14   ", Style::default().fg(t.dim))];
                for (_, done, fires) in &hist {
                    row.push(match (done, fires) {
                        (true, _) => Span::styled("■", Style::default().fg(t.teal)),
                        (false, true) => Span::styled("□", Style::default().fg(t.dim)),
                        (false, false) => Span::styled("·", Style::default().fg(t.dimmer)),
                    });
                }
                lines.push(Line::from(row));
            }
        }
        calendar::Kind::Event => {}
    }

    // What the block points at, resolved to something you can read.
    if let Some(link) = linked_label(app, &it) {
        lines.push(kv("linked to", link));
    }

    if let Some(desc) = &it.description {
        lines.push(Line::raw(""));
        for para in desc.lines() {
            lines.extend(wrap_line(
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(para.to_owned(), Style::default().fg(t.fg)),
                ]),
                1,
                width,
            ));
        }
    }

    let _ = focused;
    f.render_widget(Paragraph::new(lines).scroll((app.scroll, 0)), area);
}

/// `ref_type`/`ref_id` are polymorphic and unconstrained — resolve them to the
/// referenced row's title, and say plainly when the target is gone.
fn linked_label(app: &App, it: &calendar::Item) -> Option<String> {
    let ev = app
        .rows
        .events
        .iter()
        .find(|e| e.v.id.to_string() == it.id && it.kind == calendar::Kind::Event)?;
    let (kind, id) = (ev.v.ref_type.as_deref()?, ev.v.ref_id.as_deref()?);

    let title = match kind {
        "note" => app
            .bag
            .notes
            .iter()
            .find(|n| n.note.id.as_deref() == Some(id))
            .map(|n| n.note.title.clone()),
        "task" => app
            .rows
            .tasks
            .iter()
            .find(|r| r.v.id.to_string() == id)
            .map(|r| r.v.title.clone()),
        "daily" => app
            .rows
            .dailies
            .iter()
            .find(|r| r.v.id.to_string() == id)
            .map(|r| r.v.title.clone()),
        "project" | "bundle" => app
            .rows
            .projects
            .iter()
            .find(|r| r.v.id.to_string() == id)
            .map(|r| r.v.title.clone()),
        // Canvas boards are out of scope, so say so rather than pretending.
        "sandbox" => Some("(a sandbox — not shown in the terminal)".into()),
        _ => None,
    };
    Some(match title {
        Some(t) => format!("{kind}: {t}"),
        None => format!("{kind}: {id} (missing)"),
    })
}

// ── settings ────────────────────────────────────────────────────────

fn settings_view(f: &mut TFrame, app: &mut App, a: Rect, b: Rect, c: Rect) {
    let t = app.theme_copy();
    let section = app.section();
    let entries = app.entries();

    // Sections
    let active = app.w().col == 1;
    let sel_sec = app.w().group;
    let lines: Vec<Line> = settings::Section::ALL
        .iter()
        .enumerate()
        .map(|(i, sec)| {
            let n = if *sec == settings::Section::Terminal {
                settings::terminal_entries(&app.root, &app.editor, "", "").len()
            } else {
                settings::shared_entries(app.bag.settings_map()).len()
            };
            let width = a.width as usize;
            let cnt = n.to_string();
            let room = width.saturating_sub(cnt.len() + 3);
            Line::styled(
                format!(" {:<room$} {} ", trunc(sec.name(), room), cnt),
                row_style(app, i == sel_sec, active),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(lines), a);

    // Keys
    let active = app.w().col >= 2;
    if entries.is_empty() {
        f.render_widget(Paragraph::new(empty_marker(app, "nothing here", active)), b);
        return;
    }
    let sel = app.w().item.min(entries.len() - 1);
    let width = b.width as usize;
    let rows: Vec<Line> = entries
        .iter()
        .enumerate()
        .skip(scroll_to(sel, b.height as usize, entries.len()))
        .take(b.height as usize)
        .map(|(i, e)| {
            let base = row_style(app, i == sel, active);
            // Mark the editable rows rather than the read-only ones: five of
            // the twenty-odd keys are ours, so marking those is the smaller
            // amount of ink, and it answers "can I change this?" before you try.
            let editable = e.kind != settings::Kind::ReadOnly;
            let mark = if editable { "›" } else { " " };
            let val = trunc(&e.value, width.saturating_sub(20));
            let room = width.saturating_sub(val.chars().count() + 6);
            Line::from(vec![
                Span::styled(
                    format!(" {mark} "),
                    if i == sel && active { base } else { base.fg(t.teal) },
                ),
                Span::styled(pad(&trunc(&e.key, room), room), base),
                Span::styled(
                    format!("{val} "),
                    if i == sel && active { base } else { base.fg(t.dim) },
                ),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(rows), b);

    // Detail
    let e = &entries[sel];
    let mut det = vec![
        Line::styled(
            format!(" {}", e.key),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled(" value      ", Style::default().fg(t.dim)),
            Span::styled(e.value.clone(), Style::default().fg(t.fg)),
        ]),
    ];
    match e.kind {
        settings::Kind::Choice(opts) => {
            det.push(Line::from(vec![
                Span::styled(" choices    ", Style::default().fg(t.dim)),
                Span::styled(opts.join(" · "), Style::default().fg(t.fg)),
            ]));
            det.push(Line::raw(""));
            det.push(Line::styled(
                " Space or Enter cycles it, and saves.",
                Style::default().fg(t.teal),
            ));
        }
        settings::Kind::Text => {
            det.push(Line::raw(""));
            det.push(Line::styled(
                " Enter to change it, and save.",
                Style::default().fg(t.teal),
            ));
        }
        settings::Kind::ReadOnly => {
            det.push(Line::raw(""));
            det.push(Line::styled(" read-only", Style::default().fg(t.dimmer)));
        }
    }
    det.push(Line::raw(""));
    for l in wrap_line(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(e.help.clone(), Style::default().fg(t.dim)),
        ]),
        1,
        c.width.saturating_sub(1) as usize,
    ) {
        det.push(l);
    }

    if section != settings::Section::Terminal {
        det.push(Line::raw(""));
        det.push(Line::styled(
            " Source: settings.siddran",
            Style::default().fg(t.dimmer),
        ));
    }
    f.render_widget(Paragraph::new(det).scroll((app.scroll, 0)), c);
}

/// `▓▓▓▓▓░░░░░  5/8` — a count alone makes you do the arithmetic.
fn progress_bar(done: usize, total: usize, width: usize) -> String {
    if total == 0 {
        return "0/0".into();
    }
    let filled = (done * width) / total;
    format!(
        "{}{}  {done}/{total}",
        "▓".repeat(filled),
        "░".repeat(width - filled)
    )
}

/// Priority in words.
///
/// This was `▁ ▄ █` — three shaded blocks that nobody could be expected to
/// decode, and which did not even say which way round they went. `normal` is
/// the default and shows nothing at all: a marker on almost every row carries
/// no information, it is just noise to read past.
fn prio_label(p: &str) -> &'static str {
    match p {
        "high" => "high",
        "low" => "low",
        _ => "",
    }
}

/// The seven derived project buckets, readable. Same words, minus the
/// snake_case — they mean "the average of the incomplete subtasks", so
/// "quite high" is the honest phrasing rather than something invented.
fn bucket_label(p: &str) -> &'static str {
    match p {
        "very_low" => "very low",
        "quite_low" => "quite low",
        "low" => "low",
        "high" => "high",
        "quite_high" => "quite high",
        "very_high" => "very high",
        _ => "normal",
    }
}

fn prio_colour(p: &str, t: &ThemeCopy) -> ratatui::style::Color {
    match p {
        "high" | "very_high" | "quite_high" => t.flame,
        "low" | "very_low" | "quite_low" => t.dim,
        _ => t.dimmer,
    }
}

/// The window of indices that fit, keeping the selection on screen.
fn visible(sel: usize, height: usize, total: usize) -> std::ops::Range<usize> {
    let start = scroll_to(sel, height, total);
    start..(start + height).min(total)
}

/// Style one markdown line, wrapping it to `width` and returning one or more
/// terminal lines.
///
/// Wrapping is done here rather than with `Paragraph::wrap` so a continuation
/// line keeps its block's indent — an unwrapped bullet that runs on flush to
/// the left margin stops looking like a bullet.
fn render_line(l: &md::Line, t: &ThemeCopy, width: usize) -> Vec<Line<'static>> {
    let styled = style_line(l, t);
    // Continuations line up under the *text*, not under the marker — a bullet
    // whose second line runs flush left stops looking like a bullet.
    let hang = match l.kind {
        md::Kind::ListItem(_) | md::Kind::Task(..) => 3,
        md::Kind::Quote | md::Kind::Callout | md::Kind::Heading(_) => 3,
        md::Kind::Code | md::Kind::CodeFence => 3,
        // A plain line renders as " " + text, so it hangs at 2.
        _ => 2,
    };
    wrap_line(styled, hang, width)
}

fn style_line(l: &md::Line, t: &ThemeCopy) -> Line<'static> {
    let base = Style::default().fg(t.fg);
    let mut spans: Vec<Span> = Vec::new();

    // Headings need to read as headings at a glance, so each level gets its own
    // colour *and* its own marker — colour alone is too weak a signal in a
    // terminal, and invisible to anyone who cannot distinguish the two hues.
    let (prefix, line_style) = match l.kind {
        md::Kind::Heading(1) => ("▌ ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
        md::Kind::Heading(2) => ("▌ ", Style::default().fg(t.flame).add_modifier(Modifier::BOLD)),
        md::Kind::Heading(3) => ("· ", Style::default().fg(t.teal).add_modifier(Modifier::BOLD)),
        md::Kind::Heading(_) => ("· ", Style::default().fg(t.violet).add_modifier(Modifier::BOLD)),
        md::Kind::Quote => ("▏ ", Style::default().fg(t.teal)),
        md::Kind::Callout => ("▎ ", Style::default().fg(t.teal).add_modifier(Modifier::BOLD)),
        md::Kind::Code => ("  ", Style::default().fg(t.teal)),
        md::Kind::CodeFence => ("  ", Style::default().fg(t.dimmer)),
        md::Kind::Diagram => ("  ", Style::default().fg(t.violet)),
        md::Kind::Rule => ("", Style::default().fg(t.dimmer)),
        md::Kind::Table => (" ", Style::default().fg(t.dim)),
        md::Kind::ListItem(n) => (indent(n, "• "), base),
        md::Kind::Task(true, n) => (indent(n, "✓ "), Style::default().fg(t.dimmer)),
        md::Kind::Task(false, n) => (indent(n, "☐ "), base),
        _ => (" ", base),
    };

    if l.kind == md::Kind::Rule {
        return Line::styled("  ────────────────────", line_style);
    }
    // A fence reads as a stray word on its own line unless it is drawn as what
    // it is: the boundary of a code block, labelled with its language.
    if l.kind == md::Kind::CodeFence {
        let lang = l.spans.first().map(|s| s.text.as_str()).unwrap_or("");
        let label = if lang.is_empty() || lang == "───" {
            "  ╌╌╌╌╌╌╌╌╌╌╌╌".to_owned()
        } else {
            format!("  ╌╌ {lang} ╌╌╌╌╌╌╌╌")
        };
        return Line::styled(label, line_style);
    }
    spans.push(Span::styled(format!(" {prefix}"), line_style));

    for s in &l.spans {
        let st = match s.style {
            md::Style::Strong => line_style.add_modifier(Modifier::BOLD),
            md::Style::Emph => line_style.add_modifier(Modifier::ITALIC),
            md::Style::Code => Style::default().fg(t.teal),
            md::Style::Strike => Style::default().fg(t.dimmer).add_modifier(Modifier::CROSSED_OUT),
            md::Style::Highlight => Style::default().fg(t.on_accent).bg(t.accent),
            md::Style::Underline => line_style.add_modifier(Modifier::UNDERLINED),
            // Redacted until revealed, like the editor's spoiler.
            md::Style::Spoiler => Style::default().fg(t.dimmer).bg(t.dimmer),
            md::Style::Link => Style::default().fg(t.teal).add_modifier(Modifier::UNDERLINED),
            md::Style::WikiLink => Style::default().fg(t.violet).add_modifier(Modifier::UNDERLINED),
            md::Style::Tag => Style::default().fg(t.teal),
            md::Style::Plain => line_style,
        };
        spans.push(Span::styled(s.text.clone(), st));
    }
    Line::from(spans)
}

/// Break a styled line at word boundaries, preserving each span's style across
/// the break and indenting continuations under the block's text.
///
/// Done here rather than with `Paragraph::wrap` so a continuation keeps its
/// block's indent — a bullet whose second line runs flush to the left margin
/// stops looking like a bullet.
fn wrap_line(line: Line<'static>, hang: usize, width: usize) -> Vec<Line<'static>> {
    let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if width < 8 || total <= width {
        return vec![line];
    }

    // Flatten to words, each carrying the style it came from.
    let mut words: Vec<(String, Style)> = Vec::new();
    for span in &line.spans {
        let mut w = String::new();
        for ch in span.content.chars() {
            w.push(ch);
            if ch == ' ' {
                words.push((std::mem::take(&mut w), span.style));
            }
        }
        if !w.is_empty() {
            words.push((w, span.style));
        }
    }

    let cont_style = line.spans.first().map(|s| s.style).unwrap_or_default();

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;

    for (word, style) in words {
        let w = word.chars().count();
        if used + w > width && used > 0 {
            out.push(Line::from(std::mem::take(&mut cur)));
            cur.push(Span::styled(" ".repeat(hang), cont_style));
            used = hang;
            // A continuation should not open with the space that ended the
            // previous line.
            let word = word.trim_start().to_owned();
            if word.is_empty() {
                continue;
            }
            used += word.chars().count();
            cur.push(Span::styled(word, style));
            continue;
        }
        used += w;
        cur.push(Span::styled(word, style));
    }
    if !cur.is_empty() {
        out.push(Line::from(cur));
    }
    out
}

fn indent(n: u8, marker: &'static str) -> &'static str {
    match n {
        0 => marker,
        _ => "  ",
    }
}

// ── status line ─────────────────────────────────────────────────────

fn status(f: &mut TFrame, app: &mut App, area: Rect) {
    let t = app.theme_copy();
    let boxed = app.theme.frame.boxed();

    // A ringing alarm outranks everything else in the status line: it is the
    // one thing the app has to say that you did not ask for.
    if let Some(msg) = app.ringing.clone() {
        let line = Line::from(vec![
            Span::styled(
                format!(" {msg} "),
                Style::default().fg(t.on_accent).bg(t.flame).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  any key to dismiss", Style::default().fg(t.dim)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    if let Mode::Confirm { question, .. } = &app.mode {
        let line = Line::from(vec![
            Span::styled(
                " CONFIRM ",
                Style::default().fg(t.on_accent).bg(t.flame).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {question} "), Style::default().fg(t.fg)),
            Span::styled("[y/N]", Style::default().fg(t.flame).add_modifier(Modifier::BOLD)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let (mode_txt, mid, warn) = match &app.mode {
        Mode::Filter(q) => ("FILTER".to_owned(), format!("/{q}▏"), false),
        Mode::Prompt { what, buf, key } => (
            match what {
                PromptKind::Rename => "RENAME".to_owned(),
                PromptKind::Tags => "TAGS".to_owned(),
                PromptKind::Setting => key.to_uppercase(),
                PromptKind::NewNote => "NEW NOTE".to_owned(),
                PromptKind::NewNotebook => "NEW NOTEBOOK".to_owned(),
                PromptKind::NewTask => "NEW TASK".to_owned(),
                PromptKind::NewDaily => "NEW DAILY".to_owned(),
                PromptKind::NewProject => "NEW PROJECT".to_owned(),
                PromptKind::NewSubtask => "NEW SUBTASK".to_owned(),
            },
            format!("{buf}▏"),
            false,
        ),
        Mode::Flash(m, w) => ("NORMAL".to_owned(), m.clone(), *w),
        // An applied filter owns the mode chip: a list showing 3 of 44 notes
        // with nothing to say why reads as a bug.
        _ if app.w().filter.is_some() => (
            "FILTER".to_owned(),
            format!("/{}", app.w().filter.clone().unwrap_or_default()),
            false,
        ),
        _ => {
            let mid = if app.w().feature == Feature::Notes {
                app.selected_note()
                    .map(|i| app.bag.notes[i].note.title.clone())
                    .unwrap_or_else(|| "—".into())
            } else if app.w().feature == Feature::Settings {
                app.section().name().to_lowercase()
            } else if app.w().feature == Feature::Calendar {
                app.day().strftime("%a %-d %b").to_string()
            } else {
                app.bucket().name().to_lowercase()
            };
            ("NORMAL".to_owned(), mid, false)
        }
    };

    let (n, total) = if app.w().feature == Feature::Notes {
        let all = app.notes().len();
        (if all == 0 { 0 } else { app.w().item.min(all - 1) + 1 }, all)
    } else if app.w().feature == Feature::Calendar {
        let all = app.day_items().len();
        (if all == 0 { 0 } else { app.w().item.min(all - 1) + 1 }, all)
    } else if app.w().feature == Feature::Settings {
        let all = app.entries().len();
        (if all == 0 { 0 } else { app.w().item.min(all - 1) + 1 }, all)
    } else {
        let all = app.item_count();
        (if all == 0 { 0 } else { app.w().item.min(all - 1) + 1 }, all)
    };
    let pct = (n * 100).checked_div(total).unwrap_or(0);

    let feat = format!(" {} {} ", app.w().feature.index() + 1, app.w().feature.name());
    let hint = format!(" ⏎ {} · ? help ", short_editor(&app.editor));
    let pos = format!(" {n}/{total} ");
    let pctx = format!(" {pct}% ");

    let mode_style = if warn {
        Style::default().fg(t.on_accent).bg(t.flame).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.on_accent).bg(t.accent).add_modifier(Modifier::BOLD)
    };

    let left = Line::from(vec![
        Span::styled(feat, Style::default().fg(t.on_accent).bg(t.flame).add_modifier(Modifier::BOLD)),
        Span::raw(if boxed { " " } else { "" }),
        Span::styled(format!(" {mode_txt} "), mode_style),
        Span::raw(" "),
        Span::styled(mid, Style::default().fg(if warn { t.flame } else { t.fg })),
    ]);
    let right = Line::from(vec![
        Span::styled(hint, Style::default().fg(t.dim).bg(t.panel)),
        Span::raw(" "),
        Span::styled(pos, Style::default().fg(t.on_accent).bg(t.accent).add_modifier(Modifier::BOLD)),
        Span::styled(pctx, Style::default().fg(t.dim).bg(t.sel)),
    ]);

    f.render_widget(Paragraph::new(left), area);
    let rw: usize = right.spans.iter().map(|s| s.content.chars().count()).sum();
    if rw < area.width as usize {
        let ra = Rect { x: area.x + area.width - rw as u16, width: rw as u16, ..area };
        f.render_widget(Paragraph::new(right), ra);
    }
}

fn short_editor(e: &str) -> &str {
    e.rsplit('/').next().unwrap_or(e)
}

// ── help ────────────────────────────────────────────────────────────

fn help(f: &mut TFrame, app: &App, area: Rect) {
    let t = &app.theme;
    let rows = [
        ("j k / ↑ ↓", "move  ·  scroll, in the preview"),
        ("h l / ← →", "change column"),
        ("gg  G", "first / last"),
        ("^e ^y · d u", "scroll the preview"),
        ("/", "filter (fuzzy)"),
        ("1 2 3 4", "notes / tasks / calendar / settings"),
        ("< >", "calendar: previous / next month"),
        ("h j k l", "calendar: walk the grid by day / week"),
        ("J K  .", "calendar: ±1 day · jump to today"),
        ("Enter / h", "calendar: into the agenda / back"),
        ("Tab", "cycle features"),
        ("g n t c s", "jump to a feature"),
        ("Alt+1..4", "workspaces  (Alt, not Ctrl — see below)"),
        ("[ ]", "cycle workspaces"),
        ("Enter", "open the body in your editor"),
        ("r", "rename  (renames the file too)"),
        ("t", "tags"),
        ("Space / ⏎", "favourite (notes) · complete (tasks)"),
        ("a", "add — a note, notebook, task, daily, project or subtask"),
        ("D", "delete (asks first; notes go to .siddran-trash/)"),
        ("l  in a project", "step into its subtasks, then Space"),
        ("p", "cycle priority  (tasks)"),
        ("^u  ^w", "at a prompt: clear the line / a word"),
        ("R", "reload from disk"),
        ("P  F", "swap palette / frame"),
        ("q", "quit"),
    ];

    let w = 62.min(area.width.saturating_sub(4));
    let h = (rows.len() as u16 + 5).min(area.height.saturating_sub(2));
    let r = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);

    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(format!(" {k:<12}"), Style::default().fg(t.accent)),
                Span::styled((*d).to_string(), Style::default().fg(t.fg)),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        " Most terminals cannot encode Ctrl+digit — hence Alt.",
        Style::default().fg(t.dimmer),
    ));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(" keys ")
        .style(Style::default().bg(t.panel));
    f.render_widget(Paragraph::new(lines).block(block), r);
}

// ── helpers ─────────────────────────────────────────────────────────

/// Keep the selection on screen without a stateful list widget.
fn scroll_to(sel: usize, height: usize, total: usize) -> usize {
    if total <= height || height == 0 {
        return 0;
    }
    sel.saturating_sub(height / 2).min(total - height)
}

/// An empty column still needs a cursor: when it has focus, show a highlighted
/// placeholder rather than nothing at all.
fn empty_marker(app: &App, msg: &str, focused: bool) -> Line<'static> {
    let t = &app.theme;
    if focused {
        Line::styled(
            format!(" {msg} "),
            Style::default().fg(t.on_accent).bg(t.accent),
        )
    } else {
        Line::styled(format!(" {msg}"), Style::default().fg(t.dimmer))
    }
}

/// Pad to `width` *characters*. `format!("{:<w$}")` counts bytes, which
/// misaligns any column holding non-ASCII — and titles here are full of it.
fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
    let mut out = s.to_owned();
    if n < width {
        out.push_str(&" ".repeat(width - n));
    }
    out
}

/// Truncate by character count, with an ellipsis.
fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Colours only, so a `&mut App` borrow for `preview()` does not collide with a
/// `&App` borrow of the theme.
pub struct ThemeCopy {
    pub panel: ratatui::style::Color,
    pub sel: ratatui::style::Color,
    pub fg: ratatui::style::Color,
    pub dim: ratatui::style::Color,
    pub dimmer: ratatui::style::Color,
    pub accent: ratatui::style::Color,
    pub flame: ratatui::style::Color,
    pub teal: ratatui::style::Color,
    pub violet: ratatui::style::Color,
    pub on_accent: ratatui::style::Color,
}

impl App {
    pub fn theme_copy(&self) -> ThemeCopy {
        ThemeCopy {
            panel: self.theme.panel,
            sel: self.theme.sel,
            fg: self.theme.fg,
            dim: self.theme.dim,
            dimmer: self.theme.dimmer,
            accent: self.theme.accent,
            flame: self.theme.flame,
            teal: self.theme.teal,
            violet: self.theme.violet,
            on_accent: self.theme.on_accent,
        }
    }
}
