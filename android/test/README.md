# Android GUI tests

Robot Framework suites that drive the real app on a real (emulated) Android
device — install the APK, tap, type, read the screen — mirroring the desktop
suite in [`test/`](../../test) but fully isolated from it: own library, own
fixtures, own container.

```sh
android/test/run.sh                        # every phone suite (builds the debug APK first if missing)
android/test/run.sh --device tablet        # every tablet suite (suites-tablet/)
android/test/run.sh suites/queries         # one suite
android/test/run.sh --test 'TC-AND-02*'    # any robot option
android/test/run.sh --stop                 # shut the emulator containers down
```

Results land in `android/test/results/`; open `log.html`: **every test has
screenshots of what the screen showed**, and its final state.

## What runs where

Only Docker on the host, plus `/dev/kvm`:

- `docker/Dockerfile.emulator` — Android 16 emulator with **16 KB memory
  pages** (`google_apis_ps16k`, the page size Google Play requires apps to
  support), a Pixel 3-sized phone (1080×2160 @ 440 dpi, software GPU), `adb`,
  Robot Framework and Tesseract.
- `run.sh` starts one container from it per device profile and leaves it
  running, so only the first run pays the boot; `--stop` removes them.
  `in-emulator.sh` runs inside. The same image and `emulator-entry.sh` (with
  `WINDOW=1`, so the emulator gets a window) are what `scripts/run-phone.sh` and
  `run-tablet.sh` use to let a person try the app by hand, in containers of
  their own.
- **Two devices**, chosen with `--device`: `phone` (the Pixel 3 above; suites in
  `suites/`, coordinates in `resources/keywords.resource`) and `tablet` (a Pixel
  Tablet, 2560×1600 @ 320 dpi = 1280×800 dp; suites in `suites-tablet/`,
  coordinates in `resources/tablet.resource`). The tablet's AVD is made from the
  same system image by `emulator-entry.sh` on first start, so it costs no image
  rebuild. They are separate containers and can be up together.

A tablet's natural orientation is landscape, so `Set Orientation` is relative to
the device (`portrait` is rotation 1 there, 0 on the phone), and the tablet
suites start in landscape. Tablets are wider than the app's 600-point limit for
the phone layout either way up, so they get the desktop's two panels; the
tablet suites check that split, its behaviour when the tablet turns or the
window shrinks (`Set Display Size` stands in for split-screen), and the
tablet-sized keyboard and tutorial.

## How the tests see and touch the app

The UI is drawn by egui, so there is no accessibility tree (same as the desktop
suite). `resources/AndroidLibrary.py`:

- **acts** with `adb shell input` (tap, swipe, long-press, key events, text) and
  `am start` intents — including the *real* entry points a user has: "Open with"
  (VIEW) and the share sheet (SEND);
- **sees** with `screencap`, read by Tesseract OCR (light-on-dark text is
  inverted first) and pixel probes;
- **checks logcat**: every test teardown fails the test if the app crashed, hit
  a Rust panic, or died — the UI can look fine while a background thread has
  panicked.

Documents are loaded by intent (`Share Text To App`, `Open File In App`), which
needs no file picker; the system pickers themselves are covered in
`suites/pickers`, driving Android's own picker UI with `uiautomator`.

## Why not Appium

Tried on 2026-09-20, twice (Appium 2.19, then Appium 3.7 with the UiAutomator2 8.7
driver on Node 22; both attached to the running emulator): it connects fine, and the
result is identical on both. Its whole view of the app is **four empty nodes** (`FrameLayout > LinearLayout > FrameLayout > View`). None of "Run",
"Source", "Results", "Parsed in" or a document's values could be found, by text or
by content description. egui paints one OpenGL surface, so there is no accessibility
tree for Appium's locators (or TalkBack) to read; Appium would fall back to the same
coordinates and screenshots used here, on top of Node, a server, a driver and
instrumentation APKs installed on the device. It does drive *system* UI well, but
`uiautomator` already covers that (`suites/pickers`).

What would change this is the app exposing an accessibility tree: egui builds an
AccessKit tree, but the `accesskit_winit` pinned by eframe 0.36 has no Android
adapter, so it would take a bridge (egui's tree -> a Kotlin `AccessibilityNodeProvider`
over JNI). That would also give TalkBack users a usable app, which is the reason to
do it; the tests getting stable element locators would be a side effect.

## Gotchas learnt the hard way

- OCR reads big, high-contrast text well and small or tinted text badly.
  Assert on the status line and tree rows, not on decorative text; when an
  assertion fails, look at the saved screenshot before suspecting the app.
- `input text` types ASCII through the *key event* path (like a hardware
  keyboard). Text arriving from a real soft keyboard takes a different path
  (`ImeView`); `suites/keyboard` covers both.
- A swipe that starts at the screen's edge is Android's **back gesture**, not a
  drag inside the app: it closed the app and let the next taps open the
  launcher. Start drags a little inside the edge.
- Animations are switched off at install time; a tap right after a launch can
  still land before the first frame, so the launch keywords wait for the UI.
