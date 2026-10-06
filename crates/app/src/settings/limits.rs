//! The limits on the size of files: what each is for, what it is by default, how
//! a size is typed and written, and how the limits go in and out of
//! `settings.json`.

use jsonquery_core::LoadLimits;
use jsonquery_query::lazy::{Limits as QueryLimits, Parallel};
use serde_json::{Map, Value};

/// Sizes, as the app means them everywhere (`human_bytes`): 1 MB is 1024 × 1024
/// bytes.
pub const KB: u64 = 1024;
pub const MB: u64 = 1024 * KB;
pub const GB: u64 = 1024 * MB;
pub const TB: u64 = 1024 * GB;

/// The least and the most a limit can be set to: below the first none of them
/// is of any use (a limit of a few bytes refuses everything), and the second is
/// as good as no limit (a petabyte) while sums of limits and sizes cannot
/// overflow.
pub const MIN_LIMIT: u64 = KB;
pub const MAX_LIMIT: u64 = 1024 * TB;

/// The most the Tools window takes of its documents, all together, by default.
/// They work in memory (a document is parsed, copied into jq's own values, and
/// the result built on top), so it is for documents that are not very large.
pub const DEFAULT_TOOLS_BYTES: u64 = 128 * MB;

/// The most that "Copy to Clipboard" takes, by default, from a value of a
/// document kept on disk, in bytes of the file: more is for Save…, which does
/// not hold it in memory.
pub const DEFAULT_COPY_BYTES: u64 = 64 * MB;

/// One of the limits on files that can be set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// From what size a file is kept on disk, memory-mapped and indexed, instead
    /// of being read and parsed.
    KeepOnDisk,
    /// The most that is read from a web address or from a pipe.
    Download,
    /// The most the Tools window takes.
    Tools,
    /// The most that "Copy to Clipboard" takes from a document kept on disk.
    Copy,
    /// A list or an object that a query on a document kept on disk may parse
    /// whole.
    QueryParse,
    /// The same for a string or any other value.
    QueryValue,
    /// One result of such a query.
    QueryResult,
    /// What such a query may gather into a list in memory.
    QueryGather,
    /// What the keys of a list that such a query sorts or groups may take.
    QueryKeys,
}

/// The sections of the Settings window the limits are in (those under
/// *Advanced*: see [`Limit::MAIN`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Opening,
    Tools,
    Copying,
    Queries,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::Opening, Group::Tools, Group::Copying, Group::Queries];

    pub fn title(self) -> &'static str {
        match self {
            Group::Opening => "Opening files",
            Group::Tools => "Tools window",
            Group::Copying => "Copy to Clipboard",
            Group::Queries => "Queries on a file kept on disk",
        }
    }

    /// What the section's limits have in common, if that is worth saying: the
    /// window shows it as a tooltip on the title.
    pub fn note(self) -> Option<&'static str> {
        match self {
            Group::Queries => Some(
                "A query on a file kept on disk reads the file as it goes, and only what it \
                 has to hold is in memory. These are how much of that a query may hold.",
            ),
            _ => None,
        }
    }
}

impl Limit {
    /// The one limit the Settings window shows at first: from what size a file
    /// is memory-mapped and kept on disk. It is the one that decides how the app
    /// treats the biggest files, which is what a person may want to change; the
    /// others are guards, and are behind *Advanced*.
    pub const MAIN: Limit = Limit::KeepOnDisk;

    pub const ALL: [Limit; 9] = [
        Limit::KeepOnDisk,
        Limit::Download,
        Limit::Tools,
        Limit::Copy,
        Limit::QueryParse,
        Limit::QueryValue,
        Limit::QueryResult,
        Limit::QueryGather,
        Limit::QueryKeys,
    ];

