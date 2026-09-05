*** Settings ***
Documentation     Saving -- see test/docs/09_saving.md. Every case that needs
...               the native Save dialog to actually complete (TC-SAVE-001/
...               003/004/006/007/008/009/010) is BLOCKED -- see
...               00_test_strategy.md. The two cases below don't need the
...               dialog to open at all -- just its trigger button's
...               enabled/disabled state -- so they're implemented here
...               rather than left blocked.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        saving
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

*** Test Cases ***
TC-SAVE-002 Source Save... Stays Available Regardless Of Query State
    [Documentation]    Unlike the Results header's Save... (TC-SAVE-005), the
    ...    Source header's Save... has no enabled-state gating on the query
    ...    at all -- checked here by confirming its label's color never
    ...    changes across "no query run yet" and "a query just ran".
    [Tags]    p3
    ${before_query}=    Get Pixel Color    566    129
    Run Query    .[0].name
    ${after_query}=    Get Pixel Color    566    129
    Colors Should Match    ${before_query}    ${after_query}
    ...    msg=Expected Source's Save... to look the same regardless of query state

TC-SAVE-005 Results Save... Is Disabled Until There Are Results To Save
    [Tags]    p1
    ${disabled_color}=    Get Pixel Color    1165    129
    Run Query    .[0].name
    ${enabled_color}=    Get Pixel Color    1165    129
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected Results' Save... to visibly enable once a query produced results
