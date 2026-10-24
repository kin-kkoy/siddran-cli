//! Flowchart text → laid-out Sandbox items.
//!
//! Placing boxes and arrows one coordinate at a time is the slow way to draw a
//! flowchart — for a person without a pen, and even more so for an AI, which
//! cannot see where things landed. So diagrams can be *written* instead, in the
//! subset of Mermaid's flowchart syntax people already know:
//!
//! ```text
//! flowchart TD
//!   A[Start] --> B{Logged in?}
//!   B -->|yes| C(Dashboard)
//!   B -->|no| D[/Login form/]
//!   D --> B
//! ```
//!
//! and laid out here: layered top-down (or left-right), with each box under the
//! boxes that lead to it. The output is ordinary Sandbox items — `shape` and
//! `connector`, the same JSON the desktop's canvas writes — so the result is
//! fully editable afterwards, and connectors stay attached when boxes move.
//!
//! Coordinates are whole numbers on purpose: serde_json writes `120.0` where the
//! desktop writes `120` (see `json.rs`).

use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Down,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub label: String,
    /// A Sandbox shape kind from `shapes/registry.js`.
    pub kind: &'static str,
    /// Corner radius; a stadium is a rounded rect whose radius is half its height.
    pub round: Round,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Round {
    None,
    Some,
    Pill,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub head: bool,
    pub dashed: bool,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub dir: Option<Dir>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    fn node(&mut self, key: &str, shape: Option<(String, &'static str, Round)>) {
        match self.nodes.iter_mut().find(|n| n.key == key) {
            Some(n) => {
                if let Some((label, kind, round)) = shape {
                    n.label = label;
                    n.kind = kind;
                    n.round = round;
                }
            }
            None => {
                let (label, kind, round) = shape.unwrap_or_else(|| (key.to_owned(), "rect", Round::Some));
                self.nodes.push(Node { key: key.to_owned(), label, kind, round });
            }
        }
    }
}

// ── parsing ─────────────────────────────────────────────────────────

/// Parse flowchart text. Lenient: a line it cannot read is reported, not fatal,
/// so one typo does not throw away a twenty-line diagram.
pub fn parse(text: &str) -> Result<(Graph, Vec<String>), String> {
    let mut g = Graph::default();
    let mut problems = Vec::new();
    for raw in text.lines().flat_map(|l| l.split(';')) {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("%%") || line.starts_with("```") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("flowchart").or_else(|| lower.strip_prefix("graph")) {
            g.dir = Some(match rest.trim() {
                "lr" | "rl" => Dir::Right,
                _ => Dir::Down,
            });
            continue;
        }
        // Styling directives are Mermaid's, not ours; skip rather than misread.
        if ["classdef ", "class ", "style ", "linkstyle ", "click ", "subgraph", "end", "direction "]
            .iter()
            .any(|k| lower == k.trim() || lower.starts_with(k))
        {
            continue;
        }
        // All or nothing per line: a line that fails halfway must not leave its
        // first node behind as a stray box.
        let mut trial = Graph { dir: g.dir, nodes: g.nodes.clone(), edges: g.edges.clone() };
        match statement(line, &mut trial) {
            Ok(()) => g = trial,
            Err(e) => problems.push(format!("{line:?}: {e}")),
        }
    }
    if g.nodes.is_empty() {
        return Err(if problems.is_empty() {
            "no nodes — write something like `A[Start] --> B[Next]`".into()
        } else {
            format!("could not read any of it: {}", problems.join("; "))
        });
    }
    Ok((g, problems))
}

struct Cursor<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn ws(&mut self) {
        while self.rest().starts_with([' ', '\t']) {
            self.i += 1;
        }
    }
    fn eat(&mut self, p: &str) -> bool {
        if self.rest().starts_with(p) {
            self.i += p.len();
            true
        } else {
            false
        }
    }
    fn done(&self) -> bool {
        self.i >= self.s.len()
    }
}

