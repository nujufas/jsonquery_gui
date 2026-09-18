//! JSONPath (RFC 9535) lessons, over the specification's own bookstore.

use super::{Example, Lesson, Topic, STORE};

pub(super) static TOPICS: &[Topic] = &[
    Topic::new(
        "Basics",
        &[
            Lesson::new(
                "Root & child segments",
                "Every JSONPath starts at the root `$`. Add child segments with dots (`.name`) or brackets (`['name']`) — brackets also cope with keys that contain spaces or symbols.",
                &[
                    Example::new(
                        "Dot notation",
                        STORE,
                        "$.store.bicycle.color",
                        &[
                            ("$", "The root: the whole document."),
                            (".store", "The child named `store`."),
                            (".bicycle", "Then its child `bicycle`."),
                            (".color", "Then `color` → `\"red\"`."),
                        ],
                    ),
                    Example::new(
                        "Bracket notation — the same path",
                        STORE,
                        "$['store']['bicycle']['color']",
                        &[
                            ("['store']", "The bracket form of `.store`. Quotes may be single or double."),
                            ("['bicycle']", "Same idea, one level down."),
                            ("['color']", "Same result as the dotted version."),
                        ],
                    ),
                ],
            )
            .tips(&["A query in JSONPath always begins with `$` — that's also how the main window auto-detects it."]),
            Lesson::new(
                "Array indexes",
                "`[n]` picks one element (zero-based). A negative index counts from the end.",
                &[
                    Example::new(
                        "The first book",
                        STORE,
                        "$.store.book[0].title",
                        &[
                            (".book", "The array of books."),
                            ("[0]", "Index 0: the first element."),
                            (".title", "Its title."),
                        ],
                    ),
                    Example::new(
                        "The last book",
                        STORE,
                        "$.store.book[-1].title",
                        &[("[-1]", "A negative index counts from the end: `-1` is the last element.")],
                    ),
                ],
            ),
            Lesson::new(
                "Results are lists",
                "JSONPath always returns a *list* of matches: zero, one or many. A path that matches nothing is not an error — it is an empty list.",
                &[
                    Example::new(
                        "Many matches",
                        STORE,
                        "$.store.book[*].title",
                        &[("[*]", "Every element of the array, so there is one match per book — four results.")],
                    ),
                    Example::new(
                        "No match",
                        STORE,
                        "$.store.book[9].title",
                        &[("[9]", "There is no book at index 9, so nothing matches — zero results, and no error.")],
                    ),
                ],
            )
            .tips(&["JSON Pointer, by contrast, reports an error when a location doesn't exist."]),
        ],
    ),
    Topic::new(
        "Selecting many",
        &[
            Lesson::new(
                "Wildcards",
                "`[*]` or `.*` selects every element of an array, or every member of an object.",
                &[
                    Example::new(
                        "Every element of an array",
                        STORE,
                        "$.store.book[*].author",
                        &[
                            ("[*]", "Every element of `book`."),
                            (".author", "Each one's `author` — four results."),
                        ],
                    ),
                    Example::new(
                        "Every member of an object",
                        STORE,
                        "$.store.bicycle.*",
                        &[(".*", "Every member of `bicycle`, whatever its name → `\"red\"` and `19.95`.")],
                    ),
                ],
            ),
            Lesson::new(
                "Slices",
                "`[start:end:step]` selects a range. `end` is exclusive, every part is optional, negative values count from the end, and a negative step walks backwards.",
                &[
                    Example::new(
                        "A range",
                        STORE,
                        "$.store.book[1:3].title",
                        &[("[1:3]", "Elements at index 1 and 2 — the end is exclusive.")],
                    ),
                    Example::new(
                        "Every other element",
                        STORE,
                        "$.store.book[::2].title",
                        &[("[::2]", "Start and end left open, step 2: elements 0 and 2.")],
                    ),
                ],
            )
            .tips(&["`[-2:]` is the last two elements, and `[::-1]` walks the whole array backwards."]),
            Lesson::new(
                "Unions",
                "Put several selectors in one bracket, separated by commas, to collect them together — in the order written.",
                &[
                    Example::new(
                        "Several indexes",
                        STORE,
                        "$.store.book[0,2].title",
                        &[("[0,2]", "The elements at index 0 and index 2.")],
                    ),
                    Example::new(
                        "Several names",
                        STORE,
                        "$.store.bicycle['color','price']",
                        &[("['color','price']", "Two member names at once → `\"red\"`, `19.95`.")],
                    ),
                ],
            ),
            Lesson::new(
                "Descendants: ..",
                "`..` searches the whole subtree, at any depth, instead of one fixed level.",
                &[
                    Example::new(
                        "Every price in the document",
                        STORE,
                        "$..price",
                        &[("..price", "Every `price` member anywhere below the root: the four books and the bicycle — five results.")],
                    ),
                    Example::new(
                        "A named array, wherever it is",
                        STORE,
                        "$..book[2].title",
                        &[
                            ("..book", "Find `book` at any depth."),
                            ("[2].title", "Then take its third element's title."),
                        ],
                    ),
                ],
            )
            .tips(&["`$..*` returns every value in the whole document — usually far more than you want."]),
        ],
    ),
    Topic::new(
        "Filters",
        &[
            Lesson::new(
                "Comparisons",
                "`[?condition]` keeps the children for which the condition is true. Inside it, `@` stands for the child being tested.",
                &[
                    Example::new(
                        "Cheap books",
                        STORE,
                        "$.store.book[?@.price < 10].title",
                        &[
                            ("[?", "Start of a filter: test every element of `book`."),
                            ("@.price", "`@` is the element being tested; read its `price`."),
                            ("< 10", "The comparison — `==`, `!=`, `<`, `<=`, `>` and `>=` are available."),
                        ],
                    ),
                    Example::new(
                        "Comparing text",
                        STORE,
                        "$.store.book[?@.category == 'fiction'].title",
                        &[("@.category == 'fiction'", "Strings are quoted, with single or double quotes.")],
                    ),
                ],
            )
            .tips(&["Parentheses around the condition are allowed: `[?(@.price < 10)]` means the same."]),
            Lesson::new(
                "Combining conditions",
                "`&&` (and), `||` (or) and `!` (not) combine tests. Use parentheses to group.",
                &[
                    Example::new(
                        "Both must hold",
                        STORE,
                        "$.store.book[?@.category == 'fiction' && @.price < 13].title",
                        &[
                            ("@.category == 'fiction'", "The first test…"),
                            ("&&", "…and…"),
                            ("@.price < 13", "…the second, both true."),
                        ],
                    ),
                    Example::new(
                        "Either may hold",
                        STORE,
                        "$.store.book[?@.price < 9 || @.price > 20].title",
                        &[("||", "True when at least one side is: very cheap or very expensive.")],
                    ),
                ],
            ),
            Lesson::new(
                "Existence tests",
                "A bare path inside a filter tests whether it *exists* — no comparison needed. Put `!` in front to test that it doesn't.",
                &[
                    Example::new(
                        "Books that have an ISBN",
                        STORE,
                        "$.store.book[?@.isbn].title",
                        &[("@.isbn", "True when the element has an `isbn` member at all.")],
                    ),
                    Example::new(
                        "Books without one",
                        STORE,
                        "$.store.book[?!@.isbn].title",
                        &[("!@.isbn", "The opposite: the element has no `isbn` member.")],
                    ),
                ],
            ),
            Lesson::new(
                "Comparing with the rest of the document",
                "Inside a filter, `$` still means the document root, so an element can be compared with any other value in the document.",
                &[Example::new(
                    "Books cheaper than the bicycle",
                    STORE,
                    "$.store.book[?@.price < $.store.bicycle.price].title",
                    &[
                        ("@.price", "The book's own price."),
                        ("$.store.bicycle.price", "An absolute path back to the root: the bicycle's price, `19.95`."),
                    ],
                )],
            ),
            Lesson::new(
                "Functions",
                "RFC 9535 defines `length()`, `count()`, `match()`, `search()` and `value()` for use inside filters.",
                &[
                    Example::new(
                        "Long titles",
                        STORE,
                        "$.store.book[?length(@.title) > 15].title",
                        &[
                            ("length(@.title)", "The number of characters in the title."),
                            ("> 15", "Keep books whose title is longer than 15 characters."),
                        ],
                    ),
                    Example::new(
                        "Authors matching a pattern",
                        STORE,
                        r#"$.store.book[?match(@.author, "^[NE].*")].title"#,
                        &[
                            ("match(", "A regular-expression test that must match the *whole* string."),
                            (r#""^[NE].*""#, "Names starting with `N` or `E`, followed by anything."),
                        ],
                    ),
                ],
            )
            .tips(&[
                r#"`search(@.title, "the")` matches anywhere inside the string; `match` needs the whole string to match."#,
                "`count(@.*)` counts how many members an element has: `[?count(@.*) > 4]` finds the books that carry an ISBN.",
                "Functions only work inside `[? … ]`: `$.store.book.length()` is a syntax error.",
            ]),
        ],
    ),
    Topic::new(
        "Gotchas",
        &[Lesson::new(
            "Filter, then index",
            "Selectors chain: each one applies to the *children* of what the previous one matched. So `[?…][0]` does not mean \"the first match\".",
            &[
                Example::new(
                    "The matches themselves",
                    STORE,
                    "$.store.book[?@.price < 10].title",
                    &[("[?@.price < 10]", "Matches two book objects; `.title` then reads each one's title.")],
                ),
                Example::new(
                    "Trying to take the first match",
                    STORE,
                    "$.store.book[?@.price < 10][0].title",
                    &[
                        ("[?@.price < 10]", "Matches the two cheap book objects…"),
                        ("[0]", "…then looks for index 0 *inside each book* — an object, which has no index 0 — so nothing is left."),
                    ],
                ),
            ],
        )
        .tips(&["To get only the first match, take the first item of the result list — in this app, the first row in Results."])],
    ),
    Topic::new(
        "Reference",
        &[Lesson::cheat_sheet(
            "Cheat sheet",
            "The everyday JSONPath selectors at a glance, over the bookstore. Press ▶ to run one.",
            STORE,
            &[
                ("$", "the whole document"),
                ("$.store.bicycle.color", "child segments"),
                ("$['store']['bicycle']", "bracket form"),
                ("$.store.book[0]", "an array element"),
                ("$.store.book[-1]", "the last element"),
                ("$.store.book[*].title", "every element"),
                ("$.store.book[1:3]", "a slice (end exclusive)"),
                ("$.store.book[0,2]", "a union of indexes"),
                ("$.store.bicycle['color','price']", "a union of names"),
                ("$..price", "descendants: at any depth"),
                ("$.store.book[?@.price < 10]", "a filter"),
                ("$.store.book[?@.isbn]", "an existence test"),
                ("$.store.book[?@.category == 'fiction' && @.price < 13]", "combined conditions"),
                ("$.store.book[?match(@.author, \"^N\")]", "a function in a filter"),
            ],
        )],
    ),
];
