//! JSON Pointer (RFC 6901) lessons. A small language, so a short tutorial:
//! there is deliberately no padding to match the other dialects' length.

use super::{Example, Lesson, Topic};

const SHOP: &str = r#"{
  "store": {
    "book": [
      { "title": "Moby Dick", "price": 8.99 },
      { "title": "Ulysses", "price": 12.5 }
    ],
    "bicycle": { "color": "red" }
  }
}"#;

const NUMERIC_KEY: &str = r#"{ "10": "ten", "list": ["a", "b"] }"#;

const ESCAPES: &str = r#"{
  "a/b": 1,
  "m~n": 8,
  "c%d": 2,
  "": 0
}"#;

const CHEAT_DOC: &str = r#"{
  "store": { "book": [ { "title": "Moby Dick" }, { "title": "Ulysses" } ] },
  "a/b": 1,
  "m~n": 2
}"#;

pub(super) static TOPICS: &[Topic] = &[
    Topic::new(
        "Basics",
        &[
            Lesson::new(
                "What is a JSON Pointer?",
                "A JSON Pointer (RFC 6901) is a string that names exactly one value in a document. Every step starts with `/` and is either an object key or an array index.",
                &[Example::new(
                    "Walk down to one value",
                    SHOP,
                    "/store/bicycle/color",
                    &[
                        ("/store", "Step into the `store` key."),
                        ("/bicycle", "Then into `bicycle`."),
                        ("/color", "Then `color` — the one value the pointer names → `\"red\"`."),
                    ],
                )],
            )
            .tips(&[
                "The empty pointer (no characters at all) names the whole document.",
                "A lone `/` names the key `\"\"` — an empty-string key, which JSON allows.",
                "In the main window, a query that starts with `/` is auto-detected as a JSON Pointer.",
            ]),
            Lesson::new(
                "Keys & array indexes",
                "The same `/token` syntax indexes arrays: use the zero-based position. Numbers aren't special on objects — there `/10` is simply the key `\"10\"`.",
                &[
                    Example::new(
                        "An array element",
                        SHOP,
                        "/store/book/0/title",
                        &[
                            ("/book", "The `book` array."),
                            ("/0", "An array index: zero-based, so `0` is the first element."),
                            ("/title", "Its `title` → `\"Moby Dick\"`."),
                        ],
                    ),
                    Example::new(
                        "A number used as an object key",
                        NUMERIC_KEY,
                        "/10",
                        &[("/10", "On an object, `10` is just a key name; the same token on an array would be an index.")],
                    ),
                ],
            )
            .tips(&["Indexes have no sign and no leading zeros: `/list/01` and `/list/+1` don't match anything."]),
        ],
    ),
    Topic::new(
        "Special characters",
        &[Lesson::new(
            "Escaping / and ~",
            "A key may contain `/` or `~`, but both have a meaning inside a pointer, so they are written `~1` (for `/`) and `~0` (for `~`).",
            &[
                Example::new(
                    "A key that contains a slash",
                    ESCAPES,
                    "/a~1b",
                    &[("~1", "Stands for a literal `/`, so this pointer names the key `a/b`.")],
                ),
                Example::new(
                    "A key that contains a tilde",
                    ESCAPES,
                    "/m~0n",
                    &[("~0", "Stands for a literal `~`, so this pointer names the key `m~n`.")],
                ),
            ],
        )
        .tips(&[
            "Nothing else needs escaping: spaces, `%`, quotes and so on are written as they are — `/c%d` names the key `c%d`.",
            "`~01` means a tilde followed by `1`, not a slash — `~1` is decoded first, so the order of the two characters matters.",
        ])],
    ),
    Topic::new(
        "When it doesn't work",
        &[
            Lesson::new(
                "Missing values & bad pointers",
                "A pointer names exactly one value. If nothing is there, that is an *error* here (JSONPath, by contrast, returns an empty list). A pointer that isn't well-formed is a syntax error.",
                &[
                    Example::new(
                        "Nothing at that location",
                        SHOP,
                        "/store/book/9/title",
                        &[("/9", "There is no element at index 9 — `book` has only 0 and 1 — so the whole pointer fails.")],
                    )
                    .failing(),
                    Example::new(
                        "Not a pointer at all",
                        SHOP,
                        "store/book",
                        &[("store", "A pointer must be empty or start with `/`; this one starts with a letter.")],
                    )
                    .failing(),
                ],
            ),
            Lesson::new(
                "What pointers can't do",
                "A pointer names one exact location. There is no negative index, no wildcard and no filter — for those, use JSONPath, jq or JMESPath.",
                &[
                    Example::new(
                        "No negative indexes",
                        SHOP,
                        "/store/book/-1",
                        &[("-1", "RFC 6901 has no negative indexes (jq has `.[-1]`, JSONPath `[-1]`).")],
                    )
                    .failing(),
                    Example::new(
                        "No wildcards",
                        SHOP,
                        "/store/book/*/title",
                        &[("*", "`*` is looked up as an ordinary key literally named `*`, which doesn't exist.")],
                    )
                    .failing(),
                ],
            )
            .tips(&[
                "A lone `-` (as in `/store/book/-`) means \"the position after the last element\" — meaningful when *adding* with JSON Patch, but never a readable value.",
                "Want every title? JSONPath: `$.store.book[*].title` · jq: `.store.book[].title` · JMESPath: `store.book[*].title`.",
            ]),
        ],
    ),
    Topic::new(
        "Reference",
        &[
            Lesson::new(
                "Where pointers are used",
                "You'll meet JSON Pointers outside this app more often than you might expect.",
                &[],
            )
            .tips(&[
                r#"JSON Patch (RFC 6902) addresses its targets with pointers: `{"op": "replace", "path": "/store/bicycle/color", "value": "blue"}`."#,
                r##"JSON Schema's `$ref` uses the URI-fragment form — `"$ref": "#/definitions/address"`. Here, leave out the `#`: the `#/…` form isn't accepted."##,
                "Validators and API errors often report the failing location as a pointer such as `/members/2/age` — paste it into the query box to see that value.",
            ]),
            Lesson::cheat_sheet(
                "Cheat sheet",
                "Everything a JSON Pointer can express. Press ▶ to run one.",
                CHEAT_DOC,
                &[
                    ("/store", "an object member"),
                    ("/store/book", "an array"),
                    ("/store/book/0", "an array element (zero-based)"),
                    ("/store/book/1/title", "keep walking down"),
                    ("/a~1b", "the key `a/b` (`~1` is a slash)"),
                    ("/m~0n", "the key `m~n` (`~0` is a tilde)"),
                ],
            ),
        ],
    ),
];