fn statement(line: &str, g: &mut Graph) -> Result<(), String> {
    let mut c = Cursor { s: line, i: 0 };
    let mut left = node_group(&mut c, g)?;
    loop {
        c.ws();
        if c.done() {
            return Ok(());
        }
        let edge = edge_op(&mut c)?;
        c.ws();
        let right = node_group(&mut c, g)?;
        for a in &left {
            for b in &right {
                g.edges.push(Edge {
                    from: a.clone(),
                    to: b.clone(),
                    label: edge.0.clone(),
                    head: edge.1,
                    dashed: edge.2,
                });
            }
        }
        left = right;
    }
}

/// `A`, `A[Label]`, or `A & B[Label] & C`.
fn node_group(c: &mut Cursor, g: &mut Graph) -> Result<Vec<String>, String> {
    let mut keys = vec![node_ref(c, g)?];
    loop {
        let save = c.i;
        c.ws();
        if c.eat("&") {
            c.ws();
            keys.push(node_ref(c, g)?);
        } else {
            c.i = save;
            return Ok(keys);
        }
    }
}

fn node_ref(c: &mut Cursor, g: &mut Graph) -> Result<String, String> {
    c.ws();
    let start = c.i;
    while let Some(ch) = c.rest().chars().next() {
        // A hyphen joins words in a key (`log-in`) but starts an arrow (`-->`, `->`).
        let joins = ch == '-' && c.rest()[1..].chars().next().is_some_and(|n| n.is_alphanumeric() || n == '_');
        if ch.is_alphanumeric() || ch == '_' || joins {
            c.i += ch.len_utf8();
        } else {
            break;
        }
    }
    if c.i == start {
        return Err(format!("expected a node at {:?}", c.rest().chars().take(12).collect::<String>()));
    }
    let key = c.s[start..c.i].to_owned();
    let shape = node_shape(c)?;
    g.node(&key, shape);
    Ok(key)
}

/// Bracket forms, longest opener first so `((` is not read as `(`.
const SHAPES: [(&str, &str, &str, Round); 11] = [
    ("([", "])", "roundedRect", Round::Pill),
    ("[(", ")]", "cylinder", Round::None),
    ("((", "))", "ellipse", Round::None),
    ("{{", "}}", "hexagon", Round::None),
    ("[/", "/]", "parallelogram", Round::None),
    ("[\\", "\\]", "parallelogram", Round::None),
    ("[/", "\\]", "parallelogram", Round::None),
    (">", "]", "rect", Round::None),
    ("[", "]", "rect", Round::Some),
    ("(", ")", "roundedRect", Round::Some),
    ("{", "}", "rhombus", Round::None),
];

fn node_shape(c: &mut Cursor) -> Result<Option<(String, &'static str, Round)>, String> {
    for (open, close, kind, round) in SHAPES {
        if !c.rest().starts_with(open) {
            continue;
        }
        let body = &c.rest()[open.len()..];
        // Not closed this way — another form with the same opener may fit.
        let Some(end) = body.find(close) else { continue };
        let label = body[..end].trim().trim_matches('"').to_owned();
        c.i += open.len() + end + close.len();
        return Ok(Some((label, kind, round)));
    }
    match c.rest().chars().next() {
        Some(ch @ ('[' | '(' | '{')) => Err(format!("{ch} is never closed")),
        _ => Ok(None),
    }
}

