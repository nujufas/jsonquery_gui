//! JMESPath lessons, over the same team document as the jq lessons so the
//! two dialects can be compared question by question.
//!
//! Prose that mentions a backtick literal fences it with double backticks
//! (see `parse_inline`); queries themselves are plain text.

use super::{Example, Lesson, Topic, TEAM};

pub(super) static TOPICS: &[Topic] = &[
    Topic::new(
        "Basics",
        &[
            Lesson::new(
                "Identifiers & missing values",
                "A JMESPath expression is a path: `office.city` reads `city` inside `office`, with no leading `.` or `$`. A key that doesn't exist is not an error — the result is `null`.",
                &[
                    Example::new(
                        "A sub-expression",
                        TEAM,
                        "office.city",
                        &[
                            ("office", "A top-level key."),
                            (".city", "A sub-expression: the `city` key of that object → `\"Berlin\"`."),
                        ],
                    ),
                    Example::new(
                        "A missing key",
                        TEAM,
                        "office.nope",
                        &[("nope", "A key that doesn't exist evaluates to `null` — it is not an error.")],
                    ),
                ],
            )
            .tips(&[
                "In the main window, pin JMESPath with the engine buttons at the top right: a plain path like `office.city` is otherwise auto-detected as jq. ▶ Try it does this for you.",
                r#"Keys with spaces or symbols go in double quotes: `"first name"`."#,
            ]),
            Lesson::new(
                "Array indexes",
                "`[n]` selects an element (zero-based); a negative number counts from the end.",
                &[
                    Example::new(
                        "The first member",
                        TEAM,
                        "members[0].name",
                        &[
                            ("members", "The array."),
                            ("[0]", "Index 0: the first element."),
                            (".name", "Its name → `\"Ada\"`."),
                        ],
                    ),
                    Example::new(
                        "The last member",
                        TEAM,
                        "members[-1].name",
                        &[("[-1]", "A negative index counts from the end: the last element → `\"Alan\"`.")],
                    ),
                ],
            )
            .tips(&["An index past the end gives `null`: `members[9].name` is `null`."]),
        ],
    ),
    Topic::new(
        "Projections",
        &[
            Lesson::new(
                "Slices",
                "`[start:end:step]` works like a Python slice: the end is exclusive, negative numbers count from the end, and every part is optional.",
                &[
                    Example::new(
                        "A range",
                        TEAM,
                        "members[1:3].name",
                        &[
                            ("[1:3]", "Elements 1 and 2 (the end is exclusive). A slice starts a projection, so the rest of the expression runs on each element."),
                            (".name", "Their names → `[\"Linus\",\"Grace\"]`."),
                        ],
                    ),
                    Example::new(
                        "Every other element",
                        TEAM,
                        "members[::2].name",
                        &[("[::2]", "Start and end left open, step 2: elements 0 and 2.")],
                    ),
                ],
            )
            .tips(&["`members[-2:]` is the last two elements."]),
            Lesson::new(
                "Projections: [*] and *",
                "`[*]` is JMESPath's central idea: a *projection* applies everything to its right to *each* element and gathers the results — dropping any `null`s — into a list.",
                &[
                    Example::new(
                        "Project a field from every element",
                        TEAM,
                        "members[*].name",
                        &[
                            ("members", "An array."),
                            ("[*]", "Projection: for every element of the array…"),
                            (".name", "…read `name`. The results are collected into a list."),
                        ],
                    ),
                    Example::new(
                        "Project over an object's values",
                        TEAM,
                        "office.*",
                        &[(".*", "On an object, `*` projects over its *values* → `[\"Berlin\",3]`.")],
                    ),
                ],
            )
            .tips(&[
                "`members[*].email` returns only two e-mails: `null` results are dropped from a projection (Linus's is `null`, Grace's is missing).",
            ]),
            Lesson::new(
                "Flatten: []",
                "`[]` flattens one level of nesting — and, like `[*]`, it starts a projection.",
                &[
                    Example::new(
                        "Before flattening",
                        TEAM,
                        "members[*].skills",
                        &[("skills", "Each member's own list of skills — so the result is a list of lists.")],
                    ),
                    Example::new(
                        "After flattening",
                        TEAM,
                        "members[*].skills[]",
                        &[
                            ("members[*].skills", "A list of skill lists."),
                            ("[]", "Flatten: merge the inner lists into one flat list (one level only)."),
                        ],
                    ),
                ],
            ),
            Lesson::new(
                "Pipes stop projections",
                "After a projection, everything to its right runs on *each* element. A pipe `|` ends the projection, so what follows sees the whole result.",
                &[
                    Example::new(
                        "Inside the projection",
                        TEAM,
                        "members[*].skills[0]",
                        &[("skills[0]", "Runs *inside* the projection: index 0 of each member's own skills → the first skill of every member.")],
                    ),
                    Example::new(
                        "After a pipe",
                        TEAM,
                        "members[*].skills | [0]",
                        &[("| [0]", "The pipe ends the projection, so `[0]` now indexes the whole list of skill lists → the first member's skills.")],
                    ),
                ],
            ),
        ],
    ),
    Topic::new(
        "Filtering",
        &[
            Lesson::new(
                "Filters & literals",
                "`[?condition]` keeps the elements for which the condition is true. Numbers, booleans and `null` are written as JSON literals in backticks — `` `30` `` — while plain strings can use single quotes: `'dev'`.",
                &[
                    Example::new(
                        "Compare with a number",
                        TEAM,
                        "members[?age > `30`].name",
                        &[
                            ("[?", "Start of a filter: keep the elements for which the test is true."),
                            ("age > `30`", "Compares `age` with `` `30` `` — a number literal, which must be written in backticks."),
                            (".name", "Then project their names."),
                        ],
                    ),
                    Example::new(
                        "Compare with a string",
                        TEAM,
                        "members[?role == 'dev'].name",
                        &[("'dev'", "A raw string literal in single quotes. The JSON literal `` `\"dev\"` `` works too.")],
                    ),
                ],
            )
            .tips(&[
                "The comparison operators are `==`, `!=`, `<`, `<=`, `>` and `>=`.",
                "Decimal literals such as `` `9.5` `` don't evaluate correctly in this build yet; whole numbers, strings, booleans and `null` are fine.",
            ]),
            Lesson::new(
                "&&, || and !",
                "`&&` (and), `||` (or) and `!` (not) combine tests inside a filter.",
                &[
                    Example::new(
                        "Both must hold",
                        TEAM,
                        "members[?role == 'dev' && age > `30`].name",
                        &[
                            ("role == 'dev'", "The first test…"),
                            ("&&", "…and…"),
                            ("age > `30`", "…the second, both true → `[\"Grace\"]`."),
                        ],
                    ),
                    Example::new(
                        "Either may hold",
                        TEAM,
                        "members[?age > `40` || role == 'lead'].name",
                        &[("||", "True when at least one side is: older than 40, or the lead.")],
                    ),
                ],
            )
            .tips(&["`!` negates a test: `members[?!active].name` returns `[\"Grace\"]`. Parentheses group tests."]),
            Lesson::new(
                "Functions in filters",
                "Any function can appear in a condition, which is how you test text and arrays.",
                &[
                    Example::new(
                        "Does an array contain a value?",
                        TEAM,
                        "members[?contains(skills, 'sql')].name",
                        &[("contains(skills, 'sql')", "True when the array `skills` includes the string `'sql'` (on a string, `contains` looks for a substring).")],
                    ),
                    Example::new(
                        "How many items?",
                        TEAM,
                        "members[?length(skills) > `1`].name",
                        &[
                            ("length(skills)", "The number of elements in `skills`."),
                            ("> `1`", "…compared with the number 1: members with two or more skills."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "Functions are strict about types: `ends_with(email, '.com')` raises an error for Linus, whose e-mail is `null`. Guard it with `type(email) == 'string' && ends_with(email, '.com')`.",
            ]),
        ],
    ),
    Topic::new(
        "Reshaping",
        &[
            Lesson::new(
                "Multiselect list",
                "`[a, b, c]` builds a *list* from several expressions — JMESPath's counterpart to jq's comma.",
                &[
                    Example::new(
                        "Several values as one list",
                        TEAM,
                        "[team, office.city]",
                        &[
                            ("team", "The first value."),
                            ("office.city", "The second value → `[\"Platform\",\"Berlin\"]`."),
                        ],
                    ),
                    Example::new(
                        "One list per element",
                        TEAM,
                        "members[*].[name, age]",
                        &[
                            ("members[*]", "For every member…"),
                            (".[name, age]", "…build a two-item list."),
                        ],
                    ),
                ],
            ),
            Lesson::new(
                "Multiselect hash",
                "`{key: expression, …}` builds an object, so you can rename, pick and combine values.",
                &[
                    Example::new(
                        "Rename fields",
                        TEAM,
                        "members[*].{who: name, years: age}",
                        &[("{who: name, years: age}", "Build an object for each member: the key `who` gets the name and `years` gets the age.")],
                    ),
                    Example::new(
                        "A summary object",
                        TEAM,
                        "{team: team, count: length(members), oldest: max_by(members, &age).name}",
                        &[
                            ("team: team", "Copy a value under a key."),
                            ("count: length(members)", "A computed value."),
                            ("oldest: max_by(members, &age).name", "Functions and paths combine freely."),
                        ],
                    ),
                ],
            )
            .tips(&["Objects come back with their keys in alphabetical order, not the order you wrote them."]),
        ],
    ),
    Topic::new(
        "Functions",
        &[
            Lesson::new(
                "Counting & math",
                "`length`, `avg`, `min`, `max` and `sum` work on arrays (and `length` works on strings and objects too).",
                &[Example::new(
                    "Count, average, youngest, oldest",
                    TEAM,
                    "[length(members), avg(members[*].age), min(members[*].age), max(members[*].age)]",
                    &[
                        ("length(members)", "How many members → `4`."),
                        ("avg(members[*].age)", "The mean age → `37.5`."),
                        ("min(members[*].age)", "The youngest → `28`."),
                        ("max(members[*].age)", "The oldest → `45`."),
                    ],
                )],
            )
            .tips(&["`sum` is there too — and, like `abs`, it returns a floating-point number: `sum(members[*].age)` prints `150.0`."]),
            Lesson::new(
                "Sorting & picking",
                "`sort_by`, `min_by` and `max_by` take an *expression reference*: `&age` means \"evaluate `age` on each element\" rather than evaluating it right now.",
                &[
                    Example::new(
                        "Sort by a field",
                        TEAM,
                        "sort_by(members, &age)[*].name",
                        &[
                            ("sort_by(", "Sort an array by a key…"),
                            ("members", "…this array…"),
                            ("&age", "…the key, as a reference: `&` postpones `age` so it is evaluated on every element."),
                            ("[*].name", "Then project the names, youngest first."),
                        ],
                    ),
                    Example::new(
                        "The element with the largest value",
                        TEAM,
                        "max_by(members, &age).name",
                        &[
                            ("max_by(members, &age)", "The member with the largest `age` — the whole object."),
                            (".name", "Read its name → `\"Grace\"`."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "`sort(list)` sorts plain values and `reverse(list)` flips a list: `reverse(sort_by(members, &age))[*].name` sorts oldest first.",
                "`min_by` is the counterpart of `max_by`.",
            ]),
            Lesson::new(
                "Strings, join & map",
                "`join(glue, list)` builds a string, and `map(&expr, list)` applies an expression to every element.",
                &[
                    Example::new(
                        "Join names into one string",
                        TEAM,
                        "join(', ', members[*].name)",
                        &[
                            ("join(", "Glue a list of strings into one string."),
                            ("', '", "The separator (a raw string)."),
                            ("members[*].name", "The list to join."),
                        ],
                    ),
                    Example::new(
                        "Apply an expression to each element",
                        TEAM,
                        "map(&length(skills), members)",
                        &[
                            ("map(", "Run an expression on every element of a list and collect the results."),
                            ("&length(skills)", "The expression, as a reference: how many skills each one has."),
                            ("members", "The list to map over → `[2,2,1,0]`."),
                        ],
                    ),
                ],
            )
            .tips(&["`contains`, `starts_with`, `ends_with`, `length` and `reverse` work on strings too: `reverse('abc')` is `\"cba\"`."]),
            Lesson::new(
                "Types & defaults",
                "`type` names a value's type. `a || b` falls back to `b` when `a` is `null`, missing or empty, and `not_null(a, b, …)` returns the first argument that isn't `null`.",
                &[
                    Example::new(
                        "A default for missing e-mail",
                        TEAM,
                        "members[*].{name: name, email: email || 'n/a'}",
                        &[("email || 'n/a'", "`||` returns the right side when the left is `null`, missing or empty — so Linus and Grace get `\"n/a\"`.")],
                    ),
                    Example::new(
                        "What type is it?",
                        TEAM,
                        "[type(team), type(members), type(members[0].active), type(members[1].email)]",
                        &[
                            ("type(team)", "`\"string\"`."),
                            ("type(members)", "`\"array\"`."),
                            ("type(members[0].active)", "`\"boolean\"`."),
                            ("type(members[1].email)", "`\"null\"` — Linus's e-mail is explicitly `null`."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "Mind the projection: `members[*].email || 'n/a'` does *not* default each item — the projection yields a list, and a non-empty list is already truthy. Put the `||` inside a multiselect hash, as above.",
                "`to_string(x)` and `to_number(x)` convert: `to_number('42')` is `42`.",
            ]),
        ],
    ),
    Topic::new(
        "Reference",
        &[Lesson::cheat_sheet(
            "Cheat sheet",
            "The everyday JMESPath expressions at a glance, over the team document. Press ▶ to run one.",
            TEAM,
            &[
                ("office.city", "a sub-expression"),
                ("members[0]", "an array element"),
                ("members[-1].name", "the last element"),
                ("members[1:3]", "a slice"),
                ("members[*].name", "project a field from every element"),
                ("members[*].skills[]", "flatten nested lists"),
                ("members[?age > `30`]", "a filter (numbers use backtick literals)"),
                ("members[?role == 'dev' && active]", "combined conditions (strings use quotes)"),
                ("members[?contains(skills, 'sql')].name", "a function in a filter"),
                ("members[*].{who: name, years: age}", "build objects"),
                ("members[*].[name, age]", "build lists"),
                ("office.*", "every value of an object"),
                ("length(members)", "count"),
                ("sort_by(members, &age)[*].name", "sort"),
                ("max_by(members, &age).name", "the element with the largest value"),
                ("join(', ', members[*].name)", "join into a string"),
                ("members[*].skills | [0]", "a pipe ends a projection"),
            ],
        )],
    ),
];
