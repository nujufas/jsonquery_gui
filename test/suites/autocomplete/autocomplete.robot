*** Settings ***
Documentation     Query-box autocomplete/suggestion popup -- see
...               crates/query/src/suggest.rs (dialect scope + document-aware
...               path completion) and crates/app/src/query_suggest.rs (the
...               egui popup itself). Experimental and off by default, so
...               every case here turns it on explicitly via the 💡 toggle
...               except TC-AC-001, which checks that default.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        autocomplete
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Launch Jsonquery App
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures
# A tighter version of SUGGEST_POPUP_AREA, just for the *collapsed*
# 11-row (10 items + "more") popup -- confirmed during implementation that
# the full-height SUGGEST_POPUP_AREA extends far enough below a short,
# collapsed popup to also catch the Source tree panel's own row list
# underneath it (e.g. its own "10: 10" row), which a positive
# Suggest-Popup-Should-Contain check doesn't care about but a Should-Not-
# Contain check for an index past the fold (like TC-AC-040's "10") would
# wrongly fail against. Not used as the shared default because other cases
# (e.g. TC-AC-041, via Navigate To Last Suggestion And Verify) need to see
# a fully expanded, scrolled popup that's genuinely taller than this.
@{SUGGEST_POPUP_COLLAPSED_AREA}    0    100    480    250
# The "… 40 more" row's app-relative click point for TC-AC-040, right after
# typing "." against many_numbers.json's 60-item root array on the default
# 1200x800 window -- calibrated against egui's own reported widget rect
# ([[15.0 333.0] - [90.3 351.0]]) during implementation, since OCR can't
# read that row (see TC-AC-040's own documentation).
${MORE_ROW_X}    50
${MORE_ROW_Y}    340

*** Keywords ***
Load People Fixture
    [Documentation]    people.json -- a root ARRAY of 3 objects
    ...    (name/age/role: Alice/34/engineer, Bob/19/intern, Carol/45/manager).
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}

Load Simple Object Fixture
    [Documentation]    simple_object.json -- a root OBJECT
    ...    (name/version/active/notes/tags) whose "tags" field is itself a
    ...    3-item array (["gui","rust","json"]) -- for object-key and, one
    ...    level deeper, nested-array-index completion.
    ${json}=    Get File    ${FIXTURES}/simple_object.json
    Load Fixture Via Paste    ${json}

Load Many Numbers Fixture
    [Documentation]    many_numbers.json -- a flat 60-element root array
    ...    (0..59), comfortably past both the popup's 10-row collapse
    ...    threshold and suggest.rs's own 50-candidate cap -- for the
    ...    collapse/expand/scroll cases.
    ${json}=    Get File    ${FIXTURES}/many_numbers.json
    Load Fixture Via Paste    ${json}

Type Query Text
    [Documentation]    Replaces the query box's contents with `text` and
    ...    waits for the suggestion popup to recompute. Deliberately does
    ...    NOT run the query (unlike Run Query/Type And Run Query in
    ...    keywords.resource) -- these cases inspect the box mid-edit.
    [Arguments]    ${text}
    Click At    100    58
    Sleep    0.2s
    Press Keys    ctrl    a
    Press Key    delete
    Sleep    0.1s
    Type Text    ${text}
    Sleep    0.4s

Suggest Popup Should Contain
    [Documentation]    Polls rather than a one-shot OCR read: confirmed
    ...    during implementation that a popup row can occasionally misread
    ...    on a single capture even when correctly rendered (the same class
    ...    of transient OCR flakiness documented elsewhere in this
    ...    resource), so this retries within a short timeout instead of
    ...    failing on one bad frame. `${psm}` defaults to Tesseract's normal
    ...    "uniform block of text" mode (6, matching the rest of this
    ...    resource) -- the "… N more" row is a rare exception that needs
    ...    psm 11 ("sparse text") instead, since its low-contrast weak/
    ...    italic style (confirmed during implementation, by direct pixel
    ...    inspection, to peak at roughly a third of a normal row's
    ...    brightness) reads as pure noise under psm 6 no matter how many
    ...    times it's retried.
    [Arguments]    ${text}    ${psm}=6
    Wait Until Region Contains Text    @{SUGGEST_POPUP_AREA}    ${text}    timeout=3    psm=${psm}

