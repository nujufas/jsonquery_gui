//! The tools: what each is called and does, what it takes (as the JSON Schema the
//! assistant is given), what the user must allow for it, and how what it is asked
//! is read into a [`Call`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use jsonquery_query::infer::{self, Draft};
use jsonquery_query::reformat::{self, Indent};
use jsonquery_query::Kind;
use serde_json::{json, Map, Value};

use crate::{Access, DEFAULT_PAGE};

/// A query language: one of the app's four engines.
pub type Language = Kind;

/// Which tree a tool looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// The document that is open in the app.
    Source,
    /// The results of the query in the app's query box.
    Results,
}

impl Target {
    pub fn key(self) -> &'static str {
        match self {
            Target::Source => "source",
            Target::Results => "results",
        }
    }
}

/// A JSON value a tool works on, as the call names it.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The document open in the app.
    Document,
    /// The results the user sees.
    Results,
    /// The JSON itself, in the call.
    Json(Value),
    /// JSON text, in the call (or, where the tool says so, plain text: a query, an
    /// address).
    Text(String),
    /// A JSON file, by its full path: reading one needs [`Access::Edit`].
    File(PathBuf),
    /// An item of the workspace (its path, as `workspace_list` gives it).
    Workspace(String),
    /// What the assistant's last `run_query`, or tool of the Tools, gave: kept out of
    /// sight by the app, for the next calls to read, query, check or save.
    Last,
}

impl Input {
    /// What reading it needs.
    pub fn needs(&self) -> Access {
        match self {
            Input::File(_) => Access::Edit,
            _ => Access::Read,
        }
    }

    /// How a call names it again, as its `input` (for the call an answer says reads
    /// on): none for the JSON, or its text, given in the call.
    pub fn argument(&self) -> Option<Value> {
        Some(match self {
            Input::Document => json!("document"),
            Input::Results => json!("results"),
            Input::Last => json!("last"),
            Input::File(path) => json!({"file": path.display().to_string()}),
            Input::Workspace(rel) => json!({ "workspace": rel }),
            Input::Json(_) | Input::Text(_) => return None,
        })
    }
}

/// A page of the Tools, with how it is set, for `show_in_tool`.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolPage {
    Merge {
        filter: String,
    },
    Format(reformat::Options),
    Diff,
    /// `merge_patch`: a JSON Merge Patch (RFC 7386) rather than a JSON Patch (RFC
    /// 6902); none: what the patch looks like says.
    Patch {
        merge_patch: Option<bool>,
    },
    Validate {
        check_formats: bool,
    },
    CreateSchema(infer::Options),
}

impl ToolPage {
    /// How many inputs the page takes, in the order of its boxes (Merge: any number
    /// of files, at least two).
    pub fn boxes(&self) -> &'static [&'static str] {
        match self {
            ToolPage::Merge { .. } => &[],
            ToolPage::Format(_) | ToolPage::CreateSchema(_) => &["document"],
            ToolPage::Diff => &["left", "right"],
            ToolPage::Patch { .. } => &["document", "patch"],
            ToolPage::Validate { .. } => &["document", "schema"],
        }
    }
}

/// What kind of item `workspace_save` makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveKind {
    Query(Language),
    Schema,
    Patch,
    Document,
    /// A web address.
    Address,
}

impl SaveKind {
    /// Its content is text (a query, an address), not JSON.
    pub fn is_text(self) -> bool {
        matches!(self, SaveKind::Query(_) | SaveKind::Address)
    }
}

/// What a tool was asked to do, read and checked.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    /// `get_state`
    State,
    /// `document_info`: about the value at `pointer` of `input`.
    Info {
        input: Input,
        pointer: String,
        max_bytes: usize,
    },
    /// `describe`
    Describe {
        input: Input,
        pointer: String,
        depth: usize,
        sample: usize,
        max_bytes: usize,
    },
    /// `read_value`
    ReadValue {
        input: Input,
        pointer: String,
        offset: usize,
        limit: usize,
        max_bytes: usize,
    },
    /// `search`
    Search {
        input: Input,
        text: String,
        regex: bool,
        offset: usize,
        limit: usize,
        max_bytes: usize,
    },
    /// `run_query`: out of sight, on `input`.
    RunQuery {
        query: String,
        language: Option<Language>,
        input: Input,
        offset: usize,
        limit: usize,
        max_bytes: usize,
        timeout: Duration,
    },
    /// `get_results`
    Results {
        offset: usize,
        limit: usize,
        max_bytes: usize,
    },
    /// `open_document`: one of `path` and `text`.
    OpenDocument {
        path: Option<PathBuf>,
        text: Option<String>,
        timeout: Duration,
    },
    /// `set_query`
    SetQuery {
        query: String,
        language: Option<Language>,
        run: bool,
        limit: usize,
        max_bytes: usize,
        timeout: Duration,
    },
    /// `reveal`
    Reveal { target: Target, pointer: String },
    /// `workspace_list`
    WorkspaceList {
        folder: Option<String>,
        offset: usize,
        max_bytes: usize,
    },
    /// `workspace_read`
    WorkspaceRead {
        path: String,
        sample: bool,
        max_bytes: usize,
    },
    /// `workspace_open`
    WorkspaceOpen {
        path: String,
        sample: bool,
        timeout: Duration,
    },
    /// `query_help`: answered here, without the app.
    Help {
        language: Option<Language>,
        topic: Option<String>,
        offset: usize,
        max_bytes: usize,
    },
    /// `format_json`
    Format {
        input: Input,
        options: reformat::Options,
        max_bytes: usize,
    },
    /// `diff_documents`
    Diff {
        left: Input,
        right: Input,
        offset: usize,
        limit: usize,
        as_patch: bool,
        max_bytes: usize,
        timeout: Duration,
    },
    /// `apply_patch`
    Patch {
        input: Input,
        patch: Input,
        merge_patch: Option<bool>,
        max_bytes: usize,
    },
    /// `merge_documents`: the jq `filter` runs over the documents as one array.
    Merge {
        inputs: Vec<Input>,
        filter: String,
        max_bytes: usize,
        timeout: Duration,
    },
    /// `validate_json`
    Validate {
        input: Input,
        schema: Input,
        check_formats: bool,
        offset: usize,
        limit: usize,
        max_bytes: usize,
        timeout: Duration,
    },
    /// `create_schema`
    CreateSchema {
        input: Input,
        options: infer::Options,
        max_bytes: usize,
        timeout: Duration,
    },
    /// `show_in_tool`
    ShowInTool {
        page: ToolPage,
        inputs: Vec<Input>,
        run: bool,
    },
    /// `workspace_save`: `folder` and `name` as the user would type them.
    WorkspaceSave {
        folder: String,
        name: String,
        kind: SaveKind,
        content: Input,
        sample: Option<Input>,
        replace: bool,
    },
    /// `workspace_new_folder`
    WorkspaceNewFolder { path: String },
    /// `workspace_move`: to the folder `to` (`""`: the top), named `name` (none: as
    /// it is).
    WorkspaceMove {
        path: String,
        to: Option<String>,
        name: Option<String>,
    },
    /// `workspace_delete`
    WorkspaceDelete { path: String },
    /// `save_file`
    SaveFile {
        input: Input,
        path: PathBuf,
        indent: Indent,
        replace: bool,
    },
    /// `open_url`
    OpenUrl { url: String, timeout: Duration },
}

impl Call {
    /// The name of the tool that was called.
    pub fn tool(&self) -> &'static str {
        match self {
            Call::State => "get_state",
            Call::Info { .. } => "document_info",
            Call::Describe { .. } => "describe",
            Call::ReadValue { .. } => "read_value",
            Call::Search { .. } => "search",
            Call::RunQuery { .. } => "run_query",
            Call::Results { .. } => "get_results",
            Call::OpenDocument { .. } => "open_document",
            Call::SetQuery { .. } => "set_query",
            Call::Reveal { .. } => "reveal",
            Call::WorkspaceList { .. } => "workspace_list",
            Call::WorkspaceRead { .. } => "workspace_read",
            Call::WorkspaceOpen { .. } => "workspace_open",
            Call::Help { .. } => "query_help",
            Call::Format { .. } => "format_json",
            Call::Diff { .. } => "diff_documents",
            Call::Patch { .. } => "apply_patch",
            Call::Merge { .. } => "merge_documents",
            Call::Validate { .. } => "validate_json",
            Call::CreateSchema { .. } => "create_schema",
            Call::ShowInTool { .. } => "show_in_tool",
            Call::WorkspaceSave { .. } => "workspace_save",
            Call::WorkspaceNewFolder { .. } => "workspace_new_folder",
            Call::WorkspaceMove { .. } => "workspace_move",
            Call::WorkspaceDelete { .. } => "workspace_delete",
            Call::SaveFile { .. } => "save_file",
            Call::OpenUrl { .. } => "open_url",
        }
    }

    /// What the user must have allowed for this call: changing what the app shows or
    /// what is kept, reading a file, or fetching a web address, needs [`Access::Edit`].
    pub fn needs(&self) -> Access {
        let most = |inputs: &mut dyn Iterator<Item = &Input>| {
            inputs.map(Input::needs).max().unwrap_or(Access::Read)
        };
        match self {
            Call::OpenDocument { .. }
            | Call::SetQuery { .. }
            | Call::Reveal { .. }
            | Call::WorkspaceOpen { .. }
            | Call::WorkspaceSave { .. }
            | Call::WorkspaceNewFolder { .. }
            | Call::WorkspaceMove { .. }
            | Call::WorkspaceDelete { .. }
            | Call::SaveFile { .. }
            | Call::OpenUrl { .. } => Access::Edit,
            Call::Info { input, .. }
            | Call::Describe { input, .. }
            | Call::ReadValue { input, .. }
            | Call::Search { input, .. }
            | Call::RunQuery { input, .. }
            | Call::Format { input, .. }
            | Call::CreateSchema { input, .. } => input.needs(),
            Call::Diff { left, right, .. } => most(&mut [left, right].into_iter()),
            Call::Patch { input, patch, .. } => most(&mut [input, patch].into_iter()),
            Call::Validate { input, schema, .. } => most(&mut [input, schema].into_iter()),
            Call::Merge { inputs, .. } => most(&mut inputs.iter()),
            Call::ShowInTool { .. } => Access::Edit,
            _ => Access::Read,
        }
    }

    /// Make the call's answer fit in a page of `page` bytes (the user's choice): what
    /// it may give of values, items and text is that, less room for the rest of the
    /// answer.
    pub fn fit(&mut self, page: usize) {
        let budget = budget_of(page);
        match self {
            Call::Info { max_bytes, .. }
            | Call::Describe { max_bytes, .. }
            | Call::ReadValue { max_bytes, .. }
            | Call::Search { max_bytes, .. }
            | Call::RunQuery { max_bytes, .. }
            | Call::Results { max_bytes, .. }
            | Call::SetQuery { max_bytes, .. }
            | Call::WorkspaceList { max_bytes, .. }
            | Call::WorkspaceRead { max_bytes, .. }
            | Call::Help { max_bytes, .. }
            | Call::Format { max_bytes, .. }
            | Call::Diff { max_bytes, .. }
            | Call::Patch { max_bytes, .. }
            | Call::Merge { max_bytes, .. }
            | Call::Validate { max_bytes, .. }
            | Call::CreateSchema { max_bytes, .. } => *max_bytes = budget,
            Call::State
            | Call::OpenDocument { .. }
            | Call::Reveal { .. }
            | Call::WorkspaceOpen { .. }
            | Call::ShowInTool { .. }
            | Call::WorkspaceSave { .. }
            | Call::WorkspaceNewFolder { .. }
            | Call::WorkspaceMove { .. }
            | Call::WorkspaceDelete { .. }
            | Call::SaveFile { .. }
            | Call::OpenUrl { .. } => {}
        }
    }
}

