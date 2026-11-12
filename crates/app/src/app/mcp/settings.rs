//! What is kept of the MCP server between runs: `mcp.json`, beside `settings.json` (in
//! `.jsonquery` of the home). It holds the token an assistant connects with, so it is
//! written for its owner alone to read, and only once the server has been started.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use jsonquery_mcp::{Access, DEFAULT_PAGE, DEFAULT_PORT, MAX_PAGE, MIN_PAGE};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};

/// The name of the file.
pub(super) const FILE: &str = "mcp.json";

/// How many random bytes a token is made of (it is written as twice as many hex digits).
const TOKEN_BYTES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct McpSettings {
    pub port: u16,
    pub token: String,
    /// Start the server when the app starts.
    pub start_with_app: bool,
    pub access: Access,
    /// Do what Edit allows without asking the user first.
    pub always_allow: bool,
    /// The most an answer may hold, in bytes (kept in KB: `answer_kb`).
    pub page: usize,
}

impl Default for McpSettings {
    /// The defaults without a token: [`load`] gives one.
    fn default() -> Self {
        McpSettings {
            port: DEFAULT_PORT,
            token: String::new(),
            start_with_app: false,
            access: Access::Read,
            always_allow: false,
            page: DEFAULT_PAGE,
        }
    }
}

/// A new token: random, from the system's own source.
pub(super) fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    if SystemRandom::new().fill(&mut bytes).is_err() {
        // The system has no randomness to give (it never has not): the time and the
        // process, hashed, are better than nothing.
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        let digest = ring::digest::digest(&ring::digest::SHA256, seed.as_bytes());
        bytes.copy_from_slice(&digest.as_ref()[..TOKEN_BYTES]);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_token(text: &str) -> bool {
    text.len() == TOKEN_BYTES * 2 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What is kept in `file`, or the defaults with a new token; and what could not be
/// used of it, said for the panel.
pub(super) fn load(file: Option<&Path>) -> (McpSettings, Option<String>) {
    let mut settings = McpSettings {
        token: new_token(),
        ..McpSettings::default()
    };
    let Some(file) = file else {
        return (settings, None);
    };
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (settings, None),
        Err(e) => return (settings, Some(format!("{FILE} could not be read: {e}"))),
    };
    let Ok(Value::Object(kept)) = serde_json::from_str::<Value>(&text) else {
        return (
            settings,
            Some(format!(
                "{FILE} is not what it should be: the defaults are used"
            )),
        );
    };
    let mut unusable = Vec::new();
    match kept
        .get("port")
        .map(|p| p.as_u64().and_then(|p| u16::try_from(p).ok()))
    {
        None => {}
        Some(Some(port)) if port > 0 => settings.port = port,
        Some(_) => unusable.push("port"),
    }
    match kept
        .get("token")
        .map(|t| t.as_str().filter(|t| is_token(t)))
    {
        None => {}
        Some(Some(token)) => settings.token = token.to_owned(),
        Some(None) => unusable.push("token"),
    }
    match kept.get("start_with_app").map(Value::as_bool) {
        None => {}
        Some(Some(start)) => settings.start_with_app = start,
        Some(None) => unusable.push("start_with_app"),
    }
    match kept
        .get("access")
        .map(|a| a.as_str().and_then(Access::from_key))
    {
        None => {}
        Some(Some(access)) => settings.access = access,
        Some(None) => unusable.push("access"),
    }
    match kept.get("always_allow").map(Value::as_bool) {
        None => {}
        Some(Some(always)) => settings.always_allow = always,
        Some(None) => unusable.push("always_allow"),
    }
    match kept.get("answer_kb").map(|kb| {
        kb.as_u64()
            .and_then(|kb| usize::try_from(kb).ok())
            .map(|kb| kb.saturating_mul(1_000))
            .filter(|page| (MIN_PAGE..=MAX_PAGE).contains(page))
    }) {
        None => {}
        Some(Some(page)) => settings.page = page,
        Some(None) => unusable.push("answer_kb"),
    }
    let note = (!unusable.is_empty()).then(|| {
        format!(
            "{FILE}: {} could not be used, the default stands in",
            unusable.join(", ")
        )
    });
    (settings, note)
}

/// Write `settings` to `file`, for its owner alone to read: to a file beside it first,
/// which then takes its place, so that the file is never half written.
pub(super) fn save(file: &Path, settings: &McpSettings) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let text = serde_json::to_string_pretty(&json!({
        "port": settings.port,
        "token": settings.token,
        "start_with_app": settings.start_with_app,
        "access": settings.access.key(),
        "always_allow": settings.always_allow,
        "answer_kb": settings.page / 1_000,
    }))?;
    let staged = file.with_extension("json.new");
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut out = options.open(&staged)?;
        out.write_all(text.as_bytes())?;
        out.write_all(b"\n")?;
        out.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&staged, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_dir::ScratchDir;

    #[test]
    fn with_nothing_kept_the_defaults_have_a_new_token() {
        let (a, note) = load(None);
        let (b, _) = load(None);
        assert!(note.is_none());
        assert!(is_token(&a.token) && is_token(&b.token));
        assert_ne!(a.token, b.token);
        assert_eq!(a.port, DEFAULT_PORT);
        assert_eq!(a.access, Access::Read);
        assert!(!a.start_with_app);
        assert!(!a.always_allow);
        assert_eq!(a.page, DEFAULT_PAGE);
    }

    #[test]
    fn what_is_saved_is_loaded_and_only_its_owner_can_read_it() {
        let dir = ScratchDir::new("mcp", "settings");
        let file = dir.join("sub").join(FILE);
        let settings = McpSettings {
            port: 9999,
            token: new_token(),
            start_with_app: true,
            access: Access::Edit,
            always_allow: true,
            page: 8_000,
        };
        save(&file, &settings).unwrap();
        assert_eq!(load(Some(&file)), (settings, None));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!file.with_extension("json.new").exists());
    }

    #[test]
    fn what_cannot_be_used_is_said_and_the_default_stands_in() {
        let dir = ScratchDir::new("mcp", "settings-bad");
        let file = dir.join(FILE);
        fs::write(
            &file,
            r#"{"port": 70000, "token": "short", "access": "full", "start_with_app": true,
                "always_allow": "yes", "answer_kb": 1}"#,
        )
        .unwrap();
        let (settings, note) = load(Some(&file));
        assert_eq!(settings.port, DEFAULT_PORT);
        assert!(is_token(&settings.token));
        assert_eq!(settings.access, Access::Read);
        assert!(settings.start_with_app);
        assert!(!settings.always_allow);
        assert_eq!(settings.page, DEFAULT_PAGE);
        let note = note.unwrap();
        assert!(
            note.contains("port, token, access, always_allow, answer_kb"),
            "{note}"
        );
        fs::write(&file, "not json").unwrap();
        assert!(load(Some(&file))
            .1
            .unwrap()
            .contains("not what it should be"));
    }
}
