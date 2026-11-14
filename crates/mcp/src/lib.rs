//! The MCP server of the JsonQuery desktop app: what lets an AI assistant (Claude
//! Code, VS Code, Cursor, …) look at what the app has open and work with it, through
//! the Model Context Protocol.
//!
//! The app has it through its cargo feature `mcp`, a default one (the builds named
//! `_nomcp` are made without it), and it runs only while the user has switched it on.
//!
//! What is here knows nothing of the window: it speaks the protocol (`protocol.rs`)
//! over HTTP on the loopback address (`http.rs`), says which tools there are and
//! reads what they are asked (`catalogue.rs`), and has the parts of their answers
//! that need no app — the shape of a value, a page of it (`shape.rs`), a snippet of a
//! value too big for a page and the notice that says so, the same in every answer
//! (`snippet.rs`), the help on the query languages (`help.rs`). What a tool does in the
//! app is the app's to do, through the one method of [`Host`].
//!
//! For an assistant that starts its servers itself and speaks to them over stdin and
//! stdout, the program `jsonquery-mcp` (`bridge.rs`) takes their calls to the app.

pub mod bridge;
mod catalogue;
mod help;
mod http;
mod protocol;
pub mod shape;
pub mod snippet;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use serde_json::Value;

pub use catalogue::{
    language_key, parse, Call, Input, Language, SaveKind, Target, ToolPage, TOOL_NAMES,
};
pub use http::{url_of, Config, Server};
pub use protocol::{Activity, Status, PROTOCOL_VERSIONS};

/// The path the server answers on: `http://127.0.0.1:<port>/mcp`.
pub const PATH: &str = "/mcp";

/// The port the server listens on unless the user picks another.
pub const DEFAULT_PORT: u16 = 7457;

/// What the server is called, to the assistant and in the configurations it is
/// added with.
pub const SERVER_NAME: &str = "jsonquery";

/// What the user lets the assistant do. Each tool needs one of these; the user's
/// choice is the most a call may need.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    /// Look at what the app has open and at the workspace, and work on it with the
    /// tools of the Tools, without changing anything the user sees or keeps.
    #[default]
    Read,
    /// That, and change what the app shows (open a document, put a query in the query
    /// box and run it, show a node, fill a tool's page), read files, and change what is
    /// kept (save, move and delete in the workspace, save a file, load a web address).
    /// The user is asked in the app before anything is deleted or replaced, written
    /// outside the workspace, or fetched — unless they chose to always allow it.
    Edit,
}

impl Access {
    pub const ALL: [Access; 2] = [Access::Read, Access::Edit];

    /// What it is called where it is kept (`mcp.json`).
    pub fn key(self) -> &'static str {
        match self {
            Access::Read => "read",
            Access::Edit => "edit",
        }
    }

    pub fn from_key(key: &str) -> Option<Access> {
        Access::ALL.into_iter().find(|a| a.key() == key)
    }

    /// What the user is shown.
    pub fn label(self) -> &'static str {
        match self {
            Access::Read => "Read",
            Access::Edit => "Edit",
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            Access::Read => 0,
            Access::Edit => 1,
        }
    }

    fn from_u8(value: u8) -> Access {
        match value {
            1 => Access::Edit,
            _ => Access::Read,
        }
    }
}

/// The most an answer may hold, in bytes of JSON, as the user may set it: an answer
/// that would hold more comes a page at a time. A small one suits a model with a small
/// context (one run on the user's own computer); about four bytes make a token.
pub const PAGE_SIZES: [usize; 6] = [4_000, 8_000, 16_000, 24_000, 64_000, 200_000];

/// The size of a page unless the user sets another.
pub const DEFAULT_PAGE: usize = 24_000;

/// The least and the most a page may be set to (`mcp.json` may hold any size between).
pub const MIN_PAGE: usize = 2_000;
pub const MAX_PAGE: usize = 200_000;

/// What the app does for the server: everything a tool does that needs the app.
pub trait Host: Send + Sync + 'static {
    /// Do what `call` asks, and say what came of it: the value is what the assistant
    /// is given (as JSON text); an error is told to it as the tool's error, so that it
    /// can do something about it. `cancel` is set when the call is cancelled or the
    /// server stops.
    fn call(&self, call: Call, cancel: &Arc<AtomicBool>) -> Result<Value, String>;
}
