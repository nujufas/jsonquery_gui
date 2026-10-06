//! jq lessons. jsonquery runs jq programs with jaq, so everything here was
//! checked against that engine (see `documented_gaps_are_still_real` in
//! `mod.rs` for the jq 1.7 features it lacks).

use super::{Example, Lesson, Topic, TEAM};

const PERSON: &str = r#"{ "name": "Ada", "age": 36 }"#;
const ODD_KEYS: &str = r#"{ "first name": "Ada L.", "a-b": 1 }"#;
const LANGS: &str = r#"{ "langs": ["rust", "go", "c", "zig", "lua"] }"#;
const UPDATE: &str = r#"{ "name": "Ada", "age": 36, "tmp": true }"#;
const NESTED: &str = r#"{ "a": [1, [2, 3]], "b": { "c": "x" } }"#;
const NUMBERS: &str = r#"["1", "two", "3"]"#;

/// Values that CSV and TSV have to quote or escape: a comma, a double quote,
/// a line break, a tab and a backslash.
const AWKWARD: &str = r#"[
  { "name": "Smith, Jo", "note": "said \"hi\"" },
  { "name": "Ada", "note": "two\nlines" },
  { "name": "Tab\tman", "note": "C:\\temp" }
]"#;

/// Two tables that point at each other, for `INDEX` and `JOIN`.
const SHOP: &str = r#"{
  "customers": [
    { "code": "ADA", "name": "Ada", "city": "London" },
    { "code": "LIN", "name": "Linus", "city": "Helsinki" },
    { "code": "GRA", "name": "Grace", "city": "New York" }
  ],
  "orders": [
    { "no": 101, "customer": "ADA", "item": "keyboard", "qty": 2 },
    { "no": 102, "customer": "GRA", "item": "monitor", "qty": 1 },
    { "no": 103, "customer": "ADA", "item": "mouse", "qty": 3 },
    { "no": 104, "customer": "LIN", "item": "cable", "qty": 10 }
  ]
}"#;

