//! The lazy tree against `serde_json`, which it has to agree with: on what is
//! JSON (and what is not, down to the words of the error), on what is in every
//! node, and on the text a node is written as.

use serde_json::{json, Value};

use super::*;

// ---- the oracle -----------------------------------------------------------

/// What `Document` makes of bytes when it parses them whole: no value is an
/// empty array, several are an array of them.
fn parse_whole(bytes: &[u8]) -> Result<(Value, usize), String> {
    let mut values = Vec::new();
    for value in serde_json::Deserializer::from_slice(bytes).into_iter::<Value>() {
        values.push(value.map_err(|e| e.to_string())?);
    }
    let count = values.len();
    Ok(match count {
        0 => (Value::Array(Vec::new()), 0),
        1 => (values.pop().unwrap(), 1),
        _ => (Value::Array(values), count),
    })
}

/// An index with a checkpoint nearly everywhere, so that small documents are
/// walked the way big ones are.
fn fine() -> IndexConfig {
    IndexConfig {
        min_children: 2,
        min_bytes: 12,
        cp_children: 2,
        cp_bytes: 16,
    }
}

fn configs() -> [IndexConfig; 3] {
    [
        IndexConfig::default(),
        fine(),
        // Checkpoints by bytes alone, and every container indexed.
        IndexConfig {
            min_children: 1,
            min_bytes: 1,
            cp_children: usize::MAX,
            cp_bytes: 3,
        },
    ]
}

/// Check that `bytes` is treated the same by `serde_json` and by the lazy tree.
fn agree(bytes: &[u8]) {
    let oracle = parse_whole(bytes);
    for cfg in configs() {
        let lazy = LazyTree::from_vec_with(bytes.to_vec(), cfg);
        match (&oracle, &lazy) {
            (Ok((value, count)), Ok(tree)) => {
                assert_eq!(tree.top_level_values(), *count, "{:?}", show(bytes));
                let root = tree.root();
                assert_eq!(
                    &root.to_value(usize::MAX).unwrap(),
                    value,
                    "{:?}",
                    show(bytes)
                );
                same_everywhere(root, value, cfg, &show(bytes));
            }
            (Err(expected), Err(got)) => {
                assert_eq!(&got.to_string(), expected, "{:?}", show(bytes));
            }
            (Ok(_), Err(got)) => panic!("{:?} is JSON but was refused: {got}", show(bytes)),
            (Err(expected), Ok(_)) => {
                panic!(
                    "{:?} is not JSON ({expected}) but was accepted",
                    show(bytes)
                )
            }
        }
    }
}

fn show(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    if text.chars().count() > 200 {
        format!("{}…", text.chars().take(200).collect::<String>())
    } else {
        text.into_owned()
    }
}

/// Walk the node and the value together and compare everything that can be
/// asked of one.
fn same_everywhere(node: Node<'_>, value: &Value, cfg: IndexConfig, context: &str) {
    assert_eq!(node.kind(), ValueKind::of(value), "{context}");
    match value {
        Value::Array(items) => {
            assert_eq!(node.child_count(), items.len(), "{context}");
            let children: Vec<_> = node.children().collect();
            assert_eq!(children.len(), items.len(), "{context}");
            for (i, (child, item)) in children.iter().zip(items).enumerate() {
                assert_eq!(child.index, i, "{context}");
                assert!(child.key.is_none());
                same_everywhere(child.node, item, cfg, context);
                // Found by number as well as by walking.
                let found = node
                    .child(i)
                    .unwrap_or_else(|| panic!("child {i} of {context}"));
                assert_eq!(
                    found.node.byte_range(),
                    child.node.byte_range(),
                    "{context}"
                );
                let from = node.children_from(i).next().unwrap();
                assert_eq!(from.node.byte_range(), child.node.byte_range(), "{context}");
            }
            assert!(node.child(items.len()).is_none(), "{context}");
            assert!(node.child_by_key("a").is_none());
        }
        Value::Object(map) => {
            assert_eq!(node.child_count(), map.len(), "{context}");
            let children: Vec<_> = node.children().collect();
            assert_eq!(children.len(), map.len(), "{context}");
            for (i, (child, (key, item))) in children.iter().zip(map).enumerate() {
                assert_eq!(child.index, i, "{context}");
                assert_eq!(child.key.unwrap().to_string(), *key, "{context}");
                assert!(child.key.unwrap().is(key), "{context}");
                same_everywhere(child.node, item, cfg, context);
                let found = node.child_by_key(key).unwrap();
                assert_eq!(
                    found.node.byte_range(),
                    child.node.byte_range(),
                    "{context}"
                );
                let by_number = node.child(i).unwrap();
                assert_eq!(by_number.node.byte_range(), child.node.byte_range());
            }
            assert!(node.child(map.len()).is_none(), "{context}");
        }
        scalar => {
            assert_eq!(node.child_count(), 0);
            assert_eq!(node.children().count(), 0);
            assert_eq!(node.scalar().as_ref(), Some(scalar), "{context}");
            let preview = node.scalar_preview().unwrap();
            let parsed = ValueView::scalar_preview(&scalar).unwrap();
            if node.raw().len() <= 1024 + 2 {
                assert_eq!(preview, parsed, "{context}");
            } else {
                // A long string is shown by its beginning, which is the same.
                let beginning = preview.strip_suffix("\"…").unwrap_or_else(|| {
                    panic!("a long string is cut short: {preview:?} ({context})")
                });
                assert!(parsed.starts_with(beginning), "{context}");
            }
        }
    }
    // The node as text is what the value is as text, and as a value again.
    let (text, cut) = node.to_pretty_string(PrettyLimits::default());
    assert!(!cut, "{context}");
    assert_eq!(
        text,
        serde_json::to_string_pretty(value).unwrap(),
        "{context}"
    );
    assert_eq!(&node.to_value(usize::MAX).unwrap(), value, "{context}");
    // The same, with a limit that does not bite.
    let (limited, cut) = node.to_pretty_string(PrettyLimits {
        nodes: usize::MAX / 2,
        bytes: usize::MAX / 2,
        string_bytes: usize::MAX / 2,
    });
    assert!(!cut);
    assert_eq!(limited, text);
    let _ = cfg;
}