    /// The name the limit has in `settings.json`.
    pub fn key(self) -> &'static str {
        match self {
            Limit::KeepOnDisk => "keep_on_disk_from",
            Limit::Download => "download",
            Limit::Tools => "tools",
            Limit::Copy => "copy",
            Limit::QueryParse => "query_parse",
            Limit::QueryValue => "query_value",
            Limit::QueryResult => "query_result",
            Limit::QueryGather => "query_gather",
            Limit::QueryKeys => "query_keys",
        }
    }

    /// Whether the Settings window keeps it under *Advanced*.
    pub fn is_advanced(self) -> bool {
        self != Self::MAIN
    }

    pub fn group(self) -> Group {
        match self {
            Limit::KeepOnDisk | Limit::Download => Group::Opening,
            Limit::Tools => Group::Tools,
            Limit::Copy => Group::Copying,
            Limit::QueryParse
            | Limit::QueryValue
            | Limit::QueryResult
            | Limit::QueryGather
            | Limit::QueryKeys => Group::Queries,
        }
    }

    /// What the Settings window calls it.
    pub fn label(self) -> &'static str {
        match self {
            Limit::KeepOnDisk => "Keep a file on disk from",
            Limit::Download => "Largest download or pipe",
            Limit::Tools => "Largest input",
            Limit::Copy => "Largest copy",
            Limit::QueryParse => "List or object parsed whole",
            Limit::QueryValue => "String or other value",
            Limit::QueryResult => "One result",
            Limit::QueryGather => "Gathered into a list",
            Limit::QueryKeys => "Keys of a sort or group",
        }
    }

    /// What it does, for the Settings window to say in a tooltip.
    pub fn help(self) -> &'static str {
        match self {
            Limit::KeepOnDisk => {
                "A file this big or bigger is not loaded into memory: it is memory-mapped, \
                 indexed once and read from the disk as you look at it. A smaller one is \
                 loaded and parsed, which is quicker but takes about twelve times its size in \
                 memory. Applies to the next file you open."
            }
            Limit::Download => {
                "The most that is read from a web address or from a pipe (such as standard \
                 input), which have no size to go by."
            }
            Limit::Tools => {
                "The most that Merge, Diff, Patch and Validate take, all of a job's documents \
                 together: they work in memory, with room for several copies of what they are \
                 given. Format writes a file kept on disk out as it goes, whatever its size. A \
                 document the Tools window can use has to be loaded, not kept on disk (see \
                 \"Keep a file on disk from\")."
            }
            Limit::Copy => {
                "The most that Copy to Clipboard takes from a value of a file kept on disk, in \
                 bytes of the file. For more, use Save…, which does not hold it in memory."
            }
            Limit::QueryParse => {
                "When a step of a query can't be done by reading the file, the list or object \
                 it needs is parsed and handed to jq: this is the biggest that is. (In memory \
                 it takes some twenty times its size.)"
            }
            Limit::QueryValue => {
                "The same, for a string or any other value that is not a list or object."
            }
            Limit::QueryResult => {
                "The biggest single result a query may make: a bigger one is not made into a \
                 value, which the results panel would have to hold."
            }
            Limit::QueryGather => {
                "How much [ … ], map(…) and the like may gather in memory, as an estimate of \
                 what it takes."
            }
            Limit::QueryKeys => {
                "How much the keys of a list that is sorted, grouped or searched for its \
                 smallest may take, as an estimate of the memory: every element has one, and \
                 where it is in the file."
            }
        }
    }
}

/// The limits on files. [`Default`] is what the app has always used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileLimits {
    keep_on_disk: u64,
    download: u64,
    tools: u64,
    copy: u64,
    query_parse: u64,
    query_value: u64,
    query_result: u64,
    query_gather: u64,
    query_keys: u64,
}

impl Default for FileLimits {
    fn default() -> Self {
        // Each from where it is defined, so that there is one place a default
        // is changed in.
        let load = LoadLimits::default();
        let query = QueryLimits::default();
        Self {
            keep_on_disk: load.lazy_threshold,
            download: load.stream_bytes,
            tools: DEFAULT_TOOLS_BYTES,
            copy: DEFAULT_COPY_BYTES,
            query_parse: query.materialize_bytes as u64,
            query_value: query.scalar_bytes as u64,
            query_result: query.result_bytes as u64,
            query_gather: query.collect_bytes as u64,
            query_keys: query.key_bytes as u64,
        }
    }
}

impl FileLimits {
    pub fn get(&self, limit: Limit) -> u64 {
        match limit {
            Limit::KeepOnDisk => self.keep_on_disk,
            Limit::Download => self.download,
            Limit::Tools => self.tools,
            Limit::Copy => self.copy,
            Limit::QueryParse => self.query_parse,
            Limit::QueryValue => self.query_value,
            Limit::QueryResult => self.query_result,
            Limit::QueryGather => self.query_gather,
            Limit::QueryKeys => self.query_keys,
        }
    }

