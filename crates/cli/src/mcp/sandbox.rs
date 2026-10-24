//! Sandbox tools — whiteboards, for a model that cannot see them.
//!
//! The design problem is that an AI placing boxes by coordinate is blind: it
//! cannot tell that two boxes overlap or that an arrow crosses a third. So the
//! tools speak in structure instead of geometry:
//!
//! - `sandbox_diagram` takes flowchart text and lays it out (core::diagram).
//! - `sandbox_read` hands a board back *as* flowchart text, plus the real ids.
//! - `sandbox_edit` adds a box "after" an existing one, connects boxes by their
//!   text, and finds free space itself.
//!
//! Coordinates are still accepted where a model really wants them, but nothing
//! requires them.

use super::tools::{need, pick, s, writing, R};
use serde_json::{json, Value};
use siddran_core::bag::{notes, sandbox};
use siddran_core::diagram::{self, Dir, Style};
use siddran_core::logic::deadline;
use std::path::Path;

fn tool(name: &str, description: &str, props: Value, required: &[&str]) -> Value {
    json!({ "name": name, "description": description, "inputSchema": { "type": "object", "properties": props, "required": required } })
}

pub fn definitions() -> Vec<Value> {
    vec![
        tool("sandboxes_list", "List the user's sandboxes (whiteboards).", json!({}), &[]),
        tool("sandbox_read", "Read a board. Shapes and arrows come back as flowchart text (Mermaid-style) with each shape's id; other things (free text, note/task cards, handwriting, PDFs) are listed or counted.", json!({
            "sandbox": { "type": "string", "description": "Board id or title" },
        }), &["sandbox"]),
        tool("sandbox_create", "Create an empty board.", json!({ "title": { "type": "string" } }), &["title"]),
        tool("sandbox_diagram", "Draw a flowchart from text and lay it out automatically. Syntax (Mermaid flowchart subset): optional first line `flowchart TD` (top-down) or `flowchart LR`; nodes `A[Box]`, `B(Rounded)`, `C{Decision}`, `D((Circle))`, `E[(Database)]`, `F[/Input/]`, `G([Start/End])`, `H{{Prep}}`; edges `A --> B`, `A -->|label| B`, `A -- label --> B`, `A --- B` (no arrowhead), `A -.-> B` (dashed), chains `A --> B --> C`, fan-out `A --> B & C`. Goes on a board (placed beside what is there) or, with `note`, into a note as an embedded diagram.", json!({
            "text": { "type": "string", "description": "The flowchart" },
            "sandbox": { "type": "string", "description": "Existing board id or title. Omit (with `title`) to make a new board." },
            "title": { "type": "string", "description": "Title for a new board" },
            "note": { "type": "string", "description": "Instead of a board: append the diagram to this note (id or title)" },
        }), &["text"]),
        tool("sandbox_edit", "Change a board without coordinates. `add` boxes (each optionally `after` an existing box, which places it below and draws the arrow), `connect` boxes, `update` a box's text or shape, `remove` things. Refer to boxes by id or by their text.", json!({
            "sandbox": { "type": "string" },
            "add": { "type": "array", "items": { "type": "object", "properties": {
                "text": { "type": "string" },
                "shape": { "type": "string", "enum": ["rect", "roundedRect", "rhombus", "ellipse", "hexagon", "cylinder", "parallelogram", "text"], "default": "roundedRect" },
                "after": { "type": "string", "description": "Box (id or text) this one follows" },
                "label": { "type": "string", "description": "Label on the arrow from `after`" },
                "x": { "type": "number" }, "y": { "type": "number" },
            }, "required": ["text"] } },
            "connect": { "type": "array", "items": { "type": "object", "properties": {
                "from": { "type": "string" }, "to": { "type": "string" }, "label": { "type": "string" },
            }, "required": ["from", "to"] } },
            "update": { "type": "array", "items": { "type": "object", "properties": {
                "box": { "type": "string" }, "text": { "type": "string" },
                "shape": { "type": "string" }, "fill": { "type": "string", "description": "#rrggbb or transparent" },
            }, "required": ["box"] } },
            "remove": { "type": "array", "items": { "type": "string" }, "description": "Boxes (id or text) or connector ids. A box's arrows go with it." },
        }), &["sandbox"]),
    ]
}

