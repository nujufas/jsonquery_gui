*** Settings ***
Documentation     "Search..." (a Notepad++-style Find dialog: Find steps through
...               matches, Find All lists them) and "Find in Source" -- see
...               test/docs/08_search_and_find_in_source.md. TC-SRCH-006 (the
...               5,000-match cap) and TC-SRCH-007's load/query sub-cases
...               aren't implemented -- constructing a fixture with 5,000+
...               matches and reliably distinguishing "search invalidated"
...               from "dialog just not showing it yet" added more cost than
...               value here; the Clear sub-case of TC-SRCH-007 is cheap and is
...               included below.
...
...               "Find" reports in the dialog itself (the status line under
...               the buttons: "N of M", "No matches found.", "Search error:
...               ..."), and reveals each match in its tree -- so those checks
...               read that status line (normal-contrast text, reliable for
...               OCR) and look for the revealed row's highlight by pixel
...               (`Source Row Is Highlighted`). "Find All" fills the bottom
...               panel with a hit list, which is read like the Find in Source
...               candidate list (same panel, same fixed row geometry).
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        search
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Load People Fixture
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures
# Centers of the first two lines of the bottom "Find in Source" list, at 18px
# stride -- the panel always has at least one candidate and keeps its default
# height. Fixed rather than found by OCR: the *selected* line sits on the blue highlight, and OCR was
# confirmed to sometimes drop that line's "[Source]" token, returning the
# next line's position instead -- so a test could misjudge which line is
# selected.
${HIT_ROW_1_Y}    637
${HIT_ROW_2_Y}    655
${HIT_ROW_3_Y}    673
${HIT_TEXT_X}     60

*** Keywords ***
Load People Fixture
    Launch Jsonquery App
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}

Open Source Search Dialog
    Click At    200    300
    Sleep    0.2s
    Press Keys    ctrl    f
    Sleep    0.3s

Search For
    [Documentation]    Types `${text}` into an already-open Search dialog's
    ...    Find field and submits it via Find -- retrying the whole type+click
    ...    if Find doesn't visibly register (the same click-right-after-typing
    ...    timing flakiness as Load Via Url). Waits on the dialog's status
    ...    line, which reports every outcome. It goes blank the moment the
    ...    text changes (the old outcome was for the old text), so a stale
    ...    "1 of 1" from a previous search can't satisfy the wait. Any digit
    ...    counts, since OCR reads the "of" of "2 of 4" as "0F" now and then --
    ...    and the wait must not fail on a Find that did register, because
    ...    the retry would press Find again and step on to the next match.
    [Arguments]    ${text}
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Type And Submit Search    ${text}

Type And Submit Search
    [Arguments]    ${text}
    Click At    ${SEARCH_FIND_FIELD_X}    ${SEARCH_FIND_FIELD_Y}
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    ${text}
    Sleep    0.3s
    Click At    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    Wait Until Region Matches    @{SEARCH_STATUS_AREA}    \\d|of|found|error    timeout=3

Click Find
    [Documentation]    One click on the Find button, then a moment for the
    ...    next frame to draw its outcome.
    Click At    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    Sleep    0.4s

