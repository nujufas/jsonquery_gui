*** Settings ***
Documentation     Using the app on a tablet: queries with their results beside the
...               source, the engine chips, touch browsing of the tree, and finding
...               text. The engines are the desktop's; these check the two-panel
...               screen and the touch entry points.
Resource          ../../resources/tablet.resource
Force Tags        usage
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-TAB-020 A Query's Results Appear Beside The Source
    [Documentation]    Source and Results are on screen together: the names the query
    ...    picked are in the right panel, and are not in the left one, which
    ...    still shows the untouched document.
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[] | select(.age > 21) | .name
    Status Should Contain    2 result(s)
    ${right}=    Right Half
    Screen Should Contain Text    Alice    region=${right}
    Screen Should Contain Text    Carol    region=${right}
    ${left}=    Left Half
    # (The dim "(3 keys)" rows do not OCR; the heading does.)
    Screen Should Contain Text    Source    region=${left}
    Screen Should Not Contain Text    Alice    region=${left}
    Screen Should Contain Text    Results    region=${right}

TC-TAB-021 Choosing An Engine Overrides Auto-Detection
    [Documentation]    With a chip picked the status names the engine without "auto".
    [Tags]    p1
    Load Fixture    people.json
    Choose Engine    ${CHIP_JQ_FROM_RIGHT}
    Run Query    .[0].name
    Status Should Contain    [jq]
    ${band}=    Status Band
    ${status}=    Read Screen Text    ${band}    psm=6
    Should Not Contain    ${status.lower()}    auto    the engine was chosen, not detected: ${status}

TC-TAB-022 A JSONPath Query Is Detected And Runs
    [Tags]    p1
    Load Fixture    people.json
    Run Query    $[*].name
    Status Should Contain    3 result(s)
    Status Should Contain    JSONPath

TC-TAB-023 A Bad Query Reports An Error
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[
    Screen Should Contain Text    error

TC-TAB-024 Tapping A Row's Arrow Expands It
    [Tags]    p1
    Load Fixture    people.json
    Tap At    ${ROW0_ARROW_X}    ${ROW0_Y}
    # The values are coloured and read well; the dim key names do not OCR.
    Wait Until Screen Contains Text    Alice    timeout=5
    Screen Should Contain Text    engineer

TC-TAB-025 Long Press Opens The Row Menu
    [Documentation]    egui maps a long press to a secondary click.
    [Tags]    p1
    Load Fixture    people.json
    Long Press At    ${ROW0_TEXT_X}    ${ROW0_Y}
    Wait Until Screen Contains Text    Copy JSON Path    timeout=5
    Screen Should Contain Text    Search

TC-TAB-026 The Text View Shows Pretty-Printed JSON
    [Tags]    p1
    Load Fixture    people.json
    Tap At    ${SOURCE_TEXT_X}    ${SOURCE_TAB_Y}
    Wait Until Screen Contains Text    Apply    timeout=10
    ${left}=    Left Half
    Screen Should Contain Text    Carol    region=${left}
    Screen Should Contain Text    manager    region=${left}

TC-TAB-027 Search Opens From The Toolbar And Finds A Match
    [Documentation]    A touch screen has no Ctrl+F, and this layout has no menu, so
    ...    the toolbar has a Search button of its own.
    [Tags]    p1
    Load Fixture    people.json
    Open Search
    Type Text    Bob
    Tap Text    Find    occurrence=2
    # Find reveals the match: Bob's record opens in the Source tree, showing a
    # value that was hidden before ("intern"). (The dialog's dim "1 of 1"
    # counter does not OCR.)
    ${left}=    Left Half
    Wait Until Screen Contains Text    intern    timeout=10    region=${left}

TC-TAB-028 Find All Lists Every Match
    [Tags]    p2
    Load Fixture    sites.json
    Open Search
    Type Text    75001
    Tap Text    Find All
    Wait Until Screen Matches    2 match|matches    timeout=10
