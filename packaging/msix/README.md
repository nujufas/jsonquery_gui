# MSIX packaging (Microsoft Store)

The Microsoft Store takes the app as an **MSIX** package. Microsoft re-signs it after certification, so no
code-signing certificate is needed ([their words][sign]), and an individual developer account is free.

The package is a thin wrapper: it contains the released `jsonquery_gui.exe` (the same bytes winget, Scoop
and Chocolatey install), the icons and a manifest. Nothing is rebuilt for the Store.

| File | What it is |
| --- | --- |
| [`AppxManifest.xml`](AppxManifest.xml) | Template: full-trust desktop app, `jsonquery-gui` command alias, Windows 10 1809+ x64. |
| [`identity.env`](identity.env) | The package identity: `STORE_*` from Partner Center (the reserved name's identity), `DEV_*` for sideload tests. |
| [`build-msix.ps1`](build-msix.ps1) | Stages the release zip's exe + icons + rendered manifest and packs it with MakeAppx (`-Flavor Store` unsigned, `-Flavor Test` self-signed). |
| [`Assets/`](Assets/), [`make-assets.py`](make-assets.py) | The generated icon set (committed) and the script that regenerates it from `assets/icon.png`. It also writes the Store logos (1:1 box art, 9:16 poster art) to the git-ignored `dist/store-listing/`. |
| [`tools/test-msix-package.ps1`](tools/test-msix-package.ps1) + [`.cmd`](tools/test-msix-package.cmd) | Windows VM: trust the test certificate, install, check registration, launch (with a screenshot), uninstall. |
| [`../../.github/workflows/msix.yml`](../../.github/workflows/msix.yml) | Builds both flavors on a GitHub Windows runner from a published release tag. |
| [`../../PRIVACY.md`](../../PRIVACY.md) | The privacy policy the Store requires. |

## What `runFullTrust` means here

A Win32 program can't run inside MSIX's normal app sandbox, so the manifest declares the restricted
capability `runFullTrust`: the packaged exe runs as an ordinary desktop process with the user's own rights,
just as it does from the zip. It is a *restricted* capability, so Partner Center asks for a written
justification and a tester reviews it: say that it is a native Win32 program packaged with the Desktop Bridge (no
UWP version), that it opens and saves the files the user picks and fetches a web address the user types, and that
it asks for no other restricted capability. `internetClient` is not needed: it only applies to sandboxed apps.

## Build

```sh
gh workflow run msix.yml -f tag=v0.4.1            # the workflow must be on the default branch
gh run watch                                      # ~2 minutes
gh run download <run id> -n msix-v0.4.1 -D dist/msix
```

The workflow downloads the release's Windows zip, checks it against the digest GitHub recorded for the asset,
then builds a **test** package (signed with a throwaway certificate; the `.cer` comes with it) and, once
`STORE_*` in `identity.env` is filled in, the **Store** package (unsigned). The version becomes `0.4.1.0`: the
Store reserves the fourth number and requires it to be 0.

## Test on a real Windows

Same Docker VM as the other channels ([`../winget/tools/README.md`](../winget/tools/README.md)):

```sh
packaging/winget/tools/windows-vm.sh stage msix     # the test .msix + .cer from dist/msix, and the test script
#   in Windows: double-click test-msix-package.cmd
packaging/winget/tools/windows-vm.sh results msix   # also logs/msix-launch.png: a screenshot of the running app
```

The launch step is worth reading: the VM has no GPU, like many of the machines the Store's certification testers
use. The app renders through wgpu (DirectX 12/Vulkan, with a software fallback), which should come up anyway, but
this is the check that proves it.

## Submit (once)

1. **Account** (you, ~15 minutes): <https://storedeveloper.microsoft.com> > Get started for free > Individual
   developer, sign in with your Microsoft account, verify with a government ID and a selfie on your phone. The
   new flow has no registration fee; other entry points may show the old paid one.
2. **Reserve the name** in Partner Center (Apps and games > New product > **MSIX or PWA app**): `jsonquery gui`.
   Not *EXE or MSI app*: that product type wants an Authenticode-signed installer hosted at a URL (its Packages
   page asks for a "Package URL" and silent-install parameters, and the Store does not re-sign it), and the UI cannot
   change the type afterwards (a support request can). If it was created wrong: reserve a second name on that product,
   delete the first name there (Product management > Manage app names; a product must keep one name), then create a
   new MSIX or PWA product and reserve `jsonquery gui` again.
3. **Identity:** Product management > Product identity. Paste *Package/Identity/Name*, *Package/Identity/Publisher*
   and *Package/Properties/PublisherDisplayName* into `STORE_*` in `identity.env`, exactly (they are case-sensitive).
4. **Build** the Store package with the workflow above (it is then in the artifact as `jsonquery-gui_<ver>.0_x64.msix`).
5. **Privacy policy:** push [`PRIVACY.md`](../../PRIVACY.md) to `master` so its URL works.
6. **Submission:** create the submission, upload the `.msix`, and fill in each page: description, features,
   keywords, 1920x1080 light-theme screenshots (made per release), Store logos, notes for certification. The
   listing text and images are deliberately not kept in the repo.
7. **Submit for certification.** The `runFullTrust` justification is asked on *Submission options*. Certification
   usually takes a few business days; the restricted capability can add time. Results and any rejection reasons
   arrive in Partner Center and by email.

## Later releases

Each new version is a new submission: bump the tag, run the workflow, upload the new `.msix` (version `x.y.z.0`
must be higher than the last), and paste the release notes into *What's new*. The capability justification is not
asked again unless the package declares another restricted capability.

## Notes

- **Unsigned by design** for the Store. Only the test package is signed, and only with a throwaway certificate.
- The manifest asks for Windows 10 1809 (build 17763) or later, x64 only.
- The `jsonquery-gui` command alias is registered for the user by Windows (`%LOCALAPPDATA%\Microsoft\WindowsApps`).
- The app does not add itself as a `.json` handler; that is deliberate (no surprise file-association prompts).

[sign]: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements#code-signing-for-microsoft-store-submissions
