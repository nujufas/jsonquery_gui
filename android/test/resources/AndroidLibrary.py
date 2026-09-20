"""Robot Framework library that drives the jsonquery Android app on an emulator.

Input goes in through `adb shell input` and `am start` intents. Output is read
back the way the desktop suite does it (test/resources/AppLibrary.py): the app
is drawn by egui, so it has no accessibility tree, and the only eyes are
screenshots -- read with Tesseract OCR and probed pixel by pixel.

Two things are Android-specific:

* Screens are big (1080x2160 on the test phone) and text is set large, so OCR
  runs on the whole screen without upscaling and light-on-dark text is
  inverted first (Tesseract reads dark-on-light far better).
* logcat is a second witness. The app logs panics to it (tag `jsonquery`),
  and Android logs every crash of the process, so `Application Should Not
  Have Crashed` turns a native crash the UI might have hidden into a failure.
"""
import io
import re
import subprocess
import time
from pathlib import Path

import pytesseract
from PIL import Image, ImageOps
from robot.api import logger
from robot.api.deco import keyword, library
from robot.libraries.BuiltIn import BuiltIn

ACTIVITY = "io.github.nujufas.jsonquery.MainActivity"


def _adb(*args, timeout=60, binary=False, check=True):
    result = subprocess.run(
        ["adb", *args], capture_output=True, timeout=timeout, text=not binary
    )
    if check and result.returncode != 0:
        stderr = result.stderr if not binary else result.stderr.decode(errors="replace")
        raise AssertionError(f"adb {' '.join(args)} failed ({result.returncode}): {stderr}")
    return result.stdout


def _sh_quote(text):
    """`text` as one word for the device's shell."""
    return "'" + text.replace("'", "'\\''") + "'"


