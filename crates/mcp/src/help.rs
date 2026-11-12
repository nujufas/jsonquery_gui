//! `query_help`: the app's own lessons on its query languages (the tutorial's), and
//! the list of where its jq differs from jq (the About window's "Known issues").

use std::fmt::Write;

use jsonquery_query::tutorial::{self, Lesson};
use jsonquery_query::{known_issues, Kind};

use crate::catalogue::language_key;

pub fn help(language: Option<Kind>, topic: Option<&str>) -> String {
    let topic = topic.map(str::trim).filter(|t| !t.is_empty());
    // What the lessons are searched for is in lower case.
    let needle = topic.map(str::to_lowercase).unwrap_or_default();
    let text = match (language, topic) {
        (_, Some(t)) if is_known_issues(t) => known_issues::markdown(),
        (None, None) => overview(),
        (None, Some(t)) => {
            let found: Vec<(Kind, &Lesson)> = Kind::ALL
                .into_iter()
                .flat_map(|kind| lessons(kind).map(move |lesson| (kind, lesson)))
                .filter(|(_, lesson)| tutorial::lesson_matches(lesson, &needle))
                .collect();
            if found.is_empty() {
                format!("No lesson is about \"{t}\".\n\n{}", overview())
            } else {
                let mut out = String::new();
                for (kind, lesson) in found {
                    let _ = writeln!(out, "[{}]", tutorial::title(kind));
                    lesson_in_full(&mut out, lesson);
                }
                out
            }
        }
        (Some(kind), None) => language_outline(kind),
        (Some(kind), Some(t)) => {
            let found: Vec<&Lesson> = lessons(kind)
                .filter(|lesson| tutorial::lesson_matches(lesson, &needle))
                .collect();
            if found.is_empty() {
                format!(
                    "No {} lesson is about \"{t}\".\n\n{}",
                    tutorial::title(kind),
                    language_outline(kind)
                )
            } else {
                let mut out = format!("{}: lessons about \"{t}\"\n\n", tutorial::title(kind));
                for lesson in found {
                    lesson_in_full(&mut out, lesson);
                }
                out
            }
        }
    };
    text
}

fn is_known_issues(topic: &str) -> bool {
    let t = topic.to_ascii_lowercase();
    t.contains("known") || t.contains("issue") || t.contains("differ")
}

fn lessons(kind: Kind) -> impl Iterator<Item = &'static Lesson> {
    tutorial::topics(kind)
        .iter()
        .flat_map(|topic| topic.lessons.iter())
}

fn overview() -> String {
    let mut out = String::from(
        "The app queries JSON in four languages (the `language` of run_query and set_query):\n\n",
    );
    for kind in Kind::ALL {
        let _ = writeln!(
            out,
            "- {} (\"{}\"): {}\n  {}",
            tutorial::title(kind),
            language_key(kind),
            tutorial::tagline(kind),
            kind.example()
        );
    }
    out.push_str(
        "\nWithout a `language`, a query is taken as jq unless it is plainly another: `$…` is \
         JSONPath, `/…` a JSON Pointer.\n\
         query_help with a `language` gives its lessons; with a `topic` too, the lessons about \
         it. The topic \"known issues\" lists where the app's jq (jaq) differs from jq 1.8.\n",
    );
    out
}

fn language_outline(kind: Kind) -> String {
    let mut out = format!(
        "{} — {}\n\nLessons (query_help with a `topic` gives one in full, with examples):\n",
        tutorial::title(kind),
        tutorial::tagline(kind)
    );
    for topic in tutorial::topics(kind) {
        let _ = writeln!(out, "\n{}", topic.title);
        for lesson in topic.lessons {
            let _ = writeln!(out, "- {}: {}", lesson.title, lesson.summary);
        }
    }
    for lesson in lessons(kind) {
        if let Some(sheet) = &lesson.cheat_sheet {
            let _ = writeln!(out, "\nCheat sheet ({}):", lesson.title);
            for (syntax, meaning) in sheet.rows {
                let _ = writeln!(out, "  {syntax}  —  {meaning}");
            }
        }
    }
    out
}