/// What a tool may give of values, items and text in a page of `page` bytes: the
/// rest of the page is room for what the answer says about them.
pub fn budget_of(page: usize) -> usize {
    (page - page / 8).max(MIN_BUDGET)
}

/// The least a tool is given to fill, however small the page.
const MIN_BUDGET: usize = 1_000;

/// The names of the tools, in the order they are listed: those that only read, then
/// those that need the user to allow Edit.
pub const TOOL_NAMES: [&str; 27] = [
    "get_state",
    "document_info",
    "describe",
    "read_value",
    "search",
    "run_query",
    "get_results",
    "query_help",
    "workspace_list",
    "workspace_read",
    "format_json",
    "diff_documents",
    "apply_patch",
    "merge_documents",
    "validate_json",
    "create_schema",
    "open_document",
    "set_query",
    "reveal",
    "workspace_open",
    "show_in_tool",
    "workspace_save",
    "workspace_new_folder",
    "workspace_move",
    "workspace_delete",
    "save_file",
    "open_url",
];

pub const DEFAULT_LIMIT: usize = 20;
pub const MAX_LIMIT: usize = 1000;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(600);
pub const DEFAULT_DEPTH: usize = 4;
pub const MAX_DEPTH: usize = 12;
pub const DEFAULT_SAMPLE: usize = 200;
pub const MAX_SAMPLE: usize = 5_000;
pub const DEFAULT_SEARCH_LIMIT: usize = 50;

/// What a call may fill until the server fits it to the user's page ([`Call::fit`]).
fn default_budget() -> usize {
    budget_of(DEFAULT_PAGE)
}

/// The key a language is given and taken by.
pub fn language_key(language: Language) -> &'static str {
    match language {
        Kind::Jq => "jq",
        Kind::JsonPath => "jsonpath",
        Kind::JmesPath => "jmespath",
        Kind::JsonPointer => "pointer",
    }
}

fn language_of(text: &str) -> Option<Language> {
    let key: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    match key.as_str() {
        "jq" | "jaq" => Some(Kind::Jq),
        "jsonpath" | "path" => Some(Kind::JsonPath),
        "jmespath" | "jmes" => Some(Kind::JmesPath),
        "pointer" | "jsonpointer" => Some(Kind::JsonPointer),
        _ => None,
    }
}

/// The `tools` of the answer to `tools/list`.
pub fn tool_list() -> Vec<Value> {
    TOOL_NAMES.iter().map(|name| definition(name)).collect()
}

// What each tool is told to be is what a model reads before it does anything, every
// time: a model run on the user's own computer has little room, so it is said short.
fn definition(name: &str) -> Value {
    let (title, description, properties, required, read_only): (_, _, Value, &[&str], bool) =
        match name {
            "get_state" => (
                "What the app has open",
                "What JsonQuery has open now: the document (where from, size, what is at its \
                 top), the query in the query box and how it went, the page in front, the \
                 workspace, and what the user allows. Call this first.",
                json!({}),
                &[],
                true,
            ),
            "document_info" => (
                "About a document",
                "Facts about a JSON document without its data: where it is from, its size and \
                 when it changed, JSON or NDJSON, how many values of each kind and how deep, \
                 its biggest arrays and members, and how many pages reading it whole takes. \
                 For a large one, call this (then describe) before reading it.",
                json!({
                    "input": input_schema(DOCUMENT),
                    "pointer": pointer_schema()
                }),
                &[],
                true,
            ),
            "describe" => (
                "Shape of a value",
                "The shape of a value without its data: an object's keys and their kinds, an \
                 array's length and the shape of its items (from a sample), number ranges and \
                 a few example strings. Learn a large document with this before reading or \
                 querying it.",
                json!({
                    "input": input_schema(DOCUMENT),
                    "pointer": pointer_schema(),
                    "depth": {"type": "integer", "minimum": 1, "maximum": MAX_DEPTH,
                              "description": format!("Levels down (default {DEFAULT_DEPTH}).")},
                    "sample": {"type": "integer", "minimum": 1, "maximum": MAX_SAMPLE,
                               "description": format!("Items of a long array looked at (default {DEFAULT_SAMPLE}).")}
                }),
                &[],
                true,
            ),
            "read_value" => (
                "Read a value",
                "The value at a JSON Pointer, a page at a time: an array's items or an \
                 object's members from `offset`, at most `limit`. Of a value too big for a \
                 page, a snippet, said in `snippet`: a part left out is {\"$omitted\": what, \
                 \"pointer\": where}, read by that pointer.",
                json!({
                    "input": input_schema(DOCUMENT),
                    "pointer": pointer_schema(),
                    "offset": offset_schema(),
                    "limit": limit_schema(DEFAULT_LIMIT)
                }),
                &[],
                true,
            ),
            "search" => (
                "Find text",
                "Find the values and keys that contain `text` (any case), or match it as a \
                 regular expression: their JSON Pointers, each with a preview.",
                json!({
                    "text": {"type": "string", "description": "What to look for."},
                    "regex": {"type": "boolean", "description": "`text` is a regular expression (default false)."},
                    "input": input_schema(DOCUMENT),
                    "offset": offset_schema(),
                    "limit": limit_schema(DEFAULT_SEARCH_LIMIT)
                }),
                &["text"],
                true,
            ),
            "run_query" => (
                "Run a query",
                "Run a query out of sight: the user sees nothing change. jq unless `language` \
                 says otherwise or the query plainly is another ($.a JSONPath, /a JSON \
                 Pointer). `count` is how many results there are (all of them when \
                 `complete`); they are kept as \"last\" if not too many.",
                json!({
                    "query": {"type": "string", "description": "The query."},
                    "language": language_schema(),
                    "input": input_schema(DOCUMENT),
                    "offset": offset_schema(),
                    "limit": limit_schema(DEFAULT_LIMIT),
                    "timeout_ms": timeout_schema()
                }),
                &["query"],
                true,
            ),
            "get_results" => (
                "Results the user sees",
                "The results the user sees, with their query and how it went.",
                json!({
                    "offset": offset_schema(),
                    "limit": limit_schema(DEFAULT_LIMIT)
                }),
                &[],
                true,
            ),
            "query_help" => (
                "Help on the query languages",
                "Help on the query languages. No `language`: what each is. With one: its \
                 lessons and cheat sheet; with `topic` too, the lessons about it, with \
                 examples. `topic` \"known issues\": where this jq (jaq) differs from jq 1.8.",
                json!({
                    "language": language_schema(),
                    "topic": {"type": "string", "description": "A word or two to find lessons by, or \"known issues\"."},
                    "offset": offset_schema()
                }),
                &[],
                true,
            ),
            "workspace_list" => (
                "The workspace",
                "The user's saved queries, schemas, patches, documents and web addresses, by \
                 folder, with the path each is read and opened by.",
                json!({
                    "folder": {"type": "string", "description": "Only this folder (a path as listed)."},
                    "offset": offset_schema()
                }),
                &[],
                true,
            ),
            "workspace_read" => (
                "Read a workspace item",
                "What an item of the workspace holds: a query's text and language, a schema, a \
                 patch, a document, a web address; with `sample`, its sample too.",
                json!({
                    "path": {"type": "string", "description": "The item's path, as workspace_list gives it."},
                    "sample": {"type": "boolean", "description": "Give its sample too (default false)."}
                }),
                &["path"],
                true,
            ),
            "open_document" => (
                "Open a document",
                "Open a JSON document in the app in place of what is open, as the user would: \
                 a file `path` or JSON `text`. Waits until it has loaded and says what it is. \
                 Needs Edit.",
                json!({
                    "path": {"type": "string", "description": "A JSON file (a full path)."},
                    "text": {"type": "string", "description": "JSON text, instead of a file."},
                    "timeout_ms": timeout_schema()
                }),
                &[],
                false,
            ),
            "set_query" => (
                "Put a query in the query box",
                "Put a query in the app's query box and run it, as the user would: the user \
                 sees it and its results. Says how it went, with the first results. Needs Edit.",
                json!({
                    "query": {"type": "string", "description": "The query."},
                    "language": language_schema(),
                    "run": {"type": "boolean", "description": "Run it (default true)."},
                    "limit": limit_schema(DEFAULT_LIMIT),
                    "timeout_ms": timeout_schema()
                }),
                &["query"],
                false,
            ),
            "reveal" => (
                "Show a node",
                "Show the user a value: the Source or Results tree opens down to `pointer` and \
                 scrolls to it. Needs Edit.",
                json!({
                    "target": target_schema(),
                    "pointer": pointer_schema()
                }),
                &["pointer"],
                false,
            ),
            "workspace_open" => (
                "Open a workspace item",
                "Open a workspace item as a click on it does: a query goes in the query box \
                 (its sample opened and the query run), a document or web address is opened, a \
                 schema or patch goes to its tool. Needs Edit.",
                json!({
                    "path": {"type": "string", "description": "The item's path, as workspace_list gives it."},
                    "sample": {"type": "boolean", "description": "Open its sample too and run the query (default true)."},
                    "timeout_ms": timeout_schema()
                }),
                &["path"],
                false,
            ),
            "format_json" => (
                "Format JSON",
                "Write a JSON value as text, as the Format tool does: indented or minified, \
                 keys sorted and non-ASCII escaped if asked.",
                json!({
                    "input": input_schema("The value."),
                    "indent": indent_schema(),
                    "sort_keys": {"type": "boolean", "description": "Sort every object's keys (default false)."},
                    "ascii_only": {"type": "boolean", "description": "Write non-ASCII as \\uXXXX (default false)."}
                }),
                &["input"],
                true,
            ),
            "diff_documents" => (
                "Compare two JSON values",
                "Compare two JSON values, as the Diff tool does: what was added, removed and \
                 changed, each with its JSON Pointer and both sides. With `as_patch`, the JSON \
                 Patch (RFC 6902) from left to right too, kept as \"last\".",
                json!({
                    "left": input_schema("Before."),
                    "right": input_schema("After."),
                    "as_patch": {"type": "boolean", "description": "Give the JSON Patch too (default false)."},
                    "offset": offset_schema(),
                    "limit": limit_schema(DEFAULT_LIMIT),
                    "timeout_ms": timeout_schema()
                }),
                &["left", "right"],
                true,
            ),
            "apply_patch" => (
                "Apply a patch",
                "Apply a JSON Patch (RFC 6902, an array) or a JSON Merge Patch (RFC 7386, an \
                 object) to a value, as the Patch tool does. The result is kept as \"last\".",
                json!({
                    "input": input_schema("The value."),
                    "patch": input_schema("The patch."),
                    "merge_patch": {"type": "boolean", "description": "true: a merge patch; false: a JSON Patch (default: an object is a merge patch)."}
                }),
                &["input", "patch"],
                true,
            ),
            "merge_documents" => (
                "Merge JSON values",
                "Merge JSON values into one, as the Merge tool does. `how`: append (arrays end \
                 to end, objects' keys combined, the later wins), unique (append, sorted, \
                 repeats dropped), deep (objects merged all the way down) or bundle (each kept \
                 whole, by name). Or a jq `filter` over the values as one array ($files: their \
                 names). The result is kept as \"last\".",
                json!({
                    "inputs": {"type": "array", "minItems": 2, "maxItems": MAX_MERGED, "items": input_schema("A value."),
                               "description": "The values, in order."},
                    "how": {"type": "string", "enum": ["append", "unique", "deep", "bundle"],
                            "description": "Default append."},
                    "filter": {"type": "string", "description": "A jq filter, instead of `how`."},
                    "timeout_ms": timeout_schema()
                }),
                &["inputs"],
                true,
            ),
            "validate_json" => (
                "Check against a schema",
                "Check a JSON value against a JSON Schema (drafts 4 to 2020-12), as the \
                 Validate tool does: whether it is valid, and each problem with the value's \
                 JSON Pointer, what is wrong and the keyword.",
                json!({
                    "input": input_schema("The value."),
                    "schema": input_schema("The JSON Schema."),
                    "check_formats": {"type": "boolean", "description": "Check strings' `format` too (default false)."},
                    "offset": offset_schema(),
                    "limit": limit_schema(DEFAULT_LIMIT),
                    "timeout_ms": timeout_schema()
                }),
                &["input", "schema"],
                true,
            ),
            "create_schema" => (
                "Make a schema",
                "Make a JSON Schema from a JSON value, as the Create Schema tool does: types, \
                 the keys every object has (required), string patterns, and if asked enums and \
                 number bounds. It is kept as \"last\".",
                json!({
                    "input": input_schema("The value."),
                    "draft": draft_schema(),
                    "required": {"type": "boolean", "description": "Default true."},
                    "closed": {"type": "boolean", "description": "additionalProperties false (default false)."},
                    "patterns": {"type": "boolean", "description": "Default true."},
                    "enums": {"type": "boolean", "description": "Enums of strings with few values (default false)."},
                    "bounds": {"type": "boolean", "description": "Numbers' least and greatest (default false)."},
                    "timeout_ms": timeout_schema()
                }),
                &["input"],
                true,
            ),
            "show_in_tool" => (
                "Show it on a tool's page",
                "Show the user a page of the Tools with these inputs in its boxes, and run it: \
                 format and create_schema take one, diff two (left, right), patch two \
                 (document, patch), validate two (document, schema), merge two or more \
                 {\"file\"}s. `options` as for the tool that computes the same. Needs Edit.",
                json!({
                    "tool": {"type": "string", "enum": ["format", "diff", "patch", "validate", "create_schema", "merge"]},
                    "inputs": {"type": "array", "minItems": 1, "maxItems": MAX_MERGED, "items": input_schema("A box's value."),
                               "description": "In the page's order."},
                    "options": {"type": "object", "description": "indent, sort_keys, ascii_only (format); merge_patch (patch); check_formats (validate); draft, required, closed, patterns, enums, bounds (create_schema); how or filter (merge)."},
                    "run": {"type": "boolean", "description": "Run it (default true)."}
                }),
                &["tool", "inputs"],
                false,
            ),
            "workspace_save" => (
                "Save in the workspace",
                "Save an item in the user's workspace: a query (in its language), a schema, a \
                 patch, a document or a web address, in a folder (made if needed), with a \
                 sample if given. An item of that name is replaced only with `replace`, and the \
                 app may ask the user first. Needs Edit.",
                json!({
                    "path": {"type": "string", "description": "Folder and name: \"orders/late\"."},
                    "kind": {"type": "string", "enum": ["query", "schema", "patch", "document", "address"], "description": "Default: what the name's ending or the content says."},
                    "content": {"description": "A query or web address: its text. Otherwise the JSON itself, or \"document\", \"results\", \"last\"."},
                    "language": language_schema(),
                    "sample": input_schema("A sample document for a query, schema or patch."),
                    "replace": {"type": "boolean", "description": "Replace an item of that name (default false)."}
                }),
                &["path", "content"],
                false,
            ),
            "workspace_new_folder" => (
                "Make a folder",
                "Make a folder in the user's workspace (and those it is in). Needs Edit.",
                json!({
                    "path": {"type": "string", "description": "The folder: \"reports/2026\"."}
                }),
                &["path"],
                false,
            ),
            "workspace_move" => (
                "Move or rename",
                "Move or rename an item or a folder of the user's workspace. Nothing is \
                 replaced. Needs Edit.",
                json!({
                    "path": {"type": "string", "description": "The item or folder, as workspace_list gives it."},
                    "to": {"type": "string", "description": "The folder to move it to (\"\": the top)."},
                    "name": {"type": "string", "description": "Its new name."}
                }),
                &["path"],
                false,
            ),
            "workspace_delete" => (
                "Delete from the workspace",
                "Delete an item (and its sample) or a folder (and all in it) from the user's \
                 workspace. The app may ask the user first. Needs Edit.",
                json!({
                    "path": {"type": "string", "description": "The item or folder, as workspace_list gives it."}
                }),
                &["path"],
                false,
            ),
            "save_file" => (
                "Save to a file",
                "Save JSON to a file (a full path). A file that is there is replaced only with \
                 `replace`. The app may ask the user first. Needs Edit.",
                json!({
                    "input": input_schema("What to save."),
                    "path": {"type": "string", "description": "The file (a full path)."},
                    "indent": indent_schema(),
                    "replace": {"type": "boolean", "description": "Replace a file that is there (default false)."}
                }),
                &["input", "path"],
                false,
            ),
            "open_url" => (
                "Open a web address",
                "Load the JSON at a web address (http or https) as the Source, as typing it in \
                 the Source field does. The app may ask the user first. Needs Edit.",
                json!({
                    "url": {"type": "string", "description": "The address."},
                    "timeout_ms": timeout_schema()
                }),
                &["url"],
                false,
            ),
            _ => unreachable!("a tool of TOOL_NAMES"),
        };
    let (destructive, open_world) = match name {
        "workspace_save" | "workspace_delete" | "save_file" => (true, false),
        "open_url" => (false, true),
        _ => (false, false),
    };
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        },
        "annotations": {
            "title": title,
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "idempotentHint": read_only,
            "openWorldHint": open_world
        }
    })
}

