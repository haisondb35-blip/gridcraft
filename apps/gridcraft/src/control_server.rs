//! Localhost JSON-lines control server. Each connection may send many requests; each line
//! `{"id":…, "method":…, "params":…}` gets one reply line `{"id":…, "ok":…, …}`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use gridcraft_ui_egui::ControlRequest;
use serde_json::{Value as Json, json};

const MAX_LINE: usize = 64 * 1024 * 1024;

pub fn start(port: u16, ctx: egui::Context) -> std::io::Result<Receiver<ControlRequest>> {
    start_with(port, ctx, None).map(|(_, rx)| rx)
}

/// Listen on 127.0.0.1:`port` (0: a free port the system picks). With a `token`, a request that
/// doesn't carry it (`"token": "…"`) is refused: the Claude pane's channel answers only the
/// Claude it started (`GRIDCRAFT_CONTROL_TOKEN`). Returns the port and the requests.
pub fn start_with(port: u16, ctx: egui::Context, token: Option<String>) -> std::io::Result<(u16, Receiver<ControlRequest>)> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = channel();
    let token: Option<std::sync::Arc<str>> = token.map(Into::into);
    std::thread::Builder::new().name("control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            let ctx = ctx.clone();
            let token = token.clone();
            let _ = std::thread::Builder::new().name("control-conn".into()).spawn(move || serve(stream, tx, ctx, token.as_deref()));
        }
    })?;
    log::info!("control channel listening on 127.0.0.1:{port}");
    Ok((port, rx))
}

/// Does the request carry the channel's token (when it has one)? Compared in constant time.
fn authorised(msg: &Json, token: Option<&str>) -> bool {
    match token {
        None => true,
        Some(t) => msg
            .get("token")
            .and_then(Json::as_str)
            .is_some_and(|given| given.len() == t.len() && given.bytes().zip(t.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0),
    }
}

fn serve(stream: TcpStream, tx: Sender<ControlRequest>, ctx: egui::Context, token: Option<&str>) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    let reader = BufReader::new(read);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if line.len() > MAX_LINE {
            let _ = writeln!(out, "{}", json!({"ok": false, "error": "request too large"}));
            continue;
        }
        let msg: Json = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let _ = writeln!(out, "{}", json!({"ok": false, "error": format!("invalid JSON: {e}")}));
                continue;
            }
        };
        let id = msg.get("id").cloned().unwrap_or(Json::Null);
        if !authorised(&msg, token) {
            let _ = writeln!(out, "{}", json!({"id": id, "ok": false, "error": "this control channel needs its token"}));
            continue;
        }
        let method = msg.get("method").and_then(Json::as_str).unwrap_or("").to_string();
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let (req, rx) = ControlRequest::new(method, params);
        if tx.send(req).is_err() {
            break;
        }
        // Wake the UI thread: egui only runs frames when something happens.
        ctx.request_repaint();
        let mut reply = rx
            .recv_timeout(Duration::from_secs(60))
            .unwrap_or_else(|_| json!({"ok": false, "error": "timed out waiting for the UI (is the window minimised?)"}));
        if let Some(o) = reply.as_object_mut() {
            o.insert("id".into(), id);
        }
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tokened_channel_answers_only_requests_with_its_token() {
        let req = |t: Option<&str>| match t {
            Some(t) => json!({"id": 1, "method": "ui.inspect", "token": t}),
            None => json!({"id": 1, "method": "ui.inspect"}),
        };
        assert!(authorised(&req(None), None), "the --control channel takes no token");
        assert!(authorised(&req(Some("abc123")), Some("abc123")));
        for wrong in [None, Some(""), Some("abc12"), Some("abc124"), Some("abc1234")] {
            assert!(!authorised(&req(wrong), Some("abc123")), "{wrong:?}");
        }
    }
}
