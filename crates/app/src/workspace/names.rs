//! What a file in the workspace is: which kind of item its name says it is, the
//! sample that goes with it, and which names both a file system and this app can
//! take.
//!
//! The kind of an item is in the end of its file name, so that the files are what
//! other tools expect of them (a `.jq` file is a jq program, a `.schema.json` file
//! a JSON Schema, a `.patch.json` file a JSON Patch) and so that a person who
//! opens the folder can tell them apart:
//!
//! | File | What it is |
//! |---|---|
//! | `name.jq`, `name.jsonpath`, `name.jmespath`, `name.jsonptr` | a query in that language |
//! | `name.schema.json` | a JSON Schema, for Validate |
//! | `name.patch.json` | a JSON Patch or a JSON Merge Patch, for Patch |
//! | `name.json`, `name.ndjson`, `name.jsonl` | a document, to open as Source |
//! | `name.sample.json` | the sample of the items called `name` in that folder |
//!
//! Anything else in a folder is not the workspace's, and is left alone.

use jsonquery_query::Kind as Engine;

/// What a sample's file name ends in.
pub const SAMPLE_SUFFIX: &str = ".sample.json";

/// The longest a name may be, in bytes: a file name (with the end that says what
/// it is) has to fit in the 255 that the file systems allow.
pub const MAX_NAME_BYTES: usize = 120;

/// What an item holds, and so where it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// A query, in one of the four languages: it goes in the query box.
    Query(Engine),
    /// A JSON Schema: it goes in the Schema box of Validate.
    Schema,
    /// A JSON Patch or a JSON Merge Patch: it goes in the Patch box of Patch.
    Patch,
    /// A JSON document, to open as the Source.
    Document,
}

/// The sorts of item the pane can be asked to list: one of its chips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Group {
    #[default]
    All,
    Queries,
    Schemas,
    Patches,
    Documents,
}

impl Group {
    pub const ALL: [Group; 5] = [
        Group::All,
        Group::Queries,
        Group::Schemas,
        Group::Patches,
        Group::Documents,
    ];

    /// What the chip says.
    pub fn label(self) -> &'static str {
        match self {
            Group::All => "All",
            Group::Queries => "Queries",
            Group::Schemas => "Schemas",
            Group::Patches => "Patches",
            Group::Documents => "Docs",
        }
    }

    /// Whether an item of `kind` is one of these.
    pub fn takes(self, kind: ItemKind) -> bool {
        match self {
            Group::All => true,
            Group::Queries => matches!(kind, ItemKind::Query(_)),
            Group::Schemas => kind == ItemKind::Schema,
            Group::Patches => kind == ItemKind::Patch,
            Group::Documents => kind == ItemKind::Document,
        }
    }
}

impl ItemKind {
    /// The end that a file of this kind is given when it is made.
    pub fn suffix(self) -> &'static str {
        match self {
            ItemKind::Query(Engine::Jq) => ".jq",
            ItemKind::Query(Engine::JsonPath) => ".jsonpath",
            ItemKind::Query(Engine::JmesPath) => ".jmespath",
            ItemKind::Query(Engine::JsonPointer) => ".jsonptr",
            ItemKind::Schema => ".schema.json",
            ItemKind::Patch => ".patch.json",
            ItemKind::Document => ".json",
        }
    }

    /// The few letters in the little tag beside an item.
    pub fn badge(self) -> &'static str {
        match self {
            ItemKind::Query(Engine::Jq) => "jq",
            ItemKind::Query(Engine::JsonPath) => "JP",
            ItemKind::Query(Engine::JmesPath) => "JM",
            ItemKind::Query(Engine::JsonPointer) => "PT",
            ItemKind::Schema => "schema",
            ItemKind::Patch => "patch",
            ItemKind::Document => "json",
        }
    }

    /// What it is called in a sentence.
    pub fn what(self) -> &'static str {
        match self {
            ItemKind::Query(_) => "query",
            ItemKind::Schema => "schema",
            ItemKind::Patch => "patch",
            ItemKind::Document => "document",
        }
    }
}

