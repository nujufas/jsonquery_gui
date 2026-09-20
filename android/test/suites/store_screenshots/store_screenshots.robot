*** Settings ***
Documentation     Not a test: takes the phone screenshots for the Google Play listing
...               (android/play/metadata/android/en-US/images/phoneScreenshots) from the
...               real app on the emulator, so they are always current. The status bar is
...               put in Android's demo mode (12:00, full battery, no notifications).
...               Run with: android/scripts/store-screenshots.sh
Resource          ../../resources/keywords.resource
Force Tags        screenshots
Suite Setup       Prepare For Store Screenshots
Test Setup        Start The App
Test Teardown     Stop The App

*** Variables ***
${QUERY}          .orders[] | select(.total > 50)
${RESULT_ROW1_Y}    874
${DEPTH2_ARROW_X}    136

*** Keywords ***
Prepare For Store Screenshots
    Install The App Once
    Enable Clean Status Bar

Open The Demo Orders
    [Documentation]    The demo document, opened as a file named orders.json (so the
    ...    toolbar shows a real file name rather than "(pasted JSON)").
    ${path}=    Push Fixture    ${FIXTURES}/store_demo.json    orders.json
    Open File In App    ${path}
    Wait Until Screen Contains Text    orders.json    timeout=15

Show The Orders Query
    [Documentation]    The demo document with a query run: the results, first one opened up.
    Open The Demo Orders
    Run Query    ${QUERY}
    Sleep    1s
    Tap At    ${ROW1_ARROW_X}    ${RESULT_ROW1_Y}
    Sleep    1s

*** Test Cases ***
Screenshot 1 Query Results
    Show The Orders Query
    Wait Until Screen Contains Text    2 result(s)
    Save Store Screenshot    1-query-results

Screenshot 2 Source Tree
    [Documentation]    The document browsed as a tree: the orders array opened, then its first order.
    Open The Demo Orders
    Tap At    ${ROW1_ARROW_X}    1113
    Sleep    1s
    Tap At    ${DEPTH2_ARROW_X}    1233
    Sleep    1s
    Save Store Screenshot    2-source-tree

Screenshot 3 Row Menu
    [Documentation]    Long-press a row: copy its JSON path, save it, find it in the source, search.
    Open The Demo Orders
    Run Query    ${QUERY}
    Sleep    1s
    Long Press At    324    ${RESULT_ROW1_Y}
    Wait Until Screen Contains Text    Copy JSON Path    timeout=5
    Save Store Screenshot    3-row-menu

Screenshot 4 Tutorial
    [Documentation]    The built-in tutorial, full screen, with a runnable example.
    Choose Menu Item    Tutorial
    Wait Until Screen Contains Text    Identity    timeout=10
    Save Store Screenshot    4-tutorial

Screenshot 5 Light Theme
    [Documentation]    The same results in the light theme.
    Show The Orders Query
    Choose Menu Item    Light theme
    Sleep    1.5s
    Save Store Screenshot    5-light-theme