Find All For
    [Documentation]    Types `${text}` into an already-open Search dialog's
    ...    Find field and submits it via Find All -- retrying the whole
    ...    type+click if Find All doesn't visibly register (the same
    ...    click-right-after-typing timing flakiness as Load Via Url).
    ...    Waits on the panel heading "Search results" rather than the
    ...    weak-styled match count, since that reads reliably regardless of
    ...    hit count. (A heading left over from an earlier Find All would
    ...    satisfy that too, so a test doing two in a row must check the
    ...    second list's content instead.) Unlike Find, repeating a Find All
    ...    is harmless: it lists the same matches again.
    [Arguments]    ${text}
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Type And Submit Find All    ${text}

Type And Submit Find All
    [Arguments]    ${text}
    Click At    ${SEARCH_FIND_FIELD_X}    ${SEARCH_FIND_FIELD_Y}
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    ${text}
    Sleep    0.3s
    Click At    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Search results    timeout=3

Source Row Is Highlighted
    [Documentation]    Whether the Source tree row whose center is at
    ...    `${row_y}` is drawn with the "revealed" highlight: its far-right,
    ...    text-free end (x=580) either carries the highlight tint or the
    ...    plain panel background, compared against a point well below the
    ...    tree, which is always plain background. Fixed pixels rather than
    ...    OCR -- a highlighted row is exactly where OCR drops text.
    [Arguments]    ${row_y}
    ${row}=    Get Pixel Color    580    ${row_y}
    ${plain}=    Get Pixel Color    580    700
    ${diff}=    Evaluate    max(abs(a - b) for a, b in zip($row, $plain))
    RETURN    ${diff > 6}

Highlighted Source Row Should Be
    [Documentation]    Exactly the row at `${row_y}` is highlighted, out of
    ...    the `${candidates}` rows given (a Find only ever highlights one).
    [Arguments]    ${row_y}    @{candidates}
    FOR    ${y}    IN    @{candidates}
        ${lit}=    Source Row Is Highlighted    ${y}
        IF    ${y} == ${row_y}
            Should Be True    ${lit}    msg=Expected the Source row at y=${y} to be highlighted
        ELSE
            Should Not Be True    ${lit}    msg=Expected the Source row at y=${y} NOT to be highlighted
        END
    END

Load Fixture Over People
    [Documentation]    Swaps the People fixture (loaded by Test Setup) for
    ...    `${name}` via Clear + paste, same as TC-SRCH-002 does for
    ...    simple_object.json.
    [Arguments]    ${name}
    Click At    199    11
    Sleep    0.3s
    ${json}=    Get File    ${FIXTURES}/${name}
    Load Fixture Via Paste    ${json}

Load Duplicates Fixture
    [Documentation]    duplicates.json -- three users, two of them
    ...    `"country": "US"`.
    Load Fixture Over People    duplicates.json

Load Sites Fixture
    [Documentation]    sites.json -- the zips `75001` (sites[0], sites[2]) and
    ...    `00100` (sites[1]) each also occur under another key: `depots[2].code`
    ...    (same value, same array index as sites[2]) and `hq.code`, both listed
    ...    before `sites` in the document. Only the row's key tells them apart.
    Load Fixture Over People    sites.json

Find In Source On Result Row
    [Documentation]    Runs `${query}`, right-clicks the Results row at
    ...    (650, ${row_y}) and chooses Find in Source, waiting for either
    ...    outcome to show up: the bottom panel's "Find in Source" heading (a
    ...    list), or -- for a single exact hit, which is revealed directly
    ...    with no panel -- nothing further to wait on.
    [Arguments]    ${query}    ${row_y}
    Run Query    ${query}
    Open Row Context Menu    650    ${row_y}
    @{menu}=    Row Context Menu Region    650    ${row_y}
    Click Text In Region    @{menu}    Find in Source

Find In Source On Nested Result Row
    [Documentation]    Runs `${query}`, expands the Results row at
    ...    (650, ${output_y}) and chooses Find in Source on the child row at
    ...    (650, ${child_y}). Expansion is a double-click (a toggle), so it is
    ...    verified -- via `${marker}`, a child's text that a collapsed
    ...    `{...} (2 keys)` row never shows -- before ever being repeated.
    [Arguments]    ${query}    ${output_y}    ${child_y}    ${marker}
    Run Query    ${query}
    FOR    ${attempt}    IN RANGE    3
        ${expanded}=    Run Keyword And Return Status
        ...    Region Should Contain Text    @{RESULTS_PANEL}    ${marker}
        IF    ${expanded}    BREAK
        Double Click At    650    ${output_y}
        Sleep    0.4s
    END
    Region Should Contain Text    @{RESULTS_PANEL}    ${marker}
    Open Row Context Menu    650    ${child_y}
    @{menu}=    Row Context Menu Region    650    ${child_y}
    Click Text In Region    @{menu}    Find in Source

