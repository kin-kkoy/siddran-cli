//! `parseWikilink` (Editor/utils/parseWikilink.js), one parser shared by the
//! preview and anything that follows a link — the desktop made that a deliberate
//! rule, and two parsers that must agree is the expensive kind of duplication.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `[[Note]]` or `[[Note#heading]]`
    Note { title: String, heading: Option<String> },
    Task(String),
    Daily(String),
    Project(String),
    /// `[[sandbox:id]]` — parsed so it can be shown as unfollowable rather than
    /// silently treated as a note title.
    Sandbox(String),
}

/// The inner text of a `[[…]]`, minus the brackets.
pub fn parse_typed_link(inner: &str) -> Option<(Target, Option<String>)> {
    let (body, alias) = match inner.split_once('|') {
        Some((b, a)) => (b.trim(), Some(a.trim().to_owned())),
        None => (inner.trim(), None),
    };

    for (prefix, make) in [
        ("task:", (|s: &str| Target::Task(s.into())) as fn(&str) -> Target),
        ("daily:", |s| Target::Daily(s.into())),
        ("bundle:", |s| Target::Project(s.into())),
        ("project:", |s| Target::Project(s.into())),
        ("sandbox:", |s| Target::Sandbox(s.into())),
    ] {
        if let Some(id) = body.strip_prefix(prefix) {
            return Some((make(id.trim()), alias));
        }
    }

    let (title, heading) = match body.split_once('#') {
        Some((t, h)) => (t.trim(), Some(h.trim().to_owned())),
        None => (body, None),
    };
    if title.is_empty() {
        return None;
    }
    Some((
        Target::Note {
            title: title.to_owned(),
            heading,
        },
        alias,
    ))
}

/// What to show for a link: the alias if given, else the title without its
/// type prefix.
pub fn display_text(inner: &str) -> &str {
    let (body, alias) = match inner.split_once('|') {
        Some((b, a)) => (b.trim(), Some(a.trim())),
        None => (inner.trim(), None),
    };
    if let Some(a) = alias {
        return a;
    }
    for p in ["task:", "daily:", "bundle:", "project:", "sandbox:"] {
        if let Some(rest) = body.strip_prefix(p) {
            return rest.trim();
        }
    }
    body
}
