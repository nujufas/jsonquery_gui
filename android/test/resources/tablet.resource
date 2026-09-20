*** Settings ***
Documentation     Keywords shared by the tablet suites (suites-tablet/): install and
...               launch, and the UI-level actions built on AndroidLibrary's adb + OCR
...               primitives.
...
...               The coordinates are for the emulator profile made by
...               emulator-entry.sh (AVD=tablet: a Pixel Tablet, 2560x1600 at 320 dpi,
...               which is 1280x800 dp, wider than the app's 600-point breakpoint
...               either way up), measured from real screenshots. The layout is the
...               desktop's two panels, so the controls are anchored to the top left
...               (toolbar, query bar, the Source panel: the same in both
...               orientations) or to the right edge ("from right" values), and
...               everything that moves with content is found by its text instead.
Library           ${CURDIR}/AndroidLibrary.py    ${PACKAGE}
Library           OperatingSystem

*** Variables ***
${PACKAGE}        io.github.nujufas.jsonquery.debug
${APK}            ${EMPTY}
${FIXTURES}       ${CURDIR}/fixtures

# --- top left, in pixels ---
${TOOLBAR_Y}      90        # the row of Open File... / Open URL... / Clear
${OPEN_FILE_X}    115
${RUN_X}          176       # the Run button, at the start of the query row
${RUN_Y}          184
${QUERY_Y}        290       # the query box (its x is anywhere across the middle)
${SOURCE_TEXT_X}  367       # the Tree | Text switch of the Source panel
${SOURCE_TAB_Y}   434
${ROW0_ARROW_X}   63        # tree rows of the Source panel: root, then children 88px apart
${ROW0_Y}         635
${ROW0_TEXT_X}    330

# --- from the right edge (px inward from the screen's right edge) ---
${TUTORIAL_FROM_RIGHT}    248    # the toolbar's tutorial, autocomplete-toggle and theme buttons
${THEME_FROM_RIGHT}       58
${CLOSE_FROM_RIGHT}       50     # the tutorial window's close button
${TUTORIAL_CLOSE_Y}       97
${CHIP_JQ_FROM_RIGHT}         498    # the engine chips on the query row
${CHIP_POINTER_FROM_RIGHT}    392
${CHIP_JSONPATH_FROM_RIGHT}   246
${CHIP_JMESPATH_FROM_RIGHT}   88

# --- the on-screen keyboard, landscape: the first letter row ---
${KEY_ROW1_Y}     1028
${KEY_Q_X}        362
${KEY_W_X}        565
${KEY_E_X}        768

*** Keywords ***
Install The App Once
    [Documentation]    Suite setup: wait for the emulator and install the APK.
    Wait For Device
    Install App    ${APK}
    Set Orientation    landscape

Start The App
    Launch App

Stop The App
    [Documentation]    Test teardown: evidence first, then a crash check, then stop
    ...    and leave the tablet the way it starts (landscape).
    Save Final Screenshot
    Run Keyword And Continue On Failure    Application Should Not Have Crashed
    Force Stop App
    Set Orientation    landscape

Fixture Text
    [Arguments]    ${name}
    ${text}=    Get File    ${FIXTURES}/${name}
    RETURN    ${text}

Load Fixture
    [Documentation]    Opens a fixture the way the share sheet delivers JSON text,
    ...    and waits for the document to be parsed.
    [Arguments]    ${name}
    ${text}=    Fixture Text    ${name}
    Share Text To App    ${text}
    Wait Until Screen Contains Text    Parsed in    timeout=15

Tap From Right
    [Documentation]    Taps `${offset}` px in from the right edge of the screen (whichever
    ...    way up it is) at height `${y}`.
    [Arguments]    ${offset}    ${y}
    ${width}    ${height}=    Get Screen Size
    ${x}=    Evaluate    ${width} - ${offset}
    Tap At    ${x}    ${y}

Open Search
    [Documentation]    Taps the toolbar's Search... button (left of the document name).
    Tap Text    Search    region=${{[0, 40, 1100, 100]}}
    Wait Until Screen Contains Text    Regex    timeout=8

