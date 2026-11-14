//! The MCP server in the app, end to end: its tools on an app that has no window and
//! draws no frame — as a hidden or covered window draws none — and the server over
//! HTTP in front of them.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::sync::MutexGuard;
use std::time::Duration;

use jsonquery_core::DocumentSource;
use jsonquery_mcp::{Call, Host, Target};
use jsonquery_query::Kind;
use serde_json::{json, Value};

use super::*;
use crate::scratch_dir::ScratchDir;
use crate::settings::Settings;

const WAIT: Duration = Duration::from_secs(20);

/// An app, and the host its server would have.
struct Rig {
    app: Arc<Mutex<App>>,
    ctx: egui::Context,
    host: AppHost,
    cancel: Arc<AtomicBool>,
}

impl Rig {
    fn new() -> Self {
        Self::with(Settings::default())
    }

    fn with(settings: Settings) -> Self {
        let ctx = egui::Context::default();
        let app = Arc::new(Mutex::new(App::with_settings(
            &ctx,
            Store::none(),
            settings,
        )));
        lock(&app).mcp.me = Arc::downgrade(&app);
        let host = AppHost::new(Arc::downgrade(&app), ctx.clone(), FileLimits::default());
        Rig {
            app,
            ctx,
            host,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn app(&self) -> MutexGuard<'_, App> {
        lock(&self.app)
    }

    /// Show `json` as the open document, as a paste does.
    fn open(&self, json: &str) {
        let doc = jsonquery_core::load_text(json).unwrap();
        self.app().document_loaded(Arc::new(doc));
    }

    fn call(&self, call: Call) -> Result<Value, String> {
        self.host.call(call, &self.cancel)
    }
}

fn run(query: &str, offset: usize, limit: usize) -> Call {
    Call::RunQuery {
        query: query.to_owned(),
        language: None,
        input: Input::Document,
        offset,
        limit,
        max_bytes: 24_000,
        timeout: WAIT,
    }
}

fn set(query: &str, language: Option<Kind>) -> Call {
    Call::SetQuery {
        query: query.to_owned(),
        language,
        run: true,
        limit: 20,
        max_bytes: 24_000,
        timeout: WAIT,
    }
}

fn read(input: Input, pointer: &str) -> Call {
    Call::ReadValue {
        input,
        pointer: pointer.to_owned(),
        offset: 0,
        limit: 20,
        max_bytes: 24_000,
    }
}

fn write(dir: &ScratchDir, rel: &str, text: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn state_says_what_is_open_and_what_is_allowed() {
    let rig = Rig::new();
    let state = rig.call(Call::State).unwrap();
    assert_eq!(state["document"], Value::Null);
    assert_eq!(state["access"], "read");
    assert_eq!(state["in_front"], "Json Query");
    rig.open(r#"{"a": [1, 2, 3]}"#);
    rig.app().query_text = ".a".to_owned();
    let state = rig.call(Call::State).unwrap();
    assert_eq!(state["document"]["source"], "pasted");
    assert_eq!(state["document"]["top"], "object with 1 member");
    assert_eq!(state["query"]["text"], ".a");
    assert_eq!(state["query"]["language"], "jq");
    assert_eq!(state["query"]["language_chosen"], false);
}

#[test]
fn run_query_answers_without_changing_what_the_user_sees() {
    let rig = Rig::new();
    rig.open("[1, 2, 3]");
    rig.app().query_text = ".mine".to_owned();
    let out = rig.call(run(".[] * 2", 0, 20)).unwrap();
    assert_eq!(out["items"], json!([2, 4, 6]));
    assert_eq!(out["count"], 3);
    assert_eq!(out["complete"], true);
    assert_eq!(out["language"], "jq");
    let app = rig.app();
    assert_eq!(app.query_text, ".mine");
    assert!(app.results.as_array().unwrap().is_empty());
    assert!(!app.query_running);
}

#[test]
fn run_query_gives_a_page_and_counts_them_all() {
    let rig = Rig::new();
    rig.open(&json!((0..100).collect::<Vec<_>>()).to_string());
    let out = rig.call(run(".[]", 10, 5)).unwrap();
    assert_eq!(out["items"], json!([10, 11, 12, 13, 14]));
    assert_eq!(out["count"], 100);
    assert_eq!(out["next_offset"], 15);
    let path = rig.call(run("$[2]", 0, 5)).unwrap();
    assert_eq!(path["language"], "jsonpath");
    assert_eq!(path["items"], json!([2]));
}

#[test]
fn what_cannot_be_run_is_the_tools_error() {
    let rig = Rig::new();
    let error = rig.call(run(".", 0, 5)).unwrap_err();
    assert!(error.contains("no document is open"), "{error}");
    rig.open("[1]");
    assert!(rig.call(run(".[", 0, 5)).is_err());
}

#[test]
fn run_query_on_a_file_leaves_the_open_document_alone() {
    let dir = ScratchDir::new("mcp", "run-file");
    let file = write(&dir, "data.json", r#"{"n": 5}"#);
    let rig = Rig::new();
    rig.open("[1]");
    let call = |path: PathBuf| Call::RunQuery {
        query: ".n".to_owned(),
        language: None,
        input: Input::File(path),
        offset: 0,
        limit: 5,
        max_bytes: 24_000,
        timeout: WAIT,
    };
    let out = rig.call(call(file.clone())).unwrap();
    assert_eq!(out["items"], json!([5]));
    // Again, from what was loaded the first time.
    assert_eq!(rig.call(call(file)).unwrap()["items"], json!([5]));
    assert!(matches!(
        rig.app().doc.as_ref().unwrap().source,
        DocumentSource::Pasted
    ));
    let error = rig.call(call(PathBuf::from("data.json"))).unwrap_err();
    assert!(error.contains("not a full path"), "{error}");
    let error = rig
        .call(call(PathBuf::from("https://example.com/a.json")))
        .unwrap_err();
    assert!(error.contains("web addresses"), "{error}");
}

#[test]
fn set_query_runs_it_in_the_app_with_no_frame_drawn() {
    let rig = Rig::new();
    rig.open("[1, 2, 3]");
    let out = rig.call(set(".[] | select(. > 1)", None)).unwrap();
    assert_eq!(out["items"], json!([2, 3]));
    assert_eq!(out["count"], 2);
    {
        let app = rig.app();
        assert_eq!(app.query_text, ".[] | select(. > 1)");
        assert_eq!(app.results, json!([2, 3]));
        assert!(!app.query_running);
        assert_eq!(app.query_engine, None);
    }
    rig.call(set("$[0]", Some(Kind::JsonPath))).unwrap();
    assert_eq!(rig.app().query_engine, Some(Kind::JsonPath));
    let error = rig.call(set(".[", None)).unwrap_err();
    assert!(error.contains("query box"), "{error}");
    assert_eq!(rig.app().query_text, ".[");
}

#[test]
fn get_results_pages_what_the_user_sees() {
    let rig = Rig::new();
    rig.open(&json!((0..30).collect::<Vec<_>>()).to_string());
    rig.call(set(".[]", None)).unwrap();
    let page = rig
        .call(Call::Results {
            offset: 25,
            limit: 10,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(page["items"], json!([25, 26, 27, 28, 29]));
    assert_eq!(page["count"], 30);
    assert_eq!(page["query"], ".[]");
    assert!(page.get("next_offset").is_none());
}

#[test]
fn open_document_waits_until_it_has_loaded() {
    let dir = ScratchDir::new("mcp", "open");
    let rig = Rig::new();
    let out = rig
        .call(Call::OpenDocument {
            path: None,
            text: Some("[1, 2]".to_owned()),
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(out["opened"], true);
    assert_eq!(out["document"]["top"], "array with 2 items");
    let file = write(&dir, "a.json", r#"{"x": 1}"#);
    let out = rig
        .call(Call::OpenDocument {
            path: Some(file.clone()),
            text: None,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(out["document"]["source"], "file");
    assert_eq!(out["document"]["from"], file.display().to_string());
    assert_eq!(rig.app().source_input, file.display().to_string());
    let bad = rig.call(Call::OpenDocument {
        path: None,
        text: Some("{nope".to_owned()),
        timeout: WAIT,
    });
    assert!(bad.is_err());
    let missing = rig.call(Call::OpenDocument {
        path: Some(dir.join("missing.json")),
        text: None,
        timeout: WAIT,
    });
    assert!(missing.is_err());
}

#[test]
fn describe_read_and_search_look_at_the_source_or_the_results() {
    let rig = Rig::new();
    rig.open(r#"{"users": [{"name": "Ann", "age": 30}, {"name": "Bo", "age": 41}]}"#);
    let shape = rig
        .call(Call::Describe {
            input: Input::Document,
            pointer: "/users".to_owned(),
            depth: 4,
            sample: 100,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(shape["pointer"], "/users");
    assert_eq!(shape["length"], 2);
    assert_eq!(shape["items"]["keys"]["age"]["range"], json!([30, 41]));
    let value = rig.call(read(Input::Document, "/users/1/name")).unwrap();
    assert_eq!(value["value"], "Bo");
    let error = rig.call(read(Input::Document, "/users/5")).unwrap_err();
    assert!(error.contains("nothing at"), "{error}");
    let found = rig
        .call(Call::Search {
            input: Input::Document,
            text: "bo".to_owned(),
            regex: false,
            offset: 0,
            limit: 10,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(found["count"], 1);
    assert_eq!(found["matches"][0]["pointer"], "/users/1/name");
    assert_eq!(found["matches"][0]["key"], "name");
    // As JSON writes it: a string with its quotes, so that "1" is not taken for 1.
    assert_eq!(found["matches"][0]["value"], "\"Bo\"");
    rig.call(set(".users[].name", None)).unwrap();
    let first = rig.call(read(Input::Results, "/0")).unwrap();
    assert_eq!(first["value"], "Ann");
    let found = rig
        .call(Call::Search {
            input: Input::Results,
            text: "^A".to_owned(),
            regex: true,
            offset: 0,
            limit: 10,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(found["matches"][0]["pointer"], "/0");
}

#[test]
fn reveal_shows_a_value_that_is_there_and_says_when_it_is_not() {
    let rig = Rig::new();
    rig.open(r#"{"a": [1, {"b": 2}]}"#);
    let out = rig
        .call(Call::Reveal {
            target: Target::Source,
            pointer: "/a/1".to_owned(),
        })
        .unwrap();
    assert_eq!(out["revealed"], "/a/1");
    assert_eq!(out["value"], "object with 1 member");
    assert!(rig
        .call(Call::Reveal {
            target: Target::Source,
            pointer: "/z".to_owned(),
        })
        .is_err());
}

#[test]
fn the_workspace_is_listed_read_and_opened() {
    let dir = ScratchDir::new("mcp", "workspace");
    write(&dir, "people.jq", ".[] | .name");
    write(&dir, "people.sample.json", r#"[{"name": "Ann"}]"#);
    write(&dir, "shapes/list.schema.json", r#"{"type": "array"}"#);
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let all = rig
        .call(Call::WorkspaceList {
            folder: None,
            offset: 0,
            max_bytes: 24_000,
        })
        .unwrap();
    let items = all["items"].as_array().unwrap();
    let query = items.iter().find(|i| i["path"] == "people.jq").unwrap();
    assert_eq!(query["kind"], "query");
    assert_eq!(query["language"], "jq");
    assert!(query["sample_bytes"].as_u64().unwrap() > 0);
    assert_eq!(all["folders"], json!(["shapes"]));
    let shapes = rig
        .call(Call::WorkspaceList {
            folder: Some("shapes".to_owned()),
            offset: 0,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(shapes["items"].as_array().unwrap().len(), 1);
    assert_eq!(shapes["items"][0]["kind"], "schema");
    let read = rig
        .call(Call::WorkspaceRead {
            path: "people.jq".to_owned(),
            sample: true,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(read["text"], ".[] | .name");
    assert_eq!(read["sample"], json!([{"name": "Ann"}]));
    let schema = rig
        .call(Call::WorkspaceRead {
            path: "shapes/list.schema.json".to_owned(),
            sample: false,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(schema["content"], json!({"type": "array"}));
    assert!(rig
        .call(Call::WorkspaceRead {
            path: "nope.jq".to_owned(),
            sample: false,
            max_bytes: 24_000,
        })
        .is_err());
    let opened = rig
        .call(Call::WorkspaceOpen {
            path: "people.jq".to_owned(),
            sample: true,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(opened["opened"], "people.jq");
    assert_eq!(opened["query"]["text"], ".[] | .name");
    assert_eq!(opened["query"]["results"]["count"], 1);
    assert_eq!(rig.app().results, json!(["Ann"]));
}

#[test]
fn a_web_address_of_the_workspace_is_read_but_never_loaded() {
    let dir = ScratchDir::new("mcp", "workspace-url");
    write(
        &dir,
        "feed.url",
        "[InternetShortcut]\nURL=https://example.com/feed.json\n",
    );
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let read = rig
        .call(Call::WorkspaceRead {
            path: "feed.url".to_owned(),
            sample: false,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(read["kind"], "web address");
    assert_eq!(read["url"], "https://example.com/feed.json");
    let error = rig
        .call(Call::WorkspaceOpen {
            path: "feed.url".to_owned(),
            sample: true,
            timeout: WAIT,
        })
        .unwrap_err();
    assert!(error.contains("does not load addresses"), "{error}");
    let app = rig.app();
    assert!(app.doc.is_none() && !app.loading);
    assert_eq!(app.source_input, "");
}

#[test]
fn without_a_workspace_folder_the_list_is_empty_and_says_why() {
    let rig = Rig::new();
    let out = rig
        .call(Call::WorkspaceList {
            folder: None,
            offset: 0,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(out["items"], json!([]));
    assert!(out["note"].as_str().unwrap().contains("no workspace"));
}

#[test]
fn a_closed_app_is_said_to_be_closing() {
    let rig = Rig::new();
    let Rig {
        app, host, cancel, ..
    } = rig;
    drop(app);
    let error = host.call(Call::State, &cancel).unwrap_err();
    assert!(error.contains("closing"), "{error}");
}

/// One request to the server, the whole of its answer back.
fn post(port: u16, token: &str, body: &str) -> Value {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.set_read_timeout(Some(WAIT)).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    serde_json::from_str(answer.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

#[test]
fn the_panel_starts_a_server_that_answers_with_what_the_app_has() {
    let rig = Rig::new();
    rig.open(r#"[{"id": 7}]"#);
    let (port, token) = {
        let mut app = rig.app();
        app.mcp.settings.port = 0;
        let limits = app.settings.limits;
        app.mcp.start(&rig.ctx, limits);
        let server = app.mcp.server.as_ref().expect("started");
        (server.port(), app.mcp.settings.token.clone())
    };
    let answer = post(
        port,
        &token,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_query","arguments":{"query":".[].id"}}}"#,
    );
    let text = answer["result"]["content"][0]["text"].as_str().unwrap();
    let out: Value = serde_json::from_str(text).unwrap();
    assert_eq!(out["items"], json!([7]));
    // What needs more than the user allows is refused, and allowed once it is.
    let set = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"set_query","arguments":{"query":".[0]"}}}"#;
    let refused = post(port, &token, set);
    assert_eq!(refused["result"]["isError"], true);
    {
        let mut app = rig.app();
        app.mcp.settings.access = Access::Edit;
        app.mcp.server.as_ref().unwrap().set_access(Access::Edit);
    }
    let allowed = post(port, &token, set);
    assert_eq!(allowed["result"]["isError"], false);
    assert_eq!(rig.app().query_text, ".[0]");
    rig.app().mcp.stop();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn a_port_that_is_taken_is_said_to_be() {
    let rig = Rig::new();
    let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut app = rig.app();
    app.mcp.settings.port = port;
    let limits = app.settings.limits;
    app.mcp.start(&rig.ctx, limits);
    assert!(app.mcp.server.is_none());
    let error = app.mcp.error.clone().unwrap();
    assert!(error.contains(&format!("Port {port} is taken")), "{error}");
}

#[test]
fn the_port_box_is_taken_whole_when_it_is_left() {
    let rig = Rig::new();
    let mut app = rig.app();
    let before = app.mcp.settings.port;
    // Typed one key at a time, "70000" goes through 7, 70, 700 and 7000, which are
    // ports too: none of them is taken, nor is what is not a port at all.
    for typed in ["70000", "abc", "0", "-1", ""] {
        app.mcp.port_text = typed.to_owned();
        app.mcp.take_port();
        assert_eq!(app.mcp.settings.port, before, "{typed:?}");
        assert_eq!(app.mcp.port_text, before.to_string(), "{typed:?}");
    }
    app.mcp.port_text = " 8123 ".to_owned();
    app.mcp.take_port();
    assert_eq!(app.mcp.settings.port, 8123);
    assert_eq!(app.mcp.port_text, "8123");
}

#[test]
fn the_configurations_carry_the_address_and_the_token() {
    let rig = Rig::new();
    let app = rig.app();
    let token = app.mcp.settings.token.clone();
    let url = format!("http://127.0.0.1:{}/mcp", jsonquery_mcp::DEFAULT_PORT);
    assert_eq!(
        app.mcp.configuration("Claude Code", None),
        format!(
            "claude mcp add --transport http jsonquery {url} --header \"Authorization: Bearer {token}\""
        )
    );
    let vscode: Value = serde_json::from_str(&app.mcp.configuration("VS Code", None)).unwrap();
    assert_eq!(vscode["servers"]["jsonquery"]["type"], "http");
    assert_eq!(vscode["servers"]["jsonquery"]["url"], url);
    let other: Value = serde_json::from_str(&app.mcp.configuration("JSON", None)).unwrap();
    assert_eq!(
        other["mcpServers"]["jsonquery"]["headers"]["Authorization"],
        format!("Bearer {token}")
    );
}

#[test]
fn what_starts_the_bridge_names_it_and_holds_no_token() {
    let rig = Rig::new();
    let app = rig.app();
    let program = PathBuf::from(if cfg!(windows) {
        r"C:\Program Files\JsonQuery MCP\jsonquery-mcp.exe"
    } else {
        "/opt/json query/jsonquery-mcp"
    });
    let bridge = bridge::Bridge {
        command: program.clone(),
        args: Vec::new(),
    };
    // Claude Desktop's entry, which QuickAI takes as it is (Paste the JSON of a server's
    // instructions): the program, no token.
    for name in ["Claude Desktop", "QuickAI"] {
        let text = app.mcp.configuration(name, Some(&bridge));
        assert!(!text.contains(&app.mcp.settings.token), "{name}: {text}");
        let entry: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            entry,
            json!({"mcpServers": {"jsonquery": {"command": program.display().to_string(), "args": []}}}),
            "{name}"
        );
    }
}

/// An AppImage starts the program inside it when given `mcp` first: the entries that
/// start the program give it.
#[test]
fn what_starts_the_bridge_of_an_appimage_gives_it_mcp() {
    let rig = Rig::new();
    let app = rig.app();
    let image = "/home/u/Apps/jsonquery_gui-1.0.0-x86_64.AppImage";
    let bridge = bridge::Bridge {
        command: image.into(),
        args: vec!["mcp".to_owned()],
    };
    for name in ["Claude Desktop", "QuickAI"] {
        let entry: Value =
            serde_json::from_str(&app.mcp.configuration(name, Some(&bridge))).unwrap();
        assert_eq!(
            entry["mcpServers"]["jsonquery"],
            json!({"command": image, "args": ["mcp"]}),
            "{name}"
        );
    }
}

// ---- phase 2: the tools of the Tools, and what changes what is kept --------------

use jsonquery_mcp::{Input, SaveKind, ToolPage};
use jsonquery_query::reformat::{self, Indent};

fn json_input(value: Value) -> Input {
    Input::Json(value)
}

/// The user's answer to the next question an assistant asks, given from another thread
/// as soon as it is asked: what it was about.
fn answer_next(app: &Arc<Mutex<App>>, allow: bool) -> std::thread::JoinHandle<Option<String>> {
    let app = app.clone();
    std::thread::spawn(move || {
        let until = std::time::Instant::now() + WAIT;
        while std::time::Instant::now() < until {
            if let Some(what) = lock(&app).mcp.asks.answer_first(allow) {
                return Some(what);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    })
}

#[test]
fn format_json_takes_the_document_the_results_or_what_is_given() {
    let rig = Rig::new();
    rig.open(r#"{"b": [1, 2], "a": "é"}"#);
    let options = reformat::Options {
        indent: Indent::Spaces(4),
        sort_keys: true,
        ascii_only: true,
    };
    let out = rig
        .call(Call::Format {
            input: Input::Document,
            options,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(
        out["text"],
        "{\n    \"a\": \"\\u00e9\",\n    \"b\": [\n        1,\n        2\n    ]\n}"
    );
    assert_eq!(out["complete"], true);
    rig.call(set(".b", None)).unwrap();
    let minified = reformat::Options {
        indent: Indent::Minified,
        ..reformat::Options::default()
    };
    let results = rig
        .call(Call::Format {
            input: Input::Results,
            options: minified,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(results["text"], "[[1,2]]");
    // Cut at max_bytes, and said so.
    let long = json_input(json!({"many": vec![12345; 600]}));
    let cut = rig
        .call(Call::Format {
            input: long,
            options: reformat::Options::default(),
            max_bytes: 1_000,
        })
        .unwrap();
    assert_eq!(cut["complete"], false);
    assert!(cut["text"].as_str().unwrap().len() <= 1_000);
    let error = rig
        .call(Call::Format {
            input: Input::Text("{oops".to_owned()),
            options: reformat::Options::default(),
            max_bytes: 24_000,
        })
        .unwrap_err();
    assert!(!error.is_empty());
}

#[test]
fn diff_documents_lists_the_differences_and_the_patch() {
    let rig = Rig::new();
    let out = rig
        .call(Call::Diff {
            left: json_input(json!({"a": 1, "b": 2})),
            right: json_input(json!({"a": 1, "b": 3, "c": 4})),
            offset: 0,
            limit: 20,
            as_patch: true,
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(out["same"], false);
    assert_eq!(
        (out["added"].clone(), out["changed"].clone()),
        (json!(1), json!(1))
    );
    let changes = out["changes"].as_array().unwrap();
    assert!(changes
        .iter()
        .any(|c| c["kind"] == "changed" && c["pointer"] == "/b"));
    assert!(changes
        .iter()
        .any(|c| c["kind"] == "added" && c["pointer"] == "/c"));
    let patch = out["patch"].as_array().unwrap();
    assert!(patch
        .iter()
        .any(|op| op["op"] == "replace" && op["path"] == "/b"));
    assert!(patch
        .iter()
        .any(|op| op["op"] == "add" && op["path"] == "/c"));
    let same = rig
        .call(Call::Diff {
            left: json_input(json!([1])),
            right: Input::Text("[1]".to_owned()),
            offset: 0,
            limit: 20,
            as_patch: false,
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(same["same"], true);
    assert!(same.get("patch").is_none());
}

#[test]
fn apply_patch_takes_a_json_patch_or_a_merge_patch() {
    let rig = Rig::new();
    rig.open(r#"{"a": 1, "b": {"c": 2}}"#);
    let patched = |patch: Value, merge_patch: Option<bool>| {
        rig.call(Call::Patch {
            input: Input::Document,
            patch: json_input(patch),
            merge_patch,
            max_bytes: 24_000,
        })
    };
    let out = patched(json!([{"op": "replace", "path": "/a", "value": 9}]), None).unwrap();
    assert_eq!(out["kind"], "JSON Patch");
    assert_eq!(out["result"], json!({"a": 9, "b": {"c": 2}}));
    let out = patched(json!({"b": {"c": null, "d": 3}}), None).unwrap();
    assert_eq!(out["kind"], "JSON Merge Patch");
    assert_eq!(out["result"], json!({"a": 1, "b": {"d": 3}}));
    // What the patch cannot do is said.
    assert!(patched(json!([{"op": "remove", "path": "/zz"}]), Some(false)).is_err());
    // The open document is as it was.
    assert_eq!(rig.call(run(".a", 0, 20)).unwrap()["items"], json!([1]));
}

#[test]
fn merge_documents_appends_or_merges_deep() {
    let rig = Rig::new();
    rig.open("[1, 2]");
    let merged = |inputs: Vec<Input>, filter: &str| {
        rig.call(Call::Merge {
            inputs,
            filter: filter.to_owned(),
            max_bytes: 24_000,
            timeout: WAIT,
        })
    };
    let out = merged(vec![Input::Document, json_input(json!([3]))], "add").unwrap();
    assert_eq!(out["result"], json!([1, 2, 3]));
    let deep = jsonquery_query::merge::PRESETS[2].filter;
    let out = merged(
        vec![
            json_input(json!({"a": {"x": 1}})),
            Input::Text(r#"{"a": {"y": 2}}"#.to_owned()),
        ],
        deep,
    )
    .unwrap();
    assert_eq!(out["result"], json!({"a": {"x": 1, "y": 2}}));
    assert!(merged(vec![json_input(json!([1])), json_input(json!({}))], "add").is_err());
}

#[test]
fn validate_json_says_what_is_wrong_and_where() {
    let dir = ScratchDir::new("mcp", "validate");
    write(
        &dir,
        "person.schema.json",
        r#"{"type": "object", "required": ["name"], "properties": {"age": {"type": "integer"}}}"#,
    );
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let checked = |document: Value| {
        rig.call(Call::Validate {
            input: json_input(document),
            schema: Input::Workspace("person.schema.json".to_owned()),
            check_formats: false,
            offset: 0,
            limit: 20,
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap()
    };
    let good = checked(json!({"name": "Ann", "age": 30}));
    assert_eq!(good["valid"], true);
    assert_eq!(good["problems"], json!([]));
    let bad = checked(json!({"age": "old"}));
    assert_eq!(bad["valid"], false);
    let problems = bad["problems"].as_array().unwrap();
    assert!(problems.iter().any(|p| p["keyword"] == "required"));
    assert!(problems
        .iter()
        .any(|p| p["keyword"] == "type" && p["pointer"] == "/age"));
    // A query is not JSON to check against.
    write(&dir, "q.jq", ".");
    rig.app().workspace_pane.workspace().unwrap().rescan();
    let error = rig
        .call(Call::Validate {
            input: json_input(json!({})),
            schema: Input::Workspace("q.jq".to_owned()),
            check_formats: false,
            offset: 0,
            limit: 20,
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap_err();
    assert!(error.contains("not JSON"), "{error}");
}

#[test]
fn create_schema_describes_the_document() {
    let rig = Rig::new();
    rig.open(r#"[{"id": 1, "tag": "x"}, {"id": 2, "tag": "y"}]"#);
    let out = rig
        .call(Call::CreateSchema {
            input: Input::Document,
            options: jsonquery_query::infer::Options::default(),
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(out["draft"], "Draft 2020-12");
    let schema = &out["schema"];
    assert_eq!(schema["type"], "array");
    assert_eq!(schema["items"]["properties"]["id"]["type"], "integer");
    assert_eq!(schema["items"]["required"], json!(["id", "tag"]));
}

#[test]
fn show_in_tool_fills_the_page_and_shows_it() {
    let rig = Rig::new();
    rig.open(r#"{"name": "Ann"}"#);
    let out = rig
        .call(Call::ShowInTool {
            page: ToolPage::Validate {
                check_formats: false,
            },
            inputs: vec![Input::Document, json_input(json!({"type": "object"}))],
            run: false,
        })
        .unwrap();
    assert_eq!(out["shown"], "Validate Schema");
    let mut app = rig.app();
    assert_eq!(app.tools.current_tool(), crate::tools::Tool::Validate);
    assert!(
        app.tools.in_window() || app.tools.is_docked(),
        "the page is shown"
    );
    let document: Value =
        serde_json::from_str(&app.tools.box_text(crate::tools::Tool::Validate, "Document"))
            .unwrap();
    assert_eq!(document, json!({"name": "Ann"}));
    let schema: Value =
        serde_json::from_str(&app.tools.box_text(crate::tools::Tool::Validate, "Schema")).unwrap();
    assert_eq!(schema, json!({"type": "object"}));
    drop(app);
    // A box takes what a box can show.
    let big = json_input(json!(vec!["x".repeat(1000); 1200]));
    let error = rig
        .call(Call::ShowInTool {
            page: ToolPage::Diff,
            inputs: vec![big, json_input(json!([]))],
            run: false,
        })
        .unwrap_err();
    assert!(error.contains("more than a box"), "{error}");
}

#[test]
fn workspace_save_makes_items_and_asks_before_replacing_one() {
    let dir = ScratchDir::new("mcp", "save");
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    rig.open(r#"[{"a": 1}]"#);
    let save = |content: &str, replace: bool| {
        rig.call(Call::WorkspaceSave {
            folder: "orders".to_owned(),
            name: "late".to_owned(),
            kind: SaveKind::Query(Kind::Jq),
            content: Input::Text(content.to_owned()),
            sample: Some(Input::Document),
            replace,
        })
    };
    let out = save(".[0].a", false).unwrap();
    assert_eq!(out["saved"], "orders/late.jq");
    assert_eq!(out["sample"], true);
    assert_eq!(
        std::fs::read_to_string(dir.join("orders/late.jq")).unwrap(),
        ".[0].a"
    );
    let sample: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("orders/late.sample.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(sample, json!([{"a": 1}]));
    // There is one of that name: not without `replace`, and not without the user.
    assert!(save(".x", false).unwrap_err().contains("already"));
    let said_no = answer_next(&rig.app, false);
    assert!(save(".x", true).unwrap_err().contains("did not allow"));
    assert!(said_no.join().unwrap().unwrap().contains("orders/late.jq"));
    assert_eq!(
        std::fs::read_to_string(dir.join("orders/late.jq")).unwrap(),
        ".[0].a"
    );
    let said_yes = answer_next(&rig.app, true);
    assert_eq!(save(".x", true).unwrap()["replaced"], true);
    said_yes.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("orders/late.jq")).unwrap(),
        ".x"
    );
    // A web address is kept as Windows keeps one; and it must be one.
    let address = |content: &str| {
        rig.call(Call::WorkspaceSave {
            folder: String::new(),
            name: "site".to_owned(),
            kind: SaveKind::Address,
            content: Input::Text(content.to_owned()),
            sample: None,
            replace: false,
        })
    };
    assert!(address("file:///etc/passwd").is_err());
    assert_eq!(
        address("https://example.com/a.json").unwrap()["saved"],
        "site.url"
    );
    assert!(std::fs::read_to_string(dir.join("site.url"))
        .unwrap()
        .contains("URL=https://example.com/a.json"));
}

#[test]
fn folders_are_made_items_moved_and_deleted_only_when_allowed() {
    let dir = ScratchDir::new("mcp", "keep");
    write(&dir, "q.jq", ".a");
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let made = rig
        .call(Call::WorkspaceNewFolder {
            path: "reports/2026".to_owned(),
        })
        .unwrap();
    assert_eq!(made["made"], json!(["reports", "reports/2026"]));
    assert!(dir.join("reports/2026").is_dir());
    let moved = rig
        .call(Call::WorkspaceMove {
            path: "q.jq".to_owned(),
            to: Some("reports/2026".to_owned()),
            name: Some("a-only".to_owned()),
        })
        .unwrap();
    assert_eq!(moved["now"], "reports/2026/a-only.jq");
    assert!(dir.join("reports/2026/a-only.jq").is_file());
    assert!(rig
        .call(Call::WorkspaceMove {
            path: "nothing.jq".to_owned(),
            to: Some(String::new()),
            name: None,
        })
        .is_err());
    let delete = || {
        rig.call(Call::WorkspaceDelete {
            path: "reports".to_owned(),
        })
    };
    let said_no = answer_next(&rig.app, false);
    assert!(delete().unwrap_err().contains("did not allow"));
    let asked = said_no.join().unwrap().unwrap();
    assert!(
        asked.contains("Delete from the workspace") && asked.contains("reports"),
        "{asked}"
    );
    assert!(dir.join("reports/2026/a-only.jq").is_file());
    let said_yes = answer_next(&rig.app, true);
    assert_eq!(delete().unwrap()["items"], 1);
    said_yes.join().unwrap();
    assert!(!dir.join("reports").exists());
}

#[test]
fn save_file_asks_first_and_replaces_only_when_told() {
    let dir = ScratchDir::new("mcp", "save-file");
    let rig = Rig::new();
    rig.open(r#"{"k": [1, 2]}"#);
    let path = dir.join("out/k.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let save = |replace: bool| {
        rig.call(Call::SaveFile {
            input: Input::Document,
            path: path.clone(),
            indent: Indent::Minified,
            replace,
        })
    };
    let said_no = answer_next(&rig.app, false);
    assert!(save(false).unwrap_err().contains("did not allow"));
    said_no.join().unwrap();
    assert!(!path.exists());
    let said_yes = answer_next(&rig.app, true);
    assert_eq!(save(false).unwrap()["replaced"], false);
    said_yes.join().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), r#"{"k":[1,2]}"#);
    // There is a file there now.
    assert!(save(false).unwrap_err().contains("there already"));
    assert!(rig
        .call(Call::SaveFile {
            input: Input::Results,
            path: PathBuf::from("relative.json"),
            indent: Indent::Spaces(2),
            replace: false,
        })
        .is_err());
}

#[test]
fn a_question_that_is_not_answered_is_taken_back() {
    let rig = Rig::new();
    let started = std::time::Instant::now();
    let error = rig
        .call(Call::OpenUrl {
            url: "http://127.0.0.1:9/never.json".to_owned(),
            timeout: WAIT,
        })
        .unwrap_err();
    assert!(error.contains("did not answer"), "{error}");
    assert!(started.elapsed() >= ask::ASK_TIME);
    assert_eq!(rig.app().mcp.asks.waiting(), 0, "the question is gone");
    // Nothing was loaded.
    assert!(rig.app().doc.is_none());
    assert!(!rig.app().loading);
}

// ---- out of sight, "last", metadata, pages, Always allow --------------------------

fn info(input: Input, pointer: &str) -> Call {
    Call::Info {
        input,
        pointer: pointer.to_owned(),
        max_bytes: 24_000,
    }
}

#[test]
fn document_info_says_what_a_document_is_without_its_data() {
    let dir = ScratchDir::new("mcp", "info");
    let rows: Vec<Value> = (0..200)
        .map(|i| json!({"id": i, "tags": ["a", "b"], "note": "x".repeat(40)}))
        .collect();
    let text = serde_json::to_string(&json!({"meta": {"v": 1}, "rows": rows})).unwrap();
    let file = write(&dir, "big.json", &text);
    let rig = Rig::new();
    rig.open("[1]");
    let out = rig.call(info(Input::File(file.clone()), "")).unwrap();
    assert_eq!(out["source"], "file");
    assert_eq!(out["file"], file.display().to_string());
    assert!(out["modified"].as_str().unwrap().ends_with('Z'), "{out}");
    assert_eq!(out["bytes"], text.len());
    assert_eq!(out["format"], "JSON");
    assert_eq!(out["is"], "object with 2 members");
    // The root, meta, v, rows, and each row: itself, id, tags, its two tags, note.
    assert_eq!(out["values"]["all"], 4 + 200 * 6);
    assert_eq!(out["values"]["strings"], 200 * 3);
    assert_eq!(out["depth"], 4);
    assert_eq!(
        out["longest_arrays"][0],
        json!({"pointer": "/rows", "items": 200})
    );
    assert_eq!(out["biggest_members"][0]["key"], "rows");
    assert_eq!(out["biggest_members"][0]["is"], "array with 200 items");
    assert!(out["to_read_whole"]["pages"].as_u64().unwrap() >= 1);
    assert_eq!(out["to_read_whole"]["page_bytes"], 24_000);
    // The window keeps its own document: the file was read out of sight.
    assert_eq!(rig.app().doc.as_ref().unwrap().byte_len, 3);
    // A part of it, by its pointer.
    let part = rig.call(info(Input::File(file), "/rows/0")).unwrap();
    assert_eq!(part["pointer"], "/rows/0");
    assert_eq!(part["is"], "object with 3 members");
    assert_eq!(part["bytes"], text.len(), "the document's own size stays");
    assert!(part["part_bytes"].as_u64().unwrap() < 100, "{part}");
    assert_eq!(part["values"]["all"], 6);
    // The open document, by default.
    let open = rig.call(info(Input::Document, "")).unwrap();
    assert_eq!(open["is"], "array with 1 item");
    assert_eq!(open["bytes_per_item"], 3);
}

#[test]
fn the_looking_tools_work_out_of_sight_on_any_document() {
    let dir = ScratchDir::new("mcp", "hidden");
    let file = write(
        &dir,
        "people.json",
        r#"{"people": [{"name": "Ann"}, {"name": "Bo"}]}"#,
    );
    let rig = Rig::new();
    rig.open(r#"{"mine": true}"#);
    let shape = rig
        .call(Call::Describe {
            input: Input::File(file.clone()),
            pointer: "/people".to_owned(),
            depth: 3,
            sample: 10,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(shape["length"], 2);
    let value = rig
        .call(read(Input::File(file.clone()), "/people/1/name"))
        .unwrap();
    assert_eq!(value["value"], "Bo");
    let found = rig
        .call(Call::Search {
            input: Input::File(file.clone()),
            text: "ann".to_owned(),
            regex: false,
            offset: 0,
            limit: 10,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(found["matches"][0]["pointer"], "/people/0/name");
    let ran = rig
        .call(Call::RunQuery {
            query: "[.people[].name]".to_owned(),
            language: None,
            input: Input::Json(json!({"people": [{"name": "Cy"}]})),
            offset: 0,
            limit: 5,
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(ran["items"], json!([["Cy"]]));
    // Nothing of it reached the window.
    let app = rig.app();
    assert!(app.doc.as_ref().unwrap().tree().unwrap()["mine"] == json!(true));
    assert!(app.query_text.is_empty(), "{}", app.query_text);
}

#[test]
fn what_run_query_and_the_tools_make_is_kept_as_last() {
    let rig = Rig::new();
    assert!(rig
        .call(read(Input::Last, ""))
        .unwrap_err()
        .contains("nothing is kept"));
    rig.open(r#"{"items": [{"n": 1}, {"n": 2}, {"n": 3}]}"#);
    let ran = rig.call(run(".items[] | select(.n > 1)", 0, 1)).unwrap();
    assert_eq!(ran["count"], 2);
    assert!(ran["last"].as_str().unwrap().starts_with("kept"), "{ran}");
    // All of the results are kept, not the page.
    let kept = rig.call(read(Input::Last, "")).unwrap();
    assert_eq!(kept["items"], json!([{"n": 2}, {"n": 3}]));
    let merged = rig
        .call(Call::Merge {
            inputs: vec![Input::Last, json_input(json!([{"n": 9}]))],
            filter: jsonquery_query::merge::PRESETS[0].filter.to_owned(),
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(merged["last"], "kept: give \"last\" as an input");
    let again = rig
        .call(run("length", 0, 1).with_input(Input::Last))
        .unwrap();
    assert_eq!(again["items"], json!([3]));
    // What run_query keeps then is its own results.
    let kept = rig.call(read(Input::Last, "")).unwrap();
    assert_eq!(kept["items"], json!([3]));
}

trait WithInput {
    fn with_input(self, input: Input) -> Self;
}

impl WithInput for Call {
    fn with_input(self, input: Input) -> Self {
        match self {
            Call::RunQuery {
                query,
                language,
                offset,
                limit,
                max_bytes,
                timeout,
                ..
            } => Call::RunQuery {
                query,
                language,
                input,
                offset,
                limit,
                max_bytes,
                timeout,
            },
            other => other,
        }
    }
}

#[test]
fn search_the_workspace_and_problems_come_a_page_at_a_time() {
    let dir = ScratchDir::new("mcp", "pages");
    for i in 0..30 {
        write(&dir, &format!("q{i:02}.jq"), ".a");
    }
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let rows: Vec<Value> = (0..40)
        .map(|i| json!({"name": format!("name {i}")}))
        .collect();
    rig.open(&Value::Array(rows).to_string());
    let page = |offset| {
        rig.call(Call::Search {
            input: Input::Document,
            text: "name".to_owned(),
            regex: false,
            offset,
            limit: 1_000,
            max_bytes: 1_000,
        })
        .unwrap()
    };
    let first = page(0);
    let shown = first["matches"].as_array().unwrap().len();
    assert!(shown > 0 && shown < 80, "{shown}");
    assert!(
        first.to_string().len() < 1_400,
        "{}",
        first.to_string().len()
    );
    let next = first["next_offset"].as_u64().unwrap() as usize;
    assert_eq!(next, shown);
    assert_eq!(page(next)["offset"], next);
    let list = |offset| {
        rig.call(Call::WorkspaceList {
            folder: None,
            offset,
            max_bytes: 1_000,
        })
        .unwrap()
    };
    let first = list(0);
    assert_eq!(first["count"], 30);
    let next = first["next_offset"].as_u64().unwrap() as usize;
    let second = list(next);
    assert_eq!(second["offset"], next);
    assert!(second.get("folders").is_none());
    assert_eq!(second["items"][0]["path"], format!("q{next:02}.jq"));
    let checked = rig
        .call(Call::Validate {
            input: Input::Document,
            schema: json_input(json!({"items": {"type": "number"}})),
            check_formats: false,
            offset: 5,
            limit: 3,
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(checked["count"], 40);
    assert_eq!(checked["problems"].as_array().unwrap().len(), 3);
    assert_eq!(checked["problems"][0]["pointer"], "/5");
    assert_eq!(checked["next_offset"], 8);
}

#[test]
fn always_allow_does_what_edit_allows_without_a_question() {
    let dir = ScratchDir::new("mcp", "always");
    write(&dir, "a.jq", ".a");
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    rig.app().mcp.settings.always_allow = true;
    let deleted = rig
        .call(Call::WorkspaceDelete {
            path: "a.jq".to_owned(),
        })
        .unwrap();
    assert_eq!(deleted["items"], 1);
    assert_eq!(rig.app().mcp.asks.waiting(), 0);
    assert!(!dir.join("a.jq").exists());
}

#[test]
fn the_icon_is_green_while_an_assistant_works_with_the_app_and_a_minute_after() {
    use std::time::{Duration, SystemTime};
    let now = SystemTime::now();
    let call = |ago: u64| jsonquery_mcp::Activity {
        at: now - Duration::from_secs(ago),
        tool: "get_state".to_owned(),
        ok: true,
        took: Duration::from_millis(5),
        error: None,
    };
    let mut status = jsonquery_mcp::Status::default();
    assert_eq!(
        working_left(&status, now),
        None,
        "on, and no call yet: grey"
    );
    status.running = 1;
    assert_eq!(
        working_left(&status, now),
        Some(WORKING_FOR),
        "a call is being answered"
    );
    status.running = 0;
    status.recent = vec![call(10), call(500)];
    assert_eq!(
        working_left(&status, now),
        Some(WORKING_FOR - Duration::from_secs(10)),
        "the last call ended 10 s ago"
    );
    status.recent = vec![call(61)];
    assert_eq!(
        working_left(&status, now),
        None,
        "a minute without a call: grey"
    );
}
