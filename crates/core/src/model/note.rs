//! A note is a markdown file with a frontmatter block. The block is *not* YAML:
//! it is a fixed sequence of `key: value` lines emitted by `noteToMd`
//! (localStore.js:38-53) in a strict order. Reproducing it exactly is what lets
//! the TUI and the desktop app share a Bag without rewriting each other's files.

/// Parsed form of one `.md` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: Option<String>,
    pub title: String,
    /// Kept as the raw frontmatter string. Empty tags are written as `tags: `
    /// *with* a trailing space — trimming it rewrites every note in the Bag.
    pub tags: String,
    pub favorite: bool,
    pub color: String,
    /// Emitted only when present (localStore.js:46).
    pub order: Option<i64>,
    pub created: String,
    pub updated: String,
    pub body: String,
    /// False when the file carried no frontmatter block at all. Such a file is
    /// not byte-stable: writing it adds a block, exactly as the desktop does.
    pub had_frontmatter: bool,
}

impl Note {
    /// Mirrors `mdToNote` (localStore.js:55-79): only a block anchored at offset
    /// 0, split each line on the *first* colon, unknown keys ignored.
    pub fn from_md(text: &str, fallback_title: &str) -> Note {
        let mut n = Note {
            id: None,
            title: String::new(),
            tags: String::new(),
            favorite: false,
            color: String::new(),
            order: None,
            created: String::new(),
            updated: String::new(),
            body: text.to_owned(),
            had_frontmatter: false,
        };

        if let Some((block, rest)) = split_frontmatter(text) {
            n.had_frontmatter = true;
            n.body = rest.to_owned();
            for line in block.split('\n') {
                let Some(colon) = line.find(':') else { continue };
                let key = line[..colon].trim();
                let val = line[colon + 1..].trim();
                match key {
                    // The desktop does `fm.id ? Number(fm.id) : null`, so a
                    // hand-corrupted id becomes NaN and survives. We keep the
                    // raw text instead: a value we cannot use is treated as
                    // absent rather than propagated as a poisoned number.
                    "id" => n.id = (!val.is_empty()).then(|| val.to_owned()),
                    "title" => n.title = val.to_owned(),
                    "tags" => n.tags = val.to_owned(),
                    "favorite" => n.favorite = val == "true",
                    "color" => n.color = val.to_owned(),
                    "order" => n.order = val.parse::<i64>().ok(),
                    "created" => n.created = val.to_owned(),
                    "updated" => n.updated = val.to_owned(),
                    _ => {}
                }
            }
        }

        if n.title.is_empty() {
            n.title = fallback_title.to_owned();
        }
        n
    }

    /// Mirrors `noteToMd`. Every detail here is load-bearing — see the tests.
    pub fn to_md(&self) -> String {
        let mut s = String::with_capacity(self.body.len() + 256);
        s.push_str("---\n");
        s.push_str(&format!("id: {}\n", self.id.as_deref().unwrap_or("")));
        // A newline in a title would break the block, so it is flattened.
        s.push_str(&format!("title: {}\n", self.title.replace('\n', " ")));
        // Trailing space when empty is intentional: the JS template is
        // `tags: ${note.tags || ''}` with no trim.
        s.push_str(&format!("tags: {}\n", self.tags));
        s.push_str(&format!("favorite: {}\n", self.favorite));
        s.push_str(&format!("color: {}\n", self.color));
        if let Some(o) = self.order {
            s.push_str(&format!("order: {o}\n"));
        }
        s.push_str(&format!("created: {}\n", self.created));
        s.push_str(&format!("updated: {}\n", self.updated));
        // The join's trailing '' supplies this newline; no blank line is
        // inserted between the block and the body.
        s.push_str("---\n");
        s.push_str(&self.body);
        s
    }
}

/// Returns (block, body) for `---\n…\n---\n?` anchored at offset 0.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let block = &rest[..end];
    let after = &rest[end + 4..];
    // The closing fence may or may not be followed by a newline.
    let body = after.strip_prefix('\n').unwrap_or(after);
    Some((block, body))
}