/// The ends that say what a file is, longest first (`.schema.json` is a schema, not
/// a document that ends in `.json`).
const SUFFIXES: [(&str, ItemKind); 9] = [
    (".schema.json", ItemKind::Schema),
    (".patch.json", ItemKind::Patch),
    (".jsonpath", ItemKind::Query(Engine::JsonPath)),
    (".jmespath", ItemKind::Query(Engine::JmesPath)),
    (".jsonptr", ItemKind::Query(Engine::JsonPointer)),
    (".jq", ItemKind::Query(Engine::Jq)),
    (".ndjson", ItemKind::Document),
    (".jsonl", ItemKind::Document),
    (".json", ItemKind::Document),
];

/// A file name that is an item's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    /// What the item is called: the file name without its end.
    pub name: String,
    pub kind: ItemKind,
    /// The end the file has, as it is written (`.jq`, `.NDJSON`).
    pub ext: String,
}

/// What `file` is, if it is an item's file: not a sample, not hidden, and one of
/// the ends above after at least one letter.
pub fn parse_file_name(file: &str) -> Option<Parsed> {
    if file.starts_with('.') || ends_with_ignoring_case(file, SAMPLE_SUFFIX) {
        return None;
    }
    // (Every end starts with a dot, so a file that is only an end was refused as hidden above:
    // the name is never empty here.)
    SUFFIXES.iter().find_map(|(suffix, kind)| {
        if !ends_with_ignoring_case(file, suffix) {
            return None;
        }
        let cut = file.len() - suffix.len();
        let (name, ext) = (file.get(..cut)?, file.get(cut..)?);
        Some(Parsed {
            name: name.to_owned(),
            kind: *kind,
            ext: ext.to_owned(),
        })
    })
}

/// Whether `text` ends in `suffix`, whatever the case (a file that is `A.JQ` is a
/// jq program on a system that does not tell the cases apart, and Windows is one).
fn ends_with_ignoring_case(text: &str, suffix: &str) -> bool {
    text.len() >= suffix.len()
        && text
            .get(text.len() - suffix.len()..)
            .is_some_and(|end| end.eq_ignore_ascii_case(suffix))
}

/// The language of the query that a file called `file` is, if it is one: what its name
/// ends in says (`.jq`, `.jsonpath`, `.jmespath`, `.jsonptr`), whether it is in a
/// workspace or not, and hidden or not (a `.jq` file in a home folder is the one jq
/// reads on its own).
pub fn query_language_of(file: &str) -> Option<Engine> {
    SUFFIXES.iter().find_map(|(suffix, kind)| match kind {
        ItemKind::Query(engine) if ends_with_ignoring_case(file, suffix) => Some(*engine),
        _ => None,
    })
}

/// The file name of the sample that goes with the items called `name`.
pub fn sample_file_name(name: &str) -> String {
    format!("{name}{SAMPLE_SUFFIX}")
}

/// What may not be in a name, because a file system or a path would take it for
/// something else.
const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// The names Windows keeps for devices, which no file there can have (with or
/// without an end).
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The name `raw` stands for, if a file or a folder can have it on every system
/// the workspace may be taken to: it is trimmed, and is not empty, not too long,
/// has none of the characters a path is made of, does not start or end with a
/// dot (a dot at the start hides it, and at the end Windows loses it), and is not
/// a name Windows keeps for itself. The words are what the person who typed it is
/// told.
pub fn check_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Give it a name".to_owned());
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(format!(
            "That name is too long: at most {MAX_NAME_BYTES} letters"
        ));
    }
    if name
        .chars()
        .any(|c| FORBIDDEN.contains(&c) || c.is_control())
    {
        return Err("A name can't have any of / \\ : * ? \" < > | in it".to_owned());
    }
    if name.starts_with('.') {
        return Err("A name can't start with a dot".to_owned());
    }
    if name.ends_with('.') {
        return Err("A name can't end with a dot".to_owned());
    }
    let first = name.split('.').next().unwrap_or(name);
    if RESERVED.iter().any(|r| first.eq_ignore_ascii_case(r)) {
        return Err(format!("{first} is a name that Windows keeps for itself"));
    }
    Ok(name.to_owned())
}

/// The name of an item of `kind`: as [`check_name`], and one that, with the end the
/// kind gives it, is read back as the same item (a document called `a.schema`
/// would be a file `a.schema.json`, which is a schema called `a`).
pub fn check_item_name(raw: &str, kind: ItemKind) -> Result<String, String> {
    let name = check_name(raw)?;
    let file = format!("{name}{}", kind.suffix());
    match parse_file_name(&file) {
        Some(parsed) if parsed.name == name && parsed.kind == kind => Ok(name),
        _ => Err(
            "That name can't end in .schema, .patch or .sample, which mean something here"
                .to_owned(),
        ),
    }
}

