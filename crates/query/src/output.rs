//! How a query's results are written out as text: copied to the clipboard,
//! saved to a file, shown in the Results pane's Text view.
//!
//! Results are JSON, and JSON is what gets written — pretty-printed — unless
//! the query asks for something else. A jq query that ends in `@csv` or `@tsv`
//! produces one string per result, each a row of a table. Written as JSON, that
//! is a quoted, escaped string (`"\"Ada\",36"`), which is no use in a
//! spreadsheet; the query asked for CSV, so CSV is what Copy, Save and the Text
//! view give: the rows as they are, one to a line.
//!
//! [`OutputFormat::detect`] reads that off the query text. It looks at the last
//! stage of the program only — what follows the final top-level `|` — because
//! that is what decides what the results are: `.[] | [.a, .b] | @csv` makes CSV
//! rows, while `map(@csv) | length` merely uses `@csv` on the way to a number.
//! A stage it cannot read is JSON, which is always a correct way to write a
//! result.

use std::borrow::Cow;
use std::io::{self, Write};

use serde_json::Value;

use crate::highlight::{is_word, matching_close, skip_comment, skip_string, skip_word};
use crate::Kind;

/// What the results of a query are written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Pretty-printed JSON: every result is a JSON value.
    #[default]
    Json,
    /// Each result is a row of CSV text (the query ends in `@csv`).
    Csv,
    /// Each result is a row of TSV text (the query ends in `@tsv`).
    Tsv,
}

impl OutputFormat {
    /// The format the results of `query` (run by the `kind` engine) are in.
    pub fn detect(kind: Kind, query: &str) -> Self {
        if kind != Kind::Jq {
            return Self::Json;
        }
        let code = without_comments(query);
        let mut stage = final_stage(&code);
        loop {
            // `@csv?` is still a row of CSV; it only drops the rows that fail.
            let bare = stage.trim_matches(|c: char| c == '?' || c.is_whitespace());
            match bare {
                "@csv" => return Self::Csv,
                "@tsv" => return Self::Tsv,
                _ => {}
            }
            // A stage in parentheses is a program of its own: look inside.
            let close = bare
                .starts_with('(')
                .then(|| matching_close(Kind::Jq, bare.as_bytes(), 0));
            match close {
                Some(Some(close)) if close == bare.len() - 1 => {
                    stage = final_stage(&bare[1..close]);
                }
                _ => return Self::Json,
            }
        }
    }

    /// Whether the results are rows of text rather than JSON values.
    pub fn is_tabular(self) -> bool {
        self != Self::Json
    }

    /// The name shown to the user and offered as the save dialog's file type.
    pub fn label(self) -> &'static str {
        match self {
            Self::Json => "JSON",
            Self::Csv => "CSV",
            Self::Tsv => "TSV",
        }
    }

    /// The file extension a saved file gets by default.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
        }
    }
}

/// `query` with its `# comments` taken out (the line breaks stay).
fn without_comments(query: &str) -> Cow<'_, str> {
    if !query.contains('#') {
        return Cow::Borrowed(query);
    }
    let b = query.as_bytes();
    let mut out = String::with_capacity(query.len());
    let (mut i, mut copied) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(Kind::Jq, b, i),
            b'#' => {
                out.push_str(&query[copied..i]);
                i = skip_comment(b, i);
                copied = i;
            }
            _ => i += 1,
        }
    }
    out.push_str(&query[copied..]);
    Cow::Owned(out)
}

/// The last stage of a jq program (comments already removed): the text after
/// the final `|` that is not inside brackets, a string or an `if … end`, and
/// after the final `;` that ends a `def`.
fn final_stage(code: &str) -> &str {
    let b = code.as_bytes();
    let (mut start, mut i) = (0, 0);
    // How many `if`s are open: a `|` or `;` inside one belongs to that branch.
    let mut ifs = 0usize;
    while i < b.len() {
        match b[i] {
            b'"' => i = skip_string(Kind::Jq, b, i),
            b'(' | b'[' | b'{' => {
                i = matching_close(Kind::Jq, b, i).map_or(b.len(), |close| close + 1);
            }
            // A word after one of these is a field, a variable or a format:
            // `.end` is not the keyword.
            b'.' | b'$' | b'@' => i = skip_word(b, i + 1),
            // `|=` assigns; it does not start a stage.
            b'|' if ifs == 0 && b.get(i + 1) == Some(&b'=') => i += 2,
            b'|' | b';' if ifs == 0 => {
                i += 1;
                start = i;
            }
            c if is_word(c) => {
                let end = skip_word(b, i);
                match &code[i..end] {
                    "if" => ifs += 1,
                    "end" => ifs = ifs.saturating_sub(1),
                    _ => {}
                }
                i = end;
            }
            _ => i += 1,
        }
    }
    &code[start..]
}

