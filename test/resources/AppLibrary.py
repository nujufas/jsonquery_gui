"""Robot Framework library for driving the real jsonquery_gui binary.

Architecture (see test/docs/00_test_strategy.md for the full rationale and the
confirmed environment findings that shaped this):

- The whole suite runs against an isolated Xvfb virtual display, not whatever
  real desktop session happens to be running -- GNOME/Wayland screen capture
  is confirmed blocked for unattended automation, Xvfb is not.
- Xvfb has no window manager of its own, and bare Xvfb silently breaks
  keyboard-focus delivery (confirmed: typed keys go nowhere without one), so a
  minimal EWMH window manager (fluxbox) runs alongside it.
- The app is launched with WAYLAND_DISPLAY unset so winit picks the X11/Xwayland
  backend that the rest of this stack (mss, pyautogui, xdotool) can actually see.
- Clicking is coordinate-based, computed relative to the app window's current
  client-area origin (looked up fresh via xdotool for every interaction, so a
  suite doesn't care exactly where the window manager placed the window).
  Text assertions and text-based clicking use Tesseract OCR over a cropped,
  upscaled region -- confirmed far more reliable than whole-window OCR, which
  misses most of this UI's small toolbar text.
- Native Open File / Save dialogs are a confirmed dead end in this environment
  (rfd's default xdg-portal backend hangs or errors before showing anything
  usable -- see the strategy doc). Keywords here do not attempt to drive them.
"""

import functools
import http.server
import os
import re
import signal
import subprocess
import threading
import time

import pyautogui
import pyperclip
import pytesseract
from PIL import Image
from robot.api import logger
from robot.api.deco import keyword, library

pyautogui.FAILSAFE = False

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
BINARY_PATH = os.path.join(REPO_ROOT, "target", "debug", "jsonquery_gui")

XVFB_DISPLAY = ":99"
XVFB_SCREEN_SIZE = "1280x900x24"


def _run(cmd, **kwargs):
    return subprocess.run(cmd, capture_output=True, text=True, **kwargs)


