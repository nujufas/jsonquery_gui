//! What the tools give of a value too big for a page, asked for whole as a model asks
//! for it: a snippet of it, and the notice that says so, the same in every answer
//! (`jsonquery_mcp::snippet`) — that it is large, the arrays in it, and the call that
//! reads on, which works.

use super::*;

/// `n` people, each about 90 bytes of JSON.
fn people(n: usize) -> Value {
    Value::Array(
        (0..n)
            .map(|i| json!({"id": i, "name": format!("person {i}"), "tags": ["a", "b"], "city": "Lisbon"}))
            .collect(),
    )
}

/// A document of 5,000 people (about 450 KB), and a little more.
fn big_doc() -> Value {
    json!({"people": people(5_000), "meta": {"source": "test"}})
}

/// The notice of `answer`, which it must have, for its member `member`: what every
/// notice says.
fn notice_of(answer: &Value, member: &str) -> Value {
    let notice = answer["snippet"].clone();
    assert_eq!(notice["in"], member, "{answer}");
    let note = notice["note"].as_str().expect("a note");
    assert!(note.starts_with("Large: "), "{note}");
    assert!(
        note.contains(&format!("`{member}` holds a snippet of it, not all of it")),
        "{note}"
    );
    assert!(note.contains("Do not ask for all of it"), "{note}");
    assert!(notice.to_string().len() <= jsonquery_mcp::snippet::NOTICE_BYTES);
    notice
}

/// The call `next` names, read as the server reads a call (so that what `next` says is
/// what works), fitted to the page the user set, and made.
fn follow(rig: &Rig, next: &Value) -> Value {
    let name = next["tool"].as_str().unwrap();
    let mut call = jsonquery_mcp::parse(name, Some(&next["arguments"]))
        .expect("a tool")
        .unwrap();
    call.fit(24_000);
    rig.call(call).unwrap()
}

#[test]
fn a_large_array_asked_for_whole_gives_a_page_and_says_it_is_a_snippet() {
    let rig = Rig::new();
    let text = people(5_000).to_string();
    rig.open(&text);
    let out = rig.call(read(Input::Document, "")).unwrap();
    assert_eq!(out["items"].as_array().unwrap().len(), 20);
    assert_eq!(out["next_offset"], 20);
    let notice = notice_of(&out, "items");
    let note = notice["note"].as_str().unwrap();
    assert!(
        note.contains("It holds arrays: the biggest, at \"\" (the top), has 5000 items, 20 of them given here."),
        "{note}"
    );
    assert_eq!(notice["of"], "(pasted JSON)");
    assert_eq!(notice["is"], "array with 5000 items");
    assert_eq!(notice["bytes"], text.len());
    assert_eq!(notice["tokens_about"], text.len() / 4);
    assert_eq!(
        notice["arrays"],
        json!([{"pointer": "", "items": 5000, "given": 20, "next_offset": 20}])
    );
    assert_eq!(
        notice["next"],
        json!({"tool": "read_value", "arguments": {"input": "document", "pointer": "", "offset": 20}})
    );
    // `next` reads on.
    let on = follow(&rig, &notice["next"]);
    assert_eq!(on["items"][0]["id"], 20);
    // What fits in a page is given whole, and nothing is said.
    rig.open(&people(5).to_string());
    let small = rig.call(read(Input::Document, "")).unwrap();
    assert_eq!(small["items"].as_array().unwrap().len(), 5);
    assert!(small.get("snippet").is_none());
}

