*** Settings ***
Documentation     Getting a document into the app the ways an Android user has:
...               the share sheet (SEND), "Open with" (VIEW) and bad input. The
...               system pickers themselves are in suites/pickers.
Resource          ../../resources/keywords.resource
Library           Process
Force Tags        opening
Suite Setup       Install The App Once
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-010 Shared Text Opens As A Document
    [Documentation]    JSON shared from another app becomes the source document.
    [Tags]    p1
    Load Fixture    people.json
    Screen Should Contain Text    pasted JSON
    Screen Should Contain Text    Parsed in

TC-AND-011 Open With Opens A File And Names It
    [Documentation]    A file opened from a file manager ("Open with") loads, and
    ...    the toolbar shows its name, not the private cache path it was copied to.
    [Tags]    p1
    ${device_path}=    Push Fixture    ${FIXTURES}/people.json
    Open File In App    ${device_path}
    Wait Until Screen Contains Text    people.json    timeout=15
    Screen Should Contain Text    Parsed in
    Screen Should Not Contain Text    inbox
    Screen Should Not Contain Text    cache

TC-AND-012 Invalid JSON Is Reported
    [Tags]    p1
    Share Text To App    {this is not json
    Wait Until Screen Contains Text    error    timeout=15

TC-AND-013 Sharing To A Running App Replaces The Document
    [Documentation]    A second share while the app is open (singleTask) reaches
    ...    the running instance.
    [Tags]    p1
    Load Fixture    people.json
    Share Text To App    {"only_key": 1}
    # {"only_key": 1} is 15 bytes (OCR reads "15 B" as "158"); people.json was 154.
    Wait Until Screen Matches    15\\s?[B8]+    timeout=15    region=${{[0, 60, 1080, 120]}}

TC-AND-014 Clear Removes The Document
    [Tags]    p2
    Load Fixture    people.json
    Choose Menu Item    Clear
    Wait Until Screen Contains Text    Open file    timeout=10

TC-AND-015 A Document Given At Launch Is Loaded
    [Documentation]    The very first intent (before any UI exists) must not be lost.
    [Tags]    p1
    ${text}=    Fixture Text    simple_object.json
    Launch App With Text    ${text}
    Wait Until Screen Contains Text    Parsed in    timeout=20

TC-AND-016 The Open URL Dialog Opens From The Empty State
    [Documentation]    The three ways in are real buttons, tapped by their text.
    [Tags]    p2
    Tap Text    Open URL
    Wait Until Screen Contains Text    Cancel    timeout=10
    Screen Should Contain Text    Load

TC-AND-017 A Document Can Be Loaded From A URL
    [Documentation]    "Open URL" downloads the document -- the app's one use of the
    ...    INTERNET permission, and of a writable temp directory. A tiny HTTP
    ...    server runs in the emulator's own container; the emulator reaches that
    ...    container's loopback as 10.0.2.2. The URL field takes focus (and the
    ...    keyboard rises) when the dialog opens.
    [Tags]    p1
    Start Process    python3    -m    http.server    8765    --directory    ${FIXTURES}    alias=fixtures
    Sleep    1s
    Tap Text    Open URL
    Wait Until Screen Contains Text    Cancel    timeout=10
    Type Text    http://10.0.2.2:8765/people.json
    Tap Text    Load
    Wait Until Screen Contains Text    people.json    timeout=20
    Screen Should Contain Text    Parsed in
    [Teardown]    Run Keywords    Terminate All Processes    kill=True    AND    Stop The App
