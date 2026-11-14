//! The tutorial's tabs of help on what only the desktop app has: **AI assistants**
//! (the MCP server, in the builds that have it) and **General** (the command line,
//! the keys, big files, the Workspace, where to read more). Plain lessons — what to
//! do, commands to copy, tips — kept here in the app, not with the lessons of
//! `jsonquery_query::tutorial`, which the website shows too: none of this is for the
//! website.

/// A topic of a help tab: its lessons, in order.
pub struct HelpTopic {
    pub title: &'static str,
    pub lessons: &'static [HelpLesson],
}

/// A lesson: what to do, where to read or get more (links), the commands (or words)
/// that do it, each with a Copy button, pictures of it, and what is good to know.
pub struct HelpLesson {
    pub title: &'static str,
    pub summary: &'static str,
    pub links: &'static [HelpLink],
    pub snippets: &'static [Snippet],
    pub pictures: &'static [Picture],
    pub tips: &'static [&'static str],
}

/// A picture of what a lesson shows (a PNG, built into the app), and what it is.
pub struct Picture {
    /// What its texture is called: one of its own.
    pub name: &'static str,
    pub png: &'static [u8],
    pub caption: &'static str,
}

/// A web page to open (in the browser), and what it is.
pub struct HelpLink {
    pub text: &'static str,
    pub url: &'static str,
}

/// A command (or what to ask an assistant) to copy, and what it does.
pub struct Snippet {
    pub caption: &'static str,
    pub text: &'static str,
}

impl HelpLesson {
    /// Whether the lesson mentions `needle` (lower-cased), for the filter of the
    /// tutorial's list.
    pub fn matches(&self, needle: &str) -> bool {
        let has = |text: &str| text.to_lowercase().contains(needle);
        needle.is_empty()
            || has(self.title)
            || has(self.summary)
            || self
                .links
                .iter()
                .any(|link| has(link.text) || has(link.url))
            || self.snippets.iter().any(|s| has(s.caption) || has(s.text))
            || self.tips.iter().any(|tip| has(tip))
    }
}

pub const GENERAL_TAGLINE: &str =
    "The command line, the keys, big files, the Workspace, and where to read more.";

