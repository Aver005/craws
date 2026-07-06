//! End-to-end MCP protocol test: spawn the real `craws-mcp` binary and drive it
//! over stdio exactly as pooprusteek's client does — newline-delimited JSON-RPC
//! 2.0, protocol `2024-11-05`, integer ids echoed back. This both verifies the
//! wire contract and doubles as the batch-processing demo transcript.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Client {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_craws-mcp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) // server logs to stderr; we don't need it
            .spawn()
            .expect("spawn craws-mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Client { child, stdin, stdout, next_id: 1 }
    }

    /// Send a request, return the matching JSON-RPC response (skips any
    /// notification/other-id line, like the real client tolerates).
    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.stdin, "{req}").unwrap();
        self.stdin.flush().unwrap();

        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("read response");
            assert!(n > 0, "server closed stdout while awaiting id {id} ({method})");
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                return v;
            }
        }
    }

    fn notify(&mut self, method: &str) {
        let n = json!({ "jsonrpc": "2.0", "method": method });
        writeln!(self.stdin, "{n}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Call a tool, assert it did not error, and return the flattened text (the
    /// client concatenates `content[].text`). Our tools answer with a JSON line.
    fn call_ok(&mut self, tool: &str, args: Value) -> Value {
        let resp = self.request("tools/call", json!({ "name": tool, "arguments": args }));
        let result = resp.get("result").unwrap_or_else(|| panic!("{tool} errored: {resp}"));
        assert_ne!(
            result.get("isError").and_then(Value::as_bool),
            Some(true),
            "{tool} returned isError: {result}"
        );
        let text = result["content"][0]["text"].as_str().expect("text content part");
        serde_json::from_str(text).unwrap_or_else(|_| json!({ "text": text }))
    }

    fn call_expect_error(&mut self, tool: &str, args: Value) -> String {
        let resp = self.request("tools/call", json!({ "name": tool, "arguments": args }));
        // an invalid call surfaces either as a JSON-RPC error or isError:true
        if let Some(err) = resp.get("error") {
            return err["message"].as_str().unwrap_or("").to_string();
        }
        let result = &resp["result"];
        assert_eq!(result.get("isError").and_then(Value::as_bool), Some(true), "expected error: {resp}");
        result["content"][0]["text"].as_str().unwrap_or("").to_string()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn sample_png(path: &std::path::Path, w: u32, h: u32) {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 100, 255]);
        }
    }
    let bytes = craws_codecs::encode(w, h, &px, craws_codecs::ImageFormat::Png, None).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn handshake_lists_tools_and_runs_a_batch() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::spawn();

    // ── handshake ──
    let init = c.request(
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "craws-test", "version": "0" }
        }),
    );
    let result = &init["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05", "server echoes the negotiated version");
    assert!(result["capabilities"]["tools"].is_object(), "tools capability advertised: {result}");
    c.notify("notifications/initialized");

    // ── discovery ──
    let tools = c.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "open_image", "resize", "crop", "exposure", "grayscale", "image_info", "export",
        "draw_rect", "draw_ellipse", "draw_line", "draw_arrow", "overlay", "collage",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}; got {names:?}");
    }
    // schema sanity: resize declares image_id
    let resize = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "resize").unwrap();
    assert!(
        resize["inputSchema"]["properties"]["image_id"].is_object(),
        "resize schema must expose image_id: {}",
        resize["inputSchema"]
    );

    // ── batch: open → resize → exposure → grayscale → export (the demo) ──
    let src = dir.path().join("in.png");
    sample_png(&src, 800, 500);

    let opened = c.call_ok("open_image", json!({ "path": src.to_str().unwrap() }));
    assert_eq!(opened["width"], 800);
    let id0 = opened["image_id"].as_str().unwrap().to_string();

    let resized = c.call_ok("resize", json!({ "image_id": id0, "width": 320 }));
    assert_eq!((resized["width"].as_u64(), resized["height"].as_u64()), (Some(320), Some(200)));
    let id1 = resized["image_id"].as_str().unwrap().to_string();
    assert_ne!(id1, id0, "each op yields a new handle");

    let darker = c.call_ok("exposure", json!({ "image_id": id1, "stops": -0.5 }));
    let id2 = darker["image_id"].as_str().unwrap().to_string();

    let gray = c.call_ok("grayscale", json!({ "image_id": id2 }));
    let id3 = gray["image_id"].as_str().unwrap().to_string();

    // original handle still intact (immutability across the whole session)
    let info0 = c.call_ok("image_info", json!({ "image_id": id0 }));
    assert_eq!(info0["width"], 800);

    let out = dir.path().join("out.webp");
    let exported = c.call_ok("export", json!({ "image_id": id3, "path": out.to_str().unwrap() }));
    assert!(exported["bytes"].as_u64().unwrap() > 0);

    // the file exists and is a valid 320×200 grayscale image
    let decoded = craws_codecs::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((decoded.width, decoded.height), (320, 200));
    let p = &decoded.rgba8[..4];
    assert!(p[0] == p[1] && p[1] == p[2], "exported image should be gray: {p:?}");
}

