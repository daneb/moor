//! `moor ui` — the console in a browser.
//!
//! The same ground `moor studio` covers, for people who would rather look
//! at a page than a terminal: every project, where each spec stands, the
//! agent, approvals, evidence and the audit trail. It binds loopback only,
//! runs for exactly as long as the command does (no daemon, no launchd
//! entry), and takes a per-launch token on every API call — carried in
//! the URL's fragment, which never reaches a server or a log, and sent
//! back as a header, so a page on another origin cannot drive it even
//! from the same machine.
//!
//! Actions are the `moor` binary run as a child (`jobs.rs`); reads go
//! through the same audited `docker exec` path everything else uses
//! (`data.rs`). The page itself is embedded in the binary.

mod api;
mod data;
mod http;
mod jobs;

use anyhow::{Context, Result};
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

pub struct State {
    pub token: String,
    pub jobs: jobs::Jobs,
}

impl State {
    #[cfg(test)]
    pub fn for_tests() -> State {
        State {
            token: "test".into(),
            jobs: jobs::Jobs::default(),
        }
    }
}

/// 192 bits from the kernel, hex. A console token is a capability, not
/// a password: it lives for one run of `moor ui` and is never written.
fn token() -> Result<String> {
    let mut bytes = [0u8; 24];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .context("reading /dev/urandom for the console token")?;
    Ok(hex::encode(bytes))
}

pub fn run(port: u16, open: bool) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("binding 127.0.0.1:{port} — is another moor ui running?"))?;
    let port = listener.local_addr()?.port();
    let state = Arc::new(State {
        token: token()?,
        jobs: jobs::Jobs::default(),
    });
    let url = format!("http://127.0.0.1:{port}/#{}", state.token);
    println!("moor ui is at {url}");
    println!("Loopback only; the token in the URL is this run's key. Ctrl-C stops it.");
    if open {
        let _ = std::process::Command::new("open").arg(&url).status();
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(stream, &state) {
                // A browser closing a tab mid-stream is routine, not news.
                if std::env::var("MOOR_UI_DEBUG").is_ok() {
                    eprintln!("moor ui: {e:#}");
                }
            }
        });
    }
    Ok(())
}

/// Only a loopback `Host` is served: a page elsewhere that resolves a
/// name to 127.0.0.1 (DNS rebinding) still sends its own host name.
fn host_ok(req: &http::Request) -> bool {
    match req.header("host") {
        Some(h) => {
            let h = h.rsplit_once(':').map(|(name, _)| name).unwrap_or(h);
            matches!(h, "127.0.0.1" | "localhost" | "[::1]")
        }
        None => false,
    }
}

fn handle(mut stream: TcpStream, state: &State) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let req = http::read_request(&mut stream)?;
    if !host_ok(&req) {
        return http::write_response(&mut stream, &http::Response::error(403, "wrong host"));
    }
    // The one streaming route: a job's output as server-sent events.
    let segs: Vec<&str> = req.path.trim_matches('/').split('/').collect();
    if let ("GET", ["api", "jobs", id, "stream"]) = (req.method.as_str(), segs.as_slice()) {
        if req.header("x-moor-token") != Some(state.token.as_str())
            && req.query_str("token") != Some(state.token.as_str())
        {
            return http::write_response(&mut stream, &http::Response::error(401, "missing token"));
        }
        let Some(job) = id.parse().ok().and_then(|id| state.jobs.get(id)) else {
            return http::write_response(&mut stream, &http::Response::error(404, "no such job"));
        };
        stream.set_read_timeout(None)?;
        let mut sse = http::Sse::start(&mut stream)?;
        let mut after = req.query_usize("after").unwrap_or(0);
        loop {
            let (lines, next, done) = job.wait(after, Duration::from_secs(15));
            if !lines.is_empty() {
                sse.send(
                    "lines",
                    &serde_json::json!({ "lines": lines, "next": next }),
                )?;
            } else if !done {
                sse.send("ping", &serde_json::json!({}))?;
            }
            after = next;
            if done {
                sse.send("done", &job.snapshot(after))?;
                return Ok(());
            }
        }
    }
    let resp = api::route(state, &req);
    http::write_response(&mut stream, &resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn req(host: Option<&str>) -> http::Request {
        let mut headers = HashMap::new();
        if let Some(h) = host {
            headers.insert("host".to_string(), h.to_string());
        }
        http::Request {
            method: "GET".into(),
            path: "/".into(),
            query: HashMap::new(),
            headers,
            body: vec![],
        }
    }

    #[test]
    fn only_loopback_hosts_are_served() {
        assert!(host_ok(&req(Some("127.0.0.1:7474"))));
        assert!(host_ok(&req(Some("localhost:7474"))));
        assert!(!host_ok(&req(Some("evil.example:7474"))));
        assert!(!host_ok(&req(None)));
    }

    #[test]
    fn api_calls_need_the_token() {
        let state = State::for_tests();
        let mut r = req(Some("127.0.0.1"));
        r.path = "/api/overview".into();
        let resp = api::route(&state, &r);
        assert_eq!(resp.status, 401);
        r.headers.insert("x-moor-token".into(), "test".into());
        let resp = api::route(&state, &r);
        assert_ne!(resp.status, 401);
    }

    #[test]
    fn the_page_is_served_without_a_token() {
        let state = State::for_tests();
        let resp = api::route(&state, &req(Some("localhost")));
        assert_eq!(resp.status, 200);
        assert!(String::from_utf8_lossy(&resp.body).contains("<html"));
    }
}
