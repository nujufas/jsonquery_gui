*** Settings ***
Documentation     Running queries on a phone: typing into the query box, the
...               four engines, engine choice, errors, and the switch to the Results
...               tab. Query text is typed as key events (see suites/keyboard for the
...               on-screen keyboard's own path).
Resource          ../../resources/keywords.resource
Force Tags        queries
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-020 A jq Path Query Runs
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[0].name
    Screen Should Contain Text    Alice
    Status Should Contain    1 result(s)

TC-AND-021 A jq Filter With A Comparison Runs
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[] | select(.age > 30) | .name
    Screen Should Contain Text    Alice
    Screen Should Contain Text    Carol
    Screen Should Not Contain Text    Bob
    Status Should Contain    2 result(s)

TC-AND-022 A JSON Pointer Query Runs
    [Tags]    p1
    Load Fixture    people.json
    Run Query    /1/name
    Screen Should Contain Text    Bob
    Status Should Contain    Pointer

TC-AND-023 A JSONPath Query Runs
    [Tags]    p1
    Load Fixture    people.json
    Run Query    $[*].name
    Status Should Contain    3 result(s)
    Status Should Contain    JSONPath

TC-AND-024 A JMESPath Query Runs
    [Tags]    p1
    Load Fixture    people.json
    Run Query    [?age > `30`].name
    Status Should Contain    JMESPath
    # One result: the array of matching names (collapsed in the tree).
    Status Should Contain    1 result(s)

TC-AND-025 Choosing An Engine Overrides Auto-Detection
    [Documentation]    With a chip picked the status names the engine without "auto".
    [Tags]    p1
    Load Fixture    people.json
    Choose Engine    ${CHIP_JQ_X}
    Run Query    .[0].name
    Status Should Contain    [jq]
    ${status}=    Read Screen Text    ${STATUS_BAND}
    Should Not Contain    ${status.lower()}    auto    the engine was chosen, not detected: ${status}

TC-AND-026 A Bad Query Reports An Error
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[
    Screen Should Contain Text    error

TC-AND-027 Running A Query Switches To The Results Tab
    [Documentation]    After Run the Results tab is selected and counts the results.
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[].name
    Screen Should Contain Text    Results (3)    region=${TAB_BAND}

TC-AND-028 The Source Tab Is Still One Tap Away
    [Tags]    p2
    Load Fixture    people.json
    Run Query    .[].name
    Tab Should Be Selected    Results
    Show Source Tab
    Sleep    1s
    Tab Should Be Selected    Source

TC-AND-029 Run Is Disabled Until A Document Is Loaded
    [Documentation]    With no document the Run button is dimmed and does nothing.
    [Tags]    p2
    Focus Query Box
    Type Text    .
    Tap At    ${RUN_X}    ${RUN_Y}
    Sleep    1s
    Screen Should Not Contain Text    result(s)
