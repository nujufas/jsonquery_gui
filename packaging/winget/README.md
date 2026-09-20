# winget packaging

Manifests for the [Windows Package Manager community repository][pkgs]
(`microsoft/winget-pkgs`), so users can run `winget install nujufas.JsonQueryGui`
(or `winget install jsonquery-gui` via the moniker). 0.4.1 was submitted as
[microsoft/winget-pkgs#438074](https://github.com/microsoft/winget-pkgs/pull/438074).

The package installs the GitHub Release's Windows zip as a **portable**
package (`InstallerType: zip` + `NestedInstallerType: portable`): winget copies
`jsonquery_gui.exe` under `%LOCALAPPDATA%\Microsoft\WinGet\Packages\` and puts
a `jsonquery-gui` command on `PATH`. There is no Start-menu entry — that needs
a real installer (or the Microsoft Store's MSIX). The exe imports only Windows
system DLLs, so nothing else has to ship next to it.

Three files, schema 1.12.0 (same as `jqlang.jq`): `nujufas.JsonQueryGui.yaml`
(version), `….installer.yaml`, `….locale.en-US.yaml`. This directory holds the
current version, like `packaging/homebrew/` and `packaging/aur/`.

## Bump for a release

After the GitHub Release exists, in all three files set the new `PackageVersion`
and, in the installer manifest:

- `InstallerUrl` — the release asset URL
- `InstallerSha256` — of the **published** asset (download it and hash it;
  uppercase hex), not of the local `dist/` file
- `RelativeFilePath` — the zip's inner folder carries the version
  (`jsonquery_gui-<version>-windows-x86_64\jsonquery_gui.exe`)
- `ReleaseDate`, and `ReleaseNotesUrl` / `ReleaseNotes` in the locale file

## Test and submit

The scripts in [`tools/`](tools/) do the rest; [`tools/README.md`](tools/README.md) has the
full procedure and the gotchas we ran into.

```sh
packaging/winget/tools/validate-manifest.py       # schema + cross-file check (Linux)
packaging/winget/tools/windows-vm.sh up           # throwaway Windows 11 in Docker, browser viewer
packaging/winget/tools/windows-vm.sh stage        # manifests + test script + current winget -> the VM's share
#   in Windows: double-click test-winget-manifest.cmd (validate, install, Defender scan, uninstall)
packaging/winget/tools/windows-vm.sh results      # read the outcome on the host
packaging/winget/tools/submit-to-winget-pkgs.py --dry-run    # then without --dry-run: fork + PR
```

The PR is titled `New package: nujufas.JsonQueryGui version <version>` the first time and
`Update: nujufas.JsonQueryGui to <version>` afterwards (the repo's PR template). A bot then
validates it — it installs the package in a Windows VM and runs Defender over it — and a
volunteer moderator merges. If the bot ever reports a Defender false positive on the unsigned
exe, submit the file for review at <https://www.microsoft.com/wdsi/filesubmission> and comment
on the PR.

[pkgs]: https://github.com/microsoft/winget-pkgs
