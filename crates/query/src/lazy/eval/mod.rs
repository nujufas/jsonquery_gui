//! Running a planned program against a document that is still in its file.
//!
//! The value that goes from one stage to the next is an [`Item`]: a node of the
//! file, or a value in memory. A stage that can be walked is walked while its
//! input is a node too big to be a value; a node that is small enough, or a
//! value, is given to jaq together with the rest of the program, and what jaq
//! makes of it is what the rest of the pipeline makes. So the part of the work
//! that is the file's size — a pass over the elements of a list, say — is made
//! one element at a time, and what is in memory at any moment is one element and
//! what has been gathered.
//!
//! What the walked stages do on what they are given follows what jaq does on a
//! value (the tests run both and compare), down to `.[0]` of an object being
//! `null` and an error when the input has no such thing.
//!
//! - [`lazy`]: what a node of the file, a part of a list or a list in a new order
//!   is, as a value that is still in the file;
//! - [`walk`]: the stages that are walked;
//! - [`path`]: the steps of a path, and the elements of a long list by threads;
//! - [`compose`]: expressions made of parts that are walked;
//! - [`keyed`]: what needs a key of every element of a list — `sort_by`,
//!   `group_by`, `min_by` — made from the keys and the places of the elements.

mod atom;
mod compose;
mod jmespath;
mod jsonpath;
mod keyed;
mod lazy;
mod path;
mod walk;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use jsonquery_core::engine::{QueryError as EngineError, QueryEvent};
use jsonquery_core::lazy::LazyTree;
use jsonquery_core::ValueKind;
use serde_json::Value;

use super::plan::{Kind, Pipeline, Stage};
use crate::{to_val, Program, QueryError};

pub(super) use jmespath::run_jmespath;
pub(super) use jsonpath::run_jsonpath;
pub(super) use lazy::Lazy;

/// How much of a document a query may hold at once.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// A list or an object that takes this many bytes of the file or fewer is
    /// parsed and given to jaq whole, when a stage cannot be walked. (In
    /// memory it takes some twenty times that, twice over.)
    pub materialize_bytes: usize,
    /// The same for a string or any other value that is not a list or an object.
    pub scalar_bytes: usize,
    /// How big one result may be: a bigger one is not made into a value, which
    /// the results panel would have to hold.
    pub result_bytes: usize,
    /// How much `[…]` and `map(…)` may gather, as an estimate of the memory it
    /// takes.
    pub collect_bytes: usize,
    /// How much the keys of a list may take that is sorted, grouped or searched
    /// for its smallest, as an estimate of the memory: every element has one,
    /// and where it is in the file.
    pub key_bytes: usize,
    /// How the elements of a long list are shared out to threads.
    pub parallel: Parallel,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            materialize_bytes: 4 * 1024 * 1024,
            scalar_bytes: 64 * 1024 * 1024,
            result_bytes: 16 * 1024 * 1024,
            collect_bytes: 1024 * 1024 * 1024,
            key_bytes: 1024 * 1024 * 1024,
            parallel: Parallel::default(),
        }
    }
}

/// Running a program on each element of a long list is work that the elements
/// share out among themselves: threads each take a run of them, and what they
/// make is handed on in the order of the list, as if one thread had done it.
#[derive(Clone, Copy, Debug)]
pub struct Parallel {
    /// How many threads besides the one that asked; 0 or 1 is none at all.
    pub threads: usize,
    /// The most elements a thread takes at a time.
    pub chunk: usize,
    /// How many elements are done first by the thread that asked, so that a
    /// query that finds what it is after near the beginning (a `first`, a `limit`)
    /// answers at once, without a crowd having started on the rest...
    pub head: usize,
    /// ... or as many as take this many bytes of the file, if that is fewer.
    pub head_bytes: usize,
    /// A list takes at least this many bytes of the file for threads to be worth
    /// starting on its elements, which, if they are few, are heavy.
    pub min_bytes: usize,
    /// A thread takes about this many bytes of elements at a time, if that is
    /// fewer than `chunk` of them: a list of a few large elements is shared out
    /// element by element.
    pub chunk_bytes: usize,
}

