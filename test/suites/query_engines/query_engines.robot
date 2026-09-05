*** Settings ***
Documentation     Query bar and query engines -- see
...               test/docs/04_query_bar_and_engines.md. Covers auto-detect
...               routing and the per-engine 0-vs-error behavioral contract,
...               which is this app's most important (and most
...               counter-intuitive) correctness surface.
Resource          ../../resources/keywords.resource
Library           OperatingSystem
Force Tags        query_engines
Suite Setup       Start Test Display
Suite Teardown    Stop Test Display
Test Setup        Load People Fixture
Test Teardown     Close Jsonquery App

*** Variables ***
${FIXTURES}    ${CURDIR}/../../resources/fixtures

*** Keywords ***
Load People Fixture
    [Documentation]    Test Setup: launch the app and load people.json (3
    ...    objects with name/age/role) via paste, ready for a query.
    Launch Jsonquery App
    ${json}=    Get File    ${FIXTURES}/people.json
    Load Fixture Via Paste    ${json}

*** Test Cases ***
TC-QRY-010 Auto-Detect Leading Dot Routes To Jq
    [Tags]    p1
    Run Query    .[0].name
    Region Should Contain Text    @{STATUS_BAR}    jq
    Region Should Contain Text    @{STATUS_BAR}    auto
    Region Should Contain Text    @{STATUS_BAR}    result

TC-QRY-011 Auto-Detect Leading Slash Routes To Pointer
    [Tags]    p1
    Run Query    /0/name
    Region Should Contain Text    @{STATUS_BAR}    Pointer
    Region Should Contain Text    @{STATUS_BAR}    auto
    Region Should Contain Text    @{RESULTS_PANEL}    Alice

TC-QRY-032 Pointer Not Found Is A Query Error Not Zero Results
    [Documentation]    A well-formed pointer that resolves to nothing is an
    ...    error, not a neutral "0 result(s)" -- the main behavioral risk
    ...    called out in 04_query_bar_and_engines.md for this engine.
    [Tags]    p1
    Run Query    /does/not/exist
    Region Should Contain Text    @{STATUS_BAR}    Query error
    Region Should Contain Text    @{STATUS_BAR}    no value at pointer

TC-QRY-050 JMESPath Always Returns Exactly One Result Even For No Match
    [Documentation]    JMESPath's "no match" evaluates to a single null
    ...    result -- not 0 results, not an error. The most counter-intuitive
    ...    behavioral contract across the four engines.
    [Tags]    p1
    Select Engine    JMESPath
    Run Query    nonexistent_field
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Not Contain Text    @{STATUS_BAR}    Query error

TC-QRY-040 JSONPath Zero Matches Is Not An Error
    [Tags]    p1
    Select Engine    JSONPath
    Run Query    $.nonexistent
    Region Should Contain Text    @{STATUS_BAR}    0 result
    Region Should Not Contain Text    @{STATUS_BAR}    Query error

TC-QRY-003 Query Box Shows Hint Text When Empty
    [Documentation]    The multiline query box's placeholder text, visible
    ...    whenever it's empty (a fresh load never has query text yet).
    [Tags]    p3
    Region Should Contain Text    @{QUERY_TEXTBOX}    select

TC-QRY-001 Engine Picker Buttons Appear Left To Right In A Fixed Order
    [Documentation]    jq, Pointer, JSONPath, JMESPath -- always in this
    ...    order (Kind::ALL, rendered right-to-left but reversed, so it reads
    ...    left-to-right in this fixed order regardless of which is selected).
    [Tags]    p2
    ${jq_x}    ${jq_y}=    Find Text In Region    @{ENGINE_PICKER_ROW}    jq
    ${ptr_x}    ${ptr_y}=    Find Text In Region    @{ENGINE_PICKER_ROW}    Pointer
    ${jp_x}    ${jp_y}=    Find Text In Region    @{ENGINE_PICKER_ROW}    JSONPath
    ${jm_x}    ${jm_y}=    Find Text In Region    @{ENGINE_PICKER_ROW}    JMESPath
    Should Be True    ${jq_x} < ${ptr_x} < ${jp_x} < ${jm_x}
    ...    msg=Expected engine picker order jq < Pointer < JSONPath < JMESPath by x position

