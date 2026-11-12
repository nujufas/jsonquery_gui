//! The server: MCP's "Streamable HTTP" transport, on the loopback address only.
//!
//! Each message is a POST to `/mcp` and is answered by the response to it (or by
//! "202 Accepted" for a notification); the server never opens a stream of its own, so
//! a GET is "405 Method Not Allowed", as the protocol allows.
//!
//! Anything on the computer can connect to the loopback address — a web page too, by
//! a name that resolves to it (DNS rebinding) — so a request must name the server as
//! its Host, must not come from a web page's Origin, and must carry the token the app
//! made (`Authorization: Bearer …`). Each connection has a thread of its own, and there
//! are at most [`MAX_CONNECTIONS`] of them.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use crate::protocol::{self, Dispatcher, Status, PROTOCOL_VERSIONS};
use crate::{Access, Host, PATH};

/// The most bytes of a request's head (its request line and headers).
const MAX_HEAD: usize = 32 * 1024;
/// The most bytes of a request's body.
const MAX_BODY: usize = 16 * 1024 * 1024;
/// How long a connection may wait for its next request.
const IDLE: Duration = Duration::from_secs(120);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// The most connections at once.
const MAX_CONNECTIONS: usize = 32;
/// The shortest token the server takes.
const MIN_TOKEN: usize = 16;

/// How the server is started.
pub struct Config {
    /// The port on 127.0.0.1 (0: any free one).
    pub port: u16,
    /// What a request must carry: `Authorization: Bearer <token>`.
    pub token: String,
    /// What the assistant may do, to start with.
    pub access: Access,
    /// The most an answer may hold, in bytes, to start with.
    pub page: usize,
    /// Called when there is something new to show: a call began or ended, a client
    /// connected. (The app asks for a frame.)
    pub notify: Arc<dyn Fn() + Send + Sync>,
}

/// The server, while it runs. It stops when it is dropped.
pub struct Server {
    address: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

struct Shared {
    dispatcher: Dispatcher,
    token: String,
    port: u16,
    connections: AtomicUsize,
    stopping: AtomicBool,
}

impl Server {
    /// Listen on 127.0.0.1 at the port of `config`, and answer there until stopped.
    pub fn start(config: Config, host: Arc<dyn Host>) -> io::Result<Server> {
        if config.token.len() < MIN_TOKEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the token is too short",
            ));
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))?;
        let address = listener.local_addr()?;
        let shared = Arc::new(Shared {
            dispatcher: Dispatcher::new(host, config.access, config.page, config.notify),
            token: config.token,
            port: address.port(),
            connections: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
        });
        let accept = thread::Builder::new()
            .name("mcp-listen".to_owned())
            .spawn({
                let shared = shared.clone();
                move || accept_loop(&listener, &shared)
            })?;
        Ok(Server {
            address,
            shared,
            accept: Some(accept),
        })
    }

    pub fn port(&self) -> u16 {
        self.address.port()
    }

    /// Where the assistant connects to.
    pub fn url(&self) -> String {
        url_of(self.port())
    }

    pub fn status(&self) -> Status {
        self.shared.dispatcher.status()
    }

    pub fn access(&self) -> Access {
        self.shared.dispatcher.access()
    }

    pub fn set_access(&self, access: Access) {
        self.shared.dispatcher.set_access(access);
    }

    /// The most an answer may hold from the next call on, in bytes.
    pub fn set_page(&self, page: usize) {
        self.shared.dispatcher.set_page(page);
    }

    /// Stop listening, and stop the calls that are running. Waits for nothing that is
    /// answering a call (which may be waiting for the app): only for the port to be
    /// let go, so that it can be listened on again at once.
    pub fn stop(&mut self) {
        if self.shared.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        self.shared.dispatcher.stop();
        // The thread that listens is waiting for a connection: this is one.
        if TcpStream::connect_timeout(&self.address, Duration::from_secs(1)).is_ok() {
            if let Some(accept) = self.accept.take() {
                let _ = accept.join();
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Where a server on `port` is connected to.
pub fn url_of(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let Ok(mut stream) = stream else {
            // Out of file handles, say: wait a moment rather than spin.
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        if shared.connections.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            let _ = write_response(
                &mut stream,
                &Response::text(503, "too many connections"),
                false,
            );
            continue;
        }
        shared.connections.fetch_add(1, Ordering::SeqCst);
        let mine = shared.clone();
        let spawned = thread::Builder::new()
            .name("mcp-connection".to_owned())
            .spawn(move || {
                serve(stream, &mine);
                mine.connections.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            shared.connections.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// A request's line and headers.
struct Head {
    method: String,
    path: String,
    http11: bool,
    /// Names in lower case.
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn keep_alive(&self) -> bool {
        let connection = self.header("connection").map(str::to_ascii_lowercase);
        match connection.as_deref() {
            Some(c) if c.contains("close") => false,
            Some(c) if c.contains("keep-alive") => true,
            _ => self.http11,
        }
    }
}

struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    json: bool,
}

impl Response {
    fn json(status: u16, body: &Value) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
            json: true,
        }
    }

    fn text(status: u16, message: &str) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: format!("{message}\n").into_bytes(),
            json: false,
        }
    }

    fn empty(status: u16) -> Response {
        Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            json: false,
        }
    }

    fn with(mut self, name: &'static str, value: &str) -> Response {
        self.headers.push((name, value.to_owned()));
        self
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

enum ReadError {
    /// The connection ended, or went quiet: nothing to answer.
    Gone,
    Bad(Response),
}

fn serve(stream: TcpStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(IDLE));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let Ok(reading) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(reading);
    let mut writer = stream;
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let head = match read_head(&mut reader) {
            Ok(head) => head,
            Err(ReadError::Gone) => break,
            Err(ReadError::Bad(response)) => {
                let _ = write_response(&mut writer, &response, false);
                break;
            }
        };
        if let Err(response) = check(&head, shared) {
            // The body is not read: the connection ends with the answer.
            let _ = write_response(&mut writer, &response, false);
            break;
        }
        if head
            .header("expect")
            .is_some_and(|e| e.eq_ignore_ascii_case("100-continue"))
            && writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").is_err()
        {
            break;
        }
        let body = match read_body(&mut reader, &head) {
            Ok(body) => body,
            Err(ReadError::Gone) => break,
            Err(ReadError::Bad(response)) => {
                let _ = write_response(&mut writer, &response, false);
                break;
            }
        };
        let keep_alive = head.keep_alive() && !shared.stopping.load(Ordering::SeqCst);
        let response = answer(&body, shared);
        if write_response(&mut writer, &response, keep_alive).is_err() || !keep_alive {
            break;
        }
    }
}

