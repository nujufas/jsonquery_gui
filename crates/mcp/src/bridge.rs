//! `jsonquery-mcp`: the MCP server of the app for an assistant that starts its servers
//! itself and talks to them over stdin and stdout (Claude Desktop, llama.cpp's
//! `llama-server` and so QuickAI, and others) — one JSON-RPC message to a line.
//!
//! It is not a server of its own: it answers the handshake and the list of tools
//! itself, from the same code as the app's server, and takes each call to the app
//! that is running, over its HTTP server, with the address and token the app keeps in
//! `mcp.json` (read again for every call, so that a new token or port is followed).
//! So the tools are there even while the app is not, and a call made then says what
//! to do about it. It keeps nothing between calls, and may be started for each one.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::catalogue::MAX_TIMEOUT;
use crate::protocol::{self, Dispatcher};
use crate::DEFAULT_PAGE;
use crate::{Access, Call, Host, PATH};

/// The variable that names the app's folder (its settings, `mcp.json`, the workspace)
/// in place of `.jsonquery` in the home: the app's own `JSONQUERY_HOME`.
pub const HOME_VARIABLE: &str = "JSONQUERY_HOME";

/// How long the app may take to accept a connection.
const CONNECT_TIME: Duration = Duration::from_secs(3);

/// What a call is waited for at most: the longest a tool may be asked to take, and a
/// little more.
const ANSWER_TIME: Duration = Duration::from_secs(MAX_TIMEOUT.as_secs() + 30);

/// The app's folder, as the app finds it (`settings.rs` of the app): what
/// [`HOME_VARIABLE`] names, or `.jsonquery` in the home — `HOME`, or on Windows
/// `USERPROFILE` (or `HOMEDRIVE` and `HOMEPATH`). Only a path from the root counts.
/// `var` reads the environment.
pub fn app_folder(var: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    let absolute = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    if let Some(folder) = absolute(HOME_VARIABLE) {
        return Some(folder);
    }
    let home = if windows {
        absolute("USERPROFILE").or_else(|| {
            let mut path = var("HOMEDRIVE").filter(|v| !v.is_empty())?;
            path.push(var("HOMEPATH").filter(|v| !v.is_empty())?);
            Some(PathBuf::from(path)).filter(|path| path.is_absolute())
        })?
    } else {
        absolute("HOME")?
    };
    Some(home.join(".jsonquery"))
}

/// Where the app's server is, and the token it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub port: u16,
    pub token: String,
}

/// The server the app keeps in `mcp.json` of `folder`.
pub fn target_in(folder: Option<&Path>) -> Result<Target, String> {
    let never = "JsonQuery's MCP server has not been set up on this computer: in JsonQuery \
                 (the build with MCP), click the 🔌 at the right of the status bar and Start";
    let folder = folder.ok_or(never)?;
    let text = match std::fs::read_to_string(folder.join("mcp.json")) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(never.to_owned()),
        Err(e) => return Err(format!("JsonQuery's mcp.json could not be read: {e}")),
    };
    let kept: Value = serde_json::from_str(&text)
        .map_err(|_| "JsonQuery's mcp.json is not what it should be".to_owned())?;
    let port = kept["port"]
        .as_u64()
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p > 0)
        .unwrap_or(crate::DEFAULT_PORT);
    let token = kept["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("JsonQuery's mcp.json has no token: start the server in JsonQuery once")?
        .to_owned();
    Ok(Target { port, token })
}

/// What answers the handshake here: the app's own, with no app behind it.
struct Unused;

impl Host for Unused {
    fn call(&self, _: Call, _: &Arc<AtomicBool>) -> Result<Value, String> {
        Err("calls go to the app".to_owned())
    }
}