// ---- random documents -----------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn chance(&mut self, n: usize) -> bool {
        self.below(n) == 0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn random_string(rng: &mut Rng) -> String {
    const PIECES: &[&str] = &[
        "a",
        "b",
        "Z",
        "0",
        " ",
        "key",
        "été",
        "日本語",
        "😀",
        "\"",
        "\\",
        "/",
        "\n",
        "\t",
        "\r",
        "\u{8}",
        "\u{c}",
        "\u{1}",
        "\u{7f}",
        "\u{2028}",
        "é",
        "x y",
        "ab\"cd",
        "back\\slash",
    ];
    let len = match rng.below(10) {
        0 => 0,
        1..=6 => rng.below(8),
        7 | 8 => rng.below(40),
        _ => 200 + rng.below(300),
    };
    (0..len).map(|_| *rng.pick(PIECES)).collect()
}

fn random_number(rng: &mut Rng) -> Value {
    const TEXTS: &[&str] = &[
        "0",
        "-0",
        "1",
        "-1",
        "42",
        "-17",
        "3.5",
        "-0.25",
        "1e3",
        "1E3",
        "1e+3",
        "1e-3",
        "2.5E-7",
        "123456789012345678901234567890",
        "-9223372036854775808",
        "18446744073709551615",
        "18446744073709551616",
        "0.000001",
        "1.0",
        "1.50",
        "10e400",
        "5e-324",
        "1.7976931348623157e308",
    ];
    let text = if rng.chance(3) {
        rng.pick(TEXTS).to_string()
    } else {
        format!("{}", rng.next() as i64 >> rng.below(60))
    };
    serde_json::from_str(&text).unwrap()
}

fn random_value(rng: &mut Rng, depth: usize, budget: &mut usize) -> Value {
    // A document is at most a few hundred nodes, whatever the dice say.
    let choices = if depth >= 5 || *budget == 0 { 5 } else { 8 };
    *budget = budget.saturating_sub(1);
    match rng.below(choices) {
        0 => Value::Null,
        1 => Value::Bool(rng.chance(2)),
        2 | 3 => random_number(rng),
        4 => Value::String(random_string(rng)),
        5 | 6 => {
            let len = match rng.below(8) {
                0 => 0,
                1..=5 => rng.below(6),
                6 => rng.below(40),
                _ => rng.below(150),
            };
            Value::Array(
                (0..len)
                    .map(|_| random_value(rng, depth + 1, budget))
                    .collect(),
            )
        }
        _ => {
            let len = match rng.below(6) {
                0 => 0,
                1..=4 => rng.below(6),
                _ => rng.below(60),
            };
            let mut map = serde_json::Map::new();
            for n in 0..len {
                // Unique keys: the lazy tree keeps a repeated key where a parsed
                // value keeps one, and that has a test of its own.
                let key = format!("{}#{n}", random_string(rng));
                map.insert(key, random_value(rng, depth + 1, budget));
            }
            Value::Object(map)
        }
    }
}

fn space(rng: &mut Rng, out: &mut Vec<u8>) {
    match rng.below(8) {
        0 => out.push(b' '),
        1 => out.push(b'\n'),
        2 => out.extend_from_slice(b"\r\n  "),
        3 => out.push(b'\t'),
        _ => {}
    }
}

/// `value` as JSON text with white space in odd places, and strings written in
/// several of the ways that are allowed.
fn emit(value: &Value, rng: &mut Rng, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => emit_string(s, rng, out),
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    space(rng, out);
                    out.push(b',');
                }
                space(rng, out);
                emit(item, rng, out);
            }
            space(rng, out);
            out.push(b']');
        }
        Value::Object(map) => {
            out.push(b'{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    space(rng, out);
                    out.push(b',');
                }
                space(rng, out);
                emit_string(key, rng, out);
                space(rng, out);
                out.push(b':');
                space(rng, out);
                emit(item, rng, out);
            }
            space(rng, out);
            out.push(b'}');
        }
    }
}