/// The rows of `results`: one per item of an array (what the Results pane
/// holds), or the one row a single value makes. A string is its own row;
/// anything else — which a query ending in `@csv` never produces, but a value
/// is a value — is written as compact JSON.
pub fn rows(results: &Value) -> Box<dyn Iterator<Item = Cow<'_, str>> + '_> {
    match results {
        Value::Array(items) => Box::new(items.iter().map(row)),
        single => Box::new(std::iter::once(row(single))),
    }
}

fn row(value: &Value) -> Cow<'_, str> {
    match value {
        Value::String(text) => Cow::Borrowed(text),
        other => Cow::Owned(other.to_string()),
    }
}

/// `results` as text, a row to a line with no line break after the last: what
/// "Copy to Clipboard" puts on the clipboard.
pub fn rows_text(results: &Value) -> String {
    rows(results).collect::<Vec<_>>().join("\n")
}

/// Like [`rows_text`] for the first `limit` rows only, and whether there were
/// more: what the Text view shows.
pub fn rows_bounded(results: &Value, limit: usize) -> (String, bool) {
    let mut taken: Vec<Cow<'_, str>> = rows(results).take(limit.saturating_add(1)).collect();
    let truncated = taken.len() > limit;
    taken.truncate(limit);
    (taken.join("\n"), truncated)
}