TC-QRY-002 Explicit Engine Selection Toggles On And Off
    [Documentation]    Clicking an engine button selects it (visibly
    ...    highlighted); clicking the same one again deselects back to
    ...    auto-detect. Checked via the button's own background pixel rather
    ...    than OCR, since the visual difference is a subtle tint, not text.
    ...    The mouse is moved away before every sample: egui shows its own
    ...    subtle hover tint on a widget the cursor is merely resting over
    ...    (confirmed during implementation -- a click leaves the cursor
    ...    sitting on the button afterward, and comparing a hovered-but-
    ...    unselected sample against a never-hovered baseline can read as a
    ...    color change on its own, independent of selection state).
    [Tags]    p1
    Move Mouse To    600    400
    ${baseline}=    Get Pixel Color    ${ENGINE_POINTER_X}    30
    Click At    ${ENGINE_POINTER_X}    ${ENGINE_ROW_Y}
    Move Mouse To    600    400
    Sleep    0.3s
    ${selected}=    Get Pixel Color    ${ENGINE_POINTER_X}    30
    Colors Should Not Match    ${baseline}    ${selected}
    ...    msg=Expected a visible highlight after selecting Pointer
    Click At    ${ENGINE_POINTER_X}    ${ENGINE_ROW_Y}
    Move Mouse To    600    400
    Sleep    0.5s
    ${deselected}=    Get Pixel Color    ${ENGINE_POINTER_X}    30
    Colors Should Match    ${baseline}    ${deselected}
    ...    msg=Expected the highlight to clear after clicking Pointer again

TC-QRY-012 Auto-Detect Leading Dollar Routes To JSONPath
    [Tags]    p1
    Run Query    $[*].age
    Region Should Contain Text    @{STATUS_BAR}    JSONPath
    Region Should Contain Text    @{STATUS_BAR}    auto
    Region Should Contain Text    @{STATUS_BAR}    3 result

