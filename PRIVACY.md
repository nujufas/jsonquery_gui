# Privacy policy: jsonquery gui

_Last updated: 2026-11-11_

jsonquery gui ("the app") is a desktop program for browsing and querying JSON files on your own
computer. This policy covers the app however you install it: the Microsoft Store, GitHub Releases,
winget, Scoop, Chocolatey, Snap or Homebrew.

## In short

The app itself does not collect, store or send personal information to its developer or to anyone
else. It has no accounts, no analytics or telemetry, no advertising, no crash reporting and no
automatic update check of its own. Some of the stores and package managers you can install it
through do their own data collection, independent of anything the app does — see
[Stores and package managers](#stores-and-package-managers) below.

## What the app does with your data

- **Files and pasted JSON.** The app reads JSON files you choose (with the file dialog, a path you
  type, or drag and drop) and JSON you paste. They are processed on your computer and are never
  uploaded anywhere.
- **URLs you enter.** If you type a web address into the Source field (or open one that you saved
  in the workspace), the app requests it directly from your computer with an ordinary HTTP(S)
  request. The server behind that address sees the request, including your IP address, and handles
  it under its own policies. Nothing is sent to the developer. The response is saved to a temporary
  file in your operating system's temporary-files folder (on Windows, your user temp folder; on
  Linux, typically `/tmp`, or a private per-app location if you installed via Snap) and read from
  there. The app does not delete that file, so it stays until you or your OS's own temp-file cleanup
  removes it.
- **Big results of a merge.** A merge whose result is as big as the size from which the app keeps a
  file on disk (256 MB unless you changed it in the settings) is written, while you look at it, to a
  temporary file in your operating system's temporary-files folder, readable only by you where the
  system has file modes, and read from there instead of being held in memory. The file is taken out of
  the folder the moment it is made and is gone when the app lets go of it (on Windows, when it is
  closed), so nothing of it is left behind. A smaller result stays in memory.
- **Links.** The links in the ⓘ window, and the 🌐 button in the status bar (which goes to the
  project's website, <https://jsonquery.dev>), hand an address to your web browser when you click
  them. The app sends nothing itself. The site you land on, GitHub or jsonquery.dev, sees an
  ordinary visit from your browser and handles it under its own policy (the website's is at
  <https://jsonquery.dev/privacy>).
- **Files you save.** The app writes a file only when you choose Save, and only where you choose.
- **Settings.** The app keeps no history of your files or queries. It does save your settings — the
  limits on file sizes you set, the theme, whether autocomplete is on, whether the Tools are docked
  in the main window, how big its window and panes were, and how the workspace pane (below) is shown
  and in which folder it is kept — in one small file on your computer, `settings.json` in the
  `.jsonquery` folder of your home folder (`%USERPROFILE%\.jsonquery` on Windows). It holds nothing
  about your files or queries, is never sent anywhere, and can be deleted at any time to put
  everything back to the defaults.
- **The workspace.** If you save a query, a schema, a patch, a document or a web address in the
  workspace, the app writes it as a file — and, if you ask for it, a sample of data with it (not with
  a web address) — in the workspace's folder, which is the folder `workspace` beside the settings
  (`~/.jsonquery/workspace`) unless you chose another, together with a small hidden file in it,
  `.jsonquery/index.json`, that holds an id and a SHA-256 (a fingerprint of the file's text) for each
  item. The app writes these only when you choose to save, import or change something in the
  workspace, only in that folder (or where you choose, for Export), and never sends them anywhere.
  They stay until you delete them: they are ordinary files in an ordinary folder.

## The MCP server

The app can also act as a [Model Context Protocol](https://modelcontextprotocol.io) server for an
AI assistant you choose, such as Claude Code, Claude Desktop, VS Code or QuickAI. See
[docs/mcp.md](docs/mcp.md). The builds with **_nomcp** in their name (downloads of the GitHub
release) do not have it.

- **Off until you start it.** The server runs only after you start it with the 🔌 in the status bar
  (or tick **Start it with JsonQuery**), and only while the app runs. It listens on `127.0.0.1`, so
  it cannot be reached from another computer, and it answers only requests that carry its token.
- **What the assistant gets.** An assistant you connect can read what the app has open — the
  document, the query and its results, the items and samples of your workspace — and, if you set
  **Allow** to **Edit**, read files of your computer and open them in the app. What it reads is sent to
  that assistant, and so to whoever runs its AI model (for a cloud assistant, its provider), under
  their policies. Nothing is sent to the developer. A local model, such as QuickAI's, keeps it on
  your computer.
- **What it may change.** Only with **Allow** set to **Edit** can the assistant save, move or delete
  in your workspace, write a file, or have the app load a document from a web address (the app then
  connects to that address, as when you type it in the Source field). The app asks you first, each
  time, before anything is deleted or replaced, a file is written outside the workspace, or an
  address is fetched — unless you tick **Always allow** (in the panel, or in the question), after
  which it does them without asking until you untick it.
- **What is kept.** The port, a random token, whether to start with the app, what the assistant is
  allowed, whether to always allow it and the size of an answer are kept in `mcp.json`, beside
  `settings.json`, readable only by you (where the system has file modes). The last 50 calls are
  listed in the panel while the app runs, and what the assistant's last query or tool made is kept
  in memory for its next calls while the server runs; neither is kept after it stops.

## Stores and package managers

These collect data independently of the app — none of it passes through jsonquery gui's own code,
and most of it doesn't reach the developer at all.

### Microsoft Store

If you install the app from the Microsoft Store, Microsoft may collect data about the installation
and use of apps under [Microsoft's privacy statement](https://privacy.microsoft.com/privacystatement).
The developer can see the aggregate reports the Store provides (for example install counts, and crash
reports from people who chose to share diagnostic data with Microsoft). These do not identify you.

### winget

The `winget` client itself (not this app, and not the Microsoft Store) is instrumented to send usage
and diagnostic data to Microsoft, governed by the same Windows diagnostic-data setting as the Store —
see [winget's own privacy statement](https://github.com/microsoft/winget-cli/blob/master/PRIVACY.md).
The developer doesn't receive any per-package data from this.

### Snap Store

If you install the app from the Snap Store, `snapd` periodically checks for and installs updates on
its own, and Canonical's servers see those requests under
[Canonical's privacy policy](https://canonical.com/legal/data-privacy). The developer can see
aggregate install metrics the Snap Store provides (for example counts by country, architecture and
app version). These do not identify you.

### Chocolatey

Chocolatey's Community Repository logs the IP address, package name and timestamp of each download
to produce public install-count statistics shown on the package's page — see
[Chocolatey's privacy policy](https://chocolatey.org/privacy). The `choco` client itself sends no
telemetry of its own.

### Homebrew

Homebrew (the package manager, not this project) collects anonymous install analytics by default for
formulae from any tap, including this one — package and tap name, CPU architecture, OS, and whether
the install was explicit — with no user identifier or IP address recorded, per
[Homebrew's analytics documentation](https://docs.brew.sh/Analytics). It's on by default; turn it off
with `brew analytics off`. This feeds Homebrew's own public aggregate statistics, not a per-project
dashboard the developer can see.

### Others

GitHub Releases (the direct tarball/AppImage download, and what winget and Scoop both fetch from),
Scoop, and the AUR add no telemetry of their own beyond what's already covered above — a GitHub
release page shows public per-file download counts, visible to anyone, not just the developer.

## Children

The app is not directed at children and does not knowingly collect information from anyone.

## Changes

If this policy changes, the new version is published in this file with a new date.

## Contact

Questions about this policy: open an issue at
<https://github.com/nujufas/jsonquery_gui/issues>.
