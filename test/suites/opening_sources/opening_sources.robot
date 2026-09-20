*** Settings ***
Documentation     Opening a JSON source via paste, or by typing a URL or
...               local path into the toolbar's source field (the "..."
...               browse button opens a native file dialog: those cases are
...               BLOCKED -- see test/docs/00_test_strategy.md and
...               02_opening_sources.md). All of them go through the identical
...               load/parse worker path.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        opening_sources
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

*** Test Cases ***
TC-OPEN-008 Paste JSON Auto-Loads On Ctrl+V
    [Documentation]    No explicit submit button -- pasting valid JSON loads
    ...    it immediately, labelled "(pasted JSON)".
    [Tags]    p1
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Contain Text    @{STATUS_AREA}    pasted JSON

TC-OPEN-012 NDJSON Wraps Into One Array
    [Documentation]    Three newline-separated top-level JSON values load as
    ...    a single array; toolbar shows the record count.
    [Tags]    p2
    ${a}=    Get File    ${FIXTURES}/simple_object.json
    ${b}=    Get File    ${FIXTURES}/people.json
    Set Clipboard    ${a}\n{"only":"one"}\n[1,2,3]
    Click At    200    300
    Press Keys    ctrl    v
    Wait Until Region Contains Text    @{STATUS_AREA}    NDJSON    timeout=5
    Region Should Contain Text    @{STATUS_AREA}    3

TC-OPEN-014 Malformed JSON Shows A Load Error
    [Documentation]    Invalid JSON surfaces "Load error: parsing JSON: ..."
    ...    in the status bar; no document loads.
    [Tags]    p1
    Set Clipboard    {"unterminated": "oops"
    Click At    200    300
    Press Keys    ctrl    v
    Wait Until Region Matches    @{STATUS_BAR}    Load error.*parsing JSON    timeout=5

TC-OPEN-003 A URL Typed Into The Source Field Loads A Remote JSON Document
    [Documentation]    Exercised against a local fixture HTTP server rather
    ...    than a real remote host -- avoids a network dependency and lets
    ...    the failure/non-JSON/empty variants below be constructed on demand.
    [Tags]    p1
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/valid.json
    # people.json's 3 array elements are collapsed by default (root-only-
    # expanded, see TC-TREE-003) -- "3 items" is visible without expanding
    # any of them; "Alice" itself is nested a level deeper.
    Wait Until Region Contains Text    @{SOURCE_PANEL}    3 items    timeout=5
    Region Should Contain Text    @{SOURCE_FIELD}    valid.json
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-OPEN-004 Load Button Is Disabled While The Source Field Is Blank
    [Tags]    p3
    ${disabled_color}=    Get Pixel Color    ${LOAD_BUTTON_X}    ${TOOLBAR_Y}
    Click At    ${SOURCE_FIELD_X}    ${TOOLBAR_Y}
    Type Text    x
    Sleep    0.3s
    ${enabled_color}=    Get Pixel Color    ${LOAD_BUTTON_X}    ${TOOLBAR_Y}
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected Load's label to visibly dim while the source field is blank
    Press Keys    ctrl    a
    Press Key    delete
    Sleep    0.3s
    ${blank_again}=    Get Pixel Color    ${LOAD_BUTTON_X}    ${TOOLBAR_Y}
    Colors Should Match    ${disabled_color}    ${blank_again}
    ...    msg=Expected Load to dim again once the field is emptied

TC-OPEN-005 A Failed Request Shows A Load Error
    [Tags]    p1
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/does-not-exist.json
    Wait Until Region Contains Text    @{STATUS_BAR}    Load error    timeout=5
    Region Should Contain Text    @{STATUS_BAR}    404
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-OPEN-006 A Non-JSON Response Shows A Load Error
    [Tags]    p2
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/invalid.txt
    Wait Until Region Contains Text    @{STATUS_BAR}    Load error    timeout=5
    Region Should Contain Text    @{STATUS_BAR}    parsing JSON
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-OPEN-013 An Empty File Loads As An Empty Array, Not An Error
    [Tags]    p2
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/empty.json
    Wait Until Region Contains Text    @{STATUS_AREA}    B    timeout=5
    Region Should Not Contain Text    @{STATUS_BAR}    Load error
    # Not "0 items": confirmed during implementation that a leading digit 0
    # right before other small glyphs (here, an immediately-following "i")
    # can garble past even the general 0/O OCR fallback -- "items" alone
    # still confirms a (necessarily empty) array root rendered, without
    # depending on that specific digit surviving OCR.
    Region Should Contain Text    @{SOURCE_PANEL}    items
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-OPEN-009 Paste Loads Via Ctrl+Enter As Well As Ctrl+V
    [Tags]    p2
    Click At    200    300
    Sleep    0.2s
    Type Text    {"via": "ctrl-enter-not-paste"}
    Press Keys    ctrl    enter
    Wait Until Region Contains Text    @{STATUS_AREA}    pasted JSON    timeout=5

TC-OPEN-015 Loading A New Source Replaces The Old One And Cancels Any Query
    [Documentation]    The query *text* survives (same exception Clear makes,
    ...    see TC-OPEN-016) but its results/error are discarded, since they
    ...    belonged to the document that just got replaced. Uses the source
    ...    field for the second load, not a second paste: pasting only loads
    ...    at all while the empty-state paste box is showing (confirmed
    ...    during implementation -- once a document is loaded, that box no
    ...    longer exists to paste into, so Ctrl+V simply does nothing). The
    ...    source field has no such restriction -- it's always there and
    ...    fully functional regardless of whether a document is already
    ...    loaded, which is exactly the scenario this case is testing.
    [Tags]    p2
    ${people}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${people}
    Run Query    /does/not/exist
    Region Should Contain Text    @{STATUS_BAR}    Query error
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/valid.json
    Wait Until Pasted Source Is Replaced
    Region Should Not Contain Text    @{STATUS_BAR}    Query error
    Query Box Should Contain Text    does/not/exist
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-OPEN-017 Clear Is Disabled When There's Nothing To Clear
    [Tags]    p3
    ${disabled_color}=    Get Pixel Color    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    ${enabled_color}=    Get Pixel Color    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected Clear's label to visibly dim when there is no document, query, or error

TC-OPEN-016 Clear Resets State But Preserves Query Text And Engine
    [Documentation]    Clear wipes the loaded document back to the empty
    ...    state, but the query text box and engine-picker selection survive
    ...    -- the one deliberate exception in the app's own design. Engine
    ...    selection is checked by pixel color (the selected button has a
    ...    tinted background), not just OCR text, since the label itself is
    ...    always visible whether selected or not.
    [Tags]    p1
    ${x}    ${y}=    Find Text In Region    @{ENGINE_PICKER_ROW}    JMESPath
    ${unselected_color}=    Get Pixel Color    ${x}    ${y}
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}
    Select Engine    JMESPath
    ${selected_color}=    Get Pixel Color    ${x}    ${y}
    Colors Should Not Match    ${unselected_color}    ${selected_color}
    ...    msg=Selecting JMESPath did not visibly change its button color
    Click At    100    58
    Type Text    people[0].name
    Click At    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_PANEL}    Paste JSON here
    Query Box Should Contain Text    people
    ${after_clear_color}=    Get Pixel Color    ${x}    ${y}
    Colors Should Match    ${selected_color}    ${after_clear_color}
    ...    msg=JMESPath engine selection was not preserved across Clear

