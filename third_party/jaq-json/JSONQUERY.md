# jaq-json 2.0.3, with changes

This is [jaq-json](https://crates.io/crates/jaq-json) 2.0.3 (MIT, by Michael Färber, the
`LICENSE-MIT` file is jaq's), copied from crates.io, with the changes below. It is a path
dependency of `crates/query` (`jaq-json = { version = "2.0.3", path = "../../third_party/jaq-json" }`),
and the workspace `exclude`s it, so that `cargo fmt`, `cargo clippy` and `cargo test` of the
workspace leave it alone.

## Why

jaq-json is the type of the values jaq works on, `Val`, and `impl ValT for Val` is where every
assignment ends up: `=`, `|=`, `+=`, `//=`, `setpath`, `del` and `delpaths` (jaq's), walk the path
and call `Val::map_index` (a key or an index) or `Val::map_range` (a slice) for each step. What
they do with `null` and with the end of an array cannot be changed from outside the crate, and it
differs from jq.

## The changes

All in `src/lib.rs`, marked `jsonquery:`:

| | jaq-json 2.0.3 | here (and in jq 1.8.1) |
|---|---|---|
| `null \| .a.b = 1` | an error | `{"a":{"b":1}}`: `null` becomes the object a key asks for, the array an index asks for |
| `{"x":1} \| .a.b = 1` | an error (`null` under `a`) | `{"x":1,"a":{"b":1}}` |
| `null \| .[1:] = [9]`, `.b[1:] \|= [9]` | an error | `[9]`: a slice of `null` is the array put there |
| `[0] \| .[2] = 1` | an error (`index 2 out of bounds`) | `[0,null,1]`: an index past the end makes the array longer |
| `[1] \| .[1700000000] = 1` | an error | an error, `Array index too large`, from 16,777,216 on (jq: 536,870,912; an index meant as a key must not use up the memory) |
| `[1,2,3] \| .[-4] = 9` | an error | an error, `Out of bounds negative array index` |
| `{} \| .[0] = 1` | `{"0":1}`, an object with a key that is a number | an error, as in jq: only a string is a key |
| `{"a":1,"b":2,"c":3} \| .a \|= empty` | `{"c":3,"b":2}` (`swap_remove`) | `{"b":2,"c":3}` (`shift_remove`) |
| `null \| .a \|= empty` | an error | `null`: where nothing is put, nothing is made |

Left as they are, and different from jq: `{"x":1} | .a.b |= empty` and `.a[]? |= f` leave
`"a": null` behind (jq changes nothing), and an index that is a number with a fraction, or that
a division made (`.[length / 2]`), is an error. `docs/known-issues.md` of the app lists them.

`Cargo.toml` has two additions: the `[[test]]` of `tests/writes.rs`, and a dev-dependency on
jaq-std with all of its features (the tests of jaq were written for its own workspace, where it has
them).

## Tests

`tests/writes.rs` is new; `tests/defs.rs` and `tests/funs.rs` are jaq's own and pass as they were.
They need the `serde` feature of this crate:

    CARGO_TARGET_DIR=/tmp/jaq-json-target cargo test --manifest-path third_party/jaq-json/Cargo.toml --features serde

(Cargo makes a `Cargo.lock` and a `target/` here; both are ignored.) What is also tested through
the app, with the cases of jq 1.8.1: `crates/query/src/jq_ext.rs`, `jq_ext/properties.rs` and
`jq_ext/corpus.rs`.

## How the copy differs from the original

    diff -ru ~/.cargo/registry/src/*/jaq-json-2.0.3/src third_party/jaq-json/src

shows these changes and no others.

## Dropping the copy

When jaq-json itself does what the table says, or lets a program decide it: delete this folder,
put `jaq-json = "<version>"` back in `crates/query/Cargo.toml`, take the `exclude` out of the
`Cargo.toml` of the workspace, and run the tests; those of `jq_ext` tell what is left to do.
