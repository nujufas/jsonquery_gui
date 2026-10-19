//! Cases taken from jq 1.8.1 itself, a line each in `cases/jq_1_8_1.jsonl`: a document, a
//! program, and either what jq printed for it (compact, one text for each result) or that it
//! failed. They are the writes, deletes, `scan`, `join`, `from_entries` and `@base64d` that
//! were compared with jq on the day those were made to work as jq's do: the cases somebody
//! thought of, and some hundreds made at random, of which these are the ones that came out
//! the same (the others are the known issues, and a difference in how jq and serde write a
//! number or an escape). They are run here, so that they hold on a machine without jq, and
//! so that a new jaq, or a change of the copy of jaq-json in `third_party`, is found out if
//! it changes one.
//!
//! `cases/add_case.py` appends a case, with jq's answer to it, to the file.

use serde_json::Value;

use super::tests::run;

const CASES: &str = include_str!("cases/jq_1_8_1.jsonl");

#[test]
fn every_case_taken_from_jq_1_8_1_comes_out_as_it_did_there() {
    let (mut successes, mut failures) = (0, 0);
    for (at, line) in CASES
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.is_empty())
    {
        let line_no = at + 1;
        let case: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("cases/jq_1_8_1.jsonl line {line_no}: {error}"));
        let (input, query) = (&case["in"], case["q"].as_str().expect("a program"));
        let outcome = run(input, query);
        match case.get("out") {
            Some(Value::Array(expected)) => {
                let expected: Vec<&str> = expected
                    .iter()
                    .map(|output| output.as_str().expect("a text for each output"))
                    .collect();
                let got = outcome.unwrap_or_else(|error| {
                    panic!("line {line_no}: `{query}` over {input} fails ({error}), and jq gives {expected:?}")
                });
                let got: Vec<String> = got
                    .iter()
                    .map(|output| serde_json::to_string(output).unwrap())
                    .collect();
                assert_eq!(got, expected, "line {line_no}: `{query}` over {input}");
                successes += 1;
            }
            _ => {
                assert!(
                    outcome.is_err(),
                    "line {line_no}: `{query}` over {input} gives {outcome:?}, and jq fails"
                );
                failures += 1;
            }
        }
    }
    // (a file that lost most of its lines would still pass the loop)
    assert!(
        successes > 700 && failures > 250,
        "{successes} and {failures}"
    );
}
