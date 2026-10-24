//! A markdown reader for the preview pane.
//!
//! Not a CommonMark implementation, and deliberately not `pulldown-cmark`.
//! Siddran's markdown carries extensions the editor renders — `[[wikilinks]]`,
//! `==highlight==`, `++underline++`, `||spoiler||`, `> [!callout]`, `#hashtag`
//! and the ```` ```siddran-diagram ```` fence — so an inline scanner is needed
//! regardless, at which point cmark would be doing a fraction of the work for
//! all of the dependency. Fidelity to CommonMark is not the goal; fidelity to
//! *what the CodeMirror editor shows* is.
//!
//! Output is line-anchored on purpose: a terminal pane renders a viewport, so
//! the renderer must produce a `Vec<Line>` it can slice, not an event stream.

mod inline;
mod wikilink;

pub use inline::{Span, Style};
pub use wikilink::{parse_typed_link, Target};

/// One rendered line, already broken into styled spans.
#[derive(Debug, Clone, Default)]
pub struct Line {
    pub spans: Vec<Span>,
    pub kind: Kind,
}

/// What a line *is*, so the renderer can style the whole line (indent, gutter,
/// background) rather than only its inline runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Para,
    Heading(u8),
    Quote,
    /// `> [!note]` and friends — the first line carries the label.
    Callout,
    ListItem(u8),
    /// `- [ ]` / `- [x]`
    Task(bool, u8),
    Code,
    CodeFence,
    /// A ```` ```siddran-diagram ```` block, collapsed to one placeholder line.
    Diagram,
    Rule,
    Table,
    Blank,
}

/// Render a note body to lines.
pub fn render(body: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut fence_lang = String::new();
    let mut diagram_items: Option<usize> = None;

    for raw in body.lines() {
        let trimmed = raw.trim_start();

        // Fences first: nothing inside one is markdown.
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_fence {
                in_fence = false;
                if let Some(n) = diagram_items.take() {
                    out.push(Line {
                        spans: vec![Span::plain(format!(
                            "◇ diagram — {n} item{}",
                            if n == 1 { "" } else { "s" }
                        ))],
                        kind: Kind::Diagram,
                    });
                }
                fence_lang.clear();
                continue;
            }
            in_fence = true;
            fence_lang = rest.trim().to_owned();
            if fence_lang == "siddran-diagram" {
                // Collapsed to a placeholder: the canvas is out of scope, but
                // the block must survive an edit untouched, so it is shown as
                // present rather than hidden.
                diagram_items = Some(0);
            } else {
                out.push(Line {
                    spans: vec![Span::plain(if fence_lang.is_empty() {
                        "─".repeat(3)
                    } else {
                        fence_lang.clone()
                    })],
                    kind: Kind::CodeFence,
                });
            }
            continue;
        }

        if in_fence {
            if let Some(n) = diagram_items.as_mut() {
                *n += raw.matches("\"id\"").count();
            } else {
                out.push(Line {
                    spans: vec![Span::plain(raw.to_owned())],
                    kind: Kind::Code,
                });
            }
            continue;
        }

        out.push(block_line(raw, trimmed));
    }

    // An unterminated diagram fence still gets its placeholder, so a truncated
    // note does not silently lose the block.
    if let Some(n) = diagram_items {
        out.push(Line {
            spans: vec![Span::plain(format!("◇ diagram — {n} items"))],
            kind: Kind::Diagram,
        });
    }
    out
}

fn block_line(raw: &str, trimmed: &str) -> Line {
    let indent = ((raw.len() - trimmed.len()) / 2).min(4) as u8;

    if trimmed.is_empty() {
        return Line { spans: vec![], kind: Kind::Blank };
    }
    if matches!(trimmed, "---" | "***" | "___" | "- - -") {
        return Line { spans: vec![], kind: Kind::Rule };
    }
    if trimmed.starts_with('|') && trimmed.ends_with('|') {
        return Line { spans: inline::scan(trimmed), kind: Kind::Table };
    }

    // Headings
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
        return Line {
            spans: inline::scan(trimmed[hashes + 1..].trim()),
            kind: Kind::Heading(hashes as u8),
        };
    }

    // Quotes and callouts
    if let Some(rest) = trimmed.strip_prefix('>') {
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        if let Some(label) = callout_label(rest) {
            let mut spans = vec![Span {
                text: label.to_uppercase(),
                style: Style::Strong,
            }];
            let after = &rest[label.len() + 3..];
            if !after.trim().is_empty() {
                spans.push(Span::plain("  ".into()));
                spans.extend(inline::scan(after.trim()));
            }
            return Line { spans, kind: Kind::Callout };
        }
        return Line { spans: inline::scan(rest), kind: Kind::Quote };
    }

    // Task list items, before plain list items
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            if let Some(body) = rest.strip_prefix("[ ] ") {
                return Line { spans: inline::scan(body), kind: Kind::Task(false, indent) };
            }
            if let Some(body) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
                return Line { spans: inline::scan(body), kind: Kind::Task(true, indent) };
            }
            return Line { spans: inline::scan(rest), kind: Kind::ListItem(indent) };
        }
    }
    // Ordered list: `1. `
    if let Some(dot) = trimmed.find(". ") {
        if dot > 0 && trimmed[..dot].bytes().all(|b| b.is_ascii_digit()) {
            return Line {
                spans: inline::scan(&trimmed[dot + 2..]),
                kind: Kind::ListItem(indent),
            };
        }
    }

    Line { spans: inline::scan(trimmed), kind: Kind::Para }
}

/// `[!note] rest` -> `note`
fn callout_label(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("[!")?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}
