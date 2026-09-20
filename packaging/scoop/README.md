# Scoop packaging

[Scoop](https://scoop.sh) installs the release's Windows zip and gives the user a Start-menu
shortcut and a `jsonquery-gui` command:

```pwsh
scoop bucket add jsonquery-gui https://github.com/nujufas/scoop-jsonquery-gui
scoop install jsonquery-gui/jsonquery-gui
```

It lives in **its own bucket**, [`nujufas/scoop-jsonquery-gui`](https://github.com/nujufas/scoop-jsonquery-gui),
not in Scoop's official ones: *Main* only takes command-line tools, and *Extras* wants a project
that is "reasonably well-known" (100+ stars and/or 50+ forks on GitHub). Revisit Extras once the
project is there; the manifest below is already in the shape Extras expects, so it is a package-request
issue plus a pull request with a copy.

| File | What it is |
| --- | --- |
| [`jsonquery-gui.json`](jsonquery-gui.json) | The manifest: 64-bit zip, `extract_dir`, the `jsonquery-gui` shim, the Start-menu shortcut, `checkver` + `autoupdate`. |
| [`bucket-README.md`](bucket-README.md) | The README the bucket repository gets (`{{REPO}}` is filled in). |
| [`tools/validate-manifest.py`](tools/validate-manifest.py) | Linux: Scoop's JSON schema, version consistency, and the real download (hash, zip contents). |
| [`tools/test-scoop-manifest.ps1`](tools/test-scoop-manifest.ps1) + [`.cmd`](tools/test-scoop-manifest.cmd) | Windows VM: install Scoop, run Scoop's own checks, install/verify/uninstall the app. |
| [`tools/test-scoop-bucket.cmd`](tools/test-scoop-bucket.cmd) | Windows VM launcher: the same test against the published bucket (`-Bucket`). |
| [`tools/publish-bucket.sh`](tools/publish-bucket.sh) | Linux: create the bucket from Scoop's template the first time; push the manifest afterwards. |

## Releases need (almost) nothing

The manifest has `checkver` (the latest GitHub release of the homepage) and `autoupdate` (the URL
pattern), and the bucket, created from Scoop's [BucketTemplate][tpl], runs the **Excavator**
workflow every four hours: it notices a new release, downloads the zip, rewrites `version`, `url`,
`hash` and `extract_dir`, and commits. So a release reaches Scoop users on its own within hours.

To do it right away, or to keep this folder current, bump `jsonquery-gui.json` by hand: `version`,
the `url` (both places the version appears), `extract_dir`, and `hash` (lowercase sha256 of the
**published** asset). Then check it and push it:

```sh
packaging/scoop/tools/validate-manifest.py          # schema, version consistency, download + hash
packaging/scoop/tools/publish-bucket.sh --dry-run   # what would be pushed
packaging/scoop/tools/publish-bucket.sh
```

`publish-bucket.sh` refuses to push an older version over a newer one (Excavator may already have
bumped the bucket past this folder); `--force` overrides.

## Test on a real Windows

Uses the same Docker Windows VM as winget ([`../winget/tools/README.md`](../winget/tools/README.md)):

```sh
packaging/winget/tools/windows-vm.sh up             # once; resumes later
packaging/scoop/tools/validate-manifest.py
packaging/winget/tools/windows-vm.sh stage scoop
#   in Windows: double-click test-scoop-manifest.cmd; it installs Scoop on first use, so it
#   needs the VM's internet
packaging/winget/tools/windows-vm.sh results scoop
```

The script runs Scoop's own `checkver`, `checkhashes` and `formatjson` on the manifest (the same
checks the bucket's CI and Excavator use), installs the app from the local file, verifies the
files, the `jsonquery-gui` shim and the shortcut, and uninstalls again, checking nothing is left.
To test the **published bucket** instead (adds it with `scoop bucket add`, installs `jsonquery-gui/jsonquery-gui`
from it, uninstalls, removes the bucket; installs Git through Scoop first if needed), double-click
`test-scoop-bucket.cmd` (staged by the same `stage scoop`), then `windows-vm.sh results scoop`.

## Publish the bucket (done once, 2026-09-20)

`publish-bucket.sh` creates the public repository from Scoop's template
(`gh repo create --template ScoopInstaller/BucketTemplate`), tags it `scoop-bucket` (so it is
indexed on scoop.sh), drops in the manifest, a README and the corrected `bin/auto-pr.ps1`, and
pushes. The template brings the CI workflow, which runs Scoop's schema and style tests on every
push, and the Excavator. Watch the first runs with `gh run list --repo nujufas/scoop-jsonquery-gui`.
The first CI runs passed on both Windows PowerShell and pwsh (7 Pester tests, including Scoop's schema
validation of the manifest), and a manual Excavator run finished cleanly with nothing to update.

## Notes

- The exe is unsigned; Scoop does not care, but SmartScreen may warn at first launch.
- Scoop's *installer* refuses an elevated shell (`-RunAsAdmin` overrides); Scoop itself does not. The test VM
  has UAC turned off (dockur's unattended setup), so the test script passes `-RunAsAdmin` there.
- The exe is a GUI-subsystem program, so Scoop's shim starts it without holding the console.
- The app writes nothing next to itself, so no `persist` entries are needed.
- The bucket's CI insists on final newlines, no trailing whitespace and CRLF checkouts (the template's
  `.gitattributes` does the CRLF part); `publish-bucket.sh` checks the files it writes.
- Scoop's manifest reference: <https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests>.

[tpl]: https://github.com/ScoopInstaller/BucketTemplate