Hit List Row Is Highlighted
    [Documentation]    Whether the bottom hit-list line whose center is at
    ...    `${row_y}` is drawn selected: its far-right, text-free end
    ...    (x=1100) either carries the highlight tint or the plain panel
    ...    background, compared against a point (y=750) below the last line
    ...    of any list these tests make -- so always plain background.
    [Arguments]    ${row_y}
    ${row}=    Get Pixel Color    1100    ${row_y}
    ${plain}=    Get Pixel Color    1100    750
    ${diff}=    Evaluate    max(abs(a - b) for a, b in zip($row, $plain))
    RETURN    ${diff > 6}

Close Search Results Panel And Verify
    [Documentation]    Retries the Close click itself for the same reason as
    ...    Search For above. `${heading}` is the panel heading that must be
    ...    gone afterwards.
    [Arguments]    ${heading}=Search results
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Click Close And Verify Panel Gone    ${heading}

Click Close And Verify Panel Gone
    [Arguments]    ${heading}
    Close Search Results Panel
    Sleep    0.2s
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    ${heading}

*** Test Cases ***
TC-SRCH-001 Search Dialog Shows Its Fields And Buttons
    [Tags]    p2
    Open Source Search Dialog
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Search
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Source
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Find
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Regex
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Find All
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Cancel
    ${find_disabled}=    Get Pixel Color    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    ${find_all_disabled}=    Get Pixel Color    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Click At    ${SEARCH_FIND_FIELD_X}    ${SEARCH_FIND_FIELD_Y}
    Type Text    x
    Sleep    0.2s
    ${find_enabled}=    Get Pixel Color    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    ${find_all_enabled}=    Get Pixel Color    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Colors Should Not Match    ${find_disabled}    ${find_enabled}
    ...    msg=Expected Find's label to visibly dim while the field is blank
    Colors Should Not Match    ${find_all_disabled}    ${find_all_enabled}
    ...    msg=Expected Find All's label to visibly dim while the field is blank

TC-SRCH-002 Substring Search Is Case-Insensitive Over Keys And Values
    [Documentation]    Also covers non-string scalars: search text must match
    ...    a bool's/null's *string form*, not just literal string values.
    ...    simple_object.json's rows are name 197, version 218, active 239,
    ...    notes 260, tags 281; the three searches are ordered so each
    ...    reveals a different row than the one before (ACTIVE and TRUE both
    ...    hit `active` -- by key and by value -- so NULL sits between them),
    ...    which is what makes the highlight prove the *new* search matched.
    [Tags]    p1
    Click At    199    11
    Sleep    0.3s
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Open Source Search Dialog
    Search For    ACTIVE
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Highlighted Source Row Should Be    239    197    218    239    260    281
    Search For    NULL
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Highlighted Source Row Should Be    260    197    218    239    260    281
    Search For    TRUE
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Highlighted Source Row Should Be    239    197    218    239    260    281

TC-SRCH-003a Regex Mode Matches Per Regex Semantics
    [Documentation]    `^Ali` is no substring of anything in people.json, so a
    ...    hit at all proves the Regex box switched the matching engine.
    [Tags]    p2
    Open Source Search Dialog
    Click At    ${SEARCH_REGEX_CHECKBOX_X}    ${SEARCH_REGEX_CHECKBOX_Y}
    Search For    ^Ali
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Highlighted Source Row Should Be    218    197    218    239    260