fn emit_string(s: &str, rng: &mut Rng, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\u{8}' => out.extend_from_slice(b"\\b"),
            '\u{c}' => out.extend_from_slice(b"\\f"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes())
            }
            '/' if rng.chance(3) => out.extend_from_slice(b"\\/"),
            c if rng.chance(12) => {
                // Written as an escape, a pair of them when it takes two.
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let text = if rng.chance(2) {
                        format!("\\u{unit:04x}")
                    } else {
                        format!("\\u{unit:04X}")
                    };
                    out.extend_from_slice(text.as_bytes());
                }
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

fn random_document(rng: &mut Rng) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    // Mostly one value; sometimes none, or several, as NDJSON is.
    let values = match rng.below(12) {
        0 => 0,
        1 | 2 => 2 + rng.below(4),
        _ => 1,
    };
    space(rng, &mut out);
    for _ in 0..values {
        let value = random_value(rng, 0, &mut 400);
        emit(&value, rng, &mut out);
        // Values written one after the other need something between them when
        // they are numbers or words; a line end does for all.
        out.push(b'\n');
        space(rng, &mut out);
    }
    (out, values)
}

fn mutate(rng: &mut Rng, bytes: &mut Vec<u8>) {
    const INTERESTING: &[u8] = b"\"\\,[]{}:0123456789-+.eEtfnul \n\t\r/u\x00\x01\x1f\x80\xc3\xff";
    for _ in 0..1 + rng.below(3) {
        if bytes.is_empty() {
            bytes.push(*rng.pick(INTERESTING));
            continue;
        }
        let at = rng.below(bytes.len());
        match rng.below(6) {
            0 => {
                bytes.remove(at);
            }
            1 => bytes.insert(at, *rng.pick(INTERESTING)),
            2 => bytes[at] = *rng.pick(INTERESTING),
            3 => bytes.truncate(at),
            4 => {
                let end = (at + 1 + rng.below(8)).min(bytes.len());
                let piece = bytes[at..end].to_vec();
                bytes.splice(at..at, piece);
            }
            _ => bytes[at] ^= 1 << rng.below(8),
        }
    }
}

// ---- tests ----------------------------------------------------------------

#[test]
fn agrees_on_hand_picked_documents() {
    let documents: &[&str] = &[
        "",
        "   \n ",
        "null",
        "true",
        "false",
        "0",
        "-0",
        "-1.5e10",
        "1E5",
        "\"text\"",
        "\"\"",
        "[]",
        "{}",
        "[[]]",
        "[{}]",
        "{\"a\": []}",
        "[1, 2, 3]",
        "[1,2,3]",
        " [ 1 , 2 , 3 ] ",
        "{\"a\":1,\"b\":[true,false,null],\"c\":{\"d\":\"e\"}}",
        "1 2 3",
        "1\n2\n3\n",
        "[1][2]{}\"x\"",
        "{\"a\":1}\n{\"a\":2}\n",
        "\"a\\nb\\u00e9\\ud83d\\ude00\"",
        "\"\\/\\\"\\\\\\b\\f\\n\\r\\t\"",
        "123456789012345678901234567890",
        "[1.0, 1.50, 1e400]",
        // not JSON
        "x",
        "nul",
        "nulll",
        "truefalse",
        "1a",
        "1 a",
        "[1,]",
        "[,1]",
        "[1 2]",
        "[1",
        "[1,",
        "[",
        "]",
        "{",
        "{\"a\"",
        "{\"a\":",
        "{\"a\":1",
        "{\"a\":1,",
        "{\"a\":1,}",
        "{a:1}",
        "{\"a\" 1}",
        "{1:2}",
        "\"abc",
        "\"a\nb\"",
        "\"a\tb\"",
        "\"\\x\"",
        "\"\\u12\"",
        "\"\\u12G4\"",
        "\"\\ud800\"",
        "\"\\ud800\\u0041\"",
        "\"\\udc00\"",
        "\"\\ud800x\"",
        "01",
        "-",
        "-a",
        "1.",
        "1.e5",
        ".5",
        "1e",
        "1e+",
        "+1",
        "0x10",
        "[0123]",
        "'a'",
        "[1] x",
        "[1]]",
        "{}}",
        "/* c */ 1",
        "\u{feff}1",
        "\u{feff}[1]",
    ];
    for text in documents {
        agree(text.as_bytes());
    }
    // Bytes that are not UTF-8, in a string and out of one.
    agree(b"\"\xc3\"");
    agree(b"\"\xff\"");
    agree(b"\"a\xe2\x82\"");
    agree(b"\"\xc3\\n\xa9\"");
    agree(b"\xff");
    agree(b"[\"\xf0\x9f\x98\x80\"]");
}

#[test]
fn agrees_on_random_documents() {
    let mut rng = Rng(7);
    for _ in 0..300 {
        let (bytes, _) = random_document(&mut rng);
        agree(&bytes);
    }
}