pub(super) static TOPICS: &[Topic] = &[
    Topic::new(
        "Getting started",
        &[
            Lesson::new(
                "Identity & fields",
                "A jq program is a *filter*: it takes the input document and produces zero or more outputs. `.` is the simplest filter and `.name` reads a field.",
                &[
                    Example::new(
                        "The whole input, unchanged",
                        PERSON,
                        ".",
                        &[(".", "The identity filter: returns its input as it is. Handy for seeing (or pretty-printing) a whole document.")],
                    ),
                    Example::new(
                        "One field",
                        PERSON,
                        ".name",
                        &[(".name", "Field access: the value stored under the key `name` in the current input.")],
                    ),
                ],
            )
            .tips(&[
                "Press Ctrl+Enter in the main window to run a query. Every output appears as its own row in the Results panel.",
                r#"`.name` is shorthand for `.["name"]` — the next lesson covers keys that aren't plain identifiers."#,
            ]),
            Lesson::new(
                "Nesting & missing keys",
                "Chain fields to walk into nested objects. Asking for a key that doesn't exist is not an error — you get `null`. A comma runs several filters on the same input and emits every result.",
                &[
                    Example::new(
                        "Nested fields, a missing key, and the comma",
                        TEAM,
                        ".team, .office.city, .nope",
                        &[
                            (".team", "A top-level field → `\"Platform\"`."),
                            (", ", "The comma runs each filter on the same input and emits every result, one after another."),
                            (".office.city", "Chained fields: `city` inside `office` → `\"Berlin\"`."),
                            (".nope", "A key that doesn't exist gives `null`, not an error."),
                        ],
                    ),
                    Example::new(
                        "Keys that aren't plain identifiers",
                        ODD_KEYS,
                        r#".["first name"], ."a-b""#,
                        &[
                            (r#".["first name"]"#, "Bracket form: any string works as a key, including ones with spaces."),
                            (r#"."a-b""#, "Quoted form after the dot: keys containing `-` or other symbols."),
                        ],
                    ),
                ],
            )
            .tips(&["`.office.city` is just `.office | .city` — chained fields are two filters joined by a pipe."]),
            Lesson::new(
                "Array index & slice",
                "Index arrays with `[n]` (zero-based; negative numbers count from the end) and slice them with `[from:to]` (the end is exclusive).",
                &[Example::new(
                    "First, last and a range",
                    LANGS,
                    ".langs[0], .langs[-1], .langs[1:3]",
                    &[
                        (".langs[0]", "The first element (index 0) → `\"rust\"`."),
                        (".langs[-1]", "A negative index counts from the end → the last element, `\"lua\"`."),
                        (".langs[1:3]", "A slice from index 1 up to, but not including, 3 → `[\"go\",\"c\"]`."),
                    ],
                )],
            )
            .tips(&["Slices work on strings too, and either end may be left out: `.[:2]` or `.[2:]`."]),
            Lesson::new(
                "Iterate & collect",
                "`.[]` turns an array into a *stream* of its elements — one output each. Wrap any filter in `[ … ]` to collect a stream back into a single array.",
                &[
                    Example::new(
                        "Stream every member's name",
                        TEAM,
                        ".members[].name",
                        &[
                            (".members", "The array of members."),
                            ("[]", "Iterate: emit each element as its own output instead of the array."),
                            (".name", "Read `name` from every element — four separate results."),
                        ],
                    ),
                    Example::new(
                        "Collect a stream into one array",
                        TEAM,
                        "[.members[].age]",
                        &[
                            ("[", "Square brackets around a filter collect everything it outputs…"),
                            (".members[].age", "…here, every member's age…"),
                            ("]", "…into one array — a single result."),
                        ],
                    ),
                ],
            )
            .tips(&["`.[]` also iterates over the *values* of an object: `.office[]` gives `\"Berlin\"` then `3`."]),
        ],
    ),
    Topic::new(
        "Pipes & building output",
        &[
            Lesson::new(
                "Pipes",
                "`|` feeds each output of the filter on its left into the filter on its right. It is how small filters combine into a program.",
                &[Example::new(
                    "Names, upper-cased",
                    TEAM,
                    ".members[] | .name | ascii_upcase",
                    &[
                        (".members[]", "Produces four outputs, one per member."),
                        ("| .name", "Each output flows into the next filter, which reads `name`…"),
                        ("| ascii_upcase", "…and on into the next, which upper-cases the string."),
                    ],
                )],
            )
            .tips(&[
                "Read a pipeline left to right: `|` hands every output of its left side, one at a time, to its right side.",
                "`.a.b[0]` and `.a | .b | .[0]` mean exactly the same thing.",
            ]),
            Lesson::new(
                "Building objects",
                "`{ … }` builds a new object. `{name}` is shorthand for `{name: .name}`, and a value can be any expression — wrap computed ones in parentheses.",
                &[
                    Example::new(
                        "Reshape each member",
                        TEAM,
                        ".members[] | {name, next_age: (.age + 1)}",
                        &[
                            ("{name", "Shorthand: copy the key `name` and its value from the input (same as `name: .name`)."),
                            ("next_age", "A brand-new key…"),
                            ("(.age + 1)", "…whose value is any expression, run against each member. The parentheses group it."),
                        ],
                    ),
                    Example::new(
                        "Use a value as a key",
                        TEAM,
                        ".members | map({(.name): .age}) | add",
                        &[
                            ("map(", "Run the filter inside on every element and collect the results in an array."),
                            ("{(.name): .age}", "A computed key: the parentheses make `.name` an expression, so each member becomes a one-key object."),
                            ("| add", "Merge the array of small objects into one lookup table."),
                        ],
                    ),
                ],
            ),
            Lesson::new(
                "Strings & interpolation",
                r#"`"…\(expr)…"` inserts the result of any filter into a string. Combine it with `join`, `ascii_downcase`, `sub` and friends for text work."#,
                &[
                    Example::new(
                        "Format a line of text",
                        TEAM,
                        r#""\(.team) @ \(.office.city), floor \(.office.floor)""#,
                        &[
                            (r#"\(.team)"#, "Interpolation: anything inside `\\( )` is a full jq filter whose result is inserted into the string."),
                            (r#"\(.office.city)"#, "Nested paths work too."),
                            (r#"\(.office.floor)"#, "Numbers are converted to text automatically."),
                        ],
                    ),
                    Example::new(
                        "Join and change case",
                        TEAM,
                        r#".members | map(.name | ascii_downcase) | join(", ")"#,
                        &[
                            ("map(", "Apply the filter inside to every member…"),
                            ("ascii_downcase", "…lower-casing each name…"),
                            (r#"join(", ")"#, "…then glue the array of strings into one string with a separator."),
                        ],
                    ),
                ],
            )
            .tips(&[
                r#"More string filters: `split(",")`, `ltrimstr`, `startswith`, `endswith`, `test("regex")`, `length`, and `sub` / `gsub` for regex replacement."#,
                r#"For example `.members[0].email | sub("@.*"; "@corp.io")` gives `"ada@corp.io"`."#,
            ]),
        ],
    ),
    Topic::new(
        "Filtering & conditions",
        &[
            Lesson::new(
                "select & comparisons",
                "`select(condition)` lets a value through only when the condition is true — it is jq's way to filter.",
                &[Example::new(
                    "Members older than 30",
                    TEAM,
                    ".members[] | select(.age > 30) | .name",
                    &[
                        (".members[]", "Stream the members."),
                        ("select(.age > 30)", "Pass a member on only when the condition is true. Inside it, `.` is that one member."),
                        ("| .name", "Read the name of those that passed."),
                    ],
                )],
            )
            .tips(&[
                r#"Comparison operators: `==`, `!=`, `<`, `<=`, `>`, `>=`. Strings compare too: `select(.role == "dev")`."#,
            ]),
            Lesson::new(
                "and / or / not",
                "Combine conditions with `and` and `or`, and invert one with `not`.",
                &[
                    Example::new(
                        "Active devs or QA",
                        TEAM,
                        r#".members[] | select((.role == "dev" or .role == "qa") and .active) | .name"#,
                        &[
                            (r#"(.role == "dev" or .role == "qa")"#, "Parentheses group the `or`, which is true when either side is: the role is `dev` or `qa`."),
                            (" and ", "Both sides must be true."),
                            (".active", "A boolean field works as a condition on its own: the member is active."),
                        ],
                    ),
                    Example::new(
                        "Members who are not active",
                        TEAM,
                        ".members[] | select(.active | not) | .name",
                        &[
                            ("select(", "Keep only what the condition accepts…"),
                            (".active | not", "…and `not` flips a boolean. It is a filter, so it is piped: `.active | not`."),
                        ],
                    ),
                ],
            )
            .tips(&["Only `false` and `null` count as false; everything else — even `0` and `\"\"` — is true."]),
            Lesson::new(
                "if / elif / else",
                "`if … then … elif … else … end` picks one branch per input. Every condition and branch is itself a filter.",
                &[Example::new(
                    "Label each member by age",
                    TEAM,
                    r#".members[] | if .age >= 40 then "senior" elif .age >= 30 then "mid" else "junior" end"#,
                    &[
                        ("if .age >= 40", "The first condition."),
                        (r#"then "senior""#, "Its result when it is true."),
                        ("elif .age >= 30", "Otherwise, try the next condition."),
                        (r#"then "mid""#, "Its result."),
                        (r#"else "junior""#, "The result when nothing above matched."),
                        ("end", "Every `if` is closed with `end`."),
                    ],
                )],
            )
            .tips(&["`if` yields a value, so it can sit anywhere a filter can — inside `map(…)`, `select(…)` or an object value."]),
            Lesson::new(
                "Testing text & arrays",
                "`test`, `startswith`, `endswith` and `contains` return booleans, so they slot straight into `select`.",
                &[
                    Example::new(
                        "Names that start with A or G",
                        TEAM,
                        r#".members[] | select(.name | test("^[AG]")) | .name"#,
                        &[
                            ("select(", "Keep only members for which the test is true."),
                            (r#"test("^[AG]")"#, "A regex match on the piped-in string: `^` anchors at the start and `[AG]` is one of the letters A or G."),
                        ],
                    ),
                    Example::new(
                        "Members with a given skill",
                        TEAM,
                        r#".members[] | select(.skills | contains(["sql"])) | .name"#,
                        &[
                            ("select(", "Keep only members for which the test is true."),
                            (r#"contains(["sql"])"#, "True when the array includes every element of the argument — here the string `\"sql\"`."),
                        ],
                    ),
                ],
            )
            .tips(&[
                r#"Also handy: `startswith("A")`, `endswith(".com")`, `has("email")` (does the key exist?) and `any(.skills[]; . == "rust")` (does some element satisfy the condition?)."#,
            ]),
            Lesson::new(
                "Missing values: // and empty",
                "`a // b` gives `a` unless it is `null`, `false` or produces nothing — then it gives `b`. `empty` produces no output at all, which makes it a way to drop values.",
                &[
                    Example::new(
                        "A default for missing e-mail",
                        TEAM,
                        r#".members[] | .email // "n/a""#,
                        &[
                            (".email", "Ada and Alan have one, Linus has `null`, and Grace has no such key at all."),
                            (r#"// "n/a""#, "The alternative operator: use the right side when the left is `null`, `false` or missing."),
                        ],
                    ),
                    Example::new(
                        "Drop the missing ones",
                        TEAM,
                        "[.members[] | .email // empty]",
                        &[("// empty", "`empty` outputs nothing, so members without an e-mail simply vanish from the stream.")],
                    ),
                ],
            )
            .tips(&[
                r#"`has("email")` only checks that the *key* exists: Linus has it (with the value `null`), Grace doesn't. `select(.email)` skips both."#,
            ]),
        ],
    ),
    Topic::new(
        "Transforming",
        &[
            Lesson::new(
                "map & select together",
                "`map(f)` runs `f` on every element and collects the results into a new array. Combine it with `select` to filter and transform in one step.",
                &[Example::new(
                    "Names of the active members",
                    TEAM,
                    ".members | map(select(.active) | .name)",
                    &[
                        ("map(", "Run the filter inside on every element and collect the results (`map(f)` is short for `[.[] | f]`)."),
                        ("select(.active)", "Drop inactive members — `select` may output nothing, and `map` simply collects fewer results."),
                        ("| .name", "Keep just the name."),
                    ],
                )],
            )
            .tips(&["`map_values(f)` is the object version: it updates every value and keeps the keys."]),
            Lesson::new(
                "Updating values",
                "Assignment operators produce a modified copy of the input: `=` sets a value, `|=` updates it from its old value, `+=` (and `-=`, `*=` …) does arithmetic, and `del(path)` removes.",
                &[Example::new(
                    "Four kinds of change in one pipeline",
                    UPDATE,
                    r#".age += 1 | .name |= ascii_upcase | .role = "lead" | del(.tmp)"#,
                    &[
                        (".age += 1", "Arithmetic update: add 1 to the existing value."),
                        (".name |= ascii_upcase", "Update in place: the right side receives the old value and returns the new one."),
                        (r#".role = "lead""#, "Set a key — it is created if it didn't exist."),
                        ("del(.tmp)", "Remove a key."),
                    ],
                )],
            )
            .tips(&[
                "Your source document is never modified: jq builds a changed *copy*, which is what the Results panel shows.",
            ]),
            Lesson::new(
                "Working with objects",
                "`keys` lists the key names, `has(k)` tests for one, `a + b` merges two objects (the right side wins), and `with_entries(f)` rewrites keys and values through `{key, value}` pairs.",
                &[
                    Example::new(
                        "Key names, and whether a key exists",
                        TEAM,
                        r#".office | keys, has("city")"#,
                        &[
                            ("keys", "The object's key names, as a sorted array."),
                            (r#"has("city")"#, "Does the key exist? → `true`."),
                        ],
                    ),
                    Example::new(
                        "Merge, then rewrite every value",
                        TEAM,
                        r#".office + {country: "DE"} | with_entries(.value |= tostring)"#,
                        &[
                            (r#"+ {country: "DE"}"#, "Merge two objects: `country` is added, and on a clash the right side wins."),
                            ("with_entries(", "Turn the object into `{key, value}` pairs, run the filter on each pair, and rebuild an object."),
                            (".value |= tostring", "Update each pair's value to a string — so `3` becomes `\"3\"`."),
                        ],
                    ),
                ],
            )
            .tips(&[
                r#"`to_entries` and `from_entries` are the two halves of `with_entries`: `.office | to_entries` gives `[{"key":"city","value":"Berlin"}, …]`."#,
            ]),
            Lesson::new(
                "Types & conversion",
                "`type` names a value's type, and `tostring`, `tonumber`, `tojson` and `fromjson` convert between types.",
                &[Example::new(
                    "Inspect and convert",
                    TEAM,
                    r#".members[0] | (.age | tostring), (.skills | type), ("42" | tonumber)"#,
                    &[
                        ("(.age | tostring)", "Number → string: `\"36\"`."),
                        ("(.skills | type)", "The type's name: `\"array\"`."),
                        (r#"("42" | tonumber)"#, "String → number: `42`. Any literal can be piped into a filter."),
                    ],
                )],
            )
            .tips(&[
                "The types are `null`, `boolean`, `number`, `string`, `array` and `object`.",
                "Format strings re-encode a value: `.name | @base64`, `@uri`, `@html`, `@sh`, and `@json` (the same as `tojson`).",
            ]),
        ],
    ),
    Topic::new(
        "Aggregation",
        &[
            Lesson::new(
                "Counting & math",
                "Arrays have `length`, `add`, `min` and `max`, and the usual arithmetic works: `+ - * / %`.",
                &[Example::new(
                    "Count, total, average, extremes",
                    TEAM,
                    ".members | map(.age) | (length, add, add / length, min, max)",
                    &[
                        ("map(.age)", "An array of just the ages: `[36,28,45,41]`."),
                        ("length", "How many elements → `4`."),
                        ("add", "The sum → `150`."),
                        ("add / length", "The average → `37.5`."),
                        ("min, max", "The smallest and the largest → `28`, `45`."),
                    ],
                )],
            )
            .tips(&[
                "`min_by(.age)` and `max_by(.age)` return the whole element, not just the number.",
                "`floor` and `round` are there for rounding.",
            ]),
            Lesson::new(
                "Sort, group & unique",
                "`sort_by(key)` orders an array, `group_by(key)` buckets its elements into arrays, and `unique` drops duplicates. `flatten` merges nested arrays.",
                &[
                    Example::new(
                        "Sort by a field",
                        TEAM,
                        ".members | sort_by(.age) | map(.name)",
                        &[
                            ("sort_by(.age)", "Order the members by age, youngest first."),
                            ("map(.name)", "Keep just the names."),
                        ],
                    ),
                    Example::new(
                        "Group by a field",
                        TEAM,
                        ".members | group_by(.role) | map({role: .[0].role, names: map(.name)})",
                        &[
                            ("group_by(.role)", "An array of arrays: one group per distinct role."),
                            ("map(", "For each group…"),
                            ("{role: .[0].role, names: map(.name)}", "…build a summary: the role (taken from the group's first member) and every name in it."),
                        ],
                    ),
                    Example::new(
                        "Flatten and de-duplicate",
                        TEAM,
                        ".members | map(.skills) | flatten | unique",
                        &[
                            ("map(.skills)", "An array of skill arrays."),
                            ("flatten", "Merge the nested arrays into one flat array."),
                            ("unique", "Sort it and drop duplicates."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "`reverse` flips an array, so `sort_by(.age) | reverse` sorts oldest first.",
                "`unique_by(.role)` keeps one element per distinct role.",
            ]),
            Lesson::new(
                "reduce & foreach",
                "`reduce stream as $x (start; update)` folds a stream into a single value. `foreach` is the same, but emits the state after every step.",
                &[
                    Example::new(
                        "Count members per role",
                        TEAM,
                        "reduce .members[] as $m ({}; .[$m.role] += 1)",
                        &[
                            ("reduce .members[] as $m", "Loop over the stream, binding each element to `$m`."),
                            ("{}", "The starting value: an empty object."),
                            (".[$m.role] += 1", "Each round: bump the counter stored under this member's role (a missing key counts as 0)."),
                        ],
                    ),
                    Example::new(
                        "A running total of ages",
                        TEAM,
                        "[foreach (.members[] | .age) as $a (0; . + $a)]",
                        &[
                            ("foreach", "Like `reduce`, but outputs the state after every step instead of only the last."),
                            ("(.members[] | .age) as $a", "The stream to loop over, and the variable holding each item."),
                            ("0", "The starting state."),
                            ("; . + $a", "The update: new state = old state (`.`) plus this age."),
                        ],
                    ),
                ],
            )
            .tips(&["Inside the update expression, `.` is the accumulated state — not the input you started with."]),
            Lesson::new(
                "limit, first & range",
                "`limit(n; stream)` and `first(stream)` stop as soon as they have enough, instead of computing everything. `range` generates numbers.",
                &[
                    Example::new(
                        "At most two names",
                        TEAM,
                        "limit(2; .members[] | .name)",
                        &[
                            ("limit(2;", "Take at most 2 outputs from the stream after the `;`, then stop evaluating."),
                            (".members[] | .name", "The stream being limited: every member's name."),
                        ],
                    ),
                    Example::new(
                        "The first match only",
                        TEAM,
                        "first(.members[] | select(.age > 40)).name",
                        &[
                            ("first(", "The first output of the stream inside — evaluation stops right there."),
                            ("select(.age > 40)", "The stream: members older than 40."),
                            (".name", "Read the name of that one result."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "`range(n)` counts from 0: `[range(3)]` is `[0,1,2]`, and `[range(1; 10; 4)]` steps by 4 → `[1,5,9]`.",
                "Because they stop early, `limit` and `first` stay fast even on huge inputs.",
            ]),
        ],
    ),
    Topic::new(
        "Tables & lookups",
        &[
            Lesson::new(
                "CSV & TSV",
                "`@csv` and `@tsv` write an *array* as one row of text. Build the row with `[ … ]`, one row per result. A query that ends in one of them also makes the Results panel copy and save the rows as CSV or TSV: the page *Copy & save as CSV or TSV* has the details.",
                &[
                    Example::new(
                        "One CSV row per member",
                        TEAM,
                        ".members[] | [.name, .age, .role] | @csv",
                        &[
                            (".members[]", "Each member in turn."),
                            ("[.name, .age, .role]", "Build an array of the fields you want, in column order: one array is one row."),
                            ("@csv", "Write the array as a line of CSV: strings in quotes, numbers bare, commas between the fields."),
                        ],
                    ),
                    Example::new(
                        "A header row first, tab-separated",
                        TEAM,
                        r#"["name", "age"], (.members[] | [.name, .age]) | @tsv"#,
                        &[
                            (r#"["name", "age"]"#, "The header: an array of column titles."),
                            (", ", "The comma puts the header first and the member rows after it."),
                            ("(.members[] | [.name, .age])", "One row array per member."),
                            ("@tsv", "Applies to the header and every row: tabs between the fields, and any tab, newline or backslash inside a value written as `\\t`, `\\n` or `\\\\`."),
                        ],
                    ),
                    Example::new(
                        "Missing values are empty fields",
                        TEAM,
                        ".members[] | [.name, .email] | @csv",
                        &[
                            ("[.name, .email]", "Linus's e-mail is `null` and Grace has none at all: both come out as `null`…"),
                            ("@csv", "…which is an empty field, not the word `null`."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "Only strings, numbers, booleans and `null` fit in a row. An array or object among the fields is an error; flatten it first, for example `.skills | join(\";\")`.",
                "`@csv` and `@tsv` are jq's own names for these formats, so a query written here also works in the `jq` command line (with `-r` to print the rows without quotes).",
            ]),
            Lesson::new(
                "Records to a table",
                "Most data is an array of objects. List the columns once, in a variable, and use that one list for the header and for every row: `.[$cols[]]` reads the listed fields in order.",
                &[
                    Example::new(
                        "Header and rows from one list of columns",
                        TEAM,
                        r#"["name", "role", "age"] as $cols | $cols, (.members[] | [.[$cols[]]]) | @csv"#,
                        &[
                            (r#"["name", "role", "age"] as $cols"#, "The columns you want, in the order you want them, saved as `$cols`."),
                            ("$cols", "The header row: the column names themselves."),
                            ("(.members[] | [.[$cols[]]])", "One row per member. `.[$cols[]]` looks up each listed field in turn, and `[ … ]` collects the values into an array."),
                            ("@csv", "Writes the header and every row as CSV."),
                        ],
                    ),
                    Example::new(
                        "A list in one cell",
                        TEAM,
                        r#".members[] | [.name, (.skills | join(";"))] | @csv"#,
                        &[
                            (".members[]", "Each member in turn."),
                            (r#"(.skills | join(";"))"#, "A list can't sit in a cell as it is: `join(\";\")` glues its items into one string. An empty list becomes an empty string."),
                            ("@csv", "Writes the row."),
                        ],
                    ),
                    Example::new(
                        "An object as two columns",
                        TEAM,
                        ".office | to_entries[] | [.key, .value] | @csv",
                        &[
                            (".office", "An object with two fields, `city` and `floor`."),
                            ("to_entries[]", "One `{key, value}` entry per field."),
                            ("[.key, .value]", "A two-cell row: the field's name, then its value."),
                            ("@csv", "Writes each row."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "`keys_unsorted` lists a record's keys in the order of the file, so `(.members[0] | keys_unsorted) as $cols | …` takes the columns from the first record. Every field must then be a string, number, boolean or `null`.",
                "A field a record doesn't have reads as `null`, which is an empty cell.",
            ]),
            Lesson::new(
                "Quoting & escaping",
                "Values with commas, quotes or line breaks need care. `@csv` puts every string in double quotes and doubles any quote inside it; `@tsv` quotes nothing and writes tabs, line breaks and backslashes as escapes instead.",
                &[
                    Example::new(
                        "CSV: quotes around strings, doubled quotes inside",
                        AWKWARD,
                        ".[] | [.name, .note] | @csv",
                        &[
                            (".[]", "Each record in turn."),
                            ("[.name, .note]", "Two cells per row."),
                            ("@csv", "Strings go in double quotes, and the `\"` in `said \"hi\"` becomes `\"\"`. A comma or a line break inside the quotes stays as it is: it can't be taken for the end of the cell."),
                        ],
                    ),
                    Example::new(
                        "TSV: no quotes, escapes instead",
                        AWKWARD,
                        ".[] | [.name, .note] | @tsv",
                        &[
                            (".[]", "Each record in turn."),
                            ("[.name, .note]", "The same two cells."),
                            ("@tsv", "Cells are joined with tabs and never quoted. A tab inside a value is written `\\t`, a line break `\\n` and a backslash `\\\\`, so a record always stays on one line."),
                        ],
                    ),
                    Example::new(
                        "Only an array makes a row",
                        AWKWARD,
                        "try (.[0] | @csv) catch .",
                        &[
                            ("try (", "Run what is inside the parentheses; if it fails…"),
                            (".[0] | @csv", "…as it does here, because the first record is an object and `@csv` wants an array…"),
                            ("catch .", "…hand back the error message instead (`.` is the message)."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "In both formats a number or a boolean is written bare (`3`, `true`) and `null` is an empty cell.",
                "A tool that reads TSV should turn `\\t`, `\\n`, `\\r` and `\\\\` back into the characters they stand for.",
            ]),
            Lesson::new(
                "Copy & save as CSV or TSV",
                "When the last step of a jq query is `@csv` or `@tsv`, every result is a row of text. The Results panel then says *CSV* or *TSV* beside Tree and Text, and Copy to Clipboard, Save… and the Text view write the rows as rows, not as JSON strings.",
                &[
                    Example::new(
                        "Ends in @csv: copied and saved as CSV",
                        TEAM,
                        ".members[] | [.name, .age] | @csv",
                        &[
                            (".members[]", "One result per member…"),
                            ("[.name, .age]", "…built as an array of its cells…"),
                            ("@csv", "…and written as a row by the last step. Copy one row or all of them and paste straight into a spreadsheet."),
                        ],
                    ),
                    Example::new(
                        "Not the last step: still JSON",
                        TEAM,
                        "[.members[] | [.name, .age] | @csv]",
                        &[
                            ("[", "Collects everything inside into one array…"),
                            (".members[] | [.name, .age] | @csv", "…the same rows as above, now strings in a list…"),
                        ],
                    ),
                ],
            )
            .tips(&[
                "In the second example the one result is a JSON array of strings, so it is copied and saved as JSON, and the Results panel shows no *CSV* label.",
                "Copy to Clipboard on a row copies that row. On the *Results* line at the top it copies every row, one to a line. The Text view shows exactly those lines.",
                "Save… writes the same lines to a file, and its dialog offers `results.csv` or `results.tsv` as the name. Saving a single result saves just that row.",
                "The label and the CSV or TSV writing are for jq queries only: the other query languages have no `@csv`. The Source panel is always copied and saved as JSON.",
            ]),
            Lesson::new(
                "IN",
                "`IN(…)` asks whether its input is one of several values and answers `true` or `false`, so it belongs inside `select(…)`. `IN(source; values)` asks whether any value of `source` is among `values`.",
                &[
                    Example::new(
                        "Members whose role is one of several",
                        TEAM,
                        r#".members[] | select(.role | IN("dev", "qa")) | .name"#,
                        &[
                            ("select(", "Keep a member only if the test inside is true."),
                            (r#".role | IN("dev", "qa")"#, "True when the role equals any of the listed values."),
                            (".name", "Then take the name."),
                        ],
                    ),
                    Example::new(
                        "Is any member in QA?",
                        TEAM,
                        r#"IN(.members[].role; "qa")"#,
                        &[
                            (".members[].role", "The stream of values to check: every member's role."),
                            (r#""qa""#, "The values to look for. The answer is `true` if any of the first stream is among these."),
                        ],
                    ),
                    Example::new(
                        "Skills that are not on a list",
                        TEAM,
                        r#"[.members[].skills[] | select(IN("rust", "sql") | not)]"#,
                        &[
                            (".members[].skills[]", "Every skill of every member."),
                            (r#"IN("rust", "sql")"#, "True for a skill on the list."),
                            ("| not", "Turns it around: keep the skills that are *not* on the list."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "`IN` compares with `==`: `1` and `1.0` are equal, `\"1\"` is not.",
                "Without `select`, `IN` just gives the answer: `.members[] | .role | IN(\"dev\", \"qa\")` is a stream of `true` and `false`.",
            ]),
            Lesson::new(
                "INDEX & JOIN",
                "`INDEX(f)` turns an array into an object keyed by what `f` gives for each element: a lookup table. `INDEX(source; f)` does the same for any stream. `JOIN` matches each element of one table with its entry in another.",
                &[
                    Example::new(
                        "A lookup table by code",
                        SHOP,
                        ".customers | INDEX(.code) | map_values(.city)",
                        &[
                            (".customers", "The array of customers."),
                            ("INDEX(.code)", "Store each customer under the key `.code` gives, as one object."),
                            ("map_values(.city)", "Keep just the city of each entry."),
                        ],
                    ),
                    Example::new(
                        "Look up the customer of every order",
                        SHOP,
                        "INDEX(.customers[]; .code) as $by_code | .orders[] | {order: .no, who: $by_code[.customer].name}",
                        &[
                            ("INDEX(.customers[]; .code)", "The table, built from a stream: every customer, keyed by code."),
                            ("as $by_code", "Saved, to use in the steps that follow."),
                            (".orders[]", "Each order in turn."),
                            ("$by_code[.customer].name", "The order's customer code is the key into the table; then read the name."),
                        ],
                    ),
                    Example::new(
                        "JOIN does the matching",
                        SHOP,
                        "INDEX(.customers[]; .code) as $by_code | JOIN($by_code; .orders[]; .customer; {order: .[0].no, city: .[1].city})",
                        &[
                            ("INDEX(.customers[]; .code) as $by_code", "The same table."),
                            ("JOIN($by_code; .orders[]; .customer;", "For each order, find the table entry under `.customer`, and make the pair `[order, customer]`."),
                            ("{order: .[0].no, city: .[1].city}", "Shape each pair: `.[0]` is the order, `.[1]` its customer."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "When two elements give the same key the later one wins. Keys are always strings, so a numeric id gives the keys `\"1\"`, `\"2\"`… and the lookup is `$table[.id | tostring]`.",
                "A key with no entry in the table gives `null`: `JOIN` pairs the element with `null`.",
                "`JOIN($table; f)` over an array gives every `[element, match]` pair as one array; with a stream and a fourth argument, as above, each pair is shaped by it.",
            ]),
        ],
    ),
    Topic::new(
        "Variables & functions",
        &[
            Lesson::new(
                "Variables",
                "`expr as $name | …` binds a value for the rest of the pipeline — handy when you need it twice. A pattern on the left destructures objects and arrays.",
                &[
                    Example::new(
                        "Average age with a saved count",
                        TEAM,
                        ".members | length as $n | map(.age) | add / $n",
                        &[
                            ("length as $n", "Compute the length (4) and bind it to `$n`; the input stays the members array."),
                            ("map(.age) | add", "The sum of the ages — `.` is still the members array here."),
                            ("/ $n", "Divide by the saved count."),
                        ],
                    ),
                    Example::new(
                        "Destructuring",
                        TEAM,
                        r#". as {team: $t, office: {city: $c}} | "\($t) in \($c)""#,
                        &[
                            ("{team: $t, office: {city: $c}}", "A pattern: pull `team` into `$t` and `office.city` into `$c` in one go."),
                            (r#""\($t) in \($c)""#, "Use the variables like any value — here, inside a string."),
                        ],
                    ),
                ],
            )
            .tips(&["A variable is visible only to the right of its `as`, up to the end of the enclosing pipeline or parentheses."]),
            Lesson::new(
                "Your own functions",
                "`def name(params): body;` defines a reusable filter. A parameter written `$n` receives a value; a plain name receives a filter.",
                &[Example::new(
                    "A reusable filter",
                    TEAM,
                    "def older($n): select(.age > $n); .members[] | older(30) | .name",
                    &[
                        ("def older($n):", "Define a filter called `older` with one value parameter."),
                        ("select(.age > $n)", "Its body: reuse `select`, comparing with the parameter."),
                        (";", "Ends the definition."),
                        ("older(30)", "Call it like any built-in filter."),
                    ],
                )],
            )
            .tips(&["Parameters without a `$` are *filters*, not values: `def twice(f): f | f;` then `3 | twice(. + 1)` gives `5`."]),
            Lesson::new(
                "Recursion: .., paths, walk",
                "`..` visits every value in the document, at any depth. Pair it with a type filter such as `numbers` or `strings`. `walk(f)` rewrites every value, children first.",
                &[
                    Example::new(
                        "Every number, anywhere",
                        NESTED,
                        "[.. | numbers]",
                        &[
                            ("..", "Recursive descent: emit the document and everything nested inside it, at any depth."),
                            ("numbers", "A filter that lets only numbers through."),
                        ],
                    ),
                    Example::new(
                        "Rewrite every number",
                        NESTED,
                        r#"walk(if type == "number" then . * 10 else . end)"#,
                        &[
                            ("walk(", "Apply the filter to every value in the document, children first."),
                            (r#"if type == "number""#, "Test each value's type…"),
                            ("then . * 10 else . end", "…multiply numbers by 10 and leave everything else unchanged."),
                        ],
                    ),
                ],
            )
            .tips(&[
                r#"`[paths]` lists every location as an array of keys and indexes — `["a"]`, `["a",0]`, … — and `getpath(["a",0])` reads one."#,
            ]),
        ],
    ),
    Topic::new(
        "Event streams",
        &[
            Lesson::new(
                "tostream",
                "`tostream` flattens any value into *events*: `[path, value]` for every leaf, and a one-element `[path]` event that closes each array or object. The path lists the keys and indexes that lead to the value.",
                &[
                    Example::new(
                        "The events of a small value",
                        NESTED,
                        ".a | tostream",
                        &[
                            (".a", "The array `[1, [2, 3]]`."),
                            ("tostream", "One `[path, value]` event per number, and a closing `[path]` event after the last item of each array."),
                        ],
                    ),
                    Example::new(
                        "Every leaf as path = value",
                        NESTED,
                        r#"tostream | select(length == 2) | "\(.[0] | map(tostring) | join(".")) = \(.[1])""#,
                        &[
                            ("tostream", "Every event of the whole document."),
                            ("select(length == 2)", "Keep the leaf events; the closing ones have only a path."),
                            (r#".[0] | map(tostring) | join(".")"#, "The path as text: keys and indexes joined with dots."),
                            ("\\(.[1])", "The leaf's value."),
                        ],
                    ),
                    Example::new(
                        "Where is a value?",
                        NESTED,
                        "tostream | select(.[1] == 3) | .[0]",
                        &[
                            ("tostream", "Every event of the document, however deep."),
                            ("select(.[1] == 3)", "Keep the events whose value is 3. A closing event has no value, so for it `.[1]` is `null`."),
                            ("| .[0]", "Read the path: key `a`, then index 1, then index 1 again."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "An empty array or object is a leaf of its own, `[path, []]` or `[path, {}]`, and a lone scalar is `[[], value]`.",
                "A closing event carries the path of its container's *last* child. In the first example `[[1,1]]` ends the inner array `[2, 3]` after its index 1, and `[[1]]` ends the outer array after its index 1.",
            ]),
            Lesson::new(
                "fromstream & truncate_stream",
                "`fromstream(events)` builds values back from events: it gives one value each time an event closes a top-level value. `1 | truncate_stream(events)` cuts the first key or index off every path, so each child of the top level becomes a value of its own.",
                &[
                    Example::new(
                        "Events written by hand (the data isn't used)",
                        SHOP,
                        r#"fromstream([["a"], 1], [["b"], 2], [["b"]])"#,
                        &[
                            (r#"[["a"], 1]"#, "A leaf event: put 1 under the key `a`."),
                            (r#"[["b"], 2]"#, "Another leaf: put 2 under `b`."),
                            (r#"[["b"]]"#, "A closing event for the top-level object, so `fromstream` gives the value it has built."),
                        ],
                    ),
                    Example::new(
                        "Drop a field from every record",
                        SHOP,
                        r#".customers | fromstream(tostream | select(length == 1 or all(.[0][]; . != "city")))"#,
                        &[
                            (".customers | fromstream(", "Rebuild the customers from the events that pass the filter."),
                            ("tostream", "Every event of the array."),
                            ("select(length == 1 or ", "Keep the closing events, so every value still ends properly, and…"),
                            (r#"all(.[0][]; . != "city")"#, "…every other event whose path doesn't mention `city`, however deep it is."),
                        ],
                    ),
                    Example::new(
                        "One result per order",
                        SHOP,
                        ".orders as $o | fromstream(1 | truncate_stream($o | tostream))",
                        &[
                            (".orders as $o", "Save the array: the events given to `truncate_stream` are computed from `null`, so they can't read `.orders` themselves."),
                            ("fromstream(", "Builds values back…"),
                            ("1 | truncate_stream(", "…from events whose paths lost their first step, the order's index…"),
                            ("$o | tostream", "…the events of the whole array. Each order is now a top-level value of its own."),
                        ],
                    ),
                ],
            )
            .tips(&[
                "`fromstream(tostream)` gives the input back, whatever it is.",
                "Leave a leaf event out, as the second example does, and its key is simply absent from the rebuilt value.",
                "`truncate_stream` drops the events whose path is not longer than the cut, so scalars directly in the array vanish: only the objects and arrays inside it come out as values.",
            ]),
        ],
    ),
    Topic::new(
        "Errors",
        &[Lesson::new(
            "try, catch & ?",
            "Some filters fail on some inputs — `tonumber` on `\"two\"`, for instance. `try f catch g` recovers with a fallback, and `f?` simply drops the failure.",
            &[
                Example::new(
                    "A fallback for bad input",
                    NUMBERS,
                    r#".[] | try tonumber catch "n/a""#,
                    &[
                        ("try tonumber", "Run `tonumber`; if it raises an error…"),
                        (r#"catch "n/a""#, "…produce this instead. Inside `catch`, `.` is the error message."),
                    ],
                ),
                Example::new(
                    "Silently skip failures",
                    NUMBERS,
                    "[.[] | tonumber?]",
                    &[("tonumber?", "A trailing `?` is `try` without a `catch`: values that fail just produce no output.")],
                ),
            ],
        )
        .tips(&[r#"`error("message")` raises your own error: `try error("boom") catch "caught"` gives `"caught"`."#])],
    ),
    Topic::new(
        "Reference",
        &[
            Lesson::cheat_sheet(
                "Cheat sheet",
                "The everyday jq filters at a glance, all against the team document. Press ▶ to run one.",
                TEAM,
                &[
                    (".team", "a field"),
                    (".office.city", "nested fields"),
                    (".members[0]", "an array element (negative indexes count from the end)"),
                    (".members[1:3]", "a slice (the end is exclusive)"),
                    (".members[]", "every element, as a stream"),
                    (".members[] | .name", "pipe: feed each output into the next filter"),
                    ("[.members[].age]", "collect a stream into an array"),
                    (".members[] | select(.age > 30)", "keep only what matches"),
                    (".members | map(.age + 1)", "transform every element"),
                    (".members[] | {name, age}", "build a new object"),
                    (r#".members[] | .email // "n/a""#, "a default for null or missing values"),
                    (".members | length", "count"),
                    (".members | map(.age) | add / length", "average"),
                    (".members | sort_by(.age)", "sort"),
                    (".members | group_by(.role)", "group"),
                    (".members | map(.skills) | flatten | unique", "flatten and de-duplicate"),
                    (".members[] | [.name, .age] | @csv", "one CSV row per member (`@tsv` for tabs)"),
                    (".members | INDEX(.name)", "a lookup table keyed by name"),
                    (r#".members[] | select(.role | IN("dev", "qa"))"#, "keep when the value is one of several"),
                    (".office | keys", "an object's keys"),
                    (".office | tostream", "the value as `[path, value]` events"),
                    ("[.. | numbers]", "every number anywhere in the document"),
                    (r#""\(.team) HQ""#, "string interpolation"),
                ],
            ),
            Lesson::new(
                "jq here vs jq 1.7",
                "jsonquery runs jq programs with *jaq*, a Rust re-implementation of jq. Everyday filters behave the same, but a few jq 1.7 features are missing.",
                &[],
            )
            .tips(&[
                "jaq lacks `@csv`, `@tsv`, `IN`, `INDEX`, `JOIN`, `tostream`, `fromstream` and `truncate_stream`; jsonquery adds them, and they work as in jq.",
                "Not available: `$ENV` (use `env`), `input`, `leaf_paths` and `toarray`.",
                "Assignment can't create several missing levels at once: `{} | .a.b.c = 1` is an error here.",
                "Available and handy: `@base64`, `@uri`, `@html`, `@sh`, `@json`, regex (`test`, `capture`, `sub`, `gsub`), `reduce`, `foreach`, `limit`, `first`, `walk`, `paths`, `getpath`, `to_entries`, `with_entries`, `group_by`, `unique_by`, `min_by` and `max_by`.",
            ]),
        ],
    ),
];
