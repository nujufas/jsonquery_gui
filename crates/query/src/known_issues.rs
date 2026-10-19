//! The jq programs that do not run here as they run in jq, as data: [`GROUPS`].
//!
//! The About window shows the list under *Known issues*, and
//! `docs/known-issues.md` is made from it ([`markdown`]), so that the two say the
//! same. Each issue comes with programs that show it, what jq 1.8.1 gives for them
//! (made by running them in jq) and what this engine gives, and the tests below
//! *run* them: when an upgrade of the engine, or a fix of ours, makes one behave
//! like jq, its test fails and says which entry to take out of the list. (The same
//! idea as `documented_gaps_are_still_real` in the tutorial: a list of what is
//! wrong cannot go stale without somebody being told.)
//!
//! Like [`crate::tutorial`] this is pure data and small pure helpers, no egui; the
//! window is in `crates/app/src/issues_view.rs`.

use std::fmt::Write as _;

mod list;

pub use list::GROUPS;

/// What this engine does with an example's program.
#[derive(Clone, Copy, Debug)]
pub enum Here {
    /// It gives these results, each as the compact JSON text the window shows.
    Output(&'static [&'static str]),
    /// It gives one result, a string of the text that a value would have been
    /// written as: what the engine gave could not be shown as the value it is.
    Text,
    /// It gives one result, arrays and objects one in the next as many levels deep
    /// as this, and then the text that stands for what is nested deeper
    /// ([`crate::convert::TOO_DEEP`]).
    Cut(usize),
    /// It fails.
    Error,
    /// It never finishes, and uses more memory the longer it goes on. The tests
    /// look at the first thousand results it gives, and do not wait for the end.
    Hangs,
    /// It ends the app. The tests cannot run it: it would end them too.
    Crashes,
}

impl Here {
    /// What it does, in words and `code`, as the list and the page say it.
    pub fn describe(&self) -> String {
        match self {
            Here::Output([]) => "no result".to_owned(),
            Here::Output(texts) => format!("`{}`", texts.join(" ")),
            Here::Text => "a string of text".to_owned(),
            Here::Cut(levels) => format!("the nested arrays, cut off at {levels} levels"),
            Here::Error => "an error".to_owned(),
            Here::Hangs => "never finishes".to_owned(),
            Here::Crashes => "the app closes".to_owned(),
        }
    }
}

/// A program over a document that shows an issue.
#[derive(Clone, Copy, Debug)]
pub struct Example {
    /// The document, as JSON text (`null` when the program needs none).
    pub input: &'static str,
    pub query: &'static str,
    /// What jq 1.8.1 gives, in words and `code`: its results one after the other
    /// in one span, or `an error`.
    pub jq: &'static str,
    pub here: Here,
}

impl Example {
    pub const fn new(
        input: &'static str,
        query: &'static str,
        jq: &'static str,
        here: Here,
    ) -> Self {
        Self {
            input,
            query,
            jq,
            here,
        }
    }

    /// The program as one line: the document, piped into the query, unless there
    /// is none.
    pub fn program(&self) -> String {
        program(self.input, self.query)
    }
}

/// `input | query`, or only the query when the document is `null`.
fn program(input: &str, query: &str) -> String {
    match input {
        "null" => query.to_owned(),
        input => format!("{input} | {query}"),
    }
}

/// A way round an issue, which gives `gives` (one result, compact JSON text) over
/// `input`, here and in jq.
#[derive(Clone, Copy, Debug)]
pub struct Workaround {
    /// What to do, in words and `code`.
    pub text: &'static str,
    pub input: &'static str,
    pub query: &'static str,
    pub gives: &'static str,
}

impl Workaround {
    /// The program as one line, as [`Example::program`] has it.
    pub fn program(&self) -> String {
        program(self.input, self.query)
    }

    pub const fn new(
        text: &'static str,
        input: &'static str,
        query: &'static str,
        gives: &'static str,
    ) -> Self {
        Self {
            text,
            input,
            query,
            gives,
        }
    }
}

