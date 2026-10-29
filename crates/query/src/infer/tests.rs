use serde_json::json;

use super::*;

fn make(value: &Value, options: Options) -> Created {
    create(value, options, &AtomicBool::new(false)).expect("a schema is made")
}

fn schema_of(value: Value) -> Value {
    make(&value, Options::default()).schema
}

/// What was found, as (path, what).
fn found(created: &Created) -> Vec<(&str, &str)> {
    created
        .found
        .iter()
        .map(|f| (f.path.as_str(), f.what.as_str()))
        .collect()
}

/// The schema at `pointer`.
fn at<'a>(schema: &'a Value, pointer: &str) -> &'a Value {
    schema
        .pointer(pointer)
        .unwrap_or_else(|| panic!("{pointer} in {schema:#}"))
}

/// Whether the document fits the schema, with formats checked.
fn fits(schema: &Value, document: &Value) -> bool {
    schema::validate(
        schema,
        document,
        schema::Options {
            check_formats: true,
        },
        &AtomicBool::new(false),
    )
    .unwrap()
    .is_valid()
}

#[test]
fn an_object_has_its_members_types_and_which_are_required() {
    let schema =
        schema_of(json!({"name": "Ada", "age": 36, "score": 9.5, "admin": false, "boss": null}));
    assert_eq!(
        schema,
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer"},
                "score": {"type": "number"},
                "admin": {"type": "boolean"},
                "boss": {"type": "null"}
            },
            "required": ["name", "age", "score", "admin", "boss"]
        })
    );
    // In this order, which is the one a person reads a schema in.
    let text = crate::reformat::render(&schema, &crate::reformat::Options::default());
    let keys: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("  \""))
        .map(|l| l.trim().split('"').nth(1).unwrap())
        .collect();
    assert_eq!(keys, ["$schema", "type", "properties", "required"]);
}

#[test]
fn the_elements_of_an_array_are_one_place() {
    let schema = schema_of(json!([
        {"id": 1, "tags": ["a"], "price": 10},
        {"id": 2, "tags": [], "price": 9.99, "note": "x"},
        {"id": 3, "tags": ["b", "c"], "price": null}
    ]));
    assert_eq!(
        schema,
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "array",
            "items": {
                "type": "object",
                "properties": {
                    "id": {"type": "integer"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "price": {"type": ["number", "null"]},
                    "note": {"type": "string"}
                },
                "required": ["id", "tags", "price"]
            }
        })
    );
}

#[test]
fn values_of_several_types_are_of_all_of_them_and_what_each_has_is_kept() {
    let schema = schema_of(json!({"v": [1, "2024-01-15", {"a": true}, [null]]}));
    let v = at(&schema, "/properties/v/items");
    assert_eq!(v["type"], json!(["object", "array", "string", "integer"]));
    assert_eq!(v["format"], "date");
    assert_eq!(v["properties"]["a"]["type"], "boolean");
    assert_eq!(v["items"]["type"], "null");
}

#[test]
fn an_empty_array_says_nothing_of_its_elements_and_an_empty_document_part_nothing_at_all() {
    let schema = schema_of(json!({"list": [], "object": {}}));
    assert_eq!(at(&schema, "/properties/list"), &json!({"type": "array"}));
    assert_eq!(
        at(&schema, "/properties/object"),
        &json!({"type": "object"})
    );
}

#[test]
fn strings_have_the_format_and_the_pattern_of_what_they_are() {
    let document = json!({
        "id": "123e4567-e89b-12d3-a456-426614174000",
        "created": "2024-01-15T10:30:00Z",
        "day": "2024-01-15",
        "email": "ada@example.com",
        "site": "https://example.com/a",
        "ip": "192.168.0.1",
        "ip6": "2001:db8::1",
        "host": "api.example.com",
        "zip": "90210",
        "order": "ORD-2024-00017",
        "name": "Ada Lovelace"
    });
    let created = make(&document, Options::default());
    let schema = &created.schema;
    for (member, format) in [
        ("id", "uuid"),
        ("created", "date-time"),
        ("day", "date"),
        ("email", "email"),
        ("site", "uri"),
        ("ip", "ipv4"),
        ("ip6", "ipv6"),
        ("host", "hostname"),
    ] {
        let s = at(schema, &format!("/properties/{member}"));
        assert_eq!(s["format"], format, "{member}: {s}");
        assert!(
            s["pattern"].as_str().unwrap().starts_with('^'),
            "{member}: {s}"
        );
    }
    assert_eq!(at(schema, "/properties/zip/pattern"), "^[0-9]+$");
    assert_eq!(at(schema, "/properties/zip").get("format"), None);
    assert_eq!(
        at(schema, "/properties/order/pattern"),
        "^[A-Z]+-[0-9]+-[0-9]+$"
    );
    assert_eq!(at(schema, "/properties/name"), &json!({"type": "string"}));
    assert_eq!(
        found(&created),
        [
            (".id", "uuid"),
            (".created", "date-time"),
            (".day", "date"),
            (".email", "email"),
            (".site", "URL"),
            (".ip", "IPv4 address"),
            (".ip6", "IPv6 address"),
            (".host", "host name"),
            (".zip", "integer as text"),
            (".order", "like ORD-2024-00017"),
        ]
    );
    assert_eq!(created.problems, 0);
    assert!(fits(schema, &document));
}

