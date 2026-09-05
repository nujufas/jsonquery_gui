*** Settings ***
Documentation     Text view (pretty-printed JSON, and pasted-source editing)
...               -- see test/docs/06_text_view.md. TC-TXT-002/003/005 (long
...               unwrapped lines + horizontal scroll, the transient
...               "Rendering..." indicator, and "no re-render on unrelated
...               state changes") aren't implemented -- each needs either a
...               scroll-position assertion or catching a sub-frame transient
...               state, both too timing-fragile to assert reliably against
...               real wall-clock rendering in this harness.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        text_view
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

*** Keywords ***
Select All And Type
    [Arguments]    ${x}    ${y}    ${text}
    Click At    ${x}    ${y}
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    ${text}
    Sleep    0.2s

Apply And Verify Reload By Content
    [Documentation]    "Parsed" is a poor success signal here: it's already
    ...    showing from the *original* load before Apply ever runs, so
    ...    waiting for it can't distinguish "the click worked" from "the
    ...    click did nothing and this is stale text" (the same staleness trap
    ...    documented for sequential query runs elsewhere in this suite).
    ...    Retries the whole edit-apply-switch-to-Tree-and-check sequence
    ...    instead, verifying success the same way a human would: the new
    ...    value is actually visible in the reloaded tree.
    [Arguments]    ${apply_action}    ${expect_in_tree}
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Apply And Check Tree For    ${apply_action}    ${expect_in_tree}

Apply And Check Tree For
    [Documentation]    Clicking the Text tab only switches view mode -- it
    ...    doesn't focus the textarea itself, so "Press Ctrl Enter" (which
    ...    only fires when the textarea `has_focus()`) needs an explicit
    ...    click *into* the textarea afterward, not just onto its tab.
    [Arguments]    ${apply_action}    ${expect_in_tree}
    Click At    141    130
    Sleep    0.2s
    Click At    200    400
    Sleep    0.1s
    Run Keyword    ${apply_action}
    Sleep    0.3s
    Region Should Not Contain Text    @{STATUS_BAR}    Load error
    Click At    99    130
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_PANEL}    ${expect_in_tree}

Click Apply Button
    Click At    356    160

Click Apply Button And Wait For Load Error
    Click At    356    160
    Wait Until Region Contains Text    @{STATUS_BAR}    Load error    timeout=3

Press Ctrl Enter
    Press Keys    ctrl    enter

*** Test Cases ***
TC-TXT-001 Source And Results Text/Tree Toggles Are Independent
    [Documentation]    Switching one panel's Tree/Text toggle must not affect
    ...    the other's. The Source side is checked via the "Editable..." text
    ...    that only the (pasted) Text view shows; the Results side (whose
    ...    Text view is never editable, so it has no such text of its own) is
    ...    checked via its "Text" tab's own selected-tab highlight pixel,
    ...    the same technique already used for the engine picker and theme
    ...    toggle.
    [Tags]    p1
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}
    ${results_text_tab_baseline}=    Get Pixel Color    744    124
    Click At    141    130
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_PANEL}    Editable
    ${still_baseline}=    Get Pixel Color    744    124
    Colors Should Match    ${results_text_tab_baseline}    ${still_baseline}
    ...    msg=Switching Source to Text should not select Results' Text tab
    Click At    744    130
    Sleep    0.3s
    Region Should Contain Text    @{SOURCE_PANEL}    Editable
    ${results_text_tab_selected}=    Get Pixel Color    744    124
    Colors Should Not Match    ${results_text_tab_baseline}    ${results_text_tab_selected}
    ...    msg=Expected Results' Text tab to visibly highlight once selected
    Click At    99    130
    Sleep    0.3s
    Region Should Not Contain Text    @{SOURCE_PANEL}    Editable
    ${results_text_tab_still_selected}=    Get Pixel Color    744    124
    Colors Should Match    ${results_text_tab_selected}    ${results_text_tab_still_selected}
    ...    msg=Switching Source back to Tree should not affect Results' own toggle

TC-TXT-006 Editing And Applying A Pasted Document's Text Reloads It
    [Tags]    p1
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Click At    141    130
    Sleep    0.3s
    Select All And Type    200    250    {"changed": true, "count": 99}
    Apply And Verify Reload By Content    Click Apply Button    99

TC-TXT-007 Ctrl+Enter Applies The Edited Buffer
    [Tags]    p2
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Click At    141    130
    Sleep    0.3s
    Select All And Type    200    250    {"changed": true, "count": 77}
    Apply And Verify Reload By Content    Press Ctrl Enter    77

TC-TXT-008 Applying Invalid JSON Shows A Load Error And Keeps The Buffer
    [Tags]    p2
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Click At    141    130
    Sleep    0.3s
    Select All And Type    200    250    { this is not json
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Click Apply Button And Wait For Load Error
    Region Should Contain Text    @{SOURCE_PANEL}    this is not json

TC-TXT-010 Apply Is Disabled When The Buffer Is Blank
    [Tags]    p3
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Click At    141    130
    Sleep    0.3s
    ${enabled_color}=    Get Pixel Color    310    160
    Click At    200    250
    Sleep    0.2s
    Press Keys    ctrl    a
    Press Key    delete
    Sleep    0.3s
    ${disabled_color}=    Get Pixel Color    310    160
    Colors Should Not Match    ${enabled_color}    ${disabled_color}
    ...    msg=Expected Apply's label to visibly dim once the buffer went blank

TC-TXT-009 A File Or URL Source's Text View Has No Editable Buffer
    [Documentation]    Only a *pasted* document's Text view is editable --
    ...    file/URL sources render read-only, worker-rendered text instead
    ...    (checked here via a URL source, since Open File needs the blocked
    ...    native dialog).
    [Tags]    p2
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/valid.json
    Wait Until Region Contains Text    @{STATUS_AREA}    valid.json    timeout=5
    Click At    141    130
    Sleep    0.3s
    Region Should Not Contain Text    @{SOURCE_PANEL}    Editable
    Region Should Not Contain Text    @{SOURCE_PANEL}    Apply
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App

TC-TXT-004 A Large Document's Text View Is Capped And Shows A Notice
    [Documentation]    File/URL Text views are bounded to a 20,000-node
    ...    render budget -- exercised via a 25,000-element array over URL
    ...    (paste's Text view is unbounded by design, see 06_text_view.md, so
    ...    this needs a non-pasted source).
    [Tags]    p2
    ${base_url}=    Start Fixture Server    ${HTTP_FIXTURES_DIR}
    Load Via Url    ${base_url}/big_array.json
    Wait Until Region Contains Text    @{STATUS_AREA}    KB    timeout=5
    Click At    141    130
    Sleep    0.5s
    Region Should Contain Text    @{SOURCE_PANEL}    first 20000 nodes
    [Teardown]    Run Keywords    Stop Fixture Server    AND    Close Jsonquery App
