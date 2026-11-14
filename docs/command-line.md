# Command line

What `jsonquery_gui` opens as it starts. In the snap the command is `jsonquery-gui`;
an AppImage takes the same arguments after its file name
(`./jsonquery_gui-0.6.0-x86_64.AppImage data.json`). `jsonquery_gui --help` prints
all of this in short.

## A document, and a query to run on it

| Command | What it does |
|---|---|
| `jsonquery_gui data.json` | Opens the file, as **Load** does with its path in **Source**. A relative path is from the folder the command is run in. |
| `jsonquery_gui https://example.com/data.json` | Downloads it, as **Load** does with the address in **Source**. |
| `curl -s https://… \| jsonquery_gui -` | Opens what is piped in. **Source** stays empty and the line beside it says `(standard input)`. |
| `jsonquery_gui data.json --query ".items[] \| .name"` | Puts the query in the query box, and runs it once the document has loaded. |
| `jsonquery_gui data.json -q '$.items[*].name' --lang jsonpath` | The same, in the language named: `jq`, `jsonpath`, `jmespath` or `pointer`. Without `--lang` the language is told from the query, as when no language is picked in the app. |
| `jsonquery_gui data.json --pointer /items/0` | Shows the value at a JSON Pointer: the pointer runs as a query in the Pointer language. |
| `jsonquery_gui data.json errors.jq` | A file whose name ends in `.jq`, `.jsonpath`, `.jmespath` or `.jsonptr` is a query, as when it is dropped on the window: its text goes in the query box, in its language, and runs on the document. One from the [Workspace](workspace.md)'s folder is that item. |
| `jsonquery_gui a.json b.json c.json` | Several documents go to the Tools' **Merge** list, as when they are dropped on the window; nothing is merged until you press **Merge**. |

What is piped in may be as much as the **Largest download or pipe** limit (4 GB; see
[Settings](settings.md)). From the size files are kept on disk from (256 MB), it is
kept in a temporary file that nobody else can open and that is gone when the app
closes, and read from there as a big file is, as a download is.

A `file://` address (`file:///home/me/My%20Data/a.json`) is the file it names, which
is how a desktop's launcher may hand a file over; the **Source** field takes one too.

## A tool's page

Each of these fills the tool's page with the files, in the order of its boxes, brings
the page to the front (its tab, with the Tools docked, else their window), and runs
the tool once every box it needs is full:

| Command | Fills |
|---|---|
| `jsonquery_gui --diff old.json new.json` | Diff's left and right, and compares them |
| `jsonquery_gui --merge a.json b.json …` | Merge's list, and merges the files |
| `jsonquery_gui --patch doc.json fix.json` | Patch's document and patch (a JSON Patch or a merge patch), and applies it |
| `jsonquery_gui --validate doc.json schema.json` | Validate's document and schema, and checks the document |
| `jsonquery_gui --format doc.json` | Format's input, and formats it |
| `jsonquery_gui --create-schema doc.json` | Create Schema's document, and makes its schema |

Fewer files than boxes fill what they fill, and nothing runs (`--validate doc.json`
leaves the schema to you); none just shows the page. A file that cannot be read is
said in its box. The tools take files: a web address or `-` is refused.

### As git's difftool

```sh
git difftool -y -x "jsonquery_gui --diff" -- data.json
```

or, kept in git's settings:

```sh
git config --global difftool.jsonquery.cmd 'jsonquery_gui --diff "$LOCAL" "$REMOTE"'
git difftool -t jsonquery -y -- data.json
```

git waits for each window to be closed before it shows the next file.

## Help, version, and what cannot be done

- `--help` (`-h`) and `--version` (`-V`) print, and no window opens.
- A command line that cannot be done (an option that is not one, `--diff` with three
  files, `--query` and `--pointer` together, `-` when nothing is piped in) is said, with
  `See jsonquery_gui --help.`; the program ends with exit status 2 and no window opens.
- A document that cannot be opened is another matter: the window opens and says why,
  as **Load** does, with the path in **Source**.
- `--` ends the options: `jsonquery_gui -- -odd-name.json`.
- An option's value can follow it or be joined to it with `=`: `--query=.a`.

## Where it runs

- **Windows.** The app is a program with windows, not a console program. Started from
  a terminal, it prints `--help`, `--version` and what cannot be done into that
  terminal, but the terminal does not wait for it: its prompt comes back first (press
  Enter for a new one). `> file` and pipes work as they always do.
- **The snap** reads files in your home folder (not hidden ones), and on removable
  media once `snap connect jsonquery-gui:removable-media` has been run. A file
  elsewhere (`/tmp`, `/etc`) cannot be opened there.
- **macOS.** The arguments work when the app is started from a terminal. A file
  opened from the Finder (**Open With**) is not handed over this way, and is not
  opened yet.
- **Linux desktops.** The app's menu entry says that it opens JSON files, so a file
  manager offers it under **Open With** for a `.json` file.
- **The AppImage** takes `mcp` first as the [MCP server](mcp.md)'s program: to open a
  file called `mcp`, give `./mcp`.