#[test]
fn a_list_in_a_string_is_its_items_again_and_again() {
    let document = json!([
        {"tags": "red,green,blue", "ids": "1;2;3", "hosts": "10.0.0.1, 10.0.0.2"},
        {"tags": "black", "ids": "40", "hosts": "10.0.0.3"}
    ]);
    let created = make(&document, Options::default());
    let items = at(&created.schema, "/items/properties");
    assert_eq!(items["tags"]["pattern"], "^(?:[a-z]+(?:,[a-z]+)*)?$");
    assert_eq!(items["ids"]["pattern"], "^(?:[0-9]+(?:;[0-9]+)*)?$");
    assert!(items["hosts"]["pattern"]
        .as_str()
        .unwrap()
        .contains("(?:, ?(?:"));
    assert_eq!(
        found(&created),
        [
            (".[].tags", "comma-separated: words"),
            (".[].ids", "semicolon-separated: integer as text"),
            (".[].hosts", "comma-separated: IPv4 address"),
        ]
    );
    assert!(fits(&created.schema, &document));
    // A value that is not such a list does not fit.
    assert!(!fits(&created.schema, &json!([{"tags": "red,,blue"}])));
    assert!(!fits(&created.schema, &json!([{"hosts": "10.0.0.1, x"}])));
}

#[test]
fn an_object_whose_keys_are_data_is_a_map() {
    let document = json!({
        "users": {
            "123e4567-e89b-12d3-a456-426614174000": {"name": "Ada", "age": 36},
            "c56a4180-65aa-42ec-a945-5fd21dec0538": {"name": "Bob"}
        },
        "daily": {"2024-01-15": 3, "2024-01-16": 5},
        "rates": {"USD": 1.0, "EUR": 0.92, "GBP": 0.79}
    });
    let created = make(&document, Options::default());
    let users = at(&created.schema, "/properties/users");
    let (expression, values) = users["patternProperties"]
        .as_object()
        .unwrap()
        .iter()
        .next()
        .unwrap();
    assert!(expression.starts_with("^[0-9a-f]{8}-"), "{expression}");
    assert_eq!(values["required"], json!(["name"]), "what every value has");
    assert_eq!(users.get("properties"), None);
    assert_eq!(users.get("required"), None);
    let daily = at(&created.schema, "/properties/daily/patternProperties");
    let date = format!("^{}$", Kind::Date.pattern(&text::Facts::default(), false));
    assert_eq!(daily[&date], json!({"type": "integer"}));
    assert_eq!(daily.as_object().unwrap().len(), 1);
    assert_eq!(
        found(&created),
        [
            (".users", "keys: uuid"),
            (".daily", "keys: date"),
            (".rates", "keys: code"),
        ]
    );
    assert!(created.found.iter().all(|f| f.keys));
    assert!(fits(&created.schema, &document));

    // The names of a record are no map, however alike.
    let created = make(
        &json!({"first-name": "Ada", "last-name": "L"}),
        Options::default(),
    );
    assert!(created.found.is_empty());
    assert!(at(&created.schema, "/properties/first-name").is_object());
    // Nor is one key.
    let created = make(&json!({"2024-01-15": 1}), Options::default());
    assert!(at(&created.schema, "/properties/2024-01-15").is_object());
}

#[test]
fn members_whose_keys_are_a_name_and_a_number_are_one_place() {
    let document = json!({
        "name": "Ada",
        "phone1": "+44 20 7946 0958",
        "phone2": "+44 20 7946 0959",
        "address1": "1 Main St",
        "v2": true
    });
    let created = make(&document, Options::default());
    let schema = &created.schema;
    assert_eq!(
        at(schema, "/required"),
        &json!(["name", "address1", "v2"]),
        "the numbered ones are not required"
    );
    let phones = at(schema, "/patternProperties/^phone[0-9]+$");
    assert_eq!(
        phones["pattern"],
        format!("^{}$", Kind::Phone.pattern(&text::Facts::default(), false))
    );
    assert!(
        at(schema, "/properties/address1").is_object(),
        "one alone is a name"
    );
    assert_eq!(
        found(&created),
        [
            (".", "numbered keys: phone1, phone2"),
            (".phone<n>", "phone number")
        ]
    );
    assert!(fits(schema, &document));
}