@library(scope="SUITE", auto_keywords=False)
class AndroidLibrary:
    def __init__(self, package="io.github.nujufas.jsonquery.debug"):
        self.package = package
        self._shots = 0
        self._current_test = "suite"

    # ------------------------------------------------------------------
    # Device and app lifecycle
    # ------------------------------------------------------------------

    @keyword("Wait For Device")
    def wait_for_device(self, timeout=240):
        """Blocks until the emulator has finished booting."""
        _adb("wait-for-device", timeout=int(timeout))
        deadline = time.time() + int(timeout)
        while time.time() < deadline:
            booted = _adb("shell", "getprop", "sys.boot_completed", check=False).strip()
            if booted == "1":
                # Let the launcher settle so it doesn't steal the first launch.
                time.sleep(3)
                return
            time.sleep(2)
        raise AssertionError("emulator did not finish booting")

    @keyword("Install App")
    def install_app(self, apk):
        """(Re)installs the APK, keeping nothing of an earlier install."""
        _adb("uninstall", self.package, check=False)
        _adb("install", "-r", str(apk), timeout=180)
        # A picker/notification permission dialog must never be in the way.
        _adb("shell", "settings", "put", "global", "window_animation_scale", "0", check=False)
        _adb("shell", "settings", "put", "global", "transition_animation_scale", "0", check=False)
        _adb("shell", "settings", "put", "global", "animator_duration_scale", "0", check=False)

    @keyword("Launch App")
    def launch_app(self):
        """Starts the app from scratch (any running instance is killed first)."""
        self.force_stop_app()
        _adb("logcat", "-c")
        _adb("shell", "am", "start", "-W", "-n", f"{self.package}/{ACTIVITY}")
        self._wait_for_first_frame()

    @keyword("Force Stop App")
    def force_stop_app(self):
        _adb("shell", "am", "force-stop", self.package)

    @keyword("Launch App With Text")
    def launch_app_with_text(self, text):
        """Starts the app the way the share sheet does: a SEND intent with `text`."""
        self.force_stop_app()
        _adb("logcat", "-c")
        _adb(
            "shell",
            "am start -W"
            f" -n {self.package}/{ACTIVITY}"
            " -a android.intent.action.SEND -t text/plain"
            f" --es android.intent.extra.TEXT {_sh_quote(text)}",
        )
        self._wait_for_first_frame()

    @keyword("Share Text To App")
    def share_text_to_app(self, text):
        """Sends `text` to the *running* app, as the share sheet would."""
        _adb(
            "shell",
            "am start"
            f" -n {self.package}/{ACTIVITY}"
            " -a android.intent.action.SEND -t text/plain"
            f" --es android.intent.extra.TEXT {_sh_quote(text)}",
        )

    @keyword("Push Fixture")
    def push_fixture(self, local_path, name=None):
        """Copies a file into the app's own external files directory and
        returns its path on the device (which the app can read without any
        permission)."""
        name = name or Path(local_path).name
        directory = f"/sdcard/Android/data/{self.package}/files"
        _adb("shell", "mkdir", "-p", directory)
        _adb("push", str(local_path), f"{directory}/{name}")
        return f"{directory}/{name}"

    @keyword("Open File In App")
    def open_file_in_app(self, device_path, restart=False):
        """Sends an "Open with" (VIEW) intent for a file on the device."""
        if restart:
            self.force_stop_app()
            _adb("logcat", "-c")
        _adb(
            "shell",
            "am start -W"
            f" -n {self.package}/{ACTIVITY}"
            f" -a android.intent.action.VIEW -d file://{device_path} -t application/json",
        )
        if restart:
            self._wait_for_first_frame()

    def _wait_for_first_frame(self, timeout=40):
        """The app's toolbar/hint is the first thing egui draws; wait for it
        rather than sleeping a fixed time."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            # "Source" and "Results" are high-contrast text on screen in every state
            # (the phone's tab bar, the wide layout's panel headings); the toolbar's
            # own title is dim and OCRs badly. Either will do: a layout bug that hides
            # one panel should fail the test about layout, not every test's setup.
            text = self._normalize(self.read_screen_text(psm=11))
            if "results" in text or "source" in text:
                return
            time.sleep(1)
        self.screenshot("no-first-frame")
        raise AssertionError("the app did not draw its first frame")

    def _natural_orientation(self):
        """`portrait` for a phone, `landscape` for a tablet: the way the panel is
        built, which is rotation 0."""
        out = _adb("shell", "wm", "size")
        match = re.search(r"Physical size:\s*(\d+)x(\d+)", out)
        if not match:
            raise AssertionError(f"cannot read the screen size from `wm size`: {out!r}")
        width, height = int(match.group(1)), int(match.group(2))
        return "landscape" if width > height else "portrait"

    @keyword("Set Orientation")
    def set_orientation(self, orientation):
        """`portrait` or `landscape`, whichever way up that is on this device
        (a tablet's natural rotation is landscape, a phone's portrait)."""
        wanted = orientation.lower()
        if wanted not in ("portrait", "landscape"):
            raise AssertionError(f"orientation must be portrait or landscape, not {orientation!r}")
        rotation = 0 if wanted == self._natural_orientation() else 1
        _adb("shell", "settings", "put", "system", "accelerometer_rotation", "0")
        _adb("shell", "settings", "put", "system", "user_rotation", str(rotation))
        time.sleep(2)

    @keyword("Set Display Size")
    def set_display_size(self, width, height):
        """Pretends the display is `width`x`height` pixels (`wm size`): the app's
        window shrinks the way it does in split-screen or a freeform window.
        Undo it with `Reset Display Size`."""
        _adb("shell", "wm", "size", f"{int(width)}x{int(height)}")
        time.sleep(2)

    @keyword("Reset Display Size")
    def reset_display_size(self):
        _adb("shell", "wm", "size", "reset")
        time.sleep(2)

    @keyword("Get Screen Size")
    def get_screen_size(self):
        """[width, height] in pixels of the screen as it is now oriented."""
        image = self._capture()
        return [image.width, image.height]

    # ------------------------------------------------------------------
    # Input
    # ------------------------------------------------------------------

    @keyword("Tap At")
    def tap_at(self, x, y):
        _adb("shell", "input", "tap", str(int(float(x))), str(int(float(y))))

    @keyword("Long Press At")
    def long_press_at(self, x, y, duration_ms=900):
        x, y = str(int(float(x))), str(int(float(y)))
        _adb("shell", "input", "swipe", x, y, x, y, str(int(duration_ms)))

    @keyword("Swipe")
    def swipe(self, x1, y1, x2, y2, duration_ms=300):
        coords = [str(int(float(v))) for v in (x1, y1, x2, y2)]
        _adb("shell", "input", "swipe", *coords, str(int(duration_ms)))

    @keyword("Type Text")
    def type_text(self, text):
        """Types ASCII `text` as key events (which reach the app the same way a
        hardware keyboard's do)."""
        if not text.isascii():
            raise AssertionError(f"`input text` cannot type non-ASCII text: {text!r}")
        _adb("shell", f"input text {_sh_quote(text.replace(' ', '%s'))}")

    @keyword("Press Key")
    def press_key(self, keycode):
        """`ENTER`, `DEL`, `BACK`, `DPAD_LEFT`, ... (names of KEYCODE_*)."""
        name = keycode if keycode.startswith("KEYCODE_") else f"KEYCODE_{keycode}"
        _adb("shell", "input", "keyevent", name)

    @keyword("Tap Text")
    def tap_text(self, text, region=None, occurrence=1, psm=11):
        """Finds `text` on screen with OCR and taps the middle of it."""
        x, y = self.find_text_on_screen(text, region=region, occurrence=occurrence, psm=psm)
        self.tap_at(x, y)

    # ------------------------------------------------------------------
    # Seeing
    # ------------------------------------------------------------------

    def _capture(self):
        png = _adb("exec-out", "screencap", "-p", binary=True, timeout=30)
        return Image.open(io.BytesIO(png)).convert("RGB")

    @staticmethod
    def _slug(name):
        return re.sub(r"[^A-Za-z0-9]+", "-", name).strip("-")[:60] or "shot"

    def _save(self, image, label):
        test = self._slug(BuiltIn().get_variable_value("${TEST NAME}", "suite"))
        if test != self._current_test:
            self._current_test, self._shots = test, 0
        self._shots += 1
        outdir = Path(BuiltIn().get_variable_value("${OUTPUT DIR}", ".")) / "screenshots" / test
        outdir.mkdir(parents=True, exist_ok=True)
        path = outdir / f"{self._shots:03d}-{self._slug(label)}.png"
        image.save(path)
        rel = path.relative_to(Path(BuiltIn().get_variable_value("${OUTPUT DIR}", ".")))
        logger.info(
            f'<a href="{rel}"><img src="{rel}" width="240"></a> {label}', html=True
        )
        return path

    @keyword("Screenshot")
    def screenshot(self, label="screen", region=None, log=True):
        """Captures the screen (or a `[x, y, w, h]` region of it), logs it, and returns the PIL image."""
        image = self._capture()
        if region:
            x, y, w, h = [int(v) for v in region]
            image = image.crop((x, y, x + w, y + h))
        if log:
            self._save(image, label)
        return image

    @keyword("Save Final Screenshot")
    def save_final_screenshot(self):
        """Every test ends with evidence of what the screen looked like."""
        try:
            status = BuiltIn().get_variable_value("${TEST STATUS}", "")
            self._save(self._capture(), f"final-{status.lower()}")
        except Exception as error:  # never fail a teardown over a screenshot
            logger.warn(f"could not take the final screenshot: {error}")

    @staticmethod
    def _prepare_for_ocr(image):
        """Black text on white, by thresholding. Tesseract reads dark-on-light far
        better than the reverse, and a plain invert leaves grey buttons as
        grey-on-grey (read back as `En) Ge) Ee`); a threshold turns every
        background -- the theme's fill, a button's fill, a selection chip's -- white
        and every glyph, dim ones included, black. Which side is the text is
        decided by the theme: mostly-dark screens have light text."""
        gray = image.convert("L")
        if sum(gray.resize((32, 32)).getdata()) / 1024 < 128:
            return gray.point(lambda v: 0 if v > 105 else 255)
        return gray.point(lambda v: 0 if v < 150 else 255)

    def _words(self, image, psm):
        data = pytesseract.image_to_data(
            self._prepare_for_ocr(image), output_type=pytesseract.Output.DICT,
            config=f"--psm {int(psm)}",
        )
        words = []
        for i, text in enumerate(data["text"]):
            text = text.strip()
            if text and int(float(data["conf"][i])) >= 0:
                words.append({
                    "text": text, "left": data["left"][i], "top": data["top"][i],
                    "width": data["width"][i], "height": data["height"][i],
                    "line": (data["block_num"][i], data["par_num"][i], data["line_num"][i]),
                })
        return words

    @keyword("Read Screen Text")
    def read_screen_text(self, region=None, psm=None, log=True):
        """OCR of the screen (or a `[x, y, w, h]` region), lines in reading order."""
        image = self.screenshot("ocr", region=region, log=log)
        lines = {}
        # A whole egui screen is sparse text of many sizes (Tesseract's "sparse text"
        # mode, 11, reads it; the block mode, 6, drops buttons and tab bars); a
        # band of one or two lines reads best as a block.
        if psm is None:
            psm = 11 if region is None else 6
        for word in self._words(image, psm):
            lines.setdefault(word["line"], []).append(word)
        return "\n".join(
            " ".join(w["text"] for w in sorted(words, key=lambda w: w["left"]))
            for _, words in sorted(lines.items())
        )

    # Tesseract sometimes swaps curly for straight quotes, 0 for O, and drops
    # spaces; both sides are normalised before comparing.
    _QUOTES = str.maketrans({"‘": "'", "’": "'", "“": '"', "”": '"'})

    @classmethod
    def _normalize(cls, text):
        return text.lower().translate(cls._QUOTES)

    @classmethod
    def _contains(cls, actual, expected):
        a, e = cls._normalize(actual), cls._normalize(expected)
        if e in a:
            return True
        squash = lambda s: re.sub(r"\s+", "", s.replace("o", "0"))
        return squash(e) in squash(a)

    @keyword("Screen Should Contain Text")
    def screen_should_contain_text(self, expected, region=None, psm=None, msg=None):
        actual = self.read_screen_text(region, psm)
        if not self._contains(actual, expected):
            raise AssertionError(msg or f"expected {expected!r} on screen, OCR read:\n{actual}")

    @keyword("Screen Should Not Contain Text")
    def screen_should_not_contain_text(self, unexpected, region=None, psm=None, msg=None):
        actual = self.read_screen_text(region, psm)
        if self._contains(actual, unexpected):
            raise AssertionError(msg or f"did not expect {unexpected!r} on screen, OCR read:\n{actual}")

    @keyword("Wait Until Screen Contains Text")
    def wait_until_screen_contains_text(self, expected, region=None, timeout=15, psm=None):
        deadline, actual = time.time() + float(timeout), ""
        while time.time() < deadline:
            actual = self.read_screen_text(region, psm, log=False)
            if self._contains(actual, expected):
                self.screenshot(f"found-{expected}", region=region)
                return
            time.sleep(0.7)
        self.screenshot("timeout", region=region)
        raise AssertionError(f"{expected!r} not on screen after {timeout}s, OCR read:\n{actual}")

    @keyword("Wait Until Screen Matches")
    def wait_until_screen_matches(self, pattern, region=None, timeout=15, psm=None):
        """Like `Wait Until Screen Contains Text`, for a regular expression."""
        deadline, actual = time.time() + float(timeout), ""
        while time.time() < deadline:
            actual = self.read_screen_text(region, psm, log=False)
            if re.search(pattern, actual, re.I):
                self.screenshot(f"matched-{pattern}", region=region)
                return actual
            time.sleep(0.7)
        self.screenshot("timeout", region=region)
        raise AssertionError(f"/{pattern}/ not on screen after {timeout}s, OCR read:\n{actual}")

    @classmethod
    def _word_matches(cls, target, word):
        """Does the OCR'd `word` stand for the `target` word? Equal, or close (OCR
        misreads a letter or drops punctuation), or one contains the other when
        both are long enough to mean something -- never a one-letter fragment of
        an icon, which used to "match" a word like `tutorial` and get tapped."""
        import difflib
        word = re.sub(r"[^\w]", "", cls._normalize(word))
        target = re.sub(r"[^\w]", "", target)
        if not word or not target:
            return False
        if word == target:
            return True
        shorter, longer = sorted((word, target), key=len)
        # ("json" is inside "jsonpath", but is not it.)
        if len(shorter) >= 4 and shorter in longer and len(shorter) / len(longer) >= 0.7:
            return True
        return len(word) >= 3 and difflib.SequenceMatcher(None, target, word).ratio() >= 0.8

    @classmethod
    def _squashed_matches(cls, target, word):
        """Is the single OCR'd `word` the whole phrase run together ("Try it" read
        as "Tryit")? Much stricter than `_word_matches`: nearly equal, and about
        the same length -- "JSONPath" is not "Copy JSON Path", though it is most
        of it."""
        import difflib
        word = re.sub(r"[^\w]", "", cls._normalize(word))
        if not word or not target:
            return False
        if word == target:
            return True
        if min(len(word), len(target)) / max(len(word), len(target)) < 0.8:
            return False
        return difflib.SequenceMatcher(None, target, word).ratio() >= 0.85

    @keyword("Find Text On Screen")
    def find_text_on_screen(self, text, region=None, occurrence=1, psm=11):
        """[x, y] of the middle of the `occurrence`th place `text` is on screen
        (a phrase is matched word by word).

        The phrase is looked for word by word over the whole screen first; only
        if it is not there is a phrase OCR ran together into one word accepted.
        (Trying both at once let a word near the top of the screen, such as an
        engine chip, stand in for a menu item further down.)"""
        image = self.screenshot(f"find-{text}", region=region)
        ox, oy = (int(region[0]), int(region[1])) if region else (0, 0)
        words = self._words(image, psm)
        targets = self._normalize(text).split()
        squashed = "".join(re.sub(r"[^\w]", "", t) for t in targets)

        def windows(squash):
            size = 1 if squash else len(targets)
            for i in range(len(words) - size + 1):
                window = words[i:i + size]
                if squash:
                    if len(targets) > 1 and self._squashed_matches(squashed, window[0]["text"]):
                        yield window
                elif all(self._word_matches(t, w["text"]) for t, w in zip(targets, window)):
                    yield window

        for squash in (False, True):
            for found, window in enumerate(windows(squash), start=1):
                if found == int(occurrence):
                    left = min(w["left"] for w in window)
                    right = max(w["left"] + w["width"] for w in window)
                    top = min(w["top"] for w in window)
                    bottom = max(w["top"] + w["height"] for w in window)
                    return [ox + (left + right) // 2, oy + (top + bottom) // 2]
        raise AssertionError(
            f"{text!r} (occurrence {occurrence}) not found; OCR words: "
            + " ".join(w["text"] for w in words)
        )

    @keyword("Get Pixel Color")
    def get_pixel_color(self, x, y):
        """[r, g, b] at a screen position."""
        return list(self._capture().getpixel((int(x), int(y))))

    @keyword("Find First Different Pixel Below")
    def find_first_different_pixel_below(self, x, y_start=0, tolerance=8):
        """y of the first pixel, scanning down column `x` from `y_start`, that differs
        from the pixel at `y_start` by more than `tolerance` -- the top edge of
        whatever is drawn there (a button, say)."""
        image = self._capture()
        x, y_start = int(x), int(y_start)
        base = image.getpixel((x, y_start))
        for y in range(y_start, image.height):
            pixel = image.getpixel((x, y))
            if max(abs(a - b) for a, b in zip(pixel, base)) > int(tolerance):
                return y
        raise AssertionError(f"column {x} is one colour {base} all the way down")

    @keyword("Find Panel Divider")
    def find_panel_divider(self, y, x_from=0, x_to=None, step=20, rows=9, spacing=24):
        """x of the thin vertical line between two side-by-side panels, looked for
        on `rows` pixel rows centred on `y`, `spacing` px apart (pick a `y` inside
        the panels).

        A pixel counts when it differs by at least `step` from the pixels six
        either side of it *and those two are alike* (the edge of a text box has a
        different colour on each side, so it is not mistaken for the line), and the
        *same* x has to count on every row: a stroke of a letter is thin too, but
        it is not a line all the way down. Fails if there is no such line -- with
        tabs, or a panel that fills the screen, there is none."""
        image = self._capture()
        self._save(image, "divider-probe")
        pixels = image.load()
        apart = lambda a, b: max(abs(p - q) for p, q in zip(a, b))
        centre, half = int(y), int(rows) // 2
        ys = [centre + (i - half) * int(spacing) for i in range(int(rows))]
        ys = [v for v in ys if 0 <= v < image.height]
        last = min(int(x_to) if x_to else image.width, image.width - 6)
        for x in range(max(int(x_from), 6), last):
            if all(
                apart(pixels[x - 6, v], pixels[x + 6, v]) <= 6
                and apart(pixels[x, v], pixels[x - 6, v]) >= int(step)
                for v in ys
            ):
                return x
        raise AssertionError(
            f"no vertical line between x={x_from} and x={last} on rows {ys[0]}..{ys[-1]}"
        )

    @keyword("Find Last Pixel Of Color Above")
    def find_last_pixel_of_color_above(self, x, y_end, color, tolerance=20):
        """y of the lowest pixel in column `x` (scanning up from `y_end`) within
        `tolerance` of `color`."""
        image = self._capture()
        x = int(x)
        r, g, b = [int(c) for c in color]
        for y in range(min(int(y_end), image.height - 1), -1, -1):
            pr, pg, pb = image.getpixel((x, y))
            if abs(pr - r) <= int(tolerance) and abs(pg - g) <= int(tolerance) and abs(pb - b) <= int(tolerance):
                return y
        raise AssertionError(f"no pixel near {tuple(color)} in column {x}")

    @keyword("Region Should Contain Color")
    def region_should_contain_color(self, region, color, tolerance=12, msg=None):
        """Some pixel of the `[x, y, w, h]` region is within `tolerance` of `color`."""
        image = self.screenshot("color-probe", region=region)
        r, g, b = [int(c) for c in color]
        for pr, pg, pb in image.getdata():
            if abs(pr - r) <= tolerance and abs(pg - g) <= tolerance and abs(pb - b) <= tolerance:
                return
        raise AssertionError(msg or f"no pixel near {tuple(color)} in {region}")

    # ------------------------------------------------------------------
    # logcat
    # ------------------------------------------------------------------

    @keyword("Clear Logcat")
    def clear_logcat(self):
        _adb("logcat", "-c")

    @keyword("Get Logcat")
    def get_logcat(self):
        return _adb("logcat", "-d", timeout=60)

    @keyword("Application Should Not Have Crashed")
    def application_should_not_have_crashed(self):
        """Fails on a Java exception, a native crash, or a Rust panic of this app."""
        log = self.get_logcat()
        problems = [
            line for line in log.splitlines()
            if ("FATAL EXCEPTION" in line and self.package)
            or "Fatal signal" in line
            or re.search(r"jsonquery.*panic", line)
            or re.search(rf"Process: {re.escape(self.package)}", line)
        ]
        if problems:
            raise AssertionError("the app crashed:\n" + "\n".join(problems[:10]))
        if not self._app_is_running():
            raise AssertionError("the app is no longer running")

    def _app_is_running(self):
        return bool(_adb("shell", "pidof", self.package, check=False).strip())

    @keyword("Logcat Should Contain")
    def logcat_should_contain(self, expected):
        if expected not in self.get_logcat():
            raise AssertionError(f"{expected!r} not in logcat")

    @keyword("Get Device Page Size")
    def get_device_page_size(self):
        """The kernel's memory page size in bytes (16384 on a 16 KB device)."""
        return int(_adb("shell", "getconf", "PAGE_SIZE").strip())

    # ------------------------------------------------------------------
    # Store screenshots
    # ------------------------------------------------------------------

    STORE_DIR = "/work/android/play/metadata/android/en-US/images/phoneScreenshots"

    @keyword("Enable Clean Status Bar")
    def enable_clean_status_bar(self):
        """Android's demo mode: 12:00, full battery and signal, no notification
        icons -- what a store screenshot's status bar should look like."""
        for args in (
            ["settings", "put", "global", "sysui_demo_allowed", "1"],
        ):
            _adb("shell", *args, check=False)
        for command in (
            "enter",
            "clock -e hhmm 1200",
            "battery -e level 100 -e plugged false",
            "network -e wifi show -e level 4 -e fully true",
            "network -e mobile show -e datatype none -e level 4",
            "notifications -e visible false",
        ):
            _adb("shell", f"am broadcast -a com.android.systemui.demo -e command {command}", check=False)

    @keyword("Save Store Screenshot")
    def save_store_screenshot(self, name):
        """Saves the screen, as a 24-bit PNG (Play rejects alpha), into the Play listing's
        phone screenshots (android/play/metadata/.../phoneScreenshots/<name>.png)."""
        directory = Path(self.STORE_DIR)
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / f"{name}.png"
        self._capture().convert("RGB").save(path, optimize=True)
        logger.info(f"store screenshot -> {path}")

    # ------------------------------------------------------------------
    # Android's own UI (the system file pickers) via uiautomator
    # ------------------------------------------------------------------

    def _ui_nodes(self):
        """Every node of the current screen's accessibility tree, as dicts. The
        system pickers *do* have one, unlike the egui-drawn app."""
        import xml.etree.ElementTree as ET
        _adb("shell", "uiautomator", "dump", "/sdcard/window_dump.xml", check=False, timeout=60)
        xml = _adb("exec-out", "cat", "/sdcard/window_dump.xml", check=False)
        try:
            root = ET.fromstring(xml)
        except ET.ParseError:
            return []
        nodes = []
        for node in root.iter("node"):
            m = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", node.get("bounds", ""))
            if m:
                l, t, r, b = map(int, m.groups())
                nodes.append({
                    "text": node.get("text", ""), "desc": node.get("content-desc", ""),
                    "id": node.get("resource-id", ""), "center": ((l + r) // 2, (t + b) // 2),
                })
        return nodes

    @keyword("Wait Until UI Element Appears")
    def wait_until_ui_element_appears(self, text, timeout=15):
        """Waits until a node whose text, description or id contains `text` is on screen."""
        deadline = time.time() + float(timeout)
        while time.time() < deadline:
            if self._find_ui_node(text):
                return
            time.sleep(1)
        self.screenshot("ui-element-timeout")
        raise AssertionError(f"UI element {text!r} did not appear within {timeout}s")

    def _find_ui_node(self, text):
        """Best node for `text`: exact text/description, then an exact resource id
        (`android:id/button1`, or its part after the slash), then -- only for
        longer needles -- text that merely contains it. Never a substring of an
        id: `container_save` is a layout, not the SAVE button."""
        needle = text.lower()
        nodes = self._ui_nodes()
        for node in nodes:
            if needle in (node["text"].lower(), node["desc"].lower()):
                return node
        for node in nodes:
            rid = node["id"].lower()
            if rid and needle in (rid, rid.split("/")[-1]):
                return node
        if len(needle) >= 4:
            for node in nodes:
                if needle in node["text"].lower() or needle in node["desc"].lower():
                    return node
        return None

    @keyword("Wait Until Picker Is Showing")
    def wait_until_picker_is_showing(self, timeout=20):
        """Android's document picker (DocumentsUI) is the focused window."""
        deadline = time.time() + float(timeout)
        while time.time() < deadline:
            focus = _adb("shell", "dumpsys window | grep mCurrentFocus", check=False)
            if "documentsui" in focus.lower():
                time.sleep(1)  # let it draw
                return
            time.sleep(1)
        self.screenshot("picker-timeout")
        raise AssertionError("the system file picker did not appear")

    @keyword("Tap UI Element")
    def tap_ui_element(self, text):
        """Taps the system-UI node (a picker's button, a file in its list) whose
        text, description or resource id matches `text`."""
        node = self._find_ui_node(text)
        if node is None:
            self.screenshot("ui-element-missing")
            raise AssertionError(f"no UI element matching {text!r}")
        self.tap_at(*node["center"])

    # ------------------------------------------------------------------
    # Files on the device
    # ------------------------------------------------------------------

    @keyword("Push To Downloads")
    def push_to_downloads(self, local_path, name=None):
        """Puts a file in the shared Downloads folder, where the system file
        picker lists it. Returns its device path."""
        name = name or Path(local_path).name
        path = f"/sdcard/Download/{name}"
        _adb("push", str(local_path), path)
        # The picker's "Recent" list comes from the media store, which indexes a
        # pushed file lazily -- on a freshly booted emulator that once took longer
        # than the test waits. Ask for it now.
        _adb("shell", "am", "broadcast", "-a", "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
             "-d", f"file://{path}", check=False)
        return path

    @keyword("Remove Device File")
    def remove_device_file(self, path):
        _adb("shell", "rm", "-f", path, check=False)

    @keyword("Device File Should Contain")
    def device_file_should_contain(self, path, expected, timeout=10):
        deadline, content = time.time() + float(timeout), ""
        while time.time() < deadline:
            content = _adb("shell", "cat", path, check=False)
            if expected in content:
                return
            time.sleep(0.5)
        raise AssertionError(f"{path} does not contain {expected!r}; it holds: {content!r}")
