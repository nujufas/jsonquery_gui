*** Settings ***
Documentation     The tablet layout. A tablet is wider than the phone layout's 600
...               points either way up, so it gets the desktop's two panels side by
...               side. Their split has to start even (it once did not: the Source
...               panel filled the screen and the Results panel never appeared),
...               survive rotation and a shrinking window, and stay clear of the
...               system bars. Tablets are landscape by nature, so that is how the
...               app is started here.
Resource          ../../resources/tablet.resource
Force Tags        layout
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Variables ***
${TOOLBAR_BAND}   ${{[0, 40, 1100, 100]}}

*** Test Cases ***
TC-TAB-001 The Toolbar Has Its Own Buttons Instead Of A Menu
    [Documentation]    The phone folds them into a menu; a tablet has the room.
    [Tags]    p1
    Screen Should Contain Text    Open File    region=${TOOLBAR_BAND}
    Screen Should Contain Text    Open URL    region=${TOOLBAR_BAND}

TC-TAB-002 Source And Results Are Side By Side
    [Documentation]    Both headings are on screen, on the same line, far apart:
    ...    two columns, not the phone's one panel behind tabs.
    [Tags]    p1
    ${width}    ${height}=    Get Screen Size
    Should Be True    ${width} > ${height}    the tablet did not start in landscape
    ${source_x}    ${source_y}=    Find Text On Screen    Source
    ${results_x}    ${results_y}=    Find Text On Screen    Results
    Should Be True    abs(${results_y} - ${source_y}) < 40    the headings are on different lines
    Should Be True    ${results_x} - ${source_x} > ${width} / 3    the headings are not in two columns

TC-TAB-003 The Split Starts Even In Landscape
    [Documentation]    Regression: the first frame laid the window out at a placeholder
    ...    ~8700 points wide, and the Source panel remembered half of that,
    ...    so it filled the whole screen and left the Results panel no room.
    [Tags]    p1
    Split Should Be Even

TC-TAB-004 Portrait Also Gets Two Panels
    [Documentation]    800 dp wide is still over the phone layout's limit.
    [Tags]    p1
    Set Orientation    portrait
    Wait Until Screen Contains Text    Results    timeout=10
    ${width}    ${height}=    Get Screen Size
    Should Be True    ${height} > ${width}    the tablet did not turn to portrait
    Wait Until Keyword Succeeds    10s    1s    Split Should Be Even

TC-TAB-005 Each Orientation Keeps Its Own Split
    [Documentation]    A panel remembers its width in points, so the landscape width
    ...    used to squeeze the Results panel in portrait until "Text" and
    ...    "Save..." were cut off its header. Each way up now starts even, and
    ...    neither header loses its buttons.
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[] | .name
    Set Orientation    portrait
    Wait Until Keyword Succeeds    10s    1s    Split Should Be Even
    ${x}    ${width}=    Panel Divider
    ${band}=    Evaluate    [${x}, 380, ${width} - ${x}, 120]
    Screen Should Contain Text    Save    region=${band}
    Set Orientation    landscape
    Wait Until Keyword Succeeds    10s    1s    Split Should Be Even

TC-TAB-006 The Document And Its Result Survive A Rotation
    [Documentation]    Rotating must not restart the app or drop the document.
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[] | .name
    Set Orientation    portrait
    Wait Until Screen Contains Text    Alice    timeout=10
    Status Should Contain    3 result(s)
    Set Orientation    landscape
    Wait Until Screen Contains Text    Alice    timeout=10
    Status Should Contain    3 result(s)

TC-TAB-007 The UI Stays Clear Of The System Bars
    [Documentation]    Edge to edge: the toolbar's first button must start below the
    ...    status bar (48 px on this profile), and the status line must end
    ...    above the gesture bar (the bottom 64 px). The column is left of
    ...    the clock.
    [Tags]    p1
    ${top}=    Find First Different Pixel Below    24    5
    Should Be True    ${top} >= 48    the toolbar starts at y=${top}, inside the status bar
    Load Fixture    people.json
    ${width}    ${height}=    Get Screen Size
    ${x}    ${y}=    Find Text On Screen    Parsed in
    Should Be True    ${y} < ${height} - 64    the status line is at y=${y}, inside the gesture bar

TC-TAB-008 Buttons Are Finger Sized
    [Documentation]    Touch mode enlarges the controls. A desktop-sized button on this
    ...    screen is about 41 px tall; a touch one is over 64 (32 dp).
    [Tags]    p2
    ${top}=    Find First Different Pixel Below    24    5
    ${inside}=    Evaluate    ${top} + 10
    ${bottom}=    Find First Different Pixel Below    24    ${inside}
    Should Be True    ${bottom} - ${top} >= 64    the Open File button spans only y=${top}..${bottom}

TC-TAB-010 The Toolbar's Icon Buttons Are All Reachable In Portrait
    [Documentation]    Held upright the toolbar is narrow. The empty-state hint used to
    ...    run on underneath the tutorial, autocomplete and theme buttons,
    ...    stacking them at the screen's edge so that only the top one could be
    ...    tapped. The theme button, the rightmost, must respond (and the
    ...    tutorial, which was on top of it, must not open instead).
    [Tags]    p1
    Set Orientation    portrait
    Wait Until Screen Contains Text    Results    timeout=10
    ${width}    ${height}=    Get Screen Size
    ${x}=    Evaluate    ${width} // 4
    ${y}=    Evaluate    int(${height} * 0.7)
    ${dark}=    Get Pixel Color    ${x}    ${y}
    Tap From Right    ${THEME_FROM_RIGHT}    ${TOOLBAR_Y}
    Sleep    1s
    ${light}=    Get Pixel Color    ${x}    ${y}
    Should Be True    ${light}[0] > ${dark}[0] + 100    the theme button did not respond: ${dark} -> ${light}
    Screen Should Not Contain Text    Identity

TC-TAB-009 A Shrunken Window Falls Back To The Phone Layout
    [Documentation]    Split-screen and freeform windows can be narrower than 600 points
    ...    on a tablet. The app must follow the window, not the device: tabs
    ...    when narrow, two panels again when the window grows back, with
    ...    an even split.
    [Tags]    p2
    Set Display Size    1000    1600
    Wait Until Screen Contains Text    Results    timeout=10
    ${width}    ${height}=    Get Screen Size
    ${band}=    Evaluate    [0, ${height} - 300, ${width}, 300]
    Screen Should Contain Text    Source    region=${band}
    Screen Should Contain Text    Results    region=${band}
    Run Keyword And Expect Error    *no vertical line*    Panel Divider
    Reset Display Size
    Wait Until Keyword Succeeds    15s    1s    Split Should Be Even
    [Teardown]    Run Keywords    Reset Display Size    AND    Stop The App