/// One thing that is not as it is in jq.
#[derive(Clone, Copy, Debug)]
pub struct Issue {
    /// Names the issue in the tests and the page.
    pub id: &'static str,
    /// One line, in words and `code` (see `tutorial::parse_inline`), as the About
    /// window and the page say it.
    pub text: &'static str,
    pub examples: &'static [Example],
    pub workaround: Option<Workaround>,
    /// One that the About window leaves out, saying that there are more in the
    /// page: it is rarer, or smaller, than the others.
    pub minor: bool,
}

impl Issue {
    pub const fn new(id: &'static str, text: &'static str, examples: &'static [Example]) -> Self {
        Self {
            id,
            text,
            examples,
            workaround: None,
            minor: false,
        }
    }

    pub const fn workaround(mut self, workaround: Workaround) -> Self {
        self.workaround = Some(workaround);
        self
    }

    pub const fn minor(mut self) -> Self {
        self.minor = true;
        self
    }
}

/// Issues of one kind, under one heading.
#[derive(Clone, Copy, Debug)]
pub struct Group {
    pub title: &'static str,
    pub issues: &'static [Issue],
}

impl Group {
    pub const fn new(title: &'static str, issues: &'static [Issue]) -> Self {
        Self { title, issues }
    }

    /// The issues the About window lists: those that are not minor.
    pub fn main(&self) -> impl Iterator<Item = &'static Issue> {
        self.issues.iter().filter(|issue| !issue.minor)
    }
}

/// Every issue, in the order of the groups.
pub fn all() -> impl Iterator<Item = &'static Issue> {
    GROUPS.iter().flat_map(|group| group.issues.iter())
}

/// How many issues there are.
pub fn count() -> usize {
    all().count()
}

/// How many of them the About window leaves to the page.
pub fn minor_count() -> usize {
    all().filter(|issue| issue.minor).count()
}

/// Where the page is, from the root of the repository.
pub const PAGE: &str = "docs/known-issues.md";

/// The page's text before the groups.
const PAGE_HEAD: &str = "\
# Known issues

<!-- Made from crates/query/src/known_issues/list.rs: edit that, and write this page again with
     JSONQUERY_WRITE_DOCS=1 cargo test -p jsonquery_gui the_known_issues_page -->

