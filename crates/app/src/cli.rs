//! The command line. `jsonquery_gui data.json` opens a document as the app starts
//! (a file, a web address, or `-` for what is piped in), with a query to run on it
//! once it has loaded (`--query`, `--pointer`, or a file of one); several documents
//! go to Merge's list, as when they are dropped on the window; `--diff` and the
//! other tools fill that tool's page with files, and run it once its boxes are
//! full. `--help` and `--version` print, and open no window.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use jsonquery_query::Kind;

use crate::app::{source_of_text, SourceInput};
use crate::tools::Tool;

/// What the command line asks for.
#[derive(Debug, PartialEq)]
pub enum Ask {
    /// The window, started as this says.
    Run(Launch),
    /// The help, printed.
    Help,
    /// The version, printed.
    Version,
}

/// How the app starts, beyond opening its window.
#[derive(Debug, Default, PartialEq)]
pub struct Launch {
    /// The document the main window opens.
    pub open: Option<Source>,
    /// What goes in the query box, to run on `open` once it has loaded.
    pub query: Option<Query>,
    /// Documents for Merge's list, as when several are dropped on the window.
    pub merge: Vec<PathBuf>,
    /// A tool's page, filled with these files in the order of its boxes.
    pub tool: Option<(Tool, Vec<PathBuf>)>,
}

/// A document for the main window to open.
#[derive(Debug, PartialEq)]
pub enum Source {
    File(PathBuf),
    Url(String),
    /// What is piped into the app (`-`).
    Stdin,
}

/// A query for the query box.
#[derive(Debug, PartialEq)]
pub struct Query {
    pub text: String,
    /// Its language; none to have it told from the text, as when no language is
    /// picked in the app.
    pub language: Option<Kind>,
    /// The file it was read from, if it was.
    pub file: Option<PathBuf>,
}

/// The options that fill a tool's page, and the most files each page takes.
const TOOLS: [(&str, Tool, usize); 6] = [
    ("--diff", Tool::Diff, 2),
    ("--merge", Tool::Merge, usize::MAX),
    ("--patch", Tool::Patch, 2),
    ("--validate", Tool::Validate, 2),
    ("--format", Tool::Format, 1),
    ("--create-schema", Tool::Create, 1),
];