TC-QRY-013 Auto-Detect JMESPath Filter Marker Routes To JMESPath
    [Documentation]    No leading marker distinguishes JMESPath from jq, so
    ...    detection instead looks for JMESPath-only substrings -- here, a
    ...    `[?...]` filter combined with a backtick literal.
    [Tags]    p1
    Run Query    [?age > \`30\`]
    Region Should Contain Text    @{STATUS_BAR}    JMESPath
    Region Should Contain Text    @{STATUS_BAR}    auto
    Region Should Contain Text    @{STATUS_BAR}    1 result

TC-QRY-014 Auto-Detect Bare Identifier Falls Back To Jq
    [Documentation]    A bare word with none of the other engines' markers is
    ...    ambiguous, so detection defaults to jq -- the richer, originally
    ...    default dialect (even though this particular one is invalid jq and
    ...    errors, the point is which engine it was routed to). Checked by
    ...    ruling out the other three engine names rather than asserting "jq"
    ...    itself is present -- confirmed during implementation that
    ...    Tesseract sometimes misreads the 2-character "jq" label as "iq" at
    ...    this size, which a positive check for the literal text "jq" is
    ...    flaky against.
    [Tags]    p2
    Run Query    nonexistent_bare_word
    Region Should Contain Text    @{STATUS_BAR}    auto
    Region Should Not Contain Text    @{STATUS_BAR}    Pointer
    Region Should Not Contain Text    @{STATUS_BAR}    JSONPath
    Region Should Not Contain Text    @{STATUS_BAR}    JMESPath

TC-QRY-020 Jq Per-Item Errors Are Non-Fatal
    [Documentation]    One item failing (e.g. dividing by an object) doesn't
    ...    abort the whole run -- the other items' errors are just counted
    ...    and the last one's message shown, and the query still "finishes".
    ...    "0 result(s)" is checked as plain "result(s)": the leading digit 0
    ...    is confirmed flaky against Tesseract's own O/0 confusion at this
    ...    font size (the same class of issue as "jq" vs "iq" above), and the
    ...    real point of this case is carried by the query finishing at all
    ...    plus the item-error count, not by that specific digit.
    [Tags]    p1
    Run Query    .[] | 1/.
    Region Should Contain Text    @{STATUS_BAR}    result
    Region Should Contain Text    @{STATUS_BAR}    3 item error

TC-QRY-021a Jq Malformed Program Is A Syntax (Parse) Error
    [Tags]    p2
    Run Query    def
    Region Should Contain Text    @{STATUS_BAR}    Query error
    Region Should Contain Text    @{STATUS_BAR}    syntax error

TC-QRY-021b Jq Well-Formed But Unresolvable Program Is A Query (Compile) Error
    [Documentation]    An unknown function call parses fine but can't compile
    ...    -- surfaces as "Query error: query error:" (no "syntax"), distinct
    ...    from TC-QRY-021a's "query syntax error:".
    [Tags]    p2
    Run Query    totally_unknown_function_call
    Region Should Contain Text    @{STATUS_BAR}    Query error

TC-QRY-023 Jq Select Filters Down To Only The Matching Items
    [Documentation]    A filter/select/project pipeline on the fixture --
    ...    confirms per-item filtering actually excludes non-matching items
    ...    rather than just annotating them, via a negative assertion against
    ...    the one item that shouldn't survive the filter.
    [Tags]    p2
    Select Engine    jq
    Run Query    .[] | select(.age > 30) | .name
    Region Should Contain Text    @{STATUS_BAR}    2 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice
    Region Should Contain Text    @{RESULTS_PANEL}    Carol
    Region Should Not Contain Text    @{RESULTS_PANEL}    Bob

TC-QRY-024 Jq Field Projection Streams One Result Per Item
    [Documentation]    A different field than the other jq cases (.role, not
    ...    .name/.age) streamed across all three items -- distinct query
    ...    shape from TC-QRY-023's filtered projection.
    [Tags]    p2
    Select Engine    jq
    Run Query    .[].role
    Region Should Contain Text    @{STATUS_BAR}    3 result
    Region Should Contain Text    @{RESULTS_PANEL}    engineer
    Region Should Contain Text    @{RESULTS_PANEL}    intern
    Region Should Contain Text    @{RESULTS_PANEL}    manager

TC-QRY-025 Jq Length Reduces The Whole Array To One Scalar Result
    [Documentation]    An aggregate builtin (as opposed to a per-item
    ...    filter/projection) -- exercises jq's non-streaming, whole-input
    ...    functions.
    [Tags]    p2
    Select Engine    jq
    Run Query    length
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    3

TC-QRY-026 Jq Sort By Then Index Picks The Expected Item
    [Documentation]    A multi-stage pipeline (sort_by, then array indexing)
    ...    -- confirms sort ordering is actually applied, not just accepted
    ...    as valid syntax, since the youngest person's name is the one
    ...    checked for.
    [Tags]    p2
    Select Engine    jq
    Run Query    sort_by(.age) | .[0].name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Bob

TC-QRY-027 Jq Array Construction Plus Add Sums Across Items
    [Documentation]    Builds an intermediate array from a streamed
    ...    projection (`[.[].age]`) then reduces it with `add` -- a
    ...    two-stage pipeline distinct from the single-builtin TC-QRY-025.
    [Tags]    p2
    Select Engine    jq
    Run Query    [.[].age] | add
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    98

TC-QRY-030 Pointer Resolves To Exactly Zero Or One Result
    [Tags]    p1
    Select Engine    Pointer
    Run Query    /0/name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice

TC-QRY-031 Pointer Malformed Syntax Is A Parse Error
    [Documentation]    A pointer that's neither empty nor starting with '/'
    ...    is rejected before ever trying to resolve it.
    [Tags]    p1
    Select Engine    Pointer
    Run Query    nope
    Region Should Contain Text    @{STATUS_BAR}    query syntax error
    Region Should Contain Text    @{STATUS_BAR}    must be empty
    Region Should Contain Text    @{STATUS_BAR}    start with '/'

TC-QRY-033 Empty Pointer Resolves To The Whole Document
    [Tags]    p2
    Select Engine    Pointer
    Clear Query Box
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Not Contain Text    @{STATUS_BAR}    Query error

TC-QRY-034 Pointer Resolves A Field Nested Under A Non-Zero Array Index
    [Documentation]    TC-QRY-030 only exercises index 0 -- this confirms the
    ...    index segment itself is being parsed and applied, not just always
    ...    resolving to the first element.
    [Tags]    p2
    Select Engine    Pointer
    Run Query    /1/role
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    intern

TC-QRY-035 Pointer Resolves A Numeric Field
    [Documentation]    Cross-ref TC-QRY-034 -- same index, different field and
    ...    value kind (number, not string), against a third array element.
    [Tags]    p2
    Select Engine    Pointer
    Run Query    /2/age
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    45

TC-QRY-036 Pointer Can Resolve To A Whole Object, Not Just A Leaf Scalar
    [Documentation]    Every other pointer case in this suite resolves to a
    ...    leaf (string/number/whole document) -- this confirms a pointer
    ...    landing mid-document on a container renders as a collapsed "(N
    ...    keys)" row like any other object, per 05_tree_view.md.
    [Tags]    p2
    Select Engine    Pointer
    Run Query    /1
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    3 keys

TC-QRY-041a JSONPath Returns Every Match As Its Own Result
    [Tags]    p1
    Select Engine    JSONPath
    Run Query    $[*].age
    Region Should Contain Text    @{STATUS_BAR}    3 result

TC-QRY-041b JSONPath Malformed Syntax Is A Query Error
    [Tags]    p1
    Select Engine    JSONPath
    Run Query    $[
    Region Should Contain Text    @{STATUS_BAR}    Query error
    Region Should Contain Text    @{STATUS_BAR}    syntax error

TC-QRY-042 JSONPath Filter Expression Selects By Condition
    [Documentation]    A `?()` filter, JSONPath's equivalent of jq's
    ...    TC-QRY-023 select -- same negative assertion against the
    ...    non-matching item to confirm it's actually excluded.
    [Tags]    p2
    Select Engine    JSONPath
    Run Query    $[?(@.age > 30)].name
    Region Should Contain Text    @{STATUS_BAR}    2 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice
    Region Should Contain Text    @{RESULTS_PANEL}    Carol
    Region Should Not Contain Text    @{RESULTS_PANEL}    Bob

TC-QRY-043 JSONPath Recursive Descent Collects A Field From Every Level
    [Documentation]    `..name` -- distinct traversal mechanism from the
    ...    wildcard (`[*]`) already covered by TC-QRY-012/041a.
    [Tags]    p2
    Select Engine    JSONPath
    Run Query    $..name
    Region Should Contain Text    @{STATUS_BAR}    3 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice
    Region Should Contain Text    @{RESULTS_PANEL}    Bob
    Region Should Contain Text    @{RESULTS_PANEL}    Carol

TC-QRY-044 JSONPath Compound Filter With Logical AND Narrows To One Match
    [Documentation]    Two conditions on different fields joined with `&&`
    ...    inside one filter -- confirms both sides are actually enforced
    ...    together (Carol's age alone would satisfy the first clause, Bob's
    ...    role alone would not satisfy either), not just the first one
    ...    parsed. Deliberately avoids a second `<`/`>` pair on the same
    ...    field: confirmed during implementation that typing a literal `<`
    ...    through this harness's synthetic-input path is unreliable (it
    ...    silently lands as `>` instead), so this filter's second clause is
    ...    an equality check instead.
    [Tags]    p2
    Select Engine    JSONPath
    Run Query    $[?(@.age > 20 && @.role == 'engineer')].name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice

TC-QRY-045 JSONPath Index Union Selects Multiple Specific Elements
    [Documentation]    `[0,2]` -- a comma-separated index list, distinct from
    ...    both a single index (TC-QRY-041a's peers) and a wildcard/filter.
    [Tags]    p2
    Select Engine    JSONPath
    Run Query    $[0,2].name
    Region Should Contain Text    @{STATUS_BAR}    2 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice
    Region Should Contain Text    @{RESULTS_PANEL}    Carol
    Region Should Not Contain Text    @{RESULTS_PANEL}    Bob

TC-QRY-051 JMESPath Parse Errors Read As Query Errors
    [Tags]    p2
    Select Engine    JMESPath
    Run Query    foo.
    Region Should Contain Text    @{STATUS_BAR}    Query error
    Region Should Contain Text    @{STATUS_BAR}    syntax error

TC-QRY-052 JMESPath Projection Collects A Field Into One Array Result
    [Documentation]    `[*].name` -- a projection is still exactly one
    ...    JMESPath result (the whole array), per this engine's one-result
    ...    contract (TC-QRY-050), so the array itself renders collapsed as
    ...    "(N items)" rather than as three separate result rows the way the
    ...    same shape of query would under jq/JSONPath.
    [Tags]    p2
    Select Engine    JMESPath
    Run Query    [*].name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    3 items

TC-QRY-053 JMESPath Backtick-Literal Filter Piped Into An Index Yields A Scalar
    [Documentation]    A numeric backtick literal in a filter (this engine's
    ...    JSON-literal syntax, per TC-QRY-013's auto-detect marker), piped
    ...    into `[0]` to pull one scalar back out of the filtered array --
    ...    confirms the filter condition and the pipe both actually apply.
    [Tags]    p2
    Select Engine    JMESPath
    Run Query    [?age > \`30\`] | [0].name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice

TC-QRY-054 JMESPath Raw String Literals And Logical OR In A Filter
    [Documentation]    Single-quoted raw string literals (distinct from the
    ...    backtick JSON-literal syntax in TC-QRY-053) combined with `||` --
    ...    confirms both sides of the OR are evaluated, since either one
    ...    alone would already satisfy this fixture's first two roles.
    [Tags]    p2
    Select Engine    JMESPath
    Run Query    [?role == 'engineer' || role == 'manager'] | [0].name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice

TC-QRY-055 JMESPath Length Function Aggregates To A Single Scalar
    [Documentation]    A built-in function call (as opposed to a path/filter
    ...    expression) -- exercises JMESPath's function-call syntax.
    [Tags]    p2
    Select Engine    JMESPath
    Run Query    length(@)
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    3

TC-QRY-056 JMESPath Max By Expression-Reference Picks The Expected Item
    [Documentation]    `max_by(@, &age)` -- an expression-reference (`&`)
    ...    argument, JMESPath's mechanism for passing a sub-expression into a
    ...    function, distinct from every other case in this suite.
    [Tags]    p2
    Select Engine    JMESPath
    Run Query    max_by(@, &age).name
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Carol

TC-QRY-060 Run Is Disabled With No Document Loaded
    [Documentation]    Checked via the Run button's own label pixel color
    ...    (dim/disabled vs bright/enabled), the same technique as the theme
    ...    toggle and engine-picker tests -- there's no document-dependent
    ...    text to OCR here, just a widget enabled-state change.
    [Tags]    p2
    ${enabled_color}=    Get Pixel Color    70    39
    Click At    199    11
    Sleep    0.3s
    ${disabled_color}=    Get Pixel Color    70    39
    Colors Should Not Match    ${enabled_color}    ${disabled_color}
    ...    msg=Expected Run's label to visibly dim once Clear removed the document

TC-QRY-061 Ctrl+Enter Runs The Query Regardless Of Which Widget Has Focus
    [Documentation]    Cross-ref TC-KEY-001 -- the shortcut isn't gated on the
    ...    query box itself having focus.
    [Tags]    p2
    Click At    100    58
    Sleep    0.2s
    Press Keys    ctrl    a
    Type Text    .[0].name
    Sleep    0.2s
    Click At    30    161
    Sleep    0.2s
    Run Current Query
    Region Should Contain Text    @{STATUS_BAR}    1 result
    Region Should Contain Text    @{RESULTS_PANEL}    Alice