    pub fn set(&mut self, limit: Limit, bytes: u64) {
        let slot = match limit {
            Limit::KeepOnDisk => &mut self.keep_on_disk,
            Limit::Download => &mut self.download,
            Limit::Tools => &mut self.tools,
            Limit::Copy => &mut self.copy,
            Limit::QueryParse => &mut self.query_parse,
            Limit::QueryValue => &mut self.query_value,
            Limit::QueryResult => &mut self.query_result,
            Limit::QueryGather => &mut self.query_gather,
            Limit::QueryKeys => &mut self.query_keys,
        };
        *slot = bytes;
    }

    /// Whether `limit` is what the app has always used.
    pub fn is_default(&self, limit: Limit) -> bool {
        self.get(limit) == Self::default().get(limit)
    }

    /// Whether every limit is.
    pub fn all_default(&self) -> bool {
        *self == Self::default()
    }

    /// What decides how a file is brought in when the user opens it.
    pub fn load(&self) -> LoadLimits {
        LoadLimits {
            lazy_threshold: self.keep_on_disk,
            stream_bytes: self.download,
        }
    }

    /// What decides how a file is brought in for a tool, which has to have it
    /// parsed: whatever it is allowed to take, which is anything up to
    /// [`Self::tools`], is parsed, and not kept on disk where that is a lower
    /// size than the one the tools take.
    pub fn load_for_tools(&self) -> LoadLimits {
        LoadLimits {
            lazy_threshold: self.tools.saturating_add(1),
            stream_bytes: self.download,
        }
    }

    /// The most a download or a pipe may hand over.
    pub fn download(&self) -> u64 {
        self.download
    }

    /// From what size a file is kept on disk.
    pub fn keep_on_disk(&self) -> u64 {
        self.keep_on_disk
    }

    /// The most the Tools window takes.
    pub fn tools(&self) -> u64 {
        self.tools
    }

    /// The most that Copy to Clipboard takes from a document kept on disk.
    pub fn copy(&self) -> usize {
        usize::try_from(self.copy).unwrap_or(usize::MAX)
    }

    /// What a query on a document kept on disk holds itself to.
    pub fn query(&self) -> QueryLimits {
        let size = |bytes: u64| usize::try_from(bytes).unwrap_or(usize::MAX);
        QueryLimits {
            materialize_bytes: size(self.query_parse),
            scalar_bytes: size(self.query_value),
            result_bytes: size(self.query_result),
            collect_bytes: size(self.query_gather),
            key_bytes: size(self.query_keys),
            parallel: Parallel::default(),
        }
    }

    /// What `settings.json` holds for these: only the limits that are not what
    /// the app has always used, each as a size with its unit.
    pub fn to_json(self) -> Map<String, Value> {
        let mut limits = Map::new();
        for limit in Limit::ALL {
            if !self.is_default(limit) {
                limits.insert(
                    limit.key().to_owned(),
                    Value::String(size_text(self.get(limit))),
                );
            }
        }
        limits
    }

    /// The limits that `value`, the `"limits"` of a `settings.json`, stands for.
    /// A limit that is not there is the default; one that is no size, or is below
    /// [`MIN_LIMIT`] or above [`MAX_LIMIT`], is left as the default and said to be
    /// wrong in `notes`; what is not known (from a newer version) is left alone.
    /// A number is a size in bytes; text is a size with a unit ([`parse_size`]).
    pub fn read(value: Option<&Value>, notes: &mut Vec<String>) -> Self {
        let mut limits = Self::default();
        let Some(found) = value else {
            return limits;
        };
        let Some(given) = found.as_object() else {
            notes.push("\"limits\" is not an object: the defaults are used".to_owned());
            return limits;
        };
        for limit in Limit::ALL {
            let Some(given) = given.get(limit.key()) else {
                continue;
            };
            let bytes = match given {
                Value::Number(number) => number
                    .as_u64()
                    .ok_or_else(|| "not a whole number of bytes".to_owned()),
                Value::String(text) => parse_size(text),
                _ => Err("not a size".to_owned()),
            }
            .and_then(check_range);
            match bytes {
                Ok(bytes) => limits.set(limit, bytes),
                Err(why) => notes.push(format!(
                    "{}: {why}, so the default ({}) is used",
                    limit.key(),
                    size_text(limits.get(limit))
                )),
            }
        }
        limits
    }
}

/// `bytes` if a limit can be that, and why not if it can't.
pub fn check_range(bytes: u64) -> Result<u64, String> {
    if bytes < MIN_LIMIT {
        Err(format!("too small: at least {}", size_text(MIN_LIMIT)))
    } else if bytes > MAX_LIMIT {
        Err(format!("too big: at most {}", size_text(MAX_LIMIT)))
    } else {
        Ok(bytes)
    }
}

