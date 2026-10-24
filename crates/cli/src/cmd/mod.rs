pub mod bag;
pub mod event;
pub mod note;
pub mod settings;
pub mod task;

use crate::out::Mode;
use crate::session::Session;
use siddran_core::{Bag, Id};
use std::path::PathBuf;

/// Everything a command needs. `session` is present only for mutating commands,
/// which is what makes "did you take the lock?" a compile-time-ish question
/// rather than a convention.
pub struct Ctx {
    pub bag: Bag,
    pub root: PathBuf,
    pub session: Option<Session>,
    pub mode: Mode,
}

impl Ctx {
    /// Guard every write: the Bag must not have moved since we read it.
    pub fn before_write(&self) -> Result<(), String> {
        match &self.session {
            Some(s) => s.check_unchanged(),
            None => Err("internal: a write was attempted without a session".into()),
        }
    }
}

/// Matching by id or by title: ids are exact, titles are case-insensitive and
/// match on a prefix so `siddran note cat todo` finds "TODO".
pub fn matches(id: &Id, title: &str, needle: &str) -> bool {
    if id.to_string() == needle {
        return true;
    }
    let (t, n) = (title.to_lowercase(), needle.to_lowercase());
    t == n || t.starts_with(&n)
}