/// Answer the messages read from `input` on `output`, one to a line, until `input`
/// ends. `target` says where the app's server is (asked again for every call).
pub fn serve(
    input: impl Read,
    mut output: impl Write,
    target: &dyn Fn() -> Result<Target, String>,
) -> io::Result<()> {
    let local = Dispatcher::new(
        Arc::new(Unused),
        Access::Edit,
        DEFAULT_PAGE,
        Arc::new(|| {}),
    );
    // What the assistant said of itself in its handshake, which is answered here: told
    // to the app before each call, so that its panel can name who is calling.
    let mut client = None;
    for line in BufReader::new(input).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let answers: Vec<Value> = batch
                    .iter()
                    .filter_map(|m| answer(&local, m, target, &mut client))
                    .collect();
                (!answers.is_empty()).then_some(Value::Array(answers))
            }
            Ok(message) => answer(&local, &message, target, &mut client),
            Err(e) => Some(protocol::error(
                Value::Null,
                -32700,
                &format!("not JSON: {e}"),
            )),
        };
        if let Some(answer) = answer {
            serde_json::to_writer(&mut output, &answer)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn answer(
    local: &Dispatcher,
    message: &Value,
    target: &dyn Fn() -> Result<Target, String>,
    client: &mut Option<Value>,
) -> Option<Value> {
    match message.get("method").and_then(Value::as_str) {
        // The handshake and the list are the same as the app's, from the same code.
        Some("initialize") => {
            *client = message
                .get("params")
                .filter(|p| p.get("clientInfo").is_some())
                .cloned();
            local.handle(message)
        }
        Some("ping" | "tools/list" | "notifications/initialized") | None => local.handle(message),
        Some(_) => relay(message, target, client.as_ref()),
    }
}

/// The id of the handshake the bridge makes with the app for the assistant; its
/// answer is not looked at.
const INTRODUCTION: &str = "jsonquery-mcp-introduction";

/// Take `message` to the app, and its answer back. A request is preceded by the
/// assistant's own handshake (`client`), every time: the app may have been started
/// again since the last one, and keeps nothing between requests.
fn relay(
    message: &Value,
    target: &dyn Fn() -> Result<Target, String>,
    client: Option<&Value>,
) -> Option<Value> {
    let request = message.get("id").is_some();
    let outcome = target().and_then(|target| {
        if let (true, Some(params)) = (request, client) {
            let introduction = json!({"jsonrpc": "2.0", "id": INTRODUCTION,
                                      "method": "initialize", "params": params});
            // Whatever goes wrong with it goes wrong with the call too, and is said there.
            let _ = post(&target, &introduction);
        }
        post(&target, message)
    });
    match outcome {
        Ok(answer) => answer.filter(|_| request),
        Err(why) if request => Some(unavailable(message, &why)),
        Err(_) => None,
    }
}

/// What a request that could not reach the app is answered: a tool's call, with an
/// error the assistant reads (and can tell the user); anything else, with a JSON-RPC
/// error.
fn unavailable(message: &Value, why: &str) -> Value {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if message.get("method").and_then(Value::as_str) == Some("tools/call") {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"content": [{"type": "text", "text": why}], "isError": true}
        })
    } else {
        protocol::error(id, -32603, why)
    }
}