#[test]
fn the_options_say_what_the_schema_says() {
    let document = json!([
        {"status": "open", "n": 3, "code": "AB-12", "tags": ["x"]},
        {"status": "closed", "n": 10, "code": "CD-345", "tags": ["x", "y", "z"]},
        {"status": "open", "n": -2, "code": "EF-6", "tags": []}
    ]);
    let plain = make(&document, Options::default()).schema;
    let item = at(&plain, "/items");
    assert_eq!(item["required"], json!(["status", "n", "code", "tags"]));
    assert_eq!(item.get("additionalProperties"), None);
    assert_eq!(item["properties"]["status"], json!({"type": "string"}));

    let options = Options {
        required: false,
        closed: true,
        enums: true,
        bounds: true,
        ..Options::default()
    };
    let schema = make(&document, options).schema;
    let item = at(&schema, "/items");
    assert_eq!(item.get("required"), None);
    assert_eq!(item["additionalProperties"], false);
    assert_eq!(
        item["properties"]["status"]["enum"],
        json!(["open", "closed"])
    );
    assert_eq!(item["properties"]["status"].get("pattern"), None);
    assert_eq!(item["properties"]["n"]["minimum"], -2);
    assert_eq!(item["properties"]["n"]["maximum"], 10);
    assert_eq!(item["properties"]["code"]["minLength"], 4);
    assert_eq!(item["properties"]["code"]["maxLength"], 6);
    assert_eq!(
        item["properties"]["code"]["pattern"], "^[A-Z]{2}-[0-9]{1,3}$",
        "lengths that vary are the least and the most"
    );
    assert_eq!(item["properties"]["tags"]["minItems"], 0);
    assert_eq!(item["properties"]["tags"]["maxItems"], 3);
    assert!(fits(&schema, &document));
    // Something else does not fit it.
    assert!(!fits(
        &schema,
        &json!([{"status": "gone", "n": 3, "code": "AB-12", "tags": []}])
    ));
    assert!(!fits(
        &schema,
        &json!([{"status": "open", "n": 3, "code": "AB-12", "tags": [], "x": 1}])
    ));

    // Without patterns, strings are strings, and every key is a member.
    let options = Options {
        patterns: false,
        ..Options::default()
    };
    let created = make(&json!({"id": "2024-01-15", "a1": 1, "a2": 2}), options);
    assert!(created.found.is_empty());
    assert_eq!(
        at(&created.schema, "/properties/id"),
        &json!({"type": "string"})
    );
    assert!(at(&created.schema, "/properties/a2").is_object());
}

#[test]
fn an_enum_is_of_strings_that_come_again() {
    let enums = Options {
        enums: true,
        ..Options::default()
    };
    // Each once: no enum.
    let schema = make(&json!(["a", "b", "c"]), enums).schema;
    assert_eq!(at(&schema, "/items").get("enum"), None);
    // A null among them is one of them.
    let schema = make(&json!(["a", "b", "a", null]), enums).schema;
    assert_eq!(at(&schema, "/items/enum"), &json!(["a", "b", null]));
    // Strings among numbers are no enum.
    let schema = make(&json!(["a", "a", 1]), enums).schema;
    assert_eq!(at(&schema, "/items").get("enum"), None);
}