#[test]
fn agrees_on_random_documents_that_are_damaged() {
    let mut rng = Rng(99);
    let mut refused = 0;
    for round in 0..1500 {
        let (mut bytes, _) = random_document(&mut rng);
        if round % 4 != 0 {
            mutate(&mut rng, &mut bytes);
        }
        if LazyTree::from_vec(bytes.clone()).is_err() {
            refused += 1;
        }
        agree(&bytes);
    }
    // Most of the damaged ones are not JSON any more; the test means little if
    // none of them was.
    assert!(refused > 250, "only {refused} were refused");
}

#[test]
fn nesting_is_refused_where_serde_json_refuses_it() {
    for depth in [1, 2, 63, 64, 126, 127, 128, 129, 130, 200, 1000] {
        agree("[".repeat(depth).as_bytes());
        let closed = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        agree(closed.as_bytes());
        let objects = format!("{}1{}", "{\"a\":".repeat(depth), "}".repeat(depth));
        agree(objects.as_bytes());
    }
    // And the limit itself is where the scan thinks it is.
    let at_limit = format!(
        "{}{}",
        "[".repeat(scan::MAX_DEPTH),
        "]".repeat(scan::MAX_DEPTH)
    );
    assert!(LazyTree::from_vec(at_limit.into_bytes()).is_ok());
    let past = format!(
        "{}{}",
        "[".repeat(scan::MAX_DEPTH + 1),
        "]".repeat(scan::MAX_DEPTH + 1)
    );
    assert!(LazyTree::from_vec(past.into_bytes()).is_err());
}