/// A size as it is typed: a number, with a unit after it (`B`, `KB`, `MB`, `GB`
/// or `TB`, in any case; `KiB` and the rest, and just `K`, `M`, `G` and `T`, are
/// the same; a space is optional). A number alone is in MB. The number may have a
/// fraction (`1.5 GB`). A unit is 1024 times the one before it, as it is
/// everywhere in the app.
pub fn parse_size(text: &str) -> Result<u64, String> {
    const HOW: &str = "use a number and a unit, such as 256 MB or 4 GB";
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("nothing typed: {HOW}"));
    }
    let digits_end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits_end);
    let number: f64 = number
        .parse()
        .map_err(|_| format!("not a size: {HOW}"))
        .and_then(|n: f64| {
            if n.is_finite() {
                Ok(n)
            } else {
                Err(format!("not a size: {HOW}"))
            }
        })?;
    let unit = unit.trim().to_ascii_lowercase();
    let times = match unit.as_str() {
        "b" => 1,
        "k" | "kb" | "kib" => KB,
        "" | "m" | "mb" | "mib" => MB,
        "g" | "gb" | "gib" => GB,
        "t" | "tb" | "tib" => TB,
        _ => return Err(format!("unknown unit {:?}: {HOW}", unit)),
    };
    // A number too big for a u64 saturates, and is then too big for a limit
    // ([`check_range`]), which is for the caller to say.
    Ok((number * times as f64).round() as u64)
}