fn read_head(reader: &mut impl BufRead) -> Result<Head, ReadError> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let n = match reader
            .by_ref()
            .take((MAX_HEAD + 1 - total) as u64)
            .read_until(b'\n', &mut line)
        {
            Ok(n) => n,
            Err(_) => return Err(ReadError::Gone),
        };
        if n == 0 {
            return Err(if lines.is_empty() {
                ReadError::Gone
            } else {
                ReadError::Bad(Response::text(400, "the request ended in its head"))
            });
        }
        total += n;
        if total > MAX_HEAD {
            return Err(ReadError::Bad(Response::text(
                431,
                "the request's head is too long",
            )));
        }
        let text = String::from_utf8_lossy(&line)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if text.is_empty() {
            if lines.is_empty() {
                continue; // a blank line before a request is let pass
            }
            break;
        }
        lines.push(text);
    }
    let bad = |what: &str| ReadError::Bad(Response::text(400, what));
    let mut parts = lines[0].split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad("the request line is not one"));
    };
    if !version.starts_with("HTTP/1.") {
        return Err(bad("only HTTP/1.x is spoken here"));
    }
    let mut headers = Vec::new();
    for line in &lines[1..] {
        if line.starts_with([' ', '\t']) {
            return Err(bad("headers are not folded"));
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(bad("a header line has no colon"));
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    Ok(Head {
        method: method.to_owned(),
        path: path.to_owned(),
        http11: version == "HTTP/1.1",
        headers,
    })
}

fn read_body(reader: &mut impl BufRead, head: &Head) -> Result<Vec<u8>, ReadError> {
    let too_large = || {
        ReadError::Bad(Response::text(
            413,
            "the request is too large: put less in one call",
        ))
    };
    if let Some(encoding) = head.header("transfer-encoding") {
        if !encoding.to_ascii_lowercase().contains("chunked") {
            return Err(ReadError::Bad(Response::text(
                400,
                "unknown transfer encoding",
            )));
        }
        let mut body = Vec::new();
        loop {
            let mut line = String::new();
            if reader.by_ref().take(1024).read_line(&mut line).is_err() || line.is_empty() {
                return Err(ReadError::Gone);
            }
            let size = line.trim().split(';').next().unwrap_or("");
            let Ok(size) = usize::from_str_radix(size.trim(), 16) else {
                return Err(ReadError::Bad(Response::text(
                    400,
                    "a chunk's size is not one",
                )));
            };
            if size == 0 {
                // The trailer, up to its blank line.
                loop {
                    let mut trailer = String::new();
                    match reader
                        .by_ref()
                        .take(MAX_HEAD as u64)
                        .read_line(&mut trailer)
                    {
                        Ok(0) | Err(_) => return Err(ReadError::Gone),
                        Ok(_) if trailer.trim().is_empty() => return Ok(body),
                        Ok(_) => {}
                    }
                }
            }
            if body.len() + size > MAX_BODY {
                return Err(too_large());
            }
            let start = body.len();
            body.resize(start + size, 0);
            if reader.read_exact(&mut body[start..]).is_err() {
                return Err(ReadError::Gone);
            }
            let mut end = [0u8; 2];
            if reader.read_exact(&mut end).is_err() || &end != b"\r\n" {
                return Err(ReadError::Bad(Response::text(
                    400,
                    "a chunk does not end in CRLF",
                )));
            }
        }
    }
    let Some(length) = head.header("content-length") else {
        return Err(ReadError::Bad(Response::text(
            411,
            "the request needs a Content-Length",
        )));
    };
    let Ok(length) = length.parse::<usize>() else {
        return Err(ReadError::Bad(Response::text(
            400,
            "Content-Length is not a number",
        )));
    };
    if length > MAX_BODY {
        return Err(too_large());
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).map_err(|_| ReadError::Gone)?;
    Ok(body)
}

/// Whether the request may be answered: the right path, the server named as its Host,
/// no web page's Origin, the token, a POST of JSON, a version of the protocol the
/// server speaks.
fn check(head: &Head, shared: &Shared) -> Result<(), Response> {
    let path = head.path.split('?').next().unwrap_or("");
    if path != PATH && path != format!("{PATH}/") {
        return Err(Response::text(404, "the MCP server is at /mcp"));
    }
    let port = shared.port;
    let host = head.header("host").map(str::to_ascii_lowercase);
    let host_ok = host.is_some_and(|h| {
        ["127.0.0.1", "localhost", "[::1]"]
            .iter()
            .any(|name| h == format!("{name}:{port}"))
    });
    if !host_ok {
        return Err(Response::text(403, "this server answers only to 127.0.0.1"));
    }
    if let Some(origin) = head.header("origin") {
        let origin = origin.to_ascii_lowercase();
        let ok = ["127.0.0.1", "localhost"]
            .iter()
            .any(|name| origin == format!("http://{name}:{port}"));
        if !ok {
            return Err(Response::text(
                403,
                "requests from web pages are not answered",
            ));
        }
    }
    let token_ok = head.header("authorization").is_some_and(|value| {
        let mut parts = value.splitn(2, ' ');
        let scheme = parts.next().unwrap_or("");
        let token = parts.next().unwrap_or("").trim();
        scheme.eq_ignore_ascii_case("bearer") && same(token.as_bytes(), shared.token.as_bytes())
    });
    if !token_ok {
        return Err(Response::text(
            401,
            "a token is needed: copy the configuration from JsonQuery's MCP panel",
        )
        .with("WWW-Authenticate", "Bearer realm=\"jsonquery\""));
    }
    if head.method != "POST" {
        return Err(Response::text(405, "only POST is answered here").with("Allow", "POST"));
    }
    let json = head
        .header("content-type")
        .is_some_and(|t| t.to_ascii_lowercase().starts_with("application/json"));
    if !json {
        return Err(Response::text(415, "a message is sent as application/json"));
    }
    if let Some(version) = head.header("mcp-protocol-version") {
        if !PROTOCOL_VERSIONS.contains(&version) {
            // As a server of these versions must, "400 Bad Request"; with the error a
            // client of a later version looks for, which names the versions there are,
            // so that it falls back to the handshake they begin with.
            let mut error = protocol::error(
                Value::Null,
                UNSUPPORTED_PROTOCOL_VERSION,
                "Unsupported protocol version",
            );
            error["error"]["data"] = serde_json::json!({
                "supported": PROTOCOL_VERSIONS,
                "requested": version,
            });
            return Err(Response::json(400, &error));
        }
    }
    Ok(())
}

/// The error a server gives for a version of the protocol it does not speak (the code
/// of the protocol's later versions).
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// The same bytes, compared in the same time whichever byte differs.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn answer(body: &[u8], shared: &Shared) -> Response {
    let message: Value = match serde_json::from_slice(body) {
        Ok(message) => message,
        Err(e) => {
            return Response::json(
                400,
                &protocol::error(Value::Null, -32700, &format!("not JSON: {e}")),
            )
        }
    };
    match shared.dispatcher.handle(&message) {
        Some(reply) => Response::json(200, &reply),
        None => Response::empty(202),
    }
}