TC-SRCH-003b Invalid Regex Pattern Is A Search Error
    [Tags]    p2
    Open Source Search Dialog
    Click At    ${SEARCH_REGEX_CHECKBOX_X}    ${SEARCH_REGEX_CHECKBOX_Y}
    Search For    (unclosed
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    Search error

TC-SRCH-004 Results Panel Header Format And Close
    [Documentation]    "Find All" on text that occurs nowhere: "No matches
    ...    found." itself is weak-styled (low contrast) and, like the
    ...    match-count text, confirmed unreliable for OCR -- the zero-hit case
    ...    is instead confirmed by the *absence* of any hit line
    ...    ("[Source]", which every hit starts with) alongside the heading
    ...    that a search did run.
    [Tags]    p2
    Open Source Search Dialog
    Find All For    zzz_no_such_text
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search results
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Source
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Close Search Results Panel And Verify

TC-SRCH-005 A Hit Line Shows Its Path And Preview, And Reveals On Click
    [Documentation]    The hit is `.[0].name` (a leaf two levels deep) --
    ...    revealing it expands row 0 first, so the leaf itself ends up at
    ...    row 0's *child* position (y=218), not row 0's own row (y=197,
    ...    which is what a container-valued reveal like TC-SRCH-020 would
    ...    highlight instead). Checked as ".name" rather than the full
    ...    ".[0].name": confirmed during implementation that a digit
    ...    sandwiched between brackets is a particularly OCR-unfriendly
    ...    sequence, garbled even by the general 0/O fallback. Clicked by
    ...    ".name" rather than "Alice" too: the heading above the hit line
    ...    echoes the search term ("Search results -- Source "Alice""), so
    ...    "Alice" isn't unique in this region and OCR word order isn't
    ...    guaranteed to put the (non-clickable) heading's copy second --
    ...    confirmed during implementation that it can click straight into
    ...    the heading instead of the hit line below it.
    [Tags]    p1
    Open Source Search Dialog
    Find All For    Alice
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    .name
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Alice
    ${baseline}=    Get Pixel Color    300    218
    Click Text In Region    @{SEARCH_RESULTS_AREA}    .name
    Sleep    0.5s
    ${highlighted}=    Get Pixel Color    300    218
    Colors Should Not Match    ${baseline}    ${highlighted}
    ...    msg=Expected clicking a hit to highlight the revealed row in Source

TC-SRCH-007c Clearing The Source Invalidates The Open Search Panel
    [Tags]    p3
    Open Source Search Dialog
    Find All For    Alice
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search results
    Click At    199    11
    Sleep    0.3s
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Search results

TC-SRCH-007d Clearing The Source Clears The Find Status
    [Tags]    p3
    Open Source Search Dialog
    Search For    Alice
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Click At    199    11
    Sleep    0.3s
    Region Should Not Contain Text    @{SEARCH_STATUS_AREA}    1 of 1

TC-SRCH-039 Find With No Match Is Reported In The Dialog
    [Documentation]    The dialog stays open showing "No matches found." (red,
    ...    like the error above -- but a plain miss is not an error), and the
    ...    tree is left alone: no row is highlighted.
    [Tags]    p2
    Open Source Search Dialog
    Search For    zzz_no_such_text
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    No matches found
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Regex
    ...    msg=The dialog should stay open after a Find
    Highlighted Source Row Should Be    -1    197    218    239

TC-SRCH-040 Find Reveals The Match In The Tree
    [Documentation]    The hit is `.[0].name` (a leaf two levels deep) --
    ...    revealing it expands row 0 first, so the leaf itself ends up at
    ...    row 0's *child* position (y=218), not row 0's own row (y=197).
    [Tags]    p1
    Open Source Search Dialog
    ${baseline}=    Get Pixel Color    300    218
    Search For    Alice
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Sleep    0.3s
    ${highlighted}=    Get Pixel Color    300    218
    Colors Should Not Match    ${baseline}    ${highlighted}
    ...    msg=Expected Find to highlight the matching row in Source

TC-SRCH-030 Ctrl+F Puts The Cursor In The Find Field
    [Documentation]    Nothing is clicked between Ctrl+F and typing. Find is
    ...    disabled exactly while the field is blank, so its label dimming
    ...    and undimming is what shows the text landed in the field (reading
    ...    the field itself is unreliable: OCR trips on the text cursor
    ...    sitting right after the last character).
    [Tags]    p1
    Open Source Search Dialog
    ${disabled_color}=    Get Pixel Color    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    Type Text    quokka
    Sleep    0.3s
    ${enabled_color}=    Get Pixel Color    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected typing straight after Ctrl+F to reach the Find field

TC-SRCH-031 Search From A Row's Context Menu Also Focuses The Find Field
    [Documentation]    Same check as TC-SRCH-030, for the dialog opened from a
    ...    row's context menu instead of Ctrl+F.
    [Tags]    p1
    Open Row Context Menu    200    197
    @{menu}=    Row Context Menu Region    200    197
    Click Text In Region    @{menu}    Search
    Sleep    0.3s
    ${disabled_color}=    Get Pixel Color    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    Type Text    quokka
    Sleep    0.3s
    ${enabled_color}=    Get Pixel Color    ${SEARCH_FIND_X}    ${SEARCH_FIND_Y}
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected typing straight after Search... to reach the Find field

TC-SRCH-032 Find Steps Through The Matches One By One And Wraps
    [Documentation]    "age" in people.json matches four nodes, in document
    ...    order: the three `age` keys and `.[2].role` ("manager"). Each Find
    ...    reveals the next one; each reveal expands its object and leaves it
    ...    open, so the rows move down as it goes -- `age` rows at y=239,
    ...    323, 407 and `.[2].role` at 428 (three rows per object plus its own
    ...    row, 21px apart). After the fourth, Find wraps to the first.
    [Tags]    p1
    Open Source Search Dialog
    Search For    age
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 4
    Highlighted Source Row Should Be    239    239
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    2 of 4
    Highlighted Source Row Should Be    323    239    323
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    3 of 4
    Highlighted Source Row Should Be    407    239    323    407
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    4 of 4
    Highlighted Source Row Should Be    428    239    323    407    428
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 4
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    wrapped
    Highlighted Source Row Should Be    239    239    323    407    428

TC-SRCH-033 Enter Repeats Find And Leaves The Cursor In The Field
    [Documentation]    Keyboard only after Ctrl+F: Enter finds, and -- because
    ...    the field keeps the focus -- Enter again finds the next match.
    [Tags]    p1
    Open Source Search Dialog
    Type Text    age
    Sleep    0.3s
    Press Key    enter
    Sleep    0.5s
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 4
    Press Key    enter
    Sleep    0.5s
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    2 of 4
    Highlighted Source Row Should Be    323    239    323

TC-SRCH-034 Changing The Text Starts Again At The First Match
    [Documentation]    After stepping to "age" match 2, searching "role"
    ...    (three matches: the three `role` keys) starts at its first, the
    ...    row under `.[0]` -- y=260 once objects 0 and 1 are open.
    [Tags]    p2
    Open Source Search Dialog
    Search For    age
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    2 of 4
    Search For    role
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 3
    Highlighted Source Row Should Be    260    239    260    323    344

TC-SRCH-035 Escape Closes The Dialog
    [Tags]    p2
    Open Source Search Dialog
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Regex
    Press Key    escape
    Sleep    0.4s
    Region Should Not Contain Text    @{POPUP_DIALOG_AREA}    Regex

TC-SRCH-036 Ctrl+F On An Open Dialog Selects The Text So Typing Replaces It
    [Documentation]    Without the selection the field would read `zzzAlice`,
    ...    which matches nothing.
    [Tags]    p2
    Open Source Search Dialog
    Type Text    zzz
    Sleep    0.2s
    Press Keys    ctrl    f
    Sleep    0.3s
    Type Text    Alice
    Sleep    0.2s
    Press Key    enter
    Sleep    0.5s
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1

TC-SRCH-038 Enter Still Finds After Ticking The Regex Box
    [Documentation]    Clicking the checkbox would take the keyboard focus
    ...    away from the field; the dialog hands it straight back, so Enter
    ...    means Find again without a click on the field. `^Ali` only
    ...    matches with the box ticked.
    [Tags]    p2
    Open Source Search Dialog
    Type Text    ^Ali
    Sleep    0.3s
    Click At    ${SEARCH_REGEX_CHECKBOX_X}    ${SEARCH_REGEX_CHECKBOX_Y}
    Sleep    0.3s
    Press Key    enter
    Sleep    0.5s
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1

TC-SRCH-042 Find All Lists Every Match And Leaves The Dialog Open
    [Documentation]    "age" in people.json matches four nodes -- the three
    ...    `age` keys and `.[2].role` ("manager") -- and all four are listed,
    ...    in document order. Unlike Find, nothing is revealed or highlighted
    ...    in the tree, and nothing is selected in the list. The dialog stays
    ...    open, its status line counting the matches.
    [Tags]    p1
    Open Source Search Dialog
    Find All For    age
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    manager
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    4 matches
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Regex
    ...    msg=The dialog should stay open after a Find All
    Highlighted Source Row Should Be    -1    197    218    239
    ${selected}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    Should Not Be True    ${selected}    msg=Nothing is selected in a fresh Find All list

TC-SRCH-043 Clicking A Find All Entry Reveals It, And Find Carries On From It
    [Documentation]    The third "age" entry is `.[2].age`. Clicking it selects
    ...    the entry and reveals that row -- Source row y=281 (root, [0], [1],
    ...    [2], name, age) -- and Find then carries on from there, as
    ...    Notepad++'s Find Next carries on from where the caret was left:
    ...    "4 of 4" (`.[2].role`, y=302), not "1 of 4".
    [Tags]    p1
    Open Source Search Dialog
    Find All For    age
    Click At    ${HIT_TEXT_X}    ${HIT_ROW_3_Y}
    Sleep    0.5s
    Highlighted Source Row Should Be    281    197    218    239    260    281    302
    ${selected}=    Hit List Row Is Highlighted    ${HIT_ROW_3_Y}
    Should Be True    ${selected}    msg=Expected the clicked entry to be highlighted
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    4 of 4
    Highlighted Source Row Should Be    302    281    302

TC-SRCH-044 Find Moves The Highlight In A Find All List
    [Documentation]    With the list showing, each Find selects the entry it
    ...    reveals, so the list and the tree never disagree about which match
    ...    is current.
    [Tags]    p2
    Open Source Search Dialog
    Find All For    age
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 4
    ${first}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    Should Be True    ${first}    msg=Expected Find's first match to be selected in the list
    Click Find
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    2 of 4
    ${first}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    ${second}=    Hit List Row Is Highlighted    ${HIT_ROW_2_Y}
    Should Not Be True    ${first}    msg=Expected the first entry to lose its highlight
    Should Be True    ${second}    msg=Expected Find's second match to be selected in the list

TC-SRCH-045 Find All Over Results Lists Results Rows
    [Documentation]    Ctrl+F after clicking the Results panel searches that
    ...    tree: the hit line is tagged `[Results]`, and clicking it reveals
    ...    the row in the Results tree (`.[1].name`, at y=239 once row 1 is
    ...    expanded), not in Source.
    [Tags]    p2
    Run Query    .[]
    Click At    800    300
    Sleep    0.2s
    Press Keys    ctrl    f
    Sleep    0.3s
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Results
    Find All For    bob
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Results]
    ${baseline}=    Get Pixel Color    900    239
    Click Text In Region    @{SEARCH_RESULTS_AREA}    .name
    Sleep    0.5s
    ${highlighted}=    Get Pixel Color    900    239
    Colors Should Not Match    ${baseline}    ${highlighted}
    ...    msg=Expected clicking a hit to highlight the revealed row in Results

