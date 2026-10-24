//! Inline scanning: one pass, no AST.
//!
//! Covers what the CodeMirror editor decorates — and only that. The delimiters
//! are matched non-greedily and never across a line, which is both what the
//! editor does and what keeps this a single pass.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Strong,
    Emph,
    Code,
    Strike,
    Highlight,
    Underline,
    /// Rendered as a redaction block until revealed.
    Spoiler,
    Link,
    /// `[[Note]]`, `[[Note#heading]]`, `[[task:id|alias]]`
    WikiLink,
    Tag,
}

#[derive(Debug, Clone)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn plain(text: String) -> Span {
        Span { text, style: Style::Plain }
    }
}

/// Delimited runs, longest marker first so `**` wins over `*`.
const PAIRS: [(&str, Style); 8] = [
    ("**", Style::Strong),
    ("~~", Style::Strike),
    ("==", Style::Highlight),
    ("++", Style::Underline),
    ("||", Style::Spoiler),
    ("`", Style::Code),
    ("*", Style::Emph),
    ("_", Style::Emph),
];

pub fn scan(s: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let b = s.as_bytes();

    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                out.push(Span::plain(std::mem::take(&mut plain)));
            }
        };
    }

    while i < s.len() {
        // `[[wikilink]]` — checked before `[link](url)` since both start with '['
        if s[i..].starts_with("[[") {
            if let Some(end) = s[i + 2..].find("]]") {
                flush!();
                let inner = &s[i + 2..i + 2 + end];
                out.push(Span {
                    text: crate::md::wikilink::display_text(inner).to_owned(),
                    style: Style::WikiLink,
                });
                i += end + 4;
                continue;
            }
        }

        // `[text](url)` — only the text is shown; a terminal cannot click a URL.
        if b[i] == b'[' {
            if let Some(close) = s[i..].find("](") {
                if let Some(paren) = s[i + close..].find(')') {
                    flush!();
                    out.push(Span {
                        text: s[i + 1..i + close].to_owned(),
                        style: Style::Link,
                    });
                    i += close + paren + 1;
                    continue;
                }
            }
        }

        // `#hashtag` — only at a word boundary, so `C#` and `#` alone are not tags.
        if b[i] == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace()) {
            let rest = &s[i + 1..];
            let len = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '/'))
                .unwrap_or(rest.len());
            if len > 0 {
                flush!();
                out.push(Span {
                    text: s[i..i + 1 + len].to_owned(),
                    style: Style::Tag,
                });
                i += 1 + len;
                continue;
            }
        }

        // Delimited runs
        let mut matched = false;
        for (marker, style) in PAIRS {
            if !s[i..].starts_with(marker) {
                continue;
            }
            let from = i + marker.len();
            let Some(end) = s[from..].find(marker) else { continue };
            if end == 0 {
                continue; // empty run, e.g. `****`
            }
            flush!();
            out.push(Span {
                text: s[from..from + end].to_owned(),
                style,
            });
            i = from + end + marker.len();
            matched = true;
            break;
        }
        if matched {
            continue;
        }

        // Ordinary character. Step by char, not byte, so UTF-8 survives.
        let ch = s[i..].chars().next().expect("in bounds");
        plain.push(ch);
        i += ch.len_utf8();
    }

    if !plain.is_empty() {
        out.push(Span::plain(plain));
    }
    if out.is_empty() {
        out.push(Span::plain(String::new()));
    }
    out
}
