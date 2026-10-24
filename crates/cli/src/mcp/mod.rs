//! `siddran mcp` — the Bag as tools an AI can call, over the Model Context
//! Protocol (JSON-RPC 2.0, one message per line on stdin/stdout).
//!
//! Same stance as the rest of this crate: a **peer** of the desktop app, not a
//! client of it. It reads and writes the Bag folder directly, so it works with
//! the desktop closed, and with it open the desktop's watcher shows each change
//! as it lands — a note the AI edits updates in the open editor, merged with
//! whatever you are typing.
//!
//! Every tool call is one short write session: take the Bag lock (waiting out
//! anyone mid-write), read the Bag fresh, apply the change, write atomically,
//! let go. Nothing is cached between calls, so the AI never acts on a picture
//! the desktop or the TUI has since changed.
//!
//! Hand-rolled rather than an SDK: the protocol surface used here is four
//! methods, and every dependency is compile time and binary size this project
//! counts.

mod sandbox;
mod tools;

/// The same tools without MCP: `siddran tools` lists them, `siddran call <tool>
/// '<json>'` runs one. For any AI that has a shell — no setup, nothing running,
/// no tool descriptions loaded until it actually asks.
pub fn list_tools(json_out: bool) {
    // writeln!, not println!: `siddran tools | head` closes the pipe early, and
    // println! panics on that. A reader that has seen enough is not an error.
    let mut out = std::io::stdout().lock();
    let defs = tools::definitions();
    if json_out {
        let _ = writeln!(out, "{}", serde_json::to_string_pretty(&defs).unwrap_or_default());
        return;
    }
    let _ = writeln!(out, "{INSTRUCTIONS}\n");
    let _ = writeln!(out, "Run one with: siddran call <tool> '<json arguments>'   (or pipe the JSON on stdin)");
    let _ = writeln!(out, "Full argument schemas: siddran tools --json\n");
    for t in defs.as_array().into_iter().flatten() {
        let props = t["inputSchema"]["properties"].as_object();
        let req: Vec<&str> = t["inputSchema"]["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        let args: Vec<String> = props
            .map(|p| p.keys().map(|k| if req.contains(&k.as_str()) { k.clone() } else { format!("{k}?") }).collect())
            .unwrap_or_default();
        if writeln!(out, "{}  {{{}}}\n    {}\n", t["name"].as_str().unwrap_or(""), args.join(", "), t["description"].as_str().unwrap_or("")).is_err() {
            return;
        }
    }
}

/// Run one tool. Ok(json) on stdout; Err(message) for the caller to print.
pub fn call_tool(root: &std::path::Path, name: &str, args: &str) -> Result<String, String> {
    let args: Value = if args.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args).map_err(|e| format!("arguments are not valid JSON: {e}"))?
    };
    tools::call(root, name, &args).map(|v| render(&v))
}

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "\
Siddran is the user's local notes / tasks / calendar / whiteboard app. These tools read and \
write their Bag (a folder of markdown notes and JSON files) directly; if the desktop app is \
open, it shows your changes live.

Start with `siddran_overview` to see what is there and what the user has open right now\n(including text they have selected in a note, if the desktop app is open).
Refer to things by id where you have one; titles also work (exact, then unique prefix), and an \
ambiguous title is an error that lists the candidates.
Edit notes with `note_edit` (exact find/replace, append, prepend) rather than rewriting the \
whole body — the user may be typing in the same note, and a targeted edit merges cleanly with \
theirs.
Dates: `today`, `tomorrow`, `YYYY-MM-DD` (all-day), or `YYYY-MM-DDTHH:MM` (local time).
Diagrams: `sandbox_diagram` takes a small flowchart text (`A[Start] --> B{OK?}`, \
`B -->|yes| C`) and lays it out on a whiteboard for you — prefer it to placing shapes by hand.";

pub fn run(root: PathBuf) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(&root, &msg),
            Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(r) = reply {
            let s = serde_json::to_string(&r).map_err(|e| e.to_string())?;
            writeln!(out, "{s}").map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn handle(root: &std::path::Path, msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    // A message with no id is a notification: never answered.
    let id = msg.get("id").cloned()?;
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = if PROTOCOLS.contains(&asked) { asked } else { PROTOCOLS[0] };
            ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "siddran", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }),
            )
        }
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools::definitions() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // A tool that fails is a *result* the model should read and react to,
            // not a protocol error — so it comes back as isError, not as -32xxx.
            match tools::call(root, name, &args) {
                Ok(v) => ok(id, json!({ "content": [{ "type": "text", "text": render(&v) }], "isError": false })),
                Err(e) => ok(id, json!({ "content": [{ "type": "text", "text": e }], "isError": true })),
            }
        }
        "resources/list" => ok(id, json!({ "resources": [] })),
        "prompts/list" => ok(id, json!({ "prompts": [] })),
        _ => error(id, -32601, &format!("method not found: {method}")),
    })
}

fn render(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_get_no_reply() {
        let msg = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        assert!(handle(std::path::Path::new("/nonexistent"), &msg).is_none());
    }

    #[test]
    fn initialize_echoes_a_supported_version() {
        let msg = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26" } });
        let r = handle(std::path::Path::new("/nonexistent"), &msg).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "siddran");
    }

    #[test]
    fn unknown_methods_are_protocol_errors() {
        let msg = json!({ "jsonrpc": "2.0", "id": 7, "method": "nope" });
        let r = handle(std::path::Path::new("/nonexistent"), &msg).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn every_tool_has_a_schema() {
        for t in tools::definitions().as_array().unwrap() {
            assert!(t["name"].is_string());
            assert!(t["description"].as_str().unwrap().len() > 10, "{}", t["name"]);
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
    }
}
