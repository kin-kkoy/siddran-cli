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
        tool("sandbox_read", "Read a board. Boxes and arrows come back as flowchart text (Mermaid-style); `box_ids` gives each key's id and real shape (the text only has a few bracket forms). Sticky notes are boxes too and are also listed under `sticky_notes`. `frames` are titled regions, each with what lies inside it (box keys, text ids, nested frame ids, counts of the rest). `formats` are widgets (Table, Kanban, Timeline, Matrix) holding records: their own (listed, each with its field values) or linked to the Bag's tasks (the filter is given; read the tasks themselves with the task tools). Other things (free text, note/task cards, handwriting, lines, PDFs) are listed or counted.", json!({
            "sandbox": { "type": "string", "description": "Board id or title" },
        }), &["sandbox"]),
        tool("sandbox_create", "Create an empty board.", json!({ "title": { "type": "string" } }), &["title"]),
        tool("sandbox_diagram", "Draw a flowchart from text and lay it out automatically. `shapes` sets any node to a shape the text can't express; `links` styles every arrow. Syntax (Mermaid flowchart subset): optional first line `flowchart TD` (top-down) or `flowchart LR`; nodes `A[Box]`, `B(Rounded)`, `C{Decision}`, `D((Circle))`, `E[(Database)]`, `F[/Input/]`, `G([Start/End])`, `H{{Prep}}`; edges `A --> B`, `A -->|label| B`, `A -- label --> B`, `A --- B` (no arrowhead), `A -.-> B` (dashed), chains `A --> B --> C`, fan-out `A --> B & C`. Goes on a board (placed beside what is there) or, with `note`, into a note as an embedded diagram.", json!({
            "text": { "type": "string", "description": "The flowchart" },
            "sandbox": { "type": "string", "description": "Existing board id or title. Omit (with `title`) to make a new board." },
            "title": { "type": "string", "description": "Title for a new board" },
            "note": { "type": "string", "description": "Instead of a board: append the diagram to this note (id or title)" },
            "shapes": { "type": "object", "additionalProperties": { "type": "string", "enum": diagram::BOX_KINDS }, "description": "Node key → shape kind, e.g. {\"D\": \"document\", \"S\": \"sticky\"}" },
            "links": link_props(),
        }), &["text"]),
        tool("sandbox_edit", "Change a board without coordinates. `add` boxes, sticky notes or frames (a box optionally `after` an existing box, which places it below and draws the arrow; a frame `around` boxes), `connect` boxes, `update` a box's text, shape or fill, a frame's title, or an arrow's label and style, `remove` things. Refer to boxes by id or by their text, frames by id or title, arrows by id.", json!({
            "sandbox": { "type": "string" },
            "add": { "type": "array", "items": { "type": "object", "properties": {
                "text": { "type": "string", "description": "The box's text; a frame's title" },
                "shape": { "type": "string", "enum": add_kinds(), "default": "roundedRect", "description": "A box shape, `sticky` (sticky note), `frame` (titled region) or `text` (free text)" },
                "fill": { "type": "string", "description": "#rrggbb or transparent" },
                "after": { "type": "string", "description": "Box (id or text) this one follows" },
                "label": { "type": "string", "description": "Label on the arrow from `after`" },
                "around": { "type": "array", "items": { "type": "string" }, "description": "Frames only: boxes (id or text) the frame is drawn around" },
                "x": { "type": "number" }, "y": { "type": "number" }, "w": { "type": "number" }, "h": { "type": "number" },
                "routing": link_props()["properties"]["routing"], "head": link_props()["properties"]["head"],
                "headStart": link_props()["properties"]["headStart"], "lineStyle": link_props()["properties"]["lineStyle"],
            }, "required": ["text"] } },
            "connect": { "type": "array", "items": { "type": "object", "properties": {
                "from": { "type": "string" }, "to": { "type": "string" }, "label": { "type": "string" },
                "routing": link_props()["properties"]["routing"], "head": link_props()["properties"]["head"],
                "headStart": link_props()["properties"]["headStart"], "lineStyle": link_props()["properties"]["lineStyle"],
            }, "required": ["from", "to"] } },
            "update": { "type": "array", "items": { "type": "object", "properties": {
                "box": { "type": "string", "description": "Box (id or text), frame (id or title) or arrow (id)" },
                "text": { "type": "string", "description": "A box's text, a frame's title, an arrow's label" },
                "shape": { "type": "string", "enum": diagram::BOX_KINDS }, "fill": { "type": "string", "description": "#rrggbb or transparent" },
                "routing": link_props()["properties"]["routing"], "head": link_props()["properties"]["head"],
                "headStart": link_props()["properties"]["headStart"], "lineStyle": link_props()["properties"]["lineStyle"],
            }, "required": ["box"] } },
            "remove": { "type": "array", "items": { "type": "string" }, "description": "Boxes (id or text) or connector ids. A box's arrows go with it." },
        }), &["sandbox"]),
    ]
}

