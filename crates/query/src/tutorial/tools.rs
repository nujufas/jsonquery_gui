//! The tutorial's Tools tab: a topic for each tool of the Tools window, whose
//! lessons have samples — the documents a page takes, in its boxes (or, for
//! Merge, as files) — and how the page is set. The window shows the samples and
//! what the tool makes of them, *live*: [`run`] runs the same code the page does
//! (`merge`, `reformat`, `diff`, `patch`, `schema`, `infer`), so a lesson can't
//! say something the tool no longer does. "▶ Try it" puts the samples in the
//! page and runs it there, to be changed and run again.

use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::diff::{self, ChangeKind};
use crate::infer::{self, Draft};
use crate::reformat::{self, Indent};
use crate::{merge, patch, schema};

/// A tool of the Tools window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Merge,
    Format,
    Diff,
    Patch,
    Validate,
    Create,
}

impl Tool {
    /// What the tab of its page says.
    pub fn label(self) -> &'static str {
        match self {
            Tool::Merge => "Merge JSON",
            Tool::Format => "Format JSON",
            Tool::Diff => "Diff JSON",
            Tool::Patch => "Patch JSON",
            Tool::Validate => "Validate Schema",
            Tool::Create => "Create Schema",
        }
    }

    /// The boxes of its page, in order, that an example's samples go in. Merge
    /// has none: its samples are files, named as they are.
    pub fn boxes(self) -> &'static [&'static str] {
        match self {
            Tool::Merge => &[],
            Tool::Format => &["Input"],
            Tool::Diff => &["Left", "Right"],
            Tool::Patch => &["Document", "Patch"],
            Tool::Validate => &["Document", "Schema"],
            Tool::Create => &["Document"],
        }
    }
}

/// How the page is set for an example: the tool, and its options.
#[derive(Clone, Copy, Debug)]
pub enum Setting {
    Merge {
        filter: &'static str,
    },
    Format(reformat::Options),
    Diff,
    /// `merge_patch`: a JSON Merge Patch (RFC 7386), not a list of operations.
    Patch {
        merge_patch: bool,
    },
    Validate {
        check_formats: bool,
    },
    Create(infer::Options),
}

impl Setting {
    pub fn tool(&self) -> Tool {
        match self {
            Setting::Merge { .. } => Tool::Merge,
            Setting::Format(_) => Tool::Format,
            Setting::Diff => Tool::Diff,
            Setting::Patch { .. } => Tool::Patch,
            Setting::Validate { .. } => Tool::Validate,
            Setting::Create(_) => Tool::Create,
        }
    }

    /// The options, as the page's command row shows them: "Indent 2 spaces ·
    /// Sort keys". Nothing for a page that has none.
    pub fn describe(&self) -> String {
        match self {
            Setting::Merge { filter } => format!("Filter: {filter}"),
            Setting::Format(options) => {
                let mut parts = vec![match options.indent {
                    Indent::Spaces(n) => format!("Indent {n} spaces"),
                    Indent::Tab => "Indent Tab".to_owned(),
                    Indent::Minified => "Minified".to_owned(),
                }];
                if options.sort_keys {
                    parts.push("Sort keys".to_owned());
                }
                if options.ascii_only {
                    parts.push("ASCII only".to_owned());
                }
                parts.join(" · ")
            }
            Setting::Diff => String::new(),
            Setting::Patch { merge_patch: false } => "Kind: Operations (RFC 6902)".to_owned(),
            Setting::Patch { merge_patch: true } => "Kind: Merge patch (RFC 7386)".to_owned(),
            Setting::Validate { check_formats } => {
                format!(
                    "Check formats: {}",
                    if *check_formats { "on" } else { "off" }
                )
            }
            Setting::Create(options) => {
                let mut parts = vec![options.draft.name().to_owned()];
                for (on, name) in [
                    (options.patterns, "Patterns"),
                    (options.required, "Required"),
                    (options.closed, "No other keys"),
                    (options.enums, "Enums"),
                    (options.bounds, "Bounds"),
                ] {
                    if on {
                        parts.push(name.to_owned());
                    }
                }
                parts.join(" · ")
            }
        }
    }
}

/// A document an example works on: the box it goes in (or, for Merge, the
/// file's name), and its text.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub name: &'static str,
    pub text: &'static str,
}

const fn sample(name: &'static str, text: &'static str) -> Sample {
    Sample { name, text }
}

/// One run of a tool: what it is for, how the page is set, its samples, and
/// what to look at in what comes out.
#[derive(Clone, Copy, Debug)]
pub struct ToolExample {
    /// One plain-English line: what this example does.
    pub caption: &'static str,
    pub setting: Setting,
    pub samples: &'static [Sample],
    /// What to look at in the result, a point each. Same markup as the
    /// lessons' prose.
    pub notes: &'static [&'static str],
    /// The example is meant to fail, to show what the error says.
    pub fails: bool,
}

impl ToolExample {
    const fn new(
        caption: &'static str,
        setting: Setting,
        samples: &'static [Sample],
        notes: &'static [&'static str],
    ) -> Self {
        Self {
            caption,
            setting,
            samples,
            notes,
            fails: false,
        }
    }

    const fn failing(mut self) -> Self {
        self.fails = true;
        self
    }

    pub fn tool(&self) -> Tool {
        self.setting.tool()
    }
}

/// A page of the Tools tab: a summary, the examples, and tips.
#[derive(Clone, Copy, Debug)]
pub struct ToolLesson {
    pub title: &'static str,
    pub summary: &'static str,
    pub examples: &'static [ToolExample],
    pub tips: &'static [&'static str],
}

impl ToolLesson {
    const fn new(
        title: &'static str,
        summary: &'static str,
        examples: &'static [ToolExample],
    ) -> Self {
        Self {
            title,
            summary,
            examples,
            tips: &[],
        }
    }