#[test]
fn the_draft_is_said_and_only_its_formats_are_used() {
    let document = json!({"id": "123e4567-e89b-12d3-a456-426614174000", "day": "2024-01-15", "at": "2024-01-15T10:30:00Z"});
    for draft in Draft::ALL {
        let options = Options {
            draft,
            ..Options::default()
        };
        let created = make(&document, options);
        assert_eq!(created.schema["$schema"], draft.uri());
        let format = |member: &str| created.schema["properties"][member].get("format").cloned();
        assert_eq!(format("id").is_some(), draft >= Draft::D201909, "{draft:?}");
        assert_eq!(format("day").is_some(), draft >= Draft::D7, "{draft:?}");
        assert_eq!(format("at"), Some(json!("date-time")), "{draft:?}");
        assert_eq!(created.problems, 0, "{draft:?}");
        // The validator reads the draft the schema says.
        let report = schema::validate(
            &created.schema,
            &document,
            schema::Options {
                check_formats: true,
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(report.draft, draft.name());
    }
}

#[test]
fn a_format_the_validator_does_not_hold_the_value_to_is_taken_out_and_the_pattern_stays() {
    // A duration of weeks and days: the expression takes it, RFC 3339 does not.
    let document = json!({"d": "P1W2D", "e": "P3D"});
    let created = make(&document, Options::default());
    let d = at(&created.schema, "/properties/d");
    assert_eq!(d.get("format"), None, "{d}");
    assert!(d.get("pattern").is_some());
    assert_eq!(at(&created.schema, "/properties/e/format"), "duration");
    let entry = created.found.iter().find(|f| f.path == ".d").unwrap();
    assert_eq!(entry.format, None);
    assert_eq!(created.problems, 0);
}

#[test]
fn every_kind_s_expression_is_one_the_validator_reads_the_same() {
    // Each kind's samples, as the values of a place: the schema has the kind's
    // pattern there, and the validator, which reads it as ECMA-262 does, finds
    // that every sample fits.
    for (kind, good, _) in kinds::tests::samples() {
        let document = Value::Array(good.iter().map(|s| Value::from(*s)).collect());
        let created = make(&document, Options::default());
        assert_eq!(created.problems, 0, "{kind:?}");
        let entry = created
            .found
            .iter()
            .find(|f| f.path == ".[]")
            .unwrap_or_else(|| panic!("{kind:?}: {:?}", created.found));
        assert_eq!(entry.what, kind.name(), "{kind:?}: {}", entry.expression);
        assert!(
            created.schema["items"].get("pattern").is_some(),
            "{kind:?}: the pattern stays"
        );
        // And as items of a list.
        let list = Value::from(good.join(","));
        if !good.iter().any(|s| s.contains(',')) {
            let created = make(&json!([list]), Options::default());
            assert_eq!(created.problems, 0, "{kind:?} in a list");
        }
    }
}

#[test]
fn a_schema_fits_the_document_it_was_made_of_whatever_the_options() {
    let documents = [
        json!(null),
        json!("x"),
        json!([1, 2.5, "a", null, true, {"a": [1, {"b": "2024-01-15"}]}]),
        json!({"a": {"b": {"c": [{"d": "x,y"}, {"d": "z"}, {"e": "AB-1"}]}}}),
        json!([{"k1": 1, "k2": "2"}, {"k3": null, "x": [1, [2, [3]]]}]),
        json!({"m": {"1": {"a": 1}, "2": {"b": 2}, "30": {}}}),
        json!(["2024-01-15T10:30:00Z", "2024-01-15T10:30:00.5+01:00"]),
        json!({"weird keys": {"a/b": 1, "~c": 2, "": 3, "é": "ü"}}),
        json!([{"tags": "a|b|c"}, {"tags": "d"}, {"tags": ""}]),
        serde_json::from_str(r#"{"big": 123456789012345678901234567890, "small": -1e-7}"#).unwrap(),
    ];
    for document in &documents {
        for required in [false, true] {
            for closed in [false, true] {
                for (patterns, enums, bounds) in [
                    (true, false, false),
                    (false, true, false),
                    (true, true, true),
                    (false, false, true),
                ] {
                    for draft in [Draft::D202012, Draft::D4] {
                        let options = Options {
                            draft,
                            required,
                            closed,
                            patterns,
                            enums,
                            bounds,
                        };
                        let created = make(document, options);
                        assert_eq!(created.problems, 0, "{document} {options:?}");
                        assert!(fits(&created.schema, document), "{document} {options:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn paths_are_said_as_jq_says_them() {
    assert_eq!(step("", "a"), ".a");
    assert_eq!(step(".a", "b_2"), ".a.b_2");
    assert_eq!(step("", "a b"), ".[\"a b\"]");
    assert_eq!(step(".a", "b-c"), ".a[\"b-c\"]");
    assert_eq!(step(".a", "2x"), ".a[\"2x\"]");
    assert_eq!(every(""), ".[]");
    assert_eq!(every(".a"), ".a[]");
    assert_eq!(shown(""), ".");
    assert_eq!(numbered_step(".c", "phone"), ".c.phone<n>");
    let created = make(&json!({"a b": [{"c": "2024-01-15"}]}), Options::default());
    assert_eq!(found(&created), [(".[\"a b\"][].c", "date")]);
}

#[test]
fn a_key_is_a_name_and_a_number_only_with_a_letter_before_its_digits() {
    assert_eq!(numbered("phone2"), Some("phone"));
    assert_eq!(numbered("line_10"), Some("line_"));
    assert_eq!(numbered("phone"), None);
    assert_eq!(numbered("12"), None);
    assert_eq!(numbered("-1"), None);
}

#[test]
fn a_document_too_deep_is_described_as_far_as_it_goes() {
    let mut deep = json!("bottom");
    for _ in 0..(MAX_DEPTH + 20) {
        deep = json!([deep]);
    }
    let created = make(&deep, Options::default());
    assert_eq!(created.problems, 0);
    let mut schema = &created.schema;
    let mut depth = 0;
    while let Some(items) = schema.get("items") {
        schema = items;
        depth += 1;
    }
    assert_eq!(depth, MAX_DEPTH);
    assert_eq!(schema, &json!({}), "anything, down there");
}

#[test]
fn an_object_with_more_keys_than_are_kept_apart_has_one_schema_for_their_values() {
    // Keys of letters only, which are no kind.
    let name = |mut i: usize| {
        let mut key = String::new();
        for _ in 0..4 {
            key.push((b'a' + (i % 26) as u8) as char);
            i /= 26;
        }
        key
    };
    let members: Map<String, Value> = (0..MAX_KEYS + 5)
        .map(|i| (name(i), json!({"n": i})))
        .collect();
    let document = Value::Object(members);
    let created = make(&document, Options::default());
    assert_eq!(
        at(&created.schema, "/additionalProperties/properties/n"),
        &json!({"type": "integer"})
    );
    assert_eq!(created.schema.get("properties"), None);
    assert_eq!(created.problems, 0);

    // Keys with numbers are a map of their shape.
    let members: Map<String, Value> = (0..MAX_KEYS + 5)
        .map(|i| (format!("k{i}"), json!(i)))
        .collect();
    let created = make(&Value::Object(members), Options::default());
    assert_eq!(
        at(&created.schema, "/patternProperties/^k[0-9]+$"),
        &json!({"type": "integer"})
    );
}

#[test]
fn a_walk_that_is_cancelled_says_so() {
    let document = Value::Array((0..CANCEL_EVERY * 2).map(Value::from).collect());
    let cancelled = create(&document, Options::default(), &AtomicBool::new(true));
    assert_eq!(cancelled.err(), Some(Error::Cancelled));
}

#[test]
fn numbers_keep_their_digits_in_the_bounds() {
    let bounds = Options {
        bounds: true,
        ..Options::default()
    };
    let document: Value =
        serde_json::from_str(r#"[123456789012345678901234567890, 0.10, 2]"#).unwrap();
    let schema = make(&document, bounds).schema;
    assert_eq!(at(&schema, "/items/type"), "number");
    assert_eq!(at(&schema, "/items/minimum").to_string(), "0.10");
    assert_eq!(
        at(&schema, "/items/maximum").to_string(),
        "123456789012345678901234567890"
    );
}

/// The pattern found at `path`.
fn found_at<'a>(created: &'a Created, path: &str) -> &'a Found {
    created
        .found
        .iter()
        .find(|f| f.path == path)
        .unwrap_or_else(|| panic!("{path}: {:?}", created.found))
}

/// Keys of `value`, in order.
fn keys_of(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

#[test]
fn a_pattern_of_strings_is_changed_made_plain_and_put_back() {
    let document = json!({"id": "123e4567-e89b-12d3-a456-426614174000", "n": 1});
    let created = make(&document, Options::default());
    let made = created.schema.clone();
    let mut schema = created.schema.clone();
    let id = found_at(&created, ".id");

    // An expression of one's own, where the found one was.
    let mine = Now::Expression("^[0-9a-f-]{36}$".to_owned());
    change(&mut schema, &made, id, &Now::Found, &mine).unwrap();
    assert_eq!(at(&schema, "/properties/id/pattern"), "^[0-9a-f-]{36}$");
    assert_eq!(
        at(&schema, "/properties/id/format"),
        "uuid",
        "the format stays"
    );
    assert_eq!(expression_now(id, &mine), Some("^[0-9a-f-]{36}$"));

    // Plain: a string, of any kind.
    change(&mut schema, &made, id, &mine, &Now::Plain).unwrap();
    assert_eq!(at(&schema, "/properties/id"), &json!({"type": "string"}));
    assert_eq!(expression_now(id, &Now::Plain), None);
    assert!(fits(&schema, &json!({"id": "anything", "n": 1})));

    // Back as it was found, in its place among the keywords.
    change(&mut schema, &made, id, &Now::Plain, &Now::Found).unwrap();
    assert_eq!(schema, made);
    assert_eq!(
        keys_of(at(&schema, "/properties/id")),
        ["type", "format", "pattern"]
    );
    // And an expression put in a plain one goes where a pattern goes.
    change(&mut schema, &made, id, &Now::Found, &Now::Plain).unwrap();
    change(&mut schema, &made, id, &Now::Plain, &mine).unwrap();
    assert_eq!(keys_of(at(&schema, "/properties/id")), ["type", "pattern"]);
}

#[test]
fn a_pattern_that_cannot_be_used_or_a_place_that_is_gone_changes_nothing() {
    let created = make(&json!({"day": "2024-01-15"}), Options::default());
    let made = created.schema.clone();
    let mut schema = created.schema.clone();
    let day = found_at(&created, ".day");
    let bad = Now::Expression("^[0-9{4}$".to_owned());
    let error = change(&mut schema, &made, day, &Now::Found, &bad).unwrap_err();
    assert!(error.starts_with("The pattern can't be used: "), "{error}");
    let error = change(
        &mut schema,
        &made,
        day,
        &Now::Found,
        &Now::Expression(String::new()),
    )
    .unwrap_err();
    assert!(error.contains("make it plain"), "{error}");
    assert_eq!(schema, made, "nothing changed");
    // ECMA-262's look-ahead is a pattern the validator takes.
    assert!(check_expression("^(?=.{10}$)[0-9-]+$").is_ok());

    // The person took the place out of the schema by hand.
    schema["properties"].as_object_mut().unwrap().remove("day");
    let error = change(&mut schema, &made, day, &Now::Found, &Now::Plain).unwrap_err();
    assert_eq!(error, "That place is no longer in the schema");
}

#[test]
fn the_pattern_of_a_map_s_keys_is_renamed_made_any_key_and_put_back() {
    let document = json!({"visits": {"2024-01-15": 3, "2024-01-16": 5}});
    let created = make(&document, Options::default());
    let made = created.schema.clone();
    let mut schema = created.schema.clone();
    let visits = found_at(&created, ".visits");
    assert!(visits.keys);

    let mine = Now::Expression("^2024-".to_owned());
    change(&mut schema, &made, visits, &Now::Found, &mine).unwrap();
    assert_eq!(
        at(&schema, "/properties/visits/patternProperties"),
        &json!({"^2024-": {"type": "integer"}})
    );
    // Any key: the values' schema is that of any key.
    change(&mut schema, &made, visits, &mine, &Now::Plain).unwrap();
    assert_eq!(
        at(&schema, "/properties/visits"),
        &json!({"type": "object", "additionalProperties": {"type": "integer"}})
    );
    assert!(fits(&schema, &json!({"visits": {"monday": 1}})));
    assert!(!fits(&schema, &json!({"visits": {"monday": "x"}})));
    // And back.
    change(&mut schema, &made, visits, &Now::Plain, &Now::Found).unwrap();
    assert_eq!(schema, made);
}

#[test]
fn keys_that_allowed_nothing_else_do_so_again_when_put_back() {
    let options = Options {
        closed: true,
        ..Options::default()
    };
    let document = json!({"name": "Ada", "phone1": "+44 20 7946 0958", "phone2": "+44 20 7946 0959",
                          "line1": "1 Main St", "line2": "Flat 2"});
    let created = make(&document, options);
    let made = created.schema.clone();
    let mut schema = created.schema.clone();
    let phones = created
        .found
        .iter()
        .find(|f| f.keys && f.expression == "^phone[0-9]+$")
        .unwrap();
    let lines = created
        .found
        .iter()
        .find(|f| f.keys && f.expression == "^line[0-9]+$")
        .unwrap();

    change(&mut schema, &made, phones, &Now::Found, &Now::Plain).unwrap();
    assert_eq!(keys_of(at(&schema, "/patternProperties")), ["^line[0-9]+$"]);
    assert!(at(&schema, "/additionalProperties").is_object());
    // Only one schema can be that of any key.
    let error = change(&mut schema, &made, lines, &Now::Found, &Now::Plain).unwrap_err();
    assert!(error.contains("any key already"), "{error}");
    // A rename may not take a pattern the keys already have.
    let clash = Now::Expression("^line[0-9]+$".to_owned());
    let error = change(&mut schema, &made, phones, &Now::Plain, &clash).unwrap_err();
    assert!(error.contains("already have the pattern"), "{error}");
    // Back: the object allows no other key again, and the patterns are in their order
    // again but for the one that came back, which is last.
    change(&mut schema, &made, phones, &Now::Plain, &Now::Found).unwrap();
    assert_eq!(at(&schema, "/additionalProperties"), &json!(false));
    assert_eq!(
        keys_of(at(&schema, "/patternProperties")),
        ["^line[0-9]+$", "^phone[0-9]+$"]
    );
    assert!(fits(&schema, &document));
}

#[test]
fn a_keyword_put_back_takes_its_place_among_the_others() {
    let mut members = Map::new();
    for key in ["type", "properties", "required"] {
        members.insert(key.to_owned(), Value::Null);
    }
    put(&mut members, "pattern", json!("x"));
    put(&mut members, "patternProperties", json!({}));
    put(&mut members, "x-custom", json!(1));
    put(&mut members, "type", json!("object"));
    let keys: Vec<&str> = members.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "type",
            "pattern",
            "properties",
            "patternProperties",
            "required",
            "x-custom"
        ]
    );
    assert_eq!(
        members["type"], "object",
        "one that is there keeps its place"
    );
}

/// The text of each range of `text`.
fn texts<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
    ranges.iter().map(|range| &text[range.clone()]).collect()
}

const ORDERS: &str = r#"{
  "orders": [
    {"id": "ORD-2024-00017", "when": "2024-01-15"},
    {"id": "ORD-2024-00018", "when": null},
    {"id": "ORD-2024-00019"}
  ],
  "daily": {"2024-01-15": 3, "2024-01-16": 5},
  "contact": {"name": "Ada", "phone1": "+44 20 7946 0958", "address1": "1 Main St",
              "phone2": "+44 20 7946 0959"}
}"#;

#[test]
fn what_a_pattern_is_about_is_found_where_it_is_in_the_document() {
    let created = make(&serde_json::from_str(ORDERS).unwrap(), Options::default());
    let of = |path: &str| occurrences(found_at(&created, path), ORDERS);

    assert_eq!(
        texts(ORDERS, &of(".orders[].id")),
        [
            r#""ORD-2024-00017""#,
            r#""ORD-2024-00018""#,
            r#""ORD-2024-00019""#
        ]
    );
    // The keys of a map, which are there and not where the same text is a value.
    let daily = of(".daily");
    assert_eq!(
        texts(ORDERS, &daily),
        [r#""2024-01-15""#, r#""2024-01-16""#]
    );
    assert!(daily[0].start > ORDERS.find("daily").unwrap());
    // Of the members, those with a name and a number, and then their values.
    assert_eq!(
        texts(ORDERS, &of(".contact")),
        [r#""phone1""#, r#""phone2""#]
    );
    assert_eq!(
        texts(ORDERS, &of(".contact.phone<n>")),
        [r#""+44 20 7946 0958""#, r#""+44 20 7946 0959""#]
    );
    // Text that is not JSON has none.
    assert!(occurrences(found_at(&created, ".orders[].id"), "{\"orders\": [").is_empty());

    // A stream of values is an array of them, as it is when it is read.
    let stream = "{\"id\": \"123e4567-e89b-12d3-a456-426614174000\"}\n\
                  {\"id\": \"c56a4180-65aa-42ec-a945-5fd21dec0538\"}\n";
    let created = make(
        &json!([
            {"id": "123e4567-e89b-12d3-a456-426614174000"},
            {"id": "c56a4180-65aa-42ec-a945-5fd21dec0538"}
        ]),
        Options::default(),
    );
    assert_eq!(
        texts(stream, &occurrences(found_at(&created, ".[].id"), stream)),
        [
            "\"123e4567-e89b-12d3-a456-426614174000\"",
            "\"c56a4180-65aa-42ec-a945-5fd21dec0538\""
        ]
    );

    // The values of a map, and of an object of too many keys to name.
    let document = json!({"users": {
        "123e4567-e89b-12d3-a456-426614174000": {"seen": "2024-01-15"},
        "c56a4180-65aa-42ec-a945-5fd21dec0538": {"seen": "2024-02-01"}
    }});
    let text = serde_json::to_string_pretty(&document).unwrap();
    let created = make(&document, Options::default());
    assert_eq!(
        texts(
            &text,
            &occurrences(found_at(&created, ".users[].seen"), &text)
        ),
        ["\"2024-01-15\"", "\"2024-02-01\""]
    );
    let name = |i: usize| format!("{}{}", ["ab", "cd", "ef"][i % 3], "x".repeat(i / 3 + 1));
    let members: Map<String, Value> = (0..MAX_KEYS + 5)
        .map(|i| (name(i), json!({"at": "2024-01-15T10:00:00Z"})))
        .collect();
    let document = Value::Object(members);
    let text = serde_json::to_string(&document).unwrap();
    let created = make(&document, Options::default());
    assert_eq!(
        occurrences(found_at(&created, ".[].at"), &text).len(),
        MAX_KEYS + 5
    );
}

#[test]
fn a_pattern_is_found_in_the_schema_as_it_is_now() {
    let created = make(&serde_json::from_str(ORDERS).unwrap(), Options::default());
    let mut schema = created.schema.clone();
    let text = |schema: &Value| serde_json::to_string_pretty(schema).unwrap();
    let place = |path: &str, now: &Now, text: &str| {
        place_in_schema(found_at(&created, path), now, text).map(|range| text[range].to_owned())
    };

    let id = found_at(&created, ".orders[].id");
    assert_eq!(
        place(".orders[].id", &Now::Found, &text(&schema)).unwrap(),
        format!("\"pattern\": \"{}\"", id.expression)
    );
    // Made plain, it is the schema of its place.
    change(&mut schema, &created.schema, id, &Now::Found, &Now::Plain).unwrap();
    let plain = place(".orders[].id", &Now::Plain, &text(&schema)).unwrap();
    assert!(
        plain.starts_with("\"id\": {") && plain.ends_with('}'),
        "{plain}"
    );
    assert!(!plain.contains("pattern"), "{plain}");

    // Keys: the key that is their expression, or any key.
    let daily = found_at(&created, ".daily");
    assert_eq!(
        place(".daily", &Now::Found, &text(&schema)).unwrap(),
        format!("\"{}\"", daily.expression)
    );
    let mine = Now::Expression("^[0-9-]+$".to_owned());
    change(&mut schema, &created.schema, daily, &Now::Found, &mine).unwrap();
    assert_eq!(
        place(".daily", &mine, &text(&schema)).unwrap(),
        "\"^[0-9-]+$\""
    );
    change(&mut schema, &created.schema, daily, &mine, &Now::Plain).unwrap();
    assert_eq!(
        place(".daily", &Now::Plain, &text(&schema)).unwrap(),
        "\"additionalProperties\""
    );

    // Not in a text that is not JSON, or that no longer has the place.
    assert_eq!(place(".orders[].id", &Now::Found, "{\"type\": "), None);
    assert_eq!(
        place(".orders[].id", &Now::Found, "{\"type\": \"object\"}"),
        None
    );
    // A document that is one string: its schema is the place.
    let created = make(&json!("2024-01-15"), Options::default());
    let whole = r#"{"type": "string"}"#;
    assert_eq!(
        place_in_schema(&created.found[0], &Now::Plain, whole),
        Some(0..whole.len())
    );
}

const TICKETS: &str = r#"[
  {"id": "T-1", "status": "open", "owner": "ada"},
  {"id": "T-2", "status": "closed", "owner": null},
  {"id": "T-3", "status": "open", "owner": "ada"}
]"#;

fn enums() -> Options {
    Options {
        enums: true,
        ..Options::default()
    }
}

#[test]
fn an_enum_is_in_the_list_with_its_values_and_how_often_each_came() {
    let created = make(&serde_json::from_str(TICKETS).unwrap(), enums());
    let status = found_at(&created, ".[].status");
    assert!(status.is_enum());
    assert_eq!(status.what, "enum");
    assert_eq!(status.values, [json!("open"), json!("closed")]);
    assert_eq!(status.times, [2, 1]);
    assert_eq!(status.expression, "");
    assert_eq!(expression_now(status, &Now::Found), None, "no pattern");
    assert_eq!(values_now(status, &Now::Found), Some(&status.values[..]));
    // A null is one of the values.
    let owner = found_at(&created, ".[].owner");
    assert_eq!(owner.values, [json!("ada"), Value::Null]);
    assert_eq!(owner.times, [2, 1]);
    // A pattern has no values.
    let created = make(&json!({"id": "2024-01-15"}), enums());
    assert!(!created.found[0].is_enum());
    assert_eq!(values_now(&created.found[0], &Now::Found), None);
}

#[test]
fn an_enum_loses_values_gains_others_is_made_a_string_and_put_back() {
    let document: Value = serde_json::from_str(TICKETS).unwrap();
    let created = make(&document, enums());
    let made = created.schema.clone();
    let mut schema = made.clone();
    let status = found_at(&created, ".[].status");

    // One left out, one put in.
    let mine = Now::Values(vec![json!("open"), json!("waiting")]);
    change(&mut schema, &made, status, &Now::Found, &mine).unwrap();
    assert_eq!(
        at(&schema, "/items/properties/status/enum"),
        &json!(["open", "waiting"])
    );
    assert!(!fits(&schema, &document), "a status is closed");
    assert_eq!(
        values_now(status, &mine),
        Some(&[json!("open"), json!("waiting")][..])
    );

    // A string of any kind.
    change(&mut schema, &made, status, &mine, &Now::Plain).unwrap();
    assert_eq!(
        at(&schema, "/items/properties/status"),
        &json!({"type": "string"})
    );
    assert_eq!(values_now(status, &Now::Plain), None);

    // Values again, where an enum goes; and back as it was made.
    change(&mut schema, &made, status, &Now::Plain, &mine).unwrap();
    change(&mut schema, &made, status, &mine, &Now::Found).unwrap();
    assert_eq!(schema, made);

    // None at all, the same twice, or an expression: refused, nothing changed.
    for (to, said) in [
        (Now::Values(vec![]), "Keep a value at least"),
        (
            Now::Values(vec![json!("open"), json!("open")]),
            "\"open\" is in the enum already",
        ),
        (Now::Expression("^o".to_owned()), "not a pattern"),
    ] {
        let error = change(&mut schema, &made, status, &Now::Found, &to).unwrap_err();
        assert!(error.contains(said), "{error}");
        assert_eq!(schema, made);
    }
    // Nor are values put in a pattern.
    let created = make(&json!({"id": "2024-01-15"}), enums());
    let mut schema = created.schema.clone();
    let error = change(
        &mut schema,
        &created.schema,
        &created.found[0],
        &Now::Found,
        &Now::Values(vec![json!("x")]),
    )
    .unwrap_err();
    assert!(error.contains("not values"), "{error}");
    assert_eq!(schema, created.schema);
}

#[test]
fn an_enum_is_found_in_the_document_and_in_the_schema() {
    let created = make(&serde_json::from_str(TICKETS).unwrap(), enums());
    let status = found_at(&created, ".[].status");
    assert_eq!(
        texts(TICKETS, &occurrences(status, TICKETS)),
        ["\"open\"", "\"closed\"", "\"open\""]
    );
    // Its nulls are among its values.
    let owner = found_at(&created, ".[].owner");
    assert_eq!(
        texts(TICKETS, &occurrences(owner, TICKETS)),
        ["\"ada\"", "null", "\"ada\""]
    );

    let mut schema = created.schema.clone();
    let text = |schema: &Value| serde_json::to_string_pretty(schema).unwrap();
    let place = |now: &Now, text: &str| {
        place_in_schema(status, now, text).map(|range| text[range].to_owned())
    };
    let shown = place(&Now::Found, &text(&schema)).unwrap();
    assert!(shown.starts_with("\"enum\": ["), "{shown}");
    assert!(
        shown.ends_with(']') && shown.contains("\"closed\""),
        "{shown}"
    );
    let mine = Now::Values(vec![json!("open")]);
    change(&mut schema, &created.schema, status, &Now::Found, &mine).unwrap();
    let shown = place(&mine, &text(&schema)).unwrap();
    assert!(
        shown.starts_with("\"enum\": [") && !shown.contains("closed"),
        "{shown}"
    );
    // Made plain, it is the schema of its place.
    change(&mut schema, &created.schema, status, &mine, &Now::Plain).unwrap();
    let plain = place(&Now::Plain, &text(&schema)).unwrap();
    assert!(plain.starts_with("\"status\": {"), "{plain}");
    assert!(!plain.contains("enum"), "{plain}");
}