fn add_kinds() -> Vec<&'static str> {
    let mut k = diagram::BOX_KINDS.to_vec();
    k.extend([diagram::FRAME_KIND, "text"]);
    k
}

/// An arrow's look, as the desktop's floating bar sets it.
fn link_props() -> Value {
    json!({ "type": "object", "properties": {
        "routing": { "type": "string", "enum": diagram::ROUTINGS, "default": "elbow" },
        "head": { "type": "string", "enum": diagram::HEADS, "description": "Arrowhead at the end", "default": "arrow" },
        "headStart": { "type": "string", "enum": diagram::HEADS, "description": "Arrowhead at the start", "default": "none" },
        "lineStyle": { "type": "string", "enum": diagram::LINE_STYLES, "default": "solid" },
    } })
}

/// Apply `routing` / `head` / `headStart` / `lineStyle` from `spec` to a
/// connector payload. Only what is given changes.
fn style_link(spec: &Value, p: &mut Value) -> Result<(), String> {
    let get = |k: &str, allowed: &[&str]| -> Result<Option<String>, String> {
        match s(spec, k) {
            None => Ok(None),
            Some(v) if allowed.contains(&v) => Ok(Some(v.to_owned())),
            Some(v) => Err(format!("`{k}` {v:?} is not one of {}", allowed.join(", "))),
        }
    };
    if let Some(r) = get("routing", diagram::ROUTINGS)? {
        p["routing"] = json!(r);
    }
    if let Some(h) = get("head", diagram::HEADS)? {
        p["head"] = json!(h);
    }
    if let Some(h) = get("headStart", diagram::HEADS)? {
        match h.as_str() {
            "none" => {
                if let Some(o) = p.as_object_mut() {
                    o.remove("headStart");
                }
            }
            _ => p["headStart"] = json!(h),
        }
    }
    if let Some(l) = get("lineStyle", diagram::LINE_STYLES)? {
        // `lineStyle` wins over the older `dash` flag on the desktop; drop it so the two can't disagree.
        if let Some(o) = p.as_object_mut() {
            o.remove("dash");
        }
        p["lineStyle"] = json!(l);
    }
    Ok(())
}

fn box_kind(k: &str) -> Result<&'static str, String> {
    diagram::BOX_KINDS.iter().find(|x| **x == k).copied().ok_or_else(|| format!("unknown shape {k:?}; use one of {}", diagram::BOX_KINDS.join(", ")))
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

/// What `other` calls a kind of item.
fn other_name(it: &Value) -> String {
    if diagram::is_format(it) {
        return "formats".into();
    }
    match it["type"].as_str().unwrap_or("") {
        "stroke" => "handwriting strokes".into(),
        "shape" => "lines".into(),
        t => t.to_owned(),
    }
}

/// The frame an item sits in: the one it's attached to, else the smallest one holding its centre (the
/// desktop's `frameChildren` rule — rotation turns about the centre, so the
/// centre is just x + w/2, y + h/2). A frame sits in another only when it
/// fits wholly inside it, so two overlapping frames never hold each other.
fn frame_of<'a>(it: &Value, frames: &[&'a Value]) -> Option<&'a Value> {
    // Attached to a frame (the desktop's sticky attachment, `payload.attachedTo`):
    // it belongs there wherever it hangs.
    if let Some(id) = it["payload"]["attachedTo"].as_str() {
        if let Some(fr) = frames.iter().find(|fr| fr["id"] == id) {
            return Some(*fr);
        }
    }
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
    let (x, y, w, h) = (f(it, "x"), f(it, "y"), f(it, "w"), f(it, "h"));
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let holds = |fr: &Value, px: f64, py: f64| px >= f(fr, "x") && px <= f(fr, "x") + f(fr, "w") && py >= f(fr, "y") && py <= f(fr, "y") + f(fr, "h");
    let whole = diagram::is_frame(it);
    frames
        .iter()
        .filter(|fr| fr["id"] != it["id"])
        .filter(|fr| if whole { holds(fr, x, y) && holds(fr, x + w, y + h) } else { holds(fr, cx, cy) })
        .min_by(|a, b| (f(a, "w") * f(a, "h")).total_cmp(&(f(b, "w") * f(b, "h"))))
        .copied()
}