/// Read the command line `args` (the program's name left out). A relative path is
/// taken from `cwd`, the folder the app was started in (where that is not known,
/// it is left as it is), and a file of a query is read with `read`.
pub fn parse(
    args: impl IntoIterator<Item = OsString>,
    cwd: Option<&Path>,
    read: impl Fn(&Path) -> Result<String, String>,
) -> Result<Ask, String> {
    let mut args = args.into_iter();
    let mut words = Vec::new();
    let mut query: Option<String> = None;
    let mut language: Option<Kind> = None;
    let mut pointer: Option<String> = None;
    let mut tool: Option<(&str, Tool, usize)> = None;
    let mut options_ended = false;

    while let Some(arg) = args.next() {
        let text = match arg.to_str() {
            Some(text) if !options_ended && text.starts_with('-') && text != "-" => text.to_owned(),
            _ => {
                words.push(arg);
                continue;
            }
        };
        if text == "--" {
            options_ended = true;
            continue;
        }
        let (name, value) = match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (text, None),
        };
        match name.as_str() {
            "-h" | "--help" => return Ok(Ask::Help),
            "-V" | "--version" => return Ok(Ask::Version),
            "-q" | "--query" => {
                let text = value_of(&name, "a query", value, &mut args)?;
                once(&mut query, &name, text)?;
            }
            "-l" | "--lang" => {
                let name_of = value_of(&name, "a language", value, &mut args)?;
                once(&mut language, &name, language_called(&name_of)?)?;
            }
            "-p" | "--pointer" => {
                let text = value_of(&name, "a JSON Pointer", value, &mut args)?;
                once(&mut pointer, &name, json_pointer(text)?)?;
            }
            other => match TOOLS.iter().find(|(flag, ..)| *flag == other) {
                Some(&entry) => {
                    if value.is_some() {
                        return Err(format!("{other} takes no value: the files come after it"));
                    }
                    if let Some((before, ..)) = tool {
                        return Err(if before == other {
                            format!("{other} was given twice")
                        } else {
                            format!("{before} and {other} cannot both be given: one tool's page is filled")
                        });
                    }
                    tool = Some(entry);
                }
                None => return Err(format!("{other} is not an option")),
            },
        }
    }

    // What the words name: documents, and (unless a tool's page is being filled)
    // files of queries, which go in the query box as when they are dropped.
    let mut documents = Vec::new();
    let mut query_files = Vec::new();
    for word in words {
        let source = source_of(word, cwd);
        if source == Source::Stdin && documents.contains(&Source::Stdin) {
            return Err("- (what is piped in) was given twice".to_owned());
        }
        match source {
            Source::File(path) if tool.is_none() => match language_of_file(&path) {
                Some(language) => query_files.push((path, language)),
                None => documents.push(Source::File(path)),
            },
            source => documents.push(source),
        }
    }

    if pointer.is_some() && query.is_some() {
        return Err(
            "--query and --pointer cannot both be given: there is one query box".to_owned(),
        );
    }
    if pointer.is_some() && language.is_some() {
        return Err("--pointer is a JSON Pointer: --lang is for --query".to_owned());
    }
    if language.is_some() && query.is_none() {
        return Err("--lang says what language --query is in: give --query too".to_owned());
    }
    let query =
        match (query, pointer, query_files.as_slice()) {
            (None, None, []) => None,
            (Some(text), None, []) => Some(Query {
                text,
                language,
                file: None,
            }),
            (None, Some(text), []) => Some(Query {
                text,
                language: Some(Kind::JsonPointer),
                file: None,
            }),
            (None, None, [(path, language)]) => Some(Query {
                text: read(path)?,
                language: Some(*language),
                file: Some(path.clone()),
            }),
            (None, None, [..]) => {
                return Err(
                    "one query goes in the query box, and several files of queries were given"
                        .to_owned(),
                )
            }
            _ => return Err(
                "one query goes in the query box: give a file of a query, or --query or --pointer"
                    .to_owned(),
            ),
        };

    if let Some((flag, tool, most)) = tool {
        if query.is_some() {
            return Err(format!(
                "{flag} fills a tool's page: --query and --pointer are for a document that the main window opens"
            ));
        }
        let files = files_of(documents, &format!("{flag} takes files"))?;
        if files.len() > most {
            let takes = if most == 1 {
                "one file".to_owned()
            } else {
                format!("{most} files")
            };
            return Err(format!(
                "{flag} takes {takes}, and {} were given",
                files.len()
            ));
        }
        return Ok(Ask::Run(Launch {
            tool: Some((tool, files)),
            ..Launch::default()
        }));
    }

    if documents.len() > 1 {
        if query.is_some() {
            return Err(
                "a query runs on one document, and several were given (several go to Merge's list, as when they are dropped on the window)"
                    .to_owned(),
            );
        }
        let merge = files_of(
            documents,
            "several documents go to Merge's list, which takes files",
        )?;
        return Ok(Ask::Run(Launch {
            merge,
            ..Launch::default()
        }));
    }
    Ok(Ask::Run(Launch {
        open: documents.pop(),
        query,
        ..Launch::default()
    }))
}

/// The value of the option `name`: given with it (`--query=.a`), or the next
/// argument. `what` is what it takes, for when there is none.
fn value_of(
    name: &str,
    what: &str,
    given: Option<String>,
    rest: &mut impl Iterator<Item = OsString>,
) -> Result<String, String> {
    match given {
        Some(value) => Ok(value),
        None => rest
            .next()
            .ok_or_else(|| format!("{name} needs {what} after it"))?
            .into_string()
            .map_err(|_| format!("what follows {name} is not text")),
    }
}

/// Set `slot` to `value`, unless the option `name` that gives it was given before.
fn once<T>(slot: &mut Option<T>, name: &str, value: T) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("{name} was given twice"));
    }
    *slot = Some(value);
    Ok(())
}

