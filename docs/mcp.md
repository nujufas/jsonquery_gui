# MCP server

The build of jsonquery gui with **mcp** in its name has a
[Model Context Protocol](https://modelcontextprotocol.io) server: an AI assistant (Claude Code,
Claude Desktop, VS Code, Cursor, QuickAI and others) can look at what the app has open and work with
it — read and query the document, put a query in the query box and run it, open your saved
queries — while you watch it happen in the window.

It can also work out of sight — query, read and search any JSON file, use the six Tools (Merge,
Format, Diff, Patch, Validate, Create Schema) — without anything changing in the window, and, if you
allow it, save in the workspace and to files, asking you first, in the app, before anything is
deleted, replaced, written outside the workspace or fetched from the web.

It is a separate download, attached to each
[GitHub Release](https://github.com/nujufas/jsonquery_gui/releases):
`jsonquery_gui-mcp-<version>-linux-x86_64.tar.gz`, `…-linux-aarch64.tar.gz` and
`…-windows-x86_64.zip`. The packages of the stores and package managers (Snap, Homebrew, Scoop,
winget, Chocolatey, the Microsoft Store, the AUR) and the AppImages do not have it. Nothing of it
runs until you start it.

## Starting it

Click the 🔌 at the right of the status bar, then **Start**. The server listens on
`http://127.0.0.1:7457/mcp`, on this computer only, for as long as the app runs; **Stop** stops it
and disconnects every assistant. **Start it with JsonQuery** starts it whenever the app starts.
The 🔌 is green while it runs, and a spinner turns beside it while an assistant's call is being
answered. The port can be changed in the panel while the server is stopped.

## Adding it to an assistant

The panel's **Add to** buttons copy what adds the server to one:

| Button | What it copies | Where it goes |
|---|---|---|
| Claude Code | `claude mcp add --transport http jsonquery http://127.0.0.1:7457/mcp --header "Authorization: Bearer …"` | Run it in a terminal |
| Claude Desktop | An `mcpServers` entry that starts `jsonquery-mcp` | `claude_desktop_config.json` |
| VS Code | A `servers` entry of type `http` | VS Code's `mcp.json` (of the user or of a folder) |
| QuickAI | `--mcp-servers-json '{"mcpServers":{"jsonquery":{"command":"…/jsonquery-mcp"}}}'` | QuickAI's **Extra command-line arguments** (llama.cpp): llama-server then offers the tools to the model |
| JSON | An `mcpServers` entry of type `http` | A project's `.mcp.json` (Claude Code), Cursor's `mcp.json`, and other assistants |

What an assistant connects with over HTTP carries the token: keep it to yourself.

Claude Desktop and QuickAI (through llama.cpp's `llama-server`) start their servers themselves and
talk to them over stdin and stdout. For them the download has **jsonquery-mcp** beside the app: it
answers the handshake and the list of tools itself, and takes each call to the app that is running,
with the address and token it reads from `mcp.json` (so what they are given holds no token). The
tools are listed even while the app is closed; a call made then says what to do. Their buttons are
greyed out when `jsonquery-mcp` is not beside the app.

## What the assistant may do

The panel's **Allow** says how far the assistant may go:

- **Read** (the default): look at the open document, the query in the query box and its results,
  and the workspace, and work on them out of sight — run queries, read, search, describe, use the
  six Tools. Nothing you see changes, and nothing is written.
- **Edit**: also open a document (a file, or JSON text), put a query in the query box and run it,
  show a value in the trees, open an item of the workspace, fill a page of the Tools and run it, read
  JSON files that are not open, save items in the workspace, make folders, move, rename and delete
  there, save a file, and load a document from a web address. **You are asked first**, in a dialog
  over the main window, before anything is deleted, an item is replaced, a file is written outside
  the workspace, or a web address is fetched: **Allow**, **Always allow** or **Don't allow**
  (Escape). A question that is not answered within two minutes is refused. Saving a new item,
  making a folder and moving one (nothing is replaced by a move) are not asked about.
- **Always allow** (beside Edit in the panel, or the question's own button): what Edit allows is
  done without asking. Untick it in the panel to be asked again.

A call that needs more than you allow is refused, and the assistant is told to ask you. With Read,
the server never writes or deletes a file, never changes the workspace or the window, and never
loads a web address.

## The tools

| Tool | What it does | Needs |
|---|---|---|
| `get_state` | What the app has open: the document, the query and how it went, what is in front, the workspace | Read |
| `document_info` | Facts about a document without its data: where it is from, its size, when it changed, JSON or NDJSON, how many values of each kind and how deep, its longest arrays, widest objects and biggest members, and how many pages reading it whole takes | Read |
| `describe` | The shape of a value without its data: keys and their kinds, lengths, ranges, example strings, from a sample of long arrays | Read |
| `read_value` | The value at a JSON Pointer, a page at a time | Read |
| `search` | The values and keys that contain a text (or match a regular expression), with their pointers | Read |
| `run_query` | Run a query (jq, JSONPath, JMESPath, JSON Pointer) and get its results a page at a time, without changing what you see | Read |
| `get_results` | A page of the results you see | Read |
| `query_help` | The app's lessons on each language, and where its jq differs from jq | Read |
| `workspace_list` | Your saved queries, schemas, patches, documents and web addresses | Read |
| `workspace_read` | What an item holds, and its sample | Read |
| `format_json` | Format a value as the Format tool does (indent, sorted keys, ASCII) | Read |
| `diff_documents` | Compare two values as the Diff tool does: the differences with their pointers, and the JSON Patch | Read |
| `apply_patch` | Apply a JSON Patch or a JSON Merge Patch and give the result | Read |
| `merge_documents` | Merge values as the Merge tool does (append, unique, deep, bundle, or a jq filter) | Read |
| `validate_json` | Check a value against a JSON Schema: valid or not, and each problem with its pointer | Read |
| `create_schema` | Make a JSON Schema from a value, as Create Schema does | Read |
| `open_document` | Open a file or JSON text as the Source | Edit |
| `set_query` | Put a query in the query box and run it | Edit |
| `reveal` | Open a tree down to a value and show it | Edit |
| `workspace_open` | Open an item as a click on it does | Edit |
| `show_in_tool` | Fill a page of the Tools with values and run it, for you to see | Edit |
| `workspace_save` | Save a query, schema, patch, document or web address in the workspace (replacing one: asked) | Edit |
| `workspace_new_folder` | Make a folder in the workspace | Edit |
| `workspace_move` | Move or rename an item or a folder (nothing is replaced) | Edit |
| `workspace_delete` | Delete an item or a folder (asked) | Edit |
| `save_file` | Save JSON to a file, indented as asked (asked) | Edit |
| `open_url` | Load a document from a web address as the Source (asked) | Edit |

## Out of sight

The tools that only read change nothing you see: only the panel's list of calls shows them. They
take any document as their `input`:

- `"document"` (the one open, the default), `"results"` (those you see);
- `{"file": "/full/path.json"}` — read for the assistant alone (it needs Edit): the window keeps its
  own document;
- `{"workspace": "orders/late.json"}` — a JSON item of the workspace;
- `{"json": …}` or `{"text": "…"}` — JSON given in the call;
- `"last"` — what the assistant's last `run_query` (all of its results, up to 16 MB of them),
  `apply_patch`, `merge_documents`, `create_schema` or `diff_documents` (with `as_patch`) made, kept
  by the app while the server runs: so that a big result is read a page at a time, queried, checked
  or saved (`save_file`, `workspace_save`) without passing through the conversation.

The six Tools work to the same size limits as the Tools window (⚙ Settings); `show_in_tool` is the
one that shows you a page of them (a box of a page takes up to 1 MB).

## Large documents, and models with a small context

Every answer is at most a page, of the size the panel's **Answers** says: 24 KB by default (about
6,000 tokens of a model), 4 KB to 200 KB. A model that runs on your own computer has a small
context: a smaller page keeps it from filling. Where there is more, the answer says `next_offset`,
and the assistant asks again with `offset`; `limit` caps how many items a page has (20 by default).
A part too big for the page is written as
`{"$omitted": "array with 120000 items", "pointer": "/users"}`, to be read with that pointer.
`describe` goes fewer levels down if more would not fit, and says so; the help comes in pages too.

`document_info` and `describe` are how an assistant learns a large document before it reads it: the
first counts what is in it (for up to 10 seconds; it says if it counted only a part) and says where
the bulk of it is and how many pages reading it whole would take; the second gives its shape from a
sample, and looks at no more than 50,000 values however big the document is. Both work the same on a
document that is kept on disk (256 MB and over by default).

What a model reads of the tools before it does anything — their descriptions and what they take — is
about 19 KB.

The assistant's own queries (`run_query`, `search`) run on a worker of their own, over the document
the app already holds, so they never wait behind yours, nor yours behind them; one is stopped after
`timeout_ms` (30 seconds by default). A call that changes what the app shows works while the window
is minimized or covered.

## Safety

- The server listens on `127.0.0.1` only. It answers only requests that name it as their host, that
  come from no web page, and that carry its token (`Authorization: Bearer`): what keeps other
  programs on the computer, and web pages, from using it.
- The token, the port and the panel's choices are kept in `mcp.json`, beside `settings.json` (in
  `.jsonquery` of your home, or the folder `JSONQUERY_HOME` names), which only you can read. **New token** makes another:
  assistants set up with the old one must be added again (`jsonquery-mcp` follows it by itself).
- A JSON document can hold text written to steer an AI ("prompt injection"). With **Read** the
  assistant can change nothing; with **Edit** it can open files and write, and what deletes,
  replaces, writes outside the workspace or reaches the web waits for your Allow — unless you tick
  **Always allow**, which is best kept for documents you trust.
- The panel lists the last calls: the tool, how long it took, and the error of one that failed.

What the assistant reads through the server goes to the assistant, and so to whoever runs its model
(see [PRIVACY.md](../PRIVACY.md)).

## Versions of the protocol

The server speaks MCP 2024-11-05 to 2025-11-25 (the `initialize` handshake) over Streamable HTTP,
without sessions: an assistant that was connected before the app was restarted goes on as it was. A
client of a later version (2026-07-28) is answered with the versions the server speaks, and falls
back to the handshake. `jsonquery-mcp` speaks the same over stdin and stdout.

## Building it yourself

```sh
cargo build --release -p jsonquery_gui -p jsonquery-mcp --features jsonquery_gui/mcp
# target/release/jsonquery_gui, with the server, and target/release/jsonquery-mcp beside it
```
