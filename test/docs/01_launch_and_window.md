# Launch, window, and theme — test requirements

Source: `crates/app/src/main.rs`, `crates/app/src/app.rs` (toolbar theme toggle,
empty-state placeholder). See the strategy doc for the fixed-window-size/fixed-theme
rule these tests rely on for image/OCR stability.

## Preconditions common to this whole area

- Freshly launched `jsonquery_gui` process, no document loaded, no prior state
  (each test launches its own process — see strategy doc's Process-library note).

## Test cases

### TC-WIN-001 — App launches to a known default state
Priority: P1
Steps:
1. Launch the binary.
2. Wait for the main window to appear.
Expected:
- Window title bar reads exactly `jsonquery` (never changes at runtime — confirm
  this stays true after loading a document too, as a negative check in
  TC-OPEN-001 rather than repeating a full launch here).
- Window size is 1200×800 on first launch.
- Theme is **Dark** (the app always starts in Dark, regardless of any previous
  run's state — there is no persisted theme setting).
- The toolbar's theme toggle button shows the "switch to light" icon (`☀`),
  consistent with currently being in Dark mode.
- The left panel shows the "no document" placeholder text (see TC-WIN-003).
Automation notes: assert window title via the window manager (`xdotool
getwindowname` / equivalent), not OCR, since it's OS chrome, not app-rendered.

### TC-WIN-002 — Theme toggle switches Dark ↔ Light and updates its own icon/tooltip
Priority: P2
Preconditions: app launched (Dark, per TC-WIN-001).
Steps:
1. Click the theme toggle button (top-right of toolbar).
2. Observe the button and overall panel background.
3. Click it again.
Expected:
- After step 1: panel background switches to the light palette; button icon
  becomes `🌙`; hovering it shows tooltip "Switch to dark theme"; tree-view
  value colors switch to their light-theme RGB variants (see
  [05_tree_view.md](05_tree_view.md) if a document is loaded for this check).
- After step 3: back to Dark, icon `☀`, tooltip "Switch to light theme".
Automation notes: icon glyph is a poor OCR target (small symbol font) —
prefer a template-image match of the button in each state, or a coarse
background-color sample (single-pixel/region average color check) rather than
OCR for this one.

### TC-WIN-003 — Empty-state placeholder text, no document loaded
Priority: P2
Steps:
1. Launch the binary, don't load anything.
2. Read the left (Source) panel.
Expected:
- Weak/gray text reads: `No document loaded — drag & drop a JSON file anywhere,
  use Open File, or paste JSON on the left.`
- A paste textarea is present below it with hint text `Paste JSON here…`
  (hint text only shows while the field is empty and unfocused — don't assert
  it after focusing the field in the same test).
Automation notes: OCR region over the left panel; treat as "contains" not exact
match (wrapped line breaks are layout-dependent).

### TC-WIN-004 — Minimum window size is enforced
Priority: P3
Steps:
1. Attempt to resize the window below 640×420 via the window manager.
Expected: window does not shrink past 640×420.
Automation notes: window-manager-level check (`xdotool`/`wmctrl`), not app UI.

### TC-WIN-005 — No About/Help/version UI exists anywhere
Priority: P3
Steps: Inspect the toolbar, panel headers, and check for any menu bar.
Expected: no menu bar is present at all; no About dialog, no version string
displayed anywhere in the running app.
Automation notes: this is a negative/absence assertion — cheapest as a one-time
manual-review checkbox rather than an automated image search for "nothing",
but included here for completeness of the requirement. If automated, assert
no menu-bar-shaped region exists at the window's top edge above the toolbar.