/// How many values `merge_documents` and `show_in_tool` take at most.
pub const MAX_MERGED: usize = 100;

/// What the tools that look at a document say of their `input`.
const DOCUMENT: &str = "Default \"document\".";

/// A JSON value a tool works on ([`Input`]): any value is taken (see [`input_of`]), and
/// the schema says so in a few words, which is what a small model reads of it.
fn input_schema(what: &str) -> Value {
    json!({
        "description": format!("{what} The JSON itself (or its text), \"document\" (open), \"results\" (the user's), \"last\" (your last result), a JSON file's full path, or {{\"workspace\": path}}.")
    })
}

fn indent_schema() -> Value {
    json!({"type": "string", "enum": ["2", "4", "tab", "minified"], "description": "Default \"2\"."})
}

fn draft_schema() -> Value {
    json!({"type": "string", "enum": ["2020-12", "2019-09", "7", "6", "4"], "description": "Default 2020-12."})
}

fn target_schema() -> Value {
    json!({"type": "string", "enum": ["source", "results"],
           "description": "The open document (default) or the results in the query box."})
}

fn pointer_schema() -> Value {
    json!({"type": "string", "description": "A JSON Pointer: \"\" (default) is the whole, \"/users/0\" a value in it."})
}

fn language_schema() -> Value {
    json!({"type": "string", "enum": ["jq", "jsonpath", "jmespath", "pointer"],
           "description": "Guessed from the query if not given."})
}

fn offset_schema() -> Value {
    json!({"type": "integer", "minimum": 0, "description": "A next_offset given before (default 0)."})
}

fn limit_schema(default: usize) -> Value {
    json!({"type": "integer", "minimum": 1, "maximum": MAX_LIMIT,
           "description": format!("At most this many (default {default}).")})
}

fn timeout_schema() -> Value {
    json!({"type": "integer", "minimum": 100, "maximum": MAX_TIMEOUT.as_millis() as u64,
           "description": format!("Default {}.", DEFAULT_TIMEOUT.as_millis())})
}

/// Read what tool `name` was asked (`arguments` of `tools/call`). `Ok(None)`: there is
/// no such tool. What the answer may hold is the default page's, until the server fits
/// the call to the user's ([`Call::fit`]).
pub fn parse(name: &str, arguments: Option<&Value>) -> Option<Result<Call, String>> {
    let allowed: &[&str] = match name {
        "get_state" => &[],
        "document_info" => &["input", "pointer"],
        "describe" => &["input", "pointer", "depth", "sample"],
        "read_value" => &["input", "pointer", "offset", "limit"],
        "search" => &["text", "regex", "input", "offset", "limit"],
        "run_query" => &[
            "query",
            "language",
            "input",
            "offset",
            "limit",
            "timeout_ms",
        ],
        "get_results" => &["offset", "limit"],
        "query_help" => &["language", "topic", "offset"],
        "workspace_list" => &["folder", "offset"],
        "workspace_read" => &["path", "sample"],
        "open_document" => &["path", "text", "timeout_ms"],
        "set_query" => &["query", "language", "run", "limit", "timeout_ms"],
        "reveal" => &["target", "pointer"],
        "workspace_open" => &["path", "sample", "timeout_ms"],
        "format_json" => &["input", "indent", "sort_keys", "ascii_only"],
        "diff_documents" => &["left", "right", "as_patch", "offset", "limit", "timeout_ms"],
        "apply_patch" => &["input", "patch", "merge_patch"],
        "merge_documents" => &["inputs", "how", "filter", "timeout_ms"],
        "validate_json" => &[
            "input",
            "schema",
            "check_formats",
            "offset",
            "limit",
            "timeout_ms",
        ],
        "create_schema" => &[
            "input",
            "draft",
            "required",
            "closed",
            "patterns",
            "enums",
            "bounds",
            "timeout_ms",
        ],
        "show_in_tool" => &["tool", "inputs", "options", "run"],
        "workspace_save" => &["path", "kind", "content", "language", "sample", "replace"],
        "workspace_new_folder" => &["path"],
        "workspace_move" => &["path", "to", "name"],
        "workspace_delete" => &["path"],
        "save_file" => &["input", "path", "indent", "replace"],
        "open_url" => &["url", "timeout_ms"],
        _ => return None,
    };
    let arguments = usual_names(name, allowed, arguments);
    Some(
        Args::new(arguments.as_ref(), allowed)
            .and_then(|args| read_call(name, &args))
            .map_err(|e| match example(name) {
                Some(example) => format!("{e}. For example: {example}"),
                None => e,
            }),
    )
}