#[test]
fn annotation_and_collage_over_the_wire() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::spawn();
    c.request("initialize", json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }));
    c.notify("notifications/initialized");

    // annotate a screenshot: arrow + circle (ellipse) + rounded rect, hex + named colors
    let src = dir.path().join("shot.png");
    sample_png(&src, 600, 400);
    let id = c.call_ok("open_image", json!({ "path": src.to_str().unwrap() }))["image_id"].as_str().unwrap().to_string();
    let id = c.call_ok("draw_rect", json!({ "image_id": id, "x": 40, "y": 40, "width": 200, "height": 120, "corner_radius": 12, "stroke": "#ff3030", "stroke_width": 5 }))["image_id"].as_str().unwrap().to_string();
    let id = c.call_ok("draw_ellipse", json!({ "image_id": id, "x": 300, "y": 200, "width": 140, "height": 140, "stroke": "yellow", "stroke_width": 6 }))["image_id"].as_str().unwrap().to_string();
    let id = c.call_ok("draw_arrow", json!({ "image_id": id, "x1": 120, "y1": 300, "x2": 300, "y2": 240, "color": "#00a0ff", "thickness": 6 }))["image_id"].as_str().unwrap().to_string();
    let out = dir.path().join("annotated.png");
    c.call_ok("export", json!({ "image_id": id, "path": out.to_str().unwrap() }));
    let d = craws_codecs::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((d.width, d.height), (600, 400), "annotations keep image size");

    // collage two shots
    let a = c.call_ok("open_image", json!({ "path": src.to_str().unwrap() }))["image_id"].as_str().unwrap().to_string();
    let b = c.call_ok("open_image", json!({ "path": src.to_str().unwrap() }))["image_id"].as_str().unwrap().to_string();
    let coll = c.call_ok("collage", json!({ "image_ids": [a, b], "target_width": 900, "row_height": 220, "background": "#101010" }));
    assert_eq!(coll["width"], 900, "collage fills target width");

    // a bad color is a clean tool error, not a crash
    let msg = c.call_expect_error("draw_line", json!({ "image_id": id, "x1": 0, "y1": 0, "x2": 10, "y2": 10, "color": "chartreuse" }));
    assert!(msg.to_lowercase().contains("color"), "color error: {msg}");
}

#[test]
fn invalid_calls_report_errors_without_killing_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::spawn();
    c.request("initialize", json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }));
    c.notify("notifications/initialized");

    // unknown handle
    let msg = c.call_expect_error("grayscale", json!({ "image_id": "img-nope" }));
    assert!(msg.contains("img-nope"), "error should name the bad handle: {msg}");

    // out-of-bounds crop on a real image
    let src = dir.path().join("in.png");
    sample_png(&src, 100, 100);
    let opened = c.call_ok("open_image", json!({ "path": src.to_str().unwrap() }));
    let id = opened["image_id"].as_str().unwrap().to_string();
    let msg = c.call_expect_error("crop", json!({ "image_id": id, "x": 0, "y": 0, "width": 999, "height": 10 }));
    assert!(msg.to_lowercase().contains("crop") || msg.contains("fit"), "crop error: {msg}");

    // server is still alive: a normal call still works
    let gray = c.call_ok("grayscale", json!({ "image_id": id }));
    assert!(gray["image_id"].as_str().unwrap().starts_with("img-"));
}
