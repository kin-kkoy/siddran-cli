//! Sandboxes — the whiteboards. One file per board, `sandboxes/<id>.siddran`:
//!
//! ```json
//! { "sandbox": { "id", "title", "item_count", "created_at", "updated_at" },
//!   "items": [ { "rotation", "z_index", "payload", "id", "type", "x", "y", "w", "h", … } ] }
//! ```
//!
//! (localStore.js `flushNow`, `handleSandboxes`). Items are kept as raw JSON:
//! a board holds freehand strokes, PDFs, note cards and whatever the canvas
//! grows next, and this crate only ever needs to understand shapes, text and
//! connectors — everything else must round-trip untouched.

use crate::error::{io, Result};
use crate::json;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

use super::atomic;

pub struct Board {
    pub path: PathBuf,
    pub meta: Map<String, Value>,
    pub items: Vec<Value>,
}

impl Board {
    pub fn id(&self) -> &str {
        self.meta.get("id").and_then(Value::as_str).unwrap_or("")
    }
    pub fn title(&self) -> &str {
        self.meta.get("title").and_then(Value::as_str).unwrap_or("Untitled Sandbox")
    }

    /// Write back, keeping `item_count` and `updated_at` honest the way the
    /// desktop's batch endpoint does.
    pub fn save(&mut self) -> Result<()> {
        let now = crate::logic::deadline::now_iso();
        self.meta.insert("item_count".into(), json!(self.items.len() as i64));
        self.meta.insert("updated_at".into(), json!(now));
        let doc = json!({ "sandbox": Value::Object(self.meta.clone()), "items": self.items });
        atomic::write(&self.path, json::to_string(&doc).as_bytes())
    }

    /// Next z-index above everything on the board — new things go on top.
    pub fn next_z(&self) -> i64 {
        self.items.iter().filter_map(|i| i.get("z_index").and_then(Value::as_i64)).max().unwrap_or(0) + 1
    }

    /// Bounding box of everything with a position, as (min x, min y, max x, max y).
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        for it in &self.items {
            if it["type"] == "connector" {
                continue;
            }
            let (Some(x), Some(y)) = (it["x"].as_f64(), it["y"].as_f64()) else { continue };
            let (w, h) = (it["w"].as_f64().unwrap_or(0.0), it["h"].as_f64().unwrap_or(0.0));
            b = Some(match b {
                None => (x, y, x + w, y + h),
                Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x + w), e.max(y + h)),
            });
        }
        b
    }
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("sandboxes")
}

pub fn read(path: &Path) -> Result<Board> {
    let v = json::parse(path)?;
    Ok(Board {
        path: path.to_path_buf(),
        meta: v.get("sandbox").and_then(Value::as_object).cloned().unwrap_or_default(),
        items: v.get("items").and_then(Value::as_array).cloned().unwrap_or_default(),
    })
}

/// Every board, most recently changed first — the desktop's hub order.
pub fn list(root: &Path) -> Result<Vec<Board>> {
    let d = dir(root);
    if !d.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(&d).map_err(io(&d))? {
        let p = e.map_err(io(&d))?.path();
        if p.extension().and_then(|x| x.to_str()) == Some("siddran") {
            // One unreadable board must not hide the rest.
            if let Ok(b) = read(&p) {
                if !b.id().is_empty() {
                    out.push(b);
                }
            }
        }
    }
    out.sort_by(|a, b| {
        let u = |x: &Board| x.meta.get("updated_at").and_then(Value::as_str).unwrap_or("").to_owned();
        u(b).cmp(&u(a))
    });
    Ok(out)
}

/// A new, empty board, in the desktop's key order (localStore.js handleSandboxes).
pub fn create(root: &Path, title: &str) -> Result<Board> {
    let id = crate::ids::uuid_v4();
    let now = crate::logic::deadline::now_iso();
    let meta = json!({ "id": id, "title": title, "item_count": 0, "created_at": now, "updated_at": now });
    let mut b = Board {
        path: dir(root).join(format!("{id}.siddran")),
        meta: meta.as_object().cloned().unwrap_or_default(),
        items: Vec::new(),
    };
    b.save()?;
    Ok(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_list_save_round_trip() {
        let t = tempfile::tempdir().unwrap();
        let mut b = create(t.path(), "Plans").unwrap();
        b.items.push(json!({ "rotation": 0, "z_index": 1, "payload": { "points": [1, 2] }, "id": "s1", "type": "stroke", "x": 0, "y": 0, "w": 1, "h": 1 }));
        b.save().unwrap();
        let all = list(t.path()).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title(), "Plans");
        assert_eq!(all[0].meta["item_count"], 1);
        // An item we do not model comes back exactly as written.
        assert_eq!(all[0].items[0]["payload"]["points"], json!([1, 2]));
        assert_eq!(all[0].next_z(), 2);
    }
}