Suggest Popup Should Not Contain
    [Arguments]    ${text}
    Region Should Not Contain Text    @{SUGGEST_POPUP_AREA}    ${text}

Click Suggestion
    [Documentation]    Finds `text` by OCR inside the popup and clicks it --
    ...    same technique Select Engine/Close Search Results Panel use
    ...    elsewhere in this suite for OCR-located (not fixed-pixel) clicks.
    ...    Retried like Select Engine: a popup row can occasionally misread
    ...    on a single OCR capture even when correctly rendered.
    [Arguments]    ${text}
    Wait Until Keyword Succeeds    3x    0.3s
    ...    Click Text In Region    @{SUGGEST_POPUP_AREA}    ${text}
    Sleep    0.3s

Navigate To Last Suggestion And Verify
    [Documentation]    Retypes "." fresh (resetting keyboard selection to 0)
    ...    then presses ArrowDown 49 times to reach index 49 of the
    ...    50-candidate (capped) list -- past the 10-row collapse fold, which
    ...    should auto-expand and scroll the selected row into view. A
    ...    helper rather than inline in the test case so the whole sequence
    ...    (not just the final check) can be retried if a keypress or two
    ...    gets dropped along the way (same synthetic-input flakiness
    ...    documented throughout this resource).
    Type Query Text    .
    FOR    ${i}    IN RANGE    49
        Press Key    down
        Sleep    0.03s
    END
    Sleep    0.3s
    Suggest Popup Should Contain    49

*** Test Cases ***
TC-AC-001 Autocomplete Is Disabled By Default
    [Documentation]    Experimental feature, off until the 💡 toggle is
    ...    clicked -- a fresh launch must show no popup at all.
    [Tags]    p1
    Load People Fixture
    Type Query Text    .
    Suggest Popup Should Not Contain    object

TC-AC-002 Toggle Enables And Disables Autocomplete
    [Documentation]    Behavioral check (does typing actually show/hide
    ...    suggestions), cross-referenced against the icon's own
    ...    relative-pixel-color change (Toggle Autocomplete, same technique
    ...    as TC-WIN-002/TC-QRY-002).
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Contain    object
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Not Contain    object

TC-AC-010 Root Array Completion Shows Indices With Value Previews
    [Documentation]    people.json's root is a 3-object array -- "." at the
    ...    very start of the query should offer indices 0/1/2, each
    ...    previewed with its value's shape ("object{3}"), and untagged
    ...    (jq's leading "." is itself an unambiguous marker, so scope never
    ...    widens to more than one engine here).
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Contain    0
    Suggest Popup Should Contain    object
    Suggest Popup Should Not Contain    JMESPath

TC-AC-011 Accepting Root Index Inserts Bracket Syntax And Runs Correctly
    [Documentation]    Regression test: accepting index "0" from the bare
    ...    root "." must insert "[0]" (bracket notation), not a bare "0" --
    ...    ".0" is a jq *syntax error* (confirmed empirically: jaq parses it
    ...    as `Nothing` followed by a stray "0", not "index 0"), which is
    ...    exactly the bug this suite was written to catch. See suggest.rs's
    ...    `format_index`/`doc_candidates` array branch. Checked by the run
    ...    outcome rather than OCR-reading the query box itself: a bracket
    ...    vs. bare-digit query is only a couple of characters' difference,
    ...    confirmed during implementation to be too short and punctuation-
    ...    heavy for Tesseract to read reliably even when correctly
    ...    rendered -- the old bug's "Query error: query syntax error" vs.
    ...    the fix's "1 result(s)" is both a stronger and more legible
    ...    signal of the same fact.
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    .
    Press Key    enter
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Not Contain Text    @{STATUS_BAR}    Query error

