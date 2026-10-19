//! The list: what is not as it is in jq, by kind, the worst first in each. The
//! programs were each run in jq 1.8.1 for the `jq` text; the tests in the parent
//! module run them here. Take an issue (or an example of one) out when it is fixed.

use super::{Example, Group, Here, Issue, Workaround};

/// The groups, in the order the About window and the page show them. An issue that
/// is `minor` is left out of the window, and is on the page.
pub const GROUPS: &[Group] = &[
    Group::new("Not available", &[
        Issue::new(
            "env-vars",
            "`$ENV` (use `env`), `$ARGS` and `$__loc__`",
            &[
                Example::new("null", "$ENV | type", r#"`"object"`"#, Here::Error),
                Example::new("null", "$ARGS | keys", r#"`["named","positional"]`"#, Here::Error),
                Example::new("null", "$__loc__.line", "`1`", Here::Error),
            ],
        )
        .workaround(Workaround::new("Use `env` for `$ENV`", "null", "env | type", r#""object""#))
        ,
        Issue::new(
            "input",
            "`input`, `inputs`, `input_filename` and `input_line_number`: a query runs over the one open document",
            &[
                Example::new("null", "[inputs]", "`[]`", Here::Error),
                Example::new("null", "input_filename", r#"`"<stdin>"`"#, Here::Error),
            ],
        )
        ,
        Issue::new(
            "builtins-missing",
            "`trimstr`, `format(…)`, `builtins`, `have_decnum`, `have_literal_numbers`, `lgamma_r`, `modulemeta` and `get_search_list`",
            &[
                Example::new(r#""aba""#, r#"trimstr("a")"#, r#"`"b"`"#, Here::Error),
                Example::new(r#""x""#, r#"format("text")"#, r#"`"x"`"#, Here::Error),
                Example::new("null", "builtins | length", "`226`", Here::Error),
                Example::new("null", "have_decnum", "`true`", Here::Error),
            ],
        )
        ,
        Issue::new(
            "syntax",
            r#"Syntax: `?//`, patterns like `{$a: [$b]}`, numbers like `.5` and `5.`, and `"\ud83d\ude00"` escapes"#,
            &[
                Example::new("[[1],2]", ".[] as [$a] ?// $a | $a", "`1 2`", Here::Error),
                Example::new(r#"{"a":[1]}"#, ". as {$a: [$b]} | [$a, $b]", "`[[1],1]`", Here::Error),
                Example::new("null", ".5 + .25", "`0.75`", Here::Error),
                Example::new("null", "5. + 1", "`6`", Here::Error),
                Example::new("null", r#""\ud83d\ude00""#, r#"`"😀"`"#, Here::Error),
            ],
        )
        ,
        Issue::new(
            "modules",
            "Modules: `import`, `include` and `module`",
            &[
                Example::new("null", r#"import "a" as a; 1"#, "loads the module file", Here::Error),
                Example::new("null", r#"include "a"; 1"#, "loads the module file", Here::Error),
            ],
        )
        ,
    ]),
    Group::new("Fails where jq works", &[
        Issue::new(
            "path-first",
            "`first(f)`, `limit(n; f)`, `skip(n; f)`, `last(f)` and `try f` can't be the target of `=` or `|=`",
            &[
                Example::new("[1,2,3]", "first(.[] | select(. > 1)) |= 10", "`[1,10,3]`", Here::Error),
                Example::new("[1,2,3]", "limit(2; .[]) |= 0", "`[0,0,3]`", Here::Error),
            ],
        )
        .workaround(Workaround::new("Take the path first, then update it", "[1,2,3]", "getpath(path(first(.[] | select(. > 1)))) |= 10", "[1,10,3]"))
        ,
        Issue::new(
            "regex-features",
            r#"Regex: Unicode classes (`\p{L}`), look-ahead and look-behind, back-references, atomic groups and `test(["re","flags"])` fail"#,
            &[
                Example::new(r#""é""#, r#"test("\\p{L}")"#, "`true`", Here::Error),
                Example::new(r#""ab""#, r#"test("(?<=a)b")"#, "`true`", Here::Error),
                Example::new(r#""aa""#, r#"test("(a)\\1")"#, "`true`", Here::Error),
                Example::new(r#""a""#, r#"test(["A","i"])"#, "`true`", Here::Error),
            ],
        )
        ,
        Issue::new(
            "dates",
            "Dates: `gmtime | todate`, `mktime` of a short array, `strptime` of a time without a date or with `%Z`, and dates beyond 9999-12-30 fail",
            &[
                Example::new("1425599621", "gmtime | todate", r#"`"2015-03-05T23:53:41Z"`"#, Here::Error),
                Example::new("[2015,2,5]", "mktime", "`1425513600`", Here::Error),
                Example::new(r#""10:15""#, r#"strptime("%H:%M")"#, "`[1900,0,0,10,15,0,8,367]`", Here::Error),
                Example::new(r#""Thu, 05 Mar 2015 23:51:47 GMT""#, r#"strptime("%a, %d %b %Y %H:%M:%S %Z")"#, "`[2015,2,5,23,51,47,4,63]`", Here::Error),
                Example::new("253402300799", "todate", r#"`"9999-12-31T23:59:59Z"`"#, Here::Error),
            ],
        )
        .workaround(Workaround::new("Go through `mktime`", "1425599621", "gmtime | mktime | todate", r#""2015-03-05T23:53:41Z""#))
        ,
        Issue::new(
            "slice-null",
            "`null | .[1:]` fails, and so does an index with a fraction, or that a division made: `.[1.5]`, `.[length / 2]`, `.[1.2:3.5]`",
            &[
                Example::new("null", ".[1:]", "`null`", Here::Error),
                Example::new("[1,2,3]", ".[1.5]", "`2`", Here::Error),
                Example::new("[1,2,3,4]", ".[length / 2]", "`3`", Here::Error),
                Example::new("[1,2,3]", ".[1.2:2.5]", "`[2,3]`", Here::Error),
            ],
        )
        .workaround(Workaround::new("Round it down first", "[1,2,3,4]", ".[length / 2 | floor]", "3"))
        ,
        Issue::new(
            "fraction-args",
            "`ldexp`, `scalbln`, `jn` and `yn` fail on a fraction as an argument",
            &[
                Example::new("null", "ldexp(10; 0.5)", "`10`", Here::Error),
                Example::new("null", "jn(1.5; 2)", "`0.5767248077568733`", Here::Error),
            ],
        )
        .minor()
        ,
        Issue::new(
            "write-limit",
            "A value can be put at an index of an array up to 16,777,215, and `.[20000000] = 1` on `[]` is an error (jq: up to 536,870,911)",
            &[
                Example::new("[]", ".[20000000] = 1 | length", "`20000001`", Here::Error),
            ],
        )
        .minor()
        ,
    ]),
    Group::new("Gives a different result", &[
        Issue::new(
            "percent-precedence",
            "`%` binds tighter than `*` and `/`: `2 * 3 % 4` is 6 (jq: 2)",
            &[
                Example::new("null", "2 * 3 % 4", "`2`", Here::Output(&["6"])),
                Example::new("null", "20 / 6 % 4", "`3`", Here::Output(&["10.0"])),
            ],
        )
        .workaround(Workaround::new("Use parentheses", "null", "(2 * 3) % 4", "2"))
        ,
        Issue::new(
            "regex-semantics",
            r#"Regex: `\w`, `\d` and `\s` match only ASCII (`"héllo"` gives `h` and `llo`), groups that took no part are left out, and `gsub` with several replacements gives every combination"#,
            &[
                Example::new(r#""héllo wörld""#, r#"[match("\\w+";"g").string]"#, r#"`["héllo","wörld"]`"#, Here::Output(&[r#"["h","llo","w","rld"]"#])),
                Example::new(r#""b""#, r#"capture("(?<a>a)?(?<b>b)")"#, r#"`{"a":null,"b":"b"}`"#, Here::Output(&[r#"{"b":"b"}"#])),
                Example::new(r#""aa""#, r#"[gsub("a";"1","2")]"#, r#"`["11","22"]`"#, Here::Output(&[r#"["11","12","21","22"]"#])),
            ],
        )
        ,
        Issue::new(
            "odd-arguments",
            "`has(-1)` is true, `nth(1,2; f)` gives only the first, and `limit(-1; f)` and `flatten(-1)` don't complain (jq: false, both, an error)",
            &[
                Example::new("[1]", "has(-1)", "`false`", Here::Output(&["true"])),
                Example::new("null", "[nth(1,2; range(10))]", "`[1,2]`", Here::Output(&["[1]"])),
                Example::new("null", "[limit(-1; 1,2)]", "an error", Here::Output(&["[]"])),
                Example::new("[1,[2]]", "flatten(-1)", "an error", Here::Output(&["[1,[2]]"])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "number-functions",
            "`5.5 % 2` is 1.5 (jq: 1), `gamma` is the Γ function (jq: lgamma), and `nearbyint` rounds halves up (jq: to even)",
            &[
                Example::new("null", "5.5 % 2", "`1`", Here::Output(&["1.5"])),
                Example::new("5", "gamma", "`3.1780538303479458`", Here::Output(&["24.0"])),
                Example::new("2.5", "nearbyint", "`2`", Here::Output(&["3.0"])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "div-zero",
            "`1 / 0` and `0 / 0` give `Infinity` and `NaN` (jq: an error)",
            &[
                Example::new("null", "1 / 0", "an error", Here::Output(&[r#""Infinity""#])),
                Example::new("null", r#"try (1 / 0) catch "caught""#, r#"`"caught"`"#, Here::Output(&[r#""Infinity""#])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "output-order",
            "With generators on both sides of an operator the results come in another order: `[(1,2) * (3,4)]` is 3 4 6 8 (jq: 3 6 4 8); `reduce` and `foreach` follow every output of the update",
            &[
                Example::new("null", "[(1,2) * (3,4)]", "`[3,6,4,8]`", Here::Output(&["[3,4,6,8]"])),
                Example::new("null", "[(1,2) + (10,20)]", "`[11,12,21,22]`", Here::Output(&["[11,21,12,22]"])),
                Example::new("null", "reduce (1,2) as $x (0; ., . + $x)", "`3`", Here::Output(&["0", "2", "1", "3"])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "date-details",
            "`todate` keeps fractions of a second, `strftime` of `%c` and `%x` is worded differently, and `fromdate` accepts `+01:00` where jq refuses",
            &[
                Example::new("1425599621.678", "todate", r#"`"2015-03-05T23:53:41Z"`"#, Here::Output(&[r#""2015-03-05T23:53:41.678Z""#])),
                Example::new("1425599621", r#"strftime("%c")"#, r#"`"Thu Mar  5 23:53:41 2015"`"#, Here::Output(&[r#""2015 M03 5, Thu 23:53:41""#])),
                Example::new(r#""2015-03-05T23:51:47+01:00""#, "fromdate", "an error", Here::Output(&["1425595907"])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "lenient",
            "A program jq rejects runs: `.[0]` on an object is `null`",
            &[
                Example::new(r#"{"a":1}"#, ".[0]", "an error", Here::Output(&["null"])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "fromjson-reader",
            "`fromjson` reads `#` comments, and refuses `.5`, `5.`, `nan`, `inf` and a byte-order mark (jq: the reverse)",
            &[
                Example::new(r#""1 # one""#, "fromjson", "an error", Here::Output(&["1"])),
                Example::new(r#"".5""#, "fromjson", "`0.5`", Here::Error),
                Example::new(r#""nan""#, "fromjson", "`null`", Here::Error),
            ],
        )
        .minor()
        ,
        Issue::new(
            "update-empty",
            r#"`.a.b |= empty` and `.a[]? |= f`, where `a` isn't there, leave `"a": null` behind (jq changes nothing)"#,
            &[
                Example::new(r#"{"x":1}"#, ".a.b |= empty", r#"`{"x":1}`"#, Here::Output(&[r#"{"x":1,"a":null}"#])),
                Example::new(r#"{"x":1}"#, ".a[]? |= 1", r#"`{"x":1}`"#, Here::Output(&[r#"{"x":1,"a":null}"#])),
            ],
        )
        .minor()
        ,
        Issue::new(
            "object-keys",
            "An object can be made with a key that is not a string: `{(1): 2}` is an error in jq, and here an object that is shown as text",
            &[
                Example::new("null", "{(1): 2}", "an error", Here::Text),
            ],
        )
        .minor()
        ,
    ]),
    Group::new("Looks different", &[
        Issue::new(
            "float-zero",
            "Whole numbers from arithmetic and math functions show a `.0`: `10 / 2` is `5.0` (jq: `5`)",
            &[
                Example::new("null", "10 / 2", "`5`", Here::Output(&["5.0"])),
                Example::new("[1,2,3]", "add / length", "`2`", Here::Output(&["2.0"])),
                Example::new("16", "sqrt", "`4`", Here::Output(&["4.0"])),
            ],
        )
        ,
        Issue::new(
            "nan-text",
            r#"`NaN` and infinities turn the whole result into text: `[nan]` shows as "[NaN]" (jq: `[null]`)"#,
            &[
                Example::new("null", "[nan]", "`[null]`", Here::Output(&[r#""[NaN]""#])),
                Example::new("null", "infinite", "`1.7976931348623157e+308`", Here::Output(&[r#""Infinity""#])),
            ],
        )
        ,
        Issue::new(
            "deep-limits",
            "A result is shown down to 127 levels of nesting and cut off there (jq: 256), and `fromjson` reads text nested up to 1,000 levels (jq: 10,000)",
            &[
                Example::new("null", "reduce range(200) as $i ([]; [.])", "all 201 levels", Here::Cut(127)),
                Example::new("null", r#""[" * 1001 + "]" * 1001 | fromjson | length"#, "`1`", Here::Error),
            ],
        )
        .minor()
        ,
        Issue::new(
            "spelling",
            "`1e3` shows as `1e+3` and `-0` as `0`, and error messages are worded differently",
            &[
                Example::new("null", "1e3", "`1E+3`", Here::Output(&["1e+3"])),
                Example::new("-0", ".", "`-0`", Here::Output(&["0"])),
                Example::new(r#""abc""#, "try (-.) catch .", r#"`"string (\"abc\") cannot be negated"`"#, Here::Output(&[r#""cannot use \"abc\" as number""#])),
            ],
        )
        .minor()
        ,
    ]),
];
