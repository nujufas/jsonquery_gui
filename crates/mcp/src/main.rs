//! `jsonquery-mcp`: the JsonQuery app's MCP server over stdin and stdout, for an
//! assistant that starts its servers itself (see `bridge.rs`).

use std::io::IsTerminal;

const VERSION: &str = concat!("jsonquery-mcp ", env!("CARGO_PKG_VERSION"));

const HELP: &str = "\
The MCP server of the JsonQuery app, for an AI assistant that starts its servers itself
(Claude Desktop, QuickAI's llama-server, ...): it speaks MCP on stdin and stdout, one
message to a line, and takes each tool call to the JsonQuery app that is running (the
build with MCP), whose server you start with the plug at the right of its status bar.

It finds the app's address and token in mcp.json, in .jsonquery of your home, or in the
folder JSONQUERY_HOME names. See docs/mcp.md of the app.

usage: jsonquery-mcp [--version | --help]";

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("{VERSION}");
            return;
        }
        Some("--help" | "-h") => {
            println!("{VERSION}\n\n{HELP}");
            return;
        }
        Some(other) => {
            eprintln!("jsonquery-mcp: unknown argument {other:?}\n\n{HELP}");
            std::process::exit(2);
        }
        None => {}
    }
    if std::io::stdin().is_terminal() {
        eprintln!(
            "{VERSION}: waiting for an assistant's messages on stdin (it is started by one; \
             see --help). Ctrl+D ends it."
        );
    }
    if let Err(e) = jsonquery_mcp::bridge::main() {
        eprintln!("jsonquery-mcp: {e}");
        std::process::exit(1);
    }
}
