*** Settings ***
Documentation     App start-up on a phone: the empty state, the phone layout, the
...               system-bar insets, rotation, theme, and running on a 16 KB-page
...               kernel (which Google Play now requires apps to support).
Resource          ../../resources/keywords.resource
Force Tags        launch
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-001 App Starts And Shows The Empty State
    [Documentation]    With nothing loaded the app offers the three ways to get a
    ...    document: open a file, paste, or a URL.
    [Tags]    p1
    Screen Should Contain Text    Open file
    Screen Should Contain Text    Paste from clipboard
    Screen Should Contain Text    Open URL

TC-AND-002 Phone Layout Uses A Tab Bar
    [Documentation]    Narrow screens show one panel at a time behind Source /
    ...    Results tabs at the bottom of the screen.
    [Tags]    p1
    Screen Should Contain Text    Source    region=${TAB_BAND}
    Screen Should Contain Text    Results    region=${TAB_BAND}

TC-AND-003 Runs On A 16 KB Page Size Kernel
    [Documentation]    Google Play requires apps to support 16 KB memory pages. The
    ...    emulator image has them; the native library loading and the app
    ...    drawing its first frame prove it works there, and Android's "app
    ...    isn't 16 KB compatible" warning must not have appeared.
    [Tags]    p1    play
    ${page}=    Get Device Page Size
    Should Be Equal As Integers    ${page}    16384
    Screen Should Contain Text    Open file
    Screen Should Not Contain Text    16 KB compatible
    Screen Should Not Contain Text    page size compatible mode

TC-AND-004 The UI Stays Clear Of The Status Bar
    [Documentation]    The window draws edge to edge; the toolbar's menu button must
    ...    start below the status bar (66px on this profile), not under the
    ...    clock and battery icons. Scans down the button's column to where it
    ...    starts.
    [Tags]    p1
    ${top}=    Find First Different Pixel Below    40    5
    Should Be True    ${top} >= 66    the menu button starts at y=${top}, inside the status bar

TC-AND-005 The Tab Bar Stays Above The Gesture Bar
    [Documentation]    Likewise at the bottom: the selected Source tab (filled with the
    ...    selection colour) must end above the gesture bar's band (the bottom
    ...    66px = y 2094), not run under it.
    [Tags]    p1
    ${blue}=    Get Pixel Color    ${TAB_SOURCE_X}    ${TAB_Y}
    ${bottom}=    Find Last Pixel Of Color Above    ${TAB_SOURCE_X}    2159    ${blue}
    Should Be True    ${bottom} < 2094    the tab reaches y=${bottom}, into the gesture bar

TC-AND-006 Rotating To Landscape Shows Both Panels Side By Side
    [Documentation]    A wide window has room for Source and Results together, as on
    ...    a desktop: the toolbar's own buttons, and a divider between the two
    ...    panels somewhere in the middle third. (It used to be checked only
    ...    by the toolbar text, which a screen filled by the Source panel
    ...    shows just as well.)
    [Tags]    p2
    Set Orientation    landscape
    Wait Until Screen Contains Text    Query:    timeout=10
    Screen Should Contain Text    Open File
    ${width}    ${height}=    Get Screen Size
    ${row}=    Evaluate    int(${height} * 0.7)
    ${from}=    Evaluate    int(${width} * 0.15)
    ${to}=    Evaluate    int(${width} * 0.85)
    ${x}=    Find Panel Divider    ${row}    x_from=${from}    x_to=${to}
    ${share}=    Evaluate    ${x} / ${width}
    Should Be True    0.40 <= ${share} <= 0.60    the divider is at x=${x} of ${width} (${share})
    [Teardown]    Run Keywords    Set Orientation    portrait    AND    Stop The App

TC-AND-007 The App Survives A Rotation With A Document Open
    [Tags]    p2
    Load Fixture    people.json
    Set Orientation    landscape
    Wait Until Screen Contains Text    pasted JSON    timeout=10
    Screen Should Contain Text    Parsed in
    Set Orientation    portrait
    Wait Until Screen Contains Text    pasted JSON    timeout=10
    Screen Should Contain Text    Parsed in
    [Teardown]    Run Keywords    Set Orientation    portrait    AND    Stop The App

TC-AND-008 Light Theme Can Be Chosen From The Menu
    [Documentation]    The menu's theme item flips the whole UI to the light theme
    ...    (the app starts dark) and back.
    [Tags]    p2
    ${dark}=    Get Pixel Color    500    1300
    Choose Menu Item    Light theme
    Sleep    1s
    ${light}=    Get Pixel Color    500    1300
    Should Be True    ${light}[0] > ${dark}[0] + 100    the background did not get lighter: ${dark} -> ${light}
    Choose Menu Item    Dark theme
    Sleep    1s
    ${again}=    Get Pixel Color    500    1300
    Should Be True    ${again}[0] < 90    the background did not go back to dark: ${again}