/// Write `results` a row to a line, each ending in a line break: what a saved
/// file holds. Rows end in a bare `\n`, as `jq -r` writes them; spreadsheets
/// read that as they read any other.
pub fn write_rows(results: &Value, out: &mut impl Write) -> io::Result<()> {
    for row in rows(results) {
        out.write_all(row.as_bytes())?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jq(query: &str) -> OutputFormat {
        OutputFormat::detect(Kind::Jq, query)
    }

    #[test]
    fn a_query_ending_in_csv_or_tsv_makes_rows() {
        assert_eq!(jq(".members[] | [.name, .age] | @csv"), OutputFormat::Csv);
        assert_eq!(jq(".[] | [.a, .b] | @tsv"), OutputFormat::Tsv);
        assert_eq!(jq("@csv"), OutputFormat::Csv);
        assert_eq!(jq("  @tsv \n"), OutputFormat::Tsv);
        assert_eq!(jq(".[]\n| [.a, .b]\n| @csv\n"), OutputFormat::Csv);
    }

    #[test]
    fn it_is_the_last_stage_that_decides() {
        assert_eq!(jq(".[] | [.a] | @csv | length"), OutputFormat::Json);
        assert_eq!(jq("map([.a] | @csv)"), OutputFormat::Json);
        assert_eq!(jq("[.[] | [.a] | @csv] | length"), OutputFormat::Json);
        assert_eq!(jq(".[] | [.a] | @csv, @tsv"), OutputFormat::Json);
        assert_eq!(jq(".[] | [.a] | @csv | ascii_upcase"), OutputFormat::Json);
        assert_eq!(jq(".[] | .name"), OutputFormat::Json);
        assert_eq!(jq(""), OutputFormat::Json);
    }

    #[test]
    fn a_format_string_is_not_a_row() {
        assert_eq!(jq(r#".[] | @csv "\(.a),\(.b)""#), OutputFormat::Json);
    }

    #[test]
    fn a_failure_swallowed_by_a_question_mark_still_makes_rows() {
        assert_eq!(jq(".[] | [.a] | @csv?"), OutputFormat::Csv);
        assert_eq!(jq(".[] | [.a] | @tsv ?"), OutputFormat::Tsv);
    }

    #[test]
    fn parentheses_around_the_last_stage_are_looked_into() {
        assert_eq!(jq(".[] | ([.a] | @csv)"), OutputFormat::Csv);
        assert_eq!(jq("(.[] | [.a]) | @tsv"), OutputFormat::Tsv);
        assert_eq!(jq("((@csv))"), OutputFormat::Csv);
        assert_eq!(jq(".[] | ([.a] | length)"), OutputFormat::Json);
        // Parentheses that close before the end are not around the stage.
        assert_eq!(jq("(.a) + (@csv)"), OutputFormat::Json);
    }

    #[test]
    fn a_comment_does_not_hide_or_fake_the_last_stage() {
        assert_eq!(
            jq(".[] | [.a] | @csv # to a spreadsheet"),
            OutputFormat::Csv
        );
        assert_eq!(jq("# header\n.[] | [.a] | @tsv"), OutputFormat::Tsv);
        assert_eq!(jq(".[] | [.a] | @csv\n# | length"), OutputFormat::Csv);
        assert_eq!(jq(".a | length # | @csv"), OutputFormat::Json);
    }

    #[test]
    fn strings_brackets_and_fields_do_not_start_a_stage() {
        assert_eq!(jq(r#""a | @csv""#), OutputFormat::Json);
        assert_eq!(jq(r#".["x | @tsv"]"#), OutputFormat::Json);
        assert_eq!(jq(r#""\([.a] | @csv)""#), OutputFormat::Json);
        assert_eq!(jq(".a |= (. | @csv)"), OutputFormat::Json);
        assert_eq!(jq("{a: (. | @csv)}"), OutputFormat::Json);
        assert_eq!(jq(".end | [.] | @csv"), OutputFormat::Csv);
        assert_eq!(jq(".a as $end | [$end] | @tsv"), OutputFormat::Tsv);
    }

    #[test]
    fn a_def_is_not_the_program() {
        assert_eq!(
            jq("def row: [.a, .b] | @csv; .[] | [.a, .b] | @csv"),
            OutputFormat::Csv
        );
        // The last stage is a call, which this does not look into.
        assert_eq!(
            jq("def row: [.a, .b] | @csv; .[] | row"),
            OutputFormat::Json
        );
        assert_eq!(jq("def row: [.a] | @csv;"), OutputFormat::Json);
    }

    #[test]
    fn pipes_inside_an_if_belong_to_it() {
        assert_eq!(
            jq("if .a then [.a] | @csv else empty end"),
            OutputFormat::Json
        );
        assert_eq!(
            jq(".[] | if .a then 1 else 2 end | [.] | @csv"),
            OutputFormat::Csv
        );
        // Unclosed, as while it is being typed.
        assert_eq!(jq("if .a then [.a] | @csv"), OutputFormat::Json);
        assert_eq!(jq(".[] | [.a"), OutputFormat::Json);
        assert_eq!(jq(".[] | \"abc"), OutputFormat::Json);
    }

    #[test]
    fn only_jq_has_these_formats() {
        for kind in [Kind::JsonPointer, Kind::JsonPath, Kind::JmesPath] {
            assert_eq!(OutputFormat::detect(kind, "@csv"), OutputFormat::Json);
            assert_eq!(OutputFormat::detect(kind, "a | @tsv"), OutputFormat::Json);
        }
    }

    #[test]
    fn labels_and_extensions() {
        assert_eq!(OutputFormat::default(), OutputFormat::Json);
        assert!(!OutputFormat::Json.is_tabular());
        for (format, label, extension) in [
            (OutputFormat::Json, "JSON", "json"),
            (OutputFormat::Csv, "CSV", "csv"),
            (OutputFormat::Tsv, "TSV", "tsv"),
        ] {
            assert_eq!(format.label(), label);
            assert_eq!(format.extension(), extension);
        }
        assert!(OutputFormat::Csv.is_tabular() && OutputFormat::Tsv.is_tabular());
    }

    #[test]
    fn every_string_result_is_a_row_as_it_is() {
        let results = json!(["\"Ada\",36", "\"Linus\",28"]);
        assert_eq!(rows_text(&results), "\"Ada\",36\n\"Linus\",28");
        let mut file = Vec::new();
        write_rows(&results, &mut file).unwrap();
        assert_eq!(
            String::from_utf8(file).unwrap(),
            "\"Ada\",36\n\"Linus\",28\n"
        );
    }

    #[test]
    fn a_single_string_is_one_row() {
        assert_eq!(rows_text(&json!("a\tb")), "a\tb");
        let mut file = Vec::new();
        write_rows(&json!("a\tb"), &mut file).unwrap();
        assert_eq!(file, b"a\tb\n");
    }

    #[test]
    fn a_result_that_is_not_a_string_is_written_as_json() {
        assert_eq!(
            rows_text(&json!(["a", 1, [2, "x"], null])),
            "a\n1\n[2,\"x\"]\nnull"
        );
    }

    #[test]
    fn nothing_is_nothing() {
        assert_eq!(rows_text(&json!([])), "");
        assert_eq!(rows_bounded(&json!([]), 5), (String::new(), false));
        let mut file = Vec::new();
        write_rows(&json!([]), &mut file).unwrap();
        assert!(file.is_empty());
    }

    #[test]
    fn the_text_view_stops_at_its_limit_and_says_so() {
        let results = json!(["a", "b", "c"]);
        assert_eq!(rows_bounded(&results, 2), ("a\nb".to_owned(), true));
        assert_eq!(rows_bounded(&results, 3), ("a\nb\nc".to_owned(), false));
        assert_eq!(rows_bounded(&results, 9), ("a\nb\nc".to_owned(), false));
        assert_eq!(
            rows_bounded(&results, usize::MAX),
            ("a\nb\nc".to_owned(), false)
        );
        assert_eq!(rows_bounded(&results, 0), (String::new(), true));
    }
}