/// Names a model often gives an argument, and the name the tool knows it by. A name is
/// taken for the other by a tool that has the other and not the name itself, when the
/// call does not give the other too.
const ALIASES: &[(&str, &str)] = &[
    ("overwrite", "replace"),
    ("force", "replace"),
    ("type", "kind"),
    ("lang", "language"),
    ("engine", "language"),
    ("dialect", "language"),
    ("expression", "query"),
    ("filter", "query"),
    ("jq", "query"),
    // Where a tool takes a query, its text is the query (before `input` below).
    ("text", "query"),
    ("data", "input"),
    ("json", "input"),
    ("document", "input"),
    ("doc", "input"),
    ("value", "input"),
    ("source", "input"),
    ("content", "input"),
    ("text", "input"),
    ("data", "content"),
    ("json", "content"),
    ("value", "content"),
    ("text", "content"),
    ("input", "content"),
    ("body", "content"),
    ("file", "path"),
    ("filename", "path"),
    ("file_path", "path"),
    ("filepath", "path"),
    ("documents", "inputs"),
    ("files", "inputs"),
    ("values", "inputs"),
    ("first", "left"),
    ("old", "left"),
    ("before", "left"),
    ("second", "right"),
    ("new", "right"),
    ("after", "right"),
];

/// The call's arguments as the tool names them: arguments sent as JSON text read, the
/// names of [`ALIASES`] changed, a file given to a tool that reads an input taken as its
/// input, and a workspace item's folder and name taken as its path.
fn usual_names(name: &str, allowed: &[&str], arguments: Option<&Value>) -> Option<Value> {
    let mut map = match arguments {
        None | Some(Value::Null) => return None,
        Some(Value::Object(map)) => map.clone(),
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => map,
            _ => return arguments.cloned(),
        },
        Some(other) => return Some(other.clone()),
    };
    let takes = |key: &str| allowed.contains(&key);
    for &(alias, usual) in ALIASES {
        if takes(usual) && !takes(alias) && !map.contains_key(usual) {
            if let Some(value) = map.remove(alias) {
                map.insert(usual.to_owned(), value);
            }
        }
    }
    if takes("input") && !takes("path") && !map.contains_key("input") {
        for key in ["path", "file", "filename", "file_path"] {
            if let Some(Value::String(path)) = map.get(key).cloned() {
                map.remove(key);
                map.insert("input".to_owned(), json!({ "file": path }));
                break;
            }
        }
    }
    if name == "open_document" && !map.contains_key("text") && !map.contains_key("path") {
        for key in ["json", "data", "content", "document", "input", "value"] {
            if let Some(value) = map.remove(key) {
                map.insert("text".to_owned(), value);
                break;
            }
        }
    }
    if name == "workspace_save" && !map.contains_key("path") {
        if let Some(Value::String(item)) = map.remove("name") {
            let path = match map.remove("folder") {
                Some(Value::String(folder)) if !folder.trim().is_empty() => {
                    format!("{}/{item}", folder.trim_end_matches('/'))
                }
                _ => item,
            };
            map.insert("path".to_owned(), Value::String(path));
        }
    }
    Some(Value::Object(map))
}

/// A call of tool `name` that works, for an error to end with: a model that sees one
/// makes the next call right.
fn example(name: &str) -> Option<&'static str> {
    Some(match name {
        "document_info" => r#"{} for the open document, or {"input": {"a": [1, 2]}}"#,
        "describe" => r#"{"pointer": "/items", "depth": 2}"#,
        "read_value" => r#"{"pointer": "/items/0"}"#,
        "search" => r#"{"text": "error"}"#,
        "run_query" => r#"{"query": ".items[] | select(.price > 10)", "language": "jq"}"#,
        "set_query" => r#"{"query": ".items | length"}"#,
        "query_help" => r#"{"language": "jq", "topic": "select"}"#,
        "workspace_read" => r#"{"path": "queries/late"}"#,
        "workspace_open" => r#"{"path": "queries/late"}"#,
        "open_document" => r#"{"text": "{\"a\": 1}"} or {"path": "/home/me/data.json"}"#,
        "reveal" => r#"{"target": "results", "pointer": "/0"}"#,
        "format_json" => r#"{"input": {"b": 1, "a": [1, 2]}, "sort_keys": true}"#,
        "diff_documents" => r#"{"left": {"a": 1}, "right": {"a": 2}}"#,
        "apply_patch" => {
            r#"{"input": {"a": 1}, "patch": [{"op": "replace", "path": "/a", "value": 2}]}"#
        }
        "merge_documents" => r#"{"inputs": [{"a": 1}, {"b": 2}]}"#,
        "validate_json" => {
            r#"{"input": {"id": 1}, "schema": {"type": "object", "required": ["id"]}}"#
        }
        "create_schema" => r#"{"input": [{"id": 1, "name": "x"}]}"#,
        "show_in_tool" => r#"{"tool": "diff", "inputs": [{"a": 1}, {"a": 2}]}"#,
        "workspace_save" => {
            r#"{"path": "examples/orders", "content": {"orders": []}}, or {"path": "queries/late", "kind": "query", "content": ".orders[] | select(.late)"}"#
        }
        "workspace_new_folder" => r#"{"path": "reports/2026"}"#,
        "workspace_move" => r#"{"path": "queries/late", "to": "archive"}"#,
        "workspace_delete" => r#"{"path": "queries/late"}"#,
        "save_file" => r#"{"input": "last", "path": "/home/me/out.json"}"#,
        "open_url" => r#"{"url": "https://example.com/data.json"}"#,
        _ => return None,
    })
}

fn read_call(name: &str, args: &Args<'_>) -> Result<Call, String> {
    let max_bytes = default_budget();
    Ok(match name {
        "get_state" => Call::State,
        "document_info" => Call::Info {
            input: args.document()?,
            pointer: args.pointer()?,
            max_bytes,
        },
        "describe" => Call::Describe {
            input: args.document()?,
            pointer: args.pointer()?,
            depth: args.number("depth", DEFAULT_DEPTH, 1, MAX_DEPTH)?,
            sample: args.number("sample", DEFAULT_SAMPLE, 1, MAX_SAMPLE)?,
            max_bytes,
        },
        "read_value" => Call::ReadValue {
            input: args.document()?,
            pointer: args.pointer()?,
            offset: args.offset()?,
            limit: args.limit()?,
            max_bytes,
        },
        "search" => Call::Search {
            input: args.document()?,
            text: args.required_text("text")?,
            regex: args.flag("regex", false)?,
            offset: args.offset()?,
            limit: args.number("limit", DEFAULT_SEARCH_LIMIT, 1, MAX_LIMIT)?,
            max_bytes,
        },
        "run_query" => Call::RunQuery {
            query: args.required_text("query")?,
            language: args.language()?,
            input: args.document()?,
            offset: args.offset()?,
            limit: args.limit()?,
            max_bytes,
            timeout: args.timeout()?,
        },
        "get_results" => Call::Results {
            offset: args.offset()?,
            limit: args.limit()?,
            max_bytes,
        },
        "query_help" => Call::Help {
            language: args.language()?,
            topic: args.text("topic")?.filter(|t| !t.trim().is_empty()),
            offset: args.offset()?,
            max_bytes,
        },
        "workspace_list" => Call::WorkspaceList {
            folder: args.text("folder")?.filter(|f| !f.trim().is_empty()),
            offset: args.offset()?,
            max_bytes,
        },
        "workspace_read" => Call::WorkspaceRead {
            path: args.required_text("path")?,
            sample: args.flag("sample", false)?,
            max_bytes,
        },
        "open_document" => {
            let mut path = args.text("path")?.filter(|p| !p.trim().is_empty());
            // The JSON itself is its text; a JSON file's full path given as the text is the file.
            let mut text = match args.get("text") {
                None => None,
                Some(Value::String(text)) => Some(text.clone()),
                Some(json) => Some(json.to_string()),
            };
            if path.is_none() {
                if let Some(Input::File(file)) = text.as_deref().map(text_input) {
                    path = Some(file.display().to_string());
                    text = None;
                }
            }
            if path.is_some() == text.is_some() {
                return Err("give one of `path` and `text`".to_owned());
            }
            Call::OpenDocument {
                path: path.map(PathBuf::from),
                text,
                timeout: args.timeout()?,
            }
        }
        "set_query" => Call::SetQuery {
            query: args.required_text("query")?,
            language: args.language()?,
            run: args.flag("run", true)?,
            limit: args.limit()?,
            max_bytes,
            timeout: args.timeout()?,
        },
        "reveal" => Call::Reveal {
            target: args.target()?,
            pointer: args.pointer()?,
        },
        "workspace_open" => Call::WorkspaceOpen {
            path: args.required_text("path")?,
            sample: args.flag("sample", true)?,
            timeout: args.timeout()?,
        },
        "format_json" => Call::Format {
            input: args.input("input")?,
            options: args.format_options()?,
            max_bytes,
        },
        "diff_documents" => Call::Diff {
            left: args.input("left")?,
            right: args.input("right")?,
            offset: args.offset()?,
            limit: args.limit()?,
            as_patch: args.flag("as_patch", false)?,
            max_bytes,
            timeout: args.timeout()?,
        },
        "apply_patch" => Call::Patch {
            input: args.input("input")?,
            patch: args.input("patch")?,
            merge_patch: args.optional_flag("merge_patch")?,
            max_bytes,
        },
        "merge_documents" => {
            let inputs = args.inputs("inputs")?;
            if inputs.len() < 2 {
                return Err("`inputs` needs two values or more".to_owned());
            }
            Call::Merge {
                inputs,
                filter: args.merge_filter()?,
                max_bytes,
                timeout: args.timeout()?,
            }
        }
        "validate_json" => Call::Validate {
            input: args.input("input")?,
            schema: args.input("schema")?,
            check_formats: args.flag("check_formats", false)?,
            offset: args.offset()?,
            limit: args.limit()?,
            max_bytes,
            timeout: args.timeout()?,
        },
        "create_schema" => Call::CreateSchema {
            input: args.input("input")?,
            options: args.schema_options()?,
            max_bytes,
            timeout: args.timeout()?,
        },
        "show_in_tool" => {
            let options = match args.get("options") {
                None => None,
                Some(Value::Object(map)) => Some(map),
                Some(_) => return Err("`options` must be an object".to_owned()),
            };
            let tool = args.required_text("tool")?;
            let page = page_of(&tool, options)?;
            let inputs = args.inputs("inputs")?;
            let boxes = page.boxes();
            match &page {
                ToolPage::Merge { .. } => {
                    if inputs.len() < 2 || inputs.iter().any(|i| !matches!(i, Input::File(_))) {
                        return Err(
                            "merge shows two JSON files or more: give each as {\"file\": …}"
                                .to_owned(),
                        );
                    }
                }
                _ if inputs.len() != boxes.len() => {
                    return Err(format!(
                        "{tool} takes {} input(s): {}",
                        boxes.len(),
                        boxes.join(", ")
                    ));
                }
                _ => {}
            }
            Call::ShowInTool {
                page,
                inputs,
                run: args.flag("run", true)?,
            }
        }
        "workspace_save" => {
            let (folder, name) = split_path(&args.required_text("path")?)?;
            // A name that ends as an item's file does says what it is; the item is named
            // without the ending.
            let (name, ending) = item_ending(&name);
            let language = args.language()?;
            let kind = match args.text("kind")? {
                Some(kind) => save_kind(&kind, language).ok_or(
                    "`kind` is one of \"query\", \"schema\", \"patch\", \"document\", \"address\"",
                )?,
                None => match ending {
                    Some(SaveKind::Query(ending)) => SaveKind::Query(language.unwrap_or(ending)),
                    Some(kind) => kind,
                    None => kind_of_content(args.get("content"), language),
                },
            };
            let content = if kind.is_text() {
                Input::Text(text_content(args.get("content"))?)
            } else {
                args.input("content")?
            };
            let sample = args.optional_input("sample")?;
            if sample.is_some() && matches!(kind, SaveKind::Document | SaveKind::Address) {
                return Err("a document or a web address has no sample".to_owned());
            }
            Call::WorkspaceSave {
                folder,
                name,
                kind,
                content,
                sample,
                replace: args.flag("replace", false)?,
            }
        }
        "workspace_new_folder" => Call::WorkspaceNewFolder {
            path: workspace_path(&args.required_text("path")?)?,
        },
        "workspace_move" => {
            let to = args.text("to")?.map(|t| workspace_folder(&t)).transpose()?;
            let name = args.text("name")?.filter(|n| !n.trim().is_empty());
            if to.is_none() && name.is_none() {
                return Err("give `to`, `name` or both".to_owned());
            }
            Call::WorkspaceMove {
                path: workspace_path(&args.required_text("path")?)?,
                to,
                name,
            }
        }
        "workspace_delete" => Call::WorkspaceDelete {
            path: workspace_path(&args.required_text("path")?)?,
        },
        "save_file" => Call::SaveFile {
            input: args.input("input")?,
            path: PathBuf::from(args.required_text("path")?),
            indent: args.indent()?,
            replace: args.flag("replace", false)?,
        },
        "open_url" => {
            let url = args.required_text("url")?.trim().to_owned();
            let scheme = url.split("://").next().unwrap_or("").to_ascii_lowercase();
            if !url.contains("://") || !(scheme == "http" || scheme == "https") {
                return Err("`url` is an http or https address".to_owned());
            }
            Call::OpenUrl {
                url,
                timeout: args.timeout()?,
            }
        }
        _ => unreachable!("a tool `parse` knows"),
    })
}