TC-AC-012 Nested Completion Through An Accepted Key Runs Correctly
    [Documentation]    Regression test, second shape of the same bug: after
    ...    accepting an object key ("tags") and then descending into the
    ...    array it names, the separator dot typed before the index must be
    ...    dropped (not just bracket-wrapped) -- "tags.[0]" is a parse error
    ...    in both JSONPath and JMESPath (confirmed empirically), so the
    ...    result must read "tags[0]", which works in all three dotted
    ...    dialects.
    [Tags]    p1
    Load Simple Object Fixture
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Contain    tags
    Click Suggestion    tags
    Type Text    .
    Sleep    0.4s
    Suggest Popup Should Contain    0
    Press Key    enter
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Not Contain Text    @{STATUS_BAR}    Query error
    Region Should Contain Text    @{RESULTS_PANEL}    gui

TC-AC-013 Arrow Keys Change Which Suggestion Enter Accepts
    [Documentation]    Checked via the resolved *value* rather than OCR-
    ...    reading the accepted "[1]" text directly (too short/punctuation-
    ...    heavy to read reliably, per TC-AC-011's note) -- if ArrowDown had
    ...    no effect, Enter would accept index 0 (Alice) instead.
    [Tags]    p2
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    .
    Press Key    down
    Press Key    enter
    Type Text    .name
    Sleep    0.4s
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Bob

TC-AC-014 Clicking A Suggestion Row Accepts It And Closes The Popup
    [Documentation]    Same resolved-value technique as TC-AC-013, for the
    ...    mouse-click accept path instead of keyboard.
    [Tags]    p2
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    .
    Click Suggestion    2
    Suggest Popup Should Not Contain    object
    Type Text    .name
    Sleep    0.4s
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Carol

TC-AC-015 Escape Dismisses The Popup And Disables Autocomplete Until Re-Enabled
    [Documentation]    Also a regression check for the egui-Memory
    ...    global-focus-clear-on-Escape bug documented in query_suggest.rs:
    ...    typing immediately after Escape (with no intervening click) must
    ...    still land in the query box.
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Contain    object
    Press Key    escape
    Sleep    0.3s
    Suggest Popup Should Not Contain    object
    Type Text    abcxyz
    # Checked against "abcxy" (dropping the trailing "z"), not the full
    # "abcxyz": the box's own text cursor sits right after the last typed
    # character, and confirmed during implementation (reproducibly, not
    # just occasionally -- polling didn't help) to make Tesseract misread
    # just that last glyph (e.g. back as "abcxyq"). The first five
    # characters landing correctly is already conclusive proof typing
    # reached the box.
    Region Should Contain Text    @{QUERY_TEXTBOX}    abcxy
    Type Query Text    .
    Suggest Popup Should Not Contain    object
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Contain    object

TC-AC-020 Explicit Engine Selection Restricts Suggestions To That Engine
    [Documentation]    "sub" is a jq builtin with no JMESPath equivalent;
    ...    "avg" is the reverse -- a JMESPath builtin jq doesn't have. Each
    ...    should surface only under its own engine's explicit selection.
    ...    Selects jq by fixed pixel position rather than Select Engine's
    ...    OCR: confirmed during implementation that Tesseract can misread
    ...    the 2-character "jq" label consistently (not just occasionally)
    ...    within a given run, so retrying the same OCR read doesn't help --
    ...    the position is already calibrated in ENGINE_JQ_X/ENGINE_ROW_Y
    ...    for exactly this reason elsewhere (e.g. TC-QRY-002).
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Click At    ${ENGINE_JQ_X}    ${ENGINE_ROW_Y}
    Sleep    0.3s
    Type Query Text    sub
    Suggest Popup Should Contain    sub
    Type Query Text    avg
    Suggest Popup Should Not Contain    avg
    Click At    ${ENGINE_JQ_X}    ${ENGINE_ROW_Y}
    Sleep    0.3s
    Select Engine    JMESPath
    Type Query Text    avg
    Suggest Popup Should Contain    avg
    Type Query Text    sub
    Suggest Popup Should Not Contain    sub

TC-AC-021 Ambiguous Bare Word Tags Candidates By Engine
    [Documentation]    "sor" (matching "sort"/"sort_by") has no leading
    ...    marker, so with no explicit engine selected it's genuinely
    ...    ambiguous between jq and JMESPath -- both dialects' matches show,
    ...    tagged by engine so it's clear which is which.
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    sor
    Suggest Popup Should Contain    sort_by
    Suggest Popup Should Contain    JMESPath

TC-AC-022 Unambiguous Marker Narrows Automatically Without An Explicit Selection
    [Documentation]    A leading "." is itself an unambiguous jq marker (see
    ...    suggest::engines_in_scope) -- candidates narrow to jq alone with
    ...    no engine-picker selection needed, so nothing is tagged.
    [Tags]    p2
    Load Simple Object Fixture
    Toggle Autocomplete
    Type Query Text    .
    Suggest Popup Should Contain    tags
    Suggest Popup Should Not Contain    JMESPath

TC-AC-030 JSON Pointer Suggests Array Indices At Root
    [Tags]    p2
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    /
    Suggest Popup Should Contain    0
    Suggest Popup Should Contain    object
    Suggest Popup Should Not Contain    JMESPath

TC-AC-031 JSON Pointer Suggests Nested Object Keys And Accepting Runs Correctly
    [Documentation]    Accepts via Enter rather than Click Suggestion, and
    ...    doesn't check for "name" in the popup: "name" is always the
    ...    default-selected (index 0) row here, and confirmed during
    ...    implementation that Tesseract consistently (not just
    ...    occasionally -- polling doesn't help) misreads a *selected*
    ...    row's text, presumably from its blue highlight lowering
    ...    contrast, while its unselected neighbors ("age", "role") read
    ...    fine every time. Those two are already conclusive evidence that
    ...    object-key completion is offering all three fields; accepting
    ...    "name" specifically (the still-selected default) and checking
    ...    the *result* is Alice is what actually proves it was "name" that
    ...    got accepted. Mouse-click accept itself is already covered by
    ...    TC-AC-014, so Enter here just needs to be reliable, not another
    ...    exercise of the click path.
    [Tags]    p1
    Load People Fixture
    Toggle Autocomplete
    Type Query Text    /0/
    Suggest Popup Should Contain    age
    Suggest Popup Should Contain    role
    Press Key    enter
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice

TC-AC-040 Long Candidate List Collapses With An Expandable More Row
    [Documentation]    Regression test for the Area/ScrollArea sizing bug
    ...    documented in query_suggest.rs (a popup that first renders small
    ...    could get stuck there) -- the list must start collapsed to 10
    ...    rows plus a trailing "… N more" row, and clicking that row must
    ...    reveal the rest. Checked structurally (item 10 hidden, then
    ...    visible) rather than by reading the "more" text itself: confirmed
    ...    during implementation, by direct pixel inspection, that its
    ...    weak/italic style renders at roughly a third of a normal row's
    ...    brightness, which Tesseract reads as pure noise regardless of psm
    ...    mode or retries. Clicked by fixed position rather than Click
    ...    Suggestion for the same reason (${MORE_ROW_X}/${MORE_ROW_Y},
    ...    calibrated against this exact fixture/window -- confirmed via
    ...    egui's own reported widget rect during implementation).
    [Tags]    p1
    Load Many Numbers Fixture
    Toggle Autocomplete
    Type Query Text    .
    Wait Until Region Contains Text    @{SUGGEST_POPUP_COLLAPSED_AREA}    9    timeout=3
    Region Should Not Contain Text    @{SUGGEST_POPUP_COLLAPSED_AREA}    10
    Click At    ${MORE_ROW_X}    ${MORE_ROW_Y}
    Sleep    0.3s
    Wait Until Region Contains Text    @{SUGGEST_POPUP_COLLAPSED_AREA}    10    timeout=3

TC-AC-041 Keyboard Navigation Past The Fold Auto-Expands And Scrolls
    [Documentation]    Reaching item 49 of the 50 (capped) candidates via
    ...    keyboard, well past the 10-row collapse fold, must both
    ...    auto-expand the list and scroll the selected row into view
    ...    (Response::scroll_to_me) -- otherwise Enter would be accepting a
    ...    row the user never saw.
    [Tags]    p2
    Load Many Numbers Fixture
    Toggle Autocomplete
    Wait Until Keyword Succeeds    3x    1s
    ...    Navigate To Last Suggestion And Verify
    Press Key    enter
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Not Contain Text    @{STATUS_BAR}    Query error
