*** Settings ***
Documentation     The built-in tutorial on a phone: a full-screen window that can
...               be closed, one column with an "All lessons" list, and "Try it"
...               handing an example to the main screen.
Resource          ../../resources/keywords.resource
Force Tags        tutorial
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Variables ***
${CLOSE_X}        1010
${CLOSE_Y}        133
${TRY_IT_X}       203
${TRY_IT_Y}       1595

*** Test Cases ***
TC-AND-050 The Tutorial Opens Full Screen
    [Tags]    p1
    Choose Menu Item    Tutorial
    Wait Until Screen Contains Text    Identity    timeout=10
    Screen Should Contain Text    JSONPath
    Screen Should Contain Text    JMESPath
    Screen Should Contain Text    All lessons

TC-AND-051 The Tutorial Can Be Closed
    [Documentation]    An embedded window has no native close button; the tutorial
    ...    draws its own, and it must work.
    [Tags]    p1
    Choose Menu Item    Tutorial
    Wait Until Screen Contains Text    Identity    timeout=10
    Tap At    ${CLOSE_X}    ${CLOSE_Y}
    Wait Until Screen Contains Text    Open file    timeout=8
    Screen Should Not Contain Text    Identity

TC-AND-052 All Lessons Lists The Topics
    [Tags]    p2
    Choose Menu Item    Tutorial
    Wait Until Screen Contains Text    All lessons    timeout=10
    Tap Text    All lessons
    Wait Until Screen Contains Text    Getting started    timeout=8
    Screen Should Contain Text    Back to the lesson

TC-AND-053 The Dialect Tabs Switch Lessons
    [Tags]    p2
    Choose Menu Item    Tutorial
    Wait Until Screen Contains Text    Identity    timeout=10
    Tap Text    JSONPath
    # The lesson changes to the JSONPath one.
    Wait Until Screen Contains Text    Every JSONPath    timeout=8
    Screen Should Not Contain Text    Identity

TC-AND-054 Try It Runs The Example In The Main Screen
    [Documentation]    "Try it" loads the sample and query, runs it, closes the tutorial
    ...    and leaves the result showing.
    [Tags]    p1
    Choose Menu Item    Tutorial
    Wait Until Screen Contains Text    Try it    timeout=10
    Tap Text    Try it
    Wait Until Screen Matches    result\\(s\\)    timeout=15
    Screen Should Not Contain Text    Identity
    Screen Should Contain Text    Results (1)    region=${TAB_BAND}