/// POST `message` to the app's server; its JSON answer, or none for "202 Accepted".
fn post(target: &Target, message: &Value) -> Result<Option<Value>, String> {
    let not_running = "JsonQuery is not running, or its MCP server is stopped: start JsonQuery \
                       (the build with MCP) and its server, from the 🔌 at the right of its \
                       status bar";
    let address = SocketAddr::from(([127, 0, 0, 1], target.port));
    let mut stream =
        TcpStream::connect_timeout(&address, CONNECT_TIME).map_err(|_| not_running.to_owned())?;
    let _ = stream.set_read_timeout(Some(ANSWER_TIME));
    let _ = stream.set_write_timeout(Some(CONNECT_TIME));
    let body = message.to_string();
    let head = format!(
        "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        target.port,
        target.token,
        body.len()
    );
    let lost = |e: io::Error| format!("the connection to JsonQuery was lost: {e}");
    stream.write_all(head.as_bytes()).map_err(lost)?;
    stream.write_all(body.as_bytes()).map_err(lost)?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).map_err(lost)?;
    let split = reply
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("JsonQuery's answer was cut short")?;
    let head = String::from_utf8_lossy(&reply[..split]);
    let body = &reply[split + 4..];
    let status: u16 = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("JsonQuery's answer was not HTTP")?;
    if status == 202 {
        return Ok(None);
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(answer) if answer.get("jsonrpc").is_some() => Ok(Some(answer)),
        _ if status == 401 => Err(
            "JsonQuery did not take the token in its mcp.json: stop its MCP server and start it \
             again"
                .to_owned(),
        ),
        _ => Err(format!(
            "JsonQuery answered {status}: {}",
            String::from_utf8_lossy(body).trim()
        )),
    }
}

