*** Settings ***
Documentation     Android's own file pickers, which the desktop suite could never
...               drive: Open file... and Save... go through the system
...               "documents" UI, automated with uiautomator, and the results are
...               checked on the device's file system.
Resource          ../../resources/keywords.resource
Force Tags        pickers
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-070 Open File Picks A Document And Loads It
    [Tags]    p1
    ${path}=    Push To Downloads    ${FIXTURES}/people.json    picker_people.json
    Choose Menu Item    Open file
    Wait Until Picker Is Showing
    Wait Until UI Element Appears    picker_people.json    timeout=60
    Tap UI Element    picker_people.json
    Wait Until Screen Contains Text    picker_people.json    timeout=15
    Screen Should Contain Text    Parsed in
    [Teardown]    Run Keywords    Remove Device File    /sdcard/Download/picker_people.json    AND    Stop The App

TC-AND-071 Cancelling The Picker Leaves The App As It Was
    [Tags]    p1
    Choose Menu Item    Open file
    Wait Until Picker Is Showing
    Press Key    BACK
    Wait Until Screen Contains Text    Open file    timeout=10

TC-AND-072 Save Writes The Results Where The User Chose
    [Documentation]    Long-press a results row, Save..., accept the suggested name in
    ...    the system picker, and find the pretty-printed JSON on the device.
    [Tags]    p1
    Remove Device File    /sdcard/Download/item_0.json
    Load Fixture    people.json
    Run Query    [.[0]]
    Long Press At    324    ${ROW1_Y}
    # The row menu's "Save…" (the results header has one too, higher up).
    Wait Until Screen Contains Text    Copy JSON Path    timeout=5
    Tap Text    Save    region=${{[0, 900, 1080, 500]}}
    Wait Until Picker Is Showing
    Wait Until UI Element Appears    item_0.json    timeout=15
    Tap UI Element    SAVE
    # The file on the device is the proof (the "Saved" toast is gone within seconds).
    Device File Should Contain    /sdcard/Download/item_0.json    "name": "Alice"
    [Teardown]    Run Keywords    Remove Device File    /sdcard/Download/item_0.json    AND    Stop The App

TC-AND-073 Saving The Whole Source Suggests The File's Name
    [Tags]    p2
    Remove Device File    /sdcard/Download/data.json
    Load Fixture    people.json
    Tap Text    Save    occurrence=1
    Wait Until Picker Is Showing
    Wait Until UI Element Appears    data.json    timeout=15
    Tap UI Element    SAVE
    Device File Should Contain    /sdcard/Download/data.json    "role": "engineer"
    [Teardown]    Run Keywords    Remove Device File    /sdcard/Download/data.json    AND    Stop The App