TC-SRCH-046 Find All With An Invalid Regex Is Reported In The Dialog
    [Documentation]    The error goes to the dialog's status line, as for Find
    ...    (TC-SRCH-003b); no hit list opens.
    [Tags]    p2
    Open Source Search Dialog
    Type Text    (unclosed
    Sleep    0.3s
    Click At    ${SEARCH_REGEX_CHECKBOX_X}    ${SEARCH_REGEX_CHECKBOX_Y}
    Sleep    0.2s
    Click At    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Wait Until Region Contains Text    @{SEARCH_STATUS_AREA}    Search error    timeout=3
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Search results

TC-SRCH-020 Find In Source Reveals A Structurally-Equal Result
    [Tags]    p1
    Run Query    .[0]
    Open Row Context Menu    650    197
    @{menu}=    Row Context Menu Region    650    197
    ${baseline}=    Get Pixel Color    300    197
    Click Text In Region    @{menu}    Find in Source
    Sleep    0.3s
    ${highlighted}=    Get Pixel Color    300    197
    Colors Should Not Match    ${baseline}    ${highlighted}
    ...    msg=Expected the matching Source row to be highlighted after Find in Source

TC-SRCH-021 Find In Source Reports Not Found For A Computed Value
    [Tags]    p1
    Run Query    .[0].name + "!"
    Open Row Context Menu    650    197
    @{menu}=    Row Context Menu Region    650    197
    Click Text In Region    @{menu}    Find in Source
    Wait Until Region Contains Text    @{STATUS_BAR}    Not found in source    timeout=5

TC-SRCH-023 Find In Source Lists Every Match, Best One Selected And Revealed
    [Documentation]    `.users[].country` yields US, UK, US; the row clicked
    ...    is output 2 (the second "US"), so of the two `country` nodes
    ...    holding "US" the one at `users[2]` is the best guess -- listed
    ...    first, highlighted in the list, and revealed in Source. Clicking
    ...    the other entry moves both highlights. The Source rows checked
    ...    (y=302, y=260) are the `country` leaf of users[2] / users[0]
    ...    once that user is the one expanded.
    [Tags]    p1
    Load Duplicates Fixture
    ${baseline}=    Get Pixel Color    300    302
    Find In Source On Result Row    .users[].country    239
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    .country
    ${selected}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    Should Be True    ${selected}    msg=Expected the best match (first entry) to be highlighted
    ${revealed}=    Get Pixel Color    300    302
    Colors Should Not Match    ${baseline}    ${revealed}
    ...    msg=Expected users[2].country to be revealed in Source

TC-SRCH-024 Clicking Another Candidate Moves The Selection And The Reveal
    [Tags]    p1
    Load Duplicates Fixture
    Find In Source On Result Row    .users[].country    239
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    ${before}=    Get Pixel Color    300    260
    Click At    ${HIT_TEXT_X}    ${HIT_ROW_2_Y}
    Sleep    0.5s
    ${first_selected}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    ${second_selected}=    Hit List Row Is Highlighted    ${HIT_ROW_2_Y}
    Should Not Be True    ${first_selected}    msg=Expected the first entry to lose its highlight
    Should Be True    ${second_selected}    msg=Expected the clicked entry to be highlighted
    ${after}=    Get Pixel Color    300    260
    Colors Should Not Match    ${before}    ${after}
    ...    msg=Expected users[0].country to be revealed in Source

TC-SRCH-025 Find In Source Falls Back To A Text Search For A Transformed String
    [Documentation]    `ascii_downcase` turns "Ann" into "ann", which equals no
    ...    node -- so the lookup falls back to a text search, whose hit is only
    ...    listed: it is approximate, so nothing is highlighted in the list
    ...    and the Source tree is left where it was.
    [Tags]    p1
    Load Duplicates Fixture
    ${baseline}=    Get Pixel Color    300    239
    Find In Source On Result Row    .users[0].name | ascii_downcase    197
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    .name
    ${selected}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    Should Not Be True    ${selected}    msg=A text-search fallback hit must not be pre-selected
    ${after}=    Get Pixel Color    300    239
    Colors Should Match    ${baseline}    ${after}
    ...    msg=Expected the Source tree not to move on a text-search fallback

TC-SRCH-026 A New Search Replaces A Find In Source List
    [Documentation]    Find All fills the same bottom panel, so its list
    ...    replaces a Find in Source list (Find alone leaves it, TC-SRCH-041).
    [Tags]    p3
    Load Duplicates Fixture
    Find In Source On Result Row    .users[].country    239
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Open Source Search Dialog
    Find All For    Ann
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search results
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Find in Source

TC-SRCH-041 Find Leaves A Find In Source List In Place
    [Documentation]    Find reports in the dialog and reveals in the tree; it
    ...    doesn't touch a hit list already in the bottom panel (Find All is
    ...    what replaces one, TC-SRCH-026).
    [Tags]    p3
    Load Duplicates Fixture
    Find In Source On Result Row    .users[].country    239
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Open Source Search Dialog
    Search For    Ann
    Region Should Contain Text    @{SEARCH_STATUS_AREA}    1 of 1
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Find in Source