#[test]
fn a_repeated_key_is_there_once_with_the_last_value_in_the_place_of_the_first() {
    let tree = LazyTree::from_vec(br#"{"a": 1, "b": 2, "a": 3}"#.to_vec()).unwrap();
    let root = tree.root();
    // As parsed: two keys, `a` first, holding 3.
    assert_eq!(root.child_count(), 2);
    assert_eq!(
        root.child_by_key("a").unwrap().node.scalar(),
        Some(json!(3))
    );
    assert_eq!(
        root.child_by_key("b").unwrap().node.scalar(),
        Some(json!(2))
    );
    let members: Vec<_> = root
        .children()
        .map(|c| (c.key.unwrap().to_string(), c.node.scalar().unwrap()))
        .collect();
    assert_eq!(
        members,
        [("a".to_owned(), json!(3)), ("b".to_owned(), json!(2))]
    );
    assert_eq!(root.child(1).unwrap().node.scalar(), Some(json!(2)));
    assert!(root.child(2).is_none());
    assert_eq!(root.children_from(1).count(), 1);
    assert_eq!(
        root.to_pretty_string(PrettyLimits::default()).0,
        serde_json::to_string_pretty(&json!({"a": 3, "b": 2})).unwrap()
    );
}

/// JSON text of nested lists and objects whose keys come from a few, so that they
/// repeat — now and then written another way (`\u0061` for `a`) — at every size.
fn text_with_repeated_keys(rng: &mut Rng, depth: usize, out: &mut String) {
    const KEYS: [&str; 7] = ["a", "b", "c", "\\u0061", "b\\u0062", "k", "\\u00e9"];
    match rng.below(if depth > 3 { 4 } else { 7 }) {
        0 => out.push_str(&rng.below(100).to_string()),
        1 => out.push_str("\"text\""),
        2 => out.push_str("null"),
        3 => out.push_str(if rng.chance(2) { "true" } else { "[]" }),
        4 | 5 => {
            out.push('{');
            let members = match rng.below(5) {
                0 => 0,
                1..=3 => 1 + rng.below(6),
                // Many, which is a longer search for a repeat.
                _ => 20 + rng.below(40),
            };
            for n in 0..members {
                if n > 0 {
                    out.push(',');
                }
                // Many members with few keys repeat; some objects have no repeat at all.
                if rng.chance(8) {
                    out.push_str(&format!("\"unique{n}\":"));
                } else {
                    out.push_str(&format!("\"{}\":", rng.pick(&KEYS)));
                }
                text_with_repeated_keys(rng, depth + 1, out);
            }
            out.push('}');
        }
        _ => {
            out.push('[');
            for n in 0..rng.below(5) {
                if n > 0 {
                    out.push(',');
                }
                text_with_repeated_keys(rng, depth + 1, out);
            }
            out.push(']');
        }
    }
}

#[test]
fn objects_with_a_repeated_key_are_what_a_parsed_value_is_at_every_size_and_depth() {
    let mut rng = Rng(0x5EED_0FD0_0B1E);
    let mut with_repeats = 0;
    for _ in 0..400 {
        let mut text = String::new();
        text_with_repeated_keys(&mut rng, 0, &mut text);
        // The oracle is the parse, which is what has no key twice.
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let tree = LazyTree::from_vec(text.clone().into_bytes()).unwrap();
        with_repeats += usize::from(!tree.index.repeated.is_empty());
        agree(text.as_bytes());
        assert_eq!(tree.root().to_value(usize::MAX).unwrap(), parsed);
    }
    assert!(with_repeats > 50, "{with_repeats} had one");
}

#[test]
fn a_big_object_with_a_repeated_key_is_read_as_one_without() {
    // Thousands of members, one of them repeated at the very end, and a key that
    // is the same as another's only once its escapes are read.
    let mut text = String::from("{");
    for n in 0..5_000 {
        text.push_str(&format!("\"k{n}\":{n},"));
    }
    text.push_str("\"k17\":\"again\",\"k\\u0034\\u0032\":\"escaped\"}");
    agree(text.as_bytes());
    let tree = LazyTree::from_vec(text.into_bytes()).unwrap();
    let root = tree.root();
    assert_eq!(root.child_count(), 5_000);
    assert_eq!(
        root.child_by_key("k17").unwrap().node.scalar(),
        Some(json!("again"))
    );
    assert_eq!(
        root.child_by_key("k42").unwrap().node.scalar(),
        Some(json!("escaped"))
    );
}

#[test]
fn what_is_found_in_a_document_with_a_repeated_key_is_what_is_in_the_parsed_one() {
    let text = r#"[{"id": 1, "tag": "x", "id": 2}, {"id": 3, "tag": "y"}]"#;
    let value: Value = serde_json::from_str(text).unwrap();
    let tree = LazyTree::from_vec(text.as_bytes().to_vec()).unwrap();
    for needle in ["1", "2", "3", "tag", "id", "x"] {
        assert_eq!(
            search(tree.root(), needle, false).unwrap(),
            crate::tree::search(&value, needle, false).unwrap(),
            "{needle:?}"
        );
    }
}

#[test]
fn keys_written_with_escapes_are_found() {
    let tree = LazyTree::from_vec(br#"{"a\u0062": 1, "x\"y": 2, "\u00e9": 3}"#.to_vec()).unwrap();
    let root = tree.root();
    assert_eq!(
        root.child_by_key("ab").unwrap().node.scalar(),
        Some(json!(1))
    );
    assert_eq!(
        root.child_by_key("x\"y").unwrap().node.scalar(),
        Some(json!(2))
    );
    assert_eq!(
        root.child_by_key("é").unwrap().node.scalar(),
        Some(json!(3))
    );
    assert!(root.child_by_key("a\\u0062").is_none());
}

#[test]
fn a_stream_of_values_is_an_array_of_them() {
    let tree = LazyTree::from_vec(b"{\"a\":1}\n{\"a\":2}\n[3]\n4".to_vec()).unwrap();
    assert_eq!(tree.top_level_values(), 4);
    let root = tree.root();
    assert_eq!(root.kind(), ValueKind::Array);
    assert_eq!(root.child_count(), 4);
    assert_eq!(
        root.child(2).unwrap().node.to_value(usize::MAX).unwrap(),
        json!([3])
    );
    assert_eq!(root.child(3).unwrap().node.scalar(), Some(json!(4)));
    assert_eq!(root.byte_range(), 0..tree.len());

    let empty = LazyTree::from_vec(b"  \n".to_vec()).unwrap();
    assert_eq!(empty.top_level_values(), 0);
    assert_eq!(empty.root().kind(), ValueKind::Array);
    assert_eq!(empty.root().child_count(), 0);
    assert_eq!(empty.root().to_value(usize::MAX).unwrap(), json!([]));
}

#[test]
fn a_big_array_is_found_by_number_through_its_checkpoints() {
    let count = 5_000;
    let mut text = String::from("[");
    for i in 0..count {
        if i > 0 {
            text.push(',');
        }
        text.push_str(&format!("{{\"id\":{i},\"name\":\"item {i}\"}}"));
    }
    text.push(']');
    let tree = LazyTree::from_vec(text.clone().into_bytes()).unwrap();
    let root = tree.root();
    assert_eq!(root.child_count(), count);
    for i in [0, 1, 63, 64, 65, 127, 128, 999, 1000, 4999] {
        let child = root.child(i).unwrap();
        assert_eq!(child.index, i);
        let value = child.node.to_value(usize::MAX).unwrap();
        assert_eq!(value, json!({"id": i, "name": format!("item {i}")}));
    }
    assert!(root.child(count).is_none());
    assert!(root.child(usize::MAX).is_none());

    // Few entries and few checkpoints: the index is a tiny part of the file.
    assert!(
        tree.index_bytes() * 100 < tree.len(),
        "{} bytes of index for {}",
        tree.index_bytes(),
        tree.len()
    );
    // Only the array has an entry: its elements are small.
    assert_eq!(tree.index.containers.len(), 1);
}

#[test]
fn checkpoints_by_bytes_serve_a_few_big_children() {
    // Children of a megabyte each: a checkpoint by number would be far too
    // sparse, so there are some by size.
    let cfg = IndexConfig {
        min_children: 1,
        cp_bytes: 4000,
        ..IndexConfig::default()
    };
    let item = format!("\"{}\"", "x".repeat(2000));
    let text = format!("[{}]", vec![item; 40].join(","));
    let tree = LazyTree::from_vec_with(text.into_bytes(), cfg).unwrap();
    let container = tree.container_at(0).unwrap();
    assert_eq!(container.count, 40);
    // One checkpoint for every two children (two of them are over 4000 bytes).
    assert!(container.cp_len >= 15, "{} checkpoints", container.cp_len);
    for i in 0..40 {
        assert_eq!(
            tree.root().child(i).unwrap().node.string().unwrap().len(),
            2000
        );
    }
}

#[test]
fn what_is_too_big_is_not_made_into_a_value() {
    let tree = LazyTree::from_vec(br#"{"a": [1, 2, 3], "b": "text"}"#.to_vec()).unwrap();
    let a = tree.root().child_by_key("a").unwrap().node;
    assert!(matches!(
        a.to_value(4),
        Err(MaterializeError::TooLarge { bytes: 9 })
    ));
    assert_eq!(a.to_value(9).unwrap(), json!([1, 2, 3]));
}

#[test]
fn a_long_string_is_previewed_in_part() {
    let long = "é".repeat(50_000);
    let tree = LazyTree::from_vec(format!("\"{long}\"").into_bytes()).unwrap();
    let preview = tree.root().scalar_preview().unwrap();
    assert!(preview.ends_with("…"));
    assert!(preview.len() < 3_000, "{}", preview.len());
    assert!(preview.starts_with("\"éé"));

    // Cut where an escape is, it is not cut in two.
    let escapes = "\\u00e9".repeat(10_000);
    let tree = LazyTree::from_vec(format!("\"{escapes}\"").into_bytes()).unwrap();
    let preview = tree.root().scalar_preview().unwrap();
    assert!(preview.starts_with("\"éé"));
    assert!(preview.ends_with("é\"…"));
    // Nor a pair of surrogates.
    let pairs = "\\ud83d\\ude00".repeat(5_000);
    let tree = LazyTree::from_vec(format!("\"{pairs}\"").into_bytes()).unwrap();
    let preview = tree.root().scalar_preview().unwrap();
    assert!(preview.starts_with("\"😀😀"));
    assert!(!preview.contains('\u{fffd}'));
}

#[test]
fn text_is_cut_by_nodes_the_way_a_parsed_value_is() {
    let value = json!({"a": [1, 2, 3, 4, 5], "b": {"c": [6, 7], "d": "x"}, "e": []});
    let tree = LazyTree::from_vec(serde_json::to_vec(&value).unwrap()).unwrap();
    for budget in 0..=14 {
        let (expected, expected_cut) = crate::tree::pretty_print_bounded(&value, budget);
        let (got, cut) = tree.root().to_pretty_string(PrettyLimits {
            nodes: budget,
            ..PrettyLimits::default()
        });
        assert_eq!(got, expected, "budget {budget}");
        assert_eq!(cut, expected_cut, "budget {budget}");
    }
}

#[test]
fn text_is_cut_by_bytes_and_a_long_string_by_its_own_limit() {
    let value = json!({"long": "y".repeat(1000), "rest": [1, 2, 3]});
    let tree = LazyTree::from_vec(serde_json::to_vec(&value).unwrap()).unwrap();
    let (text, cut) = tree.root().to_pretty_string(PrettyLimits {
        string_bytes: 10,
        ..PrettyLimits::default()
    });
    assert!(cut);
    assert!(text.contains("\"yyyyyyyyyy…\""), "{text}");
    assert!(text.contains("\"rest\""));

    let (text, cut) = tree.root().to_pretty_string(PrettyLimits {
        bytes: 20,
        ..PrettyLimits::default()
    });
    assert!(cut);
    assert!(text.len() < 1100, "{}", text.len());
}

#[test]
fn numbers_are_shown_as_serde_json_shows_them() {
    for text in [
        "0",
        "-0",
        "7",
        "-7",
        "1.50",
        "1e5",
        "1E5",
        "1e+5",
        "2.5E-7",
        "-0.0",
        "10e400",
        "123456789012345678901234567890",
        "18446744073709551616",
    ] {
        let tree = LazyTree::from_vec(text.as_bytes().to_vec()).unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        let root = tree.root();
        assert_eq!(
            root.scalar_preview(),
            ValueView::scalar_preview(&&parsed),
            "{text}"
        );
        assert_eq!(
            root.to_pretty_string(PrettyLimits::default()).0,
            serde_json::to_string_pretty(&parsed).unwrap(),
            "{text}"
        );
    }
}

#[test]
fn the_tree_functions_work_on_a_lazy_root() {
    use crate::tree::{flatten_visible, locate, new_expanded_at_root, resolve, search};

    let value = json!({"users": [{"name": "Ada", "tags": ["x", "y"]}, {"name": "Alan"}], "n": 3});
    let tree = LazyTree::from_vec(serde_json::to_vec(&value).unwrap()).unwrap();
    let mut expand = new_expanded_at_root();
    expand.insert(vec![PathSegment::Key("users".into())]);
    expand.insert(vec![
        PathSegment::Key("users".into()),
        PathSegment::Index(0),
    ]);

    let lazy_rows = flatten_visible(tree.root(), &expand);
    let rows = flatten_visible(&value, &expand);
    assert_eq!(lazy_rows.len(), rows.len());
    for (lazy, parsed) in lazy_rows.iter().zip(&rows) {
        assert_eq!(lazy.path, parsed.path);
        assert_eq!(lazy.depth, parsed.depth);
        assert_eq!(lazy.kind, parsed.kind);
        assert_eq!(lazy.child_count, parsed.child_count);
        assert_eq!(lazy.scalar_preview, parsed.scalar_preview);
        assert_eq!(lazy.expanded, parsed.expanded);
    }

    let path = vec![
        PathSegment::Key("users".into()),
        PathSegment::Index(0),
        PathSegment::Key("tags".into()),
        PathSegment::Index(1),
    ];
    assert_eq!(
        resolve(tree.root(), &path).unwrap().scalar(),
        Some(json!("y"))
    );
    assert!(resolve(tree.root(), &vec![PathSegment::Index(0)]).is_none());

    assert_eq!(
        search(tree.root(), "al", false).unwrap(),
        search(&value, "al", false).unwrap()
    );
    assert_eq!(
        search(tree.root(), r"^A\w+$", true).unwrap(),
        search(&value, r"^A\w+$", true).unwrap()
    );
    let found = locate(
        tree.root(),
        &json!("Alan"),
        1,
        &[PathSegment::Key("name".into())],
    );
    assert_eq!(
        found,
        locate(
            &value,
            &json!("Alan"),
            1,
            &[PathSegment::Key("name".into())]
        )
    );
}

fn count_nodes(value: &Value) -> usize {
    1 + match value {
        Value::Array(items) => items.iter().map(count_nodes).sum(),
        Value::Object(map) => map.values().map(count_nodes).sum(),
        _ => 0,
    }
}

#[test]
fn a_sample_has_the_beginning_of_every_list_and_cuts_what_is_long() {
    let items: Vec<Value> = (0..200)
        .map(|i| json!({"id": i, "name": "n".repeat(500), "deep": [[[[[[[[[[[[1]]]]]]]]]]]]}))
        .collect();
    let doc = json!({"items": items, "n": 3, "small": [1, 2]});
    let tree = LazyTree::from_vec(serde_json::to_vec(&doc).unwrap()).unwrap();
    let sample = tree.sample();

    assert_eq!(sample["n"], json!(3));
    assert_eq!(sample["small"], json!([1, 2]));
    let list = sample["items"].as_array().unwrap();
    assert_eq!(list.len(), 50);
    assert_eq!(list[49]["id"], json!(49));
    let name = list[0]["name"].as_str().unwrap();
    assert!(name.ends_with('…') && name.len() < 300, "{}", name.len());
    // Ten levels and no more: what is deeper is empty.
    let mut deep = &list[0]["deep"];
    let mut levels = 0;
    while let Some(inner) = deep.as_array().and_then(|a| a.first()) {
        deep = inner;
        levels += 1;
    }
    assert!(levels <= 10, "{levels} levels");
    // It is made once.
    assert!(std::ptr::eq(tree.sample(), sample));
}

#[test]
fn a_sample_is_small_however_big_the_document_is() {
    let wide: Vec<Value> = (0..300)
        .map(|_| Value::Array((0..300).map(|n| json!(n)).collect()))
        .collect();
    let tree = LazyTree::from_vec(serde_json::to_vec(&wide).unwrap()).unwrap();
    // 4000 nodes at most (and a few that are cut off).
    assert!(
        count_nodes(tree.sample()) <= 4_100,
        "{}",
        count_nodes(tree.sample())
    );
    assert!(count_nodes(tree.sample()) > 100);
}

// ---- search -----------------------------------------------------------------

fn texts_of(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| texts_of(v, out)),
        Value::Object(map) => {
            for (k, v) in map {
                out.push(k.clone());
                texts_of(v, out);
            }
        }
        other => out.push(other.to_string()),
    }
}

#[test]
fn a_search_finds_what_the_search_of_a_value_finds() {
    let fixed = [
        "a", "A", "x y", "é", "É", "ß", "İ", "i", "1", "-", "e+", ".5", "true", "null", "key",
        "\\", "\"", "😀", "#", "0", "tru", "NULL",
    ];
    let regexes = [
        r"^\d+$",
        r"x",
        r"^[a-z]+#\d+$",
        r"(?i)KEY",
        r"\bnull\b",
        r"^$",
        r".+é",
    ];
    let mut rng = Rng(2024);
    let mut compared = 0usize;
    let mut hits = 0usize;
    for _ in 0..250 {
        let (bytes, _) = random_document(&mut rng);
        let (value, _) = parse_whole(&bytes).unwrap();
        let tree = LazyTree::from_vec_with(bytes, fine()).unwrap();
        let mut from_the_document = Vec::new();
        texts_of(&value, &mut from_the_document);
        let mut needles: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
        for _ in 0..4 {
            if let Some(text) = from_the_document.get(rng.below(from_the_document.len().max(1))) {
                // A piece of what is there, and the same in the other case.
                let chars: Vec<char> = text.chars().collect();
                if chars.is_empty() {
                    continue;
                }
                let from = rng.below(chars.len());
                let to = (from + 1 + rng.below(4)).min(chars.len());
                let piece: String = chars[from..to].iter().collect();
                needles.push(piece.to_uppercase());
                needles.push(piece);
            }
        }
        for needle in &needles {
            let expected = crate::tree::search(&value, needle, false).unwrap();
            hits += expected.len();
            assert_eq!(
                search(tree.root(), needle, false).unwrap(),
                expected,
                "{needle:?}"
            );
            compared += 1;
        }
        for re in regexes {
            assert_eq!(
                search(tree.root(), re, true).unwrap(),
                crate::tree::search(&value, re, true).unwrap(),
                "{re:?}"
            );
            compared += 1;
        }
    }
    // The comparison means little if almost nothing was found.
    assert!(
        compared > 5_000 && hits > 2_000,
        "{compared} searches, {hits} hits"
    );
}

#[test]
fn a_search_stops_at_the_same_number_of_hits_and_refuses_the_same_expressions() {
    let items = vec![Value::String("a".into()); 6_000];
    let value = Value::Array(items);
    let tree = LazyTree::from_vec(serde_json::to_vec(&value).unwrap()).unwrap();
    let found = search(tree.root(), "A", false).unwrap();
    assert_eq!(found.len(), 5_000);
    assert_eq!(found, crate::tree::search(&value, "A", false).unwrap());

    assert!(search(tree.root(), "(unclosed", true).is_err());
    assert!(crate::tree::search(&value, "(unclosed", true).is_err());
}

#[test]
fn a_search_finds_a_key_written_with_an_escape_and_a_string_written_with_one() {
    let text = r#"{"café": "naïve", "plain": "Tab\there"}"#;
    let value: Value = serde_json::from_str(text).unwrap();
    let tree = LazyTree::from_vec(text.as_bytes().to_vec()).unwrap();
    for needle in ["café", "CAFÉ", "naïve", "tab\there", "plain", "xyz"] {
        assert_eq!(
            search(tree.root(), needle, false).unwrap(),
            crate::tree::search(&value, needle, false).unwrap(),
            "{needle:?}"
        );
    }
    assert!(!search(tree.root(), "café", false).unwrap().is_empty());
}

// ---- a file that is cut short while it is mapped -------------------------

/// A file of numbers that takes a few pages, and the mapping of it, made in a
/// directory of its own (each test has its own file: one that cuts a file short
/// under another that has it mapped would end that one).
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn mapped_file(tag: &str) -> (std::path::PathBuf, std::fs::File, Mmap) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "jsonquery-guard-{tag}-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("numbers.json");
    let numbers: Vec<String> = (0..40_000).map(|n| n.to_string()).collect();
    std::fs::write(&path, format!("[{}]", numbers.join(","))).unwrap();
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    // SAFETY: the point of the test is what happens when the file is cut short.
    let map = unsafe { Mmap::map(&file) }.unwrap();
    (path, file, map)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_file_cut_short_while_it_is_open_does_not_end_the_process() {
    let (path, file, map) = mapped_file("open");
    let length = map.len();
    let tree = LazyTree::from_mmap(map).unwrap();
    assert!(!tree.damaged());
    assert_eq!(tree.root().child_count(), 40_000);

    // Another process cuts the file in half.
    file.set_len((length / 2) as u64).unwrap();

    // What is before the cut is as it was, and says nothing is wrong...
    assert_eq!(tree.bytes()[0], b'[');
    assert_eq!(
        tree.root().child(10).unwrap().node.scalar(),
        Some(json!(10))
    );
    assert!(!tree.damaged());

    // ... and reading past it, which used to be the end of the process, is
    // zeros, and says so.
    let last = tree.bytes()[length - 1];
    assert_eq!(last, 0);
    assert!(tree.damaged());
    assert!(tree.bytes()[length / 2 + 4096..].iter().all(|&b| b == 0));

    // What is looked at after that does not panic either, whatever it shows.
    let _ = tree.root().child_count();
    let _ = tree.root().to_pretty_string(PrettyLimits::default());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_file_cut_short_before_it_is_indexed_is_an_error_that_says_so() {
    let (path, file, map) = mapped_file("indexing");
    // Cut at a page boundary (of any size of page there is), so that the first
    // byte past the cut is one that is gone, and not zeros of the page it is in.
    file.set_len(16_384).unwrap();
    let error = LazyTree::from_mmap(map).err().expect("an error");
    assert_eq!(error.kind, ErrorKind::FileChanged);
    assert_eq!(
        error.to_string(),
        "the file was changed while it was being read"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_tree_that_is_dropped_stops_being_watched_and_another_is_watched_alone() {
    let (path_a, file_a, map_a) = mapped_file("first");
    let (path_b, file_b, map_b) = mapped_file("second");
    let (length_a, length_b) = (map_a.len(), map_b.len());
    let a = LazyTree::from_mmap(map_a).unwrap();
    let b = LazyTree::from_mmap(map_b).unwrap();

    file_b.set_len(100).unwrap();
    assert_eq!(b.bytes()[length_b - 1], 0);
    assert!(b.damaged());
    // The other is none the worse.
    assert!(!a.damaged());
    assert_eq!(a.bytes()[length_a - 1], b']');

    drop(b);
    file_a.set_len(100).unwrap();
    assert_eq!(a.bytes()[length_a - 1], 0);
    assert!(a.damaged());
    for path in [path_a, path_b] {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
