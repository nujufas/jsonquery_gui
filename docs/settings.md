# Settings

What can be set, where it is kept, and what the app remembers of how you left it.

The **⚙** button at the right end of the status bar, just left of the **ⓘ**,
opens the **Settings** window. It is a small window of its own, like the Tools
window and the tutorial, so it can stay open beside the main window while you try
a limit.

## Limits on file sizes

Every size the app holds a file to is a setting, and each is what it has always
been until you change it. The window shows one of them, the one that decides how
the biggest files are opened, **Keep a file on disk from**: the size from which a
file is memory-mapped and indexed instead of being loaded and parsed. The other
eight are under **Advanced**, closed until you open it (a click on its name; the
window grows to hold them, and goes back when you close it), in groups: opening
files, the Tools window, Copy to Clipboard, and queries on a file kept on disk.
When some of those are not what the app starts with, the header says how many:
*Advanced (2 changed)*.

The window does not explain itself in writing: **hover the name of a limit** to
see what it does and what it is by default (the pointer becomes a **?**), hover
the box for how a size is typed, and the **Reset** button for what it puts back.
The table is the same information in one place.

| Limit | Default | What it is |
|---|---|---|
| **Keep a file on disk from** | 256 MB | From this size a file is not loaded and parsed but kept where it is, memory-mapped and indexed, and read as you look at it. Below it, a file is parsed, which is quicker but takes about twelve times its size in memory. Also decides whether a download is held in memory or spilled to a file. |
| **Largest download or pipe** | 4 GB | The most that is read from a web address, or from a pipe (such as standard input), which have no size to go by. |
| **Largest input** (Tools window) | 128 MB | The most that Merge, Diff, Patch and Validate take, all of a job's documents together: they work in memory, with several copies of what they are given. Format writes a file kept on disk out as it goes, whatever its size. |
| **Largest copy** | 64 MB | The most that Copy to Clipboard takes from a value of a file kept on disk, in bytes of the file. For more, there is Save…. |
| **List or object parsed whole** | 4 MB | On a file kept on disk, when a step of a query can't be done by reading the file, the list or object it needs is parsed and handed to jq: this is the biggest that is. |
| **String or other value** | 64 MB | The same, for a string or any other value that is not a list or an object. |
| **One result** | 16 MB | The biggest single result such a query may make. |
| **Gathered into a list** | 1 GB | How much `[ … ]`, `map(…)` and the like may gather in memory, as an estimate of what it takes. |
| **Keys of a sort or group** | 1 GB | How much the keys of a list that is sorted, grouped or searched for its smallest may take, as an estimate of the memory. |

A size is typed with its unit — `256 MB`, `4 GB`, `1.5 GB`, `512 KB`, `100 B` —
in any case; `MiB`, `GiB`, and just `M` and `G`, mean the same. As everywhere in
the app, 1 MB is 1,048,576 bytes. A number with no unit is in MB. A limit can be
from 1 KB to 1024 TB (the biggest, which is as good as none).

A box takes what is typed in it when it loses the focus — **Enter**, **Tab** or a
click elsewhere — or when the window is closed. What is not a size is said so
under the box and is not taken; the limit stays what it was, and **Esc** puts back
what was there. **Reset** (lit when the limit is not its default) puts one limit
back to its default, and **Restore defaults** all of them, the ones under
Advanced too.

A change applies from the next thing that uses the limit: the next file you open
(the first two limits), the next job you run in the Tools window, the next copy,
the next query. A document that is already open stays as it was opened.

Two of the limits work together. The Tools window needs a document in memory, so
a file it is given is parsed whatever size files are kept on disk from, as long as
it is under what the tools take; the open document, though, has been opened one way
or the other, and one that is kept on disk can't be given to Diff, Patch or
Validate (it can to Format). To use the tools on a bigger file, raise **Keep a
file on disk from** (and **Largest input**) *before* opening it. The tooltips say
so where it matters.

The limits are there to protect the machine: a limit higher than the memory there
is makes the app slow, or ends it, when a file needs more than there is. The
defaults are the ones the app was made and measured with.

## What is remembered

The app starts as you left it:

- **the theme**, light or dark (the ☀ / 🌙 button), dark the first time;
- **whether autocomplete suggestions are on** (the 💡 button), off the first time;
- **the size of the window**, and whether it was maximized (the size it goes back to
  is the one it had before), 1200 × 800 the first time. A size that is bigger than
  the screen it opens on is brought down to the screen. The position is not kept:
  window managers decide that;
- **the size of the panes**: how high the query panel is, and how much of the width
  Source has against Results. These are kept while the window is not maximized or
  full screen.

Nothing else is: not the files you opened, not your queries, not the pane layout
of a window that was popped out. (See the [privacy policy](../PRIVACY.md).)

## The file

Everything is in one file, `settings.json`, in the folder `.jsonquery` of your home
folder:

| | |
|---|---|
| Linux, macOS | `~/.jsonquery/settings.json` |
| Windows | `%USERPROFILE%\.jsonquery\settings.json` |

The Settings window says where it is, at the bottom. The environment variable
`JSONQUERY_HOME` names another folder for it (a full path): the file is then
`$JSONQUERY_HOME/settings.json`, which is how a portable install, a container or a
test keeps the settings of one run from another.

Only what is not the default is written, so the file of someone who has not
changed anything is not there at all, and a default that is better one day
reaches everyone who never touched it:

```json
{
  "limits": {
    "keep_on_disk_from": "512 MB",
    "tools": "1 GB"
  },
  "theme": "light",
  "autocomplete": true,
  "window": { "width": 1400.0, "height": 900.0, "maximized": true },
  "panes": { "query_height": 140.0, "source_share": 0.42 }
}
```

The names of the limits are `keep_on_disk_from`, `download`, `tools`, `copy`,
`query_parse`, `query_value`, `query_result`, `query_gather` and `query_keys`, in the
order of the table. A limit is text with a unit, as it is typed in the window, or a
number of bytes. `theme` is `"dark"` or `"light"`; `source_share` is from 0 to 1.

The file can be edited by hand while the app is not running (the app rewrites it
as a whole, and keeps nothing of what it does not know). A file that is not there,
or cannot be read, is no trouble: the app starts with the defaults. So is one with
something in it that cannot be used — a limit that is no size, a theme that is
neither — which is left out, and the Settings window says what and why. A file that
cannot be written (a folder that is read only) is said so in the same place; the
settings then last for as long as the app runs. Delete the file to put everything
back.

What the interface looks like is written a moment after it stops changing — less
than a second (a window dragged to a size is written at the size it stops at) — and
when the app closes; a limit is written at once.
