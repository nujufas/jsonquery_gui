# jsonquery for Android

The Android app: the same Rust engine and egui UI as the desktop app, in a
phone-friendly layout, packaged as an Android App Bundle for Google Play.

Everything Android-specific lives in this directory, so it stays out of the
way of the desktop build:

```
android/
├── Cargo.toml, Cargo.lock   Android's own Cargo workspace (not part of the root one)
├── rust/                    the cdylib: NativeActivity entry point + the Kotlin bridge
├── app/                     Gradle module: manifest, Kotlin shell, resources, signing
├── docker/                  toolchain image (build) and emulator image (tests)
├── scripts/                 build.sh, run-phone.sh, run-tablet.sh, preflight.sh, publish.sh, ...
├── play/                    Google Play listing (text, icon, graphics, screenshots, release notes)
├── docs/                    Play Console runbook, privacy policy, declaration answers
└── test/                    Robot Framework suites, run against an emulator in Docker
```

The only things that live outside `android/` are what the app *shares* with the
desktop build: `crates/app` is now a library as well as the desktop binary, with
a `desktop` feature (default) and an `android` feature, and a small
`platform` seam (see [How it fits together](#how-it-fits-together)).

**All you need on the host is Docker** (and `/dev/kvm` to run the emulator
tests). Every compiler, the Android SDK/NDK and Rust run in containers.

## Build

```sh
android/scripts/build.sh apk          # debug APK for an emulator/phone -> android/dist/
android/scripts/build.sh aab          # signed release bundle for Google Play -> android/dist/
android/scripts/build.sh check        # rustfmt + Android lint
android/scripts/preflight.sh          # is that bundle acceptable to Play?
```

The first run builds the toolchain image (`docker/Dockerfile.build`, a ~7 GB image of
SDK, NDK and Rust) — once. Build state (Cargo target dir, Gradle and Cargo
caches, the container's home) lives in `android/.build/`; delete it to start
clean. Options: `--abis arm64-v8a,x86_64` and `--profile ci|release` (see
`scripts/build.sh`). A debug APK defaults to `x86_64` + the quick `ci` Rust
profile (right for the emulator); a bundle defaults to `arm64-v8a`,
`armeabi-v7a`, `x86_64` + `release`.

To try it on a phone: `adb install android/dist/jsonquery-<version>-debug.apk`
(build with `--abis arm64-v8a` for a phone; the default `x86_64` only runs on
emulators and Chromebooks).

## Try it by hand

```sh
android/scripts/run-phone.sh          # a Pixel 3 emulator window, with the app open
android/scripts/run-tablet.sh         # a Pixel Tablet (landscape), the same
android/scripts/run-phone.sh ~/data.json     # ...and put your own file in its Downloads folder
```

Each builds the debug APK if the sources are newer than it, boots the emulator
in a container (about a minute the first time), installs the app, puts the
sample JSON files from `test/resources/fixtures/` in the Downloads folder (so
**Open File…** has something to pick), and opens the app. The emulator's window
appears on your desktop: a click is a tap, click-and-drag swipes, holding the
button is a long press, and your keyboard types into the app. The strip beside
the window rotates the device and has Back and Home.

Changed some code? Run the same script again: it rebuilds, updates the app in
the running emulator (its saved data is kept) and reopens it, in a few seconds
when nothing needs building. Also `--build` / `--no-build`, `--fresh` (a clean
emulator), `--logs` (follow the app's log), `--stop` (or just close the window),
`--stop-all` (phone and tablet; they can run side by side, a few GB of RAM each).

It needs a desktop session (an X display: a Wayland desktop has one through
XWayland) and `/dev/kvm`, and nothing else on the host. Drawing is in software
(no GPU is used), so it is slower than a real device, the tablet most of all;
judge how the UI *looks* and *behaves* from it, not how fast it scrolls. These
emulators are separate from the headless ones `test/run.sh` uses.

## Test

```sh
android/test/run.sh                   # builds the APK if needed, boots a headless emulator, runs every phone suite
android/test/run.sh --device tablet   # the same on a tablet emulator (suites-tablet/)
android/test/run.sh suites/launch     # one suite
android/test/run.sh --stop            # shut the emulator container down
```

See [`test/README.md`](test/README.md). The emulator image is Android 16 with
**16 KB memory pages**, the page size Google Play requires apps to support, so a
green run also proves the native library loads there.

## Release to Google Play

```sh
android/scripts/keystore.sh           # once: the upload key (BACK IT UP; never commit it)
android/scripts/bump-version.sh       # versionCode +1 for every upload
android/scripts/build.sh aab
android/scripts/publish.sh            # preflight, then upload as a draft on the internal track
```

The Play Console steps only a person can do (account, app, declarations, the
first upload, the 12-testers/14-days rule for new personal accounts) are in
[`docs/play-store.md`](docs/play-store.md); what to answer is in
[`docs/play-console-answers.md`](docs/play-console-answers.md); the privacy
policy Play needs is [`docs/privacy-policy.md`](docs/privacy-policy.md).
CI does the same on a `v*` tag: `.github/workflows/android.yml`.

Store screenshots come from the real app, so they can't go stale:
`android/scripts/store-screenshots.sh` retakes them on the emulator (status bar
in demo mode) into `play/metadata/android/en-US/images/phoneScreenshots/`.

**Signing.** `app/build.gradle.kts` signs the release build with the upload key
from `android/signing/keystore.properties` (made by `keystore.sh`) or, in CI,
from the `ANDROID_KEYSTORE_FILE`, `ANDROID_KEYSTORE_PASSWORD`,
`ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD` environment variables. With
neither, the bundle is unsigned: fine to inspect, rejected by `preflight.sh`.
Google Play App Signing holds the real app-signing key; losing the *upload* key
is recoverable through Play support, but back it up regardless.

**Versions.** `versionName` is the workspace version in the root `Cargo.toml`
(so desktop and Android release together); `versionCode` is in
`version.properties` and only ever goes up. The application id
(`io.github.nujufas.jsonquery`) is permanent once published.

## How it fits together

```
 ┌───────────────────────── Android process ─────────────────────────┐
 │  MainActivity (Kotlin, a NativeActivity)                           │
 │    document picker · clipboard · on-screen keyboard · insets ·     │
 │    "Open with" / share intents                                     │
 │        │  ▲                                                        │
 │   JNI  │  │ JNI                                                    │
 │        ▼  │                                                        │
 │  libjsonquery_android.so  (android/rust)                           │
 │    android_main → eframe/egui (OpenGL ES) → jsonquery_gui::App     │
 │    implements jsonquery_gui::platform::Platform over the JNI seam  │
 └────────────────────────────────────────────────────────────────────┘
```

- **The UI is the desktop UI.** `jsonquery_gui::App` (in `crates/app`) runs
  unchanged; the seam is the `Platform` trait (`crates/app/src/platform.rs`):
  file pickers, clipboard, "open with" intents, keyboard and system-bar
  hooks. On desktop it is `rfd` dialogs and no-ops; here, `rust/src/platform.rs`
  calls into Kotlin.
- **Files.** Android hands out `content://` URIs, not paths. `Documents.kt`
  copies a picked document into the app cache (one directory per copy, so an
  open, memory-mapped document is never overwritten) and Rust opens that path.
  Saving is the reverse: Rust writes a staged file, then `Documents.commit`
  copies it to the URI the user chose.
- **The soft keyboard.** winit ignores the text an Android keyboard commits, so
  `ImeView.kt` — an invisible view that owns an `InputConnection` — receives it
  and forwards it over JNI; `crates/app/src/soft_keyboard.rs` (unit-tested on
  the desktop) turns it into egui events. Hardware keyboards go through winit
  as usual.
- **Phones and tablets.** Below 600 points wide (the window, not the device:
  about 690 dp, because touch mode zooms by 1.15) the UI switches to a compact
  layout (☰ menu, Source/Results tabs at the bottom); wider (tablets, landscape,
  a big split-screen window) it is the desktop's two panels. Touch mode
  enlarges rows and buttons; a long press is the right-click (egui does that
  natively). On touch the two panels each keep at least 280 points, and each
  orientation remembers its own split (a panel's width is remembered in points,
  so one shared split would squeeze a panel after a rotation).
- **System bars.** The window draws edge to edge (Android 15+ requires it);
  Kotlin sends the system-bar and keyboard insets, which become egui's safe
  area.

JNI names must stay in sync in three places: `MainActivity.kt`/`Native.kt`,
`rust/src/jvm.rs`/`native.rs`, and `app/proguard-rules.pro` (R8 must not rename
what Rust calls by name).

## Known limitations

- **No Ctrl shortcuts from a hardware keyboard** (Ctrl+F, Ctrl+S, Ctrl+A/C/V,
  Ctrl+Enter). winit's Android backend reports typed characters but no modifier
  keys, so egui never sees Ctrl. Typing works; use the menu and buttons.
- **No touch text-selection handles** (egui has none). Long-press a tree row to
  copy its JSON path; the Text view is selectable with a drag.
- **A picked document is copied into the app's cache** (Android hands out
  `content://` URIs, not paths), so a very large file briefly takes twice the
  space. The copy is deleted the next time the app starts.
- **Landscape phones and tablets ≥ 600 pt wide** get the desktop's two-panel
  layout, sized for touch; there is no separate tablet design. It is tested on
  a Pixel Tablet emulator (`test/run.sh --device tablet`), not on real hardware.
  Controls are about 37 dp tall (egui's touch size), under Material's 48 dp
  advice.

## Troubleshooting

- **Disk space.** The two images are ~7.4 GB (build) and ~9.3 GB (emulator), plus
  `android/.build/` (Cargo target dir, Gradle and Cargo caches: a few GB per
  ABI). `rm -rf android/.build` and `docker rmi jsonquery-android-build:1
  jsonquery-android-emulator:2` give it all back; the next build recreates it.
- **`INSTALL_FAILED_UPDATE_INCOMPATIBLE`** installing a debug APK: it was signed
  with a different debug key (another machine, or an APK from CI). Uninstall the
  installed one first. On one machine the key is stable, kept in
  `android/.build/home/.android/`.
- **An emulator test run can't start**: it needs `/dev/kvm` writable by you
  (`kvm` group). `android/test/run.sh --stop` resets the emulator container.
- **Logs.** `adb logcat -s jsonquery`. Raise `with_max_level` in
  `rust/src/lib.rs` to `Debug` to see the safe-area diagnostics.

## Upgrading the toolchain

Versions are pinned in `docker/Dockerfile.build` (SDK, NDK, Rust, cargo-ndk,
bundletool), `gradle/wrapper/gradle-wrapper.properties`, and `build.gradle.kts`
(AGP). After bumping the image, rebuild it with `scripts/image.sh` and rename
the tag in `scripts/lib.sh`. Google Play raises the required target API every
August: bump `targetSdk`/`compileSdk` in `app/build.gradle.kts` and the
`preflight.sh` check together.
