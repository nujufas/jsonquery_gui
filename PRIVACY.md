# Privacy policy: jsonquery gui

_Last updated: 2026-09-22_

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
- **URLs you enter.** If you type a web address into the Source field, the app requests it directly
  from your computer with an ordinary HTTP(S) request. The server behind that address sees the
  request, including your IP address, and handles it under its own policies. Nothing is sent to the
  developer. The response is saved to a temporary file in your operating system's temporary-files
  folder (on Windows, your user temp folder; on Linux, typically `/tmp`, or a private per-app
  location if you installed via Snap) and read from there. The app does not delete that file, so it
  stays until you or your OS's own temp-file cleanup removes it.
- **Files you save.** The app writes a file only when you choose Save, and only where you choose.
- **Settings and history.** The app keeps no history of your files or queries and does not save
  preferences between runs.

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