/// The program: serve stdin and stdout, for the app whose folder the environment names.
pub fn main() -> io::Result<()> {
    let folder = app_folder(|name| std::env::var_os(name), cfg!(windows));
    let target = move || target_in(folder.as_deref());
    serve(io::stdin().lock(), io::stdout().lock(), &target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Config, Server};
    use crate::protocol::tests::Echo;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    fn rooted(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{path}")
        } else {
            path.to_owned()
        }
    }

    #[test]
    fn the_folder_is_the_apps() {
        let named = rooted("/data/jq");
        let home = rooted("/home/a");
        assert_eq!(
            app_folder(env(&[(HOME_VARIABLE, &named), ("HOME", &home)]), false),
            Some(PathBuf::from(&named))
        );
        assert_eq!(
            app_folder(env(&[(HOME_VARIABLE, "relative"), ("HOME", &home)]), false),
            Some(PathBuf::from(&home).join(".jsonquery"))
        );
        assert_eq!(app_folder(env(&[]), false), None);
    }

    /// Run the bridge over `lines`, with `target`; what it wrote, a message to a line.
    fn bridge(lines: &[Value], target: &dyn Fn() -> Result<Target, String>) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let mut output = Vec::new();
        serve(input.as_bytes(), &mut output, target).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn call(id: i64, name: &str) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
               "params": {"name": name, "arguments": {}}})
    }

    #[test]
    fn the_handshake_and_the_tools_are_answered_without_the_app() {
        let nowhere = || Err::<Target, _>("no app".to_owned());
        let answers = bridge(
            &[
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                                  "clientInfo": {"name": "llama-server"}}}),
                json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
                call(3, "get_state"),
            ],
            &nowhere,
        );
        assert_eq!(answers.len(), 3, "{answers:?}");
        assert_eq!(answers[0]["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(
            answers[1]["result"]["tools"].as_array().unwrap().len(),
            crate::TOOL_NAMES.len()
        );
        // A call is the tool's error, said for the assistant to read.
        assert_eq!(answers[2]["id"], 3);
        assert_eq!(answers[2]["result"]["isError"], true);
        assert_eq!(answers[2]["result"]["content"][0]["text"], "no app");
    }

    #[test]
    fn a_call_is_taken_to_the_app_and_its_answer_brought_back() {
        let token = "0123456789abcdef0123456789abcdef".to_owned();
        let server = Server::start(
            Config {
                port: 0,
                token: token.clone(),
                access: Access::Read,
                page: DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .unwrap();
        let here = Target {
            port: server.port(),
            token: token.clone(),
        };
        let target = move || Ok(here.clone());
        let answers = bridge(
            &[
                call(7, "get_state"),
                // What needs more than the app allows is the app's to refuse.
                json!({"jsonrpc": "2.0", "id": 8, "method": "tools/call",
                       "params": {"name": "set_query", "arguments": {"query": "."}}}),
                json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                       "params": {"requestId": 1}}),
                json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list"}),
            ],
            &target,
        );
        assert_eq!(answers.len(), 3, "{answers:?}");
        assert_eq!(answers[0]["id"], 7);
        assert_eq!(answers[0]["result"]["isError"], false);
        assert_eq!(
            serde_json::from_str::<Value>(
                answers[0]["result"]["content"][0]["text"].as_str().unwrap()
            )
            .unwrap(),
            json!({"state": "ok"})
        );
        assert_eq!(answers[1]["result"]["isError"], true);
        assert_eq!(answers[2]["error"]["code"], -32601);
        assert_eq!(server.status().calls, 2);
    }

    #[test]
    fn the_app_learns_who_is_calling_through_the_bridge() {
        let token = "0123456789abcdef0123456789abcdef".to_owned();
        let server = Server::start(
            Config {
                port: 0,
                token: token.clone(),
                access: Access::Read,
                page: DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .unwrap();
        let here = Target {
            port: server.port(),
            token,
        };
        let target = move || Ok(here.clone());
        // The handshake alone is answered here and tells the app nothing.
        let handshake = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                               "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                                          "clientInfo": {"name": "llama-server", "version": "b11889"}}});
        bridge(std::slice::from_ref(&handshake), &target);
        assert!(server.status().clients.is_empty());
        // A call is preceded by it, and is still the one call.
        let answers = bridge(&[handshake, call(2, "get_state")], &target);
        assert_eq!(answers.len(), 2, "{answers:?}");
        assert_eq!(answers[1]["id"], 2);
        assert_eq!(
            server.status().clients,
            vec!["llama-server b11889".to_owned()]
        );
        assert_eq!(server.status().calls, 1);
        // An assistant that made no handshake is called nothing.
        let other = Server::start(
            Config {
                port: 0,
                token: "fedcba9876543210fedcba9876543210".to_owned(),
                access: Access::Read,
                page: DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .unwrap();
        let there = Target {
            port: other.port(),
            token: "fedcba9876543210fedcba9876543210".to_owned(),
        };
        let target = move || Ok(there.clone());
        bridge(&[call(3, "get_state")], &target);
        assert!(other.status().clients.is_empty());
        assert_eq!(other.status().calls, 1);
    }

    #[test]
    fn a_wrong_token_or_a_stopped_server_is_said_plainly() {
        let mut server = Server::start(
            Config {
                port: 0,
                token: "0123456789abcdef0123456789abcdef".to_owned(),
                access: Access::Read,
                page: DEFAULT_PAGE,
                notify: Arc::new(|| {}),
            },
            Arc::new(Echo::default()),
        )
        .unwrap();
        let port = server.port();
        let wrong = move || {
            Ok(Target {
                port,
                token: "ffffffffffffffffffffffffffffffff".to_owned(),
            })
        };
        let answers = bridge(&[call(1, "get_state")], &wrong);
        let text = answers[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("did not take the token"), "{text}");
        server.stop();
        let answers = bridge(&[call(2, "get_state")], &wrong);
        let text = answers[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not running"), "{text}");
        // Not a call: a JSON-RPC error.
        let answers = bridge(
            &[json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})],
            &wrong,
        );
        assert_eq!(answers[0]["error"]["code"], -32603);
    }

    #[test]
    fn mcp_json_says_where_the_server_is() {
        let dir = std::env::temp_dir().join(format!("jsonquery-mcp-bridge-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let error = target_in(Some(&dir)).unwrap_err();
        assert!(error.contains("click the 🔌"), "{error}");
        std::fs::write(dir.join("mcp.json"), r#"{"port": 9001, "token": "abc"}"#).unwrap();
        assert_eq!(
            target_in(Some(&dir)).unwrap(),
            Target {
                port: 9001,
                token: "abc".to_owned()
            }
        );
        std::fs::write(dir.join("mcp.json"), "{").unwrap();
        assert!(target_in(Some(&dir)).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(target_in(None).is_err());
    }
}