    const fn tips(mut self, tips: &'static [&'static str]) -> Self {
        self.tips = tips;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToolTopic {
    pub title: &'static str,
    pub lessons: &'static [ToolLesson],
}

/// What the tab says under its title.
pub fn tagline() -> &'static str {
    "Merge, format, diff, patch, validate and create schemas — each lesson's samples go into the \
     tool's page with ▶ Try it."
}

/// Whether `lesson` mentions `needle` (already lower-cased) where a reader
/// might look for it: title, summary, captions, settings and samples' names.
pub fn lesson_matches(lesson: &ToolLesson, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let hit = |s: &str| s.to_lowercase().contains(needle);
    hit(lesson.title)
        || hit(lesson.summary)
        || lesson.examples.iter().any(|e| {
            hit(e.caption)
                || hit(e.tool().label())
                || hit(&e.setting.describe())
                || e.samples.iter().any(|s| hit(s.name))
        })
}

/// What the tool made of an example's samples.
#[derive(Debug, Default, Clone)]
pub struct ToolOutcome {
    /// What it is, in a few words: "3 differences", "Valid".
    pub summary: String,
    /// What it made, as text: a document, a list of differences or problems.
    pub text: String,
    /// The tool failed (on purpose, for an example that `fails`).
    pub error: Option<String>,
}

/// Run `example`'s tool on its samples, as its page would.
pub fn run(example: &ToolExample) -> ToolOutcome {
    let mut values = Vec::new();
    for sample in example.samples {
        match serde_json::from_str::<Value>(sample.text) {
            Ok(value) => values.push(value),
            Err(e) => {
                return ToolOutcome {
                    error: Some(format!("{}: {e}", sample.name)),
                    ..ToolOutcome::default()
                }
            }
        }
    }
    let result = match example.setting {
        Setting::Merge { filter } => run_merge(example, values, filter),
        Setting::Format(options) => Ok(run_format(&values, options)),
        Setting::Diff => run_diff(&values),
        Setting::Patch { merge_patch } => run_patch(values, merge_patch),
        Setting::Validate { check_formats } => run_validate(&values, check_formats),
        Setting::Create(options) => run_create(&values, options),
    };
    result.unwrap_or_else(|error| ToolOutcome {
        error: Some(error),
        ..ToolOutcome::default()
    })
}

fn pretty(value: &Value) -> String {
    reformat::render(value, &reformat::Options::default())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

fn first(values: &[Value], at: usize) -> Result<&Value, String> {
    values
        .get(at)
        .ok_or_else(|| "the example has too few samples".to_owned())
}

fn run_merge(
    example: &ToolExample,
    values: Vec<Value>,
    filter: &str,
) -> Result<ToolOutcome, String> {
    let names: Vec<String> = example.samples.iter().map(|s| s.name.to_owned()).collect();
    let files = values.len();
    let merged =
        merge::merge(values, &names, filter, &AtomicBool::new(false)).map_err(|e| e.to_string())?;
    Ok(ToolOutcome {
        summary: format!("merged from {}", plural(files, "file", "files")),
        text: pretty(&merged.value),
        error: None,
    })
}

fn run_format(values: &[Value], options: reformat::Options) -> ToolOutcome {
    let text = values
        .first()
        .map_or_else(String::new, |v| reformat::render(v, &options));
    ToolOutcome {
        summary: plural(text.lines().count(), "line", "lines"),
        text,
        error: None,
    }
}

fn run_diff(values: &[Value]) -> Result<ToolOutcome, String> {
    let (left, right) = (first(values, 0)?, first(values, 1)?);
    let found = diff::diff(left, right, &AtomicBool::new(false)).map_err(|e| e.to_string())?;
    if found.is_empty() {
        return Ok(ToolOutcome {
            summary: "the same".to_owned(),
            text: "The documents are the same.".to_owned(),
            error: None,
        });
    }
    let mut text = String::new();
    for change in &found.changes {
        let what = match change.kind {
            ChangeKind::Added => "added  ",
            ChangeKind::Removed => "removed",
            ChangeKind::Changed => "changed",
        };
        let path = if change.path.is_empty() {
            "(document)"
        } else {
            &change.path
        };
        let values = match (&change.before, &change.after) {
            (Some(before), Some(after)) => format!("{before} → {after}"),
            (Some(before), None) => before.clone(),
            (None, Some(after)) => after.clone(),
            (None, None) => String::new(),
        };
        text.push_str(&format!("{what} {path}  {values}\n"));
    }
    text.push_str("\nPatch (RFC 6902), Left to Right:\n");
    for operation in &found.operations {
        text.push_str(&serde_json::to_string(operation).unwrap_or_default());
        text.push('\n');
    }
    let mut counts = Vec::new();
    for (n, what) in [
        (found.added, "added"),
        (found.removed, "removed"),
        (found.changed, "changed"),
    ] {
        if n > 0 {
            counts.push(format!("{n} {what}"));
        }
    }
    Ok(ToolOutcome {
        summary: counts.join(" · "),
        text: text.trim_end().to_owned(),
        error: None,
    })
}

fn run_patch(values: Vec<Value>, merge_patch: bool) -> Result<ToolOutcome, String> {
    let mut values = values.into_iter();
    let (Some(document), Some(patch_value)) = (values.next(), values.next()) else {
        return Err("the example has too few samples".to_owned());
    };
    let (patched, summary) = if merge_patch {
        (
            patch::apply_merge(document, &patch_value),
            "merge patch".to_owned(),
        )
    } else {
        let count = patch_value.as_array().map_or(0, Vec::len);
        let patched = patch::apply(document, &patch_value).map_err(|e| e.to_string())?;
        (patched, plural(count, "operation", "operations"))
    };
    Ok(ToolOutcome {
        summary,
        text: pretty(&patched),
        error: None,
    })
}

fn run_validate(values: &[Value], check_formats: bool) -> Result<ToolOutcome, String> {
    let (document, schema_value) = (first(values, 0)?, first(values, 1)?);
    let report = schema::validate(
        schema_value,
        document,
        schema::Options { check_formats },
        &AtomicBool::new(false),
    )
    .map_err(|e| format!("The schema can't be used: {e}"))?;
    if report.is_valid() {
        return Ok(ToolOutcome {
            summary: "valid".to_owned(),
            text: format!("The document is valid ({})", report.draft),
            error: None,
        });
    }
    let text: Vec<String> = report
        .problems
        .iter()
        .map(|p| {
            let path = if p.path.is_empty() {
                "(document)"
            } else {
                &p.path
            };
            format!("{path}: {} [{}]", p.message, p.keyword)
        })
        .collect();
    Ok(ToolOutcome {
        summary: plural(report.problems.len(), "problem", "problems"),
        text: text.join("\n"),
        error: None,
    })
}

fn run_create(values: &[Value], options: infer::Options) -> Result<ToolOutcome, String> {
    let document = first(values, 0)?;
    let created =
        infer::create(document, options, &AtomicBool::new(false)).map_err(|e| e.to_string())?;
    let mut text = pretty(&created.schema);
    let (enums, patterns): (Vec<&infer::Found>, Vec<&infer::Found>) =
        created.found.iter().partition(|f| f.is_enum());
    if !patterns.is_empty() {
        text.push_str("\n\nPatterns found:\n");
        for found in patterns.iter().copied() {
            let format = found
                .format
                .as_ref()
                .map_or(String::new(), |f| format!(" · format {f}"));
            text.push_str(&format!(
                "{}  {}{format}\n    {}\n",
                found.path, found.what, found.expression
            ));
        }
    }
    let mut summary = format!(
        "{} · {}",
        plural(created.properties, "property", "properties"),
        plural(patterns.len(), "pattern", "patterns")
    );
    if !enums.is_empty() {
        summary.push_str(&format!(" · {}", plural(enums.len(), "enum", "enums")));
    }
    Ok(ToolOutcome {
        summary,
        text: text.trim_end().to_owned(),
        error: None,
    })
}

// ---------------------------------------------------------------------------
// The lessons.
// ---------------------------------------------------------------------------

const PRETTY: reformat::Options = reformat::Options {
    indent: Indent::Spaces(2),
    sort_keys: false,
    ascii_only: false,
};

const CREATE: infer::Options = infer::Options {
    draft: Draft::D202012,
    required: true,
    closed: false,
    patterns: true,
    enums: false,
    bounds: false,
};

const ORDERS_JAN: &str = r#"[
  { "id": 101, "customer": "Ada", "total": 25.5 },
  { "id": 102, "customer": "Linus", "total": 12 }
]"#;