/// The query language called `name`: by the name the MCP server takes it by, in
/// any case, with or without what is not a letter or a digit ("JSONPath",
/// "json-pointer").
fn language_called(name: &str) -> Result<Kind, String> {
    let key: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    match key.as_str() {
        "jq" | "jaq" => Ok(Kind::Jq),
        "jsonpath" | "path" => Ok(Kind::JsonPath),
        "jmespath" | "jmes" => Ok(Kind::JmesPath),
        "pointer" | "jsonpointer" => Ok(Kind::JsonPointer),
        _ => Err(format!(
            "--lang takes jq, jsonpath, jmespath or pointer, not {name:?}"
        )),
    }
}

/// `text`, if it is a JSON Pointer: nothing (the whole document), or what starts
/// with a `/`.
fn json_pointer(text: String) -> Result<String, String> {
    if text.is_empty() || text.starts_with('/') {
        Ok(text)
    } else {
        Err(format!(
            "--pointer takes a JSON Pointer, which starts with / (such as /items/0), not {text:?}"
        ))
    }
}

/// The language of the query in the file at `path`, if what its name ends in says
/// it is one (`.jq`, `.jsonpath`, `.jmespath`, `.jsonptr`).
fn language_of_file(path: &Path) -> Option<Kind> {
    path.file_name()
        .and_then(OsStr::to_str)
        .and_then(crate::workspace::query_language_of)
}

/// What `word` names: `-` what is piped in, an `http://` or `https://` address, or
/// a file (a `file://` address is one, which a desktop's launcher may hand over),
/// whose path is made whole from `cwd`.
fn source_of(word: OsString, cwd: Option<&Path>) -> Source {
    if word == "-" {
        return Source::Stdin;
    }
    let source = match word.to_str() {
        Some(text) => source_of_text(text),
        None => SourceInput::Path(PathBuf::from(word)),
    };
    match source {
        SourceInput::Url(url) => Source::Url(url),
        SourceInput::Path(path) => Source::File(whole(path, cwd)),
    }
}

/// The folder the app was started in (`physical`, as the system has it, its links
/// resolved), as the shell that started it calls it: `pwd` (`$PWD`) when that is the
/// same folder, which through a link is the path the user sees in the shell.
pub fn working_folder(pwd: Option<&OsStr>, physical: PathBuf) -> PathBuf {
    match pwd.map(Path::new) {
        Some(logical)
            if logical.is_absolute()
                && logical.canonicalize().ok() == physical.canonicalize().ok() =>
        {
            logical.to_path_buf()
        }
        _ => physical,
    }
}

/// `path`, from `cwd` if it is relative and `cwd` is known, without its `.` steps.
fn whole(path: PathBuf, cwd: Option<&Path>) -> PathBuf {
    match cwd {
        Some(cwd) if path.is_relative() => {
            let mut whole = cwd.to_path_buf();
            whole.extend(path.components().filter(|part| *part != Component::CurDir));
            whole
        }
        _ => path,
    }
}

/// The files of `documents`, all of which must be files: `takes` says what wants
/// them, for when one is not.
fn files_of(documents: Vec<Source>, takes: &str) -> Result<Vec<PathBuf>, String> {
    documents
        .into_iter()
        .map(|document| match document {
            Source::File(path) => Ok(path),
            Source::Url(url) => Err(format!("{takes}, and {url} is a web address")),
            Source::Stdin => Err(format!("{takes}, not - (what is piped in)")),
        })
        .collect()
}

/// What the program is called where it was started from, for its help and its
/// messages: the snap's command in the snap (`SNAP_NAME`), the AppImage's file in
/// an AppImage (`APPIMAGE`), else the name it was started by.
pub fn program_name(
    argv0: Option<&OsStr>,
    snap_name: Option<&OsStr>,
    appimage: Option<&OsStr>,
) -> String {
    let file = |path: &OsStr| {
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    };
    snap_name
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .or_else(|| appimage.and_then(file))
        .or_else(|| {
            let name = file(argv0?)?;
            Some(
                name.strip_suffix(".exe")
                    .map_or(name.clone(), str::to_owned),
            )
        })
        .unwrap_or_else(|| "jsonquery_gui".to_owned())
}