Focus Query Box
    ${width}    ${height}=    Get Screen Size
    ${x}=    Evaluate    ${width} // 2
    Tap At    ${x}    ${QUERY_Y}
    Sleep    1s

Run Query
    [Documentation]    Types `${query}` into the query box (as key events, the way a
    ...    hardware keyboard would), taps Run, and waits for an outcome.
    [Arguments]    ${query}
    Focus Query Box
    Type Text    ${query}
    Tap At    ${RUN_X}    ${RUN_Y}
    Wait Until Screen Matches    result\\(s\\)|error    timeout=15

Choose Engine
    [Arguments]    ${from_right}
    Tap From Right    ${from_right}    ${RUN_Y}

Panel Divider
    [Documentation]    x of the line between the Source and Results panels, and the
    ...    screen's width, read off a row in the empty lower part of the panels.
    ${width}    ${height}=    Get Screen Size
    ${row}=    Evaluate    int(${height} * 0.7)
    ${from}=    Evaluate    int(${width} * 0.15)
    ${to}=    Evaluate    int(${width} * 0.85)
    ${x}=    Find Panel Divider    ${row}    x_from=${from}    x_to=${to}
    RETURN    ${x}    ${width}

Split Should Be Even
    [Documentation]    The two panels share the width about equally (the divider lies
    ...    between 40% and 60% of it). This is what fails when one panel
    ...    fills the screen and the other is squeezed to nothing.
    ${x}    ${width}=    Panel Divider
    ${share}=    Evaluate    ${x} / ${width}
    Should Be True    0.40 <= ${share} <= 0.60    the divider is at x=${x} of ${width} (${share})

Left Half
    [Documentation]    OCR region: the Source panel's body, below the toolbar and query bar.
    ${width}    ${height}=    Get Screen Size
    RETURN    ${{[0, 300, ${width} // 2 - 10, ${height} - 500]}}

Right Half
    [Documentation]    OCR region: the Results panel's body.
    ${width}    ${height}=    Get Screen Size
    RETURN    ${{[${width} // 2 + 12, 300, ${width} // 2 - 12, ${height} - 500]}}

Status Band
    [Documentation]    The line under the panels ("Parsed in ...", "Query ran in ...").
    ${width}    ${height}=    Get Screen Size
    RETURN    ${{[0, ${height} - 170, ${width}, 120]}}

Status Should Contain
    [Arguments]    ${expected}
    ${band}=    Status Band
    ${status}=    Read Screen Text    ${band}    psm=6
    Should Contain    ${status.lower()}    ${expected.lower()}    OCR of the status line: ${status}

Keyboard Should Be Showing
    [Documentation]    The on-screen keyboard is a light panel over the app's dark
    ...    lower edge; OCR of its keys is too unreliable to use.
    ${width}    ${height}=    Get Screen Size
    ${y}=    Evaluate    ${height} - 300
    ${pixel}=    Get Pixel Color    10    ${y}
    Should Be True    ${pixel}[0] > 200    no on-screen keyboard (pixel ${pixel})

Keyboard Should Not Be Showing
    ${width}    ${height}=    Get Screen Size
    ${y}=    Evaluate    ${height} - 300
    ${pixel}=    Get Pixel Color    10    ${y}
    Should Be True    ${pixel}[0] < 100    the on-screen keyboard is still showing (pixel ${pixel})

Wait For Keyboard
    Wait Until Keyword Succeeds    10s    0.5s    Keyboard Should Be Showing

Warm Up The Keyboard
    [Documentation]    Suite setup for the keyboard tests. The emulator's input method
    ...    starts lazily, so on a freshly booted device the first request to
    ...    show it can be slower than a test's 10 s wait (seen only right after
    ...    boot, never on a warm device). Raise it once, patiently and tapping
    ...    again as a user would, then start the suite from a clean launch.
    ...    Each test still has to raise it within its own 10 s.
    Install The App Once
    Launch App
    Wait Until Keyword Succeeds    120s    12s    Raise The Keyboard
    Force Stop App

Raise The Keyboard
    Focus Query Box
    Wait For Keyboard