pub fn call(root: &Path, name: &str, a: &Value) -> Option<R> {
    Some(match name {
        "sandboxes_list" => list(root),
        "sandbox_read" => read(root, a),
        "sandbox_create" => create(root, a),
        "sandbox_diagram" => draw(root, a),
        "sandbox_edit" => edit(root, a),
        _ => return None,
    })
}

fn boards(root: &Path) -> Result<Vec<sandbox::Board>, String> {
    sandbox::list(root).map_err(|e| e.to_string())
}

fn find(root: &Path, what: &str) -> Result<sandbox::Board, String> {
    let all = boards(root)?;
    let hit = pick("sandbox", what, &all, |b| b.id().to_owned(), |b| b.title().to_owned())?;
    sandbox::read(&hit.path).map_err(|e| e.to_string())
}

fn list(root: &Path) -> R {
    Ok(Value::Array(
        boards(root)?
            .iter()
            .map(|b| json!({ "id": b.id(), "title": b.title(), "items": b.items.len(), "updated": b.meta.get("updated_at") }))
            .collect(),
    ))
}

fn read(root: &Path, a: &Value) -> R {
    let b = find(root, need(a, "sandbox")?)?;
    let (flow, keys) = diagram::describe(&b.items);
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut texts = Vec::new();
    for it in &b.items {
        match it["type"].as_str().unwrap_or("") {
            "shape" | "connector" => {}
            "text" => texts.push(json!({ "id": it["id"], "text": it["payload"]["text"] })),
            other => *counts.entry(if other == "stroke" { "handwriting strokes".into() } else { other.to_owned() }).or_default() += 1,
        }
    }
    Ok(json!({
        "id": b.id(), "title": b.title(),
        "flowchart": flow,
        "box_ids": keys.into_iter().map(|(k, id)| json!({ "key": k, "id": id })).collect::<Vec<_>>(),
        "text_items": texts,
        "other": counts,
    }))
}

fn create(root: &Path, a: &Value) -> R {
    let title = need(a, "title")?.to_owned();
    writing(root, |_| {
        let b = sandbox::create(root, &title).map_err(|e| e.to_string())?;
        Ok(json!({ "id": b.id(), "title": b.title() }))
    })
}

fn draw(root: &Path, a: &Value) -> R {
    let text = need(a, "text")?;
    let (g, problems) = diagram::parse(text)?;
    let dir = g.dir.unwrap_or(Dir::Down);
    let now = deadline::now_iso();

    if let Some(n) = s(a, "note") {
        let placed = diagram::layout(&g, dir, (0, 0), &Style::default(), &now, siddran_core::ids::uuid_v4);
        return writing(root, |bag| {
            let nf = super::tools::find_note(bag, n)?;
            let mut note = nf.note.clone();
            if !note.body.is_empty() && !note.body.ends_with('\n') {
                note.body.push('\n');
            }
            note.body.push_str(&fence(&placed.items));
            note.body.push('\n');
            note.updated = now.clone();
            notes::save(&note, &nf.path).map_err(|e| e.to_string())?;
            Ok(json!({ "note": note.title, "boxes": g.nodes.len(), "arrows": g.edges.len(), "skipped_lines": problems }))
        });
    }

    writing(root, |_| {
        let mut b = match s(a, "sandbox") {
            Some(w) => find(root, w)?,
            None => sandbox::create(root, s(a, "title").unwrap_or("Diagram")).map_err(|e| e.to_string())?,
        };
        // Beside whatever is already there, never on top of it.
        let origin = match b.bounds() {
            Some((_, y0, x1, _)) => ((x1 + 160.0).round() as i64, y0.round() as i64),
            None => (0, 0),
        };
        let z0 = b.next_z();
        let mut placed = diagram::layout(&g, dir, origin, &Style::default(), &now, siddran_core::ids::uuid_v4);
        for (i, it) in placed.items.iter_mut().enumerate() {
            it["z_index"] = json!(z0 + i as i64);
        }
        b.items.extend(placed.items);
        b.save().map_err(|e| e.to_string())?;
        Ok(json!({
            "sandbox": { "id": b.id(), "title": b.title() },
            // In the order they were written, not HashMap order.
            "boxes": g.nodes.iter().map(|n| json!({ "key": n.key, "text": n.label, "id": placed.ids[&n.key] })).collect::<Vec<_>>(),
            "arrows": g.edges.len(),
            "skipped_lines": problems,
        }))
    })
}