fn lesson_in_full(out: &mut String, lesson: &Lesson) {
    let _ = writeln!(out, "## {}\n{}", lesson.title, lesson.summary);
    for example in lesson.examples {
        let _ = writeln!(out, "\nExample: {}", example.caption);
        let _ = writeln!(out, "  query: {}", example.query);
        for (part, meaning) in example.parts {
            let _ = writeln!(out, "    {part}  —  {meaning}");
        }
        if example.fails {
            out.push_str("  (this one is meant to fail, to show the error)\n");
        }
        let data = compact(example.data);
        if data.len() <= 300 {
            let _ = writeln!(out, "  on: {data}");
        }
    }
    if let Some(sheet) = &lesson.cheat_sheet {
        for (syntax, meaning) in sheet.rows {
            let _ = writeln!(out, "  {syntax}  —  {meaning}");
        }
    }
    for tip in lesson.tips {
        let _ = writeln!(out, "- {tip}");
    }
    out.push('\n');
}

/// The sample of an example on one line.
fn compact(data: &str) -> String {
    serde_json::from_str::<serde_json::Value>(data)
        .map(|v| v.to_string())
        .unwrap_or_else(|_| data.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A page of the help `text`, from byte `offset` (where the page before said the next
/// one starts): as much as fits in `max_bytes`, ended at the end of a line, and where
/// the next page starts if there is more.
pub fn page(text: &str, offset: usize, max_bytes: usize) -> String {
    if offset >= text.len() {
        return format!(
            "[the help is {} bytes: there is nothing from offset {offset}]",
            text.len()
        );
    }
    let mut start = offset;
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let rest = &text[start..];
    if rest.len() <= max_bytes {
        return rest.to_owned();
    }
    let mut end = max_bytes;
    while !rest.is_char_boundary(end) {
        end -= 1;
    }
    // At the end of a line, if one ends in the second half of the page.
    if let Some(line_end) = rest[..end].rfind('\n').filter(|at| *at > end / 2) {
        end = line_end + 1;
    }
    format!(
        "{}\n[more: query_help with the same language and topic, and offset {}]",
        &rest[..end],
        start + end
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overview_names_each_language_by_its_key() {
        let text = help(None, None);
        for key in ["\"jq\"", "\"jsonpath\"", "\"jmespath\"", "\"pointer\""] {
            assert!(text.contains(key), "{key}");
        }
    }

    #[test]
    fn a_language_lists_its_lessons_and_a_topic_gives_them_in_full() {
        let outline = help(Some(Kind::Jq), None);
        let first = lessons(Kind::Jq).next().unwrap();
        assert!(outline.contains(first.title));
        let full = help(Some(Kind::Jq), Some(first.title));
        assert!(full.contains(&format!("## {}", first.title)), "{full}");
        if let Some(example) = first.examples.first() {
            assert!(full.contains(example.query));
        }
        assert!(help(Some(Kind::Jq), Some("zzzz-nothing")).starts_with("No jq lesson"));
    }

    #[test]
    fn known_issues_are_the_about_windows_list() {
        assert_eq!(help(None, Some("known issues")), known_issues::markdown());
        assert_eq!(
            help(Some(Kind::JmesPath), Some("Known")),
            known_issues::markdown()
        );
    }

    #[test]
    fn the_help_comes_a_page_at_a_time_ended_at_lines_and_all_of_it_is_given() {
        let text = help(Some(Kind::Jq), None);
        assert!(text.len() > 3_000, "{}", text.len());
        let mut offset = 0;
        let mut read = String::new();
        let mut pages = 0;
        loop {
            let page = page(&text, offset, 1_500);
            assert!(page.len() <= 1_500 + 100, "{}", page.len());
            pages += 1;
            match page
                .rsplit_once("\n[more: query_help with the same language and topic, and offset ")
            {
                Some((part, next)) => {
                    assert!(part.ends_with('\n'), "{part:?}");
                    read.push_str(part);
                    offset = next.trim_end_matches(']').parse().unwrap();
                }
                None => {
                    read.push_str(&page);
                    break;
                }
            }
        }
        assert!(pages > 2);
        assert_eq!(read, text);
        assert!(page(&text, text.len(), 1_500).contains("nothing from offset"));
        // A page in the middle of a character starts at its start.
        assert_eq!(page("aé\nb", 2, 100), "é\nb");
    }
}