/// `-->`, `---`, `-.->`, `==>`, with an optional `|label|` or `-- label -->`.
/// Returns (label, has arrowhead, dashed).
fn edge_op(c: &mut Cursor) -> Result<(Option<String>, bool, bool), String> {
    let r = c.rest();
    // `-- label -->` / `-. label .->` / `== label ==>`: text inside the arrow.
    for (open, closes, dashed) in [("--", ["-->", "---"], false), ("-.", [".->", ".-"], true), ("==", ["==>", "==="], false)] {
        if r.starts_with(open) && !r.starts_with("-->") && !r.starts_with("---") && !r.starts_with("-.-") && !r.starts_with("==>") && !r.starts_with("===") {
            let body = &r[open.len()..];
            for close in closes {
                if let Some(end) = body.find(close) {
                    let label = body[..end].trim();
                    if !label.is_empty() {
                        c.i += open.len() + end + close.len();
                        return Ok((Some(label.to_owned()), close.ends_with('>'), dashed));
                    }
                }
            }
        }
    }
    let ops: [(&str, bool, bool); 10] = [
        ("-.->", true, true),
        ("-.-", false, true),
        ("==>", true, false),
        ("===", false, false),
        ("-->", true, false),
        ("--x", true, false),
        ("--o", true, false),
        ("---", false, false),
        ("->", true, false),
        ("--", false, false),
    ];
    for (op, head, dashed) in ops {
        if c.eat(op) {
            // Swallow extra dashes: `---->` is a longer arrow, same meaning.
            while c.eat("-") || c.eat("=") || c.eat(".") {}
            c.eat(">");
            c.ws();
            let mut label = None;
            if c.eat("|") {
                let body = c.rest();
                let Some(end) = body.find('|') else { return Err("edge label `|…` is never closed".into()) };
                label = Some(body[..end].trim().trim_matches('"').to_owned()).filter(|s| !s.is_empty());
                c.i += end + 1;
            }
            return Ok((label, head, dashed));
        }
    }
    Err(format!("expected an arrow like --> at {:?}", r.chars().take(12).collect::<String>()))
}

// ── layout ──────────────────────────────────────────────────────────

pub struct Style {
    pub stroke: String,
    pub text: String,
    pub fill: String,
    pub font: i64,
}

impl Default for Style {
    fn default() -> Self {
        // The canvas's own defaults (useSandboxCanvas.js DEFAULT_SHAPE_STYLE), so a
        // drawn-from-text diagram looks like one made by hand.
        Style { stroke: "#e2ddf5".into(), text: "#e2ddf5".into(), fill: "transparent".into(), font: 16 }
    }
}

pub struct Placed {
    /// Sandbox items: every shape, then every connector.
    pub items: Vec<Value>,
    /// node key → item id, so a caller can refer back to what it asked for.
    pub ids: HashMap<String, String>,
    pub width: i64,
    pub height: i64,
}

