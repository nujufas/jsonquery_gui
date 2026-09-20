*** Settings ***
Documentation     Keywords shared by the Android suites: install/launch, and the
...               UI-level actions (menu, tabs, running a query) built on
...               AndroidLibrary's adb + OCR primitives.
...
...               Fixed coordinates are for the emulator profile in
...               docker/Dockerfile.emulator (Pixel 3: 1080x2160, 440 dpi), measured from
...               real screenshots of the phone layout. Everything that moves with
...               content is found by its text instead.
Library           ${CURDIR}/AndroidLibrary.py    ${PACKAGE}
Library           OperatingSystem

*** Variables ***
${PACKAGE}        io.github.nujufas.jsonquery.debug
${APK}            ${EMPTY}
${FIXTURES}       ${CURDIR}/fixtures

# --- the phone layout, portrait (pixels) ---
${MENU_X}         76        # the ☰ button, top left below the status bar
${MENU_Y}         122
${RUN_X}          119       # the Run button
${RUN_Y}          254
${CHIP_Y}         254       # the engine chips share the Run row
${CHIP_JQ_X}      311
${QUERY_X}        540       # the query box, right under that row
${QUERY_Y}        400
${TAB_Y}          2027      # the Source | Results tabs at the bottom
${TAB_SOURCE_X}   275
${TAB_RESULTS_X}  805
${TREE_TAB_Y}     596       # the Tree | Text switch above a panel
${TEXT_TAB_X}     267
${ROW0_Y}         753       # tree rows: the root, then its children 120px apart
${ROW1_Y}         874
${ROW1_ARROW_X}   86        # the expand arrow of a depth-1 row
# [x, y, width, height] bands of the screen, for OCR.
# The Source | Results tab bar at the bottom:
${TAB_BAND}       ${{[0, 1960, 1080, 200]}}
# The line above the tabs: "Parsed in ..." / "Query ran in ... result(s)".
${STATUS_BAND}    ${{[0, 1740, 1080, 230]}}
# The on-screen keyboard (AOSP/Gboard-style, in the emulator image): a few keys.
${KEY_ROW1_Y}     1534      # the q w e r ... row
${KEY_Q_X}        59
${KEY_W_X}        164
${KEY_E_X}        271
${KEY_R_X}        370
${KEY_BACKSPACE_X}    991
${KEY_BACKSPACE_Y}    1814

*** Keywords ***
Install The App Once
    [Documentation]    Suite setup: wait for the emulator and install the APK.
    Wait For Device
    Install App    ${APK}
    Set Orientation    portrait

Start The App
    Launch App

Stop The App
    [Documentation]    Test teardown: evidence first, then a crash check, then stop.
    Save Final Screenshot
    Run Keyword And Continue On Failure    Application Should Not Have Crashed
    Force Stop App

Fixture Text
    [Arguments]    ${name}
    ${text}=    Get File    ${FIXTURES}/${name}
    RETURN    ${text}

Open Menu
    Tap At    ${MENU_X}    ${MENU_Y}

Choose Menu Item
    [Arguments]    ${item}
    Open Menu
    Wait Until Screen Contains Text    ${item}    timeout=5
    Tap Text    ${item}

Load Fixture
    [Documentation]    Opens a fixture the way the share sheet delivers JSON text,
    ...    and waits for the document to be parsed.
    [Arguments]    ${name}
    ${text}=    Fixture Text    ${name}
    Share Text To App    ${text}
    Wait Until Screen Contains Text    Parsed in    timeout=15

Show Source Tab
    Tap At    ${TAB_SOURCE_X}    ${TAB_Y}

Show Results Tab
    Tap At    ${TAB_RESULTS_X}    ${TAB_Y}

Focus Query Box
    Tap At    ${QUERY_X}    ${QUERY_Y}
    Sleep    1s

Run Query
    [Documentation]    Types `${query}` into the query box (as key events, the way a
    ...    hardware keyboard would), taps Run, and waits for an outcome. Ends
    ...    on the Results tab, as a user's would.
    [Arguments]    ${query}
    Focus Query Box
    Type Text    ${query}
    Tap At    ${RUN_X}    ${RUN_Y}
    Wait Until Screen Matches    result\\(s\\)|error    timeout=15

Choose Engine
    [Arguments]    ${x}
    Tap At    ${x}    ${CHIP_Y}

Tab Should Be Selected
    [Documentation]    The selected bottom tab is filled with the selection blue.
    [Arguments]    ${tab}
    ${x}=    Set Variable If    '${tab}' == 'Source'    ${TAB_SOURCE_X}    ${TAB_RESULTS_X}
    ${pixel}=    Get Pixel Color    ${x}    ${TAB_Y}
    Should Be True    ${pixel}[2] > ${pixel}[0] + 40    the ${tab} tab is not selected (pixel ${pixel})

Keyboard Should Be Showing
    [Documentation]    The on-screen keyboard is a light panel over the app's dark
    ...    bottom edge; OCR of its keys is too unreliable to use.
    ${pixel}=    Get Pixel Color    300    2050
    Should Be True    ${pixel}[0] > 200    no on-screen keyboard (pixel ${pixel})

Keyboard Should Not Be Showing
    ${pixel}=    Get Pixel Color    300    2050
    Should Be True    ${pixel}[0] < 100    the on-screen keyboard is still showing (pixel ${pixel})

Wait For Keyboard
    Wait Until Keyword Succeeds    10s    0.5s    Keyboard Should Be Showing

Tap Keys
    [Documentation]    Taps keys of the on-screen keyboard's first letter row by x.
    [Arguments]    @{xs}
    FOR    ${x}    IN    @{xs}
        Tap At    ${x}    ${KEY_ROW1_Y}
        Sleep    0.4s
    END

Status Should Contain
    [Arguments]    ${expected}
    ${status}=    Read Screen Text    ${STATUS_BAND}    psm=6
    Should Contain    ${status.lower()}    ${expected.lower()}    OCR of the status band: ${status}

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
