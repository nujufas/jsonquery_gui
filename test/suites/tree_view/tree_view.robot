*** Settings ***
Documentation     Tree view rendering and interaction -- see
...               test/docs/05_tree_view.md. TC-TREE-006 (highlight +
...               centered scroll on reveal) is exercised by the search
...               suite's TC-SRCH-020 instead of duplicated here (same
...               underlying `TreeView::reveal`, cross-ref only). TC-TREE-004
...               (streamed results preserve expand state) needs catching a
...               query mid-stream, which is too timing-dependent to assert
...               reliably against real wall-clock query latency -- not
...               implemented.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        tree_view
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

*** Test Cases ***
TC-TREE-001 Each Value Kind Renders With Its Own Color And Text Shape
    [Documentation]    string/number/bool/null/array/object each get distinct
    ...    treatment -- checked both by OCR'd text shape (quoted string,
    ...    bare number/bool/null, "(N keys)"/"(N items)" containers) and by
    ...    pixel color contrast between the scalar kinds, since color is the
    ...    one part OCR can't read directly.
    [Tags]    p1
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Contain Text    @{SOURCE_PANEL}    jsonquery
    Region Should Contain Text    @{SOURCE_PANEL}    3
    Region Should Contain Text    @{SOURCE_PANEL}    true
    Region Should Contain Text    @{SOURCE_PANEL}    null
    Region Should Contain Text    @{SOURCE_PANEL}    3 items
    ${string_color}=    Get Pixel Color    150    182
    ${number_color}=    Get Pixel Color    136    203
    ${bool_color}=    Get Pixel Color    140    225
    Colors Should Not Match    ${string_color}    ${number_color}
    ...    msg=Expected string and number rows to render in different colors
    Colors Should Not Match    ${number_color}    ${bool_color}
    ...    msg=Expected number and bool rows to render in different colors
    Colors Should Not Match    ${string_color}    ${bool_color}
    ...    msg=Expected string and bool rows to render in different colors

TC-TREE-003 Only The Root Is Expanded By Default
    [Tags]    p2
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Region Should Contain Text    @{SOURCE_PANEL}    tags
    Region Should Not Contain Text    @{SOURCE_PANEL}    gui

TC-TREE-002 A Container Row Toggles Via Its Arrow Or A Double-Click
    [Tags]    p1
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}
    Region Should Not Contain Text    @{SOURCE_PANEL}    Alice
    Click At    28    182
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_PANEL}    Alice
    Click At    28    182
    Sleep    0.3s
    Region Should Not Contain Text    @{SOURCE_PANEL}    Alice
    Double Click At    100    182
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_PANEL}    Alice

TC-TREE-005 Scrolling A Long List Reveals Rows Outside The Initial Viewport
    [Documentation]    A 200-element array is far more rows than fit in the
    ...    panel at once -- confirms the tree actually scrolls (rather than,
    ...    say, silently capping what it draws) by checking a near-the-end
    ...    element is absent before scrolling and present once scrolled all
    ...    the way down. Targets row 197 rather than the true last row (199):
    ...    confirmed during implementation that scrolling this list to its
    ...    max still leaves 198/199 clipped by a few pixels at the very
    ...    bottom of the panel, too little of either row for OCR to read --
    ...    197 is the last row that scrolls fully into view. Targets a fixed
    ...    row near the end (rather than some point further up the middle)
    ...    so "scroll repeatedly until scrolling stops helping" is an
    ...    unambiguous stopping condition -- a target further from the end
    ...    risks a single large scroll step jumping clean over the one
    ...    viewport where it would have been visible, between two of this
    ...    loop's checks.
    [Tags]    p2
    ${items}=    Evaluate    list(range(200))
    ${json}=    Evaluate    __import__("json").dumps(${items})
    Load Fixture Via Paste    ${json}
    Region Should Not Contain Text    @{SOURCE_PANEL}    197:
    FOR    ${i}    IN RANGE    15
        Scroll At    300    400    -60
        Sleep    0.2s
        ${found}=    Run Keyword And Return Status
        ...    Region Should Contain Text    @{SOURCE_PANEL}    197:
        Exit For Loop If    ${found}
    END
    Region Should Contain Text    @{SOURCE_PANEL}    197:
