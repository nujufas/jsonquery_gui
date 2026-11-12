//! The Model Context Protocol over JSON-RPC 2.0: the handshake, the list of tools,
//! the calls, and what is kept of them for the app to show (what was called, how it
//! went, who is connected).
//!
//! The server keeps no sessions: every request stands on its own, so an assistant
//! that was connected before the app was restarted goes on working as it was.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Map, Value};

use crate::catalogue::{self, Call};
use crate::{help, Access, Host, SERVER_NAME};

/// The versions of the protocol the server speaks, newest first. A client that asks
/// for one of them gets it; any other is offered the newest.
pub const PROTOCOL_VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// How many calls the app is told of.
const RECENT: usize = 50;
/// How many clients it names.
const CLIENTS: usize = 5;
/// The room the word on an answer that was cut takes.
const CUT_NOTE: usize = 300;

/// What the assistant is told of the server when it connects.
const INSTRUCTIONS: &str = "\
JsonQuery is a desktop app for exploring and querying JSON (jq, JSONPath, JMESPath, JSON \
Pointer). These tools work on what the user has open in it: the Source (the open document), the \
query box and its Results, and the user's Workspace of saved queries.
- Start with get_state. Documents can be gigabytes: learn one with document_info and describe \
before reading it, then read it with read_value, or query it with run_query.
- Every answer is at most a page, of the size the user set (small for a model with a small \
context): where there is more, next_offset says where the next page starts. Narrow the query \
rather than read everything.
- The tools that only read work out of sight: the user sees nothing change. They take any \
document as `input`: \"document\" (open in the app), \"results\" (the user's), {\"file\": …}, \
{\"workspace\": …}, {\"json\": …}, {\"text\": …}, or \"last\": what your last run_query or \
tool of the Tools gave, kept by the app, so that a big result is read, queried or saved without \
passing through the conversation.
- The others (open_document, set_query, reveal, workspace_open, show_in_tool, saving, moving and \
deleting in the workspace, save_file, open_url), and reading a file, need the user to allow \
\"Edit\" in the app's MCP panel; the app may ask the user before anything is deleted, replaced, \
written outside the workspace or fetched.
- The jq here is jaq: query_help with topic \"known issues\" says where it differs from jq.";

/// One call of a tool, as the app shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Activity {
    pub at: SystemTime,
    pub tool: String,
    pub ok: bool,
    pub took: Duration,
    /// What went wrong, for a call that did not go well.
    pub error: Option<String>,
}

/// What the app shows of the server.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// Calls of tools since the server started.
    pub calls: u64,
    /// Calls that are being answered now.
    pub running: usize,
    /// The assistants that have connected, the last first (their name and version).
    pub clients: Vec<String>,
    /// The last calls, the last first.
    pub recent: Vec<Activity>,
}

/// What answers the messages: the same for every connection.
pub(crate) struct Dispatcher {
    host: Arc<dyn Host>,
    access: AtomicU8,
    /// The most an answer may hold, in bytes (the user's choice).
    page: AtomicUsize,
    status: Mutex<Status>,
    recent: Mutex<VecDeque<Activity>>,
    /// The calls being answered, by the id of their request, each with what stops it.
    running: Mutex<Vec<(Value, Arc<AtomicBool>)>>,
    stopping: AtomicBool,
    notify: Arc<dyn Fn() + Send + Sync>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Dispatcher {
    pub(crate) fn new(
        host: Arc<dyn Host>,
        access: Access,
        page: usize,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Dispatcher {
            host,
            access: AtomicU8::new(access.as_u8()),
            page: AtomicUsize::new(page),
            status: Mutex::new(Status::default()),
            recent: Mutex::new(VecDeque::new()),
            running: Mutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
            notify,
        }
    }

    pub(crate) fn access(&self) -> Access {
        Access::from_u8(self.access.load(Ordering::Relaxed))
    }

    pub(crate) fn set_access(&self, access: Access) {
        self.access.store(access.as_u8(), Ordering::Relaxed);
    }

    pub(crate) fn page(&self) -> usize {
        self.page.load(Ordering::Relaxed)
    }

    pub(crate) fn set_page(&self, page: usize) {
        self.page.store(page, Ordering::Relaxed);
    }

    pub(crate) fn status(&self) -> Status {
        let mut status = locked(&self.status).clone();
        status.recent = locked(&self.recent).iter().cloned().collect();
        status
    }

    /// Stop what is running, and answer nothing more.
    pub(crate) fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        for (_, cancel) in locked(&self.running).iter() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// The answer to one message (or to a batch of them): none for a notification,
    /// or for a response.
    pub(crate) fn handle(&self, message: &Value) -> Option<Value> {
        match message {
            Value::Array(batch) if !batch.is_empty() => {
                let answers: Vec<Value> = batch.iter().filter_map(|m| self.handle_one(m)).collect();
                (!answers.is_empty()).then_some(Value::Array(answers))
            }
            _ => self.handle_one(message),
        }
    }

    fn handle_one(&self, message: &Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(error(Value::Null, -32600, "a message is a JSON object"));
        };
        let id = object.get("id").cloned();
        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error(
                id.unwrap_or(Value::Null),
                -32600,
                "\"jsonrpc\" must be \"2.0\"",
            ));
        }
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            // A response: the server asks nothing, so there is nothing to do with it.
            return None;
        };
        let Some(id) = id else {
            self.notification(method, &params);
            return None;
        };
        if !(id.is_string() || id.is_i64() || id.is_u64()) {
            return Some(error(
                Value::Null,
                -32600,
                "an id is a string or a whole number",
            ));
        }
        Some(match self.request(method, &id, &params) {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => error(id, code, &message),
        })
    }

    fn notification(&self, method: &str, params: &Value) {
        if method == "notifications/cancelled" {
            if let Some(id) = params.get("requestId") {
                for (running, cancel) in locked(&self.running).iter() {
                    if running == id {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
            }
        }
        // "notifications/initialized" and anything else need nothing done.
    }

    fn request(&self, method: &str, id: &Value, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": catalogue::tool_list()})),
            "tools/call" => self.call_tool(id, params),
            _ => Err((-32601, format!("no method \"{method}\""))),
        }
    }

    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = asked
            .filter(|v| PROTOCOL_VERSIONS.contains(v))
            .unwrap_or(PROTOCOL_VERSIONS[0]);
        if let Some(client) = params.get("clientInfo") {
            let name = client.get("name").and_then(Value::as_str).unwrap_or("?");
            let label = match client.get("version").and_then(Value::as_str) {
                Some(v) => format!("{name} {v}"),
                None => name.to_owned(),
            };
            let mut status = locked(&self.status);
            status.clients.retain(|c| *c != label);
            status.clients.insert(0, label);
            status.clients.truncate(CLIENTS);
        }
        (self.notify)();
        json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {
                "name": SERVER_NAME,
                "title": "JsonQuery",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": INSTRUCTIONS
        })
    }

    fn call_tool(&self, id: &Value, params: &Value) -> Result<Value, (i64, String)> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err((-32602, "tools/call needs the tool's \"name\"".to_owned()));
        };
        let Some(parsed) = catalogue::parse(name, params.get("arguments")) else {
            return Err((-32602, format!("no tool \"{name}\"")));
        };
        let started = Instant::now();
        let outcome = match parsed {
            Err(e) => Err(format!("{name}: {e}")),
            Ok(call) if call.needs() > self.access() => Err(format!(
                "{name} needs the user to allow \"{}\": it is set to \"{}\" in JsonQuery's MCP \
                 panel (the button in its status bar). Ask the user to change it, or use the \
                 tools that only read.",
                call.needs().label(),
                self.access().label()
            )),
            Ok(_) if self.stopping.load(Ordering::Relaxed) => {
                Err("the JsonQuery MCP server is stopping".to_owned())
            }
            Ok(mut call) => {
                call.fit(self.page());
                self.run(id, call)
            }
        };
        let took = started.elapsed();
        self.note(name, &outcome, took);
        Ok(match outcome {
            Ok(Value::String(text)) => tool_result(self.fitted(text), false),
            Ok(value) => tool_result(self.fitted(value.to_string()), false),
            Err(message) => tool_result(message, true),
        })
    }

    /// `text` as it is, if it fits in a page (with room to spare: the tools keep what
    /// they give to about a page); or the start of it, said to be cut. A tool should
    /// never give that much: this is so that a model with little room is never given
    /// more than the user said it can take.
    fn fitted(&self, text: String) -> String {
        let page = self.page();
        if text.len() <= page + page / 4 {
            return text;
        }
        let mut end = page.saturating_sub(CUT_NOTE);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{}\n[cut: the answer was {} bytes, more than the {page} a page may hold (as the \
             user set it in JsonQuery). Ask for less: a smaller limit, a value further in, a \
             narrower query.]",
            &text[..end],
            text.len()
        )
    }

    fn run(&self, id: &Value, call: Call) -> Result<Value, String> {
        if let Call::Help {
            language,
            topic,
            offset,
            max_bytes,
        } = &call
        {
            let text = help::help(*language, topic.as_deref());
            return Ok(Value::String(help::page(&text, *offset, *max_bytes)));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        locked(&self.running).push((id.clone(), cancel.clone()));
        locked(&self.status).running += 1;
        (self.notify)();
        let answer = catch_unwind(AssertUnwindSafe(|| self.host.call(call, &cancel)))
            .unwrap_or_else(|_| {
                Err("the app stopped this call because of an internal error".to_owned())
            });
        locked(&self.running).retain(|(_, c)| !Arc::ptr_eq(c, &cancel));
        let mut status = locked(&self.status);
        status.running = status.running.saturating_sub(1);
        if cancel.load(Ordering::Relaxed) && answer.is_err() {
            return answer.map_err(|e| format!("{e} (cancelled)"));
        }
        answer
    }

    fn note(&self, tool: &str, outcome: &Result<Value, String>, took: Duration) {
        locked(&self.status).calls += 1;
        let mut recent = locked(&self.recent);
        recent.push_front(Activity {
            at: SystemTime::now(),
            tool: tool.to_owned(),
            ok: outcome.is_ok(),
            took,
            error: outcome
                .as_ref()
                .err()
                .map(|e| e.chars().take(200).collect()),
        });
        recent.truncate(RECENT);
        drop(recent);
        (self.notify)();
    }
}

