//! Ids are heterogeneous on disk: legacy rows carry a JSON number, rows created
//! since the UUID switch carry a string, and both live in the same array. The
//! desktop's `parseId` (localStore.js:17) exists for exactly this.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Num(i64),
    Str(String),
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Id::Num(n) => write!(f, "{n}"),
            Id::Str(s) => f.write_str(s),
        }
    }
}

impl Id {
    /// Mirrors `parseId`: a digit-only string is coerced back to a number so
    /// that ids compare equal however they arrived.
    pub fn parse(raw: &str) -> Option<Id> {
        if raw.is_empty() {
            return None;
        }
        if raw.bytes().all(|b| b.is_ascii_digit()) {
            raw.parse::<i64>().ok().map(Id::Num)
        } else {
            Some(Id::Str(raw.to_owned()))
        }
    }

    /// The desktop sorts on `String(a.id)` (localStore.js:215) — lexicographic
    /// on the *string* form, so "1000" < "999". Sorting numerically instead
    /// would rewrite every `.siddran` file on first save.
    pub fn sort_key(&self) -> String {
        self.to_string()
    }
}

/// A v4 UUID from the OS RNG.
///
/// Tasks, dailies, projects and events use UUIDs so two offline devices cannot
/// collide. Notes must **not** — `mdToNote` does `Number(fm.id)`, so a UUID in
/// note frontmatter comes back as `NaN` on the desktop side; they use
/// `bag::notes::next_note_id` instead.
pub fn uuid_v4() -> String {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .expect("/dev/urandom is readable");
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 1
    let h = |r: &[u8]| r.iter().map(|x| format!("{x:02x}")).collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        h(&b[0..4]),
        h(&b[4..6]),
        h(&b[6..8]),
        h(&b[8..10]),
        h(&b[10..16])
    )
}
