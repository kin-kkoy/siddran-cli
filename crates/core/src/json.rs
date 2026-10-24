//! JSON that matches what the desktop app writes, byte for byte.
//!
//! The desktop persists with `JSON.stringify(value, null, 2)` and no trailing
//! newline. `serde_json`'s 2-space pretty printer agrees with V8 on every point
//! that shows up in a Bag — `{}` / `[]` for empties, `\b \t \n \f \r` short
//! escapes with `\u00XX` for the remaining C0 controls, UTF-8 passed through
//! unescaped, and shortest-round-trip floats. `identity_roundtrip` proves that
//! against the real files rather than trusting it.
//!
//! The one divergence to stay clear of is float formatting on *construction*:
//! serde_json writes `2.0` where JS writes `2`. Values parsed from disk keep
//! whatever number type they arrived as, so this only bites when we build a
//! number ourselves — hence: never construct an integral quantity from `f64`.

use crate::error::{io, Error, Result};
use serde_json::Value;
use std::path::Path;

/// `preserve_order` is load-bearing: without it serde_json backs objects with a
/// `BTreeMap` and every key gets alphabetized on the first write, diffing the
/// entire Bag.
pub fn parse(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).map_err(io(path))?;
    serde_json::from_str(&text).map_err(|source| Error::Json {
        path: path.to_path_buf(),
        source,
    })
}

/// `JSON.stringify(v, null, 2)` — note the deliberate absence of a trailing
/// newline, which is what `fs::write` of a stringify result produces.
pub fn to_string(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("serializing a parsed Value cannot fail")
}