/// The page `tool` names, set as `options` say.
fn page_of(tool: &str, options: Option<&Map<String, Value>>) -> Result<ToolPage, String> {
    let key: String = tool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    // The names of the tools that do the same out of sight, and others a model gives them.
    let key = match key.as_str() {
        "formatjson" | "reformat" | "prettify" | "pretty" => "format".to_owned(),
        "diffjson" | "diffdocuments" | "compare" => "diff".to_owned(),
        "patchjson" | "applypatch" => "patch".to_owned(),
        "validatejson" | "validateschema" | "validation" => "validate".to_owned(),
        "schema" | "makeschema" | "inferschema" | "generateschema" => "createschema".to_owned(),
        "mergejson" | "mergedocuments" | "combine" => "merge".to_owned(),
        _ => key,
    };
    let allowed: &[&str] = match key.as_str() {
        "format" => &["indent", "sort_keys", "ascii_only"],
        "diff" => &[],
        "patch" => &["merge_patch"],
        "validate" => &["check_formats"],
        "createschema" | "create" => &["draft", "required", "closed", "patterns", "enums", "bounds"],
        "merge" => &["how", "filter"],
        _ => {
            return Err(
                "`tool` is one of \"format\", \"diff\", \"patch\", \"validate\", \"create_schema\", \"merge\""
                    .to_owned(),
            )
        }
    };
    let options = options.map(|m| Value::Object(m.clone()));
    let args = Args::new(options.as_ref(), allowed).map_err(|e| {
        e.replace("argument", "option")
            .replace("this tool", "this page")
    })?;
    Ok(match key.as_str() {
        "format" => ToolPage::Format(args.format_options()?),
        "diff" => ToolPage::Diff,
        "patch" => ToolPage::Patch {
            merge_patch: args.optional_flag("merge_patch")?,
        },
        "validate" => ToolPage::Validate {
            check_formats: args.flag("check_formats", false)?,
        },
        "merge" => ToolPage::Merge {
            filter: args.merge_filter()?,
        },
        _ => ToolPage::CreateSchema(args.schema_options()?),
    })
}

/// A path of the workspace as the app writes it: parts between `/`, none of them
/// empty, `.` or `..`.
fn workspace_path(path: &str) -> Result<String, String> {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("give a path in the workspace".to_owned());
    }
    if parts.iter().any(|p| *p == "." || *p == "..") {
        return Err(format!("\"{path}\" is not a path in the workspace"));
    }
    Ok(parts.join("/"))
}

/// A folder of the workspace: `""` is the top.
fn workspace_folder(path: &str) -> Result<String, String> {
    if path.trim().trim_matches('/').is_empty() {
        Ok(String::new())
    } else {
        workspace_path(path)
    }
}

/// `"orders/late"`: the folder `orders` and the name `late`.
fn split_path(path: &str) -> Result<(String, String), String> {
    let full = workspace_path(path)?;
    Ok(match full.rsplit_once('/') {
        Some((folder, name)) => (folder.to_owned(), name.to_owned()),
        None => (String::new(), full),
    })
}

/// What `value` names as the input of a tool (`key` is the argument's name). A model
/// reads a few words of a schema and gives what it means the way it finds easiest -- the
/// JSON itself, or its text -- as often as the forms the schema says. So:
/// - "document", "results" and "last" (and a few other names of them) are what they say;
/// - an object of one member `json`, `text`, `file` or `workspace` is that;
/// - the full path of a .json, .jsonl, .ndjson or .geojson file is that file;
/// - another string is the JSON it holds, or else text (which the tool that reads it then
///   says is not JSON, and where);
/// - any other value is the JSON itself.
fn input_of(value: &Value, key: &str) -> Result<Input, String> {
    match value {
        Value::String(text) => Ok(named_input(text).unwrap_or_else(|| text_input(text))),
        Value::Object(map) if map.len() == 1 => {
            let (what, given) = map.iter().next().expect("one member");
            match (what.as_str(), given) {
                ("json", value) => Ok(Input::Json(value.clone())),
                ("text", Value::String(text)) => Ok(Input::Text(text.clone())),
                ("file", Value::String(path)) if !path.trim().is_empty() => {
                    Ok(Input::File(PathBuf::from(path.trim())))
                }
                ("workspace", Value::String(path)) => Ok(Input::Workspace(workspace_path(path)?)),
                ("text" | "file" | "workspace", _) => Err(format!(
                    "`{key}`: {{\"{what}\": …}} takes a string{}",
                    if what == "file" { ", a full path" } else { "" }
                )),
                _ => Ok(Input::Json(value.clone())),
            }
        }
        _ => Ok(Input::Json(value.clone())),
    }
}

/// "document", "results", "last", by the names a model gives them.
fn named_input(text: &str) -> Option<Input> {
    let name = text.trim().to_ascii_lowercase();
    let name = name.strip_prefix("the ").unwrap_or(&name).trim();
    Some(match name {
        "document" | "doc" | "source" | "open document" | "current document" => Input::Document,
        "results" | "result" | "query results" => Input::Results,
        "last" | "kept" | "last result" | "previous" => Input::Last,
        _ => return None,
    })
}

/// A string that names none of those: a JSON file by its full path (on this system: on
/// Windows one starts with its drive; or from the home, `~`), the JSON it holds, or
/// text.
fn text_input(text: &str) -> Input {
    let trimmed = text.trim();
    let path = Path::new(trimmed);
    let full = path.is_absolute() || trimmed.starts_with("~/") || trimmed.starts_with("~\\");
    let json_file = full
        && !trimmed.contains('\n')
        && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ["json", "jsonl", "ndjson", "geojson"].contains(&e.to_ascii_lowercase().as_str())
        });
    if json_file {
        return Input::File(path.to_path_buf());
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(json) => Input::Json(json),
        Err(_) => Input::Text(text.to_owned()),
    }
}

/// What a `kind` of `workspace_save` names, by the words a model gives it.
fn save_kind(kind: &str, language: Option<Language>) -> Option<SaveKind> {
    let key: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    Some(match key.as_str() {
        "query" | "filter" | "expression" => SaveKind::Query(language.unwrap_or(Kind::Jq)),
        "schema" | "jsonschema" => SaveKind::Schema,
        "patch" | "jsonpatch" | "mergepatch" => SaveKind::Patch,
        "document" | "doc" | "json" | "data" | "file" => SaveKind::Document,
        "address" | "url" | "link" | "webaddress" => SaveKind::Address,
        _ => SaveKind::Query(language_of(&key)?),
    })
}

/// The name of an item without the ending of its file, and what the ending says it is:
/// "late.jq", a jq query; "orders.schema.json", a schema.
fn item_ending(name: &str) -> (String, Option<SaveKind>) {
    let lower = name.to_ascii_lowercase();
    let endings: [(&str, SaveKind); 11] = [
        (".schema.json", SaveKind::Schema),
        (".patch.json", SaveKind::Patch),
        (".jq", SaveKind::Query(Kind::Jq)),
        (".jsonpath", SaveKind::Query(Kind::JsonPath)),
        (".jmespath", SaveKind::Query(Kind::JmesPath)),
        (".jsonptr", SaveKind::Query(Kind::JsonPointer)),
        (".url", SaveKind::Address),
        (".json", SaveKind::Document),
        (".jsonl", SaveKind::Document),
        (".ndjson", SaveKind::Document),
        (".geojson", SaveKind::Document),
    ];
    for (ending, kind) in endings {
        if lower.ends_with(ending) && lower.len() > ending.len() {
            return (name[..name.len() - ending.len()].to_owned(), Some(kind));
        }
    }
    (name.to_owned(), None)
}

/// What an item is, when the call says neither its kind nor an ending: a web address,
/// a query (text that is not JSON), a schema (an object with `$schema`), a patch (an
/// array of operations), or a document.
fn kind_of_content(content: Option<&Value>, language: Option<Language>) -> SaveKind {
    let query = SaveKind::Query(language.unwrap_or(Kind::Jq));
    let json = match content {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                return SaveKind::Address;
            }
            if language.is_some() {
                return query;
            }
            match named_input(text) {
                Some(_) => return SaveKind::Document,
                None => match serde_json::from_str::<Value>(trimmed) {
                    Ok(json) => json,
                    Err(_) => return query,
                },
            }
        }
        Some(value) => value.clone(),
        None => return SaveKind::Document,
    };
    match &json {
        Value::Object(map) if map.contains_key("$schema") => SaveKind::Schema,
        Value::Array(ops)
            if !ops.is_empty()
                && ops
                    .iter()
                    .all(|op| op.get("op").is_some_and(Value::is_string)) =>
        {
            SaveKind::Patch
        }
        _ => SaveKind::Document,
    }
}