/// What `--version` prints.
pub fn version(program: &str) -> String {
    let built = if cfg!(feature = "mcp") {
        ""
    } else {
        " (built without the MCP server)"
    };
    format!("{program} {}{built}", env!("CARGO_PKG_VERSION"))
}

/// What `--help` prints.
pub fn help(program: &str) -> String {
    let p = program;
    format!(
        "{version}: look at JSON of any size, query it, and work on it with the Tools.

Usage:
  {p} [DOCUMENT] [QUERY_FILE] [OPTIONS]
  {p} DOCUMENT DOCUMENT ...
  {p} --diff LEFT RIGHT
  {p} --merge FILE ...
  {p} --patch DOCUMENT PATCH
  {p} --validate DOCUMENT SCHEMA
  {p} --format FILE
  {p} --create-schema FILE

A DOCUMENT is a file, a web address (http:// or https://), or - for what is
piped in. One is opened; several go to Merge's list, as when they are dropped on
the window. A QUERY_FILE is a file whose name ends in .jq, .jsonpath, .jmespath
or .jsonptr: its query goes in the query box.

--diff, --merge, --patch, --validate, --format and --create-schema fill that
tool's page with the files, in the order of its boxes, and run it once they are
all filled. A patch is a JSON Patch or a merge patch.

Options:
  -q, --query QUERY       put QUERY in the query box, and run it on the document
  -l, --lang LANGUAGE     QUERY's language: jq, jsonpath, jmespath or pointer
                          (without it, the language is told from QUERY)
  -p, --pointer POINTER   show the value at a JSON Pointer, such as /items/0
  -h, --help              print this help
  -V, --version           print the version

Examples:
  {p} data.json --query \".items[] | .name\"
  curl -s https://api.github.com/repos/jqlang/jq | {p} -
  git difftool -y -x \"{p} --diff\" -- data.json
",
        version = version(program),
    )
}

/// Make what is printed reach the terminal the app was started from. On Windows
/// the app is a program with windows, not one of the console, and has no console
/// of its own: unless what it prints is sent somewhere (`> file`, a pipe), it
/// borrows the console of the terminal it was started from, if it was started
/// from one. (The terminal does not wait for it: its prompt comes back first.)
#[cfg(windows)]
pub fn use_the_terminal() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> isize;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
    // SAFETY: neither takes a pointer; a handle that is not there is 0 (or -1).
    unsafe {
        if matches!(GetStdHandle(STD_OUTPUT_HANDLE), 0 | -1) {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

/// Elsewhere what the app prints goes where its output goes.
#[cfg(not(windows))]
pub fn use_the_terminal() {}

#[cfg(test)]
mod tests {
    use super::*;

    const CWD: &str = if cfg!(windows) { r"C:\work" } else { "/work" };

    /// The path of `name` in [`CWD`].
    fn at(name: &str) -> PathBuf {
        Path::new(CWD).join(name)
    }

    fn args(line: &[&str]) -> Vec<OsString> {
        line.iter().map(OsString::from).collect()
    }

    /// What `line` asks for, a file of a query holding "the query in NAME".
    fn ask(line: &[&str]) -> Result<Ask, String> {
        parse(args(line), Some(Path::new(CWD)), |path| {
            Ok(format!(
                "the query in {}",
                path.file_name().unwrap().to_string_lossy()
            ))
        })
    }

    fn launch(line: &[&str]) -> Launch {
        match ask(line) {
            Ok(Ask::Run(launch)) => launch,
            other => panic!("{line:?} is not a start: {other:?}"),
        }
    }

    fn error(line: &[&str]) -> String {
        match ask(line) {
            Err(error) => error,
            other => panic!("{line:?} is not an error: {other:?}"),
        }
    }

    fn query(text: &str, language: Option<Kind>) -> Option<Query> {
        Some(Query {
            text: text.to_owned(),
            language,
            file: None,
        })
    }

    #[test]
    fn nothing_opens_the_window_as_it_always_has() {
        assert_eq!(launch(&[]), Launch::default());
    }

    #[test]
    fn help_and_version_print_wherever_they_are() {
        for line in [
            &["--help"][..],
            &["-h"],
            &["data.json", "--help"],
            &["--help", "--bogus"],
        ] {
            assert_eq!(ask(line), Ok(Ask::Help), "{line:?}");
        }
        for line in [&["--version"][..], &["-V"], &["--diff", "-V"]] {
            assert_eq!(ask(line), Ok(Ask::Version), "{line:?}");
        }
    }

    #[test]
    fn a_document_is_opened_from_where_the_app_was_started() {
        assert_eq!(
            launch(&["data.json"]).open,
            Some(Source::File(at("data.json")))
        );
        assert_eq!(
            launch(&["./sub/data.json"]).open,
            Some(Source::File(at("sub").join("data.json")))
        );
        let elsewhere = if cfg!(windows) {
            r"D:\x\a.json"
        } else {
            "/x/a.json"
        };
        assert_eq!(
            launch(&[elsewhere]).open,
            Some(Source::File(elsewhere.into()))
        );
        // Where the app was started is not known: the path is left as it is.
        let line = parse(args(&["data.json"]), None, |_| unreachable!());
        assert_eq!(
            line,
            Ok(Ask::Run(Launch {
                open: Some(Source::File("data.json".into())),
                ..Launch::default()
            }))
        );
    }

    #[test]
    fn a_web_address_is_downloaded_and_a_file_address_is_a_file() {
        let url = "https://example.com/data.json?x=1";
        assert_eq!(launch(&[url]).open, Some(Source::Url(url.to_owned())));
        assert_eq!(
            launch(&["HTTP://example.com/a"]).open,
            Some(Source::Url("HTTP://example.com/a".to_owned()))
        );
        let (address, path) = if cfg!(windows) {
            ("file:///C:/My%20Files/a.json", r"C:/My Files/a.json")
        } else {
            (
                "file:///home/me/My%20Files/a.json",
                "/home/me/My Files/a.json",
            )
        };
        assert_eq!(launch(&[address]).open, Some(Source::File(path.into())));
    }

    #[test]
    fn a_dash_is_what_is_piped_in_and_two_dashes_end_the_options() {
        assert_eq!(launch(&["-"]).open, Some(Source::Stdin));
        assert_eq!(
            launch(&["--", "-q"]).open,
            Some(Source::File(at("-q"))),
            "a file called -q"
        );
        assert_eq!(launch(&["--", "-"]).open, Some(Source::Stdin));
    }

    #[test]
    fn a_query_goes_in_the_box_in_its_language_or_in_the_one_told_from_it() {
        let started = launch(&["data.json", "-q", ".items[]"]);
        assert_eq!(started.open, Some(Source::File(at("data.json"))));
        assert_eq!(started.query, query(".items[]", None));
        assert_eq!(
            launch(&["--query=.a=1", "x.json"]).query,
            query(".a=1", None)
        );
        assert_eq!(
            launch(&["x.json", "--query", "$.a", "--lang", "JSONPath"]).query,
            query("$.a", Some(Kind::JsonPath))
        );
        for (name, language) in [
            ("jq", Kind::Jq),
            ("jmespath", Kind::JmesPath),
            ("json-pointer", Kind::JsonPointer),
            ("pointer", Kind::JsonPointer),
        ] {
            assert_eq!(
                launch(&["-q", "a", "-l", name]).query,
                query("a", Some(language)),
                "{name}"
            );
        }
        // A query without a document is put in the box, for the one that is opened.
        assert_eq!(
            launch(&["-q", ".a"]),
            Launch {
                query: query(".a", None),
                ..Launch::default()
            }
        );
    }

    #[test]
    fn a_pointer_is_a_query_in_the_pointer_language() {
        assert_eq!(
            launch(&["data.json", "--pointer", "/items/0"]).query,
            query("/items/0", Some(Kind::JsonPointer))
        );
        assert_eq!(
            launch(&["-p", ""]).query,
            query("", Some(Kind::JsonPointer))
        );
        assert!(error(&["-p", "items/0"]).contains("starts with /"));
    }

    #[test]
    fn a_file_of_a_query_goes_in_the_box_as_when_it_is_dropped() {
        let started = launch(&["data.json", "names.jq"]);
        assert_eq!(started.open, Some(Source::File(at("data.json"))));
        assert_eq!(
            started.query,
            Some(Query {
                text: "the query in names.jq".to_owned(),
                language: Some(Kind::Jq),
                file: Some(at("names.jq")),
            })
        );
        assert_eq!(
            launch(&["q.jsonptr"]).query.map(|q| q.language),
            Some(Some(Kind::JsonPointer))
        );
        let unreadable = parse(args(&["a.json", "gone.jq"]), Some(Path::new(CWD)), |_| {
            Err("gone.jq could not be read".to_owned())
        });
        assert_eq!(unreadable, Err("gone.jq could not be read".to_owned()));
    }

    #[test]
    fn several_documents_go_to_merges_list() {
        assert_eq!(
            launch(&["a.json", "b.json", "c.json"]),
            Launch {
                merge: vec![at("a.json"), at("b.json"), at("c.json")],
                ..Launch::default()
            }
        );
        assert!(error(&["a.json", "https://x.org/b.json"])
            .contains("takes files, and https://x.org/b.json is a web address"));
        assert!(error(&["a.json", "-"]).contains("not - (what is piped in)"));
        assert!(error(&["a.json", "b.json", "-q", "."]).contains("a query runs on one document"));
    }

    #[test]
    fn a_tool_takes_the_files_in_the_order_of_its_boxes() {
        assert_eq!(
            launch(&["--diff", "old.json", "new.json"]),
            Launch {
                tool: Some((Tool::Diff, vec![at("old.json"), at("new.json")])),
                ..Launch::default()
            }
        );
        assert_eq!(
            launch(&["doc.json", "--validate", "schema.json"]).tool,
            Some((Tool::Validate, vec![at("doc.json"), at("schema.json")]))
        );
        assert_eq!(
            launch(&["--patch", "doc.json", "fix.json"]).tool,
            Some((Tool::Patch, vec![at("doc.json"), at("fix.json")]))
        );
        assert_eq!(
            launch(&["--merge", "a.json", "b.json", "c.json"]).tool,
            Some((Tool::Merge, vec![at("a.json"), at("b.json"), at("c.json")]))
        );
        assert_eq!(
            launch(&["--format", "a.json"]).tool,
            Some((Tool::Format, vec![at("a.json")]))
        );
        assert_eq!(
            launch(&["--create-schema", "a.json"]).tool,
            Some((Tool::Create, vec![at("a.json")]))
        );
        // A file of a query is a file like any other for a tool.
        assert_eq!(
            launch(&["--format", "q.jq"]).tool,
            Some((Tool::Format, vec![at("q.jq")]))
        );
        // Fewer files than boxes fill what they fill; none shows the page.
        assert_eq!(
            launch(&["--diff", "a.json"]).tool,
            Some((Tool::Diff, vec![at("a.json")]))
        );
        assert_eq!(launch(&["--validate"]).tool, Some((Tool::Validate, vec![])));
    }

    #[test]
    fn what_cannot_be_done_is_said() {
        for (line, says) in [
            (&["--bogus"][..], "--bogus is not an option"),
            (&["-x"], "-x is not an option"),
            (&["-q"], "-q needs a query after it"),
            (&["--lang"], "--lang needs a language after it"),
            (
                &["-q", ".a", "-l", "sql"],
                "--lang takes jq, jsonpath, jmespath or pointer",
            ),
            (&["-q", ".a", "--query", ".b"], "--query was given twice"),
            (&["-l", "jq"], "give --query too"),
            (&["-q", ".a", "-p", "/a"], "there is one query box"),
            (&["-p", "/a", "-l", "jq"], "--lang is for --query"),
            (&["-q", ".a", "x.jq"], "one query goes in the query box"),
            (&["a.jq", "b.jq"], "several files of queries"),
            (&["-", "-"], "- (what is piped in) was given twice"),
            (
                &["--diff", "a", "b", "c"],
                "--diff takes 2 files, and 3 were given",
            ),
            (
                &["--format", "a", "b"],
                "--format takes one file, and 2 were given",
            ),
            (
                &["--diff", "--patch"],
                "--diff and --patch cannot both be given",
            ),
            (&["--diff", "--diff"], "--diff was given twice"),
            (&["--diff=a"], "--diff takes no value"),
            (
                &["--diff", "a", "https://x.org/b"],
                "--diff takes files, and https://x.org/b is a web address",
            ),
            (
                &["--validate", "-", "s.json"],
                "--validate takes files, not - (what is piped in)",
            ),
            (
                &["--diff", "a", "b", "-q", "."],
                "--diff fills a tool's page",
            ),
        ] {
            let error = error(line);
            assert!(error.contains(says), "{line:?}: {error:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_text_is_still_a_path() {
        use std::os::unix::ffi::OsStringExt;
        let name = OsString::from_vec(b"caf\xe9.json".to_vec());
        let started = parse(
            vec![name.clone()],
            Some(Path::new("/w")),
            |_| unreachable!(),
        );
        assert_eq!(
            started,
            Ok(Ask::Run(Launch {
                open: Some(Source::File(Path::new("/w").join(&name))),
                ..Launch::default()
            }))
        );
        // A value of an option has to be text.
        let error = parse(vec!["-q".into(), name], None, |_| unreachable!());
        assert_eq!(error, Err("what follows -q is not text".to_owned()));
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_started_in_is_the_shells_where_that_is_the_same_folder() {
        let dir = crate::scratch_dir::ScratchDir::new("cli", "working-folder");
        let real = dir.join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let physical = real.canonicalize().unwrap();
        let folder =
            |pwd: Option<&Path>| working_folder(pwd.map(Path::as_os_str), physical.clone());
        assert_eq!(
            folder(Some(&link)),
            link,
            "through the link, as the shell says"
        );
        assert_eq!(
            folder(Some(dir.as_ref())),
            physical,
            "another folder is not believed"
        );
        assert_eq!(
            folder(Some(Path::new("link"))),
            physical,
            "nor a relative one"
        );
        assert_eq!(folder(None), physical);
    }

    #[test]
    fn the_program_is_called_what_it_was_started_as() {
        let os = |s: &'static str| Some(OsStr::new(s));
        assert_eq!(
            program_name(os("/usr/bin/jsonquery_gui"), None, None),
            "jsonquery_gui"
        );
        assert_eq!(
            program_name(os(r"C:\Apps\jsonquery_gui.exe"), None, None)
                .replace('\\', "/")
                .rsplit('/')
                .next(),
            Some("jsonquery_gui")
        );
        assert_eq!(
            program_name(os("/snap/x/bin/jsonquery_gui"), os("jsonquery-gui"), None),
            "jsonquery-gui"
        );
        assert_eq!(
            program_name(
                os("/tmp/.mount/usr/bin/jsonquery_gui"),
                None,
                os("/home/me/jsonquery_gui-0.5.4-x86_64.AppImage")
            ),
            "jsonquery_gui-0.5.4-x86_64.AppImage"
        );
        assert_eq!(program_name(None, os(""), None), "jsonquery_gui");
    }

    #[test]
    fn the_help_names_the_program_and_every_option() {
        let help = help("jq-app");
        assert!(
            help.starts_with(&format!("jq-app {}", env!("CARGO_PKG_VERSION"))),
            "{help}"
        );
        for option in [
            "--query",
            "--lang",
            "--pointer",
            "--help",
            "--version",
            "-q,",
            "-l,",
            "-p,",
            "-h,",
            "-V,",
        ]
        .into_iter()
        .chain(TOOLS.iter().map(|(flag, ..)| *flag))
        {
            assert!(help.contains(option), "{option}");
        }
        assert!(help.contains("jq-app --diff LEFT RIGHT"));
    }
}