#[test]
fn a_large_object_asked_for_whole_gives_its_big_array_cut_down_and_reads_on() {
    let rig = Rig::new();
    rig.open(&big_doc().to_string());
    let out = rig.call(read(Input::Document, "")).unwrap();
    // The first member, too big whole, is a snippet of it: its first people.
    let shown = out["members"]["people"].as_array().unwrap();
    let given = shown.len() - 1;
    assert!(given > 50, "{given}");
    assert_eq!(shown[0]["id"], 0);
    assert_eq!(
        shown[given],
        json!({"$more": format!("{} more items", 5_000 - given), "pointer": "/people", "offset": given})
    );
    assert!(out.to_string().len() <= 24_000 + 24_000 / 4);
    let notice = notice_of(&out, "members");
    assert_eq!(notice["arrays"][0]["pointer"], "/people");
    assert_eq!(notice["arrays"][0]["items"], 5_000);
    assert_eq!(notice["arrays"][0]["given"], given);
    // The object itself is read a page at a time: reading on goes to its next member.
    assert_eq!(notice["objects"][0]["pointer"], "");
    assert_eq!(notice["next"]["arguments"]["offset"], 1);
    let on = follow(&rig, &notice["next"]);
    assert_eq!(on["members"]["meta"], json!({"source": "test"}));
    // The array, read by its pointer from where the snippet stopped.
    let rest = rig
        .call(Call::ReadValue {
            input: Input::Document,
            pointer: "/people".to_owned(),
            offset: given,
            limit: 20,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(rest["items"][0]["id"], given);
}

#[test]
fn run_query_of_all_of_a_large_document_gives_a_snippet_kept_to_read_on() {
    let rig = Rig::new();
    rig.open(&big_doc().to_string());
    // "." is the whole document, as one result.
    let out = rig.call(run(".", 0, 20)).unwrap();
    assert_eq!(out["count"], 1);
    assert_eq!(out["last"], "kept: give \"last\" as an input");
    assert_eq!(out["items"][0]["meta"], json!({"source": "test"}));
    let notice = notice_of(&out, "items");
    assert_eq!(notice["of"], "the results of the query");
    assert_eq!(notice["is"], "1 results");
    assert_eq!(notice["arrays"][0]["pointer"], "/0/people");
    let next = &notice["next"];
    assert_eq!(next["tool"], "read_value");
    assert_eq!(next["arguments"]["input"], "last");
    assert_eq!(next["arguments"]["pointer"], "/0/people");
    let on = follow(&rig, next);
    assert_eq!(on["items"][0]["id"], notice["arrays"][0]["given"]);
    // Many results: a page of them, said to be a snippet; read on in "last".
    let out = rig.call(run(".people[]", 0, 20)).unwrap();
    assert_eq!(out["count"], 5_000);
    let notice = notice_of(&out, "items");
    assert_eq!(
        notice["arrays"],
        json!([{"pointer": "", "items": 5000, "given": 20, "next_offset": 20}])
    );
    assert_eq!(
        notice["next"],
        json!({"tool": "read_value", "arguments": {"input": "last", "pointer": "", "offset": 20}})
    );
    assert_eq!(follow(&rig, &notice["next"])["items"][0]["id"], 20);
    // Results that fit are not said to be large.
    let out = rig.call(run(".people[0:3] | .[].id", 0, 20)).unwrap();
    assert_eq!(out["items"], json!([0, 1, 2]));
    assert!(out.get("snippet").is_none());
}

#[test]
fn the_results_the_user_sees_too_big_for_a_page_say_so() {
    let rig = Rig::new();
    rig.open(&big_doc().to_string());
    let out = rig.call(set(".people[]", None)).unwrap();
    assert_eq!(out["items"].as_array().unwrap().len(), 20);
    let notice = notice_of(&out, "items");
    assert_eq!(notice["of"], "the results the user sees");
    assert_eq!(
        notice["next"],
        json!({"tool": "read_value", "arguments": {"input": "results", "pointer": "", "offset": 20}})
    );
    assert_eq!(follow(&rig, &notice["next"])["items"][0]["id"], 20);
    let page = rig
        .call(Call::Results {
            offset: 40,
            limit: 10,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(page["items"][0]["id"], 40);
    assert_eq!(notice_of(&page, "items")["arrays"][0]["next_offset"], 50);
}

#[test]
fn format_json_of_a_large_document_formats_a_snippet_of_it() {
    let rig = Rig::new();
    rig.open(&big_doc().to_string());
    let out = rig
        .call(Call::Format {
            input: Input::Document,
            options: reformat::Options::default(),
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(out["complete"], false);
    let text = out["text"].as_str().unwrap();
    assert!(text.len() <= 24_000, "{}", text.len());
    // The text is JSON, formatted: the snippet, with what it leaves out in its place.
    assert!(
        text.starts_with("{\n  \"people\": [\n    {\n"),
        "{}",
        &text[..40]
    );
    let snippet: Value = serde_json::from_str(text).unwrap();
    let shown = snippet["people"].as_array().unwrap();
    assert_eq!(shown[0]["id"], 0);
    assert_eq!(shown.last().unwrap()["pointer"], "/people");
    assert_eq!(snippet["meta"], json!({"source": "test"}));
    let notice = notice_of(&out, "text");
    assert!(notice["note"]
        .as_str()
        .unwrap()
        .contains("save_file writes all of it formatted"));
    assert_eq!(notice["next"]["arguments"]["pointer"], "/people");
    assert_eq!(
        follow(&rig, &notice["next"])["items"][0]["id"],
        shown.len() - 1
    );
}

#[test]
fn what_the_tools_make_too_big_for_a_page_is_a_snippet_read_on_in_last() {
    let rig = Rig::new();
    rig.open(&big_doc().to_string());
    let out = rig
        .call(Call::Patch {
            input: Input::Document,
            patch: json_input(
                json!([{"op": "replace", "path": "/meta/source", "value": "patched"}]),
            ),
            merge_patch: None,
            max_bytes: 24_000,
        })
        .unwrap();
    assert_eq!(out["result"]["meta"]["source"], "patched");
    let notice = notice_of(&out, "result");
    assert_eq!(notice["of"], "the patched document");
    assert_eq!(notice["next"]["arguments"]["input"], "last");
    let on = follow(&rig, &notice["next"]);
    assert_eq!(on["items"][0]["id"], notice["arrays"][0]["given"]);
    // A merge too.
    let merged = rig
        .call(Call::Merge {
            inputs: vec![Input::Document, json_input(json!({"more": true}))],
            filter: "add".to_owned(),
            max_bytes: 24_000,
            timeout: WAIT,
        })
        .unwrap();
    assert_eq!(merged["result"]["more"], true);
    notice_of(&merged, "result");
}

#[test]
fn a_large_document_is_said_to_be_large_where_it_is_first_seen() {
    let rig = Rig::new();
    rig.open(&big_doc().to_string());
    let state = rig.call(Call::State).unwrap();
    let large = state["document"]["large"].as_str().unwrap();
    assert!(large.starts_with("Large: (pasted JSON) takes "), "{large}");
    assert!(large.contains("Do not ask for all of it"), "{large}");
    let info = rig
        .call(Call::Info {
            input: Input::Document,
            pointer: String::new(),
            max_bytes: 24_000,
        })
        .unwrap();
    assert!(info["large"].as_str().unwrap().starts_with("Large: "));
    let part = rig
        .call(Call::Info {
            input: Input::Document,
            pointer: "/meta".to_owned(),
            max_bytes: 24_000,
        })
        .unwrap();
    assert!(part.get("large").is_none());
    rig.open("[1, 2]");
    assert!(rig.call(Call::State).unwrap()["document"]
        .get("large")
        .is_none());
}

#[test]
fn a_large_item_of_the_workspace_and_its_sample_are_snippets_read_on_by_their_paths() {
    let dir = ScratchDir::new("mcp", "workspace-large");
    write(&dir, "data/crowd.json", &big_doc().to_string());
    write(&dir, "data/crowd.sample.json", &people(3_000).to_string());
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let out = rig
        .call(Call::WorkspaceRead {
            path: "data/crowd.json".to_owned(),
            sample: true,
            max_bytes: 24_000,
        })
        .unwrap();
    assert!(
        out.to_string().len() <= 24_000 + 24_000 / 4,
        "{}",
        out.to_string().len()
    );
    assert_eq!(out["content"]["meta"], json!({"source": "test"}));
    let notice = notice_of(&out, "content");
    assert_eq!(notice["of"], "data/crowd.json");
    assert_eq!(
        notice["next"]["arguments"]["input"],
        json!({"workspace": "data/crowd.json"})
    );
    let on = follow(&rig, &notice["next"]);
    assert_eq!(on["items"][0]["id"], notice["arrays"][0]["given"]);
    // Its sample, with its own notice, read on by the sample's path.
    assert_eq!(out["sample_path"], "data/crowd.sample.json");
    assert_eq!(out["sample"][0]["id"], 0);
    let sample = out["sample_snippet"].clone();
    assert_eq!(sample["in"], "sample");
    assert_eq!(sample["of"], "data/crowd.sample.json");
    let on = follow(&rig, &sample["next"]);
    assert_eq!(on["items"][0]["id"], sample["arrays"][0]["given"]);
}

#[test]
fn a_large_workspace_file_that_is_not_json_is_said_to_be_so_and_the_call_goes_well() {
    let dir = ScratchDir::new("mcp", "workspace-broken");
    write(
        &dir,
        "data/broken.json",
        &"this is not JSON at all ".repeat(2_000),
    );
    let settings = Settings {
        workspace_folder: Some(dir.to_path_buf()),
        ..Settings::default()
    };
    let rig = Rig::with(settings);
    let out = rig
        .call(Call::WorkspaceRead {
            path: "data/broken.json".to_owned(),
            sample: false,
            max_bytes: 24_000,
        })
        .unwrap();
    let what = out["content"]["$omitted"].as_str().unwrap();
    assert!(what.contains("not read as JSON"), "{what}");
    assert!(out["content"]["file"]
        .as_str()
        .unwrap()
        .ends_with("broken.json"));
    assert!(out.get("snippet").is_none());
}
