*** Settings ***
Documentation     The parts of the tablet UI around the two panels: the tutorial
...               (its two-column layout, not the phone's single column), the theme
...               button, and the on-screen keyboard, which takes about half of a
...               landscape tablet and must leave the query and results in reach.
Resource          ../../resources/tablet.resource
Force Tags        chrome
Suite Setup       Warm Up The Keyboard
Test Setup        Start The App
Test Teardown     Stop The App

*** Test Cases ***
TC-TAB-040 The Tutorial Shows Its Lesson List Beside The Lesson
    [Documentation]    The phone's tutorial is one column behind an "All lessons"
    ...    button. Wide enough, it lists the topics and the lesson together.
    [Tags]    p1
    Tap From Right    ${TUTORIAL_FROM_RIGHT}    ${TOOLBAR_Y}
    Wait Until Screen Contains Text    Identity    timeout=10
    # The topic list (dim hint text such as "Filter lessons" does not OCR).
    Screen Should Contain Text    Getting started
    Screen Should Contain Text    Nesting
    Screen Should Not Contain Text    All lessons

TC-TAB-041 The Tutorial Can Be Closed
    [Documentation]    An embedded window has no native close button; the tutorial draws
    ...    its own, and it must work.
    [Tags]    p1
    Tap From Right    ${TUTORIAL_FROM_RIGHT}    ${TOOLBAR_Y}
    Wait Until Screen Contains Text    Identity    timeout=10
    Tap From Right    ${CLOSE_FROM_RIGHT}    ${TUTORIAL_CLOSE_Y}
    Wait Until Screen Contains Text    Open file    timeout=8
    Screen Should Not Contain Text    Identity

TC-TAB-042 The Dialect Tabs Switch Lessons
    [Tags]    p2
    Tap From Right    ${TUTORIAL_FROM_RIGHT}    ${TOOLBAR_Y}
    Wait Until Screen Contains Text    Identity    timeout=10
    Tap Text    JSONPath
    Wait Until Screen Contains Text    Every JSONPath    timeout=8
    Screen Should Not Contain Text    Identity

TC-TAB-043 Try It Runs The Example In The Main Screen
    [Documentation]    "Try it" loads the sample and query, runs it, closes the tutorial
    ...    and leaves both panels showing, evenly split.
    [Tags]    p1
    Tap From Right    ${TUTORIAL_FROM_RIGHT}    ${TOOLBAR_Y}
    Wait Until Screen Contains Text    Try it    timeout=10
    Tap Text    Try it
    Wait Until Screen Matches    result\\(s\\)    timeout=15
    Screen Should Not Contain Text    Identity
    Split Should Be Even

TC-TAB-050 The Theme Button Flips Light And Dark
    [Documentation]    The toolbar's sun/moon button (the phone has it in the menu):
    ...    the app starts dark, and the button switches the whole UI.
    [Tags]    p2
    Load Fixture    people.json
    ${width}    ${height}=    Get Screen Size
    ${x}=    Evaluate    ${width} // 4
    ${y}=    Evaluate    int(${height} * 0.7)
    ${dark}=    Get Pixel Color    ${x}    ${y}
    Tap From Right    ${THEME_FROM_RIGHT}    ${TOOLBAR_Y}
    Sleep    1s
    ${light}=    Get Pixel Color    ${x}    ${y}
    Should Be True    ${light}[0] > ${dark}[0] + 100    the background did not get lighter: ${dark} -> ${light}
    Tap From Right    ${THEME_FROM_RIGHT}    ${TOOLBAR_Y}
    Sleep    1s
    ${again}=    Get Pixel Color    ${x}    ${y}
    Should Be True    ${again}[0] < 90    the background did not go back to dark: ${again}

TC-TAB-060 Focusing The Query Box Raises The Keyboard
    [Tags]    p1
    Focus Query Box
    Wait For Keyboard

TC-TAB-061 The Layout Makes Room For The Keyboard
    [Documentation]    The keyboard covers the lower half of a landscape tablet. The
    ...    status line, and with it the bottom of both panels, must be
    ...    above it, and the query box must still be on screen.
    [Tags]    p1
    Load Fixture    people.json
    Run Query    .[] | .name
    Focus Query Box
    Wait For Keyboard
    ${top}=    Find First Different Pixel Below    10    300    tolerance=100
    ${x}    ${y}=    Find Text On Screen    Query ran
    Should Be True    ${y} < ${top}    the status line (y=${y}) is under the keyboard (top y=${top})
    Screen Should Contain Text    Query:
    Screen Should Contain Text    Results

TC-TAB-062 Tapping Keys Types Into The Query Box
    [Documentation]    q, w, e: three real key taps become the query. Running it makes
    ...    the engine complain about `qwe`, in red text OCR reads well. The
    ...    keys are tapped for real (not injected), so this goes through
    ...    ImeView -> JNI -> soft_keyboard -> egui.
    [Tags]    p1
    Load Fixture    people.json
    Focus Query Box
    Wait For Keyboard
    Tap At    ${KEY_Q_X}    ${KEY_ROW1_Y}
    Sleep    0.4s
    Tap At    ${KEY_W_X}    ${KEY_ROW1_Y}
    Sleep    0.4s
    Tap At    ${KEY_E_X}    ${KEY_ROW1_Y}
    Sleep    0.4s
    Tap At    ${RUN_X}    ${RUN_Y}
    ${band}=    Status Band
    Wait Until Screen Contains Text    "qwe"    timeout=10    region=${band}

TC-TAB-063 The Keyboard Goes Away When Focus Leaves The Field
    [Tags]    p1
    Load Fixture    people.json
    Focus Query Box
    Wait For Keyboard
    Tap At    1500    ${TOOLBAR_Y}
    Wait Until Keyword Succeeds    10s    0.5s    Keyboard Should Not Be Showing
