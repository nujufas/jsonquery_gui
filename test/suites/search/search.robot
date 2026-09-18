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
# Centers of the first two lines of the bottom hit list, at 18px stride --
# identical for any list with at least one hit (the panel keeps its default
# height then; it only shrinks for the zero-hit case). Fixed rather than
# found by OCR: the *selected* line sits on the blue highlight, and OCR was
# confirmed to sometimes drop that line's "[Source]" token, returning the
# next line's position instead -- so a test could misjudge which line is
# selected.
${HIT_ROW_1_Y}    637
${HIT_ROW_2_Y}    655
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
    [Documentation]    Whether the Find-in-Source list line whose center is at
    ...    `${row_y}` is drawn selected: its far-right, text-free end
    ...    (x=1100) either carries the highlight tint or the plain panel
    ...    background, compared against a point 40px further down -- below
    ...    the last of at most two hit lines, so always plain background.
    [Arguments]    ${row_y}
    ${plain_y}=    Evaluate    ${row_y} + 40
    ${row}=    Get Pixel Color    1100    ${row_y}
    ${plain}=    Get Pixel Color    1100    ${plain_y}
    ${diff}=    Evaluate    max(abs(a - b) for a, b in zip($row, $plain))
    RETURN    ${diff > 6}

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
    ...    row 0's *child* position (y=218), not row 0's own row (y=197,
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
    ${baseline}=    Get Pixel Color    300    218
    Click Text In Region    @{SEARCH_RESULTS_AREA}    .name
    Sleep    0.5s
    ${highlighted}=    Get Pixel Color    300    218
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
    [Tags]    p3
    Load Duplicates Fixture
    Find In Source On Result Row    .users[].country    239
    Wait Until Region Contains Text    @{SEARCH_RESULTS_AREA}    Find in Source    timeout=5
    Open Source Search Dialog
    Search For    Ann
    Region Should Contain Text    @{SEARCH_RESULTS_AREA}    Search results
    Region Should Not Contain Text    @{SEARCH_RESULTS_AREA}    Find in Source

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