pub static GENERAL_TOPICS: &[HelpTopic] = &[
    HelpTopic {
        title: "Command line",
        lessons: &[
            HelpLesson {
                title: "Open a document as the app starts",
                summary: "Start the app with a document and it opens it, as *Load* does with it \
                          in *Source*: a file (a relative path is from the folder you are in), a \
                          web address, or `-` for what another program pipes into it.",
                links: &[],
                snippets: &[
                    Snippet {
                        caption: "A file",
                        text: "jsonquery_gui data.json",
                    },
                    Snippet {
                        caption: "A web address",
                        text: "jsonquery_gui https://api.github.com/repos/jqlang/jq",
                    },
                    Snippet {
                        caption: "What another program prints",
                        text: "curl -s https://api.github.com/repos/jqlang/jq | jsonquery_gui -",
                    },
                ],
                pictures: &[],
                tips: &[
                    "In the snap the command is `jsonquery-gui`; an AppImage takes the same \
                     after its file name.",
                    "Several documents go to the Tools' *Merge* list, as when they are dropped \
                     on the window.",
                    "What is piped in may be as big as a download (*Largest download or pipe*, \
                     under ⚙); from the size files are kept on disk from, it is kept on disk.",
                    "A `file://` address is the file it names, in *Source* too.",
                ],
            },
            HelpLesson {
                title: "Run a query as it opens",
                summary: "`--query` puts a query in the query box, and it runs once the document \
                          has loaded. `--lang` says its language — `jq`, `jsonpath`, `jmespath` or \
                          `pointer` — and without it the language is told from the query, as in \
                          the app.",
                links: &[],
                snippets: &[
                    Snippet {
                        caption: "A jq query",
                        text: "jsonquery_gui data.json --query \".items[] | .name\"",
                    },
                    Snippet {
                        caption: "A JSONPath query",
                        text: "jsonquery_gui data.json -q \"$.items[*].name\" --lang jsonpath",
                    },
                    Snippet {
                        caption: "The value at a JSON Pointer",
                        text: "jsonquery_gui data.json --pointer /items/0",
                    },
                    Snippet {
                        caption: "A query kept in a file",
                        text: "jsonquery_gui data.json names.jq",
                    },
                ],
                pictures: &[],
                tips: &[
                    "A file whose name ends in `.jq`, `.jsonpath`, `.jmespath` or `.jsonptr` is a \
                     query, as when it is dropped on the window.",
                    "Put a query in double quotes: they work in every shell, Windows' too.",
                ],
            },
            HelpLesson {
                title: "Open a tool",
                summary: "`--diff`, `--merge`, `--patch`, `--validate`, `--format` and \
                          `--create-schema` fill that tool's page with the files, in the order of \
                          its boxes, and run it once they are all filled.",
                links: &[],
                snippets: &[
                    Snippet {
                        caption: "Compare two documents",
                        text: "jsonquery_gui --diff old.json new.json",
                    },
                    Snippet {
                        caption: "Check a document against a JSON Schema",
                        text: "jsonquery_gui --validate order.json order.schema.json",
                    },
                    Snippet {
                        caption: "Apply a JSON Patch or a merge patch",
                        text: "jsonquery_gui --patch doc.json fix.json",
                    },
                    Snippet {
                        caption: "Merge files",
                        text: "jsonquery_gui --merge a.json b.json c.json",
                    },
                    Snippet {
                        caption: "Format a document",
                        text: "jsonquery_gui --format data.json",
                    },
                    Snippet {
                        caption: "Make a JSON Schema of a document",
                        text: "jsonquery_gui --create-schema data.json",
                    },
                    Snippet {
                        caption: "Compare with git, as its difftool",
                        text: "git difftool -y -x \"jsonquery_gui --diff\" -- data.json",
                    },
                ],
                pictures: &[],
                tips: &[
                    "Fewer files than boxes fill what they fill, and the tool waits: \
                     `--validate order.json` leaves the schema to you.",
                    "git waits for each window to be closed before it shows the next file.",
                ],
            },
            HelpLesson {
                title: "Help, the version, and mistakes",
                summary: "`--help` lists all of this and `--version` says which version you \
                          have; neither opens a window.",
                links: &[],
                snippets: &[
                    Snippet {
                        caption: "All the options",
                        text: "jsonquery_gui --help",
                    },
                    Snippet {
                        caption: "The version",
                        text: "jsonquery_gui --version",
                    },
                ],
                pictures: &[],
                tips: &[
                    "A command line that cannot be done is said, and no window opens (the exit \
                     status is 2). A document that cannot be opened is another matter: the \
                     window opens and says why.",
                    "`--` ends the options: `jsonquery_gui -- -odd-name.json`.",
                    "On Windows the terminal does not wait for the app: its prompt comes back \
                     before what the app prints.",
                ],
            },
        ],
    },
    HelpTopic {
        title: "Working with the app",
        lessons: &[
            HelpLesson {
                title: "Keys",
                summary: "The keys that save the most clicks.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "`Ctrl+Enter` runs the query, or the main button of a tool's page.",
                    "`Ctrl+F` finds in the pane you clicked last; `Ctrl+S` saves what it shows.",
                    "`F1` opens this window.",
                    "A right click on a row of a tree, or on a text, has what can be done with \
                     it: copy its path or its value, save it, find it.",
                ],
            },
            HelpLesson {
                title: "Big files",
                summary: "A file of 256 MB or more is not loaded whole: it is kept on disk, \
                          indexed in about a second for every 500 MB, and read as you look at \
                          it, so files of gigabytes open. A long list shows in runs.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "That size, and the other limits on files, are in ⚙ Settings.",
                    "A few jq functions that need all of a document at once may not work on one \
                     kept on disk: the app says so.",
                    "The Tools work in memory, on up to 128 MB at a time (⚙); Format writes a \
                     big file out as it goes.",
                ],
            },
            HelpLesson {
                title: "The Workspace",
                summary: "Your own queries, schemas, patches, documents and web addresses, kept \
                          in folders of a folder of your own. The button at the left end of the \
                          status bar shows its pane.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "A double click opens an item: a query goes in the query box, a schema or a \
                     patch in its tool.",
                    "The pane saves the query box, a schema, a patch or the Source as an item, \
                     with a sample if you want one.",
                    "It is an ordinary folder on your computer: nothing is sent anywhere.",
                ],
            },
            HelpLesson {
                title: "Read more",
                summary: "The README and the docs of the app's repository say more about each \
                          part; ℹ has the known issues and the privacy policy.",
                links: &[
                    HelpLink {
                        text: "The docs",
                        url: "https://github.com/nujufas/jsonquery_gui/tree/master/docs",
                    },
                    HelpLink {
                        text: "The command line, all of it",
                        url: "https://github.com/nujufas/jsonquery_gui/blob/master/docs/command-line.md",
                    },
                    #[cfg(feature = "mcp")]
                    HelpLink {
                        text: "The MCP server, all of it",
                        url: "https://github.com/nujufas/jsonquery_gui/blob/master/docs/mcp.md",
                    },
                ],
                snippets: &[],
                pictures: &[],
                tips: &[],
            },
        ],
    },
];

