//! Siddran's Bag format and domain logic — no terminal, no CLI.
//!
//! A Bag is a folder the user picks; the desktop app
//! (`Siddran-Desktop/app/src/desktop/localStore.js`) is the reference
//! implementation of its layout. This crate is a *peer* of that app, not a
//! client of it: the overriding requirement is that anything written here is
//! byte-identical to what the desktop would have written, so the two can share
//! a Bag without rewriting each other's files.

pub mod bag;
pub mod diagram;
pub mod error;
pub mod ids;
pub mod json;
pub mod logic;
pub mod md;
pub mod presence;
pub mod model;
pub mod query;
pub mod row;

/// Re-exported so callers share this crate's date types and version rather than
/// depending on jiff themselves — every date rule in Siddran lives here.
pub use jiff;

pub use bag::Bag;
pub use error::{Error, Result};
pub use ids::Id;
pub use model::Note;
pub use row::Row;