fn write_response(out: &mut impl Write, response: &Response, keep_alive: bool) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n",
        response.status,
        reason(response.status),
        response.body.len()
    );
    if !response.body.is_empty() {
        head.push_str(if response.json {
            "Content-Type: application/json\r\n"
        } else {
            "Content-Type: text/plain; charset=utf-8\r\n"
        });
    }
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !keep_alive {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes())?;
    out.write_all(&response.body)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::tests::Echo;
    use serde_json::json;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn server() -> Server {
        Server::start(
            Config {
                port: 0,
                token: TOKEN.to_owned(),
                access: Access::Read,
                page: crate::DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .unwrap()
    }

    /// Send `raw` and read until the server closes the connection.
    fn exchange(server: &Server, raw: &[u8]) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(raw).unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    }

    fn post(server: &Server, headers: &[&str], body: &str) -> String {
        let mut raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n",
            server.port()
        );
        for header in headers {
            raw.push_str(header);
            raw.push_str("\r\n");
        }
        raw.push_str(&format!(
            "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        exchange(server, raw.as_bytes())
    }

    const AUTH: &str = "Authorization: Bearer 0123456789abcdef0123456789abcdef";
    const JSON: &str = "Content-Type: application/json";
    const PING: &str = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;

    fn status_of(response: &str) -> u16 {
        response
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("no status in {response:?}"))
    }

    fn body_of(response: &str) -> Value {
        serde_json::from_str(response.split("\r\n\r\n").last().unwrap()).unwrap()
    }

    #[test]
    fn a_request_with_the_token_is_answered() {
        let s = server();
        let response = post(&s, &[AUTH, JSON], PING);
        assert_eq!(status_of(&response), 200, "{response}");
        assert!(response.contains("Content-Type: application/json"));
        assert_eq!(
            body_of(&response),
            json!({"jsonrpc": "2.0", "id": 1, "result": {}})
        );
    }

    #[test]
    fn without_the_token_nothing_is_answered() {
        let s = server();
        let response = post(&s, &[JSON], PING);
        assert_eq!(status_of(&response), 401);
        assert!(response.contains("WWW-Authenticate: Bearer"));
        let wrong = post(
            &s,
            &[
                "Authorization: Bearer 0123456789abcdef0123456789abcdeX",
                JSON,
            ],
            PING,
        );
        assert_eq!(status_of(&wrong), 401);
        let basic = post(
            &s,
            &[
                "Authorization: Basic 0123456789abcdef0123456789abcdef",
                JSON,
            ],
            PING,
        );
        assert_eq!(status_of(&basic), 401);
        assert_eq!(s.status().calls, 0);
    }

    #[test]
    fn a_web_page_or_another_host_name_is_turned_away() {
        let s = server();
        let page = post(&s, &[AUTH, JSON, "Origin: https://evil.example"], PING);
        assert_eq!(status_of(&page), 403);
        let null = post(&s, &[AUTH, JSON, "Origin: null"], PING);
        assert_eq!(status_of(&null), 403);
        let own = format!("Origin: http://localhost:{}", s.port());
        assert_eq!(status_of(&post(&s, &[AUTH, JSON, &own], PING)), 200);
        let raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: rebound.example:{}\r\n{AUTH}\r\n{JSON}\r\nContent-Length: {}\r\n\r\n{PING}",
            s.port(),
            PING.len()
        );
        assert_eq!(status_of(&exchange(&s, raw.as_bytes())), 403);
    }

    #[test]
    fn only_a_post_of_json_to_mcp_is_answered() {
        let s = server();
        let get = format!(
            "GET /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{AUTH}\r\n\r\n",
            s.port()
        );
        let response = exchange(&s, get.as_bytes());
        assert_eq!(status_of(&response), 405);
        assert!(response.contains("Allow: POST"));
        let other = format!(
            "POST /other HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{AUTH}\r\nContent-Length: 0\r\n\r\n",
            s.port()
        );
        assert_eq!(status_of(&exchange(&s, other.as_bytes())), 404);
        assert_eq!(
            status_of(&post(&s, &[AUTH, "Content-Type: text/plain"], PING)),
            415
        );
        let old = post(&s, &[AUTH, JSON, "MCP-Protocol-Version: 2023-01-01"], PING);
        assert_eq!(status_of(&old), 400);
        let error = &body_of(&old)["error"];
        assert_eq!(error["code"], -32022);
        assert_eq!(error["data"]["requested"], "2023-01-01");
        assert_eq!(error["data"]["supported"][0], PROTOCOL_VERSIONS[0]);
        let known = post(&s, &[AUTH, JSON, "MCP-Protocol-Version: 2025-06-18"], PING);
        assert_eq!(status_of(&known), 200);
    }

    #[test]
    fn a_notification_is_accepted_and_bad_json_is_a_parse_error() {
        let s = server();
        let note = post(
            &s,
            &[AUTH, JSON],
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        assert_eq!(status_of(&note), 202);
        assert!(note.contains("Content-Length: 0"));
        let bad = post(&s, &[AUTH, JSON], "{not json");
        assert_eq!(status_of(&bad), 400);
        assert_eq!(body_of(&bad)["error"]["code"], -32700);
    }

    #[test]
    fn a_connection_is_kept_for_the_next_request() {
        let s = server();
        let one = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost:{}\r\n{AUTH}\r\n{JSON}\r\nContent-Length: {}\r\n\r\n{PING}",
            s.port(),
            PING.len()
        );
        let two = one.replace("\r\n\r\n", "\r\nConnection: close\r\n\r\n");
        let response = exchange(&s, format!("{one}{two}").as_bytes());
        assert_eq!(response.matches("HTTP/1.1 200 OK").count(), 2, "{response}");
    }

    #[test]
    fn expect_continue_and_chunked_bodies_are_understood() {
        let s = server();
        let (a, b) = PING.split_at(10);
        let raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{AUTH}\r\n{JSON}\r\nExpect: 100-continue\r\n\
             Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{a}\r\n{:x};ext=1\r\n{b}\r\n0\r\n\r\n",
            s.port(),
            a.len(),
            b.len()
        );
        let response = exchange(&s, raw.as_bytes());
        assert!(
            response.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK"),
            "{response}"
        );
        assert_eq!(body_of(&response)["id"], 1);
    }

    #[test]
    fn what_is_too_big_or_malformed_is_refused() {
        let s = server();
        let huge = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{AUTH}\r\n{JSON}\r\nContent-Length: {}\r\n\r\n",
            s.port(),
            MAX_BODY + 1
        );
        assert_eq!(status_of(&exchange(&s, huge.as_bytes())), 413);
        let long_head = format!("GET /mcp HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(MAX_HEAD));
        assert_eq!(status_of(&exchange(&s, long_head.as_bytes())), 431);
        assert_eq!(status_of(&exchange(&s, b"NONSENSE\r\n\r\n")), 400);
        let no_length = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{AUTH}\r\n{JSON}\r\n\r\n",
            s.port()
        );
        assert_eq!(status_of(&exchange(&s, no_length.as_bytes())), 411);
    }

    #[test]
    fn a_stopped_server_lets_go_of_its_port_at_once() {
        let mut s = server();
        let port = s.port();
        s.stop();
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
        let again = Server::start(
            Config {
                port,
                token: TOKEN.to_owned(),
                access: Access::Read,
                page: crate::DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .unwrap();
        assert_eq!(status_of(&post(&again, &[AUTH, JSON], PING)), 200);
    }

    #[test]
    fn a_short_token_is_refused() {
        let error = Server::start(
            Config {
                port: 0,
                token: "short".to_owned(),
                access: Access::Read,
                page: crate::DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn the_whole_handshake_works_over_http() {
        let s = server();
        let init = post(
            &s,
            &[AUTH, JSON],
            r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#,
        );
        assert_eq!(body_of(&init)["result"]["protocolVersion"], "2025-11-25");
        let list = post(
            &s,
            &[AUTH, JSON],
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert!(body_of(&list)["result"]["tools"].as_array().unwrap().len() > 10);
        let call = post(
            &s,
            &[AUTH, JSON],
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_state","arguments":{}}}"#,
        );
        assert_eq!(body_of(&call)["result"]["isError"], false);
        assert_eq!(s.status().clients, vec!["test 1".to_owned()]);
    }
}
