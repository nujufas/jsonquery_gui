*** Settings ***
Documentation     The on-screen keyboard: it must appear for a text field and make
...               room, and what its keys type must reach the field. The keys are
...               tapped for real (not injected), so this exercises ImeView -> JNI
...               -> soft_keyboard -> egui, the path winit does not provide.
Resource          ../../resources/keywords.resource
Force Tags        keyboard
Suite Setup       Warm Up The Keyboard
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-AND-060 Focusing The Query Box Raises The Keyboard
    [Tags]    p1
    Focus Query Box
    Wait For Keyboard

TC-AND-061 The Layout Makes Room For The Keyboard
    [Documentation]    The tab bar must stay visible above the keyboard, not under it.
    [Tags]    p1
    Load Fixture    people.json
    Focus Query Box
    Wait For Keyboard
    ${x}    ${y}=    Find Text On Screen    Results    region=${{[0, 1000, 1080, 300]}}
    Should Be True    ${y} < 1330    the tab bar (y=${y}) is under the keyboard

TC-AND-062 Tapping Keys Types Into The Query Box
    [Documentation]    q, w, e: three real key taps become the query. Running it makes
    ...    the engine complain about `qwe`, in red text OCR reads well -- a check
    ...    that does not depend on reading the caret-adjacent glyphs of the box.
    [Tags]    p1
    Load Fixture    people.json
    Focus Query Box
    Wait For Keyboard
    Tap Keys    ${KEY_Q_X}    ${KEY_W_X}    ${KEY_E_X}
    Tap At    ${RUN_X}    ${RUN_Y}
    Wait Until Screen Contains Text    "qwe"    timeout=10    region=${STATUS_BAND}

TC-AND-063 Backspace Deletes
    [Documentation]    q, w, e, r, Backspace leaves `qwe`: the error names "qwe" and never "qwer".
    [Tags]    p1
    Load Fixture    people.json
    Focus Query Box
    Wait For Keyboard
    Tap Keys    ${KEY_Q_X}    ${KEY_W_X}    ${KEY_E_X}    ${KEY_R_X}
    Tap At    ${KEY_BACKSPACE_X}    ${KEY_BACKSPACE_Y}
    Sleep    0.5s
    Tap At    ${RUN_X}    ${RUN_Y}
    Wait Until Screen Contains Text    "qwe"    timeout=10    region=${STATUS_BAND}
    Screen Should Not Contain Text    qwer    region=${STATUS_BAND}

TC-AND-064 The Keyboard Goes Away When Focus Leaves The Field
    [Tags]    p1
    Load Fixture    people.json
    Focus Query Box
    Wait For Keyboard
    Tap At    540    1200
    Wait Until Keyword Succeeds    10s    0.5s    Keyboard Should Not Be Showing

TC-AND-065 Injected Key Events Type Too
    [Documentation]    A hardware keyboard (or `input text`) goes through winit, not
    ...    ImeView; both paths must work.
    [Tags]    p2
    Focus Query Box
    Type Text    .abc
    # (The text cursor garbles the last glyph in OCR, so read the first three.)
    Wait Until Screen Contains Text    .ab    timeout=8    region=${{[0, 330, 1080, 200]}}

TC-AND-066 The Autocomplete Setting Is In The Menu
    [Tags]    p3
    Open Menu
    Wait Until Screen Contains Text    Autocomplete    timeout=5
