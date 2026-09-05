*** Settings ***
Documentation     Shared keywords and layout constants for every suite. Every
...               *** Test Cases *** suite file that uses this resource must
...               declare its own Suite/Test Setup and Teardown (Robot
...               doesn't allow those settings in a .resource file, and an
...               __init__.robot's settings only apply when Robot is pointed
...               at its *parent* directory -- not when a single suite
...               subdirectory is run on its own, which test/run.sh supports
...               and this project relies on during development). Copy this
...               block into every new suite file:
...
...               Suite Setup       Start Test Display
...               Suite Teardown    Stop Test Display
...               Test Setup        Launch Jsonquery App
...               Test Teardown     Close Jsonquery App
Library           AppLibrary.py

*** Variables ***
# All regions are (x, y, width, height) relative to the app window's
# client-area origin, calibrated against the default 1200x800 window in the
# Dark theme (the app's fixed launch state -- see 01_launch_and_window.md).
# Recalibrate these if the layout changes; see test/docs/00_test_strategy.md
# for how (OCR a screenshot at each candidate region, cross-check positions).
@{TOOLBAR_ROW}            0      0      230     20
@{STATUS_AREA}            230    0      950     20
@{QUERY_ROW}              0      22     260     24
@{ENGINE_PICKER_ROW}      970    22     230     24
@{QUERY_TEXTBOX}          0      46     1200    55
@{SOURCE_HEADER}          0      118    300     20
@{SOURCE_HINT_ROW}        0      140    592     20
@{RESULTS_HEADER}         600    118    300     20
@{RESULTS_SAVE_BUTTON}    1140   118    60      20
@{SOURCE_PANEL}           0      140    592     600
@{RESULTS_PANEL}          600    140    600     600
# Tall enough to hold jsonpath-rust's own multi-line syntax-error messages
# (confirmed during implementation: a JSONPath parse error can render as 5-6
# lines -- "Query error: ..." plus the underlying crate's own "-->" pointer
# and "= expected ..." lines -- which a plain single-line-sized crop would
# clip, hiding the very text a query-outcome wait is polling for). A single
# short status line still reads fine inside the extra headroom.
@{STATUS_BAR}             0      705    1200    95
# Icon-only controls OCR can't read -- fixed click points, not regions.
${THEME_TOGGLE_X}         1183
${THEME_TOGGLE_Y}         10
# The "Search --- Source/Results" and "Open URL" popups both render as a
# small floating window anchored near the toolbar's top-left corner -- same
# region works for either (calibrated against both).
@{POPUP_DIALOG_AREA}      15     15     345    130
${SEARCH_FIND_FIELD_X}    183
${SEARCH_FIND_FIELD_Y}    84
${SEARCH_REGEX_CHECKBOX_X}    30
${SEARCH_REGEX_CHECKBOX_Y}    106
${SEARCH_FIND_ALL_X}      48
${SEARCH_FIND_ALL_Y}      127
${SEARCH_CANCEL_X}        104
${SEARCH_CANCEL_Y}        127
${URL_FIELD_X}            183
${URL_FIELD_Y}            84
${URL_LOAD_X}             40
${URL_LOAD_Y}             106
${URL_CANCEL_X}           90
${URL_CANCEL_Y}           106
# The bottom search-results panel sizes itself to its content rather than
# staying a fixed height (confirmed during implementation: a 0-match "No
# matches found." panel and a 1-hit panel render their header at visibly
# different y-positions, ~739 vs ~609) -- so header and hit-list share one
# region wide enough to catch either, rather than two fixed-position crops
# that only work for one particular content size. Also tall enough on its
# own for reliable OCR: a too-short crop (the original 26px-tall header-only
# region) was confirmed to make Tesseract's layout analysis fail outright,
# producing pure noise rather than a bad-but-legible read.
@{SEARCH_RESULTS_AREA}    0      595    1200    205
# Engine-picker buttons, left to right (calibrated at the default 1200x800
# window size -- see ENGINE_PICKER_ROW above for the row they sit in).
${ENGINE_JQ_X}            1025
${ENGINE_POINTER_X}       1060
${ENGINE_JSONPATH_X}      1110
${ENGINE_JMESPATH_X}      1168
${ENGINE_ROW_Y}           38
# Fixtures served over HTTP for Open URL... tests -- avoids depending on the
# public internet, and lets load-failure/non-JSON/empty/oversized cases be
# constructed on demand rather than relying on a real remote server's
# behavior. See AppLibrary's Start/Stop Fixture Server.
${HTTP_FIXTURES_DIR}      ${CURDIR}/fixtures/http