impl Default for Parallel {
    fn default() -> Self {
        // Leave the user interface, and whatever else is running, some of the
        // machine.
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            threads: cores.saturating_sub(2).clamp(1, 12),
            chunk: 256,
            head: 20_000,
            head_bytes: 1024 * 1024,
            min_bytes: 4 * 1024 * 1024,
            chunk_bytes: 256 * 1024,
        }
    }
}

/// What goes from one stage to the next.
pub(super) enum Item<'t> {
    Lazy(Lazy<'t>),
    Value(Value),
}

/// What a pipeline hands on: an item, or the error that is what a stage made —
/// or the refusal to do what was asked of what is too big for it, which, unlike
/// an error, is not the program's fault and is never taken for one (`?` and `try`
/// let an error go by and do not let this).
pub(super) enum Emission<'t> {
    Item(Item<'t>),
    Error(String),
    Refusal(String),
}

/// Why a step or a stage could not be done: as jaq would say, or because the
/// node is too big for it.
pub(super) enum Failure {
    Error(String),
    Refusal(String),
}

impl Failure {
    fn emission<'t>(self) -> Emission<'t> {
        match self {
            Failure::Error(e) => Emission::Error(e),
            Failure::Refusal(e) => Emission::Refusal(e),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Flow {
    Continue,
    Stop,
}

type Sink<'s, 't> = &'s mut dyn FnMut(Emission<'t>) -> Flow;

/// What is done with what a path finds: the rest of the pipeline is run on it,
/// by whichever evaluator is given (a thread has one of its own).
type Then<'a, 't> = &'a (dyn Fn(&Evaluator<'_>, Item<'t>, Sink<'_, 't>) -> Flow + Sync);

pub(super) struct Evaluator<'c> {
    cancelled: &'c AtomicBool,
    limits: Limits,
    /// The programs given to jaq, by their text: one is run on a lot of values.
    programs: RefCell<HashMap<String, Rc<Program>>>,
    /// Whether this evaluator may start threads: the one that was asked the
    /// question may, the ones it starts may not.
    crowd: bool,
}

impl<'c> Evaluator<'c> {
    pub(super) fn new(cancelled: &'c AtomicBool, limits: Limits) -> Self {
        Self {
            cancelled,
            limits,
            programs: RefCell::new(HashMap::new()),
            crowd: limits.parallel.threads > 1,
        }
    }

    /// One for a thread that another evaluator started.
    fn for_a_thread(cancelled: &'c AtomicBool, limits: Limits) -> Self {
        Self {
            cancelled,
            limits,
            programs: RefCell::new(HashMap::new()),
            crowd: false,
        }
    }

    fn stopped(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Run the pipeline from its `i`th stage on `item`, handing on what it makes.
    pub(super) fn feed<'t>(
        &self,
        p: &Pipeline,
        i: usize,
        item: Item<'t>,
        sink: Sink<'_, 't>,
    ) -> Flow {
        if self.stopped() {
            return Flow::Stop;
        }
        let Some(stage) = p.stages.get(i) else {
            return sink(Emission::Item(item));
        };
        match item {
            Item::Value(value) => self.jaq(stage, &value, sink),
            Item::Lazy(lazy) => {
                if self.is_small(&lazy) && !prefers_walking(&lazy, stage) {
                    match self.materialize(&lazy, usize::MAX) {
                        Ok(value) => self.jaq(stage, &value, sink),
                        Err(e) => sink(Emission::Error(e)),
                    }
                } else {
                    self.walk_stage(p, i, stage, lazy, sink)
                }
            }
        }
    }

