*** Settings ***
Documentation     Toolbar and status bar -- see
...               test/docs/03_toolbar_and_status_bar.md. Covers the source
...               label per source kind, byte-size formatting, the NDJSON
...               suffix, and parse-time/status-area conditionals. Save
...               success/error (TC-TOOL-006) needs the native Save dialog
...               and isn't covered here -- see 00_test_strategy.md.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        toolbar_and_status
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

*** Test Cases ***
TC-TOOL-005 Parse Time Text Only Appears Once A Document Is Loaded
    [Documentation]    "Parsed in ..." renders in the bottom status bar, not
    ...    the toolbar.
    [Tags]    p3
    Region Should Not Contain Text    @{STATUS_BAR}    Parsed
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Contain Text    @{STATUS_BAR}    Parsed

TC-TOOL-001 Source Label Reflects The Source Kind
    [Documentation]    Pasted JSON shows the fixed note "(pasted JSON)" beside
    ...    the (then empty) source field; a URL source shows the URL itself
    ...    in the field. File sources are the same as URLs but need the
    ...    native file dialog behind the "..." button, and aren't covered
    ...    here (a typed path is: see TC-OPEN-019).
    [Tags]    p2
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Contain Text    @{STATUS_AREA}    pasted JSON
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/valid.json
    Wait Until Pasted Source Is Replaced
    Region Should Contain Text    @{SOURCE_FIELD}    valid.json
    Region Should Contain Text    @{SOURCE_FIELD}    ${base_url}
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-TOOL-002 Byte Size Is Shown In Human-Readable Units
    [Documentation]    A tiny pasted document shows a plain byte count ("B");
    ...    a much larger one (loaded via URL, since paste has no practical
    ...    size ceiling to demonstrate this against) shows KB.
    [Tags]    p3
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    # "112 B" reads back as "1128" -- Tesseract takes the weak-gray B for an
    # 8 -- so the pattern accepts either; what it rules out is a KB size.
    Wait Until Region Matches    @{STATUS_AREA}    \\d+ ?[B8]    timeout=3
    Region Should Not Contain Text    @{STATUS_AREA}    KB
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/big_array.json
    Wait Until Region Contains Text    @{STATUS_AREA}    KB    timeout=5
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-TOOL-003 NDJSON Record Count Suffix Is Conditional
    [Documentation]    The "(N NDJSON records)" suffix only appears when the
    ...    source had more than one top-level JSON value.
    [Tags]    p3
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Not Contain Text    @{STATUS_AREA}    NDJSON
    Click At    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    Sleep    0.3s
    Load Fixture Via Paste    {"a": 1}\n{"b": 2}\n{"c": 3}
    Region Should Contain Text    @{STATUS_AREA}    3 NDJSON records

TC-TOOL-004 A Failed Reload Doesn't Blank Out The Still-Loaded Document's State
    [Documentation]    A load error only ever describes the *attempt* -- the
    ...    previously loaded document (still intact, unaffected) keeps
    ...    showing its own "Parsed in ..." alongside the new error, rather
    ...    than either one being clobbered by the other. Both render in the
    ...    bottom status bar. Uses the source field for the failing second
    ...    load, not a second paste: pasting only loads at all while the
    ...    empty-state paste box is showing (confirmed during implementation
    ...    -- once a document is loaded, there's no box left to paste into,
    ...    so Ctrl+V does nothing). The source field has no such restriction.
    [Tags]    p2
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Contain Text    @{STATUS_BAR}    Parsed
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/invalid.txt
    Wait Until Region Contains Text    @{STATUS_BAR}    Load error    timeout=5
    Region Should Contain Text    @{STATUS_BAR}    Parsed
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App