*** Keywords ***
Load Fixture Via Paste
    [Documentation]    Loads a JSON fixture by pasting its text -- avoids the
    ...    native Open File dialog entirely (confirmed blocked, see the
    ...    strategy doc's "Native OS dialogs" section). Exercises the same
    ...    load/parse code path as Open File. Retries the click+paste up to 3
    ...    times: synthetic input to a freshly-focused window is occasionally
    ...    dropped by the X server (confirmed during implementation -- not
    ...    fully root-caused, structurally defended against here rather than
    ...    chased further; see 00_test_strategy.md's flakiness note).
    ...
    ...    Only works while the empty-state "Paste JSON here..." box is
    ...    showing -- confirmed during implementation that once a document is
    ...    loaded, that box no longer exists to paste into, and Ctrl+V simply
    ...    does nothing (the app has no global paste-to-replace shortcut).
    ...    To load a *second* document over one that's already showing, use
    ...    Load Via Url instead, which works regardless of current state.
    [Arguments]    ${json_text}
    Set Clipboard    ${json_text}
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Paste And Verify Loaded

Paste And Verify Loaded
    Click At    200    300
    Sleep    0.2s
    Press Keys    ctrl    v
    Wait Until Region Contains Text    @{STATUS_AREA}    pasted    timeout=3

Run Query
    [Documentation]    Types the given query text and runs it, waiting for
    ...    the run to finish. Retries the whole click+type+run sequence up
    ...    to 5 times for the same reason as Load Fixture Via Paste above --
    ...    confirmed under implementation to occasionally need more than 3
    ...    attempts on a loaded/busy machine.
    [Arguments]    ${query_text}
    Wait Until Keyword Succeeds    5x    0.5s
    ...    Type And Run Query    ${query_text}

Type And Run Query
    [Arguments]    ${query_text}
    Click At    100    58
    Sleep    0.3s
    Press Keys    ctrl    a
    Type Text    ${query_text}
    Sleep    0.3s
    Press Keys    ctrl    enter
    Wait Until Region Matches    @{STATUS_BAR}    result\\(s\\)|error    timeout=5

Select Engine
    [Documentation]    Clicks the named engine picker button (jq / Pointer /
    ...    JSONPath / JMESPath). Clicking an already-selected engine
    ...    deselects it back to auto-detect (app's own toggle behavior).
    [Arguments]    ${engine_label}
    Click Text In Region    @{ENGINE_PICKER_ROW}    ${engine_label}

Clear Query Box
    [Documentation]    Empties the query box without typing a replacement --
    ...    needed for the handful of auto-detect cases keyed off an *empty*
    ...    query, which `Type And Run Query`'s ctrl+a-then-type approach can't
    ...    produce on its own (typing "" is a no-op, leaving the old text
    ...    selected rather than gone).
    Click At    100    58
    Sleep    0.2s
    Press Keys    ctrl    a
    Press Key    delete

Open Row Context Menu
    [Documentation]    Right-clicks an app-relative point and waits for its
    ...    row context menu to actually appear before returning, retrying the
    ...    right-click itself if needed -- confirmed during implementation
    ...    that a right-click occasionally doesn't register the first time,
    ...    the same class of synthetic-input flakiness documented elsewhere
    ...    in this suite (see keywords.resource's own module doc / the
    ...    strategy doc). Every row's menu starts with "Save...", so that's
    ...    what confirms the menu opened, regardless of which row kind it is.
    [Arguments]    ${x}    ${y}
    Wait Until Keyword Succeeds    3x    0.3s
    ...    Right Click And Verify Menu Open    ${x}    ${y}

Right Click And Verify Menu Open
    [Arguments]    ${x}    ${y}
    Right Click At    ${x}    ${y}
    Sleep    0.3s
    ${menu_x}=    Evaluate    ${x} + 5
    ${menu_y}=    Evaluate    ${y} + 10
    Region Should Contain Text    ${menu_x}    ${menu_y}    280    120    Save

Row Context Menu Region
    [Documentation]    The (x, y, width, height) region a row's context menu
    ...    renders in, given the app-relative point it was opened from --
    ...    same offsets `Right Click And Verify Menu Open` uses to confirm
    ...    the menu opened, exposed here so a test can then OCR specific
    ...    items out of that same region.
    [Arguments]    ${x}    ${y}
    ${menu_x}=    Evaluate    ${x} + 5
    ${menu_y}=    Evaluate    ${y} + 10
    RETURN    ${menu_x}    ${menu_y}    280    120

Close Search Results Panel
    [Documentation]    Located by OCR rather than a fixed point: the panel's
    ...    height (and so its "Close" button's y-position) varies with how
    ...    many hits it's showing, confirmed during implementation (~y=609
    ...    for a panel with a hit line, ~y=739 for a bare "No matches
    ...    found." panel).
    Click Text In Region    @{SEARCH_RESULTS_AREA}    Close

Load Via Url
    [Documentation]    Opens "Open URL...", types `${url}`, and submits via
    ...    Load -- retrying the whole type+click if Load doesn't visibly
    ...    register. Confirmed during implementation: clicking Load
    ...    immediately after typing can land on a frame where egui hasn't
    ...    yet re-enabled the button, since typing finishes with no frame
    ...    gap before the click that follows it -- the same class of
    ...    input-timing flakiness documented for Ctrl+Enter elsewhere in this
    ...    suite, just for a plain click instead of a hotkey.
    [Arguments]    ${url}
    Click At    131    11
    Sleep    0.3s
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Type And Submit Url    ${url}

Type And Submit Url
    [Arguments]    ${url}
    Click At    ${URL_FIELD_X}    ${URL_FIELD_Y}
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    ${url}
    Sleep    0.3s
    Click At    ${URL_LOAD_X}    ${URL_LOAD_Y}
    Sleep    0.3s
    Region Should Not Contain Text    @{POPUP_DIALOG_AREA}    Open URL

Run Current Query
    [Documentation]    Runs whatever text is already in the query box, with
    ...    no typing step first -- for the handful of cases keyed off an
    ...    already-empty box (see Clear Query Box) or text left over from a
    ...    previous step. Retries for the same reason as Run Query.
    Wait Until Keyword Succeeds    5x    0.5s
    ...    Press Ctrl Enter And Wait

Press Ctrl Enter And Wait
    Press Keys    ctrl    enter
    Wait Until Region Matches    @{STATUS_BAR}    result\\(s\\)|error    timeout=5