    /// Give jaq the rest of the program, from `stage`, to run on `value`.
    fn jaq<'t>(&self, stage: &Stage, value: &Value, sink: Sink<'_, 't>) -> Flow {
        let program = match self.program(&stage.rest) {
            Ok(program) => program,
            Err(e) => return sink(Emission::Error(e)),
        };
        for result in program.run(to_val(value), Vec::new()) {
            if self.stopped() {
                return Flow::Stop;
            }
            let flow = match result {
                Ok(v) => sink(Emission::Item(Item::Value(v))),
                Err(e) => sink(Emission::Error(e)),
            };
            if flow == Flow::Stop {
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    fn program(&self, text: &str) -> Result<Rc<Program>, String> {
        if let Some(program) = self.programs.borrow().get(text) {
            return Ok(program.clone());
        }
        let program = Rc::new(Program::compile(text, &[]).map_err(|e| e.to_string())?);
        self.programs
            .borrow_mut()
            .insert(text.to_owned(), program.clone());
        Ok(program)
    }

    /// The program `text`, which reads the `count` variables `$__l0`, `$__l1`, …
    fn program_of_parts(&self, text: &str, count: usize) -> Result<Rc<Program>, String> {
        let key = format!("{count}\u{0}{text}");
        if let Some(program) = self.programs.borrow().get(&key) {
            return Ok(program.clone());
        }
        let names: Vec<String> = (0..count).map(|n| format!("$__l{n}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let program = Rc::new(Program::compile(text, &names).map_err(|e| e.to_string())?);
        self.programs.borrow_mut().insert(key, program.clone());
        Ok(program)
    }
}

/// Whether a stage that can be walked is walked on a list that is small enough
/// to be given to jaq. It is where the list is a part of one, or one that was
/// put in order or in groups, of many elements: a few of them are all that
/// `.[0]`, `length` or `{n: length}` ask for, and parsing all of them to give
/// them to jaq takes a lot more than looking.
fn prefers_walking(lazy: &Lazy<'_>, stage: &Stage) -> bool {
    const MANY: usize = 256;
    !lazy.is_whole() && lazy.len() > MANY && !matches!(stage.kind, Kind::Opaque)
}

/// About how much memory a value takes: a `serde_json::Value` is 72 bytes where it
/// is held, and a number (kept as its text, to be exact), a string or an object's
/// key is a block of memory of its own besides.
pub(super) fn approximate_size(value: &Value) -> usize {
    const VALUE: usize = 72;
    match value {
        Value::Null | Value::Bool(_) => VALUE,
        Value::Number(n) => VALUE + 32 + n.to_string().len(),
        Value::String(s) => VALUE + 16 + s.len(),
        Value::Array(items) => VALUE + items.iter().map(approximate_size).sum::<usize>(),
        Value::Object(map) => {
            VALUE
                + map
                    .iter()
                    .map(|(k, v)| 40 + k.len() + approximate_size(v))
                    .sum::<usize>()
        }
    }
}

fn bytes_text(bytes: usize) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn shorten(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// What `type` makes of a value of this kind.
fn type_name(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Null => "null",
        ValueKind::Bool => "boolean",
        ValueKind::Number => "number",
        ValueKind::String => "string",
        ValueKind::Array => "array",
        ValueKind::Object => "object",
    }
}

/// Run the jq program `src` over the document in `tree`, handing the outputs
/// to `on_event` as they are made.
pub(super) fn run_jq(
    tree: &LazyTree,
    src: &str,
    limits: Limits,
    cancelled: &AtomicBool,
    on_event: &mut dyn FnMut(QueryEvent),
) -> Result<usize, EngineError> {
    // A program that is not jq, or that does not compile, is refused in the same
    // words as it is for a document that is a value.
    Program::compile(src, &[]).map_err(|e| match e {
        QueryError::Parse(s) => EngineError::Parse(s),
        QueryError::Compile(s) => EngineError::Engine(s),
    })?;

    let pipeline = super::plan::plan(src);
    let evaluator = Evaluator::new(cancelled, limits);
    let mut count = 0usize;
    evaluator.feed(
        &pipeline,
        0,
        Item::Lazy(Lazy::of(tree.root())),
        &mut |emission| {
            count += 1;
            let value = match emission {
                Emission::Item(item) => evaluator.to_value(item),
                Emission::Error(e) | Emission::Refusal(e) => Err(e),
            };
            on_event(match value {
                Ok(value) => QueryEvent::Item(value),
                Err(e) => QueryEvent::ItemError(e),
            });
            Flow::Continue
        },
    );
    Ok(count)
}