/// The text of a query or a web address, as a model gives it: a string, or an object
/// of one string.
fn text_content(content: Option<&Value>) -> Result<String, String> {
    match content {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        Some(Value::Object(map)) if map.len() == 1 => match map.values().next() {
            Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
            _ => Err("`content` of a query or a web address is its text".to_owned()),
        },
        None => Err("`content` is needed".to_owned()),
        Some(_) => Err("`content` of a query or a web address is its text".to_owned()),
    }
}

/// The arguments of a call, checked against what its tool takes.
struct Args<'a> {
    map: Option<&'a Map<String, Value>>,
}

impl<'a> Args<'a> {
    fn new(arguments: Option<&'a Value>, allowed: &[&str]) -> Result<Self, String> {
        let map = match arguments {
            None | Some(Value::Null) => None,
            Some(Value::Object(map)) => Some(map),
            Some(_) => return Err("the arguments must be an object".to_owned()),
        };
        if let Some(unknown) = map
            .into_iter()
            .flat_map(|m| m.keys())
            .find(|key| !allowed.contains(&key.as_str()))
        {
            return Err(if allowed.is_empty() {
                format!("unknown argument `{unknown}`: this tool takes none")
            } else {
                format!(
                    "unknown argument `{unknown}`: this tool takes {}",
                    allowed
                        .iter()
                        .map(|a| format!("`{a}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });
        }
        Ok(Args { map })
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.and_then(|m| m.get(key)).filter(|v| !v.is_null())
    }

    fn text(&self, key: &str) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("`{key}` must be a string")),
        }
    }

    fn required_text(&self, key: &str) -> Result<String, String> {
        match self.text(key)? {
            Some(text) if !text.trim().is_empty() => Ok(text),
            _ => Err(format!("`{key}` is needed")),
        }
    }

    fn number(&self, key: &str, default: usize, min: usize, max: usize) -> Result<usize, String> {
        let value = match self.get(key) {
            None => return Ok(default),
            Some(Value::Number(n)) => n.as_u64().or_else(|| {
                n.as_f64()
                    .filter(|f| f.fract() == 0.0 && *f >= 0.0)
                    .map(|f| f as u64)
            }),
            Some(Value::String(s)) => s.trim().parse::<u64>().ok(),
            Some(_) => None,
        };
        let Some(value) = value else {
            return Err(format!("`{key}` must be a whole number, 0 or more"));
        };
        let value = usize::try_from(value).unwrap_or(usize::MAX);
        if value < min {
            return Err(format!("`{key}` must be at least {min}"));
        }
        Ok(value.min(max))
    }

    fn flag(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(Value::String(s)) if s.eq_ignore_ascii_case("true") => Ok(true),
            Some(Value::String(s)) if s.eq_ignore_ascii_case("false") => Ok(false),
            Some(_) => Err(format!("`{key}` must be true or false")),
        }
    }

    fn target(&self) -> Result<Target, String> {
        match self.text("target")?.as_deref().map(str::to_ascii_lowercase) {
            None => Ok(Target::Source),
            Some(t) if t == "source" => Ok(Target::Source),
            Some(t) if t == "results" || t == "result" => Ok(Target::Results),
            Some(_) => Err("`target` is \"source\" or \"results\"".to_owned()),
        }
    }

    fn pointer(&self) -> Result<String, String> {
        let pointer = self.text("pointer")?.unwrap_or_default();
        if pointer.is_empty() || pointer.starts_with('/') {
            Ok(pointer)
        } else {
            Err(format!(
                "`pointer` is a JSON Pointer: \"\" or starting with \"/\" (as in \"/{pointer}\")"
            ))
        }
    }

    fn language(&self) -> Result<Option<Language>, String> {
        match self.text("language")? {
            None => Ok(None),
            Some(text) => language_of(&text).map(Some).ok_or_else(|| {
                "`language` is one of \"jq\", \"jsonpath\", \"jmespath\", \"pointer\"".to_owned()
            }),
        }
    }

    fn limit(&self) -> Result<usize, String> {
        self.number("limit", DEFAULT_LIMIT, 1, MAX_LIMIT)
    }

    fn offset(&self) -> Result<usize, String> {
        self.number("offset", 0, 0, usize::MAX)
    }

    fn optional_flag(&self, key: &str) -> Result<Option<bool>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(_) => self.flag(key, false).map(Some),
        }
    }

    fn input(&self, key: &str) -> Result<Input, String> {
        self.optional_input(key)?
            .ok_or_else(|| format!("`{key}` is needed"))
    }

    /// The `input` of a tool that looks at a document: the open one if none is given.
    fn document(&self) -> Result<Input, String> {
        Ok(self.optional_input("input")?.unwrap_or(Input::Document))
    }

    fn optional_input(&self, key: &str) -> Result<Option<Input>, String> {
        self.get(key).map(|v| input_of(v, key)).transpose()
    }

    fn inputs(&self, key: &str) -> Result<Vec<Input>, String> {
        match self.get(key) {
            Some(Value::Array(values)) if values.len() <= MAX_MERGED => values
                .iter()
                .enumerate()
                .map(|(i, v)| input_of(v, &format!("{key}[{i}]")))
                .collect(),
            Some(Value::Array(_)) => Err(format!("`{key}` takes {MAX_MERGED} values at most")),
            Some(_) => Err(format!("`{key}` must be an array")),
            None => Err(format!("`{key}` is needed")),
        }
    }

    fn indent(&self) -> Result<Indent, String> {
        let text = match self.get("indent") {
            None => return Ok(Indent::Spaces(2)),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::String(s)) => s.trim().to_ascii_lowercase(),
            Some(_) => String::new(),
        };
        match text.as_str() {
            "2" => Ok(Indent::Spaces(2)),
            "4" => Ok(Indent::Spaces(4)),
            "tab" | "\\t" => Ok(Indent::Tab),
            "minified" | "minify" | "compact" | "0" => Ok(Indent::Minified),
            _ => Err("`indent` is \"2\", \"4\", \"tab\" or \"minified\"".to_owned()),
        }
    }

    fn format_options(&self) -> Result<reformat::Options, String> {
        Ok(reformat::Options {
            indent: self.indent()?,
            sort_keys: self.flag("sort_keys", false)?,
            ascii_only: self.flag("ascii_only", false)?,
        })
    }

    fn schema_options(&self) -> Result<infer::Options, String> {
        let defaults = infer::Options::default();
        let draft = match self.text("draft")? {
            None => defaults.draft,
            Some(text) => {
                let key: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
                match key.as_str() {
                    "202012" => Draft::D202012,
                    "201909" => Draft::D201909,
                    "7" | "07" => Draft::D7,
                    "6" | "06" => Draft::D6,
                    "4" | "04" => Draft::D4,
                    _ => {
                        return Err(
                            "`draft` is one of \"2020-12\", \"2019-09\", \"7\", \"6\", \"4\""
                                .to_owned(),
                        )
                    }
                }
            }
        };
        Ok(infer::Options {
            draft,
            required: self.flag("required", defaults.required)?,
            closed: self.flag("closed", defaults.closed)?,
            patterns: self.flag("patterns", defaults.patterns)?,
            enums: self.flag("enums", defaults.enums)?,
            bounds: self.flag("bounds", defaults.bounds)?,
        })
    }

    /// The jq filter of a merge: `filter`, or the one `how` names.
    fn merge_filter(&self) -> Result<String, String> {
        if let Some(filter) = self.text("filter")?.filter(|f| !f.trim().is_empty()) {
            if self.get("how").is_some() {
                return Err("give `how` or `filter`, not both".to_owned());
            }
            return Ok(filter);
        }
        let presets = &jsonquery_query::merge::PRESETS;
        let index = match self
            .text("how")?
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("append") => 0,
            Some("unique") => 1,
            Some("deep") => 2,
            Some("bundle") => 3,
            Some(_) => {
                return Err(
                    "`how` is one of \"append\", \"unique\", \"deep\", \"bundle\"".to_owned(),
                )
            }
        };
        Ok(presets[index].filter.to_owned())
    }

    fn timeout(&self) -> Result<Duration, String> {
        let ms = self.number(
            "timeout_ms",
            DEFAULT_TIMEOUT.as_millis() as usize,
            1,
            MAX_TIMEOUT.as_millis() as usize,
        )?;
        Ok(Duration::from_millis(ms.max(100) as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> Result<Call, String> {
        parse(name, Some(&args)).expect("a tool")
    }

    /// The full path of a file `name` on this system (on Windows one starts with its
    /// drive: "/data/…" is not one there).
    fn full_path(name: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\data\{name}")
        } else {
            format!("/data/{name}")
        }
    }

    #[test]
    fn every_tool_is_listed_once_with_an_object_schema() {
        let list = tool_list();
        assert_eq!(list.len(), TOOL_NAMES.len());
        for (tool, name) in list.iter().zip(TOOL_NAMES) {
            assert_eq!(tool["name"], name);
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["description"].as_str().unwrap().len() > 40, "{name}");
            // What the schema says it takes is what `parse` takes.
            for key in tool["inputSchema"]["properties"]
                .as_object()
                .unwrap()
                .keys()
            {
                let mut args = Map::new();
                args.insert(key.clone(), Value::Null);
                assert!(
                    !matches!(parse(name, Some(&Value::Object(args))), Some(Err(e)) if e.starts_with("unknown")),
                    "{name} does not take its own `{key}`"
                );
            }
        }
    }

    #[test]
    fn a_tool_that_does_not_exist_is_none() {
        assert!(parse("rm_rf", None).is_none());
    }

    #[test]
    fn defaults_fill_in_what_is_not_given() {
        assert_eq!(
            call("read_value", json!({})).unwrap(),
            Call::ReadValue {
                input: Input::Document,
                pointer: String::new(),
                offset: 0,
                limit: DEFAULT_LIMIT,
                max_bytes: budget_of(DEFAULT_PAGE),
            }
        );
        assert_eq!(parse("get_state", None).unwrap().unwrap(), Call::State);
        assert_eq!(
            call("document_info", json!({})).unwrap(),
            Call::Info {
                input: Input::Document,
                pointer: String::new(),
                max_bytes: budget_of(DEFAULT_PAGE),
            }
        );
    }

    #[test]
    fn a_call_is_fitted_to_the_page_the_user_set_and_the_page_is_not_asked_for() {
        let mut read = call("read_value", json!({})).unwrap();
        read.fit(4_000);
        assert!(
            matches!(
                read,
                Call::ReadValue {
                    max_bytes: 3_500,
                    ..
                }
            ),
            "{read:?}"
        );
        let mut help = call("query_help", json!({"offset": 10})).unwrap();
        help.fit(200_000);
        assert!(
            matches!(
                help,
                Call::Help {
                    offset: 10,
                    max_bytes: 175_000,
                    ..
                }
            ),
            "{help:?}"
        );
        assert_eq!(budget_of(500), MIN_BUDGET);
        // Every tool that gives values, items or text is fitted.
        for (name, args) in [
            ("document_info", json!({})),
            ("describe", json!({})),
            ("search", json!({"text": "a"})),
            ("run_query", json!({"query": "."})),
            ("get_results", json!({})),
            ("set_query", json!({"query": "."})),
            ("workspace_list", json!({})),
            ("workspace_read", json!({"path": "a"})),
            ("format_json", json!({"input": "document"})),
            (
                "diff_documents",
                json!({"left": "document", "right": "results"}),
            ),
            (
                "apply_patch",
                json!({"input": "document", "patch": "results"}),
            ),
            (
                "merge_documents",
                json!({"inputs": ["document", "results"]}),
            ),
            (
                "validate_json",
                json!({"input": "document", "schema": "results"}),
            ),
            ("create_schema", json!({"input": "document"})),
        ] {
            let mut fitted = call(name, args).unwrap();
            fitted.fit(8_000);
            assert!(
                format!("{fitted:?}").contains("max_bytes: 7000"),
                "{name}: {fitted:?}"
            );
        }
        for name in TOOL_NAMES {
            let schema = definition(name);
            assert!(
                schema["inputSchema"]["properties"]
                    .get("max_bytes")
                    .is_none(),
                "{name}: the page is the user's to set"
            );
        }
        assert!(call("read_value", json!({"max_bytes": 1000}))
            .unwrap_err()
            .contains("unknown"));
    }

    #[test]
    fn what_a_model_reads_of_the_tools_is_kept_short() {
        // A model reads every tool's description and schema before it does anything: a
        // small one, run on the user's computer, has little room for them.
        let read: usize = tool_list()
            .iter()
            .map(|t| t["description"].to_string().len() + t["inputSchema"].to_string().len())
            .sum();
        assert!(read < 20_000, "{read} bytes");
    }

    #[test]
    fn numbers_are_kept_to_their_range_and_may_come_as_text() {
        let Call::ReadValue { limit, offset, .. } =
            call("read_value", json!({"limit": "5000", "offset": 3.0})).unwrap()
        else {
            panic!()
        };
        assert_eq!((limit, offset), (MAX_LIMIT, 3));
        assert!(call("read_value", json!({"limit": 0})).is_err());
        assert!(call("read_value", json!({"offset": -1})).is_err());
        assert!(call("read_value", json!({"offset": 1.5})).is_err());
    }

    #[test]
    fn an_unknown_argument_names_the_ones_there_are() {
        let error = call("search", json!({"text": "a", "query": "b"})).unwrap_err();
        assert!(
            error.contains("`query`") && error.contains("`text`"),
            "{error}"
        );
        let error = call("get_state", json!({"x": 1})).unwrap_err();
        assert!(error.contains("takes none"), "{error}");
    }

    #[test]
    fn a_pointer_must_be_one() {
        assert!(call("read_value", json!({"pointer": "users/0"})).is_err());
        assert!(call("read_value", json!({"pointer": "/users/0"})).is_ok());
    }

    #[test]
    fn languages_are_read_loosely() {
        for (text, kind) in [
            ("jq", Kind::Jq),
            ("JSONPath", Kind::JsonPath),
            ("json_pointer", Kind::JsonPointer),
            ("JMESPath", Kind::JmesPath),
        ] {
            let Call::RunQuery { language, .. } =
                call("run_query", json!({"query": ".", "language": text})).unwrap()
            else {
                panic!()
            };
            assert_eq!(language, Some(kind));
            assert_eq!(language_of(language_key(kind)), Some(kind));
        }
        assert!(call("run_query", json!({"query": ".", "language": "sql"})).is_err());
    }

    #[test]
    fn open_document_takes_a_path_or_text_not_both() {
        assert!(call("open_document", json!({})).is_err());
        assert!(call("open_document", json!({"path": "/a.json", "text": "[]"})).is_err());
        assert!(call("open_document", json!({"text": "[]"})).is_ok());
    }

    #[test]
    fn what_changes_the_app_or_reads_a_file_needs_edit() {
        let read = call("run_query", json!({"query": "."})).unwrap();
        let file = call(
            "run_query",
            json!({"query": ".", "input": {"file": "/a.json"}}),
        )
        .unwrap();
        assert_eq!(read.needs(), Access::Read);
        assert_eq!(file.needs(), Access::Edit);
        let info = call("document_info", json!({"input": {"file": "/a.json"}})).unwrap();
        assert_eq!(info.needs(), Access::Edit);
        for name in ["set_query", "reveal", "workspace_open"] {
            let args = json!({"query": ".", "pointer": "", "path": "a"});
            let args: Map<String, Value> = args
                .as_object()
                .unwrap()
                .iter()
                .filter(|(k, _)| {
                    tool_list().iter().find(|t| t["name"] == name).unwrap()["inputSchema"]
                        ["properties"]
                        .get(k.as_str())
                        .is_some()
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let parsed = call(name, Value::Object(args)).unwrap();
            assert_eq!(parsed.needs(), Access::Edit, "{name}");
            assert_eq!(parsed.tool(), name);
        }
    }

    #[test]
    fn the_annotations_say_which_tools_only_read() {
        let changing = [
            "open_document",
            "set_query",
            "reveal",
            "workspace_open",
            "show_in_tool",
            "workspace_save",
            "workspace_new_folder",
            "workspace_move",
            "workspace_delete",
            "save_file",
            "open_url",
        ];
        for tool in tool_list() {
            let name = tool["name"].as_str().unwrap();
            assert_eq!(
                tool["annotations"]["readOnlyHint"],
                !changing.contains(&name),
                "{name}"
            );
            let destroys = ["workspace_save", "workspace_delete", "save_file"].contains(&name);
            assert_eq!(tool["annotations"]["destructiveHint"], destroys, "{name}");
            assert_eq!(
                tool["annotations"]["openWorldHint"],
                name == "open_url",
                "{name}"
            );
        }
    }

    #[test]
    fn the_tools_that_look_take_any_document_out_of_sight() {
        let file = json!({"file": "/big.json"});
        for (name, args) in [
            ("describe", json!({"input": file})),
            ("read_value", json!({"input": file})),
            ("search", json!({"text": "a", "input": file})),
            ("run_query", json!({"query": ".", "input": file})),
            ("document_info", json!({"input": file})),
        ] {
            let parsed = call(name, args).unwrap();
            assert_eq!(parsed.needs(), Access::Edit, "{name}: a file needs Edit");
        }
        for (name, args) in [
            ("describe", json!({"input": "last"})),
            ("read_value", json!({"input": "results", "pointer": "/0"})),
            (
                "search",
                json!({"text": "a", "input": {"workspace": "q/a"}}),
            ),
            (
                "run_query",
                json!({"query": ".", "input": {"json": [1, 2]}}),
            ),
        ] {
            let parsed = call(name, args).unwrap();
            assert_eq!(parsed.needs(), Access::Read, "{name}");
        }
        let Call::Search { input, .. } = call("search", json!({"text": "a"})).unwrap() else {
            panic!()
        };
        assert_eq!(input, Input::Document);
        assert!(call("describe", json!({"target": "results"}))
            .unwrap_err()
            .contains("unknown"));
    }

    #[test]
    fn an_input_is_what_a_model_means_by_it() {
        let format = |input: Value| call("format_json", json!({ "input": input }));
        let input_of = |input: Value| match format(input) {
            Ok(Call::Format { input, .. }) => Ok(input),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e),
        };
        // The forms the schema says.
        assert_eq!(input_of(json!("document")), Ok(Input::Document));
        assert_eq!(input_of(json!("Results")), Ok(Input::Results));
        assert_eq!(input_of(json!("last")), Ok(Input::Last));
        assert_eq!(input_of(json!("the document")), Ok(Input::Document));
        assert_eq!(
            input_of(json!({"json": {"a": [1]}})),
            Ok(Input::Json(json!({"a": [1]})))
        );
        assert_eq!(
            input_of(json!({"text": "[1"})),
            Ok(Input::Text("[1".into()))
        );
        assert_eq!(
            input_of(json!({"file": "/data/a.json"})),
            Ok(Input::File("/data/a.json".into()))
        );
        assert_eq!(
            input_of(json!({"workspace": "/q//orders.json"})),
            Ok(Input::Workspace("q/orders.json".into()))
        );
        // What a model gives as often: the JSON itself, its text, a file's full path.
        assert_eq!(
            input_of(json!({"users": [{"name": "Ana"}]})),
            Ok(Input::Json(json!({"users": [{"name": "Ana"}]})))
        );
        assert_eq!(input_of(json!([1, 2])), Ok(Input::Json(json!([1, 2]))));
        assert_eq!(input_of(json!(7)), Ok(Input::Json(json!(7))));
        assert_eq!(
            input_of(json!({"json": 1, "text": "1"})),
            Ok(Input::Json(json!({"json": 1, "text": "1"})))
        );
        assert_eq!(
            input_of(json!("{\"a\": [1, 2]}")),
            Ok(Input::Json(json!({"a": [1, 2]})))
        );
        let orders = full_path("orders.JSON");
        assert_eq!(
            input_of(json!(orders)),
            Ok(Input::File(orders.as_str().into()))
        );
        assert_eq!(
            input_of(json!("~/data/orders.json")),
            Ok(Input::File("~/data/orders.json".into()))
        );
        // A path that is not a full one is not taken for a file.
        assert_eq!(
            input_of(json!("data/orders.json")),
            Ok(Input::Text("data/orders.json".into()))
        );
        // Text that is not JSON is given to the tool, which says what is wrong with it.
        assert_eq!(
            input_of(json!("not json, really")),
            Ok(Input::Text("not json, really".into()))
        );
        for wrong in [
            json!({"file": ""}),
            json!({"file": 3}),
            json!({"workspace": "../etc"}),
        ] {
            assert!(input_of(wrong.clone()).is_err(), "{wrong}");
        }
        let needed = call("format_json", json!({})).unwrap_err();
        assert!(needed.contains("`input` is needed"), "{needed}");
        assert!(needed.contains("For example: {\"input\""), "{needed}");
    }

    #[test]
    fn the_names_a_model_gives_arguments_are_the_tools() {
        let input_of = |args: Value| match call("format_json", args) {
            Ok(Call::Format { input, .. }) => input,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            input_of(json!({"json": {"a": 1}})),
            Input::Json(json!({"a": 1}))
        );
        assert_eq!(input_of(json!({"data": "[1]"})), Input::Json(json!([1])));
        assert_eq!(
            input_of(json!({"path": "/tmp/a.json"})),
            Input::File("/tmp/a.json".into())
        );
        // Arguments sent as JSON text are read.
        assert_eq!(
            input_of(json!("{\"input\": {\"b\": 2}}")),
            Input::Json(json!({"b": 2}))
        );
        // A name the tool has is its own: search's `text` is what is searched for.
        assert!(matches!(
            call("search", json!({"text": "Ana"})),
            Ok(Call::Search { .. })
        ));
        let Ok(Call::RunQuery { query, .. }) =
            call("run_query", json!({"filter": ".a", "lang": "jq"}))
        else {
            panic!()
        };
        assert_eq!(query, ".a");
        let Ok(Call::RunQuery { query, input, .. }) = call("run_query", json!({"text": ".b"}))
        else {
            panic!()
        };
        assert_eq!((query.as_str(), input), (".b", Input::Document));
        // What is not known is still said, with a call that works.
        let unknown = call("format_json", json!({"input": 1, "colour": "red"})).unwrap_err();
        assert!(unknown.contains("unknown argument `colour`"), "{unknown}");
        assert!(unknown.contains("For example:"), "{unknown}");
    }

    #[test]
    fn open_document_and_show_in_tool_take_what_a_model_gives_them() {
        let open = |args: Value| match call("open_document", args) {
            Ok(Call::OpenDocument { path, text, .. }) => (path, text),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            open(json!({"json": {"a": 1}})),
            (None, Some("{\"a\":1}".to_owned()))
        );
        assert_eq!(
            open(json!({"text": [1, 2]})),
            (None, Some("[1,2]".to_owned()))
        );
        let data = full_path("data.json");
        assert_eq!(
            open(json!({ "text": data })),
            (Some(data.as_str().into()), None)
        );
        assert_eq!(
            open(json!({"file": "/a/b.json"})),
            (Some("/a/b.json".into()), None)
        );
        let page = |tool: &str, inputs: Value| match call(
            "show_in_tool",
            json!({"tool": tool, "inputs": inputs}),
        ) {
            Ok(Call::ShowInTool { page, .. }) => page,
            other => panic!("{tool}: {other:?}"),
        };
        assert!(matches!(
            page("validate_json", json!([{"a": 1}, {"type": "object"}])),
            ToolPage::Validate { .. }
        ));
        assert!(matches!(
            page("Format JSON", json!([1])),
            ToolPage::Format(_)
        ));
        assert!(matches!(
            page("make schema", json!([1])),
            ToolPage::CreateSchema(_)
        ));
    }

    #[test]
    fn workspace_save_takes_what_the_name_and_the_content_say() {
        let save = |args: Value| match call("workspace_save", args) {
            Ok(Call::WorkspaceSave {
                folder,
                name,
                kind,
                content,
                ..
            }) => (folder, name, kind, content),
            other => panic!("{other:?}"),
        };
        // The screenshot's call: no kind, the JSON itself.
        let (folder, name, kind, content) =
            save(json!({"path": "examples/complex", "content": {"users": [1]}}));
        assert_eq!((folder.as_str(), name.as_str()), ("examples", "complex"));
        assert_eq!(kind, SaveKind::Document);
        assert_eq!(content, Input::Json(json!({"users": [1]})));
        // A name with its file's ending, given as `name` and `folder`.
        let (folder, name, kind, _) =
            save(json!({"folder": "q", "name": "late.jq", "content": ".orders[]"}));
        assert_eq!((folder.as_str(), name.as_str()), ("q", "late"));
        assert_eq!(kind, SaveKind::Query(Kind::Jq));
        let (_, name, kind, _) =
            save(json!({"path": "s/orders.schema.json", "content": {"type": "object"}}));
        assert_eq!((name.as_str(), kind), ("orders", SaveKind::Schema));
        // What the content is says it, without a kind or an ending.
        assert_eq!(
            save(json!({"path": "q/a", "content": ".a | b"})).2,
            SaveKind::Query(Kind::Jq)
        );
        assert_eq!(
            save(json!({"path": "q/a", "content": "$.a", "language": "jsonpath"})).2,
            SaveKind::Query(Kind::JsonPath)
        );
        assert_eq!(
            save(json!({"path": "w/feed", "content": "https://example.com/a.json"})).2,
            SaveKind::Address
        );
        assert_eq!(
            save(json!({"path": "s/x", "content": {"$schema": "https://json-schema.org/draft/2020-12/schema"}})).2,
            SaveKind::Schema
        );
        assert_eq!(
            save(json!({"path": "p/x", "content": [{"op": "add", "path": "/a", "value": 1}]})).2,
            SaveKind::Patch
        );
        assert_eq!(
            save(json!({"path": "d/x", "content": "{\"a\": 1}"})).2,
            SaveKind::Document
        );
        // A kind by another name; a query's text in an object.
        assert_eq!(
            save(json!({"path": "d/x", "kind": "json", "content": {}})).2,
            SaveKind::Document
        );
        assert_eq!(
            save(json!({"path": "q/x", "type": "jmespath", "content": "a.b"})).2,
            SaveKind::Query(Kind::JmesPath)
        );
        assert_eq!(
            save(json!({"path": "q/x", "kind": "query", "content": {"text": ".a"}})).3,
            Input::Text(".a".into())
        );
        let wrong = call(
            "workspace_save",
            json!({"path": "x", "kind": "banana", "content": 1}),
        )
        .unwrap_err();
        assert!(wrong.contains("`kind` is one of"), "{wrong}");
        assert!(wrong.contains("For example: {\"path\""), "{wrong}");
    }

    #[test]
    fn a_file_and_changing_what_is_kept_need_edit() {
        let file = json!({"file": "/a.json"});
        let level = |name: &str, args: Value| call(name, args).unwrap().needs();
        assert_eq!(
            level("format_json", json!({"input": "document"})),
            Access::Read
        );
        assert_eq!(level("format_json", json!({"input": file})), Access::Edit);
        assert_eq!(
            level("diff_documents", json!({"left": "document", "right": file})),
            Access::Edit
        );
        assert_eq!(
            level(
                "merge_documents",
                json!({"inputs": ["document", "results"]})
            ),
            Access::Read
        );
        assert_eq!(
            level(
                "show_in_tool",
                json!({"tool": "diff", "inputs": ["document", "results"]})
            ),
            Access::Edit
        );
        for (name, args) in [
            (
                "workspace_save",
                json!({"path": "a", "kind": "query", "content": "."}),
            ),
            ("workspace_new_folder", json!({"path": "a"})),
            ("workspace_move", json!({"path": "a", "to": ""})),
            ("workspace_delete", json!({"path": "a"})),
            (
                "save_file",
                json!({"input": "results", "path": "/tmp/a.json"}),
            ),
            ("open_url", json!({"url": "https://example.com/a.json"})),
        ] {
            assert_eq!(level(name, args), Access::Edit, "{name}");
        }
    }

    #[test]
    fn a_merge_is_a_preset_or_a_filter_not_both() {
        let filter = |args: Value| match call("merge_documents", args) {
            Ok(Call::Merge { filter, .. }) => Ok(filter),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e),
        };
        let two = json!(["document", "results"]);
        assert_eq!(filter(json!({"inputs": two})), Ok("add".to_owned()));
        assert_eq!(
            filter(json!({"inputs": two, "how": "deep"})).unwrap(),
            jsonquery_query::merge::PRESETS[2].filter
        );
        assert_eq!(
            filter(json!({"inputs": two, "filter": ".[0]"})),
            Ok(".[0]".to_owned())
        );
        assert!(filter(json!({"inputs": two, "how": "deep", "filter": "."})).is_err());
        assert!(filter(json!({"inputs": two, "how": "zip"})).is_err());
        assert!(filter(json!({"inputs": ["document"]})).is_err());
    }

    #[test]
    fn a_page_takes_as_many_inputs_as_it_has_boxes() {
        let show = |args: Value| call("show_in_tool", args);
        assert!(show(json!({"tool": "diff", "inputs": ["document"]}))
            .unwrap_err()
            .contains("left, right"));
        assert!(show(json!({"tool": "validate", "inputs": ["document", {"json": {}}]})).is_ok());
        assert!(show(json!({"tool": "merge", "inputs": ["document", "results"]})).is_err());
        assert!(show(
            json!({"tool": "merge", "inputs": [{"file": "/a.json"}, {"file": "/b.json"}], "options": {"how": "unique"}})
        )
        .is_ok());
        let Call::ShowInTool { page, .. } = show(
            json!({"tool": "Create Schema", "inputs": ["document"], "options": {"draft": "draft-07", "closed": true}}),
        )
        .unwrap() else {
            panic!()
        };
        let ToolPage::CreateSchema(options) = page else {
            panic!("{page:?}")
        };
        assert_eq!((options.draft, options.closed), (Draft::D7, true));
        let error =
            show(json!({"tool": "format", "inputs": ["document"], "options": {"draft": "7"}}))
                .unwrap_err();
        assert!(error.contains("unknown option `draft`"), "{error}");
        assert!(show(json!({"tool": "sort", "inputs": ["document"]})).is_err());
    }

    #[test]
    fn a_workspace_path_is_split_into_its_folder_and_name() {
        let Call::WorkspaceSave {
            folder, name, kind, content, ..
        } = call(
            "workspace_save",
            json!({"path": "/orders//late/", "kind": "query", "language": "jsonpath", "content": "$.a"}),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!((folder.as_str(), name.as_str()), ("orders", "late"));
        assert_eq!(kind, SaveKind::Query(Kind::JsonPath));
        assert_eq!(content, Input::Text("$.a".into()));
        let Call::WorkspaceSave {
            folder, content, ..
        } = call(
            "workspace_save",
            json!({"path": "top", "kind": "document", "content": "results"}),
        )
        .unwrap()
        else {
            panic!()
        };
        assert_eq!((folder.as_str(), content), ("", Input::Results));
        assert!(call(
            "workspace_save",
            json!({"path": "a/../b", "kind": "schema", "content": "document"})
        )
        .is_err());
        assert!(call(
            "workspace_save",
            json!({"path": "a", "kind": "document", "content": "document", "sample": "results"})
        )
        .is_err());
        assert!(call("workspace_move", json!({"path": "a"})).is_err());
        let Call::WorkspaceMove { to, name, .. } =
            call("workspace_move", json!({"path": "a/b", "to": "/"})).unwrap()
        else {
            panic!()
        };
        assert_eq!((to.as_deref(), name), (Some(""), None));
    }

    #[test]
    fn only_a_web_address_is_opened() {
        assert!(call("open_url", json!({"url": "https://example.com/a.json"})).is_ok());
        assert!(call("open_url", json!({"url": "HTTP://127.0.0.1:8000/a"})).is_ok());
        for wrong in [
            "file:///etc/passwd",
            "/home/a.json",
            "ftp://x/y",
            "javascript:alert(1)",
        ] {
            assert!(
                call("open_url", json!({ "url": wrong })).is_err(),
                "{wrong}"
            );
        }
    }

    #[test]
    fn indents_are_read_loosely() {
        let indent = |value: Value| match call(
            "save_file",
            json!({"input": "results", "path": "/a", "indent": value}),
        ) {
            Ok(Call::SaveFile { indent, .. }) => Ok(indent),
            Ok(other) => panic!("{other:?}"),
            Err(e) => Err(e),
        };
        assert_eq!(indent(json!(4)), Ok(Indent::Spaces(4)));
        assert_eq!(indent(json!("TAB")), Ok(Indent::Tab));
        assert_eq!(indent(json!("minified")), Ok(Indent::Minified));
        assert!(indent(json!(3)).is_err());
    }
}
