*** Settings ***
Documentation     Global keyboard shortcuts -- see
...               test/docs/10_keyboard_shortcuts.md. TC-KEY-001 (Ctrl+Enter
...               runs the query regardless of focus) is exercised by
...               TC-QRY-061 instead of duplicated here. TC-KEY-003 (Ctrl+S
...               saves the focused panel) needs the blocked native Save
...               dialog -- not implemented. TC-KEY-007 (native text-editing
...               keys smoke test) is not implemented: it would only be
...               re-confirming egui's own TextEdit widget behavior, not
...               anything this app added.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        keyboard_shortcuts
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Load People Fixture
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

*** Keywords ***
Load People Fixture
    Launch Jsonquery App
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}

Open Url And Submit Via Enter
    [Arguments]    ${url}
    Click At    131    11
    Sleep    0.3s
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Type Url And Press Enter    ${url}

Type Url And Press Enter
    [Arguments]    ${url}
    Click At    ${URL_FIELD_X}    ${URL_FIELD_Y}
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    ${url}
    Sleep    0.3s
    Press Key    enter
    Sleep    0.3s
    Region Should Not Contain Text    @{POPUP_DIALOG_AREA}    Open URL

Search For Via Enter
    [Arguments]    ${text}
    Click At    200    300
    Sleep    0.2s
    Press Keys    ctrl    f
    Sleep    0.3s
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Type Search And Press Enter    ${text}

Type Search And Press Enter
    [Arguments]    ${text}
    Click At    ${SEARCH_FIND_FIELD_X}    ${SEARCH_FIND_FIELD_Y}
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    ${text}
    Sleep    0.3s
    Press Key    enter
    Wait Until Region Matches    @{SEARCH_RESULTS_AREA}    results|error    timeout=3

*** Test Cases ***
TC-KEY-000 And TC-KEY-002 Ctrl+F Opens Search Scoped To The Last-Clicked Panel
    [Tags]    p2
    Click At    200    300
    Sleep    0.2s
    Press Keys    ctrl    f
    Sleep    0.3s
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Source
    Click At    ${SEARCH_CANCEL_X}    ${SEARCH_CANCEL_Y}
    Sleep    0.3s
    Click At    800    300
    Sleep    0.2s
    Press Keys    ctrl    f
    Sleep    0.3s
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Results

TC-KEY-004 Ctrl+Enter Loads A Paste, But Only While The Paste Box Has Focus
    [Tags]    p2
    Click At    199    11
    Sleep    0.3s
    Click At    200    300
    Sleep    0.2s
    Type Text    {"via": "ctrl-enter"}
    Click At    800    300
    Sleep    0.2s
    Press Keys    ctrl    enter
    Sleep    0.3s
    Region Should Not Contain Text    @{STATUS_AREA}    pasted JSON
    ...    msg=Ctrl+Enter should not submit the paste box once it lost focus
    Click At    200    300
    Sleep    0.2s
    Press Keys    ctrl    enter
    Wait Until Region Contains Text    @{STATUS_AREA}    pasted JSON    timeout=5

TC-KEY-006a Enter Submits The Open URL Popup
    [Tags]    p3
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Open Url And Submit Via Enter    ${base_url}/valid.json
    Wait Until Region Contains Text    @{STATUS_AREA}    valid.json    timeout=5
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-KEY-006b Enter Submits The Search Popup
    [Tags]    p3
    Search For Via Enter    Alice
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Alice
