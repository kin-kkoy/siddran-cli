//! Output shaping. A TTY gets aligned columns; a pipe gets tab-separated fields
//! so `siddran task ls | awk -F'\t'` works without flags. `--json` overrides
//! both. This is the whole reason the CLI half exists, so it is not an
//! afterthought.

use std::io::IsTerminal;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Table,
    Tsv,
    Json,
}

impl Mode {
    pub fn pick(json: bool) -> Mode {
        if json {
            Mode::Json
        } else if std::io::stdout().is_terminal() {
            Mode::Table
        } else {
            Mode::Tsv
        }
    }
}

/// Width in terminal cells. Good enough for the CJK and emoji that turn up in
/// note titles; a full `unicode-width` dependency arrives with the TUI, which
/// actually needs it for layout.
pub fn width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let cp = c as u32;
            if (0x1100..=0x115F).contains(&cp)
                || (0x2E80..=0xA4CF).contains(&cp)
                || (0xAC00..=0xD7A3).contains(&cp)
                || (0xF900..=0xFAFF).contains(&cp)
                || (0xFF00..=0xFF60).contains(&cp)
                || (0x1F300..=0x1FAFF).contains(&cp)
            {
                2
            } else if c == '\u{FE0F}' || (0x0300..=0x036F).contains(&cp) {
                0
            } else {
                1
            }
        })
        .sum()
}

pub fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = width(&c.to_string());
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// An empty result is ambiguous — "nothing there" and "your filter hid it all"
/// look identical. Say which, on stderr so a pipe still sees nothing.
pub fn empty(mode: Mode, hint: &str) {
    if mode == Mode::Table {
        eprintln!("{hint}");
    }
}

pub fn rows(mode: Mode, rows: &[Vec<String>]) {
    if rows.is_empty() {
        return;
    }
    match mode {
        Mode::Tsv => {
            for r in rows {
                println!("{}", r.join("\t"));
            }
        }
        Mode::Table => {
            let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
            // The last column is never padded — trailing spaces are noise.
            let widths: Vec<usize> = (0..cols)
                .map(|i| rows.iter().filter_map(|r| r.get(i)).map(|s| width(s)).max().unwrap_or(0))
                .collect();
            for r in rows {
                let mut line = String::new();
                for (i, cell) in r.iter().enumerate() {
                    if i + 1 == r.len() {
                        line.push_str(cell);
                    } else {
                        line.push_str(cell);
                        line.push_str(&" ".repeat(widths[i].saturating_sub(width(cell)) + 2));
                    }
                }
                println!("{}", line.trim_end());
            }
        }
        Mode::Json => unreachable!("json is emitted by the caller, which knows the shape"),
    }
}
