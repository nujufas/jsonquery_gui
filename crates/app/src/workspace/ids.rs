//! The identity of what the workspace keeps: a SHA-256 of what an item holds, and
//! an id of its own that it keeps for as long as it is stored, whatever becomes of
//! its name, its place or its text. Nothing here is shown to the person who uses the
//! app; it is held so that later work — keeping a workspace in step with another,
//! noticing that an item is the one that was shared, or that a copy is the same
//! as the original — has something to go by that does not change when the item is
//! renamed or moved.

use std::io::Read;
use std::path::Path;

use ring::digest::{Context, SHA256};
use ring::rand::{SecureRandom, SystemRandom};

/// The SHA-256 of `bytes`, as 64 lower-case hexadecimal digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut context = Context::new(&SHA256);
    context.update(bytes);
    hex(context.finish().as_ref())
}

/// The SHA-256 of the file at `path`, read a piece at a time (a sample may be a
/// document of many megabytes), and how many bytes it was.
pub fn sha256_file(path: &Path) -> std::io::Result<(String, u64)> {
    let mut file = std::fs::File::open(path)?;
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((hex(context.finish().as_ref()), total))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// Whether `text` is a SHA-256 as [`sha256_hex`] writes it.
pub fn is_sha256(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `text` can be an id: what [`IdSource`] makes, or anything else of
/// letters, digits and hyphens that another program put in the index.
pub fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Where the ids come from, so that a test can have ones it can predict.
pub trait IdSource: Send {
    /// An id that no other item has had, or will have.
    fn new_id(&mut self) -> String;
}

/// Random ids, as UUIDs of version 4: 122 random bits from the system's own source
/// of them, which makes two the same a matter of never.
pub struct RandomIds {
    random: SystemRandom,
    made: u64,
}

impl RandomIds {
    pub fn new() -> Self {
        Self {
            random: SystemRandom::new(),
            made: 0,
        }
    }
}

impl Default for RandomIds {
    fn default() -> Self {
        Self::new()
    }
}

impl IdSource for RandomIds {
    fn new_id(&mut self) -> String {
        self.made += 1;
        let mut bytes = [0_u8; 16];
        if self.random.fill(&mut bytes).is_err() {
            // The system has no randomness to give (it never has not): the time, the
            // process and a count are as unlike each other as ids need to be.
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let seed = format!("{nanos}-{}-{}", std::process::id(), self.made);
            let digest = sha256_hex(seed.as_bytes());
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&digest[i * 2..i * 2 + 2], 16).unwrap_or(0);
            }
        }
        uuid_v4(bytes)
    }
}

/// `bytes` written as a version 4 UUID: the digits that say which version and
/// which variant it is set, the rest as they are.
fn uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let digits = hex(&bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &digits[0..8],
        &digits[8..12],
        &digits[12..16],
        &digits[16..20],
        &digits[20..32]
    )
}

/// Ids a test can predict: `id-1`, `id-2`…
#[cfg(test)]
#[derive(Default)]
pub struct SequentialIds(pub u64);

#[cfg(test)]
impl IdSource for SequentialIds {
    fn new_id(&mut self) -> String {
        self.0 += 1;
        format!("id-{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_dir::ScratchDir;
    use std::collections::HashSet;

    #[test]
    fn the_hash_is_the_sha_256_that_the_standard_gives() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn a_hash_is_64_lower_case_digits_and_nothing_else_is() {
        assert!(is_sha256(&sha256_hex(b"x")));
        assert!(!is_sha256(""));
        assert!(!is_sha256(&sha256_hex(b"x").to_uppercase()));
        assert!(!is_sha256(&sha256_hex(b"x")[1..]));
        assert!(!is_sha256(&format!("{}a", sha256_hex(b"x"))));
        assert!(!is_sha256(&format!("{}g", &sha256_hex(b"x")[1..])));
    }

    #[test]
    fn a_file_is_hashed_as_the_bytes_in_it_are_however_many_pieces_it_is_read_in() {
        let dir = ScratchDir::new("ids", "file");
        // More than three pieces, and not a whole number of them.
        let bytes: Vec<u8> = (0..200_000_u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("big"), &bytes).unwrap();
        let (hash, length) = sha256_file(&dir.join("big")).unwrap();
        assert_eq!(hash, sha256_hex(&bytes));
        assert_eq!(length, 200_000);
        std::fs::write(dir.join("empty"), b"").unwrap();
        assert_eq!(
            sha256_file(&dir.join("empty")).unwrap(),
            (sha256_hex(b""), 0)
        );
        assert!(sha256_file(&dir.join("nothing")).is_err());
    }

    #[test]
    fn an_id_is_a_version_4_uuid_and_no_two_are_alike() {
        let mut ids = RandomIds::new();
        let mut seen = HashSet::new();
        for _ in 0..2000 {
            let id = ids.new_id();
            let bytes = id.as_bytes();
            assert_eq!(id.len(), 36, "{id}");
            for at in [8, 13, 18, 23] {
                assert_eq!(bytes[at], b'-', "{id}");
            }
            assert_eq!(bytes[14], b'4', "version: {id}");
            assert!(b"89ab".contains(&bytes[19]), "variant: {id}");
            assert!(is_id(&id), "{id}");
            assert!(seen.insert(id), "two ids were the same");
        }
    }

    #[test]
    fn the_version_and_the_variant_are_set_whatever_the_bytes_are() {
        assert_eq!(uuid_v4([0; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_v4([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
    }

    #[test]
    fn what_may_be_an_id_is_letters_digits_and_hyphens_in_a_reasonable_length() {
        assert!(is_id("6f1c0e52-9a4b-4c1e-8d52-3b7a1c9e0f21"));
        assert!(is_id("id-1"));
        assert!(!is_id(""));
        assert!(!is_id("has space"));
        assert!(!is_id("slash/ed"));
        assert!(!is_id(&"a".repeat(65)));
        assert!(is_id(&"a".repeat(64)));
    }

    #[test]
    fn the_ids_a_test_can_predict_count_up() {
        let mut ids = SequentialIds::default();
        assert_eq!((ids.new_id(), ids.new_id()), ("id-1".into(), "id-2".into()));
    }
}