/// The ```siddran-diagram fence a note embeds (Siddran-Desktop utils/diagramBlock.js
/// `serializeDiagram`): compact JSON, keys id,type,x,y,w,h,z_index,payload.
fn fence(items: &[Value]) -> String {
    let items: Vec<Value> = items
        .iter()
        .map(|it| json!({ "id": it["id"], "type": it["type"], "x": it["x"], "y": it["y"], "w": it["w"], "h": it["h"], "z_index": it["z_index"], "payload": it["payload"] }))
        .collect();
    format!("```siddran-diagram\n{}\n```", json!({ "v": 1, "items": items }))
}

/// A box on the board by id, or by its text (exact, then prefix, then substring).
fn find_box(items: &[Value], what: &str) -> Result<String, String> {
    let shapes: Vec<&Value> = items.iter().filter(|i| i["type"] == "shape" || i["type"] == "text").collect();
    let hit = pick(
        "box",
        what,
        &shapes,
        |i| i["id"].as_str().unwrap_or("").to_owned(),
        |i| i["payload"]["text"].as_str().unwrap_or("").replace('\n', " "),
    )?;
    Ok(hit["id"].as_str().unwrap_or("").to_owned())
}

fn rect(it: &Value) -> (i64, i64, i64, i64) {
    let g = |k: &str| it[k].as_f64().unwrap_or(0.0).round() as i64;
    (g("x"), g("y"), g("w"), g("h"))
}

fn overlaps(items: &[Value], r: (i64, i64, i64, i64)) -> bool {
    items.iter().filter(|i| i["type"] != "connector").any(|i| {
        let (x, y, w, h) = rect(i);
        r.0 < x + w + 24 && x < r.0 + r.2 + 24 && r.1 < y + h + 24 && y < r.1 + r.3 + 24
    })
}

/// First free spot at or right of `(x, y)` for a `w×h` box.
fn free_spot(items: &[Value], x: i64, y: i64, w: i64, h: i64) -> (i64, i64) {
    let mut x = x;
    for _ in 0..200 {
        if !overlaps(items, (x, y, w, h)) {
            break;
        }
        x += w / 2 + 24;
    }
    (x, y)
}

fn connector(from: &str, fs: &str, to: &str, ts: &str, label: Option<&str>, z: i64, now: &str) -> Value {
    let mut payload = json!({
        "from": { "itemId": from, "side": fs }, "to": { "itemId": to, "side": ts },
        "routing": "elbow", "head": "arrow", "stroke": "#e2ddf5", "strokeWidth": 2,
    });
    if let Some(l) = label.filter(|l| !l.is_empty()) {
        payload["label"] = json!(l);
    }
    json!({ "rotation": 0, "z_index": z, "payload": payload, "id": siddran_core::ids::uuid_v4(), "type": "connector", "x": 0, "y": 0, "w": 0, "h": 0, "created_at": now, "updated_at": now })
}

/// Facing sides for an arrow between two boxes: the dominant axis between centres.
fn facing(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64)) -> (&'static str, &'static str) {
    let (dx, dy) = ((b.0 + b.2 / 2) - (a.0 + a.2 / 2), (b.1 + b.3 / 2) - (a.1 + a.3 / 2));
    if dy.abs() >= dx.abs() {
        if dy >= 0 { ("s", "n") } else { ("n", "s") }
    } else if dx >= 0 {
        ("e", "w")
    } else {
        ("w", "e")
    }
}

