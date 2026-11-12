//! The tools of the Tools, for the assistant. format_json, diff_documents,
//! apply_patch, validate_json and create_schema run the Tools' own jobs
//! (`tools::jobs::run`) on the call's thread, so that they do what the pages do, to
//! the same limits; merge_documents runs the Merge page's merge over the values.
//! None of them changes anything the user sees: show_in_tool fills a page for them.
//! What they make (a patched document, a merge, a schema, a patch) is kept as "last",
//! out of sight, for the assistant's next calls to read, query or save.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonquery_mcp::{shape, Input, ToolPage};
use jsonquery_query::diff::ChangeKind;
use jsonquery_query::{infer, merge, reformat};
use serde_json::{json, Map, Value};

use super::{full_path, AppHost, NO_DOCUMENT, POLL};
use crate::tools::jobs::{self, Job, Outcome};
use crate::tools::PageSetting;
use crate::workspace::ItemKind;

/// What an answer says of what was kept as "last".
const KEPT: &str = "kept: give \"last\" as an input";

/// How long the tools that take no `timeout_ms` may take.
pub(super) const TOOL_TIME: Duration = Duration::from_secs(30);

/// The most text a box of a page is given: a box that held more would stall the
/// window (the page reads a bigger file itself, when the user picks it).
const BOX_LIMIT: u64 = 1024 * 1024;

/// The JSON of a value, as a box of a page is given it.
fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// The first `max_bytes` of `text` (at a character's start), and whether that is all.
fn cut(text: &str, max_bytes: usize) -> (&str, bool) {
    if text.len() <= max_bytes {
        return (text, true);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], false)
}

/// `value` whole if it fits in `max_bytes` of JSON; if not, its first level, a page
/// of it, with what does not fit written as `$omitted`.
fn fitted(value: &Value, max_bytes: usize) -> Value {
    let text = value.to_string();
    if text.len() <= max_bytes {
        return value.clone();
    }
    shape::read(value, &Vec::new(), 0, usize::MAX, max_bytes)
}

fn change_kind(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "added",
        ChangeKind::Removed => "removed",
        ChangeKind::Changed => "changed",
    }
}

/// Whether a JSON input is an array: what its first character that is not white
/// space says (`None` when that cannot be told).
fn starts_as_array(input: &jobs::Input) -> Option<bool> {
    let first = |text: &str| text.trim_start().chars().next().map(|c| c == '[');
    match input {
        jobs::Input::Text(text) => first(text),
        jobs::Input::File(path) => {
            use std::io::Read;
            let mut head = [0u8; 4096];
            let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
            first(&String::from_utf8_lossy(&head[..n]))
        }
        jobs::Input::Document(doc) => doc.tree().map(Value::is_array),
    }
}

