# Known issues

<!-- Made from crates/query/src/known_issues/list.rs: edit that, and write this page again with
     JSONQUERY_WRITE_DOCS=1 cargo test -p jsonquery_gui the_known_issues_page -->

jsonquery runs jq programs with [jaq](https://github.com/01mf02/jaq), a Rust re-implementation of
jq, and adds `@csv`, `@tsv`, `IN`, `INDEX`, `JOIN`, `tostream`, `fromstream` and `truncate_stream`
itself, with its own `fromjson`, `tonumber` and `tojson`. Everyday programs give the results jq 1.8
gives; the ones below do not, yet. The same list is in the app, under **Known issues** in the ⓘ
window (there without the smaller ones).

The *jq 1.8.1* column is what jq itself prints for the program, the *Here* column what this app
does. Every example is run by a test, and so is every workaround: when an issue is fixed, the test
fails until its entry is taken out of the list.

## Not available

**`$ENV` (use `env`), `$ARGS` and `$__loc__`**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `$ENV \| type` | `"object"` | an error |
| `null` | `$ARGS \| keys` | `["named","positional"]` | an error |
| `null` | `$__loc__.line` | `1` | an error |

Workaround: Use `env` for `$ENV`: `env | type` gives `"object"`.

**`input`, `inputs`, `input_filename` and `input_line_number`: a query runs over the one open document**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `[inputs]` | `[]` | an error |
| `null` | `input_filename` | `"<stdin>"` | an error |

**`trimstr`, `format(…)`, `builtins`, `have_decnum`, `have_literal_numbers`, `lgamma_r`, `modulemeta` and `get_search_list`**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `"aba"` | `trimstr("a")` | `"b"` | an error |
| `"x"` | `format("text")` | `"x"` | an error |
| `null` | `builtins \| length` | `226` | an error |
| `null` | `have_decnum` | `true` | an error |

**Syntax: `?//`, patterns like `{$a: [$b]}`, numbers like `.5` and `5.`, and `"\ud83d\ude00"` escapes**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `[[1],2]` | `.[] as [$a] ?// $a \| $a` | `1 2` | an error |
| `{"a":[1]}` | `. as {$a: [$b]} \| [$a, $b]` | `[[1],1]` | an error |
| `null` | `.5 + .25` | `0.75` | an error |
| `null` | `5. + 1` | `6` | an error |
| `null` | `"\ud83d\ude00"` | `"😀"` | an error |

**Modules: `import`, `include` and `module`**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `import "a" as a; 1` | loads the module file | an error |
| `null` | `include "a"; 1` | loads the module file | an error |

## Fails where jq works

**`first(f)`, `limit(n; f)`, `skip(n; f)`, `last(f)` and `try f` can't be the target of `=` or `|=`**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `[1,2,3]` | `first(.[] \| select(. > 1)) \|= 10` | `[1,10,3]` | an error |
| `[1,2,3]` | `limit(2; .[]) \|= 0` | `[0,0,3]` | an error |

Workaround: Take the path first, then update it: `[1,2,3] | getpath(path(first(.[] | select(. > 1)))) |= 10` gives `[1,10,3]`.

**Regex: Unicode classes (`\p{L}`), look-ahead and look-behind, back-references, atomic groups and `test(["re","flags"])` fail**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `"é"` | `test("\\p{L}")` | `true` | an error |
| `"ab"` | `test("(?<=a)b")` | `true` | an error |
| `"aa"` | `test("(a)\\1")` | `true` | an error |
| `"a"` | `test(["A","i"])` | `true` | an error |

**Dates: `gmtime | todate`, `mktime` of a short array, `strptime` of a time without a date or with `%Z`, and dates beyond 9999-12-30 fail**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `1425599621` | `gmtime \| todate` | `"2015-03-05T23:53:41Z"` | an error |
| `[2015,2,5]` | `mktime` | `1425513600` | an error |
| `"10:15"` | `strptime("%H:%M")` | `[1900,0,0,10,15,0,8,367]` | an error |
| `"Thu, 05 Mar 2015 23:51:47 GMT"` | `strptime("%a, %d %b %Y %H:%M:%S %Z")` | `[2015,2,5,23,51,47,4,63]` | an error |
| `253402300799` | `todate` | `"9999-12-31T23:59:59Z"` | an error |

Workaround: Go through `mktime`: `1425599621 | gmtime | mktime | todate` gives `"2015-03-05T23:53:41Z"`.

**`null | .[1:]` fails, and so does an index with a fraction, or that a division made: `.[1.5]`, `.[length / 2]`, `.[1.2:3.5]`**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `.[1:]` | `null` | an error |
| `[1,2,3]` | `.[1.5]` | `2` | an error |
| `[1,2,3,4]` | `.[length / 2]` | `3` | an error |
| `[1,2,3]` | `.[1.2:2.5]` | `[2,3]` | an error |

Workaround: Round it down first: `[1,2,3,4] | .[length / 2 | floor]` gives `3`.

**`ldexp`, `scalbln`, `jn` and `yn` fail on a fraction as an argument**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `ldexp(10; 0.5)` | `10` | an error |
| `null` | `jn(1.5; 2)` | `0.5767248077568733` | an error |

**A value can be put at an index of an array up to 16,777,215, and `.[20000000] = 1` on `[]` is an error (jq: up to 536,870,911)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `[]` | `.[20000000] = 1 \| length` | `20000001` | an error |

## Gives a different result

**`%` binds tighter than `*` and `/`: `2 * 3 % 4` is 6 (jq: 2)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `2 * 3 % 4` | `2` | `6` |
| `null` | `20 / 6 % 4` | `3` | `10.0` |

Workaround: Use parentheses: `(2 * 3) % 4` gives `2`.

**Regex: `\w`, `\d` and `\s` match only ASCII (`"héllo"` gives `h` and `llo`), groups that took no part are left out, and `gsub` with several replacements gives every combination**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `"héllo wörld"` | `[match("\\w+";"g").string]` | `["héllo","wörld"]` | `["h","llo","w","rld"]` |
| `"b"` | `capture("(?<a>a)?(?<b>b)")` | `{"a":null,"b":"b"}` | `{"b":"b"}` |
| `"aa"` | `[gsub("a";"1","2")]` | `["11","22"]` | `["11","12","21","22"]` |

**`has(-1)` is true, `nth(1,2; f)` gives only the first, and `limit(-1; f)` and `flatten(-1)` don't complain (jq: false, both, an error)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `[1]` | `has(-1)` | `false` | `true` |
| `null` | `[nth(1,2; range(10))]` | `[1,2]` | `[1]` |
| `null` | `[limit(-1; 1,2)]` | an error | `[]` |
| `[1,[2]]` | `flatten(-1)` | an error | `[1,[2]]` |

**`5.5 % 2` is 1.5 (jq: 1), `gamma` is the Γ function (jq: lgamma), and `nearbyint` rounds halves up (jq: to even)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `5.5 % 2` | `1` | `1.5` |
| `5` | `gamma` | `3.1780538303479458` | `24.0` |
| `2.5` | `nearbyint` | `2` | `3.0` |

**`1 / 0` and `0 / 0` give `Infinity` and `NaN` (jq: an error)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `1 / 0` | an error | `"Infinity"` |
| `null` | `try (1 / 0) catch "caught"` | `"caught"` | `"Infinity"` |

**With generators on both sides of an operator the results come in another order: `[(1,2) * (3,4)]` is 3 4 6 8 (jq: 3 6 4 8); `reduce` and `foreach` follow every output of the update**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `[(1,2) * (3,4)]` | `[3,6,4,8]` | `[3,4,6,8]` |
| `null` | `[(1,2) + (10,20)]` | `[11,12,21,22]` | `[11,21,12,22]` |
| `null` | `reduce (1,2) as $x (0; ., . + $x)` | `3` | `0 2 1 3` |

**`todate` keeps fractions of a second, `strftime` of `%c` and `%x` is worded differently, and `fromdate` accepts `+01:00` where jq refuses**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `1425599621.678` | `todate` | `"2015-03-05T23:53:41Z"` | `"2015-03-05T23:53:41.678Z"` |
| `1425599621` | `strftime("%c")` | `"Thu Mar  5 23:53:41 2015"` | `"2015 M03 5, Thu 23:53:41"` |
| `"2015-03-05T23:51:47+01:00"` | `fromdate` | an error | `1425595907` |

**A program jq rejects runs: `.[0]` on an object is `null`**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `{"a":1}` | `.[0]` | an error | `null` |

**`fromjson` reads `#` comments, and refuses `.5`, `5.`, `nan`, `inf` and a byte-order mark (jq: the reverse)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `"1 # one"` | `fromjson` | an error | `1` |
| `".5"` | `fromjson` | `0.5` | an error |
| `"nan"` | `fromjson` | `null` | an error |

**`.a.b |= empty` and `.a[]? |= f`, where `a` isn't there, leave `"a": null` behind (jq changes nothing)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `{"x":1}` | `.a.b \|= empty` | `{"x":1}` | `{"x":1,"a":null}` |
| `{"x":1}` | `.a[]? \|= 1` | `{"x":1}` | `{"x":1,"a":null}` |

**An object can be made with a key that is not a string: `{(1): 2}` is an error in jq, and here an object that is shown as text**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `{(1): 2}` | an error | a string of text |

## Looks different

**Whole numbers from arithmetic and math functions show a `.0`: `10 / 2` is `5.0` (jq: `5`)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `10 / 2` | `5` | `5.0` |
| `[1,2,3]` | `add / length` | `2` | `2.0` |
| `16` | `sqrt` | `4` | `4.0` |

**`NaN` and infinities turn the whole result into text: `[nan]` shows as "[NaN]" (jq: `[null]`)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `[nan]` | `[null]` | `"[NaN]"` |
| `null` | `infinite` | `1.7976931348623157e+308` | `"Infinity"` |

**A result is shown down to 127 levels of nesting and cut off there (jq: 256), and `fromjson` reads text nested up to 1,000 levels (jq: 10,000)**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `reduce range(200) as $i ([]; [.])` | all 201 levels | the nested arrays, cut off at 127 levels |
| `null` | `"[" * 1001 + "]" * 1001 \| fromjson \| length` | `1` | an error |

**`1e3` shows as `1e+3` and `-0` as `0`, and error messages are worded differently**

| Input | Program | jq 1.8.1 | Here |
|---|---|---|---|
| `null` | `1e3` | `1E+3` | `1e+3` |
| `-0` | `.` | `-0` | `0` |
| `"abc"` | `try (-.) catch .` | `"string (\"abc\") cannot be negated"` | `"cannot use \"abc\" as number"` |

## Not in jq 1.8.1 either

These are missing here, and from jq 1.8.1 too (older versions of jq had some of them), so they are
not issues: `leaf_paths`, `recurse_down`, `toarray`, `ascii`, `date`, `dateadd`, `datesub`,
`@base32` and `@base32d`.