jsonquery runs jq programs with [jaq](https://github.com/01mf02/jaq), a Rust re-implementation of
jq, and adds `@csv`, `@tsv`, `IN`, `INDEX`, `JOIN`, `tostream`, `fromstream` and `truncate_stream`
itself, with its own `fromjson`, `tonumber` and `tojson`. Everyday programs give the results jq 1.8
gives; the ones below do not, yet. The same list is in the app, under **Known issues** in the ⓘ
window (there without the smaller ones).

The *jq 1.8.1* column is what jq itself prints for the program, the *Here* column what this app
does. Every example is run by a test, and so is every workaround: when an issue is fixed, the test
fails until its entry is taken out of the list.
";

/// The page's text after the groups.
const PAGE_FOOT: &str = "\
## Not in jq 1.8.1 either

These are missing here, and from jq 1.8.1 too (older versions of jq had some of them), so they are
not issues: `leaf_paths`, `recurse_down`, `toarray`, `ascii`, `date`, `dateadd`, `datesub`,
`@base32` and `@base32d`.
";

/// `text` as a cell of a table: a `|` in it is not the end of the cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// `text` as the code in a cell of a table.
fn code_cell(text: &str) -> String {
    if text.contains('`') {
        cell(&format!("`` {text} ``"))
    } else {
        cell(&format!("`{text}`"))
    }
}

/// The page `docs/known-issues.md`.
pub fn markdown() -> String {
    let mut page = String::from(PAGE_HEAD);
    for group in GROUPS {
        let _ = write!(page, "\n## {}\n", group.title);
        for issue in group.issues {
            let _ = write!(page, "\n**{}**\n\n", issue.text);
            page.push_str("| Input | Program | jq 1.8.1 | Here |\n|---|---|---|---|\n");
            for example in issue.examples {
                let _ = writeln!(
                    page,
                    "| {} | {} | {} | {} |",
                    code_cell(example.input),
                    code_cell(example.query),
                    cell(example.jq),
                    cell(&example.here.describe()),
                );
            }
            if let Some(workaround) = issue.workaround {
                let _ = write!(
                    page,
                    "\nWorkaround: {}: {} gives `{}`.\n",
                    workaround.text,
                    code_span(&workaround.program()),
                    workaround.gives,
                );
            }
        }
    }
    page.push('\n');
    page.push_str(PAGE_FOOT);
    page
}

/// `text` as inline code.
fn code_span(text: &str) -> String {
    if text.contains('`') {
        format!("`` {text} ``")
    } else {
        format!("`{text}`")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::Value;

    use super::*;
    use crate::tutorial::{self, Outcome};
    use crate::{to_val, Kind, Program};

    fn run(input: &str, query: &str) -> Outcome {
        tutorial::run(Kind::Jq, input, query)
    }

    /// The results of an outcome as compact JSON text, or why there are none.
    fn shown(outcome: &Outcome) -> String {
        if let Some(error) = &outcome.error {
            return format!("fails: {error}");
        }
        if let Some(error) = outcome.item_errors.first() {
            return format!("fails: {error}");
        }
        outcome
            .items
            .iter()
            .map(|item| serde_json::to_string(item).unwrap())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn failed(outcome: &Outcome) -> bool {
        outcome.error.is_some() || !outcome.item_errors.is_empty()
    }

    /// How many levels of arrays and objects, each with one thing in it, there are
    /// down to the text that stands for what was cut off, if there is such a text.
    fn cut_at(value: &Value) -> Option<usize> {
        let (mut levels, mut inside) = (0, value);
        loop {
            inside = match inside {
                Value::Array(items) if items.len() == 1 => &items[0],
                Value::Object(entries) if entries.len() == 1 => entries.values().next()?,
                Value::String(text) if text == crate::convert::TOO_DEEP => return Some(levels),
                _ => return None,
            };
            levels += 1;
        }
    }

    /// Whether the program's results go on for ever: it gives a thousand of them,
    /// all errors, and would give more. (jaq's `fromjson` did that on text that is
    /// not JSON: its parser does not step over the token it cannot read, and
    /// whoever is reading the results, as a query box is, goes on asking.)
    fn goes_on_for_ever(input: &str, query: &str) -> bool {
        let input: Value = serde_json::from_str(input).unwrap();
        let program = Program::compile(query, &[]).expect("the program compiles");
        let results = program.run(to_val(&input), Vec::new());
        results.take(1000).filter(Result::is_err).count() == 1000
    }

    #[test]
    fn every_issue_has_an_id_a_line_and_examples() {
        let mut ids = HashSet::new();
        for issue in all() {
            assert!(ids.insert(issue.id), "{} is in the list twice", issue.id);
            assert!(!issue.examples.is_empty(), "{} has no example", issue.id);
            assert!(!issue.text.is_empty() && !issue.text.contains('\n'));
            assert!(
                issue.text.matches('`').count() % 2 == 0,
                "{}: a `code` span is left open in {:?}",
                issue.id,
                issue.text
            );
            for example in issue.examples {
                assert!(
                    serde_json::from_str::<Value>(example.input).is_ok(),
                    "{}: {} is not JSON",
                    issue.id,
                    example.input
                );
                assert!(!example.jq.is_empty(), "{}: what jq gives", issue.id);
            }
        }
        for group in GROUPS {
            assert!(
                group.main().next().is_some(),
                "{}: the window would show a heading and nothing under it",
                group.title
            );
        }
    }

    #[test]
    fn the_window_leaves_some_to_the_page_but_shows_a_good_part() {
        assert!(
            minor_count() > 0,
            "otherwise there is nothing to say of the page"
        );
        assert!(
            minor_count() * 3 < count() * 2,
            "at least a third of them are in the window"
        );
    }

    /// Says that an example no longer does what the list says it does.
    fn stale(issue: &Issue, example: &Example, now: &str) -> ! {
        panic!(
            "`{}` now gives {now}, and the list of known issues says {} (issue {:?}): if it is \
             fixed, take that example, or the whole issue, out of \
             crates/query/src/known_issues/list.rs, and write docs/known-issues.md again",
            example.program(),
            example.here.describe(),
            issue.id
        )
    }

    /// The tests that make this a list that cannot go stale.
    #[test]
    fn every_example_still_does_what_the_list_says() {
        for issue in all() {
            for example in issue.examples {
                match example.here {
                    // Not run: it would take the tests with it.
                    Here::Crashes => {}
                    Here::Hangs => {
                        if !goes_on_for_ever(example.input, example.query) {
                            stale(issue, example, "an end");
                        }
                    }
                    Here::Error => {
                        let outcome = run(example.input, example.query);
                        if !failed(&outcome) {
                            stale(issue, example, &shown(&outcome));
                        }
                    }
                    Here::Text => {
                        let outcome = run(example.input, example.query);
                        let one_string = outcome.is_ok()
                            && matches!(outcome.items.as_slice(), [Value::String(_)]);
                        if !one_string {
                            stale(issue, example, &shown(&outcome));
                        }
                    }
                    Here::Cut(levels) => {
                        let outcome = run(example.input, example.query);
                        let cut = outcome.is_ok()
                            && matches!(outcome.items.as_slice(), [item] if cut_at(item) == Some(levels));
                        if !cut {
                            stale(issue, example, &shown(&outcome));
                        }
                    }
                    Here::Output(texts) => {
                        let outcome = run(example.input, example.query);
                        if failed(&outcome) || shown(&outcome) != texts.join(" ") {
                            stale(issue, example, &shown(&outcome));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_hang_is_found_by_looking_at_the_first_results_not_by_waiting() {
        // The check itself: a program with an end is not one that goes on.
        assert!(!goes_on_for_ever("\"1\"", "tonumber"));
        assert!(!goes_on_for_ever("null", "empty"));
        // An error, and the same again: what `"abc" | tonumber` once did.
        assert!(goes_on_for_ever("null", "def f: error(\"x\"), f; f"));
        assert!(goes_on_for_ever("null", "range(5000) | error"));
        // Fewer than a thousand errors is not for ever, and text that is not JSON has one.
        assert!(!goes_on_for_ever("null", "[range(10)] | .[] | error"));
        assert!(!goes_on_for_ever("\"abc\"", "tonumber"));
        assert!(!goes_on_for_ever("\"N/A\"", "fromjson"));
    }

    #[test]
    fn every_workaround_gives_what_it_says() {
        for issue in all() {
            let Some(workaround) = issue.workaround else {
                continue;
            };
            let outcome = run(workaround.input, workaround.query);
            assert!(
                outcome.is_ok() && shown(&outcome) == workaround.gives,
                "{}: {} gives {}, not {}",
                issue.id,
                workaround.program(),
                shown(&outcome),
                workaround.gives
            );
        }
    }

    #[test]
    fn the_page_has_every_issue_with_every_example_and_workaround() {
        let page = markdown();
        for issue in all() {
            assert!(page.contains(issue.text), "{}", issue.id);
            for example in issue.examples {
                assert!(page.contains(&code_cell(example.query)), "{}", issue.id);
            }
            if let Some(workaround) = issue.workaround {
                assert!(page.contains(workaround.text), "{}", issue.id);
            }
        }
        for group in GROUPS {
            assert!(page.contains(&format!("## {}\n", group.title)));
        }
        assert!(page.starts_with("# Known issues\n"));
        assert!(page.ends_with('\n') && !page.ends_with("\n\n"));
    }

    #[test]
    fn a_pipe_in_a_program_does_not_end_the_cell_of_the_table() {
        assert_eq!(code_cell("a | b"), "`a \\| b`");
        assert_eq!(code_cell("`x` | y"), "`` `x` \\| y ``");
        let example = Example::new("[1]", ".[] | . + 1", "`2`", Here::Output(&["2"]));
        assert_eq!(example.program(), "[1] | .[] | . + 1");
        assert_eq!(Example::new("null", "1", "`1`", Here::Error).program(), "1");
    }

    #[test]
    fn what_the_engine_does_is_said_in_words() {
        assert_eq!(Here::Output(&["3", "4"]).describe(), "`3 4`");
        assert_eq!(Here::Output(&[]).describe(), "no result");
        assert_eq!(Here::Error.describe(), "an error");
        assert_eq!(
            Here::Cut(127).describe(),
            "the nested arrays, cut off at 127 levels"
        );
        assert_eq!(Here::Hangs.describe(), "never finishes");
        assert_eq!(Here::Crashes.describe(), "the app closes");
    }
}
