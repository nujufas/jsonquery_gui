# Chocolatey packaging

Package `jsonquery-gui` for the [Chocolatey community repository](https://community.chocolatey.org/),
so Windows users can run `choco install jsonquery-gui`. The package downloads the release's Windows
zip from GitHub, verifies its SHA-256, and installs it under `C:\ProgramData\chocolatey\lib`, with a
`jsonquery-gui` command and a **jsonquery gui** Start-menu shortcut. Nothing but two small scripts
is inside the package itself, so it stays at a few KB.

| File | What it is |
| --- | --- |
| [`jsonquery-gui.nuspec`](jsonquery-gui.nuspec) | Package metadata: the fields the repository's validator and moderators look at. |
| [`tools/chocolateyInstall.ps1`](tools/chocolateyInstall.ps1) | `Install-ChocolateyZipPackage` with url + checksum, the `jsonquery-gui` shim, the Start-menu shortcut. |
| [`tools/chocolateyUninstall.ps1`](tools/chocolateyUninstall.ps1) | Removes the shim and the shortcut (the extracted files go with the package folder). |
| [`dev/check-package.py`](dev/check-package.py) | Linux pre-flight: nuspec fields, version consistency, download + checksum + zip contents, URLs. |
| [`dev/choco.sh`](dev/choco.sh) | `pack` builds the `.nupkg` and `push` uploads it, both with the official Chocolatey CLI in a container. |
| [`dev/test-choco-package.ps1`](dev/test-choco-package.ps1) + [`.cmd`](dev/test-choco-package.cmd) | Windows VM: install Chocolatey, install the staged `.nupkg`, verify, uninstall. |

`tools/` is the package's own folder: **everything in it is packed**, so the development scripts live
in `dev/` and `check-package.py` fails if anything else lands in `tools/`.

## Bump for a release

After the GitHub Release exists:

1. `jsonquery-gui.nuspec`: `<version>`, and the `v<version>` tag in `<iconUrl>` and `<releaseNotes>`.
2. `tools/chocolateyInstall.ps1`: `$version` and `$checksum64` (sha256 of the **published** zip, not of the
   local `dist/` file).
3. `dev/check-package.py` (it downloads the asset and compares).

The script carries its own `$version` instead of using `$env:ChocolateyPackageVersion`, so a package fix
re-published as `0.4.1.20261001` (a "fix version", no new upstream release) still downloads the upstream
`0.4.1` files.

## Test on a real Windows

Uses the same Docker Windows VM as winget ([`../winget/tools/README.md`](../winget/tools/README.md)):

```sh
packaging/chocolatey/dev/choco.sh pack                  # check-package.py, then dist/chocolatey/jsonquery-gui.<ver>.nupkg
packaging/winget/tools/windows-vm.sh up                 # once; resumes later
packaging/winget/tools/windows-vm.sh stage choco
#   in Windows: double-click test-choco-package.cmd and accept the UAC prompt; it installs
#   Chocolatey on first use, so it needs the VM's internet
packaging/winget/tools/windows-vm.sh results choco
```

The script installs the very `.nupkg` you would push, from a local folder, and checks: Chocolatey reported
`Hashes match`, the exe is where the script says, the `jsonquery-gui` shim works (and there is *no* second
`jsonquery_gui` shim), the shortcut points at the exe, `choco list` shows it, and `choco uninstall` leaves
nothing behind.

## Submit

One-time, by you (it needs your own account):

1. Register at <https://community.chocolatey.org/account/Register> and confirm the email address.
2. Copy your API key from <https://community.chocolatey.org/account>.
3. Push `packaging/chocolatey` to GitHub first: the nuspec's `packageSourceUrl` points at it, and
   `check-package.py` warns until that URL exists. The release tag (`v<version>`) must exist too, because
   `iconUrl` uses it.

Then:

```sh
packaging/chocolatey/dev/choco.sh pack
CHOCOLATEY_API_KEY=... packaging/chocolatey/dev/choco.sh push      # asks before it uploads
```

What happens next (Chocolatey's [moderation process](https://docs.chocolatey.org/en-us/community-repository/moderation/)):
an automated validator (metadata rules), verifier (installs and uninstalls the package in a clean Windows VM)
and scanner (VirusTotal) start within about half an hour, then a human moderator reviews the package; expect
that to take a while for a first submission. Questions or requested changes arrive as comments and emails on
the package page; answer there, or fix the package and push the same version again. A submission nobody acts on
for 35 days is rejected. Later versions go through the same checks (packages with a track record can become
"trusted" and skip the human step). Progress: <https://community.chocolatey.org/packages/jsonquery-gui>.

## Decisions

- **Download, don't embed.** The zip is 8 MB and stable on GitHub, so the package fetches it and checks the
  hash. Embedding would need `VERIFICATION.txt`/`LICENSE.txt` and gets extra moderator scrutiny for binaries.
  If a moderator asks for it, embed the exe instead.
- **Our own shim.** Chocolatey shims every exe it finds; the package marks `jsonquery_gui.exe` with a `.ignore`
  file and registers `jsonquery-gui` with `Install-BinFile`, so the command is the same as on winget, Scoop, Snap
  and Homebrew.
- **Unsigned exe.** The scanner may report a few generic detections for an unsigned Rust binary; if a moderator
  asks, point at the source repository and the build script (`build/windows.sh`).
- **Later automation.** A release workflow could `choco pack` and `push` on each tag (Chocolatey's Linux image runs
  in GitHub Actions), with the API key as a repository secret. Not set up: the first version has to go through
  human review anyway.