const ORDERS_FEB: &str = r#"[
  { "id": 103, "customer": "Grace", "total": 40.25 }
]"#;

const DEFAULTS: &str = r#"{
  "server": { "host": "0.0.0.0", "port": 8080, "tls": false },
  "log": { "level": "info", "format": "text" },
  "features": ["search"]
}"#;

const PRODUCTION: &str = r#"{
  "server": { "tls": true },
  "log": { "level": "warn" },
  "features": ["search", "export"]
}"#;

const LOCAL: &str = r#"{
  "server": { "port": 3000 },
  "log": { "format": "pretty" }
}"#;

const BERLIN: &str = r#"[
  { "sku": "A-1", "qty": 5 },
  { "sku": "B-2", "qty": 1 }
]"#;

const PARIS: &str = r#"[
  { "sku": "A-1", "qty": 2 },
  { "sku": "C-3", "qty": 7 }
]"#;

/// Version 1 and 2 of a service's settings, for Diff.
const SERVICE_V1: &str = r#"{
  "name": "orders-api",
  "version": "1.4.0",
  "replicas": 2,
  "env": { "LOG_LEVEL": "info", "DEBUG": true },
  "ports": [8080, 9090]
}"#;

const SERVICE_V2: &str = r#"{
  "name": "orders-api",
  "version": "1.5.0",
  "replicas": 3,
  "env": { "LOG_LEVEL": "warn" },
  "ports": [8080, 9090, 9100],
  "owner": "platform"
}"#;

/// The document the patches of Patch JSON work on.
const SERVICE: &str = r#"{
  "name": "orders-api",
  "version": "1.4.0",
  "debug": true,
  "tags": ["api", "beta"],
  "owner": { "team": "platform" }
}"#;

const PERSON_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "properties": {
    "name": { "type": "string", "minLength": 1 },
    "email": { "type": "string", "format": "email" },
    "age": { "type": "integer", "minimum": 0 },
    "role": { "enum": ["lead", "dev", "qa"] },
    "joined": { "type": "string", "format": "date" }
  },
  "required": ["name", "email"],
  "additionalProperties": false
}"#;

const ADDRESS_SCHEMA: &str = r##"{
  "type": "object",
  "properties": {
    "billing": { "$ref": "#/$defs/address" },
    "shipping": { "$ref": "#/$defs/address" }
  },
  "required": ["billing"],
  "$defs": {
    "address": {
      "type": "object",
      "properties": {
        "street": { "type": "string" },
        "city": { "type": "string" },
        "zip": { "type": "string", "pattern": "^[0-9]{5}$" }
      },
      "required": ["street", "city", "zip"]
    }
  }
}"##;

const USERS: &str = r#"[
  { "name": "Ada", "age": 36, "admin": true, "email": "ada@example.com",
    "skills": ["rust", "sql"] },
  { "name": "Linus", "age": 28, "admin": false, "email": null,
    "skills": ["c"] },
  { "name": "Grace", "age": 45.5, "admin": false, "skills": [] }
]"#;

const ORDERS: &str = r#"[
  {
    "id": "123e4567-e89b-12d3-a456-426614174000",
    "number": "ORD-2024-00017",
    "placed": "2024-01-15T10:30:00Z",
    "email": "ada@example.com",
    "ip": "192.168.0.10",
    "site": "https://shop.example.com/orders/17",
    "zip": "90210",
    "total": "25.50"
  },
  {
    "id": "c56a4180-65aa-42ec-a945-5fd21dec0538",
    "number": "ORD-2024-00230",
    "placed": "2024-02-01T08:00:00+01:00",
    "email": "linus@example.org",
    "ip": "10.0.0.1",
    "site": "https://shop.example.com/orders/230",
    "zip": "10001",
    "total": "12.00"
  }
]"#;