#[cfg(feature = "mcp")]
pub const MCP_TAGLINE: &str = "Let an AI assistant work with what JsonQuery has open.";

#[cfg(feature = "mcp")]
pub static MCP_TOPICS: &[HelpTopic] = &[
    HelpTopic {
        title: "Getting started",
        lessons: &[
            HelpLesson {
                title: "What MCP is",
                summary: "MCP, the Model Context Protocol, is an open standard for giving an AI \
                          assistant tools: a program says which tools it has, and the assistant \
                          calls one when a question needs it, and reads its answer. JsonQuery is \
                          such a program, with 27 tools: an assistant on your computer — Claude \
                          Code, Claude Desktop, VS Code, QuickAI and others — can look at what \
                          JsonQuery has open, read and query documents of any size, and use the \
                          six Tools.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "To look: `get_state` (what is open), `document_info`, `describe` (a \
                     document's shape), `read_value` (a value by its JSON Pointer), `search`.",
                    "To query: `run_query` (jq, JSONPath, JMESPath or JSON Pointer), \
                     `get_results` (what the window shows), `query_help`.",
                    "The six Tools: `format_json`, `diff_documents`, `apply_patch`, \
                     `merge_documents`, `validate_json`, `create_schema`.",
                    "The Workspace: `workspace_list`, `workspace_read`.",
                    "With *Edit* allowed, in the window: `open_document`, `set_query`, `reveal`, \
                     `show_in_tool`, `open_url`, `save_file`; and in the Workspace \
                     `workspace_open`, `workspace_save`, `workspace_new_folder`, \
                     `workspace_move`, `workspace_delete`.",
                    "It is off until you start it, listens on this computer only (127.0.0.1), \
                     and answers only calls that carry its token. What the assistant reads goes \
                     to its model: to the model's company for one in the cloud, nowhere for one \
                     that runs on your computer, as QuickAI's does.",
                ],
            },
            HelpLesson {
                title: "Start the server",
                // (Each 🔌 is drawn as the status bar's icon, the plug with "AI".)
                summary: "🔌 in the status bar opens the server's panel. *Start* starts the \
                          server, at `http://127.0.0.1:7457/mcp`. 🔌 is green while an \
                          assistant works with the app.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "Tick *Start it with JsonQuery* to have it start with the app.",
                    "The port can be changed while the server is stopped. *New token* makes the \
                     one assistants were given useless: add the server to them again.",
                    "The panel lists the last calls; one that was refused is red.",
                ],
            },
            HelpLesson {
                title: "Read or Edit",
                summary: "With *Read*, the default, the assistant looks and computes, and \
                          changes nothing you see. With *Edit* it may also open a document, put a \
                          query in the query box and run it, show you a value, fill a page of the \
                          Tools, save, move and delete in the Workspace, save a file and load a \
                          web address.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "Before anything is deleted or replaced, written outside the Workspace, or \
                     fetched from the web, JsonQuery asks you — unless you tick *Always allow*.",
                    "*Answers* sets how big an answer may be, 4 to 200 KB: small for a model \
                     with a small context.",
                ],
            },
        ],
    },
    HelpTopic {
        title: "Add it to an assistant",
        lessons: &[
            HelpLesson {
                title: "Claude Code",
                summary: "The panel's *Claude Code* button copies the command that adds the \
                          server, with its token: `claude mcp add --transport http jsonquery \
                          http://127.0.0.1:7457/mcp --header \"Authorization: Bearer …\"`. Run \
                          it in a terminal, and Claude Code has JsonQuery's tools.",
                links: &[],
                snippets: &[],
                pictures: &[Picture {
                    name: "claude-code-chat",
                    png: include_bytes!("../../../../assets/help/claude-code-chat.png"),
                    caption: "Claude Code in a terminal answers from the open document, then \
                              puts its query in JsonQuery's query box and runs it.",
                }],
                tips: &["The command holds the token: keep it to yourself."],
            },
            HelpLesson {
                title: "Claude Desktop",
                summary: "The *Claude Desktop* button copies an `mcpServers` entry that starts \
                          `jsonquery-mcp`, the program beside JsonQuery that takes the calls to \
                          the app. Put it in `claude_desktop_config.json` (in Claude Desktop: \
                          Settings › Developer › Edit Config) and start Claude Desktop again.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "The calls are answered while JsonQuery runs with its server started; the \
                     tools are listed even when it does not.",
                    "Copy the entry again after you move the app, or after an upgrade puts it in \
                     another folder.",
                ],
            },
            HelpLesson {
                title: "VS Code",
                summary: "The *VS Code* button copies a `servers` entry of type `http`, with the \
                          address and the token. Put it in VS Code's `mcp.json` — your own, or a \
                          folder's `.vscode/mcp.json` — and start the server there.",
                links: &[],
                snippets: &[],
                pictures: &[
                    Picture {
                        name: "vscode-config",
                        png: include_bytes!("../../../../assets/help/vscode-config.png"),
                        caption: "VS Code's mcp.json with the entry that JsonQuery's VS Code \
                              button copied (its token hidden here), started: Running, 27 \
                              tools.",
                    },
                    Picture {
                        name: "vscode-chat",
                        png: include_bytes!("../../../../assets/help/vscode-chat.png"),
                        caption: "Claude Code in VS Code, with the folder's .mcp.json, answers \
                              from the open document, then puts its query in JsonQuery's \
                              query box and runs it.",
                    },
                ],
                tips: &[
                    "That entry is for GitHub Copilot's chat. The Claude Code extension in VS \
                     Code takes a folder's `.mcp.json` instead: the *JSON* button's entry, or \
                     the command of the *Claude Code* button run in the folder.",
                    "The entry holds the token: keep it to yourself.",
                ],
            },
            HelpLesson {
                title: "QuickAI",
                summary: "QuickAI runs a model on your own computer, so your documents stay on \
                          it. The *QuickAI* button copies an `mcpServers` entry that starts \
                          `jsonquery-mcp`: in QuickAI's *MCP servers*, choose *Paste the JSON of \
                          a server's instructions*.",
                links: &[HelpLink {
                    text: "Get QuickAI: quickai.everybodyai.dev",
                    url: "https://quickai.everybodyai.dev",
                }],
                snippets: &[],
                pictures: &[
                    Picture {
                        name: "quickai-config",
                        png: include_bytes!("../../../../assets/help/quickai-config.png"),
                        caption: "In QuickAI's MCP servers: the entry that JsonQuery's QuickAI \
                                  button copied, pasted, before Add these.",
                    },
                    Picture {
                        name: "quickai-chat",
                        png: include_bytes!("../../../../assets/help/quickai-chat.png"),
                        caption: "Qwen3.5-9B in QuickAI answers from the open document, then, \
                                  with Edit allowed, puts its query in JsonQuery's query box and \
                                  runs it.",
                    },
                ],
                tips: &[
                    "Use *Qwen3.5-9B-Q4_K_M* or better (a bigger model, or one less quantized), \
                     with a good context length: 32K tokens or more. The tools take about 7K \
                     tokens before the first question, and an answer up to 6K more with \
                     *Answers* at 24 KB.",
                    "With a smaller context, choose *Answers* of 8 KB in the panel.",
                ],
            },
            HelpLesson {
                title: "Other assistants",
                summary: "The *JSON* button copies an `mcpServers` entry of type `http`, with \
                          the address and the token: for a project's `.mcp.json` (Claude Code), \
                          Cursor's `mcp.json`, and other assistants that take a server by its \
                          address.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "An assistant that starts its servers itself, and takes no address, starts \
                     `jsonquery-mcp`, as Claude Desktop does.",
                    "The entry holds the token: keep it to yourself.",
                ],
            },
        ],
    },
    HelpTopic {
        title: "Using it",
        lessons: &[
            HelpLesson {
                title: "What to ask",
                summary: "Ask about what JsonQuery has open, in your own words: the assistant \
                          picks the tools.",
                links: &[],
                snippets: &[
                    Snippet {
                        caption: "With Read",
                        text: "What is open in JsonQuery, and what does it hold?",
                    },
                    Snippet {
                        caption: "With Read",
                        text: "How many orders in the open document are over 100? Show me the \
                               query.",
                    },
                    Snippet {
                        caption: "With Read",
                        text: "Check the open document against order.schema.json in the \
                               Workspace, and tell me what is wrong.",
                    },
                    Snippet {
                        caption: "With Edit",
                        text: "Put a query that lists the names in the query box and run it.",
                    },
                    Snippet {
                        caption: "With Edit",
                        text: "Make a JSON Schema of the open document and save it in the \
                               Workspace.",
                    },
                ],
                pictures: &[],
                tips: &[
                    "With *Edit*, what the assistant runs in the query box is in the window, for \
                     you to see and change.",
                ],
            },
            HelpLesson {
                title: "Large documents",
                summary: "An answer is never bigger than the *Answers* size. A value too big \
                          for it is given as a snippet — an array's first items, an object's \
                          small members, a long string's start — and the answer says so, with \
                          how to read on: the assistant reads it a page at a time.",
                links: &[],
                snippets: &[],
                pictures: &[],
                tips: &[
                    "The tools that only read work out of sight, on any document — the open \
                     one, a file, a Workspace item — so a file of gigabytes never passes through \
                     the conversation whole.",
                ],
            },
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{parse, Ask};
    use std::ffi::OsString;
    use std::path::Path;

    /// Every help tab's topics, with the tab's name.
    fn tabs() -> Vec<(&'static str, &'static [HelpTopic])> {
        #[allow(unused_mut)]
        let mut tabs = vec![("General", GENERAL_TOPICS)];
        #[cfg(feature = "mcp")]
        tabs.push(("MCP", MCP_TOPICS));
        tabs
    }

    /// The words of a command line, as a shell splits them (double quotes only).
    fn words(line: &str) -> Vec<String> {
        let (mut words, mut word, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
        for c in line.chars() {
            match c {
                '"' => (quoted, any) = (!quoted, true),
                ' ' if !quoted => {
                    if any {
                        words.push(std::mem::take(&mut word));
                    }
                    any = false;
                }
                c => (word, any) = (word + &c.to_string(), true),
            }
        }
        if any {
            words.push(word);
        }
        words
    }

    #[test]
    fn every_lesson_says_something() {
        for (tab, topics) in tabs() {
            assert!(!topics.is_empty(), "{tab}");
            for topic in topics {
                assert!(!topic.lessons.is_empty(), "{tab} › {}", topic.title);
                for lesson in topic.lessons {
                    assert!(!lesson.title.is_empty() && lesson.summary.len() > 20);
                    for text in [lesson.summary].iter().chain(lesson.tips) {
                        assert!(!text.contains("  "), "a double space: {text:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_command_line_shown_is_one_the_app_takes() {
        let mut seen = 0;
        for lesson in GENERAL_TOPICS.iter().flat_map(|t| t.lessons) {
            for snippet in lesson.snippets {
                // The app's part: after a pipe; in git's difftool, the command it is given.
                let line = snippet.text.rsplit("| ").next().unwrap();
                let line = match line.strip_prefix("git difftool -y -x \"") {
                    Some(rest) => rest.split('"').next().unwrap().to_owned() + " old.json new.json",
                    None => line.to_owned(),
                };
                let Some(args) = words(&line)
                    .strip_prefix(&["jsonquery_gui".to_owned()])
                    .map(<[String]>::to_vec)
                else {
                    continue;
                };
                let args: Vec<OsString> = args.iter().map(OsString::from).collect();
                let asked = parse(args, Some(Path::new("/w")), |_| Ok(".".to_owned()));
                assert!(
                    matches!(asked, Ok(Ask::Run(_) | Ask::Help | Ask::Version)),
                    "{:?}: {asked:?}",
                    snippet.text
                );
                seen += 1;
            }
        }
        assert!(seen >= 14, "the command lines were found: {seen}");
    }

    #[test]
    fn every_picture_is_a_png_with_a_caption() {
        let mut names = Vec::new();
        for (_, topics) in tabs() {
            for lesson in topics.iter().flat_map(|t| t.lessons) {
                for picture in lesson.pictures {
                    let image = eframe::icon_data::from_png_bytes(picture.png)
                        .unwrap_or_else(|e| panic!("{}: {e}", picture.name));
                    assert!(
                        image.width >= 600 && image.height >= 300,
                        "{}",
                        picture.name
                    );
                    assert!(picture.caption.len() > 20, "{}", picture.name);
                    assert!(!names.contains(&picture.name), "{} twice", picture.name);
                    names.push(picture.name);
                }
            }
        }
        if cfg!(feature = "mcp") {
            assert_eq!(
                names,
                [
                    "claude-code-chat",
                    "vscode-config",
                    "vscode-chat",
                    "quickai-config",
                    "quickai-chat"
                ]
            );
        }
    }

    #[test]
    fn the_filter_finds_a_lesson_by_what_it_says() {
        let lesson = &GENERAL_TOPICS[0].lessons[2];
        assert!(lesson.matches("--diff") && lesson.matches("difftool") && lesson.matches(""));
        assert!(!lesson.matches("qwen"));
    }

    /// Every tool the server has is in the first lesson, and only those.
    #[cfg(feature = "mcp")]
    #[test]
    fn the_first_lesson_names_every_tool_of_the_server() {
        let lesson = &MCP_TOPICS[0].lessons[0];
        let named: Vec<&str> = lesson
            .tips
            .iter()
            .flat_map(|tip| tip.split('`').skip(1).step_by(2))
            .collect();
        let mut named_sorted = named.clone();
        named_sorted.sort_unstable();
        let mut tools = jsonquery_mcp::TOOL_NAMES.to_vec();
        tools.sort_unstable();
        assert_eq!(named_sorted, tools, "{named:?}");
        assert!(lesson.summary.contains(&format!("{} tools", tools.len())));
    }

    #[cfg(feature = "mcp")]
    #[test]
    fn quickai_says_what_model_it_needs() {
        let quickai = MCP_TOPICS
            .iter()
            .flat_map(|t| t.lessons)
            .find(|l| l.title == "QuickAI")
            .expect("a lesson on QuickAI");
        assert!(quickai.matches("qwen3.5-9b-q4_k_m") && quickai.matches("32k tokens"));
        let [link] = quickai.links else {
            panic!("one link, to where QuickAI is got")
        };
        assert_eq!(link.url, "https://quickai.everybodyai.dev");
    }
}
