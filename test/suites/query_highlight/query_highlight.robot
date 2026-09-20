*** Settings ***
Documentation     Query box colour-coding -- see test/docs/04_query_bar_and_engines.md
...               ("Query highlighting"). Each step of the query sits on a
...               tinted chip, the same palette the tutorial window uses, and
...               the glue between steps stays plain. Checked by pixel colour,
...               not OCR: the tints are flat backgrounds, so a strip of the
...               query row either contains the tint or it doesn't.
Resource          ../../resources/keywords.resource
Force Tags        query_highlight
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Variables ***
# The tutorial's palette at the Dark theme's 85/255 alpha, blended over the
# query box's (10, 10, 10) background -- measured from a real screenshot, and
# the same values the arithmetic gives.
@{ORANGE}    92    55    20
@{BLUE}      30    57    92
@{GREEN}     30    73    47
@{PURPLE}    70    47    92
# The query row: text sits at y 51-67 inside the box, and one monospace
# character is ~7.8px wide starting at x=11 -- so character n spans
# x = 11 + 7.8n. Strips below are inset a few px from each chip's edges.
${ROW_Y}     52
${ROW_H}     14

*** Keywords ***
Type Query
    [Documentation]    Replaces the query box's text WITHOUT running it --
    ...    the tints depend on the text alone, and there is no document
    ...    loaded in these tests. Waits a beat for the repaint.
    [Arguments]    ${query_text}
    Click At    100    58
    Sleep    0.3s
    Press Keys    ctrl    a
    Type Text    ${query_text}
    Sleep    0.5s

*** Test Cases ***
TC-QRY-070 Each Step Of A jq Pipeline Is Tinted In Palette Order
    [Documentation]    `.members[] | select(.age > 30) | .name` splits into
    ...    `.members` (orange), `[]` (blue), `select(...)` (green) and
    ...    `.name` (purple) -- the tutorial's own colouring of the same query.
    [Tags]    p2
    Type Query    .members[] | select(.age > 30) | .name
    Region Should Contain Color    14     ${ROW_Y}    56     ${ROW_H}    @{ORANGE}
    Region Should Contain Color    76     ${ROW_Y}    13     ${ROW_H}    @{BLUE}
    Region Should Contain Color    116    ${ROW_Y}    128    ${ROW_H}    @{GREEN}
    Region Should Contain Color    272    ${ROW_Y}    34     ${ROW_H}    @{PURPLE}

TC-QRY-071 Pipes And Operators Between Steps Stay Plain
    [Documentation]    Only the steps are tinted; the `|` between stages and a
    ...    top-level `>` are glue, as the `|` is in the tutorial.
    [Tags]    p2
    Type Query    .members[] | select(.age > 30) | .name
    Region Should Not Contain Color    93     ${ROW_Y}    16    ${ROW_H}    @{ORANGE}
    Region Should Not Contain Color    93     ${ROW_Y}    16    ${ROW_H}    @{BLUE}
    Region Should Not Contain Color    93     ${ROW_Y}    16    ${ROW_H}    @{GREEN}
    Region Should Not Contain Color    251    ${ROW_Y}    16    ${ROW_H}    @{GREEN}
    Region Should Not Contain Color    251    ${ROW_Y}    16    ${ROW_H}    @{PURPLE}
    Type Query    .a > 1
    Region Should Contain Color        14     ${ROW_Y}    10    ${ROW_H}    @{ORANGE}
    Region Should Not Contain Color    30     ${ROW_Y}    16    ${ROW_H}    @{ORANGE}
    Region Should Not Contain Color    30     ${ROW_Y}    16    ${ROW_H}    @{BLUE}
    Region Should Contain Color        52     ${ROW_Y}    5     ${ROW_H}    @{BLUE}

TC-QRY-072 JSON Pointer Segments Are Each Tinted
    [Documentation]    A pointer has no pipes or calls, so each `/segment` is
    ...    its own chip: `/store` orange, `/book` blue, `/0` green.
    [Tags]    p2
    Type Query    /store/book/0
    Region Should Contain Color    14     ${ROW_Y}    40    ${ROW_H}    @{ORANGE}
    Region Should Contain Color    62     ${ROW_Y}    31    ${ROW_H}    @{BLUE}
    Region Should Contain Color    100    ${ROW_Y}    10    ${ROW_H}    @{GREEN}

TC-QRY-073 A Half-Typed Query Is Still Tinted
    [Documentation]    The box is coloured on every keystroke, so an unclosed
    ...    string and parenthesis must not switch the colouring off: the last
    ...    step simply runs to the end of the text.
    [Tags]    p2
    Type Query    .a | select(.b == "x
    Region Should Contain Color    14    ${ROW_Y}    10     ${ROW_H}    @{ORANGE}
    Region Should Contain Color    54    ${ROW_Y}    108    ${ROW_H}    @{BLUE}

TC-QRY-074 The Engine Picker Changes How The Query Is Split
    [Documentation]    `end` is a jq keyword -- glue, so untinted -- but a
    ...    plain field name in JMESPath. The picker's choice must reach the
    ...    colouring, not just the auto-detected dialect.
    [Tags]    p2
    Type Query    end
    Region Should Not Contain Color    13    ${ROW_Y}    20    ${ROW_H}    @{ORANGE}
    Select Engine    JMESPath
    Sleep    0.5s
    Region Should Contain Color        13    ${ROW_Y}    20    ${ROW_H}    @{ORANGE}

TC-QRY-075 A Call Inside A Call Gets Its Own Tint And The Closer Matches Its Opener
    [Documentation]    `.members | map(select(.active) | .name)` -- the
    ...    tutorial's own example. `map(` is opened up: `map(` is blue,
    ...    `select(.active)` green and `.name` purple, and the closing `)` is
    ...    blue again (the tutorial tints it the same way). Before, the whole
    ...    `map(...)` was one blue chip and the sub-function had no tint.
    [Tags]    p2
    Type Query    .members | map(select(.active) | .name)
    Region Should Contain Color        14     ${ROW_Y}    56     ${ROW_H}    @{ORANGE}
    Region Should Contain Color        100    ${ROW_Y}    24     ${ROW_H}    @{BLUE}
    Region Should Contain Color        134    ${ROW_Y}    108    ${ROW_H}    @{GREEN}
    Region Should Contain Color        272    ${ROW_Y}    32     ${ROW_H}    @{PURPLE}
    Region Should Contain Color        311    ${ROW_Y}    5      ${ROW_H}    @{BLUE}
    Region Should Not Contain Color    311    ${ROW_Y}    5      ${ROW_H}    @{PURPLE}

TC-QRY-076 A Call With No Sub-Function Stays One Chip
    [Documentation]    `map(.a | .b)` holds no function call, so it is one
    ...    orange chip -- the `|` inside it sits on the tint, and nothing
    ...    else is coloured. Only a group with a call inside is opened up.
    [Tags]    p2
    Type Query    map(.a | .b)
    Region Should Contain Color        14     ${ROW_Y}    88     ${ROW_H}    @{ORANGE}
    Region Should Contain Color        60     ${ROW_Y}    10     ${ROW_H}    @{ORANGE}
    Region Should Not Contain Color    14     ${ROW_Y}    88     ${ROW_H}    @{BLUE}
    Region Should Not Contain Color    14     ${ROW_Y}    88     ${ROW_H}    @{GREEN}