const CUSTOMER: &str = r#"{
  "name": "Ada",
  "tags": "gift,express,priority",
  "servers": "10.0.0.1; 10.0.0.2",
  "phone1": "+44 20 7946 0958",
  "phone2": "+44 20 7946 0959",
  "visits": { "2024-01-15": 3, "2024-01-16": 5, "2024-01-20": 1 }
}"#;

const TICKETS: &str = r#"[
  { "status": "open", "priority": 2, "title": "Login fails" },
  { "status": "closed", "priority": 1, "title": "Typo on the home page" },
  { "status": "open", "priority": 3, "title": "Export is slow" },
  { "status": "waiting", "priority": 2, "title": "Add dark mode" },
  { "status": "open", "priority": 5, "title": "Crash on start" }
]"#;

pub static TOPICS: &[ToolTopic] = &[
    ToolTopic {
        title: "Getting started",
        lessons: &[ToolLesson::new(
            "The Tools window",
            "Six small tools that go with the viewer, each a page: what goes in on the left, \
             what comes out on the right, the main button and the options in the row above. \
             Every lesson here has samples to study — and to try: ▶ Try it puts them in the page \
             and runs it, so you can change them and run it again.",
            &[],
        )
        .tips(&[
            "The 🛠 button beside the 📖 opens the Tools in a window of their own; the ⚓ at the \
             right of their row of tabs docks them in the main window, as tabs beside *Json \
             Query*, and the 🗗 in the status bar undocks them.",
            "A box takes JSON typed or pasted in, a file dropped on the window, *Open file…*, \
             or *Open document* — the document open in the main window.",
            "`Ctrl+Enter` runs the page's main button; *Cancel* stops a job that takes long.",
            "Text that is not JSON is reported with the box and the line: `Left: expected value \
             at line 3 column 1`.",
            "The tools work in memory, on up to 128 MB at a time (a setting, under ⚙).",
        ])],
    },
    ToolTopic {
        title: "Merge JSON",
        lessons: &[
            ToolLesson::new(
                "Append files",
                "Merge reads the files of its list, in that order, into one array — `.` in the \
                 filter — and the filter makes one document of it, as `jq -s` does. The \
                 ready-made filters are in *Merge as*; `add` joins arrays end to end.",
                &[
                    ToolExample::new(
                        "Two months of orders, one list",
                        Setting::Merge { filter: "add" },
                        &[
                            sample("orders-jan.json", ORDERS_JAN),
                            sample("orders-feb.json", ORDERS_FEB),
                        ],
                        &[
                            "`.` is `[jan, feb]`: an array of the two files, in the order of \
                             the list.",
                            "`add` joins them end to end: three orders, January's first.",
                            "Move a file with ⏶ ⏷ and the order of the result follows.",
                        ],
                    ),
                    ToolExample::new(
                        "Tags of two files, each once",
                        Setting::Merge {
                            filter: "add | unique",
                        },
                        &[
                            sample("tags-a.json", r#"["rust", "sql", "go"]"#),
                            sample("tags-b.json", r#"["sql", "python", "rust"]"#),
                        ],
                        &["`unique` takes out what is there twice — and sorts what is left."],
                    ),
                ],
            ),
            ToolLesson::new(
                "Deep-merge settings",
                "Objects can be merged key by key, all the way down: settings for every place, \
                 then what one place changes, then what one machine changes. jq's `*` merges two \
                 objects; `reduce` lays each file over the ones before it.",
                &[ToolExample::new(
                    "Defaults, production, and a local change",
                    Setting::Merge {
                        filter: "reduce .[] as $file ({}; . * $file)",
                    },
                    &[
                        sample("defaults.json", DEFAULTS),
                        sample("production.json", PRODUCTION),
                        sample("local.json", LOCAL),
                    ],
                    &[
                        "`server` keeps `host` from the defaults, takes `tls` from production \
                         and `port` from local: a later file wins a clash.",
                        "Arrays are not merged: production's `features` takes the place of the \
                         defaults'.",
                        "A key no later file has keeps its value: `log.format` is local's, \
                         `log.level` production's.",
                    ],
                )],
            )
            .tips(&[
                "The order of the list is the order of the layers: put the general file first.",
            ]),
            ToolLesson::new(
                "Your own filter",
                "Any jq filter can merge: write it in the box under the command row. `.` is the \
                 array of the files and `$files` the array of their names.",
                &[
                    ToolExample::new(
                        "Stock of two warehouses, summed by product",
                        Setting::Merge {
                            filter: "add | group_by(.sku) | map({sku: .[0].sku, qty: (map(.qty) | add)})",
                        },
                        &[sample("berlin.json", BERLIN), sample("paris.json", PARIS)],
                        &[
                            "`add` makes one list of both; `group_by(.sku)` puts the rows of a \
                             product together.",
                            "`map({…})` makes one row of each group, its `qty` the sum of the \
                             group's.",
                        ],
                    ),
                    ToolExample::new(
                        "Which file each row came from",
                        Setting::Merge {
                            filter: "[range(0; length) as $i | .[$i][] | . + {from: $files[$i]}]",
                        },
                        &[sample("berlin.json", BERLIN), sample("paris.json", PARIS)],
                        &[
                            "`$files[$i]` is the name of the file `.[$i]` is — so each row says \
                             where it came from.",
                        ],
                    ),
                ],
            ),
        ],
    },
    ToolTopic {
        title: "Format JSON",
        lessons: &[
            ToolLesson::new(
                "Pretty-print and minify",
                "Format writes a document again in the layout you pick: two or four spaces or a \
                 tab to a level, or minified, with no space at all.",
                &[
                    ToolExample::new(
                        "A minified document, indented",
                        Setting::Format(PRETTY),
                        &[sample(
                            "Input",
                            r#"{"name":"Ada","langs":["rust","sql"],"score":1.50,"address":{"city":"London","zip":"N1 9GU"}}"#,
                        )],
                        &[
                            "Each level is two spaces in; *4 spaces* and *Tab* are in the \
                             *Indent* list.",
                            "Numbers are written as they were read: `1.50` stays `1.50`.",
                        ],
                    ),
                    ToolExample::new(
                        "An indented document, minified",
                        Setting::Format(reformat::Options {
                            indent: Indent::Minified,
                            sort_keys: false,
                            ascii_only: false,
                        }),
                        &[sample("Input", DEFAULTS)],
                        &["The smallest it can be: one line, no spaces between the tokens."],
                    ),
                ],
            ),
            ToolLesson::new(
                "Sort keys and ASCII only",
                "*Sort keys* puts the members of every object in order of their keys, at every \
                 depth; *ASCII only* writes every character outside ASCII as a `\\u` escape.",
                &[ToolExample::new(
                    "Keys in order, and only ASCII",
                    Setting::Format(reformat::Options {
                        indent: Indent::Spaces(2),
                        sort_keys: true,
                        ascii_only: true,
                    }),
                    &[sample(
                        "Input",
                        r#"{"zeta": 1, "city": "Zürich", "alpha": {"delta": 2, "beta": "naïve"}}"#,
                    )],
                    &[
                        "`alpha` comes first, and inside it `beta` before `delta`; arrays keep \
                         their order.",
                        "`ü` is `\\u00fc`: the text means the same, and passes through anything \
                         that only carries ASCII.",
                    ],
                )],
            )
            .tips(&[
                "Sorted keys make two documents easier to compare by eye — Diff JSON compares \
                 them whatever the order.",
            ]),
        ],
    },
    ToolTopic {
        title: "Diff JSON",
        lessons: &[
            ToolLesson::new(
                "Two versions of a document",
                "Diff compares Left with Right: what was added, removed and changed, and where — \
                 shown side by side, listed, and as a JSON Patch that turns Left into Right.",
                &[ToolExample::new(
                    "Version 1.4 and 1.5 of a service's settings",
                    Setting::Diff,
                    &[sample("Left", SERVICE_V1), sample("Right", SERVICE_V2)],
                    &[
                        "Each difference has the JSON Pointer of where it is: `/env/DEBUG` is \
                         the `DEBUG` in `env`.",
                        "On the page, *Side by side* marks the lines; the arrows between the \
                         two move a difference from one side to the other.",
                        "The patch, put in Patch JSON with Left as the document, gives Right.",
                    ],
                )],
            ),
            ToolLesson::new(
                "Arrays and the order of keys",
                "Arrays are compared element by element, in order, and an element put in or \
                 taken out is that — not everything after it changed. The order of an object's \
                 keys is not a difference.",
                &[
                    ToolExample::new(
                        "A step put in the middle",
                        Setting::Diff,
                        &[
                            sample("Left", r#"{"steps": ["build", "test", "deploy"]}"#),
                            sample("Right", r#"{"steps": ["build", "lint", "test", "deploy"]}"#),
                        ],
                        &["One addition at `/steps/1`: `test` and `deploy` are where they were."],
                    ),
                    ToolExample::new(
                        "The same members, in another order",
                        Setting::Diff,
                        &[
                            sample("Left", r#"{"a": 1, "b": [1, 2], "c": {"x": true}}"#),
                            sample("Right", r#"{"c": {"x": true}, "b": [1, 2], "a": 1}"#),
                        ],
                        &["An object is its members, not their order: the two are the same."],
                    ),
                ],
            )
            .tips(&[
                "A key an object has more than once is valid JSON: Diff compares its members \
                 one by one, `/name#2` being the second.",
            ]),
        ],
    },
    ToolTopic {
        title: "Patch JSON",
        lessons: &[
            ToolLesson::new(
                "Operations (RFC 6902)",
                "A JSON Patch is a list of operations, run in order: `add`, `remove`, \
                 `replace`, `move`, `copy` and `test`. Each names a place by its JSON Pointer.",
                &[ToolExample::new(
                    "Release a new version",
                    Setting::Patch { merge_patch: false },
                    &[
                        sample("Document", SERVICE),
                        sample(
                            "Patch",
                            r#"[
  { "op": "test", "path": "/version", "value": "1.4.0" },
  { "op": "replace", "path": "/version", "value": "1.5.0" },
  { "op": "remove", "path": "/debug" },
  { "op": "add", "path": "/tags/-", "value": "stable" },
  { "op": "remove", "path": "/tags/1" },
  { "op": "copy", "from": "/owner/team", "path": "/team" },
  { "op": "move", "from": "/owner", "path": "/maintainer" }
]"#,
                        ),
                    ],
                    &[
                        "`test` checks the version is 1.4.0 before anything is changed.",
                        "`/tags/-` is the end of the array: `stable` is added last, then \
                         `/tags/1` (`beta`) is removed.",
                        "`copy` and `move` take `from` a place: `owner` is renamed \
                         `maintainer`, and its `team` is copied to the top.",
                    ],
                )],
            ),
            ToolLesson::new(
                "All or nothing",
                "When an operation fails — a place that is not there, a `test` that does not \
                 hold — the patch stops, and the document is left as it was: the operations \
                 before it are undone.",
                &[ToolExample::new(
                    "A test that does not hold",
                    Setting::Patch { merge_patch: false },
                    &[
                        sample("Document", SERVICE),
                        sample(
                            "Patch",
                            r#"[
  { "op": "replace", "path": "/version", "value": "2.0.0" },
  { "op": "test", "path": "/tags/0", "value": "web" }
]"#,
                        ),
                    ],
                    &[
                        "The error names the operation, by its number and its place.",
                        "The `replace` had worked, and is not kept.",
                    ],
                )
                .failing()],
            ),
            ToolLesson::new(
                "Merge patch (RFC 7386)",
                "A merge patch is a document laid over the other: its members replace or add, \
                 objects are merged member by member, and `null` deletes.",
                &[ToolExample::new(
                    "Settings changed with a merge patch",
                    Setting::Patch { merge_patch: true },
                    &[
                        sample("Document", SERVICE),
                        sample(
                            "Patch",
                            r#"{
  "version": "1.5.0",
  "debug": null,
  "tags": ["api"],
  "owner": { "email": "platform@example.com" }
}"#,
                        ),
                    ],
                    &[
                        "`debug: null` deletes `debug`.",
                        "`owner` is merged: it keeps `team` and gains `email`.",
                        "An array is replaced whole: `tags` is just `[\"api\"]`.",
                    ],
                )],
            )
            .tips(&[
                "A merge patch cannot set a value to `null`, nor change one element of an array: \
                 that takes the operations of RFC 6902.",
            ]),
        ],
    },
    ToolTopic {
        title: "Validate Schema",
        lessons: &[
            ToolLesson::new(
                "A document that fits",
                "A JSON Schema says what a document may be: the types of its values, the members \
                 its objects must have, the formats of its strings. Validate checks a document \
                 against one.",
                &[ToolExample::new(
                    "A person, as the schema says",
                    Setting::Validate {
                        check_formats: true,
                    },
                    &[
                        sample(
                            "Document",
                            r#"{
  "name": "Ada",
  "email": "ada@example.com",
  "age": 36,
  "role": "lead",
  "joined": "2021-03-01"
}"#,
                        ),
                        sample("Schema", PERSON_SCHEMA),
                    ],
                    &[
                        "`required` lists the members it must have; `additionalProperties: false` \
                         allows no others.",
                        "`format` holds a string to being an email, a date… when *Check formats* \
                         is on.",
                    ],
                )],
            ),
            ToolLesson::new(
                "What does not fit",
                "Every problem is listed with where it is in the document — a JSON Pointer — \
                 what is wrong, and the keyword of the schema that said so.",
                &[ToolExample::new(
                    "A person with six things wrong",
                    Setting::Validate {
                        check_formats: true,
                    },
                    &[
                        sample(
                            "Document",
                            r#"{
  "email": "ada(at)example.com",
  "age": -1,
  "role": "boss",
  "joined": "2021-02-30",
  "nickname": "A"
}"#,
                        ),
                        sample("Schema", PERSON_SCHEMA),
                    ],
                    &[
                        "`(document)` is the document itself: it lacks `name`.",
                        "The 30th of February is no date, and `boss` is not one of the roles.",
                        "A click on a problem highlights it in the document, and the keyword in \
                         the schema.",
                        "With the document open in the main window, a double click on a problem \
                         shows the value there.",
                    ],
                )],
            ),
            ToolLesson::new(
                "References with $defs",
                "A schema can name a part once, under `$defs`, and use it where it is needed \
                 with `$ref`. A reference has to point inside the schema: nothing is fetched.",
                &[ToolExample::new(
                    "Two addresses, one definition",
                    Setting::Validate {
                        check_formats: true,
                    },
                    &[
                        sample(
                            "Document",
                            r#"{
  "billing": { "street": "1 Main St", "city": "Springfield", "zip": "12345" },
  "shipping": { "street": "9 Side Rd", "zip": "1234" }
}"#,
                        ),
                        sample("Schema", ADDRESS_SCHEMA),
                    ],
                    &[
                        "Both addresses are held to `#/$defs/address`; only `shipping` breaks \
                         it — no `city`, and a `zip` of four digits.",
                    ],
                )],
            ),
        ],
    },
    ToolTopic {
        title: "Create Schema",
        lessons: &[
            ToolLesson::new(
                "From a document to a schema",
                "Create Schema writes the schema of a document: the types of its values, the \
                 members of its objects and which of them every one has. The elements of an \
                 array are one place: a list of records makes one schema of a record.",
                &[ToolExample::new(
                    "Three users, one schema of a user",
                    Setting::Create(CREATE),
                    &[sample("Document", USERS)],
                    &[
                        "`email` is not required: Grace has none. It is a string or `null`.",
                        "`age` is a `number`: 45.5 is among the whole numbers.",
                        "`skills` is an array of strings, its elements from all three users.",
                    ],
                )],
            ),
            ToolLesson::new(
                "Patterns in strings",
                "What every string at a place is — a date, a uuid, an address, a code of one \
                 shape — is said as a `format`, where JSON Schema has one, and as a `pattern`, \
                 the regular expression they match. The list under the schema shows them all.",
                &[ToolExample::new(
                    "Orders with ids, dates, addresses and codes",
                    Setting::Create(CREATE),
                    &[sample("Document", ORDERS)],
                    &[
                        "`id`, `placed`, `email`, `ip` and `site` have their formats: `uuid`, \
                         `date-time`, `email`, `ipv4`, `uri`.",
                        "`ORD-2024-00017` and `ORD-2024-00230` have one shape: the letters both \
                         have are written as they are, the widths they agree on as `{4}` and \
                         `{5}`.",
                        "`zip` and `total` are numbers written as text: five digits, and two \
                         decimals.",
                        "The schema is checked against the document before it is shown: the \
                         status bar says *The document fits it*.",
                    ],
                )],
            )
            .tips(&[
                "A click on a pattern in the list shows where it is, in the document and in \
                 the schema (⏶ and ⏷ step through the document), and opens it: write \
                 another expression and *Apply*, make it a *Plain string*, or *Restore* \
                 what was found.",
                "The schema is in a box: type in it too. *Reset* undoes every change.",
            ]),
            ToolLesson::new(
                "Lists, maps and numbered keys",
                "A string can hold a list — items of one kind with a comma, a semicolon or a bar \
                 between — and the pattern says so. Keys can be data too: an object whose keys \
                 are all dates, ids or numbers is a map, and `phone1`, `phone2` are one key with \
                 a number.",
                &[ToolExample::new(
                    "A customer with lists, a map and two phones",
                    Setting::Create(CREATE),
                    &[sample("Document", CUSTOMER)],
                    &[
                        "`tags` is words with commas between; `servers` IPv4 addresses with `; ` \
                         between.",
                        "`visits` is keyed by dates: one schema for its values, under \
                         `patternProperties`.",
                        "`phone1` and `phone2` are `^phone[0-9]+$`: a `phone3` would fit too.",
                    ],
                )],
            ),
            ToolLesson::new(
                "Enums and bounds",
                "Two options say more of the values: *Enums* makes a list of a few strings that \
                 come again and again, and *Bounds* says the least and the most that was seen.",
                &[ToolExample::new(
                    "Tickets, with enums and bounds",
                    Setting::Create(infer::Options {
                        enums: true,
                        bounds: true,
                        ..CREATE
                    }),
                    &[sample("Document", TICKETS)],
                    &[
                        "`status` is one of `open`, `closed`, `waiting`; `title` is no enum — \
                         every title is different.",
                        "`priority` is from 1 to 5, the titles from 11 to 21 characters long, \
                         and the array has 5 items.",
                        "Bounds fit the sample exactly: loosen them by hand before the schema \
                         checks other documents.",
                    ],
                )],
            )
            .tips(&[
                "An enum is in the list of patterns too. A click opens its values: untick one to \
                 leave it out, write another and *Add* it, or make it a *Plain string*.",
                "*Use in Validate* puts the document and the schema in Validate Schema and checks \
                 it there; *Save…* writes the schema to a file.",
            ]),
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn every_example() -> Vec<(String, &'static ToolExample)> {
        let mut out = Vec::new();
        for topic in TOPICS {
            for lesson in topic.lessons {
                for example in lesson.examples {
                    out.push((format!("{} › {}", topic.title, lesson.title), example));
                }
            }
        }
        out
    }

    #[test]
    fn every_example_runs_as_declared() {
        for (at, example) in every_example() {
            let outcome = run(example);
            if example.fails {
                assert!(
                    outcome.error.is_some(),
                    "{at}: `{}` is marked failing but succeeded",
                    example.caption
                );
            } else {
                assert!(
                    outcome.error.is_none(),
                    "{at}: `{}` failed: {:?}",
                    example.caption,
                    outcome.error
                );
                assert!(!outcome.text.is_empty(), "{at}: nothing came out");
            }
        }
    }

    #[test]
    fn every_sample_is_json_and_goes_in_a_box_of_its_page() {
        for (at, example) in every_example() {
            for sample in example.samples {
                serde_json::from_str::<Value>(sample.text)
                    .unwrap_or_else(|e| panic!("{at}: {} is not JSON: {e}", sample.name));
            }
            let names: Vec<&str> = example.samples.iter().map(|s| s.name).collect();
            match example.tool() {
                Tool::Merge => {
                    assert!(!names.is_empty(), "{at}: nothing to merge");
                    for (i, name) in names.iter().enumerate() {
                        assert!(name.ends_with(".json"), "{at}: {name} is a file's name");
                        assert!(!names[..i].contains(name), "{at}: {name} twice");
                        assert!(
                            !name.contains(['/', '\\']),
                            "{at}: {name} is a name, not a path"
                        );
                    }
                }
                tool => assert_eq!(names, tool.boxes(), "{at}"),
            }
        }
    }

    #[test]
    fn every_tool_has_a_topic_with_lessons_and_titles_are_unique() {
        let tools: Vec<Tool> = every_example().iter().map(|(_, e)| e.tool()).collect();
        for tool in [
            Tool::Merge,
            Tool::Format,
            Tool::Diff,
            Tool::Patch,
            Tool::Validate,
            Tool::Create,
        ] {
            assert!(tools.contains(&tool), "{tool:?} has no example");
            assert!(
                TOPICS.iter().any(|t| t.title == tool.label()),
                "{tool:?} has no topic of its name"
            );
        }
        let mut seen = std::collections::HashSet::new();
        for topic in TOPICS {
            assert!(!topic.lessons.is_empty(), "{}: empty topic", topic.title);
            for lesson in topic.lessons {
                assert!(seen.insert(lesson.title), "`{}` twice", lesson.title);
                assert!(!lesson.summary.is_empty());
                assert!(
                    !lesson.examples.is_empty() || !lesson.tips.is_empty(),
                    "{}: nothing to show",
                    lesson.title
                );
                for example in lesson.examples {
                    // A topic of a tool is about that tool.
                    if let Some(tool) = TOPICS
                        .iter()
                        .find(|t| t.lessons.iter().any(|l| l.title == lesson.title))
                        .and_then(|t| tool_of_topic(t.title))
                    {
                        assert_eq!(example.tool(), tool, "{}", example.caption);
                    }
                }
            }
        }
    }

    fn tool_of_topic(title: &str) -> Option<Tool> {
        [
            Tool::Merge,
            Tool::Format,
            Tool::Diff,
            Tool::Patch,
            Tool::Validate,
            Tool::Create,
        ]
        .into_iter()
        .find(|t| t.label() == title)
    }

    /// What the notes say of the results, checked against what the tools make.
    #[test]
    fn what_the_notes_say_is_what_comes_out() {
        let outcome = |caption: &str| {
            let (_, example) = every_example()
                .into_iter()
                .find(|(_, e)| e.caption == caption)
                .unwrap_or_else(|| panic!("no example `{caption}`"));
            run(example)
        };
        let json =
            |caption: &str| -> Value { serde_json::from_str(&outcome(caption).text).unwrap() };

        let orders = json("Two months of orders, one list");
        assert_eq!(orders.as_array().map(Vec::len), Some(3));
        assert_eq!(orders[0]["id"], 101);
        assert_eq!(
            json("Tags of two files, each once"),
            serde_json::json!(["go", "python", "rust", "sql"])
        );
        let settings = json("Defaults, production, and a local change");
        assert_eq!(
            settings,
            serde_json::json!({
                "server": {"host": "0.0.0.0", "port": 3000, "tls": true},
                "log": {"level": "warn", "format": "pretty"},
                "features": ["search", "export"]
            })
        );
        assert_eq!(
            json("Stock of two warehouses, summed by product"),
            serde_json::json!([
                {"sku": "A-1", "qty": 7}, {"sku": "B-2", "qty": 1}, {"sku": "C-3", "qty": 7}
            ])
        );
        assert_eq!(
            json("Which file each row came from")[3]["from"],
            "paris.json"
        );

        assert!(outcome("Keys in order, and only ASCII")
            .text
            .contains("\"Z\\u00fcrich\""));
        assert_eq!(
            outcome("An indented document, minified")
                .text
                .lines()
                .count(),
            1
        );

        let service = outcome("Version 1.4 and 1.5 of a service's settings");
        assert_eq!(service.summary, "2 added · 1 removed · 3 changed");
        assert!(
            service.text.contains("removed /env/DEBUG"),
            "{}",
            service.text
        );
        let steps = outcome("A step put in the middle");
        assert_eq!(steps.summary, "1 added");
        assert!(steps.text.starts_with("added   /steps/1"), "{}", steps.text);
        assert_eq!(
            outcome("The same members, in another order").summary,
            "the same"
        );

        assert_eq!(
            json("Release a new version"),
            serde_json::json!({
                "name": "orders-api", "version": "1.5.0", "tags": ["api", "stable"],
                "team": "platform", "maintainer": {"team": "platform"}
            })
        );
        let failed = outcome("A test that does not hold").error.unwrap();
        assert!(failed.contains("operation 2"), "{failed}");
        assert_eq!(
            json("Settings changed with a merge patch"),
            serde_json::json!({
                "name": "orders-api", "version": "1.5.0", "tags": ["api"],
                "owner": {"team": "platform", "email": "platform@example.com"}
            })
        );

        assert_eq!(outcome("A person, as the schema says").summary, "valid");
        assert_eq!(
            outcome("A person with six things wrong").summary,
            "6 problems"
        );
        let addresses = outcome("Two addresses, one definition");
        assert_eq!(addresses.summary, "2 problems");
        assert!(addresses.text.lines().all(|l| l.starts_with("/shipping")));

        let users = outcome("Three users, one schema of a user");
        let schema: Value =
            serde_json::from_str(users.text.split("\n\nPatterns found:").next().unwrap()).unwrap();
        assert_eq!(
            schema["items"]["required"],
            serde_json::json!(["name", "age", "admin", "skills"])
        );
        assert_eq!(
            schema["items"]["properties"]["email"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert_eq!(schema["items"]["properties"]["age"]["type"], "number");
        let orders = outcome("Orders with ids, dates, addresses and codes");
        for part in [
            "format uuid",
            "format date-time",
            "^ORD-[0-9]{4}-[0-9]{5}$",
            "^[0-9]{5}$",
        ] {
            assert!(orders.text.contains(part), "{part}: {}", orders.text);
        }
        let customer = outcome("A customer with lists, a map and two phones");
        for part in [
            "comma-separated: words",
            "semicolon-separated: IPv4 address",
            "keys: date",
            "^phone[0-9]+$",
        ] {
            assert!(customer.text.contains(part), "{part}: {}", customer.text);
        }
        let tickets = outcome("Tickets, with enums and bounds");
        let schema: Value =
            serde_json::from_str(tickets.text.split("\n\nPatterns found:").next().unwrap())
                .unwrap();
        let item = &schema["items"]["properties"];
        assert_eq!(
            item["status"]["enum"],
            serde_json::json!(["open", "closed", "waiting"])
        );
        assert_eq!(item["title"].get("enum"), None);
        assert_eq!(
            (
                item["priority"]["minimum"].clone(),
                item["priority"]["maximum"].clone()
            ),
            (1.into(), 5.into())
        );
        assert_eq!(
            (
                item["title"]["minLength"].clone(),
                item["title"]["maxLength"].clone()
            ),
            (11.into(), 21.into())
        );
        assert_eq!(schema["maxItems"], 5);
    }

    /// The prose is the lessons' markup: `code` and *emphasis*, and no
    /// `**`, which it does not have.
    #[test]
    fn the_prose_is_in_the_markup_the_window_reads() {
        for topic in TOPICS {
            for lesson in topic.lessons {
                let mut prose = vec![lesson.summary];
                prose.extend(lesson.tips);
                for example in lesson.examples {
                    prose.extend(example.notes);
                }
                for text in prose {
                    assert!(!text.contains("**"), "{}: {text}", lesson.title);
                    for span in crate::tutorial::parse_inline(text) {
                        if let crate::tutorial::Span::Text(plain) = span {
                            assert!(
                                !plain.contains('`'),
                                "{}: a backtick left open: {text}",
                                lesson.title
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_setting_says_the_options_as_the_page_does() {
        assert_eq!(Setting::Merge { filter: "add" }.describe(), "Filter: add");
        assert_eq!(
            Setting::Format(reformat::Options {
                indent: Indent::Spaces(4),
                sort_keys: true,
                ascii_only: false
            })
            .describe(),
            "Indent 4 spaces · Sort keys"
        );
        assert_eq!(Setting::Diff.describe(), "");
        assert_eq!(
            Setting::Create(CREATE).describe(),
            "Draft 2020-12 · Patterns · Required"
        );
    }

    #[test]
    fn a_lesson_is_found_by_its_tool_and_its_files() {
        let deep = TOPICS[1].lessons[1];
        assert!(lesson_matches(&deep, ""));
        assert!(lesson_matches(&deep, "production.json"));
        assert!(lesson_matches(&deep, "merge json"));
        assert!(!lesson_matches(&deep, "zzz"));
    }

    /// `cargo test -p jsonquery-query tutorial::tools -- --ignored --nocapture`
    /// prints what every example makes, to proof-read the notes against.
    #[test]
    #[ignore = "prints every tool example's live result for manual review"]
    fn dump_every_tool_example() {
        for (at, example) in every_example() {
            let outcome = run(example);
            println!(
                "\n== {at}\n  {} [{}]",
                example.caption,
                example.setting.describe()
            );
            match &outcome.error {
                Some(error) => println!("  ERROR: {error}"),
                None => println!("  {}\n{}", outcome.summary, outcome.text),
            }
        }
    }
}
