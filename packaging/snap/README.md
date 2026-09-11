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
build/snap.sh
# -> dist/jsonquery_gui-<version>-amd64.snap
```

Needs `snapcraft` plus a multipass or LXD build backend.

## Test locally

```sh
sudo snap install --dangerous dist/jsonquery_gui-<version>-amd64.snap
snap run jsonquery-gui
sudo snap remove jsonquery-gui   # when done
```

## Publish

Published as [`jsonquery-gui`](https://snapcraft.io/jsonquery-gui) on the
Snap Store (registered and first released 2026-09-11, under the `nujufas`
account). Per-release:

```sh
snapcraft login                        # your Ubuntu One / Snap Store account
snapcraft upload --release=stable dist/jsonquery_gui-<version>-amd64.snap
```

`upload --release` blocks until the Snap Store's automated review finishes
and, if it passes, releases straight to the `stable` channel — no manual
review step expected for a strict-confinement snap using only standard
interfaces (`gnome` extension, `network` plug).

Bump `version` in `snap/snapcraft.yaml` alongside `Cargo.toml` for each
release.
