*** Settings ***
Documentation     Browsing a document with touch: expand, long-press menu, the Text
...               view, and scrolling a tall tree.
Resource          ../../resources/keywords.resource
Force Tags        tree_view
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-030 Tapping A Row's Arrow Expands It
    [Tags]    p1
    Load Fixture    people.json
    Tap At    ${ROW1_ARROW_X}    ${ROW1_Y}
    # The values are coloured and read well; the dim key names do not OCR.
    Wait Until Screen Contains Text    Alice    timeout=5
    Screen Should Contain Text    engineer

TC-AND-031 Long Press Opens The Row Menu
    [Documentation]    egui maps a long press to a secondary click.
    [Tags]    p1
    Load Fixture    people.json
    Long Press At    324    ${ROW1_Y}
    Wait Until Screen Contains Text    Copy JSON Path    timeout=5
    Screen Should Contain Text    Search
    Screen Should Contain Text    Save

TC-AND-032 The Text View Shows Pretty-Printed JSON
    [Tags]    p1
    Load Fixture    people.json
    Tap At    ${TEXT_TAB_X}    ${TREE_TAB_Y}
    # The text view is an editor with an Apply button, showing every record.
    Wait Until Screen Contains Text    Apply    timeout=10
    Screen Should Contain Text    Carol
    Screen Should Contain Text    manager

TC-AND-033 Switching Back To The Tree Keeps The Document
    [Tags]    p2
    Load Fixture    people.json
    Tap At    ${TEXT_TAB_X}    ${TREE_TAB_Y}
    Wait Until Screen Contains Text    Apply    timeout=10
    Tap At    90    ${TREE_TAB_Y}
    Sleep    1s
    Screen Should Not Contain Text    Apply
    Screen Should Contain Text    Parsed in

TC-AND-034 A Tall Tree Scrolls With A Swipe
    [Documentation]    60 numbers do not fit on one screen; swiping up brings the
    ...    last ones into view.
    [Tags]    p1
    Load Fixture    many_numbers.json
    Screen Should Not Contain Text    59
    Swipe    540    1500    540    400    duration_ms=250
    Swipe    540    1500    540    400    duration_ms=250
    Swipe    540    1500    540    400    duration_ms=250
    Swipe    540    1500    540    400    duration_ms=250
    Swipe    540    1500    540    400    duration_ms=250
    Wait Until Screen Contains Text    59    timeout=8

TC-AND-035 Copy JSON Path Reaches The Device Clipboard
    [Documentation]    egui's copy is forwarded to Android's clipboard: pasting it
    ...    back through the menu loads it (a bare path is not JSON, so the
    ...    proof is the error that names it).
    [Tags]    p2
    Load Fixture    people.json
    Long Press At    324    ${ROW1_Y}
    Wait Until Screen Contains Text    Copy JSON Path    timeout=5
    Tap Text    Copy JSON Path
    Choose Menu Item    Paste JSON from clipboard
    Wait Until Screen Contains Text    error    timeout=10