/// Break a label into lines no wider than `max` characters, at spaces.
pub fn wrap(label: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in label.split("\\n").flat_map(|p| p.split("<br>")).flat_map(|p| p.split('\n')) {
        let mut cur = String::new();
        for word in para.split_whitespace() {
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > max {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        lines.push(cur);
    }
    lines
}

/// Box size for a label in a given shape: wide enough for the text plus the
/// room the shape's own geometry eats (a diamond's text sits in its middle half).
pub fn node_size(label: &str, kind: &str, font: i64) -> (i64, i64, String) {
    let lines = wrap(label, 22);
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as i64;
    let text_w = longest * font * 11 / 20;
    let text_h = lines.len() as i64 * font * 13 / 10;
    let (w, h) = match kind {
        "rhombus" => (text_w * 3 / 2 + 48, text_h * 2 + 32),
        "ellipse" => (text_w * 5 / 4 + 40, text_h + 40),
        "hexagon" => (text_w + 72, text_h + 36),
        "parallelogram" => (text_w + 80, text_h + 32),
        "cylinder" => (text_w + 48, text_h + 56),
        _ => (text_w + 48, text_h + 28),
    };
    (w.max(120), h.max(56), lines.join("\n"))
}

const GAP_LAYER: i64 = 72;
const GAP_SIBLING: i64 = 48;

/// Lay a graph out, top-left at `origin`. `uuid` makes item ids — passed in so
/// tests can make them deterministic.
pub fn layout(g: &Graph, dir: Dir, origin: (i64, i64), style: &Style, now: &str, mut uuid: impl FnMut() -> String) -> Placed {
    let n = g.nodes.len();
    let index: HashMap<&str, usize> = g.nodes.iter().enumerate().map(|(i, nd)| (nd.key.as_str(), i)).collect();
    let edges: Vec<(usize, usize)> = g.edges.iter().filter_map(|e| Some((*index.get(e.from.as_str())?, *index.get(e.to.as_str())?))).collect();

    // 1. Cycles: an edge back to a node still on the DFS stack is a loop ("retry");
    //    it is drawn, but ignored for layering or the layers would never settle.
    let mut back = vec![false; edges.len()];
    {
        let mut state = vec![0u8; n]; // 0 new, 1 on stack, 2 done
        fn dfs(v: usize, edges: &[(usize, usize)], state: &mut [u8], back: &mut [bool]) {
            state[v] = 1;
            for (ei, &(a, b)) in edges.iter().enumerate() {
                if a != v {
                    continue;
                }
                match state[b] {
                    0 => dfs(b, edges, state, back),
                    1 => back[ei] = true,
                    _ => {}
                }
            }
            state[v] = 2;
        }
        for v in 0..n {
            if state[v] == 0 {
                dfs(v, &edges, &mut state, &mut back);
            }
        }
    }
    let fwd: Vec<(usize, usize)> = edges.iter().zip(&back).filter(|(e, b)| !**b && e.0 != e.1).map(|(e, _)| *e).collect();

    // 2. Layers: longest path from a source, so every arrow points down a layer or more.
    let mut layer = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for &(a, b) in &fwd {
            if layer[b] < layer[a] + 1 {
                layer[b] = layer[a] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for v in 0..n {
        rows[layer[v]].push(v); // first-appearance order to start
    }

    // 3. Order within a layer: barycenter of neighbours, a few sweeps each way.
    let pos_of = |rows: &Vec<Vec<usize>>| {
        let mut p = vec![0f64; n];
        for r in rows {
            for (i, &v) in r.iter().enumerate() {
                p[v] = i as f64;
            }
        }
        p
    };
    for sweep in 0..6 {
        let down = sweep % 2 == 0;
        let range: Vec<usize> = if down { (1..depth).collect() } else { (0..depth.saturating_sub(1)).rev().collect() };
        for l in range {
            let p = pos_of(&rows);
            let mut keyed: Vec<(f64, usize)> = rows[l]
                .iter()
                .map(|&v| {
                    let nb: Vec<f64> = fwd
                        .iter()
                        .filter_map(|&(a, b)| if down && b == v { Some(p[a]) } else if !down && a == v { Some(p[b]) } else { None })
                        .collect();
                    (if nb.is_empty() { p[v] } else { nb.iter().sum::<f64>() / nb.len() as f64 }, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            rows[l] = keyed.into_iter().map(|(_, v)| v).collect();
        }
    }

    // 4. Sizes, then coordinates. "Along" is the flow axis (down or right),
    //    "across" is the sibling axis.
    let sizes: Vec<(i64, i64, String)> = g.nodes.iter().map(|nd| node_size(&nd.label, nd.kind, style.font)).collect();
    let dims = |v: usize| -> (i64, i64) {
        let (w, h, _) = &sizes[v];
        if dir == Dir::Down { (*h, *w) } else { (*w, *h) }
    };
    let along = |v: usize| dims(v).0;
    let across = |v: usize| dims(v).1;
    // Across: centre each node on its parents, then push apart to keep order and gaps.
    let mut cx = vec![0f64; n];
    for (l, r) in rows.iter().enumerate() {
        let want: Vec<f64> = r
            .iter()
            .map(|&v| {
                let parents: Vec<f64> = fwd.iter().filter(|&&(_, b)| b == v).map(|&(a, _)| cx[a]).collect();
                if l == 0 || parents.is_empty() { f64::NAN } else { parents.iter().sum::<f64>() / parents.len() as f64 }
            })
            .collect();
        // Default spread for nodes with no placed parent: packed, centred on 0.
        let total: i64 = r.iter().map(|&v| across(v)).sum::<i64>() + GAP_SIBLING * (r.len() as i64 - 1).max(0);
        let mut x = -(total as f64) / 2.0;
        let mut placed = Vec::with_capacity(r.len());
        for (i, &v) in r.iter().enumerate() {
            let half = across(v) as f64 / 2.0;
            let natural = x + half;
            let target = if want[i].is_nan() { natural } else { want[i] };
            let min = placed.last().map(|&(pc, ph): &(f64, f64)| pc + ph + GAP_SIBLING as f64 + half).unwrap_or(f64::MIN);
            let c = target.max(min);
            placed.push((c, half));
            x = c + half + GAP_SIBLING as f64;
        }
        // Pushing right only would drift the layer; recentre it on what it wanted.
        let shift: f64 = {
            let diffs: Vec<f64> = placed.iter().zip(&want).filter(|(_, w)| !w.is_nan()).map(|(p, w)| w - p.0).collect();
            if diffs.is_empty() { 0.0 } else { diffs.iter().sum::<f64>() / diffs.len() as f64 }
        };
        for (i, &v) in r.iter().enumerate() {
            cx[v] = placed[i].0 + shift;
        }
    }
    // Along: each layer as thick as its thickest node.
    let mut layer_at = vec![0i64; depth];
    let mut acc = 0i64;
    for (l, r) in rows.iter().enumerate() {
        layer_at[l] = acc;
        acc += r.iter().map(|&v| along(v)).max().unwrap_or(0) + GAP_LAYER;
    }

    // Rect for each node, then normalise so the whole thing starts at `origin`.
    let mut rects: Vec<(i64, i64, i64, i64)> = (0..n)
        .map(|v| {
            let (w, h, _) = &sizes[v];
            let thick = rows[layer[v]].iter().map(|&u| along(u)).max().unwrap_or(0);
            let a = layer_at[layer[v]] + (thick - along(v)) / 2; // centred in its layer
            let c = cx[v].round() as i64 - across(v) / 2;
            if dir == Dir::Down { (c, a, *w, *h) } else { (a, c, *w, *h) }
        })
        .collect();
    let minx = rects.iter().map(|r| r.0).min().unwrap_or(0);
    let miny = rects.iter().map(|r| r.1).min().unwrap_or(0);
    for r in &mut rects {
        r.0 += origin.0 - minx;
        r.1 += origin.1 - miny;
    }
    let width = rects.iter().map(|r| r.0 + r.2).max().unwrap_or(0) - origin.0;
    let height = rects.iter().map(|r| r.1 + r.3).max().unwrap_or(0) - origin.1;

    // 5. Items, in the key order the desktop's store writes them.
    let mut items = Vec::new();
    let mut ids = HashMap::new();
    for (v, nd) in g.nodes.iter().enumerate() {
        let id = uuid();
        let (x, y, w, h) = rects[v];
        let radius = match nd.round {
            Round::None => 0,
            Round::Some => 10,
            Round::Pill => h / 2,
        };
        items.push(json!({
            "rotation": 0,
            "z_index": items.len() as i64 + 1,
            "payload": {
                "kind": nd.kind,
                "fill": style.fill,
                "stroke": style.stroke,
                "strokeWidth": 2,
                "radius": radius,
                "text": sizes[v].2,
                "textAlign": "center",
                "textColor": style.text,
                "fontSize": style.font,
            },
            "id": id,
            "type": "shape",
            "x": x, "y": y, "w": w, "h": h,
            "created_at": now, "updated_at": now,
        }));
        ids.insert(nd.key.clone(), id);
    }
    for (ei, e) in g.edges.iter().enumerate() {
        let (Some(&a), Some(&b)) = (index.get(e.from.as_str()), index.get(e.to.as_str())) else { continue };
        let (fs, ts) = sides(dir, layer[a], layer[b], cx[a], cx[b], back.get(ei).copied().unwrap_or(false));
        let mut payload = json!({
            "from": { "itemId": ids[&e.from], "side": fs },
            "to": { "itemId": ids[&e.to], "side": ts },
            "routing": "elbow",
            "head": if e.head { "arrow" } else { "none" },
            "stroke": style.stroke,
            "strokeWidth": 2,
        });
        if let Some(l) = &e.label {
            payload["label"] = json!(l);
        }
        if e.dashed {
            payload["dash"] = json!(true);
        }
        items.push(json!({
            "rotation": 0,
            "z_index": items.len() as i64 + 1,
            "payload": payload,
            "id": uuid(),
            "type": "connector",
            "x": 0, "y": 0, "w": 0, "h": 0,
            "created_at": now, "updated_at": now,
        }));
    }
    Placed { items, ids, width, height }
}

/// Which sides an arrow leaves and enters by. Forward edges run with the flow;
/// a loop back up (a "retry") leaves and re-enters on the outside edge so it
/// does not cut back through the boxes between.
fn sides(dir: Dir, la: usize, lb: usize, ca: f64, cb: f64, back: bool) -> (&'static str, &'static str) {
    let (fwd_out, fwd_in, out_side, cross_lo, cross_hi) = match dir {
        Dir::Down => ("s", "n", "e", "w", "e"),
        Dir::Right => ("e", "w", "s", "n", "s"),
    };
    if back || lb < la {
        return (out_side, out_side);
    }
    if lb == la {
        return if cb >= ca { (cross_hi, cross_lo) } else { (cross_lo, cross_hi) };
    }
    (fwd_out, fwd_in)
}

/// Read a diagram back as flowchart text — how an AI "sees" a board. Shapes
/// get short keys (n1, n2…) in reading order; the map gives each key's real id.
pub fn describe(items: &[Value]) -> (String, Vec<(String, String)>) {
    let mut shapes: Vec<&Value> = items.iter().filter(|i| i["type"] == "shape").collect();
    shapes.sort_by_key(|i| (i["y"].as_f64().unwrap_or(0.0) as i64 / 40, i["x"].as_f64().unwrap_or(0.0) as i64));
    let mut key_of: HashMap<String, String> = HashMap::new();
    let mut keys = Vec::new();
    let mut out = String::from("flowchart TD\n");
    for (i, s) in shapes.iter().enumerate() {
        let id = s["id"].as_str().map(str::to_owned).unwrap_or_else(|| s["id"].to_string());
        let key = format!("n{}", i + 1);
        let text = s["payload"]["text"].as_str().unwrap_or("").replace('\n', " ");
        let (o, c) = match s["payload"]["kind"].as_str().unwrap_or("rect") {
            "rhombus" => ("{", "}"),
            "ellipse" => ("((", "))"),
            "roundedRect" => ("(", ")"),
            "hexagon" => ("{{", "}}"),
            "cylinder" => ("[(", ")]"),
            "parallelogram" => ("[/", "/]"),
            _ => ("[", "]"),
        };
        out.push_str(&format!("  {key}{o}{}{c}\n", if text.is_empty() { "…" } else { &text }));
        key_of.insert(id.clone(), key.clone());
        keys.push((key, id));
    }
    for cn in items.iter().filter(|i| i["type"] == "connector") {
        let end = |e: &Value| e["itemId"].as_str().and_then(|id| key_of.get(id)).cloned();
        let (Some(a), Some(b)) = (end(&cn["payload"]["from"]), end(&cn["payload"]["to"])) else { continue };
        let arrow = if cn["payload"]["head"] == "none" { "---" } else { "-->" };
        match cn["payload"]["label"].as_str() {
            Some(l) if !l.is_empty() => out.push_str(&format!("  {a} {arrow}|{l}| {b}\n")),
            _ => out.push_str(&format!("  {a} {arrow} {b}\n")),
        }
    }
    (out, keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> impl FnMut() -> String {
        let mut i = 0;
        move || {
            i += 1;
            format!("id{i}")
        }
    }

    #[test]
    fn parses_shapes_labels_and_chains() {
        let (g, problems) = parse("flowchart LR\nA[Start] --> B{OK?} -->|yes| C((Done))\nB -- no --> A").unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(g.dir, Some(Dir::Right));
        assert_eq!(g.nodes.iter().map(|n| (n.key.as_str(), n.kind, n.label.as_str())).collect::<Vec<_>>(), vec![
            ("A", "rect", "Start"),
            ("B", "rhombus", "OK?"),
            ("C", "ellipse", "Done"),
        ]);
        assert_eq!(g.edges.len(), 3);
        assert_eq!(g.edges[1].label.as_deref(), Some("yes"));
        assert_eq!(g.edges[2].label.as_deref(), Some("no"));
        assert_eq!((g.edges[2].from.as_str(), g.edges[2].to.as_str()), ("B", "A"));
    }

    #[test]
    fn parses_fan_out_and_the_less_common_shapes() {
        let (g, _) = parse("A([Begin]) --> B[(DB)] & C[/In/] & D{{Prep}}\nE --- F").unwrap();
        assert_eq!(g.edges.len(), 4);
        let kinds: Vec<_> = g.nodes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, vec!["roundedRect", "cylinder", "parallelogram", "hexagon", "rect", "rect"]);
        assert!(!g.edges[3].head);
    }

    #[test]
    fn a_bad_line_is_reported_not_fatal() {
        let (g, problems) = parse("A --> B\nthis is ??? not a diagram -->\nB --> C").unwrap();
        assert_eq!(g.nodes.len(), 3);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn layout_puts_each_step_below_the_last_and_attaches_connectors() {
        let (g, _) = parse("A[One] --> B[Two] --> C[Three]").unwrap();
        let p = layout(&g, Dir::Down, (100, 200), &Style::default(), "t", ids());
        let shapes: Vec<&Value> = p.items.iter().filter(|i| i["type"] == "shape").collect();
        let y = |i: usize| shapes[i]["y"].as_i64().unwrap();
        assert!(y(0) < y(1) && y(1) < y(2));
        assert_eq!(shapes[0]["x"].as_i64().unwrap().min(shapes[1]["x"].as_i64().unwrap()), 100);
        assert_eq!(y(0), 200);
        let conn = p.items.iter().find(|i| i["type"] == "connector").unwrap();
        assert_eq!(conn["payload"]["from"]["side"], "s");
        assert_eq!(conn["payload"]["to"]["side"], "n");
        assert_eq!(conn["payload"]["from"]["itemId"], p.ids["A"].as_str());
    }

    #[test]
    fn branches_sit_side_by_side_without_overlapping() {
        let (g, _) = parse("A{Choice} -->|left| B[Left path]\nA -->|right| C[Right path]").unwrap();
        let p = layout(&g, Dir::Down, (0, 0), &Style::default(), "t", ids());
        let b = &p.items[1];
        let c = &p.items[2];
        assert_eq!(b["y"], c["y"]);
        let (bx, bw, cx) = (b["x"].as_i64().unwrap(), b["w"].as_i64().unwrap(), c["x"].as_i64().unwrap());
        assert!(bx + bw <= cx, "siblings overlap");
    }

    #[test]
    fn a_loop_back_still_lays_out() {
        let (g, _) = parse("A --> B --> C --> A").unwrap();
        let p = layout(&g, Dir::Down, (0, 0), &Style::default(), "t", ids());
        assert_eq!(p.items.iter().filter(|i| i["type"] == "connector").count(), 3);
    }

    #[test]
    fn coordinates_are_whole_numbers() {
        let (g, _) = parse("A[A label of some length] --> B{Is it long enough to wrap onto two lines?}").unwrap();
        let p = layout(&g, Dir::Down, (0, 0), &Style::default(), "t", ids());
        for it in &p.items {
            for k in ["x", "y", "w", "h"] {
                assert!(it[k].is_i64(), "{k} = {}", it[k]);
            }
        }
    }

    #[test]
    fn describe_round_trips_structure() {
        let (g, _) = parse("A[Start] --> B{OK?}\nB -->|yes| C[End]").unwrap();
        let p = layout(&g, Dir::Down, (0, 0), &Style::default(), "t", ids());
        let (text, keys) = describe(&p.items);
        let (g2, problems) = parse(&text).unwrap();
        assert!(problems.is_empty(), "{text}");
        assert_eq!(g2.nodes.len(), 3);
        assert_eq!(g2.edges.len(), 2);
        assert_eq!(keys.len(), 3);
        assert!(text.contains("|yes|"));
    }
}

#[cfg(test)]
mod parity {
    // Siddran-Desktop's flowText.test.js pins these same numbers ("parity with
    // Siddran-CLI"); the two layouts must size boxes identically.
    #[test]
    fn node_sizes_match_the_desktop() {
        let p = |l: &str, k: &str| {
            let (w, h, _) = super::node_size(l, k, 16);
            (w, h)
        };
        assert_eq!(p("Start", "rect"), (120, 56));
        assert_eq!(p("Is it long enough to wrap onto two lines?", "rhombus"), (312, 114));
    }
}