/// A size as it is written: in the biggest unit it is a whole number of, as
/// `256 MB`, `4 GB`; a size that is no whole number of kilobytes is in bytes.
pub fn size_text(bytes: u64) -> String {
    for (unit, name) in [(TB, "TB"), (GB, "GB"), (MB, "MB"), (KB, "KB")] {
        if bytes >= unit && bytes.is_multiple_of(unit) {
            return format!("{} {name}", bytes / unit);
        }
    }
    format!("{bytes} B")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_defaults_are_the_limits_the_app_has_always_had() {
        let limits = FileLimits::default();
        assert_eq!(limits.get(Limit::KeepOnDisk), 256 * MB);
        assert_eq!(limits.get(Limit::Download), 4 * GB);
        assert_eq!(limits.get(Limit::Tools), 128 * MB);
        assert_eq!(limits.get(Limit::Copy), 64 * MB);
        assert_eq!(limits.get(Limit::QueryParse), 4 * MB);
        assert_eq!(limits.get(Limit::QueryValue), 64 * MB);
        assert_eq!(limits.get(Limit::QueryResult), 16 * MB);
        assert_eq!(limits.get(Limit::QueryGather), GB);
        assert_eq!(limits.get(Limit::QueryKeys), GB);
        assert!(limits.all_default());
        assert!(limits.to_json().is_empty());
    }

    #[test]
    fn what_is_handed_to_the_rest_of_the_app_is_what_was_set() {
        let defaults = FileLimits::default();
        assert_eq!(defaults.load(), LoadLimits::default());
        assert_eq!(defaults.query().materialize_bytes, 4 * 1024 * 1024);
        assert_eq!(defaults.copy(), 64 * 1024 * 1024);

        let mut limits = FileLimits::default();
        for (n, limit) in Limit::ALL.into_iter().enumerate() {
            limits.set(limit, (n as u64 + 1) * 10 * MB);
        }
        assert_eq!(limits.load().lazy_threshold, 10 * MB);
        assert_eq!(limits.load().stream_bytes, 20 * MB);
        assert_eq!(limits.tools(), 30 * MB);
        assert_eq!(limits.copy(), (40 * MB) as usize);
        let query = limits.query();
        assert_eq!(query.materialize_bytes, (50 * MB) as usize);
        assert_eq!(query.scalar_bytes, (60 * MB) as usize);
        assert_eq!(query.result_bytes, (70 * MB) as usize);
        assert_eq!(query.collect_bytes, (80 * MB) as usize);
        assert_eq!(query.key_bytes, (90 * MB) as usize);
    }

    #[test]
    fn a_tool_loads_what_it_may_take_whole_and_not_a_byte_more() {
        let mut limits = FileLimits::default();
        limits.set(Limit::Tools, 100 * MB);
        limits.set(Limit::KeepOnDisk, 50 * MB);
        // Parsed up to the size the tools take, though the file is kept on disk
        // from a lower one, and kept on disk from the first size past it.
        assert_eq!(limits.load_for_tools().lazy_threshold, 100 * MB + 1);
        assert_eq!(limits.load().lazy_threshold, 50 * MB);
        limits.set(Limit::Tools, u64::MAX);
        assert_eq!(limits.load_for_tools().lazy_threshold, u64::MAX);
    }

    #[test]
    fn every_limit_has_a_name_of_its_own_in_the_file_and_a_place_in_the_window() {
        let mut keys: Vec<_> = Limit::ALL.iter().map(|l| l.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Limit::ALL.len());
        let mut labels: Vec<_> = Limit::ALL.iter().map(|l| l.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), Limit::ALL.len());
        for limit in Limit::ALL {
            assert!(!limit.help().is_empty(), "{limit:?}");
            assert!(Group::ALL.contains(&limit.group()), "{limit:?}");
        }
        for group in Group::ALL {
            assert!(
                Limit::ALL.iter().any(|l| l.group() == group),
                "{group:?} has no limit in it"
            );
        }
        // Setting one changes that one only.
        for limit in Limit::ALL {
            let mut limits = FileLimits::default();
            limits.set(limit, 7 * MB);
            for other in Limit::ALL {
                assert_eq!(limits.is_default(other), other != limit, "{limit:?}");
            }
        }
    }

    #[test]
    fn the_window_shows_one_limit_and_the_others_are_advanced() {
        let shown: Vec<_> = Limit::ALL
            .into_iter()
            .filter(|limit| !limit.is_advanced())
            .collect();
        assert_eq!(
            shown,
            [Limit::KeepOnDisk],
            "the size from which a file is mapped"
        );
        assert_eq!(Limit::MAIN, Limit::KeepOnDisk);
        // A group is titled in the Advanced part only if it has a limit there.
        for group in Group::ALL {
            assert!(
                Limit::ALL
                    .iter()
                    .any(|l| l.is_advanced() && l.group() == group),
                "{group:?} has nothing under Advanced"
            );
        }
    }

    #[test]
    fn a_size_is_a_number_and_a_unit() {
        for (text, bytes) in [
            ("256 MB", 256 * MB),
            ("256MB", 256 * MB),
            ("256 mb", 256 * MB),
            ("256 MiB", 256 * MB),
            ("256 M", 256 * MB),
            ("4 GB", 4 * GB),
            ("4gib", 4 * GB),
            ("  4 g  ", 4 * GB),
            ("1 TB", TB),
            ("512 KB", 512 * KB),
            ("512 kib", 512 * KB),
            ("1.5 GB", GB + GB / 2),
            ("0.5 MB", MB / 2),
            ("1500 B", 1500),
            ("1500 b", 1500),
            // No unit is in MB, which is what every limit is about.
            ("256", 256 * MB),
            (".5", MB / 2),
        ] {
            assert_eq!(parse_size(text), Ok(bytes), "{text:?}");
        }
    }

    #[test]
    fn what_is_not_a_size_says_how_to_write_one() {
        for text in [
            "",
            "   ",
            "MB",
            "abc",
            "12 parsecs",
            "1.2.3 GB",
            "-5 MB",
            "1,5 GB",
            ".",
        ] {
            let err = parse_size(text).expect_err(text);
            assert!(err.contains("such as 256 MB"), "{text:?}: {err}");
        }
        let err = parse_size("12 parsecs").unwrap_err();
        assert!(err.contains("unknown unit"), "{err}");
    }

    #[test]
    fn a_size_too_big_for_a_number_is_too_big_for_a_limit_and_not_a_panic() {
        for text in ["99999999999999999999999 TB", "1000000 TB", "1025 TB"] {
            let bytes = parse_size(text).expect(text);
            let err = check_range(bytes).expect_err(text);
            assert!(err.starts_with("too big"), "{text:?}: {err}");
        }
    }

    #[test]
    fn a_limit_is_between_a_kilobyte_and_a_petabyte() {
        assert_eq!(check_range(KB), Ok(KB));
        assert_eq!(check_range(MAX_LIMIT), Ok(MAX_LIMIT));
        let small = check_range(KB - 1).unwrap_err();
        assert!(
            small.contains("too small") && small.contains("1 KB"),
            "{small}"
        );
        let big = check_range(MAX_LIMIT + 1).unwrap_err();
        assert!(big.contains("too big") && big.contains("1024 TB"), "{big}");
        assert!(check_range(0).is_err());
    }

    #[test]
    fn a_size_is_written_in_the_biggest_unit_that_keeps_it_whole() {
        for (bytes, text) in [
            (256 * MB, "256 MB"),
            (4 * GB, "4 GB"),
            (1536 * MB, "1536 MB"),
            (TB, "1 TB"),
            (3 * KB, "3 KB"),
            (1500, "1500 B"),
            (KB + 1, "1025 B"),
            (0, "0 B"),
        ] {
            assert_eq!(size_text(bytes), text);
        }
    }

    #[test]
    fn a_size_that_is_written_is_read_back_the_same() {
        for bytes in [
            KB,
            1500,
            MB,
            MB + 1,
            256 * MB,
            1536 * MB,
            4 * GB,
            GB + KB,
            MAX_LIMIT,
            u32::MAX as u64,
            1 << 40,
        ] {
            assert_eq!(parse_size(&size_text(bytes)), Ok(bytes), "{bytes}");
        }
    }

    #[test]
    fn only_what_was_changed_is_written() {
        let mut limits = FileLimits::default();
        limits.set(Limit::KeepOnDisk, 512 * MB);
        limits.set(Limit::QueryGather, 4 * GB);
        assert_eq!(
            Value::Object(limits.to_json()),
            json!({"keep_on_disk_from": "512 MB", "query_gather": "4 GB"})
        );
        // Changed and changed back is not changed.
        limits.set(Limit::KeepOnDisk, 256 * MB);
        assert_eq!(
            Value::Object(limits.to_json()),
            json!({"query_gather": "4 GB"})
        );
    }

    #[test]
    fn what_is_written_is_what_is_read() {
        let mut limits = FileLimits::default();
        for (n, limit) in Limit::ALL.into_iter().enumerate() {
            limits.set(limit, (n as u64 + 1) * MB + n as u64);
        }
        let mut notes = Vec::new();
        let read = FileLimits::read(Some(&Value::Object(limits.to_json())), &mut notes);
        assert_eq!(read, limits);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn a_number_is_bytes_and_text_is_a_size_with_a_unit() {
        let mut notes = Vec::new();
        let limits = FileLimits::read(
            Some(&json!({"keep_on_disk_from": 1048576, "tools": "64 MB", "copy": "2 GB"})),
            &mut notes,
        );
        assert_eq!(limits.get(Limit::KeepOnDisk), MB);
        assert_eq!(limits.get(Limit::Tools), 64 * MB);
        assert_eq!(limits.get(Limit::Copy), 2 * GB);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn what_cannot_be_used_is_left_as_the_default_and_said() {
        let mut notes = Vec::new();
        let limits = FileLimits::read(
            Some(&json!({
                "keep_on_disk_from": "banana",
                "download": 12,
                "tools": true,
                "copy": -4,
                "query_parse": "",
                "query_value": 1.5,
                "query_result": "5000 TB",
                "query_gather": "2 GB",
                "from_a_newer_version": "1 GB"
            })),
            &mut notes,
        );
        let defaults = FileLimits::default();
        for limit in [
            Limit::KeepOnDisk,
            Limit::Download,
            Limit::Tools,
            Limit::Copy,
            Limit::QueryParse,
            Limit::QueryValue,
            Limit::QueryResult,
        ] {
            assert_eq!(limits.get(limit), defaults.get(limit), "{limit:?}");
        }
        assert_eq!(
            limits.get(Limit::QueryGather),
            2 * GB,
            "the one that is fine"
        );
        assert_eq!(notes.len(), 7, "{notes:#?}");
        assert!(
            notes.iter().any(|n| n.starts_with("keep_on_disk_from: ")
                && n.contains("so the default (256 MB) is used")),
            "{notes:#?}"
        );
        assert!(
            notes.iter().any(|n| n.starts_with("download: too small")),
            "{notes:#?}"
        );
        assert!(
            notes.iter().any(|n| n.starts_with("query_result: too big")),
            "{notes:#?}"
        );
        assert!(notes.iter().all(|n| !n.contains("from_a_newer_version")));
    }

    #[test]
    fn limits_that_are_not_an_object_give_the_defaults() {
        for value in [json!(null), json!([]), json!("limits"), json!(7)] {
            let mut notes = Vec::new();
            let limits = FileLimits::read(Some(&value), &mut notes);
            assert!(limits.all_default(), "{value}");
        }
        let mut notes = Vec::new();
        assert!(FileLimits::read(None, &mut notes).all_default());
        assert!(notes.is_empty(), "nothing said is nothing wrong");
    }
}