/// `base`, or, if that is taken, `base (2)`, `base (3)`… the first that `taken`
/// says is free.
pub fn unique_name(taken: impl Fn(&str) -> bool, base: &str) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| base.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(file: &str) -> Option<(String, ItemKind)> {
        parse_file_name(file).map(|p| (p.name, p.kind))
    }

    #[test]
    fn the_end_of_a_file_name_says_what_it_is() {
        use Engine::*;
        for (file, name, kind) in [
            ("slow requests.jq", "slow requests", ItemKind::Query(Jq)),
            ("a.jsonpath", "a", ItemKind::Query(JsonPath)),
            ("a.jmespath", "a", ItemKind::Query(JmesPath)),
            ("a.jsonptr", "a", ItemKind::Query(JsonPointer)),
            ("orders-api.schema.json", "orders-api", ItemKind::Schema),
            ("bump replicas.patch.json", "bump replicas", ItemKind::Patch),
            ("invoices 2026.json", "invoices 2026", ItemKind::Document),
            ("log.ndjson", "log", ItemKind::Document),
            ("log.jsonl", "log", ItemKind::Document),
        ] {
            assert_eq!(kind_of(file), Some((name.to_owned(), kind)), "{file}");
        }
    }

    #[test]
    fn a_schema_is_not_a_document_that_happens_to_end_in_json() {
        assert_eq!(
            kind_of("a.schema.json").map(|(_, kind)| kind),
            Some(ItemKind::Schema)
        );
        assert_eq!(
            kind_of("a.patch.json").map(|(_, kind)| kind),
            Some(ItemKind::Patch)
        );
        // …and the name keeps its own dots.
        assert_eq!(
            kind_of("v1.2.schema.json"),
            Some(("v1.2".to_owned(), ItemKind::Schema))
        );
        assert_eq!(
            kind_of("v1.2.json"),
            Some(("v1.2".to_owned(), ItemKind::Document))
        );
    }

    #[test]
    fn the_end_is_read_whatever_its_case_and_kept_as_it_was_written() {
        let parsed = parse_file_name("Report.JQ").unwrap();
        assert_eq!(parsed.name, "Report");
        assert_eq!(parsed.kind, ItemKind::Query(Engine::Jq));
        assert_eq!(parsed.ext, ".JQ");
        assert_eq!(
            kind_of("Orders.Schema.JSON"),
            Some(("Orders".to_owned(), ItemKind::Schema))
        );
    }

    #[test]
    fn a_sample_a_hidden_file_and_anything_else_is_not_an_item() {
        for file in [
            "slow requests.sample.json",
            "A.SAMPLE.JSON",
            ".hidden.jq",
            ".jq",
            "jq",
            ".json",
            "notes.txt",
            "README.md",
            "a.jsonx",
            "a.jq.bak",
            "",
        ] {
            assert_eq!(kind_of(file), None, "{file:?}");
        }
    }

    #[test]
    fn a_file_name_that_is_only_an_end_has_no_name_to_give() {
        assert_eq!(kind_of("a.jq").map(|(n, _)| n), Some("a".to_owned()));
        assert_eq!(kind_of("jq"), None);
        assert_eq!(kind_of(".schema.json"), None);
        // Every end starts with a dot, which is why a name that is only an end is hidden
        // and `parse_file_name` never has an empty name to refuse.
        assert!(SAMPLE_SUFFIX.starts_with('.'));
        for (suffix, _) in SUFFIXES {
            assert!(suffix.starts_with('.'), "{suffix}");
            assert_eq!(parse_file_name(suffix), None, "{suffix}");
            assert_eq!(parse_file_name(&suffix.to_uppercase()), None, "{suffix}");
        }
    }

    #[test]
    fn what_a_kind_is_called_written_and_read_back_are_the_same() {
        use Engine::*;
        for kind in [
            ItemKind::Query(Jq),
            ItemKind::Query(JsonPath),
            ItemKind::Query(JmesPath),
            ItemKind::Query(JsonPointer),
            ItemKind::Schema,
            ItemKind::Patch,
            ItemKind::Document,
        ] {
            let file = format!("some name{}", kind.suffix());
            let parsed = parse_file_name(&file).unwrap();
            assert_eq!(parsed.kind, kind, "{file}");
            assert_eq!(parsed.name, "some name");
            assert_eq!(parsed.ext, kind.suffix());
        }
    }

    #[test]
    fn a_sample_is_named_after_the_items_it_goes_with() {
        assert_eq!(
            sample_file_name("slow requests"),
            "slow requests.sample.json"
        );
        assert!(parse_file_name(&sample_file_name("x")).is_none());
    }

    #[test]
    fn the_chips_take_what_they_say() {
        let jq = ItemKind::Query(Engine::Jq);
        assert!(Group::All.takes(jq) && Group::All.takes(ItemKind::Document));
        assert!(Group::Queries.takes(jq) && !Group::Queries.takes(ItemKind::Schema));
        assert!(Group::Schemas.takes(ItemKind::Schema) && !Group::Schemas.takes(jq));
        assert!(Group::Patches.takes(ItemKind::Patch) && !Group::Patches.takes(ItemKind::Schema));
        assert!(Group::Documents.takes(ItemKind::Document) && !Group::Documents.takes(jq));
        assert_eq!(Group::default(), Group::All);
        assert_eq!(
            Group::ALL.map(Group::label),
            ["All", "Queries", "Schemas", "Patches", "Docs"]
        );
    }

    #[test]
    fn a_name_is_trimmed_and_kept() {
        assert_eq!(
            check_name("  slow requests ").as_deref(),
            Ok("slow requests")
        );
        assert_eq!(
            check_name("5xx by endpoint (2026)").as_deref(),
            Ok("5xx by endpoint (2026)")
        );
        assert_eq!(
            check_name("café – naïve ✓").as_deref(),
            Ok("café – naïve ✓")
        );
        assert_eq!(check_name("v1.2").as_deref(), Ok("v1.2"));
    }

    #[test]
    fn a_name_that_a_file_system_would_not_take_is_refused_and_why_is_said() {
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            "a:b",
            "a*b",
            "a?b",
            "a\"b",
            "a<b",
            "a>b",
            "a|b",
            "line\nbreak",
            "tab\there",
            ".hidden",
            "..",
            ".",
            "ends.",
            "NUL",
            "con",
            "Com1.txt",
            "lpt9",
        ] {
            let said = check_name(bad);
            assert!(said.is_err(), "{bad:?} was taken");
            assert!(!said.unwrap_err().is_empty());
        }
        // The same name with something after it that is not a device's is fine.
        assert!(check_name("console").is_ok());
        assert!(check_name("nullable").is_ok());
    }

    #[test]
    fn a_name_is_no_longer_than_a_file_name_can_hold() {
        assert!(check_name(&"a".repeat(MAX_NAME_BYTES)).is_ok());
        assert!(check_name(&"a".repeat(MAX_NAME_BYTES + 1)).is_err());
        // Bytes, not letters: this is 60 two-byte letters, 120 bytes.
        assert!(check_name(&"é".repeat(60)).is_ok());
        assert!(check_name(&"é".repeat(61)).is_err());
    }

    #[test]
    fn a_name_that_would_be_read_back_as_another_item_is_refused() {
        let document = ItemKind::Document;
        for bad in ["a.schema", "a.patch", "a.sample", "A.SAMPLE", "a.SCHEMA"] {
            assert!(
                check_item_name(bad, document).is_err(),
                "a document called {bad} would be taken for another item"
            );
        }
        // A query's end is not one of those, so the same names are fine there.
        let jq = ItemKind::Query(Engine::Jq);
        assert!(check_item_name("a.schema", jq).is_ok());
        assert!(check_item_name("a.patch", jq).is_ok());
        // …and `a.sample` is a sample only with `.json` after it.
        assert!(check_item_name("a.sample", jq).is_ok());
        assert!(check_item_name("a.sample", ItemKind::Schema).is_ok());
        assert!(check_item_name("a.sample", document).is_err());
        assert_eq!(check_item_name(" ok ", document).as_deref(), Ok("ok"));
    }

    #[test]
    fn a_name_that_is_taken_gets_a_number() {
        let taken = |n: &str| ["a", "a (2)", "b"].contains(&n);
        assert_eq!(unique_name(taken, "a"), "a (3)");
        assert_eq!(unique_name(taken, "b"), "b (2)");
        assert_eq!(unique_name(taken, "c"), "c");
    }
}
