//! A row is a typed view over the raw JSON it was parsed from.
//!
//! Rows on disk are not uniform: legacy rows omit keys the current code writes,
//! and `absent`, `null` and `false` mean three different things (see
//! `docs/parity.md`). Deserializing into a plain struct and re-serializing would
//! materialize every missing key — which for `due_all_day` silently converts
//! legacy date-only deadlines into midnight appointments.
//!
//! So every row keeps the map it came from. Reads go through `v`; writes (Phase
//! 2) will patch only the touched keys back into `raw`, leaving absent keys
//! absent and unknown keys untouched.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct Row<T> {
    pub v: T,
    /// The object exactly as parsed. `preserve_order` keeps key order, so a row
    /// nothing touched round-trips byte-for-byte.
    pub raw: Map<String, Value>,
}

impl<T: DeserializeOwned> Row<T> {
    pub fn from_value(value: &Value) -> Option<Row<T>> {
        let obj = value.as_object()?;
        let v = T::deserialize(value).ok()?;
        Some(Row {
            v,
            raw: obj.clone(),
        })
    }

    /// Parses `root[key]` as an array of rows. A row that fails to deserialize
    /// is skipped rather than failing the whole Bag: one malformed entry should
    /// not make the app refuse to open.
    pub fn list(root: &Value, key: &str) -> Vec<Row<T>> {
        root.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Row::from_value).collect())
            .unwrap_or_default()
    }
}

use crate::logic::deadline;
use serde::Serialize;

impl<T> Row<T> {
    /// Set one field in both the typed view and the raw backing.
    ///
    /// This is the *only* way a row should change. Writing through `raw`
    /// directly would let the two drift; writing only through `v` would lose the
    /// change on save. Keys that were never set stay exactly where they were —
    /// and keys that were absent stay absent, which is the whole point.
    pub fn set(&mut self, key: &str, value: impl Serialize) {
        if let Ok(v) = serde_json::to_value(value) {
            self.raw.insert(key.to_owned(), v);
        }
    }

    /// Stamp `updated_at`, mirroring `touch()` (localStore.js:363). Every
    /// mutation should end with this so the desktop's last-write-wins merge sees
    /// the change.
    pub fn touch(&mut self) {
        self.set("updated_at", deadline::now_iso());
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.raw.get(key)?.as_str()
    }
}
