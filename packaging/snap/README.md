# Snap packaging

The actual manifest lives at [`snap/snapcraft.yaml`](../../snap/snapcraft.yaml)
(Snapcraft only looks for it there, not under `packaging/`). This file is
the maintainer-facing build/test/publish notes for it.

## Why not `plugin: rust`

`snap/snapcraft.yaml` uses `plugin: nil` with an `override-build` that
installs Rust via `rustup.sh` directly, instead of `plugin: rust`. That
plugin fetches its toolchain via the `rustup` *snap*, but the `gnome`
extension's own PATH-prepending line in the generated build environment
makes `/snap/bin` invisible during the plugin's environment-validation
step — so it reproducibly fails with `'rustup' not found` regardless of
build backend (confirmed not a timing flake: `rustup` was independently
verified working inside the same build instance while the plugin's own
check still failed). This is an interaction bug between the `rust` plugin
and the `gnome` extension, not anything specific to this project or host.

## Build

```sh
build/snap.sh                      # -> dist/jsonquery_gui-<version>-amd64.snap
build/snap.sh --build-for arm64    # -> dist/jsonquery_gui-<version>-arm64.snap
```

Needs `snapcraft` plus a multipass or LXD build backend.

### arm64 is cross-built on an amd64 host

No ARM machine is needed. `snapcraft.yaml` declares `arm64` with
`build-on: [amd64, arm64]`, and `override-build` branches on
`CRAFT_ARCH_BUILD_ON` vs `CRAFT_ARCH_BUILD_FOR`: equal is the plain native
`cargo build` (the amd64 path, unchanged); different adds the
`aarch64-unknown-linux-gnu` Rust target and links with
`gcc-aarch64-linux-gnu`. This works because the binary links only
libc/libm/libgcc_s; X11, Wayland, xkbcommon, GL and Vulkan are `dlopen`ed at
run time from the `gnome` content snap, so no arm64 `-dev` libraries are
needed at build time. Only the `ring` crate compiles C.

Two things to expect:

- **`libc6-dev-arm64-cross` must be listed in `build-packages` explicitly.**
  It is only a *Recommends* of `gcc-aarch64-linux-gnu`, which snapcraft
  skips. Without it the cross gcc quietly falls back to the host's amd64
  `/usr/include` and `ring` dies with `bits/libc-header-start.h: No such
  file or directory`.
- snapcraft prints `Unable to determine library dependencies for
  'bin/jsonquery_gui'`. That is expected: its library linter can't inspect a
  foreign-arch ELF. Nothing is missing — check with
  `readelf -d bin/jsonquery_gui | grep NEEDED` on the unsquashed snap.

## Test locally

```sh
sudo snap install --dangerous dist/jsonquery_gui-<version>-amd64.snap
snap run jsonquery-gui
sudo snap remove jsonquery-gui   # when done
```

### Testing the arm64 snap without ARM hardware

`snap install` can't run an arm64 snap here, so [`arm64-smoke/`](arm64-smoke/)
runs the snap's binary under `qemu-user` inside a throwaway Docker image
(qemu, Xvfb, and an Ubuntu 24.04 arm64 library sysroot standing in for core24
plus the `gnome` content snap). It doesn't register anything on the host:

```sh
unsquashfs -d /tmp/arm64-snap dist/jsonquery_gui-<version>-arm64.snap
docker build -t jsonquery-arm64-smoke packaging/snap/arm64-smoke   # ~1.5 GB, once
mkdir -p /tmp/arm64-out
docker run --rm \
  -v /tmp/arm64-snap/bin:/sysroot/app:ro \
  -v "$PWD/packaging/snap/arm64-smoke/run-gui.sh:/run-gui.sh:ro" \
  -v /tmp/arm64-out:/out \
  jsonquery-arm64-smoke bash /run-gui.sh
```

Expect `WINDOW MAPPED after ~15s` and `app still running -> alive`;
`/tmp/arm64-out/gui.png` is a screenshot of the emulated app (software
rendering, so it is slow but real). Without a display the binary should
instead exit cleanly with winit's "neither WAYLAND_DISPLAY … nor DISPLAY is
set". This exercises the aarch64 binary, glibc and the windowing/rendering
stack; it does not exercise snap confinement or the real content snap, which
only the Store's install path and a real ARM device would. Also worth a
static check on the unsquashed snap: `file bin/jsonquery_gui` (ARM aarch64)
and `readelf -V bin/jsonquery_gui` (highest `GLIBC_` must be ≤ 2.39, core24's).

## Publish

Published as [`jsonquery-gui`](https://snapcraft.io/jsonquery-gui) on the
Snap Store (registered and first released 2026-09-11, under the `nujufas`
account). Per-release:

```sh
snapcraft login                        # your Ubuntu One / Snap Store account
snapcraft upload --release=stable dist/jsonquery_gui-<version>-amd64.snap
snapcraft upload --release=stable dist/jsonquery_gui-<version>-arm64.snap
```

Each architecture is uploaded separately and gets its own Store revision
(revision numbers are shared across architectures); a release only affects
its own architecture's channel map.

`upload --release` blocks until the Snap Store's automated review finishes
and, if it passes, releases straight to the `stable` channel — no manual
review step expected for a strict-confinement snap using only standard
interfaces (`gnome` extension, `network` plug).

Bump `version` in `snap/snapcraft.yaml` alongside `Cargo.toml` for each
release.
