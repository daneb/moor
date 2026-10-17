//! The smallest HTTP/1.1 that `moor ui` needs: one request per
//! connection, a `Content-Length` body, one response, close. Hand-rolled
//! over `std::net` for the same reason the MCP server is hand-rolled over
//! stdio: the console binds loopback and serves one operator, so a
//! framework would be weight without a job.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

/// Bodies past this are refused: nothing the console sends is near it,
/// and a runaway client should not be able to exhaust memory.
const MAX_BODY: usize = 8 * 1024 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    /// Header names lower-cased.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).context("the request body isn't the JSON expected")
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn query_str(&self, key: &str) -> Option<&str> {
        self.query.get(key).map(String::as_str)
    }

    pub fn query_usize(&self, key: &str) -> Option<usize> {
        self.query_str(key).and_then(|v| v.parse().ok())
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = &s[i + 1..i + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(p), String::new()),
        })
        .collect()
}

pub fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("reading the request line")?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_ascii_uppercase();
    let target = parts.next().unwrap_or("/").to_string();
    if method.is_empty() {
        anyhow::bail!("empty request");
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_query(q)),
        None => (target.clone(), HashMap::new()),
    };

    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).context("reading headers")?;
        let h = h.trim_end_matches(['\r', '\n']);
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        if headers.len() > 100 {
            anyhow::bail!("too many headers");
        }
    }

    let len: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        anyhow::bail!("request body too large");
    }
    let mut body = vec![0u8; len];
    if len > 0 {
        reader.read_exact(&mut body).context("reading the body")?;
    }
    Ok(Request {
        method,
        path: percent_decode(&path),
        query,
        headers,
        body,
    })
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    pub extra: Vec<(&'static str, String)>,
}

impl Response {
    pub fn json(status: u16, value: &serde_json::Value) -> Response {
        Response {
            status,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(value).unwrap_or_default(),
            extra: vec![],
        }
    }

    pub fn ok(value: serde_json::Value) -> Response {
        Response::json(200, &value)
    }

    pub fn error(status: u16, message: impl std::fmt::Display) -> Response {
        Response::json(status, &serde_json::json!({ "error": message.to_string() }))
    }

    pub fn dynamic(content_type: &'static str, body: String) -> Response {
        Response {
            status: 200,
            content_type,
            body: body.into_bytes(),
            extra: vec![],
        }
    }

    pub fn asset(content_type: &'static str, body: &'static str) -> Response {
        Response {
            status: 200,
            content_type,
            body: body.as_bytes().to_vec(),
            extra: vec![],
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "",
    }
}

/// Headers every response carries. The page is served from loopback and
/// talks only to itself, so the policy says exactly that: no remote
/// script, style, font or connection of any kind.
fn security_headers() -> String {
    [
        "Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        "X-Content-Type-Options: nosniff",
        "X-Frame-Options: DENY",
        "Referrer-Policy: no-referrer",
        "Cache-Control: no-store",
        "Connection: close",
    ]
    .join("\r\n")
}

pub fn write_response(stream: &mut TcpStream, r: &Response) -> Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}\r\n",
        r.status,
        reason(r.status),
        r.content_type,
        r.body.len(),
        security_headers()
    );
    for (k, v) in &r.extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&r.body)?;
    stream.flush()?;
    Ok(())
}

/// A server-sent-events channel over the raw stream. `send` returns an
/// error once the browser has gone, which is the signal to stop.
pub struct Sse<'a> {
    stream: &'a mut TcpStream,
}

impl<'a> Sse<'a> {
    pub fn start(stream: &'a mut TcpStream) -> Result<Sse<'a>> {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{}\r\n\r\n",
            security_headers()
        );
        stream.write_all(head.as_bytes())?;
        stream.flush()?;
        Ok(Sse { stream })
    }

    pub fn send(&mut self, event: &str, data: &serde_json::Value) -> Result<()> {
        let payload = serde_json::to_string(data)?;
        // One `data:` line per line of payload keeps the frame well-formed
        // whatever the JSON contains; serde never emits a raw newline.
        self.stream
            .write_all(format!("event: {event}\ndata: {payload}\n\n").as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_and_plus() {
        assert_eq!(percent_decode("a%20b+c%2Fd"), "a b c/d");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("trailing%2"), "trailing%2");
    }

    #[test]
    fn parses_query_pairs() {
        let q = parse_query("after=12&kind=gate&flag");
        assert_eq!(q["after"], "12");
        assert_eq!(q["kind"], "gate");
        assert_eq!(q["flag"], "");
    }
}
