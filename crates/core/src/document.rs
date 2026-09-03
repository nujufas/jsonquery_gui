use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use memmap2::Mmap;
use serde_json::Value;

/// Where a [`Document`]'s bytes came from.
pub enum DocumentSource {
    File(PathBuf),
    /// JSON typed or pasted directly into the app, rather than opened from disk.
    Pasted,
}

impl DocumentSource {
    pub fn label(&self) -> String {
        match self {
            DocumentSource::File(p) => p.display().to_string(),
            DocumentSource::Pasted => "(pasted JSON)".to_string(),
        }
    }
}

/// A loaded JSON document: the parsed value plus load/parse timing and
/// provenance, used to populate the status bar.
pub struct Document {
    pub source: DocumentSource,
    pub byte_len: u64,
    pub parse_time: Duration,
    pub root: Value,
    /// Number of top-level values found in the source (>1 means the file was
    /// NDJSON / concatenated JSON and got wrapped into a single array root).
    pub top_level_values: usize,
    /// Kept alive so future phases can resolve lazily straight from the
    /// mapped bytes instead of re-reading the file; unused for now beyond that.
    _mmap: Option<Mmap>,
}

/// Load and fully parse a JSON file in one step (Phase 1: in-memory path).
///
/// The file is memory-mapped rather than read into a `Vec<u8>` so opening is
/// instant regardless of file size; Phase 1 then parses the mapped bytes
/// directly into an owned `serde_json::Value` tree.
pub fn load(path: impl AsRef<Path>) -> Result<Document> {
    let path = path.as_ref();
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let byte_len = file
        .metadata()
        .with_context(|| format!("reading metadata for {}", path.display()))?
        .len();

    // memmap2 requires a non-empty, non-zero-length mapping.
    let mmap = if byte_len == 0 {
        None
    } else {
        Some(
            unsafe { Mmap::map(&file) }
                .with_context(|| format!("memory-mapping {}", path.display()))?,
        )
    };

    let start = Instant::now();
    let (root, top_level_values) = match &mmap {
        Some(m) => parse_bytes(&m[..])?,
        None => (Value::Array(Vec::new()), 0),
    };
    let parse_time = start.elapsed();

    Ok(Document {
        source: DocumentSource::File(path.to_path_buf()),
        byte_len,
        parse_time,
        root,
        top_level_values,
        _mmap: mmap,
    })
}

/// Parse JSON typed or pasted directly into the app (as opposed to opening a
/// file) into a [`Document`]. Uses the same one-or-more-top-level-values
/// parsing as the file path, so pasted NDJSON behaves the same way.
pub fn load_text(text: &str) -> Result<Document> {
    let start = Instant::now();
    let (root, top_level_values) = parse_bytes(text.as_bytes())?;
    let parse_time = start.elapsed();

    Ok(Document {
        source: DocumentSource::Pasted,
        byte_len: text.len() as u64,
        parse_time,
        root,
        top_level_values,
        _mmap: None,
    })
}

/// Parse JSON from raw bytes, treating the input as one *or more* top-level
/// values (see Architecture §3 "NDJSON and concatenated JSON"). A single
/// value is returned as-is; multiple values (NDJSON, or plain concatenated
/// JSON) are wrapped into one top-level array so the rest of the app only
/// ever deals with one root value.
fn parse_bytes(bytes: &[u8]) -> Result<(Value, usize)> {
    let mut de = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();

    let Some(first) = de.next() else {
        return Ok((Value::Array(Vec::new()), 0));
    };
    let first = first.context("parsing JSON")?;

    match de.next() {
        None => Ok((first, 1)),
        Some(second) => {
            let mut values = vec![first, second.context("parsing JSON")?];
            for v in de {
                values.push(v.context("parsing JSON")?);
            }
            let count = values.len();
            Ok((Value::Array(values), count))
        }
    }
}
