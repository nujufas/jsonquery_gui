//! The tutorial's tabs of help (`tutorial/help.rs`): General — the command line and
//! the rest — and MCP in the build that has the server. Their lessons are words and
//! commands to copy, with nothing to run.

use super::*;

/// The tutorial open on the tab `tab`.
fn help_tab(h: &mut Harness, tab: &str) {
    let ctx = h.ctx.clone();
    h.app.tutorial.open_or_focus(&ctx);
    h.settle();
    press_on_top(h, tab);
    h.settle();
}

#[test]
fn the_general_tab_starts_on_the_command_line_and_copies_a_command() {
    let mut h = Harness::new();
    help_tab(&mut h, "General");
    for text in [
        "Command line",
        "Open a document as the app starts",
        "A file",
        "jsonquery_gui data.json",
        "What another program prints",
        "Good to know",
    ] {
        assert!(is_drawn(&h, text), "{text:?}: {:?}", h.texts());
    }
    let copies = h.pop_buttons("Copy");
    assert_eq!(copies.len(), 3, "one for each command");
    h.click(copies[0].x, copies[0].y);
    h.settle();
    assert_eq!(
        h.copied.last().map(String::as_str),
        Some("jsonquery_gui data.json")
    );
    assert!(
        is_drawn(&h, "Copied."),
        "the status says so: {:?}",
        h.texts()
    );
}

#[test]
fn the_general_tab_has_the_tools_on_the_command_line_and_the_other_help() {
    let mut h = Harness::new();
    help_tab(&mut h, "General");
    press_on_top(&mut h, "Open a tool");
    h.settle();
    assert!(
        is_drawn(&h, "jsonquery_gui --diff old.json new.json"),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn_containing(&h, "git difftool"));
    // The other topic opens, as a topic does, when it is clicked.
    press_on_top(&mut h, "Working with the app");
    h.settle();
    for lesson in ["Keys", "Big files", "The Workspace", "Read more"] {
        assert!(
            is_drawn(&h, lesson),
            "{lesson:?} is listed: {:?}",
            h.texts()
        );
    }
    press_on_top(&mut h, "Keys");
    h.settle();
    assert!(
        is_drawn_containing(&h, "opens this window"),
        "{:?}",
        h.texts()
    );
}

#[test]
fn the_filter_finds_a_help_lesson_by_what_it_says() {
    let mut h = Harness::new();
    help_tab(&mut h, "General");
    press_on_top(&mut h, "Filter lessons…");
    h.type_text("difftool");
    h.settle();
    assert!(is_drawn(&h, "Open a tool"), "{:?}", h.texts());
    assert!(!is_drawn(&h, "Keys"), "what does not say it is not listed");
}

#[cfg(feature = "mcp")]
#[test]
fn the_mcp_tab_says_how_to_start_the_server_and_what_quickai_needs() {
    let mut h = Harness::new();
    help_tab(&mut h, "AI assistants");
    for text in [
        "Getting started",
        "What MCP is",
        "Add it to an assistant",
        "Using it",
    ] {
        assert!(is_drawn(&h, text), "{text:?}: {:?}", h.texts());
    }
    press_on_top(&mut h, "Add it to an assistant");
    press_on_top(&mut h, "QuickAI");
    h.settle();
    assert!(
        is_drawn_containing(&h, "Qwen3.5-9B-Q4_K_M"),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn_containing(&h, "32K tokens or more"));
    // Where QuickAI is got, a link at the top, opened in the browser.
    press_on_top(&mut h, "Get QuickAI: quickai.everybodyai.dev");
    h.settle();
    assert_eq!(
        h.opened.last().map(String::as_str),
        Some("https://quickai.everybodyai.dev")
    );
    // Then its two pictures, under what is good to know (lower than the pane shows
    // without scrolling: made, and laid out).
    assert_eq!(h.app.tutorial.pictures_made(), 2);
}

#[cfg(not(feature = "mcp"))]
#[test]
fn the_build_without_the_server_has_no_mcp_tab() {
    let mut h = Harness::new();
    help_tab(&mut h, "General");
    assert!(!is_drawn(&h, "AI assistants"), "{:?}", h.texts());
}

/// Where the tab `title` of the tutorial is drawn (the last text of that name: the
/// tutorial's window is on top).
fn tab_rect(h: &Harness, title: &str) -> egui::Rect {
    text_rects(&h.shapes)
        .into_iter()
        .rfind(|(t, _)| t == title)
        .unwrap_or_else(|| panic!("{title:?} is drawn: {:?}", h.texts()))
        .1
}

#[test]
fn the_tabs_are_one_row_in_the_window_as_it_opens_and_two_in_the_narrowest() {
    let mut h = Harness::new();
    help_tab(&mut h, "General");
    let (jq, general) = (tab_rect(&h, "jq"), tab_rect(&h, "General"));
    assert_eq!(general.center().y, jq.center().y, "one row of tabs");
    assert!(
        general.min.x > tab_rect(&h, "Tools").max.x,
        "General is last"
    );

    // The narrowest the tutorial's window may be: every tab is in it, on two rows.
    let mut h = Harness::new();
    h.screen = egui::vec2(720.0, 600.0);
    help_tab(&mut h, "General");
    let general = tab_rect(&h, "General");
    assert!(general.max.x <= 720.0, "General ends at {}", general.max.x);
}