TC-OPEN-018 The Load Button Loads What Is In The Source Field
    [Documentation]    Same load as pressing Enter in the field, by clicking
    ...    Load instead. Nothing is loaded beforehand, so "Parsed in ..."
    ...    appearing is real evidence the click registered.
    [Tags]    p2
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Click At    ${SOURCE_FIELD_X}    ${TOOLBAR_Y}
    Type Text    ${base_url}/valid.json
    Sleep    0.5s
    Click At    ${LOAD_BUTTON_X}    ${TOOLBAR_Y}
    Wait Until Region Contains Text    @{STATUS_BAR}    Parsed    timeout=5
    Region Should Contain Text    @{SOURCE_PANEL}    3 items
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-OPEN-019 A Local Path Typed Into The Source Field Loads That File
    [Documentation]    Anything that isn't an http(s) address is a local
    ...    path. Stands in for the "..." browse button's success path, which
    ...    needs the blocked native file dialog; that button hands its path
    ...    to the same loader.
    [Tags]    p1
    Load Via Url    ${FIXTURES}/people.json
    Wait Until Region Contains Text    @{SOURCE_PANEL}    3 items    timeout=5
    Region Should Contain Text    @{SOURCE_FIELD}    people.json
    Region Should Not Contain Text    @{STATUS_AREA}    pasted JSON

TC-OPEN-020 A Path That Doesn't Exist Shows A Load Error And Keeps The Text
    [Documentation]    The attempted path stays in the field, to be corrected
    ...    rather than retyped.
    [Tags]    p2
    Load Via Url    /no/such/dir/missing.json
    Wait Until Region Contains Text    @{STATUS_BAR}    Load error    timeout=5
    Region Should Contain Text    @{STATUS_BAR}    No such file
    Region Should Contain Text    @{SOURCE_FIELD}    missing.json

TC-OPEN-021 Clear Also Empties Text That Was Typed But Never Loaded
    [Documentation]    With nothing loaded, Clear is still live once the field
    ...    has text in it, and empties just the field.
    [Tags]    p3
    ${disabled_color}=    Get Pixel Color    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    Click At    ${SOURCE_FIELD_X}    ${TOOLBAR_Y}
    Type Text    some-file-name.json
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_FIELD}    some-file-name
    ${enabled_color}=    Get Pixel Color    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected Clear to light up once the field has text
    Click At    ${CLEAR_BUTTON_X}    ${TOOLBAR_Y}
    Sleep    0.3s
    Region Should Not Contain Text    @{SOURCE_FIELD}    some-file-name
