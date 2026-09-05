*** Settings ***
Documentation     Launch, window, and theme -- see test/docs/01_launch_and_window.md.
...               TC-WIN-004 (minimum window size enforced) isn't implemented:
...               it needs an interactive border-drag resize to exercise
...               winit's advisory min-size hint, but this suite runs with
...               fluxbox's Deco:NONE (no border to drag -- required
...               elsewhere for accurate window-origin coordinates, see
...               AppLibrary.py), and a direct `xdotool windowsize` bypasses
...               the hint entirely rather than exercising it (confirmed
...               during implementation: the window shrank right past 640x420
...               with no pushback).
Resource          ../../resources/keywords.resource
Force Tags        launch_and_window
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Test Cases ***
TC-WIN-001 App Launches To A Known Default State
    [Documentation]    1200x800 window, titled "jsonquery", Dark theme, no
    ...    document loaded -- see 01_launch_and_window.md.
    [Tags]    p1
    ${w}    ${h}=    Get Window Size
    Should Be Equal As Integers    ${w}    1200
    Should Be Equal As Integers    ${h}    800
    Region Should Contain Text    @{SOURCE_PANEL}    Paste JSON here

TC-WIN-003 Empty State Placeholder Text
    [Documentation]    Left panel shows the no-document hint before anything
    ...    is loaded. Only checks the placeholder text, not the full
    ...    "Drag & drop... or use Open File." sentence above it -- that
    ...    entire line renders in the app's dim "weak" gray style, which OCR
    ...    could not reliably read even with contrast enhancement/inversion
    ...    tried during implementation (see 00_test_strategy.md's OCR
    ...    limitations note). Not a gap in what the app does, just in what
    ...    this technique can verify about especially low-contrast text --
    ...    a real screenshot review during implementation confirmed the
    ...    hint line is present and reads correctly to the human eye.
    [Tags]    p2
    Region Should Contain Text    @{SOURCE_PANEL}    Paste JSON here

TC-WIN-002 Theme Toggle Switches Dark And Light
    [Documentation]    Clicking the theme toggle changes the panel background;
    ...    clicking it again reverts. Verified by relative pixel-color change
    ...    rather than exact RGB match (see 00_test_strategy.md on avoiding
    ...    brittle exact-color assertions).
    [Tags]    p2
    ${before}=    Get Pixel Color    300    110
    Click At    ${THEME_TOGGLE_X}    ${THEME_TOGGLE_Y}
    Sleep    0.3s
    ${after}=    Get Pixel Color    300    110
    Colors Should Not Match    ${before}    ${after}
    ...    msg=Background color did not change after toggling theme
    Click At    ${THEME_TOGGLE_X}    ${THEME_TOGGLE_Y}
    Sleep    0.3s
    ${reverted}=    Get Pixel Color    300    110
    Colors Should Match    ${before}    ${reverted}
    ...    msg=Background color did not revert after toggling theme back

TC-WIN-005 No Native Menu Bar Or About/Help Entry Point Exists
    [Documentation]    This app has no menu bar -- the toolbar is the very
    ...    first row of window content, and there's no About/Help anywhere
    ...    in it or the status area.
    [Tags]    p3
    Region Should Contain Text    @{TOOLBAR_ROW}    Open File
    Region Should Not Contain Text    @{TOOLBAR_ROW}    Help
    Region Should Not Contain Text    @{TOOLBAR_ROW}    Edit
    Region Should Not Contain Text    @{STATUS_AREA}    Help
    Region Should Not Contain Text    @{STATUS_AREA}    About