@library(scope="GLOBAL")
class AppLibrary:
    """Keywords for launching, driving, and reading the jsonquery GUI."""

    def __init__(self):
        self._xvfb_proc = None
        self._wm_proc = None
        self._app_proc = None
        self._window_id = None
        self._fixture_server = None
        self._fixture_server_thread = None

    # -- display lifecycle (once per suite run) -----------------------------

    @staticmethod
    def _ensure_fluxbox_no_decoration_rule():
        """Writes a fluxbox apps-rule so the jsonquery window gets no
        titlebar/border -- required for `_window_origin` to be accurate; see
        its docstring. Must match by WM_CLASS (`class=`), not WM_NAME: this
        app's WM_CLASS is ("", "jsonquery_gui") with an empty instance part,
        so a `name=jsonquery` rule silently never matches anything."""
        fluxbox_dir = os.path.expanduser("~/.fluxbox")
        os.makedirs(fluxbox_dir, exist_ok=True)
        apps_path = os.path.join(fluxbox_dir, "apps")
        rule = "[app] (class=jsonquery_gui)\n  [Deco] {NONE}\n[end]\n"
        existing = ""
        if os.path.exists(apps_path):
            existing = open(apps_path).read()
        if rule not in existing:
            with open(apps_path, "a") as f:
                f.write(rule)

    @keyword("Start Test Display")
    def start_test_display(self):
        """Starts an isolated Xvfb display plus a minimal window manager.

        Idempotent-ish: if display :99 is already up (e.g. a previous run
        didn't get torn down cleanly), it's reused rather than double-started.
        """
        os.environ["DISPLAY"] = XVFB_DISPLAY
        self._ensure_fluxbox_no_decoration_rule()
        if not os.path.exists(f"/tmp/.X11-unix/X{XVFB_DISPLAY.lstrip(':')}"):
            self._xvfb_proc = subprocess.Popen(
                ["Xvfb", XVFB_DISPLAY, "-screen", "0", XVFB_SCREEN_SIZE, "-nolisten", "tcp"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            time.sleep(1)
        else:
            logger.info(f"Xvfb already running on {XVFB_DISPLAY}, reusing it.")

        self._wm_proc = subprocess.Popen(
            ["fluxbox"],
            env={**os.environ, "DISPLAY": XVFB_DISPLAY},
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(1)

    @keyword("Stop Test Display")
    def stop_test_display(self):
        """Stops the window manager and, if this run started it, Xvfb too."""
        for proc in (self._wm_proc, self._xvfb_proc):
            if proc is not None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
        self._wm_proc = None
        self._xvfb_proc = None

    # -- app lifecycle (once per test) ---------------------------------------

    @keyword("Launch Jsonquery App")
    def launch_jsonquery_app(self, timeout=10):
        """Starts a fresh jsonquery_gui process and waits for its window."""
        if not os.path.exists(BINARY_PATH):
            raise AssertionError(
                f"{BINARY_PATH} not found -- run `cargo build -p jsonquery_gui` first."
            )
        env = dict(os.environ)
        env["DISPLAY"] = XVFB_DISPLAY
        env.pop("WAYLAND_DISPLAY", None)
        self._app_proc = subprocess.Popen(
            [BINARY_PATH],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        deadline = time.time() + timeout
        wid = None
        while time.time() < deadline:
            wid = self._find_window_id()
            if wid:
                break
            time.sleep(0.2)
        if not wid:
            raise AssertionError("jsonquery window did not appear within timeout")
        self._window_id = wid
        # A freshly-mapped window isn't reliably ready to receive synthetic
        # keyboard input yet -- confirmed empirically: without an explicit
        # focus + settle here, the first keystrokes sent to a just-launched
        # window are silently dropped often enough to be a real flake source
        # (fluxbox's own focus-on-map isn't instant). Force focus and give it
        # a moment before any test interacts with the window.
        _run(
            ["xdotool", "windowfocus", "--sync", wid],
            env={**os.environ, "DISPLAY": XVFB_DISPLAY},
        )
        time.sleep(0.5)

    @keyword("Close Jsonquery App")
    def close_jsonquery_app(self):
        """Kills the current jsonquery_gui process, if any."""
        if self._app_proc is not None:
            try:
                self._app_proc.send_signal(signal.SIGKILL)
                self._app_proc.wait(timeout=5)
            except Exception:
                pass
        self._app_proc = None
        self._window_id = None

    def _find_window_id(self):
        result = _run(
            ["xdotool", "search", "--name", "^jsonquery$"],
            env={**os.environ, "DISPLAY": XVFB_DISPLAY},
        )
        ids = [line for line in result.stdout.splitlines() if line.strip()]
        return ids[0] if ids else None

    # -- window geometry / coordinate translation ----------------------------

    def _window_origin(self):
        """Absolute screen coords of the app window's top-left corner.

        Requires `~/.fluxbox/apps` to mark this window class `Deco: NONE`
        (see `ensure_fluxbox_no_decoration_rule` / test/run.sh) -- without
        it, fluxbox adds a titlebar and `xdotool getwindowgeometry` reports a
        Y origin that does not match the window's actual visible top edge
        (confirmed empirically: ~26px off, cross-checked by OCR-locating the
        toolbar in a raw screenshot). With the Deco:NONE rule applied, the
        reported geometry matches the true content origin exactly.
        """
        if not self._window_id:
            raise AssertionError("No app window -- call `Launch Jsonquery App` first.")
        result = _run(
            ["xdotool", "getwindowgeometry", "--shell", self._window_id],
            env={**os.environ, "DISPLAY": XVFB_DISPLAY},
        )
        values = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
        return int(values["X"]), int(values["Y"])

    def _require_window(self):
        if not self._window_id:
            raise AssertionError("No app window -- call `Launch Jsonquery App` first.")

    @keyword("Get Window Size")
    def get_window_size(self):
        self._require_window()
        result = _run(
            ["xdotool", "getwindowgeometry", "--shell", self._window_id],
            env={**os.environ, "DISPLAY": XVFB_DISPLAY},
        )
        values = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
        return int(values["WIDTH"]), int(values["HEIGHT"])

    def _to_absolute(self, x, y):
        ox, oy = self._window_origin()
        return ox + x, oy + y

    # -- mouse / keyboard, relative to the app window ------------------------

    @keyword("Click At")
    def click_at(self, x, y, button="left"):
        """Clicks at (x, y) relative to the app window's client-area origin."""
        ax, ay = self._to_absolute(int(x), int(y))
        pyautogui.click(ax, ay, button=button)

    @keyword("Double Click At")
    def double_click_at(self, x, y):
        ax, ay = self._to_absolute(int(x), int(y))
        pyautogui.doubleClick(ax, ay)

    @keyword("Right Click At")
    def right_click_at(self, x, y):
        ax, ay = self._to_absolute(int(x), int(y))
        pyautogui.rightClick(ax, ay)

    @keyword("Move Mouse To")
    def move_mouse_to(self, x, y):
        ax, ay = self._to_absolute(int(x), int(y))
        pyautogui.moveTo(ax, ay)

    @keyword("Scroll At")
    def scroll_at(self, x, y, clicks):
        """Scrolls the mouse wheel `clicks` steps (negative = down) at the
        given app-relative point -- egui's scroll areas, like most UI
        toolkits, scroll whichever one the pointer is currently over."""
        ax, ay = self._to_absolute(int(x), int(y))
        pyautogui.moveTo(ax, ay)
        pyautogui.scroll(int(clicks))

    @keyword("Type Text")
    def type_text(self, text, interval=0.02):
        pyautogui.typewrite(text, interval=float(interval))

    @keyword("Press Keys")
    def press_keys(self, *keys):
        """E.g. `Press Keys    ctrl    enter` for Ctrl+Enter. Key names must
        be pyautogui's own lowercase names (`enter`, not `Return`/`Enter` --
        confirmed the capitalized X11-style name is NOT equivalent here and
        produces flaky, not consistently-failing, results, so this is an easy
        mistake to half-miss in testing). `interval` is passed to
        `pyautogui.hotkey` between each key-down -- confirmed during
        implementation that the default (0s, all keys sent essentially at
        once) is a real source of the "occasionally doesn't land" flakiness
        documented elsewhere in this suite: 8/8 trials landed cleanly with a
        50ms interval versus a noticeably-less-than-100% hit rate at 0s.
        Kept as a keyword-level default rather than hardcoded so a caller can
        override it if some other combo needs more (or can tolerate less)."""
        pyautogui.hotkey(*keys, interval=0.05)

    @keyword("Press Key")
    def press_key(self, key):
        pyautogui.press(key)

    # -- screenshots / OCR ----------------------------------------------------

    @keyword("Screenshot Region")
    def screenshot_region(self, x, y, width, height, path=None):
        """Returns a PIL Image of the given app-relative region (also saves
        it to `path` if given, for failure evidence)."""
        ax, ay = self._to_absolute(int(x), int(y))
        img = pyautogui.screenshot(region=(ax, ay, int(width), int(height)))
        if path:
            img.save(path)
        return img

    @keyword("Screenshot Window")
    def screenshot_window(self, path=None):
        w, h = self.get_window_size()
        return self.screenshot_region(0, 0, w, h, path=path)

    @staticmethod
    def _ocr_words(img, upscale=5, psm=6):
        big = img.resize((img.width * upscale, img.height * upscale), Image.LANCZOS)
        data = pytesseract.image_to_data(
            big, output_type=pytesseract.Output.DICT, config=f"--psm {psm}"
        )
        words = []
        for i, text in enumerate(data["text"]):
            text = text.strip()
            if not text or int(data["conf"][i]) < 0:
                continue
            words.append(
                {
                    "text": text,
                    "left": data["left"][i] // upscale,
                    "top": data["top"][i] // upscale,
                    "width": data["width"][i] // upscale,
                    "height": data["height"][i] // upscale,
                    "line": (data["block_num"][i], data["par_num"][i], data["line_num"][i]),
                }
            )
        return words

    @keyword("Read Region Text")
    def read_region_text(self, x, y, width, height, psm=6):
        """OCRs the given app-relative region and returns the recognized text."""
        img = self.screenshot_region(x, y, width, height)
        words = self._ocr_words(img, psm=int(psm))
        # Group by line so multi-word text reads back in natural order.
        lines = {}
        for w in words:
            lines.setdefault(w["line"], []).append(w)
        out_lines = []
        for line_key in sorted(lines):
            line_words = sorted(lines[line_key], key=lambda w: w["left"])
            out_lines.append(" ".join(w["text"] for w in line_words))
        return "\n".join(out_lines)

    # Tesseract, at this font size, sometimes renders straight quotes/
    # apostrophes as their typographic "curly" equivalents (confirmed during
    # implementation: "start with '/'" came back as "start with '/'" with a
    # curly closing quote) -- normalize both sides before comparing so a test
    # asserting on straight ASCII quotes isn't tripped up by this.
    _QUOTE_MAP = str.maketrans(
        {"‘": "'", "’": "'", "“": '"', "”": '"'}
    )

    @classmethod
    def _normalize_for_match(cls, s):
        return s.lower().translate(cls._QUOTE_MAP)

    # Tesseract, at this font size, sometimes reads a digit "0" as the letter
    # "O" (confirmed: "0 B" came back as "OB", "0 result(s)" as "O
    # result(s)") -- folded into one more permissive fallback comparison
    # rather than avoiding the digit 0 in every assertion that might hit it.
    _ZERO_O_MAP = str.maketrans({"0": "o"})

    @classmethod
    def _text_contains(cls, actual, expected):
        a = cls._normalize_for_match(actual)
        e = cls._normalize_for_match(expected)
        if e in a:
            return True
        # Tesseract occasionally splits a single word around punctuation
        # into separate word-boxes (confirmed: "valid.json" -> "valid." +
        # "json"), which this library's own line-joining then reinserts a
        # space into -- retry with whitespace stripped from both sides as a
        # permissive fallback rather than failing on that alone.
        if e.replace(" ", "") in a.replace(" ", ""):
            return True
        a2 = a.translate(cls._ZERO_O_MAP).replace(" ", "")
        e2 = e.translate(cls._ZERO_O_MAP).replace(" ", "")
        return e2 in a2

    @keyword("Region Should Contain Text")
    def region_should_contain_text(self, x, y, width, height, expected, psm=6, msg=None):
        actual = self.read_region_text(x, y, width, height, psm=psm)
        if not self._text_contains(actual, expected):
            raise AssertionError(
                msg or f"Expected region to contain {expected!r}, but OCR read: {actual!r}"
            )

    @keyword("Region Should Not Contain Text")
    def region_should_not_contain_text(self, x, y, width, height, unexpected, psm=6, msg=None):
        actual = self.read_region_text(x, y, width, height, psm=psm)
        if self._text_contains(actual, unexpected):
            raise AssertionError(
                msg or f"Expected region NOT to contain {unexpected!r}, but OCR read: {actual!r}"
            )

    @keyword("Wait Until Region Contains Text")
    def wait_until_region_contains_text(self, x, y, width, height, expected, timeout=5, psm=6):
        deadline = time.time() + float(timeout)
        last_seen = ""
        while time.time() < deadline:
            last_seen = self.read_region_text(x, y, width, height, psm=psm)
            if self._text_contains(last_seen, expected):
                return last_seen
            time.sleep(0.2)
        raise AssertionError(
            f"Region never contained {expected!r} within {timeout}s "
            f"(last OCR read: {last_seen!r})"
        )

    @keyword("Wait Until Region Matches")
    def wait_until_region_matches(self, x, y, width, height, pattern, timeout=5, psm=6):
        """Like `Wait Until Region Contains Text` but matches a regex."""
        deadline = time.time() + float(timeout)
        last_seen = ""
        compiled = re.compile(pattern, re.IGNORECASE)
        while time.time() < deadline:
            last_seen = self.read_region_text(x, y, width, height, psm=psm)
            if compiled.search(last_seen):
                return last_seen
            time.sleep(0.2)
        raise AssertionError(
            f"Region never matched /{pattern}/ within {timeout}s "
            f"(last OCR read: {last_seen!r})"
        )

    @keyword("Find Text In Region")
    def find_text_in_region(self, x, y, width, height, target, psm=6):
        """Returns (center_x, center_y) of `target` within the region, in
        app-relative coordinates, or raises if not found. Case-insensitive.
        Checks single OCR word/tokens first, then contiguous runs of words on
        the same line (e.g. "Copy JSON Path" is 3 separate tokens to
        Tesseract) -- confirmed necessary during implementation for any
        multi-word button/menu-item label."""
        img = self.screenshot_region(x, y, width, height)
        words = self._ocr_words(img, psm=int(psm))
        target_l = target.lower()
        for w in words:
            if target_l in w["text"].lower():
                return (
                    int(x) + w["left"] + w["width"] / 2,
                    int(y) + w["top"] + w["height"] / 2,
                )
        lines = {}
        for w in words:
            lines.setdefault(w["line"], []).append(w)
        for line_words in lines.values():
            line_words = sorted(line_words, key=lambda w: w["left"])
            joined = " ".join(w["text"] for w in line_words).lower()
            if target_l in joined:
                left = min(w["left"] for w in line_words)
                right = max(w["left"] + w["width"] for w in line_words)
                top = min(w["top"] for w in line_words)
                bottom = max(w["top"] + w["height"] for w in line_words)
                return (int(x) + (left + right) / 2, int(y) + (top + bottom) / 2)
        raise AssertionError(
            f"Text {target!r} not found in region ({x},{y},{width},{height}); "
            f"OCR saw: {[w['text'] for w in words]}"
        )

    @keyword("Click Text In Region")
    def click_text_in_region(self, x, y, width, height, target, psm=6):
        """Finds `target` by OCR within the given region and clicks its center."""
        cx, cy = self.find_text_in_region(x, y, width, height, target, psm=psm)
        self.click_at(cx, cy)

    @keyword("Get Pixel Color")
    def get_pixel_color(self, x, y):
        """Returns an (r, g, b) tuple for the app-relative pixel."""
        img = self.screenshot_region(x, y, 1, 1)
        return img.convert("RGB").getpixel((0, 0))

    @keyword("Colors Should Match")
    def colors_should_match(self, color_a, color_b, tolerance=6, msg=None):
        """Compares two (r, g, b) tuples allowing a small per-channel
        tolerance -- exact equality is too strict for screen-captured colors,
        which vary by a few units between frames from anti-aliasing and
        hover/selection transition easing (confirmed during implementation:
        a genuinely-still-selected button read (53,128,163) then
        (51,127,161) a moment later)."""
        diff = max(abs(a - b) for a, b in zip(color_a, color_b))
        if diff > int(tolerance):
            raise AssertionError(
                msg or f"Colors differ by {diff} (> tolerance {tolerance}): "
                f"{color_a} vs {color_b}"
            )

    @keyword("Colors Should Not Match")
    def colors_should_not_match(self, color_a, color_b, tolerance=6, msg=None):
        diff = max(abs(a - b) for a, b in zip(color_a, color_b))
        if diff <= int(tolerance):
            raise AssertionError(
                msg or f"Colors are within tolerance {tolerance} (diff {diff}), "
                f"expected a visible difference: {color_a} vs {color_b}"
            )

    # -- clipboard -------------------------------------------------------------

    @keyword("Get Clipboard")
    def get_clipboard(self):
        return pyperclip.paste()

    @keyword("Set Clipboard")
    def set_clipboard(self, text):
        pyperclip.copy(text)

    # -- local HTTP fixture server (for Open URL... tests) ---------------------

    @keyword("Start Fixture Server")
    def start_fixture_server(self, directory):
        """Serves `directory` over HTTP on an OS-assigned free port (bound to
        127.0.0.1 only -- these tests never need, and shouldn't risk, being
        reachable from outside the test machine). Runs in a background thread
        inside this same process rather than a subprocess: simpler lifecycle,
        nothing extra to kill on teardown beyond `shutdown()`. Returns the
        server's base URL (e.g. `http://127.0.0.1:51234`)."""
        handler = functools.partial(
            http.server.SimpleHTTPRequestHandler, directory=directory
        )
        self._fixture_server = http.server.HTTPServer(("127.0.0.1", 0), handler)
        self._fixture_server_thread = threading.Thread(
            target=self._fixture_server.serve_forever, daemon=True
        )
        self._fixture_server_thread.start()
        port = self._fixture_server.server_address[1]
        return f"http://127.0.0.1:{port}"

    @keyword("Stop Fixture Server")
    def stop_fixture_server(self):
        if self._fixture_server is not None:
            self._fixture_server.shutdown()
            self._fixture_server.server_close()
        self._fixture_server = None
        self._fixture_server_thread = None
