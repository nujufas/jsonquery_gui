*** Settings ***
Documentation     "Search..." and "Find in Source" -- see
...               test/docs/08_search_and_find_in_source.md. TC-SRCH-006 (the
...               5,000-match cap) and TC-SRCH-007's load/query sub-cases
...               aren't implemented -- constructing a fixture with 5,000+
...               matches and reliably distinguishing "search invalidated"
...               from "search panel just not re-shown yet" added more cost
...               than value here; the Clear sub-case of TC-SRCH-007 is cheap
...               and is included below.
...
...               The results panel's match-count text (e.g. "1 match(es)")
...               is styled "weak" (low-contrast) and confirmed unreliable
...               for OCR even in a region sized well for everything else --
...               assertions here check hit-list content or the (normal-
...               contrast) heading text instead of that specific count.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        search
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Load People Fixture
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

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
    ...    Find field and submits it via Find All -- retrying the whole
    ...    type+click if Find All doesn't visibly register (the same
    ...    click-right-after-typing timing flakiness as Load Via Url).
    ...    Waits on the heading "Search results" rather than the weak-styled
    ...    match count, since that reads reliably regardless of hit count.
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
    Click At    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Wait Until Region Matches    @{SEARCH_RESULTS_AREA}    results|error    timeout=3

Close Search Results Panel And Verify
    [Documentation]    Retries the Close click itself for the same reason as
    ...    Search For above.
    Wait Until Keyword Succeeds    3x    0.5s
    ...    Click Close And Verify Panel Gone

Click Close And Verify Panel Gone
    Close Search Results Panel
    Sleep    0.2s
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Search results

*** Test Cases ***
TC-SRCH-001 Search Dialog Shows Its Fields And Buttons
    [Tags]    p2
    Open Source Search Dialog
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Search
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Source
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Find
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Regex
    Region Should Contain Text    @{POPUP_DIALOG_AREA}    Cancel
    ${disabled_color}=    Get Pixel Color    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Click At    ${SEARCH_FIND_FIELD_X}    ${SEARCH_FIND_FIELD_Y}
    Type Text    x
    Sleep    0.2s
    ${enabled_color}=    Get Pixel Color    ${SEARCH_FIND_ALL_X}    ${SEARCH_FIND_ALL_Y}
    Colors Should Not Match    ${disabled_color}    ${enabled_color}
    ...    msg=Expected Find All's label to visibly dim while the field is blank

TC-SRCH-002 Substring Search Is Case-Insensitive Over Keys And Values
    [Documentation]    Also covers non-string scalars: search text must match
    ...    a bool's/null's *string form*, not just literal string values.
    ...    Verified via the hit line's own content (normal contrast, reads
    ...    reliably) rather than the weak-styled match count.
    [Tags]    p1
    Click At    199    11
    Sleep    0.3s
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}
    Open Source Search Dialog
    Search For    ACTIVE
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    active
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    true
    Close Search Results Panel And Verify
    Open Source Search Dialog
    Search For    TRUE
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    active
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    true
    Close Search Results Panel And Verify
    Open Source Search Dialog
    Search For    NULL
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    notes
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    null

TC-SRCH-003a Regex Mode Matches Per Regex Semantics
    [Tags]    p2
    Open Source Search Dialog
    Click At    ${SEARCH_REGEX_CHECKBOX_X}    ${SEARCH_REGEX_CHECKBOX_Y}
    Search For    ^Ali
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    regex
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Alice

TC-SRCH-003b Invalid Regex Pattern Is A Search Error
    [Tags]    p2
    Open Source Search Dialog
    Click At    ${SEARCH_REGEX_CHECKBOX_X}    ${SEARCH_REGEX_CHECKBOX_Y}
    Search For    (unclosed
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search error

TC-SRCH-004 Results Panel Header Format And Close
    [Documentation]    "No matches found." itself is weak-styled (low
    ...    contrast) and, like the match-count text, confirmed unreliable for
    ...    OCR -- the zero-hit case is instead confirmed by the *absence* of
    ...    any hit line ("[Source]", which every hit starts with) alongside
    ...    the heading that a search did run.
    [Tags]    p2
    Open Source Search Dialog
    Search For    zzz_no_such_text
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search results
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Source
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Close Search Results Panel And Verify

TC-SRCH-005 A Hit Line Shows Its Path And Preview, And Reveals On Click
    [Documentation]    The hit is `.[0].name` (a leaf two levels deep) --
    ...    revealing it expands row 0 first, so the leaf itself ends up at
    ...    row 0's *child* position (y=203), not row 0's own row (y=182,
    ...    which is what a container-valued reveal like TC-SRCH-020 would
    ...    highlight instead). Checked as ".name" rather than the full
    ...    ".[0].name": confirmed during implementation that a digit
    ...    sandwiched between brackets is a particularly OCR-unfriendly
    ...    sequence, garbled even by the general 0/O fallback. Clicked by
    ...    ".name" rather than "Alice" too: the heading above the hit line
    ...    echoes the search term ("Search results — Source "Alice""), so
    ...    "Alice" isn't unique in this region and OCR word order isn't
    ...    guaranteed to put the (non-clickable) heading's copy second --
    ...    confirmed during implementation that it can click straight into
    ...    the heading instead of the hit line below it.
    [Tags]    p1
    Open Source Search Dialog
    Search For    Alice
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    [Source]
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    .name
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Alice
    ${baseline}=    Get Pixel Color    300    203
    Click Text In Region    @{SEARCH_RESULTS_AREA}    .name
    Sleep    0.5s
    ${highlighted}=    Get Pixel Color    300    203
    Colors Should Not Match    ${baseline}    ${highlighted}
    ...    msg=Expected clicking a hit to highlight the revealed row in Source

TC-SRCH-007c Clearing The Source Invalidates The Open Search Panel
    [Tags]    p3
    Open Source Search Dialog
    Search For    Alice
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search results
    Click At    199    11
    Sleep    0.3s
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Search results

TC-SRCH-020 Find In Source Reveals A Structurally-Equal Result
    [Tags]    p1
    Run Query    .[0]
    Open Row Context Menu    650    182
    @{menu}=    Row Context Menu Region    650    182
    ${baseline}=    Get Pixel Color    300    182
    Click Text In Region    @{menu}    Find in Source
    Sleep    0.3s
    ${highlighted}=    Get Pixel Color    300    182
    Colors Should Not Match    ${baseline}    ${highlighted}
    ...    msg=Expected the matching Source row to be highlighted after Find in Source

TC-SRCH-021 Find In Source Reports Not Found For A Computed Value
    [Tags]    p1
    Run Query    .[0].name + "!"
    Open Row Context Menu    650    182
    @{menu}=    Row Context Menu Region    650    182
    Click Text In Region    @{menu}    Find in Source
    Wait Until Region Contains Text    @{STATUS_BAR}    Not found in source    timeout=5
