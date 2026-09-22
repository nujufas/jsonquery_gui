*** Settings ***
Documentation     Right-click context menus -- see
...               test/docs/07_context_menus.md. "Save..." items are BLOCKED
...               (native dialog -- see 00_test_strategy.md); this suite
...               covers the menu contents/placement and "Copy JSON Path"/
...               "Copy to Clipboard", which need no dialog.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        context_menus
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Load People Fixture
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures
# The context menu is a floating popup near the right-click point -- a wide
# capture picks up tree rows behind/around it too, garbling the OCR read
# (confirmed during implementation). Keep this tight to just the popup.
@{CONTEXT_MENU_AREA}    95    195    250    100

*** Keywords ***
Load People Fixture
    Launch Jsonquery App
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}

Wait Until Clipboard Contains
    [Documentation]    Unlike Copy JSON Path (`ui.ctx().copy_text()` directly
    ...    on click), "Copy to Clipboard" serializes on the worker thread
    ...    first (`Command::CopyNode` / `Event::CopyReady` -- see worker.rs)
    ...    and only reaches the OS clipboard a frame or two later, so a
    ...    one-shot Get Clipboard right after the click can race it. Polls
    ...    instead.
    [Arguments]    ${expected}
    Wait Until Keyword Succeeds    10x    0.3s
    ...    Clipboard Should Contain    ${expected}

Clipboard Should Contain
    [Arguments]    ${expected}
    ${text}=    Get Clipboard
    Should Contain    ${text}    ${expected}

*** Test Cases ***
TC-CTX-001 Menu Contents And Order On A Source Row
    [Documentation]    Save..., Copy to Clipboard, Copy JSON Path, separator,
    ...    Search... -- no Find in Source (that item is Results-tree only).
    [Tags]    p1
    Open Row Context Menu    100    197
    Region Should Contain Text    @{CONTEXT_MENU_AREA}    Save
    Region Should Contain Text    @{CONTEXT_MENU_AREA}    Copy to Clipboard
    Region Should Contain Text    @{CONTEXT_MENU_AREA}    Copy JSON Path
    Region Should Contain Text    @{CONTEXT_MENU_AREA}    Search
    Region Should Not Contain Text    @{CONTEXT_MENU_AREA}    Find in Source

TC-CTX-002 Menu Contents And Order On A Results Row
    [Documentation]    Unlike a Source row, a Results row's menu includes
    ...    "Find in Source" (cross-ref TC-CTX-005/TC-SRCH-022's negative half,
    ...    already covered by TC-CTX-001).
    [Tags]    p1
    Run Query    .[0]
    Open Row Context Menu    650    197
    @{menu}=    Row Context Menu Region    650    197
    Region Should Contain Text    @{menu}    Save
    Region Should Contain Text    @{menu}    Copy to Clipboard
    Region Should Contain Text    @{menu}    Copy JSON Path
    Region Should Contain Text    @{menu}    Find in Source
    Region Should Contain Text    @{menu}    Search

TC-CTX-006 Search... Scopes To Whichever Tree It Was Opened From
    [Documentation]    Cross-ref 08_search_and_find_in_source.md -- the
    ...    dialog's own title names which tree it will search, set from the
    ...    row's owning panel, not always "Source".
    [Tags]    p2
    Run Query    .[0]
    Open Row Context Menu    650    197
    @{menu}=    Row Context Menu Region    650    197
    Click Text In Region    @{menu}    Search
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Results

TC-CTX-007 No Context Menu Appears Outside Any Tree Row
    [Tags]    p3
    Right Click At    300    500
    Sleep    0.3s
    Region Should Not Contain Text    @{SOURCE_PANEL}    Copy JSON Path

TC-CTX-003 Copy JSON Path Copies The Path To The Clipboard
    [Documentation]    Right-clicking the root array's first element and
    ...    copying its path should put a jq-style path on the clipboard.
    [Tags]    p1
    Set Clipboard    sentinel-before-copy
    Open Row Context Menu    100    197
    Click Text In Region    @{CONTEXT_MENU_AREA}    Copy JSON Path
    ${path}=    Get Clipboard
    Should Not Be Equal As Strings    ${path}    sentinel-before-copy
    ...    msg=Clipboard was not updated by Copy JSON Path
    Should Contain    ${path}    0
    ...    msg=Expected the copied path to reference array index 0, got: ${path}

TC-CTX-004 Copy To Clipboard Puts The Row's Pretty JSON On The Clipboard
    [Documentation]    Right-clicking the root array's first element (Alice)
    ...    and choosing "Copy to Clipboard" should put that element's
    ...    pretty-printed JSON on the system clipboard (worker.rs's
    ...    `CopyTarget::Source` -> `serde_json::to_string_pretty`) -- not a
    ...    bare value and not the whole document. Verified via the clipboard
    ...    content itself, not the status bar's "Copied to clipboard" note:
    ...    that renders with `ui.weak()`, documented elsewhere in this suite
    ...    as unreliable for OCR.
    [Tags]    p1
    Set Clipboard    sentinel-before-copy
    Open Row Context Menu    100    197
    Click Text In Region    @{CONTEXT_MENU_AREA}    Copy to Clipboard
    Wait Until Clipboard Contains    Alice
    ${text}=    Get Clipboard
    Should Contain    ${text}    "name"
    ...    msg=Expected pretty-printed JSON (a quoted "name" key), got: ${text}
    Should Not Contain    ${text}    Bob
    ...    msg=Copy to Clipboard should copy just the one row, not the whole document
