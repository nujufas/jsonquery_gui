# winget release tooling

The scripts used to get `nujufas.JsonQueryGui` into [winget-pkgs][pkgs] (0.4.1 went in as
[#438074][pr]): check the manifests, test them on a **real Windows** in a throwaway Docker
VM, and open the pull request. It all runs from the Linux dev box; only Docker images are
added to the host.

| File | Runs on | What it does |
| --- | --- | --- |
| [`validate-manifest.py`](validate-manifest.py) | Linux | Schema check (fetches Microsoft's JSON schema for the manifest's own `ManifestVersion`) plus the cross-file rules the pipeline enforces. |
| [`windows-vm.sh`](windows-vm.sh) | Linux | `up` / `status` / `screenshot` / `stage` / `results` / `stop` / `down` for a Windows 11 VM ([dockur/windows][dockur]: QEMU/KVM in a container, browser viewer). Shared with the [Scoop](../../scoop/README.md) and [Chocolatey](../../chocolatey/README.md) tests: `stage`/`results` take a channel (`winget` by default). |
| [`test-winget-manifest.ps1`](test-winget-manifest.ps1) + [`.cmd`](test-winget-manifest.cmd) | Windows VM | Validate, install, inspect, Defender-scan and uninstall the manifest with a current winget; writes logs back to the host. |
| [`submit-to-winget-pkgs.py`](submit-to-winget-pkgs.py) | Linux | Forks winget-pkgs (through the API, no clone), pushes one commit, opens the PR; re-running updates the same PR. |

## Prerequisites

- Docker, and read/write access to `/dev/kvm` (hardware virtualisation, group `kvm`).
- Disk: the VM disk is capped at 64 GB but sparse; it reached ~18 GB after installing and
  testing. The ISO is ~7 GB. Give it about 35 GB free. RAM: the VM gets 8 GB.
- `gh`, logged in with the `repo` scope (`gh auth status`).
- `pip install pyyaml jsonschema` for the validator (a venv is fine); Pillow is optional,
  for PNG screenshots.

## Procedure

**1. Bump and check (Linux).** Edit the three manifests as described in
[`../README.md`](../README.md), then:

```sh
packaging/winget/tools/validate-manifest.py
```

**2. Start the Windows VM.** Once; later runs resume it.

```sh
packaging/winget/tools/windows-vm.sh up
```

Open <http://127.0.0.1:8006>. The first boot downloads Windows 11 Enterprise *Evaluation*
straight from Microsoft (Microsoft's free 90-day test edition; you are responsible for
complying with its licence) and installs it unattended, about 20-30 minutes. `status` shows
progress; `screenshot` saves what the screen shows. The VM disk lives in `~/windows-vm/storage`
(outside the repo). It logs in as user `Docker` automatically.

**3. Stage and run the test.**

```sh
packaging/winget/tools/windows-vm.sh stage
```

This copies the manifests (as CRLF, exactly what will be submitted), the test script and a
current winget release into the share, which Windows sees as `\\host.lan\Data` (the **Shared**
folder on the desktop, also drive `Z:`). In Windows, double-click **`test-winget-manifest.cmd`**
and accept the UAC prompt. It runs, in order:

1. winget version gate: installs the staged App Installer if winget is older than the
   manifest's schema needs.
2. `winget settings --enable LocalManifestFiles`
3. `winget validate --manifest`
4. `winget install --manifest` (removing a previous install first)
5. the `jsonquery-gui` command resolves and the exe exists
6. Authenticode status, then a Microsoft Defender scan after updating its signatures
7. `-Launch` only: start the GUI, report whether the process stays alive
8. `winget uninstall --manifest` and a check that nothing is left behind

Options, from a prompt: `powershell -ExecutionPolicy Bypass -File \\host.lan\Data\test-winget-manifest.ps1 -Launch`
(`-KeepInstalled`, `-ManifestDir <path>` and `-NoPause` also exist; see the script header).

**4. Read the result on the host.**

```sh
packaging/winget/tools/windows-vm.sh results winget
```

(Every channel's test script writes `logs/latest-<channel>.txt` and also `logs/latest-result.txt`, which is
whichever test ran last; `results` with no channel prints that one.)

Also in `~/windows-vm/shared/logs/`: the full transcript and winget's own `WinGet-*.log`
diagnostics. `WARN` for an unsigned exe is expected; a `WARN` about stale Defender signatures
means the scan proves little; any `FAIL` means do not submit yet.

**5. Submit.**

```sh
packaging/winget/tools/submit-to-winget-pkgs.py --dry-run     # read-only: shows files, title, PR text
packaging/winget/tools/submit-to-winget-pkgs.py --validated-locally --tested-locally
```

Pass the two flags only once step 4 passed; the script ticks the other checklist boxes
itself only when it verified them (no duplicate PR, schema check passed) and leaves the CLA
box for `--cla-signed`. Then:

- The CLA bot asks you to reply `@microsoft-github-policy-service agree` on the PR. That is a
  legal agreement, so it has to be you. Once per GitHub account.
- The validation pipeline (10 checks, including an installer scan and a real install in a
  Windows VM) runs; labels such as `Azure-Pipeline-Passed` appear.
- A volunteer moderator approves and merges; that can take a while.
- If a moderator asks for changes, fix the manifests here and re-run the script: it adds a
  commit to the same branch and the PR updates.

**6. Other channels, same VM.** `windows-vm.sh stage scoop` and `windows-vm.sh stage choco` put the Scoop
manifest / the Chocolatey `.nupkg` and their test scripts into the same share; see
[`../../scoop/README.md`](../../scoop/README.md) and [`../../chocolatey/README.md`](../../chocolatey/README.md).
All three scripts work in this VM as they are: UAC is off here (see the gotcha below), so nothing needs elevating.

**7. Tear down.** `windows-vm.sh stop` (resume with `up`), `windows-vm.sh down` removes the
container; delete `~/windows-vm/storage` to reclaim the disk.

Settings (`WINVM_DIR`, `WINVM_VERSION`, `WINVM_RAM`, ...) are documented at the top of
`windows-vm.sh` (`windows-vm.sh help`). Ports are published on 127.0.0.1 only.

## Gotchas we hit

- **The ISO's winget is too old.** The 2024 Windows 11 build ships winget 1.6, which rejects
  manifest schema 1.12. The test script installs the current App Installer from the share.
- **`winget uninstall --id` cannot see a `--manifest` install.** Such a package is not
  tracked by any source, so it answers "No installed package found" (`0x8A150014`). Uninstall
  with `--manifest <dir>` instead. Real users installing from the repo are unaffected.
- **UAC is switched off in this VM.** dockur's unattended setup writes `EnableLUA=false`, so every process
  runs with the full administrator token: the "relaunch as Administrator" steps in the scripts are no-ops
  here, and there is no "normal user" to test as. Scoop's installer refuses an elevated shell unless given
  `-RunAsAdmin`, which the Scoop test passes when it finds itself elevated (Scoop itself does not care).
- **Elevated shells do not see mapped drives.** `Z:` vanishes once elevated; the script converts
  its own paths to `\\host.lan\Data`. Use the UNC form in commands you type in an admin shell.
- **winget-pkgs stores manifests with CRLF.** The submit script commits CRLF; the copies in this
  repo stay LF. Do not clone winget-pkgs for this: it is enormous.
- **PR titles** follow the repo's template: `New package: Publisher.Name version X` for the first
  submission, `Update: Publisher.Name to X` afterwards.
- **Defender signatures in the ISO date from 2023.** The script updates them and prints their age.
  The real pipeline scans the package too, and 0.4.1 passed it.
- **No GPU in the VM**, so the egui GUI may not start there. `-Launch` reports that as `WARN`.
- **"RDP is answering" only means Windows booted.** First-run setup may still be finishing
  ("This might take a few minutes"); use `screenshot` before starting the test.
- **The web viewer has no clipboard.** RDP does: `127.0.0.1:3389`, user `Docker`, password `admin`.
- **Keep the `.ps1`/`.cmd` ASCII.** Windows PowerShell 5.1 reads BOM-less files as ANSI; `stage`
  refuses non-ASCII files and converts both to CRLF on the way in.

[pkgs]: https://github.com/microsoft/winget-pkgs
[pr]: https://github.com/microsoft/winget-pkgs/pull/438074
[dockur]: https://github.com/dockur/windows