impl AppHost {
    /// Do `work` here, with a flag that is set when the call is cancelled or `timeout`
    /// passes; which of those it was is the error.
    fn timed<T>(
        &self,
        cancel: &Arc<AtomicBool>,
        timeout: Duration,
        work: impl FnOnce(&AtomicBool) -> T,
    ) -> Result<T, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let watch = {
            let (stop, timed_out, finished, cancel) = (
                stop.clone(),
                timed_out.clone(),
                finished.clone(),
                cancel.clone(),
            );
            let deadline = Instant::now() + timeout;
            std::thread::spawn(move || {
                while !finished.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                    if Instant::now() >= deadline {
                        timed_out.store(true, Ordering::Relaxed);
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                    std::thread::sleep(POLL);
                }
            })
        };
        let done = work(&stop);
        finished.store(true, Ordering::Relaxed);
        let _ = watch.join();
        if timed_out.load(Ordering::Relaxed) {
            return Err(format!(
                "it took more than {} ms and was stopped: give a larger timeout_ms, or smaller values",
                timeout.as_millis()
            ));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_owned());
        }
        Ok(done)
    }

    /// Run a job of the Tools here.
    fn run_job(
        &self,
        job: Job,
        cancel: &Arc<AtomicBool>,
        timeout: Duration,
    ) -> Result<Outcome, String> {
        let limits = self.with_app(|app| app.settings.limits)?;
        self.timed(cancel, timeout, |stop| jobs::run(job, stop, &limits))?
    }

    /// The file of a JSON item of the workspace (a document, a schema, a patch).
    pub(super) fn workspace_json(&self, rel: &str) -> Result<PathBuf, String> {
        self.with_app(|app| {
            let ws = app
                .workspace_pane
                .workspace()
                .ok_or("the app has no workspace folder")?;
            let item = ws.item(rel).ok_or_else(|| {
                format!("the workspace has no item \"{rel}\": workspace_list gives their paths")
            })?;
            match item.kind {
                ItemKind::Schema | ItemKind::Patch | ItemKind::Document => Ok(ws.abs(&item.path)),
                kind => Err(format!(
                    "\"{rel}\" is a {}, not JSON: workspace_read gives what it holds",
                    kind.what()
                )),
            }
        })?
    }

    /// What `input` names, as the Tools' jobs take it, and what to call it.
    fn job_input(&self, input: &Input) -> Result<(jobs::Input, String), String> {
        Ok(match input {
            Input::Document => {
                let doc = self.with_app(|app| app.doc.clone())?.ok_or(NO_DOCUMENT)?;
                let name = doc.source.label();
                (jobs::Input::Document(doc), name)
            }
            Input::Results => {
                let text = self.with_app(|app| app.results.to_string())?;
                (jobs::Input::Text(text), "the results".to_owned())
            }
            Input::Last => {
                let (value, what) = self.kept()?;
                (jobs::Input::Text(value.to_string()), what)
            }
            Input::Json(value) => (
                jobs::Input::Text(value.to_string()),
                "the JSON given".to_owned(),
            ),
            Input::Text(text) => (jobs::Input::Text(text.clone()), "the text given".to_owned()),
            Input::File(path) => {
                let path = full_path(path)?;
                let name = path.display().to_string();
                (jobs::Input::File(path), name)
            }
            Input::Workspace(rel) => (jobs::Input::File(self.workspace_json(rel)?), rel.clone()),
        })
    }

    /// What `input` names, as a value (for merge_documents), and what to call it.
    pub(super) fn value_of(&self, input: &Input, limit: u64) -> Result<(Value, String), String> {
        let parse = |text: &str, name: &str| {
            serde_json::from_str::<Value>(text).map_err(|e| format!("{name} is not JSON: {e}"))
        };
        let read = |path: &Path| -> Result<String, String> {
            let bytes = std::fs::metadata(path)
                .map_err(|e| format!("{} could not be read: {e}", path.display()))?
                .len();
            if bytes > limit {
                return Err(format!(
                    "{} is {bytes} bytes, more than the Tools take ({limit}; the user sets it in ⚙ Settings)",
                    path.display()
                ));
            }
            std::fs::read_to_string(path)
                .map_err(|e| format!("{} could not be read: {e}", path.display()))
        };
        Ok(match input {
            Input::Document => {
                let doc = self.with_app(|app| app.doc.clone())?.ok_or(NO_DOCUMENT)?;
                let value = doc.tree().cloned().ok_or(
                    "the open document is kept on disk (it is too big to be held): the Tools \
                     do not take it",
                )?;
                (value, doc.source.label())
            }
            Input::Results => (
                self.with_app(|app| app.results.clone())?,
                "the results".to_owned(),
            ),
            Input::Last => {
                let (value, what) = self.kept()?;
                (Arc::unwrap_or_clone(value), what)
            }
            Input::Json(value) => (value.clone(), "the JSON given".to_owned()),
            Input::Text(text) => (parse(text, "the text given")?, "the text given".to_owned()),
            Input::File(path) => {
                let path = full_path(path)?;
                let name = path.display().to_string();
                (parse(&read(&path)?, &name)?, name)
            }
            Input::Workspace(rel) => {
                let path = self.workspace_json(rel)?;
                (parse(&read(&path)?, rel)?, rel.clone())
            }
        })
    }

    /// What `input` names, as the text of a box of a page.
    fn box_text(&self, input: &Input) -> Result<String, String> {
        let too_big = |what: &str| {
            format!(
                "{what} is more than a box of the page takes (1 MB): the tool that computes the \
                 same (format_json, diff_documents…) takes it"
            )
        };
        let read = |path: &Path, what: &str| -> Result<String, String> {
            let bytes = std::fs::metadata(path)
                .map_err(|e| format!("{} could not be read: {e}", path.display()))?
                .len();
            if bytes > BOX_LIMIT {
                return Err(too_big(what));
            }
            std::fs::read_to_string(path)
                .map_err(|e| format!("{} could not be read: {e}", path.display()))
        };
        let text = match input {
            Input::Document => {
                let doc = self.with_app(|app| app.doc.clone())?.ok_or(NO_DOCUMENT)?;
                if doc.byte_len > BOX_LIMIT {
                    return Err(too_big("the open document"));
                }
                pretty(doc.tree().ok_or_else(|| too_big("the open document"))?)
            }
            Input::Results => pretty(&self.with_app(|app| app.results.clone())?),
            Input::Last => pretty(&self.kept()?.0),
            Input::Json(value) => pretty(value),
            Input::Text(text) => text.clone(),
            Input::File(path) => {
                let path = full_path(path)?;
                read(&path, &path.display().to_string())?
            }
            Input::Workspace(rel) => read(&self.workspace_json(rel)?, rel)?,
        };
        if text.len() as u64 > BOX_LIMIT {
            return Err(too_big("it"));
        }
        Ok(text)
    }

    pub(super) fn format_json(
        &self,
        input: &Input,
        options: reformat::Options,
        max_bytes: usize,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let (input, name) = self.job_input(input)?;
        let Outcome::Format(formatted) =
            self.run_job(Job::Format { input, options }, cancel, TOOL_TIME)?
        else {
            return Err("the Tools gave something else".to_owned());
        };
        if formatted.streamed.is_some() {
            return Err(format!(
                "{name} is kept on disk (it is too big to be held as text): save_file writes it \
                 formatted"
            ));
        }
        let (shown, complete) = cut(&formatted.text, max_bytes);
        let mut out = Map::new();
        out.insert("of".to_owned(), json!(name));
        out.insert("bytes".to_owned(), json!(formatted.text.len()));
        out.insert("complete".to_owned(), json!(complete));
        out.insert("text".to_owned(), json!(shown));
        if !complete {
            out.insert(
                "note".to_owned(),
                json!("the text is cut to a page: save_file writes all of it"),
            );
        }
        Ok(Value::Object(out))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn diff_documents(
        &self,
        left: &Input,
        right: &Input,
        offset: usize,
        limit: usize,
        as_patch: bool,
        max_bytes: usize,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let (left, left_name) = self.job_input(left)?;
        let (right, right_name) = self.job_input(right)?;
        let job = Job::Diff {
            left,
            right,
            take: None,
            edit: None,
        };
        let Outcome::Diff(compared) = self.run_job(job, cancel, timeout)? else {
            return Err("the Tools gave something else".to_owned());
        };
        let changes: Vec<Value> = compared
            .changes
            .iter()
            .map(|change| {
                json!({
                    "kind": change_kind(change.kind),
                    "pointer": change.path,
                    "left": change.before,
                    "right": change.after,
                })
            })
            .collect();
        let start = offset.min(changes.len());
        let (page, next) = shape::page(&changes[start..], start, start, limit, max_bytes);
        let mut out = Map::new();
        out.insert("left".to_owned(), json!(left_name));
        out.insert("right".to_owned(), json!(right_name));
        out.insert("same".to_owned(), json!(compared.equal));
        out.insert("added".to_owned(), json!(compared.added));
        out.insert("removed".to_owned(), json!(compared.removed));
        out.insert("changed".to_owned(), json!(compared.changed));
        out.insert("offset".to_owned(), json!(start));
        out.insert("changes".to_owned(), Value::Array(page));
        if let Some(next) = next {
            out.insert("next_offset".to_owned(), json!(next));
        }
        if compared.changes.len() < compared.total() {
            out.insert(
                "note".to_owned(),
                json!(format!(
                    "{} differences in all; the first {} are listed",
                    compared.total(),
                    compared.changes.len()
                )),
            );
        }
        if as_patch {
            let patch: Value = compared
                .patch
                .lines()
                .filter_map(|line| serde_json::from_str(line.trim().trim_end_matches(',')).ok())
                .collect::<Vec<Value>>()
                .into();
            out.insert("patch".to_owned(), fitted(&patch, max_bytes));
            out.insert("last".to_owned(), json!(KEPT));
            self.keep(
                patch,
                format!("the JSON Patch from {left_name} to {right_name}"),
            );
        }
        Ok(Value::Object(out))
    }

    pub(super) fn apply_patch(
        &self,
        input: &Input,
        patch: &Input,
        merge_patch: Option<bool>,
        max_bytes: usize,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let (document, name) = self.job_input(input)?;
        let (patch, patch_name) = self.job_input(patch)?;
        let merge_patch = match merge_patch {
            Some(merge) => merge,
            None => starts_as_array(&patch) == Some(false),
        };
        let job = Job::Patch {
            document,
            patch,
            merge_patch,
        };
        let Outcome::Patch(patched) = self.run_job(job, cancel, TOOL_TIME)? else {
            return Err("the Tools gave something else".to_owned());
        };
        let value = match patched.doc.tree() {
            Some(value) => value.clone(),
            None => serde_json::from_str(&patched.text)
                .map_err(|e| format!("the patched document is not JSON: {e}"))?,
        };
        let result = fitted(&value, max_bytes);
        self.keep(value, format!("{name} with {patch_name} applied"));
        Ok(json!({
            "of": name,
            "patch": patch_name,
            "kind": if merge_patch { "JSON Merge Patch" } else { "JSON Patch" },
            "operations": patched.operations,
            "result": result,
            "last": KEPT,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn merge_documents(
        &self,
        inputs: &[Input],
        filter: &str,
        max_bytes: usize,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let limit = self.with_app(|app| app.settings.limits.tools())?;
        let mut values = Vec::new();
        let mut names = Vec::new();
        for input in inputs {
            let (value, name) = self.value_of(input, limit)?;
            values.push(value);
            names.push(name);
        }
        let merged = self
            .timed(cancel, timeout, |stop| {
                merge::merge(values, &names, filter, stop)
            })?
            .map_err(|e| e.to_string())?;
        let result = fitted(&merged.value, max_bytes);
        let answer = json!({
            "documents": names,
            "filter": filter,
            "outputs": merged.outputs,
            "result": result,
            "last": KEPT,
        });
        self.keep(merged.value, format!("the merge of {}", names.join(", ")));
        Ok(answer)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn validate_json(
        &self,
        input: &Input,
        schema: &Input,
        check_formats: bool,
        offset: usize,
        limit: usize,
        max_bytes: usize,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let (document, name) = self.job_input(input)?;
        let (schema, schema_name) = self.job_input(schema)?;
        let job = Job::Validate {
            document,
            schema,
            check_formats,
        };
        let Outcome::Validate(validated) = self.run_job(job, cancel, timeout)? else {
            return Err("the Tools gave something else".to_owned());
        };
        let report = &validated.report;
        let problems: Vec<Value> = report
            .problems
            .iter()
            .skip(offset)
            .take(limit)
            .map(|p| {
                let mut out = json!({
                    "pointer": p.path,
                    "message": p.message,
                    "keyword": p.keyword,
                    "schema_pointer": p.schema_path,
                });
                if !p.unexpected.is_empty() {
                    out["unexpected"] = json!(p.unexpected);
                }
                out
            })
            .collect();
        let (page, _) = shape::page(&problems, offset, offset, limit, max_bytes);
        let end = offset + page.len();
        let mut out = json!({
            "of": name,
            "schema": schema_name,
            "valid": report.is_valid(),
            "draft": report.draft,
            "count": report.problems.len(),
            "offset": offset,
            "problems": page,
        });
        if end < report.problems.len() {
            out["next_offset"] = json!(end);
        }
        if report.more {
            out["note"] = json!("there are more problems than the check lists");
        }
        Ok(out)
    }

    pub(super) fn create_schema(
        &self,
        input: &Input,
        options: infer::Options,
        max_bytes: usize,
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<Value, String> {
        let (document, name) = self.job_input(input)?;
        let Outcome::Create(created) =
            self.run_job(Job::Create { document, options }, cancel, timeout)?
        else {
            return Err("the Tools gave something else".to_owned());
        };
        let schema: Value = serde_json::from_str(&created.text)
            .map_err(|e| format!("the schema made is not JSON: {e}"))?;
        let answer = json!({
            "of": name,
            "draft": options.draft.name(),
            "properties": created.properties,
            "schema": fitted(&schema, max_bytes),
            "last": KEPT,
        });
        self.keep(schema, format!("the schema made of {name}"));
        Ok(answer)
    }

    /// Fill a page of the Tools for the user, and show it (and run it).
    pub(super) fn show_in_tool(
        &self,
        page: &ToolPage,
        inputs: &[Input],
        run: bool,
    ) -> Result<Value, String> {
        let mut texts = Vec::new();
        let mut files = Vec::new();
        match page {
            ToolPage::Merge { .. } => {
                for input in inputs {
                    match input {
                        Input::File(path) => files.push(full_path(path)?),
                        _ => {
                            return Err(
                                "merge shows JSON files: give each as {\"file\": …}".to_owned()
                            )
                        }
                    }
                }
            }
            _ => {
                for input in inputs {
                    texts.push(self.box_text(input)?);
                }
            }
        }
        let setting = match page {
            ToolPage::Merge { filter } => PageSetting::Merge {
                filter: filter.clone(),
            },
            ToolPage::Format(options) => PageSetting::Format(*options),
            ToolPage::Diff => PageSetting::Diff,
            ToolPage::Patch { merge_patch } => PageSetting::Patch {
                merge_patch: merge_patch.unwrap_or_else(|| {
                    !texts
                        .get(1)
                        .is_some_and(|t| t.trim_start().starts_with('['))
                }),
            },
            ToolPage::Validate { check_formats } => PageSetting::Validate {
                check_formats: *check_formats,
            },
            ToolPage::CreateSchema(options) => PageSetting::Create(*options),
        };
        let ctx = self.ctx.clone();
        let (shown, place) = self.with_app(|app| {
            if let Some(request) = app.tools.fill_page(&ctx, &setting, &texts, files, run) {
                app.apply_tools_request(&ctx, request);
            }
            (app.tools.current_tool().label(), app.tools.is_docked())
        })?;
        self.shown();
        Ok(json!({
            "shown": shown,
            "where": if place { "a tab of the main window" } else { "the Tools window" },
            "ran": run,
            "note": "the user sees the page and its result; the tool that computes the same gives the result to you",
        }))
    }
}