fn tool_result(text: String, is_error: bool) -> Value {
    let mut result = Map::new();
    result.insert(
        "content".to_owned(),
        json!([{"type": "text", "text": text}]),
    );
    result.insert("isError".to_owned(), Value::Bool(is_error));
    Value::Object(result)
}

pub(crate) fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A host that answers every call with what it was asked, and counts them.
    #[derive(Default)]
    pub(crate) struct Echo {
        pub(crate) calls: AtomicUsize,
    }

    impl Host for Echo {
        fn call(&self, call: Call, cancel: &Arc<AtomicBool>) -> Result<Value, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            match call {
                Call::State => Ok(json!({"state": "ok"})),
                Call::Search { text, .. } if text == "boom" => panic!("boom"),
                Call::Search {
                    text, max_bytes, ..
                } if text == "big" => {
                    Ok(json!({"max_bytes": max_bytes, "text": "x".repeat(50_000)}))
                }
                Call::Search { text, .. } if text == "wait" => {
                    let until = Instant::now() + Duration::from_secs(5);
                    while !cancel.load(Ordering::Relaxed) && Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err("stopped".to_owned())
                }
                other => Ok(json!({"tool": other.tool()})),
            }
        }
    }

    pub(crate) fn dispatcher(access: Access) -> (Arc<Dispatcher>, Arc<Echo>) {
        let echo = Arc::new(Echo::default());
        let host: Arc<dyn Host> = echo.clone();
        (
            Arc::new(Dispatcher::new(
                host,
                access,
                crate::DEFAULT_PAGE,
                Arc::new(|| {}),
            )),
            echo,
        )
    }

    fn ask(d: &Dispatcher, id: i64, method: &str, params: Value) -> Value {
        d.handle(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .expect("an answer")
    }

    fn call(d: &Dispatcher, name: &str, arguments: Value) -> (String, bool) {
        let answer = ask(
            d,
            9,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        );
        let result = &answer["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap(),
        )
    }

    #[test]
    fn initialize_gives_the_version_asked_for_or_the_newest() {
        let (d, _) = dispatcher(Access::Read);
        let answer = ask(
            &d,
            1,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "claude-code", "version": "2.1"}}),
        );
        assert_eq!(answer["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(answer["result"]["serverInfo"]["name"], "jsonquery");
        assert!(answer["result"]["capabilities"]["tools"].is_object());
        assert_eq!(d.status().clients, vec!["claude-code 2.1".to_owned()]);
        let answer = ask(
            &d,
            2,
            "initialize",
            json!({"protocolVersion": "1999-01-01"}),
        );
        assert_eq!(answer["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn a_notification_or_a_response_gets_no_answer() {
        let (d, _) = dispatcher(Access::Read);
        assert!(d
            .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .is_none());
        assert!(d
            .handle(&json!({"jsonrpc": "2.0", "id": 3, "result": {}}))
            .is_none());
    }

    #[test]
    fn what_is_not_a_request_is_said_to_be_so() {
        let (d, _) = dispatcher(Access::Read);
        assert_eq!(d.handle(&json!(7)).unwrap()["error"]["code"], -32600);
        assert_eq!(
            d.handle(&json!({"id": 1, "method": "ping"})).unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            d.handle(&json!({"jsonrpc": "2.0", "id": null, "method": "ping"}))
                .unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            ask(&d, 4, "resources/list", json!({}))["error"]["code"],
            -32601
        );
        assert_eq!(ask(&d, 5, "ping", json!({}))["result"], json!({}));
    }

    #[test]
    fn a_batch_is_answered_by_a_batch_of_its_requests() {
        let (d, _) = dispatcher(Access::Read);
        let answer = d
            .handle(&json!([
                {"jsonrpc": "2.0", "id": 1, "method": "ping"},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {"jsonrpc": "2.0", "id": 2, "method": "ping"}
            ]))
            .unwrap();
        assert_eq!(answer.as_array().unwrap().len(), 2);
    }

    #[test]
    fn tools_are_listed() {
        let (d, _) = dispatcher(Access::Read);
        let tools = ask(&d, 1, "tools/list", json!({}));
        assert_eq!(
            tools["result"]["tools"].as_array().unwrap().len(),
            catalogue::TOOL_NAMES.len()
        );
    }

    #[test]
    fn a_call_reaches_the_host_and_its_answer_is_the_text() {
        let (d, echo) = dispatcher(Access::Read);
        let (text, is_error) = call(&d, "get_state", json!({}));
        assert!(!is_error);
        assert_eq!(
            serde_json::from_str::<Value>(&text).unwrap(),
            json!({"state": "ok"})
        );
        assert_eq!(echo.calls.load(Ordering::Relaxed), 1);
        let status = d.status();
        assert_eq!(status.calls, 1);
        assert_eq!(status.recent[0].tool, "get_state");
        assert!(status.recent[0].ok);
    }

    #[test]
    fn every_answer_is_kept_to_the_page_the_user_set() {
        let (d, _) = dispatcher(Access::Read);
        d.set_page(8_000);
        let (text, is_error) = call(&d, "search", json!({"text": "big"}));
        assert!(!is_error);
        // The tool is told how much it may give, and what it gives beyond is cut.
        assert!(text.starts_with(r#"{"max_bytes":7000,"#), "{}", &text[..60]);
        assert!(text.len() <= 8_000, "{}", text.len());
        assert!(
            text.ends_with("a narrower query.]"),
            "{}",
            &text[text.len() - 80..]
        );
        let (text, _) = call(&d, "get_state", json!({}));
        assert_eq!(text, r#"{"state":"ok"}"#);
        // The help, answered here, comes a page at a time too.
        let (help, _) = call(&d, "query_help", json!({"language": "jq"}));
        assert!(
            help.len() <= 8_000 && help.contains("[more: query_help"),
            "{}",
            help.len()
        );
    }

    #[test]
    fn a_call_needing_more_than_the_user_allows_never_reaches_the_host() {
        let (d, echo) = dispatcher(Access::Read);
        let (text, is_error) = call(&d, "set_query", json!({"query": ".a"}));
        assert!(is_error);
        assert!(
            text.contains("allow \"Edit\"") && text.contains("set to \"Read\""),
            "{text}"
        );
        assert_eq!(echo.calls.load(Ordering::Relaxed), 0);
        assert!(!d.status().recent[0].ok);
        d.set_access(Access::Edit);
        assert!(!call(&d, "set_query", json!({"query": ".a"})).1);
        assert_eq!(echo.calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn bad_arguments_are_a_tool_error_and_an_unknown_tool_a_protocol_error() {
        let (d, echo) = dispatcher(Access::Edit);
        let (text, is_error) = call(&d, "search", json!({}));
        assert!(is_error && text.contains("`text` is needed"), "{text}");
        let answer = ask(&d, 1, "tools/call", json!({"name": "nope"}));
        assert_eq!(answer["error"]["code"], -32602);
        assert_eq!(echo.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn help_is_answered_without_the_host() {
        let (d, echo) = dispatcher(Access::Read);
        let (text, is_error) = call(&d, "query_help", json!({}));
        assert!(!is_error && text.contains("JSONPath"));
        assert_eq!(echo.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_host_that_panics_gives_an_error_not_silence() {
        let (d, _) = dispatcher(Access::Read);
        let (text, is_error) = call(&d, "search", json!({"text": "boom"}));
        assert!(is_error && text.contains("internal error"), "{text}");
        assert_eq!(d.status().running, 0);
    }

    #[test]
    fn a_cancelled_call_is_stopped_and_says_so() {
        let (d, _) = dispatcher(Access::Read);
        let worker = {
            let d = d.clone();
            std::thread::spawn(move || {
                d.handle(&json!({"jsonrpc": "2.0", "id": "w", "method": "tools/call",
                                 "params": {"name": "search", "arguments": {"text": "wait"}}}))
                    .unwrap()
            })
        };
        let until = Instant::now() + Duration::from_secs(5);
        while d.status().running == 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(2));
        }
        d.handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                         "params": {"requestId": "w"}}),
        );
        let answer = worker.join().unwrap();
        assert_eq!(answer["result"]["isError"], true);
        assert!(answer["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("cancelled"));
    }
}