fn edit(root: &Path, a: &Value) -> R {
    let what = need(a, "sandbox")?;
    let style = Style::default();
    writing(root, |_| {
        let mut b = find(root, what)?;
        let now = deadline::now_iso();
        let mut z = b.next_z();
        let mut added = Vec::new();
        let rect_of = |items: &[Value], id: &str| items.iter().find(|i| i["id"] == id).map(rect);

        for spec in a.get("add").and_then(Value::as_array).into_iter().flatten() {
            let text = spec["text"].as_str().unwrap_or("").to_owned();
            let kind = spec["shape"].as_str().unwrap_or("roundedRect");
            let id = siddran_core::ids::uuid_v4();
            let item = if kind == "text" {
                let w = (text.chars().count() as i64 * 10 + 24).clamp(80, 480);
                json!({ "rotation": 0, "z_index": z, "payload": { "text": text, "fontSize": 18, "color": style.text }, "id": id, "type": "text", "x": 0, "y": 0, "w": w, "h": 44, "created_at": now, "updated_at": now })
            } else {
                let (w, h, wrapped) = diagram::node_size(&text, kind, style.font);
                json!({ "rotation": 0, "z_index": z, "payload": {
                    "kind": kind, "fill": style.fill, "stroke": style.stroke, "strokeWidth": 2,
                    "radius": if kind == "roundedRect" { 10 } else { 0 },
                    "text": wrapped, "textAlign": "center", "textColor": style.text, "fontSize": style.font,
                }, "id": id, "type": "shape", "x": 0, "y": 0, "w": w, "h": h, "created_at": now, "updated_at": now })
            };
            let (w, h) = (item["w"].as_i64().unwrap_or(120), item["h"].as_i64().unwrap_or(56));
            let mut item = item;
            let after = match spec["after"].as_str() {
                Some(t) => Some(find_box(&b.items, t)?),
                None => None,
            };
            let (x, y) = match (spec["x"].as_f64(), spec["y"].as_f64(), &after) {
                (Some(x), Some(y), _) => (x.round() as i64, y.round() as i64),
                (_, _, Some(aid)) => {
                    let r = rect_of(&b.items, aid).unwrap_or((0, 0, w, h));
                    free_spot(&b.items, r.0 + r.2 / 2 - w / 2, r.1 + r.3 + 72, w, h)
                }
                _ => match b.bounds() {
                    Some((_, y0, x1, _)) => free_spot(&b.items, x1.round() as i64 + 120, y0.round() as i64, w, h),
                    None => (0, 0),
                },
            };
            item["x"] = json!(x);
            item["y"] = json!(y);
            b.items.push(item);
            z += 1;
            if let Some(aid) = after {
                let (fs, ts) = facing(rect_of(&b.items, &aid).unwrap(), (x, y, w, h));
                b.items.push(connector(&aid, fs, &id, ts, spec["label"].as_str(), z, &now));
                z += 1;
            }
            added.push(json!({ "id": id, "text": text }));
        }

        let mut connected = 0;
        for c in a.get("connect").and_then(Value::as_array).into_iter().flatten() {
            let from = find_box(&b.items, c["from"].as_str().unwrap_or(""))?;
            let to = find_box(&b.items, c["to"].as_str().unwrap_or(""))?;
            let (fs, ts) = facing(rect_of(&b.items, &from).unwrap(), rect_of(&b.items, &to).unwrap());
            b.items.push(connector(&from, fs, &to, ts, c["label"].as_str(), z, &now));
            z += 1;
            connected += 1;
        }

        let mut updated = 0;
        for u in a.get("update").and_then(Value::as_array).into_iter().flatten() {
            let id = find_box(&b.items, u["box"].as_str().unwrap_or(""))?;
            let it = b.items.iter_mut().find(|i| i["id"] == id.as_str()).unwrap();
            if let Some(t) = u["text"].as_str() {
                let kind = it["payload"]["kind"].as_str().unwrap_or("rect").to_owned();
                if it["type"] == "shape" {
                    // Grow the box to fit, never shrink it — the user may have sized it on purpose.
                    let (w, h, wrapped) = diagram::node_size(t, &kind, style.font);
                    it["payload"]["text"] = json!(wrapped);
                    if it["w"].as_f64().unwrap_or(0.0) < w as f64 { it["w"] = json!(w); }
                    if it["h"].as_f64().unwrap_or(0.0) < h as f64 { it["h"] = json!(h); }
                } else {
                    it["payload"]["text"] = json!(t);
                }
            }
            if let Some(k) = u["shape"].as_str() {
                it["payload"]["kind"] = json!(k);
            }
            if let Some(f) = u["fill"].as_str() {
                it["payload"]["fill"] = json!(f);
            }
            it["updated_at"] = json!(now);
            updated += 1;
        }

        let mut removed = 0;
        for w in a.get("remove").and_then(|v| v.as_array()).into_iter().flatten() {
            let w = w.as_str().unwrap_or("");
            let id = if b.items.iter().any(|i| i["id"] == w) { w.to_owned() } else { find_box(&b.items, w)? };
            let before = b.items.len();
            b.items.retain(|i| {
                let p = &i["payload"];
                !(i["id"] == id.as_str() || (i["type"] == "connector" && (p["from"]["itemId"] == id.as_str() || p["to"]["itemId"] == id.as_str())))
            });
            removed += before - b.items.len();
        }

        b.save().map_err(|e| e.to_string())?;
        Ok(json!({ "sandbox": b.title(), "added": added, "connected": connected, "updated": updated, "removed": removed }))
    })
}
