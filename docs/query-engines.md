# Query Engines

jsonquery supports four query dialects — jq, JSON Pointer, JSONPath, and
JMESPath — behind a shared `jsonquery_core::engine::QueryEngine` trait. This
page covers how dialect selection works, and surveys the wider JSON-query
landscape (JSONPath variants, JSONata, the JS-idiomatic tools) for what
might be worth adding next.

## How dialect selection works

- **JSON Pointer** (`crates/query/src/json_pointer.rs`, `JsonPointerEngine`) — RFC 6901 via `serde_json::Value::pointer`.
- **JSONPath** (`crates/query/src/jsonpath.rs`, `JsonPathEngine`) — RFC 9535 via [jsonpath-rust](https://crates.io/crates/jsonpath-rust).
- **JMESPath** (`crates/query/src/jmespath_engine.rs`, `JmesPathEngine`) — via the [jmespath](https://crates.io/crates/jmespath) crate.
- **jq** (`crates/query/src/jq.rs`, `JaqEngine`) — via the embedded [jaq](https://github.com/01mf02/jaq).

A `jsonquery_query::Kind` enum (`crates/query/src/lib.rs`) ties each
dialect's id to its engine and adds `Kind::detect(query_text)` —
best-effort auto-selection from the query's own syntax (a leading `/` means
JSON Pointer, a leading `$` means JSONPath, a few JMESPath-only substrings
like `[?`/backtick literals/`&&`/`||` mean JMESPath, everything else —
including jq's own leading `.` — defaults to jq). The query bar has four
small selectable-label buttons, top right, one per dialect: none selected
(the default) means "auto," clicking one pins that engine explicitly, and
the status bar reports which engine actually ran — e.g. `jq · auto` vs. a
manually-picked `JMESPath` — so a result is never ambiguous about which
dialect produced it.

> **Pitfall: JMESPath + `arbitrary_precision`.** This workspace's
> `serde_json` runs with the `arbitrary_precision` feature; `jmespath`'s
> generic `Serialize`-based `ToJmespath` impl doesn't understand the private
> sentinel that feature emits for numbers, so feeding it a `&Value` the
> "obvious" way silently turned every number into an unrelated object and
> broke numeric filters (`` age > `30` `` matched nothing). Fixed in
> `jmespath_engine.rs` by converting `Value → jmespath::Variable` by hand,
> matching on `Value`'s variants directly instead of round-tripping through
> `Serialize`. Worth checking for again if a future engine also bridges
> through a generic `Serialize` impl rather than matching `Value` natively.

## Comparison matrix

Checked against crates.io/GitHub on 2026-09-05 — versions and maintenance
status move; re-verify before implementing something new here.

| Dialect | Crate(s) | License | Adoption | Why | Verdict |
|---|---|---|---|---|---|
| **JSON Pointer** (RFC 6901) | built into `serde_json` — already a dependency | MIT/Apache-2.0 | — | `Value::pointer()` ships today. Zero new dependency, zero new risk. | **Shipped** |
| **JSONPath** (RFC 9535) | [jsonpath-rust](https://crates.io/crates/jsonpath-rust) | MIT | ~88M downloads | Its 1.0 rewrite is RFC 9535 compliant "except several cases." Read-only, and its API — `value.query("$.foo[*]") -> Vec<&Value>` — is close to `QueryEngine::run`'s shape. | **Shipped** |
| **JSONPath** (RFC 9535) | [serde_json_path](https://crates.io/crates/serde_json_path) | MIT | ~2.3M downloads | Also serde_json-native and RFC-9535-targeted. Smaller install base than jsonpath-rust, otherwise comparable. | Alternative |
| **JMESPath** | [jmespath](https://crates.io/crates/jmespath) | MIT | ~2.8M downloads | A well-established standard in its own right (AWS CLI `--query`, Terraform). crates.io shows an update as recent as 2026-01-19. | **Shipped** as `JmesPathEngine` — see the pitfall above |
| **JMESPath** | [jmespath_community](https://crates.io/crates/jmespath_community) | Apache-2.0 | ~7K downloads | A newer community fork with a faster commit cadence per its own docs, but a tiny fraction of the original crate's adoption so far. | Alternative |
| **Rust-native query DSL** | [jql-runner](https://crates.io/crates/jql-runner) | MIT/Apache-2.0 | ~91K downloads | Already written in Rust — the smallest possible integration gap of anything here — but its own bespoke syntax has far less outside familiarity than jq, JSONPath, or JMESPath. | Watch |
| **JSONata** | [jsonata-rs](https://github.com/Stedi/jsonata-rs) | Apache-2.0 | ~205K downloads | Explicitly alpha: ~800 of 1,000 upstream conformance tests pass, function signatures and regex are unsupported, last published 2025-02-03. | Caution |
| **jsonquery** (jsonquerylang.org's own DSL) | none — JS/TS + Python reference implementations only | ISC | — | No Rust port exists anywhere. Deliberately small (~4.2 kB minified, pipe syntax, ~50 built-ins). | Future — from scratch |
| **JS-idiomatic tools** (JSPath, Lodash, Underscore, raw JavaScript) | [boa_engine](https://github.com/boa-dev/boa) — a JS engine, not a query-language crate | Unlicense OR MIT | ~4.7M downloads | None of these four have a Rust port individually — they're JS utility libraries, not declarative query languages. boa is the only pure-Rust JS engine of note, so it's the one way to support all four at once (load lodash/underscore's own source as a JS "prelude") without a QuickJS/V8 binding. | Future — bigger lift |

**Bottom line:** JSON Pointer, jsonpath-rust, and jmespath are implemented.
JSONata, jsonquerylang.org's own DSL, and a boa-backed JS/Lodash mode remain
separately-scoped follow-ups — an immature dependency, a from-scratch
implementation, and a full embedded scripting engine, respectively.

## Out of scope: format conversion

`xml2js`-style "JSON to XML" is a format converter, not a query language,
and unrelated to `QueryEngine`. A CSV/NDJSON/XML export option would be
another output format, not a new dialect — `quick-xml` or `serde-xml-rs`
would be the Rust tool for XML specifically, if it's ever wanted.

---

### Sources

Checked 2026-09-05 via crates.io's API and each project's GitHub/homepage.
Crate versions and maintenance status move — re-verify before implementation.

- [jsonquerylang/jsonquery](https://github.com/jsonquerylang/jsonquery) — the jsonquerylang.org DSL: syntax, ISC license, JS/TS + Python reference implementations, no Rust port.
- [jsonata.org](https://jsonata.org/) and [Stedi/jsonata-rs](https://github.com/Stedi/jsonata-rs) — JSONata language and its alpha-stage Rust port.
- [jsonpath-rust](https://crates.io/crates/jsonpath-rust) and [serde_json_path](https://crates.io/crates/serde_json_path) (crates.io) — RFC 9535 JSONPath implementations.
- [RFC 9535](https://www.rfc-editor.org/rfc/rfc9535.html) — JSONPath: Query Expressions for JSON.
- [jmespath](https://crates.io/crates/jmespath) and [jmespath_community](https://crates.io/crates/jmespath_community) (crates.io) — JMESPath implementations.
- [jql-runner](https://crates.io/crates/jql-runner) (crates.io) — the Rust-native JSON Query Language tool.
- [boa-dev/boa](https://github.com/boa-dev/boa) — pure-Rust JavaScript engine, evaluated as the route to JSPath/Lodash/Underscore/raw-JS support.
- RFC 6901 (JSON Pointer) — satisfied already by `serde_json::Value::pointer()`, no separate crate.
