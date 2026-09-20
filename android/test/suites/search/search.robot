*** Settings ***
Documentation     Find-in-document on a phone: the dialog from the menu, stepping through
...               matches and listing them all. (The engine is the desktop's; these
...               check the touch entry points.)
Resource          ../../resources/keywords.resource
Force Tags        search
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-040 Search Opens From The Menu
    [Tags]    p1
    Load Fixture    people.json
    Choose Menu Item    Search
    Wait Until Screen Contains Text    Find    timeout=8
    Screen Should Contain Text    Regex

TC-AND-041 Find Steps To A Match And Counts It
    [Tags]    p1
    Load Fixture    people.json
    Choose Menu Item    Search
    Wait Until Screen Contains Text    Regex    timeout=8
    Type Text    Bob
    Tap Text    Find    occurrence=2
    Wait Until Screen Matches    1 of 1    timeout=10

TC-AND-042 Find All Lists Every Match
    [Tags]    p1
    Load Fixture    sites.json
    Choose Menu Item    Search
    Wait Until Screen Contains Text    Regex    timeout=8
    Type Text    75001
    Tap Text    Find All
    Wait Until Screen Matches    2 match|matches    timeout=10

TC-AND-043 A Missing Word Is Reported
    [Tags]    p2
    Load Fixture    people.json
    Choose Menu Item    Search
    Wait Until Screen Contains Text    Regex    timeout=8
    Type Text    zzz
    Tap Text    Find    occurrence=2
    Wait Until Screen Contains Text    No matches    timeout=10
