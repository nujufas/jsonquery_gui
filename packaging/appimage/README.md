# AppImage packaging

```sh
build/appimage.sh            # -> dist/jsonquery_gui-<version>-x86_64.AppImage
build/appimage.sh aarch64    # -> dist/jsonquery_gui-<version>-aarch64.AppImage
```

Both cross-build in Docker (an old glibc, so the binary needs only
`GLIBC_2.18`) and then run `appimagetool`. The AppDir is the binary, icons, a
`.desktop` file and an `AppRun` that self-registers the launcher on first run.

Two things in it exist so the AppImage starts on *bare* systems (a container,
a CI runner, a minimal install), not just on a full desktop.

## Static runtime

`appimagetool` embeds its own runtime by default, an old one linked
dynamically against the host's libc that also `dlopen`s libfuse2. Ubuntu 24.04
and other current distros no longer install libfuse2, so such an AppImage does
not even start there. `build/appimage.sh` therefore always embeds the
maintained static type 2 runtime
(`AppImage/type2-runtime`, `runtime-<arch>`), cached as `build/runtime-<arch>`.
The AppImage catalog reports the old one as "an old AppImage runtime that needs
the C library (and libfuse2) of the system".

## Fallback libraries

The binary links only libc, libm and libgcc_s. X11, Wayland, xkbcommon and GL
are `dlopen`ed at run time from the host. A desktop has them all; a bare system
may not. The AppImage catalog's runner has `libxkbcommon0` but not
`libxkbcommon-x11`, and v0.4.2 died there at startup:

```
thread 'main' panicked at xkbcommon-dl-0.4.2/src/x11.rs:59:28:
Library libxkbcommon-x11.so could not be loaded.
```

So the AppImage carries `libxkbcommon`, `libxkbcommon-x11` and `libxcb-xkb` in
`usr/lib/fallback/` (fetched by `fetch_fallback_libs` in `build/appimage.sh`,
cached in `build/fallback-libs/`). Rules that matter:

- **The system's copies win.** `AppRun` puts the directory on
  `LD_LIBRARY_PATH` only if the system lacks at least one of the libraries, so
  a system that already works never loads them.
- **All or nothing.** `libxkbcommon-x11` shares internals with `libxkbcommon`.
  Pairing the bundled `-x11` with the system's *different* `libxkbcommon`
  segfaults, which is exactly the catalog runner's situation. The first version
  of this fix did that and crashed; hence one matched set, used whole.
- **Ubuntu 20.04 builds, pinned by sha256.** They are the newest LTS whose
  libraries reference nothing above glibc 2.17; 22.04's `libxkbcommon` needs
  2.33, which would raise the AppImage's floor above the binary's own 2.18.
  Hashes come from the archive's `Packages` index
  (`dists/focal/main/binary-<arch>/Packages.gz`). To change them, update the
  table in `build/appimage.sh`, then check the floor (`readelf --dyn-syms -W`)
  and re-run the smoke test.
- **Only this family.** libX11, libGL, libEGL and friends pair with the host's
  drivers and are not bundled.

The AppImage is still not "self-contained" in the catalog's sense: it uses the
system's C library. That is deliberate. The app loads the host's GPU drivers
and resolves host names through the host's NSS modules, both built against the
host's glibc, so bundling one is what the AppImage guidelines advise against.
The low glibc floor is the answer instead.

## Smoke test

```sh
packaging/appimage/catalog-smoke/smoke.sh                 # dist/…-x86_64.AppImage
packaging/appimage/catalog-smoke/smoke.sh path/to.AppImage
```

Needs Docker; everything runs in containers, nothing touches the host display.
It checks what the catalog checks (static runtime, glibc floor) and then runs
the AppImage as an ordinary user, without network, under Xvfb 800x600x24 with
Mesa software GL, in an Ubuntu 22.04 image shaped like the catalog's runner
(`catalog-smoke/Dockerfile`, `HOST` build arg):

| case | system has | expected |
|------|------------|----------|
| `ci` | `libxkbcommon0` only | starts on the bundled libraries |
| `desktop` | the whole xkbcommon family | starts on the **system's** libraries |
| `bare` | none of it | starts on the bundled libraries |
| `appimage` | as `ci`; runs the `.AppImage` file itself | starts via its embedded runtime |

"Starts" means a window appears within 30 s and the screenshot is not blank.
Screenshots and logs land in `dist/appimage-smoke/`. Run it before attaching an
AppImage to a release: the v0.4.2 AppImage fails `ci`, `bare` and `appimage`
and the static-runtime check, and passes `desktop`, which is why it went out
unnoticed.

### arm64

The smoke test is x86_64 only. For arm64, check statically (`unsquashfs
-offset <size of build/runtime-aarch64>` and `file`, `readelf --dyn-syms -W`
on the payload) and run the binary under qemu with the harness in
[`../snap/arm64-smoke/`](../snap/arm64-smoke/) (usage in
[`../snap/README.md`](../snap/README.md#testing-the-arm64-snap-without-arm-hardware)),
pointing `LD_LIBRARY_PATH` at the unpacked `usr/lib/fallback` to prove the
bundled arm64 set works with the binary.

## The AppImage catalog

[appimage.github.io](https://github.com/AppImage/appimage.github.io) tests every
entry in a PR (`data/jsonquery_gui`, found by their discovery workflow) on an
`ubuntu-22.04` runner and comments the result. After releasing a fixed
AppImage, the AppImage's own GitHub account (or a maintainer) comments
`/retest` on the PR to run it again; the catalog tests the AppImage of the
*latest* release.

Remarks the catalog still prints that are not failures: no AppStream metadata,
no embedded update information, and a desktop-file hint that `Development;
Utility;` are both main categories.