fn read(root: &Path, a: &Value) -> R {
    let b = find(root, need(a, "sandbox")?)?;
    let (flow, keys) = diagram::describe(&b.items);
    let key_of: std::collections::HashMap<&str, &str> = keys.iter().map(|(k, id)| (id.as_str(), k.as_str())).collect();
    let frames: Vec<&Value> = b.items.iter().filter(|i| diagram::is_frame(i)).collect();
    let mut inside: Vec<Value> = frames
        .iter()
        .map(|fr| json!({ "id": fr["id"], "title": fr["payload"]["title"].as_str().unwrap_or("Frame"), "boxes": [], "text_items": [], "frames": [], "other": {} }))
        .collect();
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut texts = Vec::new();
    let mut stickies = Vec::new();
    for it in &b.items {
        if it["type"] == "connector" {
            continue; // they follow their ends
        }
        let id = it["id"].as_str().unwrap_or("");
        let key = key_of.get(id).copied();
        let slot = frame_of(it, &frames).and_then(|fr| frames.iter().position(|f| f["id"] == fr["id"])).map(|i| &mut inside[i]);
        let (list, entry) = if diagram::is_frame(it) {
            ("frames", json!(id))
        } else if let Some(k) = key {
            if it["payload"]["kind"] == "sticky" {
                stickies.push(json!({ "key": k, "id": id, "text": it["payload"]["text"].as_str().unwrap_or(""), "color": it["payload"]["fill"] }));
            }
            ("boxes", json!(k))
        } else if it["type"] == "text" {
            texts.push(json!({ "id": it["id"], "text": it["payload"]["text"] }));
            ("text_items", json!(id))
        } else {
            let name = other_name(it);
            *counts.entry(name.clone()).or_default() += 1;
            if let Some(fr) = slot {
                let n = fr["other"][&name].as_u64().unwrap_or(0);
                fr["other"][&name] = json!(n + 1);
            }
            continue;
        };
        if let Some(fr) = slot {
            fr[list].as_array_mut().unwrap().push(entry);
        }
    }
    // Format widgets (Siddran-Desktop formats/formats.js): their own records
    // are in the payload; a linked one only names its filter.
    let formats: Vec<Value> = b
        .items
        .iter()
        .filter(|i| diagram::is_format(i))
        .map(|f| {
            let p = &f["payload"];
            let mut o = json!({ "id": f["id"], "title": p["title"].as_str().unwrap_or("Table"), "view": p["view"].as_str().unwrap_or("table"), "source": p["source"].get("type").map_or(json!({ "type": "own" }), |_| p["source"].clone()) });
            if !matches!(p["source"]["type"].as_str(), Some("tasks" | "project")) {
                o["fields"] = json!(p["fields"].as_array().map(|fs| fs.iter().map(|x| x["name"].clone()).collect::<Vec<_>>()).unwrap_or_default());
                o["records"] = json!(p["records"].as_array().map(|rs| rs.iter().map(|r| json!({ "id": r["id"], "values": r["values"] })).collect::<Vec<_>>()).unwrap_or_default());
            }
            o
        })
        .collect();
    let shape_of = |id: &str| b.items.iter().find(|i| i["id"] == id).and_then(|i| i["payload"]["kind"].as_str()).unwrap_or("rect").to_owned();
    Ok(json!({
        "id": b.id(), "title": b.title(),
        "flowchart": flow,
        "box_ids": keys.iter().map(|(k, id)| json!({ "key": k, "id": id, "shape": shape_of(id) })).collect::<Vec<_>>(),
        "sticky_notes": stickies,
        "frames": inside,
        "formats": formats,
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
    let (mut g, problems) = diagram::parse(text)?;
    if let Some(m) = a.get("shapes").and_then(Value::as_object) {
        for (key, kind) in m {
            let kind = box_kind(kind.as_str().unwrap_or(""))?;
            let n = g.nodes.iter_mut().find(|n| &n.key == key).ok_or_else(|| format!("`shapes` names {key:?}, which is not a node in the text"))?;
            n.kind = kind;
        }
    }
    let links = a.get("links").cloned().unwrap_or(Value::Null);
    let style_all = |items: &mut [Value]| -> Result<(), String> {
        for it in items.iter_mut().filter(|i| i["type"] == "connector") {
            let mut spec = links.clone();
            // An edge's own `---` / `-.->` in the text wins over the board-wide style.
            if it["payload"]["head"] == "none" {
                if let Some(o) = spec.as_object_mut() {
                    o.remove("head");
                }
            }
            if it["payload"]["dash"] == true {
                if let Some(o) = spec.as_object_mut() {
                    o.remove("lineStyle");
                }
            }
            style_link(&spec, &mut it["payload"])?;
        }
        Ok(())
    };
    let dir = g.dir.unwrap_or(Dir::Down);
    let now = deadline::now_iso();

    if let Some(n) = s(a, "note") {
        let mut placed = diagram::layout(&g, dir, (0, 0), &Style::default(), &now, siddran_core::ids::uuid_v4);
        style_all(&mut placed.items)?;
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
        style_all(&mut placed.items)?;
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

/// A box on the board by id, or by its text (exact, then prefix, then
/// substring); a frame by id or title.
fn find_box(items: &[Value], what: &str) -> Result<String, String> {
    let shapes: Vec<&Value> = items.iter().filter(|i| i["type"] == "shape" || i["type"] == "text").collect();
    let hit = pick(
        "box",
        what,
        &shapes,
        |i| i["id"].as_str().unwrap_or("").to_owned(),
        |i| {
            let p = &i["payload"];
            if diagram::is_frame(i) || diagram::is_format(i) { p["title"].as_str() } else { p["text"].as_str() }.unwrap_or("").replace('\n', " ")
        },
    )?;
    Ok(hit["id"].as_str().unwrap_or("").to_owned())
}

fn rect(it: &Value) -> (i64, i64, i64, i64) {
    let g = |k: &str| it[k].as_f64().unwrap_or(0.0).round() as i64;
    (g("x"), g("y"), g("w"), g("h"))
}

fn overlaps(items: &[Value], r: (i64, i64, i64, i64)) -> bool {
    // Frames are regions, not obstacles: a box may go inside one.
    items.iter().filter(|i| i["type"] != "connector" && !diagram::is_frame(i)).any(|i| {
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

/// An arrow, labelled and styled from `spec` (`label`, `routing`, `head`, `headStart`, `lineStyle`).
fn connector(from: &str, fs: &str, to: &str, ts: &str, spec: &Value, z: i64, now: &str) -> Result<Value, String> {
    let mut payload = json!({
        "from": { "itemId": from, "side": fs }, "to": { "itemId": to, "side": ts },
        "routing": "elbow", "head": "arrow", "stroke": "#e2ddf5", "strokeWidth": 2,
    });
    if let Some(l) = s(spec, "label") {
        payload["label"] = json!(l);
    }
    style_link(spec, &mut payload)?;
    Ok(json!({ "rotation": 0, "z_index": z, "payload": payload, "id": siddran_core::ids::uuid_v4(), "type": "connector", "x": 0, "y": 0, "w": 0, "h": 0, "created_at": now, "updated_at": now }))
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

/// Siddran-Desktop text/textStyle.js STICKY_DEFAULT.
const STICKY_DEFAULT: &str = "#FFE45C";

/// A frame (frames.js `framePayload`), at the bottom of the stack like the
/// desktop's: `around` boxes with 40 px to spare (its "Frame selection"), at
/// `x`/`y`/`w`/`h`, or in free space beside the board.
fn frame(items: &[Value], id: &str, title: &str, spec: &Value, now: &str) -> Result<Value, String> {
    const PAD: i64 = 40;
    let around: Vec<&str> = spec.get("around").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let (x, y, w, h) = if !around.is_empty() {
        let mut r: Option<(i64, i64, i64, i64)> = None;
        for what in around {
            let bid = find_box(items, what)?;
            let (x, y, w, h) = rect(items.iter().find(|i| i["id"] == bid.as_str()).unwrap());
            r = Some(match r {
                None => (x, y, x + w, y + h),
                Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x + w), d.max(y + h)),
            });
        }
        let (x0, y0, x1, y1) = r.unwrap();
        (x0 - PAD, y0 - PAD, x1 - x0 + PAD * 2, y1 - y0 + PAD * 2)
    } else {
        let g = |k: &str, d: f64| spec[k].as_f64().unwrap_or(d).round() as i64;
        let (w, h) = (g("w", 640.0).max(40), g("h", 400.0).max(40));
        match (spec["x"].as_f64(), spec["y"].as_f64()) {
            (Some(_), Some(_)) => (g("x", 0.0), g("y", 0.0), w, h),
            _ => {
                let bounds = items.iter().filter(|i| i["type"] != "connector").map(rect).reduce(|a, b| {
                    let (x0, y0, x1, y1) = (a.0.min(b.0), a.1.min(b.1), (a.0 + a.2).max(b.0 + b.2), (a.1 + a.3).max(b.1 + b.3));
                    (x0, y0, x1 - x0, y1 - y0)
                });
                match bounds {
                    Some((bx, by, bw, _)) => (bx + bw + 160, by, w, h),
                    None => (0, 0, w, h),
                }
            }
        }
    };
    let z = items.iter().filter_map(|i| i["z_index"].as_i64()).min().unwrap_or(0).min(0) - 1;
    let fill = s(spec, "fill").unwrap_or("rgba(128,128,150,0.10)");
    let title = if title.trim().is_empty() { "Frame" } else { title };
    Ok(json!({ "rotation": 0, "z_index": z, "payload": {
        "kind": diagram::FRAME_KIND, "title": title, "fill": fill, "stroke": "rgba(160,160,190,0.35)", "strokeWidth": 1,
    }, "id": id, "type": "shape", "x": x, "y": y, "w": w, "h": h, "created_at": now, "updated_at": now }))
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
            if kind == diagram::FRAME_KIND {
                added.push(json!({ "id": id, "text": text }));
                b.items.push(frame(&b.items, &id, &text, spec, &now)?);
                continue;
            }
            let item = if kind == "text" {
                let w = (text.chars().count() as i64 * 10 + 24).clamp(80, 480);
                json!({ "rotation": 0, "z_index": z, "payload": { "text": text, "fontSize": 18, "color": style.text }, "id": id, "type": "text", "x": 0, "y": 0, "w": w, "h": 44, "created_at": now, "updated_at": now })
            } else if kind == "sticky" {
                // The desktop's own sticky (templates.js `sticky`): a 192 px square, dark text picked from the fill.
                let fill = s(spec, "fill").unwrap_or(STICKY_DEFAULT);
                json!({ "rotation": 0, "z_index": z, "payload": { "kind": "sticky", "fill": fill, "text": text, "textAlign": "center" }, "id": id, "type": "shape", "x": 0, "y": 0, "w": 192, "h": 192, "created_at": now, "updated_at": now })
            } else {
                let kind = box_kind(kind)?;
                let (w, h, wrapped) = diagram::node_size(&text, kind, style.font);
                json!({ "rotation": 0, "z_index": z, "payload": {
                    "kind": kind, "fill": style.fill, "stroke": style.stroke, "strokeWidth": 2,
                    "radius": if kind == "roundedRect" { 10 } else { 0 },
                    "text": wrapped, "textAlign": "center", "textColor": style.text, "fontSize": style.font,
                }, "id": id, "type": "shape", "x": 0, "y": 0, "w": w, "h": h, "created_at": now, "updated_at": now })
            };
            let mut item = item;
            if kind != "text" && kind != "sticky" {
                if let Some(f) = s(spec, "fill") {
                    item["payload"]["fill"] = json!(f);
                }
            }
            for k in ["w", "h"] {
                if let Some(v) = spec[k].as_f64() {
                    item[k] = json!(v.round().max(8.0) as i64);
                }
            }
            let (w, h) = (item["w"].as_i64().unwrap_or(120), item["h"].as_i64().unwrap_or(56));
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
                b.items.push(connector(&aid, fs, &id, ts, spec, z, &now)?);
                z += 1;
            }
            added.push(json!({ "id": id, "text": text }));
        }

        let mut connected = 0;
        for c in a.get("connect").and_then(Value::as_array).into_iter().flatten() {
            let from = find_box(&b.items, c["from"].as_str().unwrap_or(""))?;
            let to = find_box(&b.items, c["to"].as_str().unwrap_or(""))?;
            let (fs, ts) = facing(rect_of(&b.items, &from).unwrap(), rect_of(&b.items, &to).unwrap());
            b.items.push(connector(&from, fs, &to, ts, c, z, &now)?);
            z += 1;
            connected += 1;
        }

        let mut updated = 0;
        for u in a.get("update").and_then(Value::as_array).into_iter().flatten() {
            let what = u["box"].as_str().unwrap_or("");
            if let Some(cn) = b.items.iter_mut().find(|i| i["type"] == "connector" && i["id"] == what) {
                if let Some(t) = u["text"].as_str() {
                    match t.trim() {
                        "" => {
                            if let Some(o) = cn["payload"].as_object_mut() {
                                o.remove("label");
                            }
                        }
                        l => cn["payload"]["label"] = json!(l),
                    }
                }
                style_link(u, &mut cn["payload"])?;
                cn["updated_at"] = json!(now);
                updated += 1;
                continue;
            }
            let id = find_box(&b.items, what)?;
            let it = b.items.iter_mut().find(|i| i["id"] == id.as_str()).unwrap();
            let is_frame = diagram::is_frame(it);
            if is_frame && u.get("shape").is_some() {
                return Err(format!("{what:?} is a frame; a frame can't change shape"));
            }
            // A Format widget: only its title changes here; its records stay as they are.
            if diagram::is_format(it) {
                if u.get("shape").is_some() || u.get("fill").is_some() {
                    return Err(format!("{what:?} is a widget (a table of records); only its title can change"));
                }
                if let Some(t) = u["text"].as_str() {
                    it["payload"]["title"] = json!(t);
                }
                it["updated_at"] = json!(now);
                updated += 1;
                continue;
            }
            if let Some(t) = u["text"].as_str() {
                let kind = it["payload"]["kind"].as_str().unwrap_or("rect").to_owned();
                if is_frame {
                    it["payload"]["title"] = json!(t);
                } else if it["type"] == "shape" && kind != "sticky" {
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
                it["payload"]["kind"] = json!(box_kind(k)?);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A Bag with one board holding `items`, as the desktop would have written it.
    fn bag(items: Value) -> (tempfile::TempDir, String) {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("notes")).unwrap();
        let mut b = sandbox::create(t.path(), "Board").unwrap();
        b.items = items.as_array().unwrap().clone();
        b.save().unwrap();
        let id = b.id().to_owned();
        (t, id)
    }

    fn items(root: &Path, id: &str) -> Vec<Value> {
        sandbox::read(&sandbox::dir(root).join(format!("{id}.siddran"))).unwrap().items
    }

    fn shape(id: &str, kind: &str, x: i64, y: i64, w: i64, h: i64) -> Value {
        json!({ "rotation": 0, "z_index": 1, "payload": { "kind": kind, "text": id }, "id": id, "type": "shape", "x": x, "y": y, "w": w, "h": h })
    }

    fn by_id<'a>(items: &'a [Value], id: &str) -> &'a Value {
        items.iter().find(|i| i["id"] == id).unwrap()
    }

    #[test]
    fn read_lists_frames_with_their_contents_and_stickies_as_sticky_notes() {
        let mut outer = shape("F1", "frame", 0, 0, 1000, 600);
        outer["payload"] = json!({ "kind": "frame", "title": "Sprint" });
        let mut inner = shape("F2", "frame", 500, 50, 400, 400);
        inner["payload"] = json!({ "kind": "frame", "title": "Done" });
        let mut sticky = shape("S", "sticky", 600, 100, 192, 192);
        sticky["payload"]["fill"] = json!("#FFE45C");
        let (t, id) = bag(json!([
            outer, inner, sticky,
            shape("A", "rect", 40, 40, 120, 56),
            shape("L", "arrow", 40, 300, 200, 0),
            { "id": "T", "type": "text", "x": 60, "y": 200, "w": 100, "h": 40, "payload": { "text": "hello" } },
            { "id": "K", "type": "stroke", "x": 700, "y": 350, "w": 10, "h": 10, "payload": {} },
            shape("OUT", "rect", 2000, 800, 120, 56),
            { "id": "C", "type": "connector", "x": 0, "y": 0, "w": 0, "h": 0, "payload": { "from": { "itemId": "A" }, "to": { "itemId": "S" }, "head": "arrow" } },
        ]));
        let r = read(t.path(), &json!({ "sandbox": id })).unwrap();
        // Frames and lines are not boxes; the sticky is, and links like one.
        assert_eq!(r["flowchart"], "flowchart TD\n  n1[A]\n  n2[S]\n  n3[OUT]\n  n1 --> n2\n");
        assert_eq!(r["box_ids"][1], json!({ "key": "n2", "id": "S", "shape": "sticky" }));
        assert_eq!(r["sticky_notes"], json!([{ "key": "n2", "id": "S", "text": "S", "color": "#FFE45C" }]));
        // Each thing goes in the smallest frame holding its centre.
        assert_eq!(r["frames"], json!([
            { "id": "F1", "title": "Sprint", "boxes": ["n1"], "text_items": ["T"], "frames": ["F2"], "other": { "lines": 1 } },
            { "id": "F2", "title": "Done", "boxes": ["n2"], "text_items": [], "frames": [], "other": { "handwriting strokes": 1 } },
        ]));
        assert_eq!(r["other"], json!({ "handwriting strokes": 1, "lines": 1 }));
    }

    #[test]
    fn read_puts_things_attached_to_a_frame_in_it_even_when_they_hang_outside() {
        let mut fr = shape("F", "frame", 0, 0, 200, 200);
        fr["payload"] = json!({ "kind": "frame", "title": "Small", "overflow": "clip" });
        let mut hanging = shape("A", "rect", 500, 500, 120, 56);
        hanging["payload"]["attachedTo"] = json!("F");
        let (t, id) = bag(json!([fr, hanging, shape("B", "rect", 900, 900, 120, 56)]));
        let r = read(t.path(), &json!({ "sandbox": id })).unwrap();
        assert_eq!(r["frames"][0]["boxes"], json!(["n1"]));
        // And an edit keeps the attachment and the crop setting.
        edit(t.path(), &json!({ "sandbox": id, "update": [{ "box": "A", "text": "Renamed" }] })).unwrap();
        let all = items(t.path(), &id);
        assert_eq!(by_id(&all, "A")["payload"]["attachedTo"], "F");
        assert_eq!(by_id(&all, "F")["payload"]["overflow"], "clip");
    }

    #[test]
    fn edit_adds_stickies_frames_and_styled_links() {
        let (t, id) = bag(json!([shape("A", "rect", 0, 0, 120, 56)]));
        let r = edit(t.path(), &json!({ "sandbox": id,
            "add": [
                { "text": "Report", "shape": "document", "after": "A", "headStart": "circle", "lineStyle": "dotted", "routing": "curved" },
                { "text": "Idea", "shape": "sticky", "fill": "#a9d8ff" },
            ],
        })).unwrap();
        let (doc, note) = (r["added"][0]["id"].as_str().unwrap().to_owned(), r["added"][1]["id"].as_str().unwrap().to_owned());
        let r = edit(t.path(), &json!({ "sandbox": id, "add": [{ "text": "Plan", "shape": "frame", "around": ["A", "Report"] }] })).unwrap();
        let fid = r["added"][0]["id"].as_str().unwrap().to_owned();
        let all = items(t.path(), &id);

        assert_eq!(by_id(&all, &doc)["payload"]["kind"], "document");
        let s = by_id(&all, &note);
        assert_eq!(s["payload"], json!({ "kind": "sticky", "fill": "#a9d8ff", "text": "Idea", "textAlign": "center" }));
        assert_eq!((s["w"].as_i64(), s["h"].as_i64()), (Some(192), Some(192)));

        let link = all.iter().find(|i| i["type"] == "connector").unwrap();
        assert_eq!(link["payload"]["routing"], "curved");
        assert_eq!(link["payload"]["headStart"], "circle");
        assert_eq!(link["payload"]["lineStyle"], "dotted");
        assert_eq!(link["payload"]["head"], "arrow");

        let f = by_id(&all, &fid);
        assert_eq!(f["payload"]["kind"], "frame");
        assert_eq!(f["payload"]["title"], "Plan");
        let (dx, dy, dw, dh) = rect(by_id(&all, &doc));
        assert_eq!((f["x"].as_i64(), f["y"].as_i64()), (Some(dx.min(0) - 40), Some(-40)));
        assert_eq!(f["w"].as_i64().unwrap(), (dx + dw).max(120) - dx.min(0) + 80);
        assert_eq!(f["h"].as_i64().unwrap(), dy + dh + 80);
        assert!(f["z_index"].as_i64().unwrap() < 1, "a frame goes beneath what it holds");

        // Frames are found by title and renamed through `text`; arrows restyled by id.
        let lid = link["id"].as_str().unwrap().to_owned();
        edit(t.path(), &json!({ "sandbox": id, "update": [
            { "box": "Plan", "text": "Plan B" },
            { "box": lid, "text": "next", "headStart": "none", "head": "diamond", "lineStyle": "dashed" },
        ] })).unwrap();
        let all = items(t.path(), &id);
        assert_eq!(by_id(&all, &fid)["payload"]["title"], "Plan B");
        let p = &by_id(&all, &lid)["payload"];
        assert_eq!((p["label"].as_str(), p["head"].as_str(), p["lineStyle"].as_str()), (Some("next"), Some("diamond"), Some("dashed")));
        assert!(p.get("headStart").is_none());
    }

    #[test]
    fn edit_refuses_shapes_the_board_cannot_draw() {
        let (t, id) = bag(json!([shape("A", "rect", 0, 0, 120, 56)]));
        let e = edit(t.path(), &json!({ "sandbox": id, "add": [{ "text": "x", "shape": "blob" }] })).unwrap_err();
        assert!(e.contains("unknown shape"), "{e}");
        let e = edit(t.path(), &json!({ "sandbox": id, "update": [{ "box": "A", "shape": "squircle" }] })).unwrap_err();
        assert!(e.contains("unknown shape"), "{e}");
        let e = edit(t.path(), &json!({ "sandbox": id, "connect": [{ "from": "A", "to": "A", "routing": "wiggly" }] })).unwrap_err();
        assert!(e.contains("routing"), "{e}");
        assert_eq!(items(t.path(), &id).len(), 1, "nothing written on a refusal");
    }

    #[test]
    fn new_boxes_may_land_inside_a_frame() {
        let mut fr = shape("F", "frame", -100, -100, 800, 800);
        fr["payload"] = json!({ "kind": "frame", "title": "Area" });
        let (t, id) = bag(json!([fr, shape("A", "rect", 0, 0, 120, 56)]));
        let r = edit(t.path(), &json!({ "sandbox": id, "add": [{ "text": "B", "after": "A" }] })).unwrap();
        let all = items(t.path(), &id);
        let (x, y, ..) = rect(by_id(&all, r["added"][0]["id"].as_str().unwrap()));
        assert_eq!((x, y), (0, 128), "straight below A, not pushed out of the frame");
    }

    #[test]
    fn diagram_takes_extra_shapes_and_a_link_style() {
        let (t, id) = bag(json!([]));
        let r = draw(t.path(), &json!({ "sandbox": id, "text": "A[Spec] --> B[Notes]\nB -.-> C[Done]\nC --- A",
            "shapes": { "A": "document", "B": "sticky" },
            "links": { "routing": "curved", "head": "triangle", "headStart": "bar", "lineStyle": "dotted" } })).unwrap();
        assert_eq!(r["boxes"].as_array().unwrap().len(), 3);
        let all = items(t.path(), &id);
        let kinds: Vec<_> = all.iter().filter(|i| i["type"] == "shape").map(|i| i["payload"]["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["document", "sticky", "rect"]);
        let links: Vec<&Value> = all.iter().filter(|i| i["type"] == "connector").map(|i| &i["payload"]).collect();
        assert!(links.iter().all(|p| p["routing"] == "curved" && p["headStart"] == "bar"));
        // The text's own `-.->` and `---` win over the board-wide style.
        assert_eq!((links[0]["head"].as_str(), links[0]["lineStyle"].as_str()), (Some("triangle"), Some("dotted")));
        assert_eq!((links[1]["dash"].as_bool(), links[1].get("lineStyle")), (Some(true), None));
        assert_eq!(links[2]["head"], "none");

        let e = draw(t.path(), &json!({ "sandbox": id, "text": "A --> B", "shapes": { "Z": "rect" } })).unwrap_err();
        assert!(e.contains("\"Z\""), "{e}");
    }

    #[test]
    fn read_lists_format_widgets_and_an_edit_keeps_their_records() {
        let mut own = shape("W", "format", 0, 300, 420, 320);
        own["payload"] = json!({ "kind": "format", "view": "table", "title": "Reading list", "source": { "type": "own" },
            "fields": [{ "id": "title", "name": "Title", "type": "text" }, { "id": "status", "name": "Status", "type": "select", "options": ["To do", "Done"] }],
            "records": [{ "id": "r1", "values": { "title": "Dune", "status": "Done" }, "updated_at": "2026-11-13T00:00:00.000Z" }] });
        let mut linked = shape("L", "format", 600, 300, 420, 320);
        linked["payload"] = json!({ "kind": "format", "view": "kanban", "title": "Sprint", "source": { "type": "tasks", "show": "open" }, "fields": [], "records": [] });
        let mut fr = shape("F", "frame", -100, 200, 600, 600);
        fr["payload"] = json!({ "kind": "frame", "title": "Area" });
        let (t, id) = bag(json!([fr, shape("A", "rect", 0, 0, 120, 56), own.clone(), linked]));
        let r = read(t.path(), &json!({ "sandbox": id })).unwrap();
        assert_eq!(r["flowchart"], "flowchart TD\n  n1[A]\n", "widgets are not boxes");
        assert_eq!(r["formats"], json!([
            { "id": "W", "title": "Reading list", "view": "table", "source": { "type": "own" }, "fields": ["Title", "Status"],
              "records": [{ "id": "r1", "values": { "title": "Dune", "status": "Done" } }] },
            { "id": "L", "title": "Sprint", "view": "kanban", "source": { "type": "tasks", "show": "open" } },
        ]));
        assert_eq!(r["frames"][0]["other"], json!({ "formats": 1 }));

        // An edit elsewhere, and a rename of the widget, leave its records byte for byte.
        edit(t.path(), &json!({ "sandbox": id, "update": [{ "box": "A", "text": "Renamed" }, { "box": "Reading list", "text": "Books" }], "add": [{ "text": "X" }] })).unwrap();
        let all = items(t.path(), &id);
        let w = &by_id(&all, "W")["payload"];
        assert_eq!(w["title"], "Books");
        assert_eq!((w["records"].clone(), w["fields"].clone()), (own["payload"]["records"].clone(), own["payload"]["fields"].clone()));
        assert_eq!((by_id(&all, "W")["w"].clone(), by_id(&all, "W")["h"].clone()), (json!(420), json!(320)), "not resized like a box");
        let e = edit(t.path(), &json!({ "sandbox": id, "update": [{ "box": "W", "shape": "rect" }] })).unwrap_err();
        assert!(e.contains("widget"), "{e}");
    }

    #[test]
    fn desktop_only_fields_survive_an_edit() {
        let mut a = shape("A", "rect", 0, 0, 120, 56);
        a["payload"]["flowDrop"] = json!(2);
        let link = json!({ "id": "C", "type": "connector", "x": 0, "y": 0, "w": 0, "h": 0, "payload": {
            "from": { "itemId": "A", "side": "s" }, "to": { "itemId": "B", "side": "n" },
            "routing": "curved", "avoid": true, "body": "tapered", "bend": { "x": 3, "y": 4 } } });
        let (t, id) = bag(json!([a, shape("B", "rect", 0, 200, 120, 56), link.clone()]));
        edit(t.path(), &json!({ "sandbox": id, "update": [{ "box": "A", "text": "Renamed" }, { "box": "C", "text": "go" }], "add": [{ "text": "X" }] })).unwrap();
        let all = items(t.path(), &id);
        assert_eq!(by_id(&all, "A")["payload"]["flowDrop"], 2);
        let p = &by_id(&all, "C")["payload"];
        assert_eq!((p["avoid"].clone(), p["body"].clone(), p["bend"].clone()), (json!(true), json!("tapered"), json!({ "x": 3, "y": 4 })));
        assert_eq!(p["label"], "go");
    }
}