TC-SRCH-027 A Nested Key/Value Row Lists Its Same-Key Matches, Nth One Selected
    [Documentation]    `.sites[]` yields the three site objects; output 2
    ...    (Lyon) is expanded and its `zip` row (75001) is looked up. Nodes
    ...    holding that value under the key `zip` are `sites[2].zip` (the
    ...    output's own position -- selected and revealed) and `sites[0].zip`;
    ...    `depots[2].code` (same value, same index, listed earlier in the
    ...    document) has another key and is not a candidate. The revealed leaf
    ...    is Source row y=344: root, hq, depots, sites, sites[0], sites[1],
    ...    sites[2], city, zip.
    [Tags]    p1
    Load Sites Fixture
    ${baseline}=    Get Pixel Color    300    344
    Find In Source On Nested Result Row    .sites[]    239    281    Lyon
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    .zip
    ${selected}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    Should Be True    ${selected}    msg=Expected the best match (first entry) to be highlighted
    ${revealed}=    Get Pixel Color    300    344
    Colors Should Not Match    ${baseline}    ${revealed}
    ...    msg=Expected sites[2].zip -- not depots[2].code -- to be revealed in Source

TC-SRCH-028 A Nested Row's Other Match Can Be Picked From The List
    [Documentation]    Continues TC-SRCH-027's lookup and clicks the second
    ...    entry, `sites[0].zip`: the list highlight moves to it and Source
    ...    reveals it -- with sites[2] already open from the first reveal,
    ...    that is Source row y=302 (root, hq, depots, sites, sites[0], city,
    ...    zip), where the sites[2] row itself sat before.
    [Tags]    p1
    Load Sites Fixture
    Find In Source On Nested Result Row    .sites[]    239    281    Lyon
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    ${before}=    Get Pixel Color    300    302
    Click At    ${HIT_TEXT_X}    ${HIT_ROW_2_Y}
    Sleep    0.5s
    ${first_selected}=    Hit List Row Is Highlighted    ${HIT_ROW_1_Y}
    ${second_selected}=    Hit List Row Is Highlighted    ${HIT_ROW_2_Y}
    Should Not Be True    ${first_selected}    msg=Expected the first entry to lose its highlight
    Should Be True    ${second_selected}    msg=Expected the clicked entry to be highlighted
    ${after}=    Get Pixel Color    300    302
    Colors Should Not Match    ${before}    ${after}
    ...    msg=Expected sites[0].zip to be revealed in Source

TC-SRCH-029 A Nested Row Whose Key Picks Out One Node Jumps Straight To It
    [Documentation]    `.sites[1]` is the Rome object; its `zip` is `00100`,
    ...    which `hq.code` (listed first in the document) holds too -- but
    ...    under another key. So `sites[1].zip` is the only match and is
    ...    revealed directly, with no list. Source row y=323: root, hq,
    ...    depots, sites, sites[0], sites[1], city, zip.
    [Tags]    p1
    Load Sites Fixture
    ${baseline}=    Get Pixel Color    300    323
    Find In Source On Nested Result Row    .sites[1]    197    239    Rome
    Sleep    0.5s
    ${revealed}=    Get Pixel Color    300    323
    Colors Should Not Match    ${baseline}    ${revealed}
    ...    msg=Expected sites[1].zip to be revealed in Source
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Find in Source

TC-SRCH-037 The Find In Source Panel Closes
    [Tags]    p3
    Load Duplicates Fixture
    Find In Source On Result Row    .users[].country    239
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Close Search Results Panel And Verify    Find in Source
