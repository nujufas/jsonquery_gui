//! A web address in the workspace: the `.url` item as a row of the list and what the pane
//! says of it, loading it as the Source, saving one (the new choice of the form that
//! saves), and what is done with it as with any item. (The data layer's tests are in
//! `workspace/tests.rs`, the file's in `workspace/address.rs`.)
//!
//! Where a test asks for a web address to be loaded it stops at the asking: the commands
//! the app sends to its worker are kept here instead of done, so that nothing is fetched.
//! The two tests that go on to the load fetch from a server on this computer.

use super::workspace_drag_tests::{drag, let_go, lit, notice, pick_up};
use super::workspace_tests::{
    click_button, folder_of, index_of, pick, press_last, press_one_of, press_starting, put, read,
    row_at, start_with_more, type_over, OPEN,
};
use super::*;
use crate::workspace::{sha256_hex, shortcut_text};
use crate::workspace_pane::Action;
use std::path::PathBuf;

const ADDRESS: &str = "https://example.com/data/feed.json";

/// The app, with a workspace that has web addresses in it as well: `Feeds/feed.url`, with a file
/// called for it beside it, and `Feeds/bad.url`, which holds no address.
fn start_with_feeds(name: &str) -> (Harness, PathBuf) {
    let feed = shortcut_text(ADDRESS);
    start_with_more(
        name,
        OPEN,
        &[
            ("Feeds/feed.url", &feed),
            ("Feeds/feed.sample.json", "[1]"),
            ("Feeds/bad.url", "not an address\n"),
        ],
    )
}

/// From now on the commands that the app sends to its worker are kept in the channel that is given,
/// and not done.
fn keep_commands(h: &mut Harness) -> Receiver<Command> {
    let (sender, receiver) = crossbeam_channel::unbounded();
    h.app.cmd_tx = sender;
    receiver
}

/// The web addresses that were asked for since the last time.
fn asked_for(commands: &Receiver<Command>) -> Vec<String> {
    commands
        .try_iter()
        .filter_map(|command| match command {
            Command::OpenUrl(url) => Some(url),
            _ => None,
        })
        .collect()
}

/// Click the text called `text` that was drawn last, which is the one of a menu or a window: those
/// are in front of the pane, and the pane may have a text of the same name.
fn press_in_front(h: &mut Harness, text: &str) {
    let rect = text_rects(&h.shapes)
        .into_iter()
        .rev()
        .find(|(t, _)| t == text)
        .unwrap_or_else(|| panic!("{text:?} is drawn: {:?}", h.texts()))
        .1;
    h.click(rect.center().x, rect.center().y);
}

/// Click the middle of the box with the id `id`.
fn click_box(h: &mut Harness, id: &str) {
    let at = h
        .ctx
        .read_response(egui::Id::new(id))
        .unwrap_or_else(|| panic!("the box {id} is drawn"))
        .rect
        .center();
    h.click(at.x, at.y);
}

/// What the box with the id `id` shows: its text, or, when it has none, its hint. (The last text
/// in it that is drawn: a window is in front of the panes it lies over, and an empty text is
/// drawn after the hint.)
fn box_text(h: &Harness, id: &str) -> String {
    let rect = h
        .ctx
        .read_response(egui::Id::new(id))
        .unwrap_or_else(|| panic!("the box {id} is drawn"))
        .rect;
    text_rects(&h.shapes)
        .into_iter()
        .rev()
        .find(|(text, at)| !text.is_empty() && rect.contains(at.center()))
        .map(|(text, _)| text)
        .unwrap_or_default()
}

const ADDRESS_BOX: &str = "workspace_save_address";
const NAME_BOX: &str = "workspace_save_name";

/// A document as it is when it has been loaded from `source`, though nothing was fetched.
fn show_document_from(h: &mut Harness, source: DocumentSource) {
    let doc = jsonquery_core::load_bytes(b"[1, 2]", source).unwrap();
    h.app.document_loaded(Arc::new(doc));
    h.settle();
}

/// Answer one request, over HTTP, from a port of this computer's own, with `status` (`200 OK`)
/// and `body`; the address to ask at.
fn serve_once(status: &'static str, body: &'static str) -> String {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        // The request, up to the blank line that ends its headers.
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => request.extend_from_slice(&chunk[..n]),
            }
        }
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body.as_bytes()));
    });
    format!("http://127.0.0.1:{port}/data.json")
}

// ---- the list ----------------------------------------------------------------------------------

#[test]
fn a_web_address_is_a_row_with_a_url_tag_and_a_chip_of_its_own_narrows_the_list_to_it() {
    let (mut h, file) = start_with_feeds("url-list");
    assert!(
        is_drawn(&h, "feed") && is_drawn(&h, "bad"),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn(&h, "url"), "its tag");
    // The chip is after Docs, on its row or the next.
    let (docs, urls) = (rect_of(&h, "Docs"), rect_of(&h, "URLs"));
    assert!(
        urls.top() > docs.bottom() - 2.0
            || (urls.top() - docs.top()).abs() < 2.0 && urls.left() > docs.right(),
        "{docs:?} {urls:?}"
    );

    press(&mut h, "URLs");
    assert!(is_drawn(&h, "feed") && is_drawn(&h, "bad"));
    for other in ["scratch", "orders", "bump", "logins"] {
        assert!(!is_drawn(&h, other), "{other}");
    }
    // The file called for it is not its sample: no paperclip.
    assert!(!is_drawn(&h, "📎"), "{:?}", h.texts());

    // No other chip takes it, and All does.
    for chip in ["Queries", "Schemas", "Patches", "Docs"] {
        press(&mut h, chip);
        assert!(!is_drawn(&h, "feed") && !is_drawn(&h, "bad"), "{chip}");
    }
    assert!(
        is_drawn_containing(&h, "Nothing here matches"),
        "Docs: {:?}",
        h.texts()
    );
    press(&mut h, "All");
    assert!(is_drawn(&h, "feed") && is_drawn(&h, "scratch"));

    // The filter finds it by its name, like the others.
    press(&mut h, "Filter by name…");
    h.type_text("bad");
    h.settle();
    assert!(is_drawn(&h, "bad") && !is_drawn(&h, "feed") && !is_drawn(&h, "scratch"));
    forget_settings(&file);
}

#[test]
fn a_click_on_a_web_address_shows_its_file_its_address_and_a_button_that_loads_it() {
    let (mut h, file) = start_with_feeds("url-details");
    pick(&mut h, "feed");
    assert_eq!(h.app.workspace_pane.selected_path(), Some("Feeds/feed.url"));
    assert!(is_drawn(&h, "Feeds/feed.url"), "its file");
    assert!(
        is_drawn(&h, ADDRESS),
        "the address is the line that shows what it holds"
    );
    assert!(
        is_drawn(&h, "Web address"),
        "where the others say what sample they have"
    );
    assert!(is_drawn(&h, "Load from the web"));
    // The file beside it called for it is not its sample, and there is no button for one.
    assert!(!is_drawn_containing(&h, "Sample:") && !is_drawn(&h, "No sample"));
    for button in [
        "Open",
        "Open with sample",
        "With its sample",
        "Open as Source",
    ] {
        assert!(!is_drawn(&h, button), "{button}");
    }
    assert!(h.app.source_input.is_empty(), "a click loads nothing");

    // A file with no address in it shows what it holds, and the button is there all the same.
    pick(&mut h, "bad");
    assert!(is_drawn(&h, "not an address"), "{:?}", h.texts());
    assert!(is_drawn(&h, "Web address") && is_drawn(&h, "Load from the web"));
    assert!(!is_drawn(&h, ADDRESS));
    forget_settings(&file);
}

// ---- loading it as the Source ----------------------------------------------------------------------

#[test]
fn a_web_address_is_loaded_as_the_source_by_the_button_a_double_click_and_the_menu() {
    for way in ["the button", "a double click", "the menu"] {
        let (mut h, file) = start_with_feeds("url-open");
        let commands = keep_commands(&mut h);
        dock_tools(&mut h, Tool::Format);
        assert!(h.app.tools.page_in_front());
        h.app.load_error = Some("what was said of the last one".to_owned());
        let at = row_at(&h, "feed");
        match way {
            "the button" => {
                pick(&mut h, "feed");
                press(&mut h, "Load from the web");
            }
            "a double click" => h.double_click(at.x, at.y),
            _ => {
                h.right_click(at.x, at.y);
                press_in_front(&mut h, "Load from the web");
            }
        }
        assert_eq!(asked_for(&commands), [ADDRESS], "{way}");
        assert_eq!(
            h.app.source_input, ADDRESS,
            "{way}: it is in the Source field"
        );
        assert!(
            !h.app.tools.page_in_front(),
            "{way}: the Query page is in front"
        );
        assert_eq!(
            h.app.load_error, None,
            "{way}: what was said before is not said now"
        );
        assert_eq!(notice(&h), None, "{way}: nothing is said under the list");
        assert!(
            h.app.workspace_pane.take_actions().is_empty(),
            "{way}: the action was done"
        );
        forget_settings(&file);
    }
}

#[test]
fn what_the_pane_asks_for_is_what_pressing_load_on_the_same_address_would_ask_for() {
    // Typed in the Source field, and Load pressed.
    let mut typed = Harness::new();
    let typed_commands = keep_commands(&mut typed);
    typed.app.source_input = ADDRESS.to_owned();
    typed.settle();
    press(&mut typed, "Load");
    let typed_asked_for = asked_for(&typed_commands);
    assert_eq!(typed_asked_for, [ADDRESS]);

    // Asked for by the pane.
    let (mut h, file) = start_with_more("url-action", OPEN, &[]);
    let commands = keep_commands(&mut h);
    h.app.source_input = "something that was typed".to_owned();
    h.app
        .workspace_pane
        .push_action(Action::OpenUrl(ADDRESS.to_owned()));
    h.settle();
    assert_eq!(asked_for(&commands), typed_asked_for);
    assert_eq!(h.app.source_input, typed.app.source_input);
    forget_settings(&file);
}

#[test]
fn a_file_with_no_web_address_in_it_is_said_and_nothing_is_loaded() {
    let files: Vec<(String, String)> = [
        ("empty", ""),
        ("words", "not an address\n"),
        (
            "ftp",
            "[InternetShortcut]\r\nURL=ftp://example.com/a.json\r\n",
        ),
        ("spaces", "URL=https://exa mple.com/a\n"),
        ("nohost", "URL=https://\n"),
        (
            "novalue",
            "[InternetShortcut]\r\nURL=\r\nURL=https://example.com/second\r\n",
        ),
        ("several", "https://example.com/a\nhttps://example.com/b\n"),
        ("no scheme", "URL=example.com/a.json\n"),
    ]
    .iter()
    .map(|(name, text)| (format!("Bad/{name}.url"), (*text).to_owned()))
    .collect();
    let more: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (mut h, file) = start_with_more("url-bad", OPEN, &more);
    let commands = keep_commands(&mut h);
    h.app.source_input = "as it was".to_owned();
    for (rel, _) in &files {
        let name = rel.trim_start_matches("Bad/").trim_end_matches(".url");
        let at = row_at(&h, name);
        h.double_click(at.x, at.y);
        assert_eq!(
            notice(&h).as_deref(),
            Some("There is no web address in it"),
            "{name}"
        );
        assert!(asked_for(&commands).is_empty(), "{name}");
        assert_eq!(h.app.source_input, "as it was", "{name}");
    }
    // The button and the menu say the same.
    for way in ["the button", "the menu"] {
        h.pause(9.0);
        assert_eq!(notice(&h), None, "{way}: what was said before is gone");
        if way == "the button" {
            pick(&mut h, "words");
            press(&mut h, "Load from the web");
        } else {
            let at = row_at(&h, "words");
            h.right_click(at.x, at.y);
            press_in_front(&mut h, "Load from the web");
        }
        assert_eq!(
            notice(&h).as_deref(),
            Some("There is no web address in it"),
            "{way}"
        );
        assert!(asked_for(&commands).is_empty(), "{way}");
    }
    assert!(h.app.doc.is_none());
    forget_settings(&file);
}

#[test]
fn the_ways_a_file_may_hold_its_address_are_all_read() {
    let files = [
        (
            "bom",
            "\u{feff}[InternetShortcut]\r\nURL=https://example.com/bom.json\r\n",
            "https://example.com/bom.json",
        ),
        (
            "lower",
            "[InternetShortcut]\nurl = https://example.com/lower.json\n",
            "https://example.com/lower.json",
        ),
        (
            "sections",
            "[Other]\nName=x\n[InternetShortcut]\nIconIndex=0\nURL=https://example.com/sections.json\n",
            "https://example.com/sections.json",
        ),
        (
            "loose",
            "  https://example.com/loose.json  \r\n",
            "https://example.com/loose.json",
        ),
        (
            "query",
            "URL=HTTP://example.com/q?a=b&c=d#e\r\n",
            "HTTP://example.com/q?a=b&c=d#e",
        ),
    ];
    let names: Vec<String> = files
        .iter()
        .map(|(n, _, _)| format!("Forms/{n}.url"))
        .collect();
    let more: Vec<(&str, &str)> = names
        .iter()
        .zip(files)
        .map(|(rel, (_, text, _))| (rel.as_str(), text))
        .collect();
    let (mut h, file) = start_with_more("url-lenient", OPEN, &more);
    let commands = keep_commands(&mut h);
    for (name, _, address) in files {
        let at = row_at(&h, name);
        h.double_click(at.x, at.y);
        assert_eq!(asked_for(&commands), [address], "{name}");
        assert_eq!(h.app.source_input, address, "{name}");
    }
    forget_settings(&file);
}

#[test]
fn a_web_address_is_fetched_and_shown_as_the_source_when_it_is_opened() {
    let address = serve_once("200 OK", r#"{"orders": [1, 2, 3]}"#);
    let shortcut = shortcut_text(&address);
    let (mut h, file) = start_with_more("url-fetch", OPEN, &[("Feeds/local.url", &shortcut)]);
    let at = row_at(&h, "local");
    h.double_click(at.x, at.y);
    wait_for(&mut h, "the document", |h| h.app.doc.is_some());
    let doc = h.app.doc.clone().expect("the document");
    assert_eq!(doc.source.label(), address, "it was loaded from the web");
    assert_eq!(doc.tree(), Some(&serde_json::json!({"orders": [1, 2, 3]})));
    assert_eq!(h.app.source_input, address);
    assert!(!h.app.loading && h.app.load_error.is_none());
    assert!(!is_drawn_containing(&h, "Load error"), "{:?}", h.texts());
    forget_settings(&file);
}

#[test]
fn a_web_address_that_cannot_be_fetched_is_said_the_way_a_typed_one_is() {
    let address = serve_once("404 Not Found", "nothing here");
    let shortcut = shortcut_text(&address);
    let (mut h, file) = start_with_more("url-refused", OPEN, &[("Feeds/gone.url", &shortcut)]);
    let at = row_at(&h, "gone");
    h.double_click(at.x, at.y);
    wait_for(&mut h, "the reason", |h| h.app.load_error.is_some());
    let said = h.app.load_error.clone().expect("a reason");
    assert!(said.contains(&address), "{said}");
    assert!(!h.app.loading && h.app.doc.is_none());
    assert!(is_drawn_containing(&h, "Load error: "), "{:?}", h.texts());
    assert_eq!(h.app.source_input, address, "and it is still in the field");
    forget_settings(&file);
}

// ---- saving one ---------------------------------------------------------------------------------------

#[test]
fn a_web_address_is_typed_in_the_form_and_saved_as_a_shortcut_with_an_id_and_a_sha() {
    let (mut h, file) = start_with_feeds("url-save");
    let dir = folder_of(&file);
    h.app.query_text = ".x".to_owned();
    h.settle();
    press(&mut h, "💾");
    assert!(is_drawn(&h, "Save in the workspace"));
    assert!(
        !is_drawn(&h, "https://…"),
        "no box for an address while the query is saved"
    );
    press(&mut h, "Web address");
    assert_eq!(
        box_text(&h, ADDRESS_BOX),
        "https://…",
        "its hint, while it is empty"
    );
    // (The box has the focus: it is what is typed first.)
    h.type_text("https://example.com/data/orders.json?limit=5");
    h.settle();
    assert_eq!(
        box_text(&h, NAME_BOX),
        "orders",
        "the name is made from the address"
    );
    assert!(is_drawn(&h, "Saved as orders.url"), "{:?}", h.texts());
    press_last(&mut h, "Save");
    assert!(!is_drawn(&h, "Save in the workspace"), "the window closed");

    let on_disk = std::fs::read(dir.join("orders.url")).expect("the file");
    assert_eq!(
        on_disk,
        b"[InternetShortcut]\r\nURL=https://example.com/data/orders.json?limit=5\r\n"
    );
    let entry = &index_of(&dir)["items"]["orders.url"];
    assert_eq!(entry["sha256"], sha256_hex(&on_disk));
    assert_eq!(entry["bytes"], on_disk.len());
    assert!(entry["id"].as_str().is_some_and(|id| id.len() == 36));
    assert!(entry.get("sample").is_none());
    assert!(!dir.join("orders.sample.json").exists());
    assert_eq!(h.app.workspace_pane.selected_path(), Some("orders.url"));
    assert_eq!(h.app.workspace_pane.notice_text(), Some("Saved “orders”"));
    assert_eq!(h.app.query_text, ".x", "the query box is as it was");
    let tags = h.texts().iter().filter(|(t, _)| t == "url").count();
    assert_eq!(
        tags,
        3,
        "it is in the list, with the two there were: {:?}",
        h.texts()
    );
    forget_settings(&file);
}

#[test]
fn every_choice_of_the_form_is_in_its_window_in_one_row() {
    let (mut h, file) = start_with_feeds("url-choices");
    h.app.query_text = ".x".to_owned();
    h.settle();
    press(&mut h, "💾");
    let window_of = |h: &Harness| {
        h.ctx
            .memory(|memory| memory.area_rect(egui::Id::new("workspace_save_form")))
            .expect("the window")
    };
    let window = window_of(&h);
    let first = rect_of(&h, "the query");
    for choice in [
        "the query",
        "a schema",
        "a patch",
        "the Source",
        "Web address",
    ] {
        let rect = rect_of(&h, choice);
        assert!(
            window.contains_rect(rect),
            "{choice}: {rect:?} is not in {window:?}"
        );
        assert!(
            (rect.center().y - first.center().y).abs() < 2.0,
            "{choice}: one row"
        );
    }
    let order = [
        "the query",
        "a schema",
        "a patch",
        "the Source",
        "Web address",
    ]
    .map(|choice| rect_of(&h, choice).left());
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "in this order: {order:?}"
    );
    // The box of the address is in it too, and the window is no wider than it was.
    press(&mut h, "Web address");
    let wider = window_of(&h);
    assert!(wider.contains_rect(rect_of(&h, "https://…")), "{wider:?}");
    assert!(
        (wider.width() - window.width()).abs() < 0.5,
        "{window:?} {wider:?}"
    );
    forget_settings(&file);
}

#[test]
fn the_box_starts_with_the_address_the_open_document_was_loaded_from() {
    let (mut h, file) = start_with_feeds("url-offered");
    show_document_from(&mut h, DocumentSource::Url(ADDRESS.to_owned()));
    // (The field says something else, which is not what was loaded.)
    h.app.source_input = "https://example.com/typed/other.json".to_owned();
    h.settle();
    press(&mut h, "💾");
    press(&mut h, "Web address");
    assert_eq!(box_text(&h, ADDRESS_BOX), ADDRESS);
    assert_eq!(
        box_text(&h, NAME_BOX),
        "feed",
        "and the name is made from it"
    );
    assert!(is_drawn(&h, "Saved as feed.url"));
    forget_settings(&file);
}

#[test]
fn an_address_in_the_source_field_that_was_not_loaded_is_offered_when_none_was_loaded_from() {
    // A document that came from a file, and an address typed after it.
    let (mut h, file) = start_with_feeds("url-typed");
    let path = PathBuf::from("data/records.json");
    show_document_from(&mut h, DocumentSource::File(path));
    h.app.source_input = "  https://example.com/typed/list.json \n".to_owned();
    h.settle();
    press(&mut h, "💾");
    press(&mut h, "Web address");
    assert_eq!(
        box_text(&h, ADDRESS_BOX),
        "https://example.com/typed/list.json"
    );
    assert_eq!(box_text(&h, NAME_BOX), "list");
    forget_settings(&file);

    // With nothing loaded at all.
    let (mut h, file) = start_with_feeds("url-typed-none");
    h.app.source_input = "https://example.com/never/loaded.json".to_owned();
    h.settle();
    press(&mut h, "💾");
    press(&mut h, "Web address");
    assert_eq!(
        box_text(&h, ADDRESS_BOX),
        "https://example.com/never/loaded.json"
    );
    forget_settings(&file);

    // A document that was loaded from an address that cannot be one is not offered; the field is.
    let (mut h, file) = start_with_feeds("url-typed-bad-source");
    show_document_from(
        &mut h,
        DocumentSource::Url("https://exa mple.com/a".to_owned()),
    );
    h.app.source_input = "https://example.com/typed.json".to_owned();
    h.settle();
    press(&mut h, "💾");
    press(&mut h, "Web address");
    assert_eq!(box_text(&h, ADDRESS_BOX), "https://example.com/typed.json");
    forget_settings(&file);
}

#[test]
fn the_box_starts_empty_when_there_is_no_address_to_offer() {
    for (case, source) in [
        ("a pasted document", DocumentSource::Pasted),
        ("a file", DocumentSource::File(PathBuf::from("a.json"))),
        (
            "an address that cannot be one",
            DocumentSource::Url("ftp://example.com/a".to_owned()),
        ),
    ] {
        let (mut h, file) = start_with_feeds("url-empty");
        show_document_from(&mut h, source);
        h.app.source_input = "data/records.json".to_owned();
        h.settle();
        press(&mut h, "💾");
        press(&mut h, "Web address");
        assert_eq!(
            box_text(&h, ADDRESS_BOX),
            "https://…",
            "{case}: only its hint"
        );
        assert!(!is_drawn_containing(&h, "Saved as"), "{case}: no name yet");
        forget_settings(&file);
    }
    // Nothing loaded, nothing typed.
    let (mut h, file) = start_with_feeds("url-empty-none");
    press(&mut h, "💾");
    press(&mut h, "Web address");
    assert_eq!(box_text(&h, ADDRESS_BOX), "https://…");
    forget_settings(&file);
}

#[test]
fn the_form_opens_on_what_it_did_and_a_web_address_is_not_what_it_opens_on() {
    // A query in the box, and a document that came from the web: the query.
    let (mut h, file) = start_with_feeds("url-default-query");
    load(&mut h, r#"{"a": 1}"#);
    show_document_from(&mut h, DocumentSource::Url(ADDRESS.to_owned()));
    h.app.query_text = ".a".to_owned();
    h.settle();
    press(&mut h, "💾");
    assert!(
        is_drawn_containing(&h, "Keep the Source as its sample"),
        "{:?}",
        h.texts()
    );
    assert!(!is_drawn(&h, "https://…"), "{:?}", h.texts());
    forget_settings(&file);

    // Nothing in the query box: the document that is open, as it was.
    let (mut h, file) = start_with_feeds("url-default-source");
    show_document_from(&mut h, DocumentSource::Url(ADDRESS.to_owned()));
    h.app.source_input.clear();
    h.settle();
    press(&mut h, "💾");
    assert!(is_drawn(&h, "Save in the workspace"));
    assert!(!is_drawn(&h, "https://…"), "{:?}", h.texts());
    assert!(!is_drawn_containing(&h, "Keep the Source as its sample"));
    forget_settings(&file);

    // A page of the Tools that asks for a schema gets a schema.
    let (mut h, file) = start_with_feeds("url-default-schema");
    show_document_from(&mut h, DocumentSource::Url(ADDRESS.to_owned()));
    open_tool(&mut h, Tool::Validate);
    h.app
        .tools
        .fill(Tool::Validate, "Schema", r#"{"type": "number"}"#);
    h.app.tools.fill(Tool::Validate, "Document", "1");
    h.settle();
    press(&mut h, "Save schema…");
    assert!(
        is_drawn_containing(&h, "Keep the Document box of Validate"),
        "{:?}",
        h.texts()
    );
    assert!(!is_drawn(&h, "https://…"));
    forget_settings(&file);
}

#[test]
fn the_name_is_made_from_the_address_until_it_is_typed_over() {
    let (mut h, file) = start_with_feeds("url-name");
    h.app.query_text = ".x".to_owned();
    h.settle();
    press(&mut h, "💾");
    press(&mut h, "Web address");
    h.type_text("https://example.com/data/one.json");
    h.settle();
    assert_eq!(box_text(&h, NAME_BOX), "one");
    type_over(&mut h, "https://example.com/data/two.ndjson?x=1#y");
    h.settle();
    assert_eq!(box_text(&h, NAME_BOX), "two", "it follows the address");
    assert!(is_drawn(&h, "Saved as two.url"), "{:?}", h.texts());
    // A name that is typed stays, whatever the address becomes.
    click_box(&mut h, NAME_BOX);
    type_over(&mut h, "mine");
    h.settle();
    assert!(is_drawn(&h, "Saved as mine.url"), "{:?}", h.texts());
    click_box(&mut h, ADDRESS_BOX);
    type_over(&mut h, "https://example.com/three.json");
    h.settle();
    assert_eq!(box_text(&h, NAME_BOX), "mine");
    // A name that is emptied is made from the address again, when that changes.
    click_box(&mut h, NAME_BOX);
    h.key(egui::Key::A, egui::Modifiers::COMMAND);
    h.key(egui::Key::Backspace, egui::Modifiers::NONE);
    click_box(&mut h, ADDRESS_BOX);
    type_over(&mut h, "https://example.com/four.json");
    h.settle();
    assert_eq!(box_text(&h, NAME_BOX), "four");
    // The choice made again does not take a typed name away.
    click_box(&mut h, NAME_BOX);
    type_over(&mut h, "kept");
    h.settle();
    press_starting(&mut h, "the query");
    press(&mut h, "Web address");
    assert_eq!(box_text(&h, NAME_BOX), "kept");
    forget_settings(&file);
}

#[test]
fn a_name_that_came_with_the_query_is_not_made_over_from_an_address() {
    let (mut h, file) = start_with_feeds("url-name-query");
    let at = row_at(&h, "scratch");
    h.double_click(at.x, at.y);
    press(&mut h, "💾");
    assert_eq!(
        box_text(&h, NAME_BOX),
        "scratch",
        "the query comes back under its name"
    );
    press(&mut h, "Web address");
    assert_eq!(
        box_text(&h, NAME_BOX),
        "scratch",
        "which a web address is saved under as well"
    );
    h.type_text("https://example.com/x/other.json");
    h.settle();
    assert_eq!(box_text(&h, NAME_BOX), "scratch");
    forget_settings(&file);
}

#[test]
fn what_is_not_a_web_address_is_said_and_the_form_does_not_save_it() {
    let (mut h, file) = start_with_feeds("url-invalid");
    let dir = folder_of(&file);
    press(&mut h, "💾");
    press(&mut h, "Web address");
    h.type_text("example.com/data.json");
    h.settle();
    for bad in [
        "example.com/data.json",
        "ftp://example.com/data.json",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "https://",
        "https://exa mple.com/a",
    ] {
        click_box(&mut h, ADDRESS_BOX);
        type_over(&mut h, bad);
        h.settle();
        assert!(
            is_drawn(
                &h,
                "That is not a web address (http:// or https://, no spaces)"
            ),
            "{bad}: {:?}",
            h.texts()
        );
        // Neither the button nor Enter in the box saves it.
        press_last(&mut h, "Save");
        click_box(&mut h, ADDRESS_BOX);
        h.key(egui::Key::Enter, egui::Modifiers::NONE);
        h.settle();
        assert!(
            is_drawn(&h, "Save in the workspace"),
            "{bad}: the window stays"
        );
        // (The button is off, and Enter does as it does: nothing was tried, so nothing failed.)
        assert!(
            !is_drawn(&h, "There is nothing to save"),
            "{bad}: {:?}",
            h.texts()
        );
    }
    let made: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(!made.iter().any(|name| name.ends_with(".url")), "{made:?}");

    // Nothing typed is not an error, and not a thing to save either.
    click_box(&mut h, ADDRESS_BOX);
    h.key(egui::Key::A, egui::Modifiers::COMMAND);
    h.key(egui::Key::Backspace, egui::Modifiers::NONE);
    h.settle();
    assert!(!is_drawn_containing(&h, "That is not a web address"));
    // A good one is saved, and the message is gone.
    type_over(&mut h, "  https://example.com/good/one.json  ");
    assert!(!is_drawn_containing(&h, "That is not a web address"));
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(
        read(&dir, "one.url"),
        "[InternetShortcut]\r\nURL=https://example.com/good/one.json\r\n",
        "it is trimmed"
    );
    forget_settings(&file);
}

#[test]
fn a_name_that_cannot_be_a_file_name_is_said_for_a_web_address_as_for_the_rest() {
    let (mut h, file) = start_with_feeds("url-bad-name");
    let dir = folder_of(&file);
    press(&mut h, "💾");
    press(&mut h, "Web address");
    h.type_text("https://example.com/con.json");
    h.settle();
    assert_eq!(box_text(&h, NAME_BOX), "con");
    assert!(is_drawn_containing(
        &h,
        "con is a name that Windows keeps for itself"
    ));
    press_last(&mut h, "Save");
    assert!(is_drawn(&h, "Save in the workspace") && !dir.join("con.url").exists());
    click_box(&mut h, NAME_BOX);
    type_over(&mut h, "a/b");
    h.settle();
    assert!(is_drawn_containing(&h, "A name can't have any of"));
    forget_settings(&file);
}

#[test]
fn a_web_address_of_a_name_that_is_there_is_replaced_and_keeps_its_id() {
    let (mut h, file) = start_with_feeds("url-replace");
    let dir = folder_of(&file);
    let before = index_of(&dir)["items"]["Feeds/feed.url"].clone();
    pick(&mut h, "feed");
    press(&mut h, "💾");
    press_in_front(&mut h, "Web address");
    h.type_text("https://example.com/other/feed.json");
    h.settle();
    assert!(is_drawn(&h, "Saved as Feeds/feed.url"), "{:?}", h.texts());
    assert!(
        is_drawn(
            &h,
            "There is a web address of that name here: saving replaces it."
        ),
        "{:?}",
        h.texts()
    );
    assert!(is_drawn(&h, "Replace"));
    press_last(&mut h, "Replace");
    assert_eq!(
        read(&dir, "Feeds/feed.url"),
        "[InternetShortcut]\r\nURL=https://example.com/other/feed.json\r\n"
    );
    let after = &index_of(&dir)["items"]["Feeds/feed.url"];
    assert_eq!(after["id"], before["id"], "the same item");
    assert_ne!(after["sha256"], before["sha256"]);
    assert_eq!(
        read(&dir, "Feeds/feed.sample.json"),
        "[1]",
        "what is beside it is as it was"
    );
    forget_settings(&file);
}

#[test]
fn a_web_address_can_be_saved_in_a_folder_and_a_new_one_made_for_it() {
    let (mut h, file) = start_with_feeds("url-folders");
    let dir = folder_of(&file);
    pick(&mut h, "Services");
    press(&mut h, "💾");
    press(&mut h, "Web address");
    h.type_text("https://example.com/api/rates.json");
    // (The folder that was picked is the one it goes in; and a folder is made inside it.)
    press(&mut h, "(none)");
    h.type_text("Daily");
    h.settle();
    assert!(
        is_drawn(&h, "Saved as Services/Daily/rates.url"),
        "{:?}",
        h.texts()
    );
    press_last(&mut h, "Save");
    assert_eq!(
        read(&dir, "Services/Daily/rates.url"),
        "[InternetShortcut]\r\nURL=https://example.com/api/rates.json\r\n"
    );
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("Services/Daily/rates.url")
    );
    forget_settings(&file);
}

#[test]
fn the_choice_of_a_sample_is_not_there_for_a_web_address_and_one_that_was_ticked_is_not_kept() {
    let (mut h, file) = start_with_feeds("url-no-sample-choice");
    let dir = folder_of(&file);
    load(&mut h, r#"{"a": 1}"#);
    h.app.query_text = ".a".to_owned();
    h.settle();
    press(&mut h, "💾");
    assert!(is_drawn_containing(&h, "Keep the Source as its sample"));
    press_starting(&mut h, "Keep the Source as its sample");
    press(&mut h, "Web address");
    assert!(
        !is_drawn_containing(&h, "Keep the Source as its sample"),
        "{:?}",
        h.texts()
    );
    h.type_text("https://example.com/kept.json");
    h.settle();
    assert!(
        !is_drawn_containing(&h, "and its sample"),
        "{:?}",
        h.texts()
    );
    press_last(&mut h, "Save");
    assert!(dir.join("kept.url").is_file());
    h.pause(0.5);
    assert!(!dir.join("kept.sample.json").exists());
    assert!(h.app.last_saved.is_none(), "{:?}", h.app.last_saved);
    // Back to the query: the choice is there again.
    press(&mut h, "💾");
    assert!(is_drawn_containing(&h, "Keep the Source as its sample"));
    forget_settings(&file);
}

#[test]
fn the_save_beside_run_has_the_choice_too() {
    let (mut h, file) = start_with_feeds("url-save-row");
    h.app.query_text = ".a".to_owned();
    h.settle();
    press(&mut h, "Save…");
    assert!(is_drawn(&h, "Save in the workspace") && is_drawn(&h, "Web address"));
    forget_settings(&file);
}

#[test]
fn a_folders_menu_saves_a_web_address_in_it_with_the_box_ready_for_it() {
    let (mut h, file) = start_with_feeds("url-folder-menu");
    let dir = folder_of(&file);
    let at = row_at(&h, "Services");
    h.right_click(at.x, at.y);
    assert!(is_drawn(&h, "Save the query here…"), "as it was");
    press(&mut h, "Save a web address here…");
    assert!(is_drawn(&h, "Save in the workspace"));
    assert_eq!(box_text(&h, ADDRESS_BOX), "https://…");
    // What is typed is the address: the box has the focus.
    h.type_text("https://example.com/api/orders.json");
    h.settle();
    assert!(
        is_drawn(&h, "Saved as Services/orders.url"),
        "{:?}",
        h.texts()
    );
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert_eq!(
        read(&dir, "Services/orders.url"),
        "[InternetShortcut]\r\nURL=https://example.com/api/orders.json\r\n"
    );
    // The schema of the same name is still there, and still has its sample.
    assert!(dir.join("Services/orders.schema.json").is_file());
    assert!(dir.join("Services/orders.sample.json").is_file());
    forget_settings(&file);

    // With an address to offer, it is in the box and the name is what is edited first.
    let (mut h, file) = start_with_feeds("url-folder-menu-offered");
    show_document_from(&mut h, DocumentSource::Url(ADDRESS.to_owned()));
    let at = row_at(&h, "Services");
    h.right_click(at.x, at.y);
    press(&mut h, "Save a web address here…");
    assert_eq!(box_text(&h, ADDRESS_BOX), ADDRESS);
    assert_eq!(box_text(&h, NAME_BOX), "feed");
    type_over(&mut h, "rates");
    h.settle();
    assert!(
        is_drawn(&h, "Saved as Services/rates.url"),
        "{:?}",
        h.texts()
    );
    forget_settings(&file);
}

// ---- what is done with one -----------------------------------------------------------------------------

#[test]
fn the_menu_of_a_web_address_is_that_of_an_item_that_has_no_sample() {
    let (mut h, file) = start_with_feeds("url-menu");
    let at = row_at(&h, "feed");
    h.right_click(at.x, at.y);
    for entry in [
        "Rename",
        "Duplicate",
        "Move to",
        "Copy text",
        "Show in the file manager",
        "Delete…",
    ] {
        assert!(is_drawn(&h, entry), "{entry}: {:?}", h.texts());
    }
    // (The button under the list, and the first entry of the menu.)
    let loads = h
        .texts()
        .iter()
        .filter(|(t, _)| t == "Load from the web")
        .count();
    assert_eq!(loads, 2, "{:?}", h.texts());
    assert!(!is_drawn(&h, "Open with sample") && !is_drawn(&h, "Open"));
    forget_settings(&file);
}

#[test]
fn a_web_address_is_renamed_in_the_list_and_keeps_its_id_and_leaves_the_file_called_for_it() {
    let (mut h, file) = start_with_feeds("url-rename");
    let dir = folder_of(&file);
    let id = index_of(&dir)["items"]["Feeds/feed.url"]["id"].clone();
    let at = row_at(&h, "feed");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    type_over(&mut h, "daily");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(dir.join("Feeds/daily.url").is_file() && !dir.join("Feeds/feed.url").exists());
    assert_eq!(
        read(&dir, "Feeds/feed.sample.json"),
        "[1]",
        "not taken along"
    );
    assert!(!dir.join("Feeds/daily.sample.json").exists());
    assert_eq!(index_of(&dir)["items"]["Feeds/daily.url"]["id"], id);
    assert!(is_drawn(&h, "daily") && !is_drawn(&h, "feed"));
    // A name another address has is said.
    let at = row_at(&h, "daily");
    h.right_click(at.x, at.y);
    press(&mut h, "Rename");
    type_over(&mut h, "bad");
    h.key(egui::Key::Enter, egui::Modifiers::NONE);
    h.settle();
    assert!(notice(&h).is_some_and(|t| t.contains("There is already a web address called “bad”")));
    forget_settings(&file);
}

#[test]
fn a_web_address_is_copied_with_an_id_of_its_own_and_no_sample() {
    let (mut h, file) = start_with_feeds("url-duplicate");
    let dir = folder_of(&file);
    let at = row_at(&h, "feed");
    h.right_click(at.x, at.y);
    press(&mut h, "Duplicate");
    assert_eq!(read(&dir, "Feeds/feed (2).url"), shortcut_text(ADDRESS));
    assert!(!dir.join("Feeds/feed (2).sample.json").exists());
    assert_eq!(notice(&h).as_deref(), Some("Copied as “feed (2)”"));
    let index = index_of(&dir);
    assert_ne!(
        index["items"]["Feeds/feed (2).url"]["id"],
        index["items"]["Feeds/feed.url"]["id"]
    );
    assert!(is_drawn(&h, "feed (2)"));
    forget_settings(&file);
}

#[test]
fn a_web_address_is_deleted_after_asking_and_takes_no_sample_with_it() {
    let (mut h, file) = start_with_feeds("url-delete");
    let dir = folder_of(&file);
    let at = row_at(&h, "feed");
    h.right_click(at.x, at.y);
    press(&mut h, "Delete…");
    assert!(is_drawn_containing(&h, "Delete “feed”?"), "{:?}", h.texts());
    assert!(
        !is_drawn_containing(&h, "sample goes with it"),
        "it has none: {:?}",
        h.texts()
    );
    assert!(dir.join("Feeds/feed.url").exists(), "not yet");
    press_last(&mut h, "Delete");
    assert!(!dir.join("Feeds/feed.url").exists());
    assert_eq!(read(&dir, "Feeds/feed.sample.json"), "[1]");
    assert!(!index_of(&dir)["items"]
        .as_object()
        .unwrap()
        .contains_key("Feeds/feed.url"));
    assert_eq!(notice(&h).as_deref(), Some("Deleted 1 item"));
    forget_settings(&file);
}

#[test]
fn the_text_copied_from_a_web_address_is_the_address() {
    let (mut h, file) = start_with_feeds("url-copy");
    let at = row_at(&h, "feed");
    h.right_click(at.x, at.y);
    press(&mut h, "Copy text");
    assert_eq!(h.copied.last().map(String::as_str), Some(ADDRESS));
    // A file with none to copy says so, and copies nothing.
    let copies = h.copied.len();
    let at = row_at(&h, "bad");
    h.right_click(at.x, at.y);
    press(&mut h, "Copy text");
    assert_eq!(h.copied.len(), copies);
    assert_eq!(notice(&h).as_deref(), Some("There is no web address in it"));
    forget_settings(&file);
}

#[test]
fn a_web_address_dragged_onto_a_folder_goes_into_it_alone_and_is_the_same_item() {
    let (mut h, file) = start_with_feeds("url-drag");
    let dir = folder_of(&file);
    let before = index_of(&dir)["items"]["Feeds/feed.url"].clone();
    let (from, to) = (row_at(&h, "feed"), row_at(&h, "Services"));
    drag(&mut h, from, to);
    assert_eq!(read(&dir, "Services/feed.url"), shortcut_text(ADDRESS));
    assert!(!dir.join("Feeds/feed.url").exists());
    assert_eq!(
        read(&dir, "Feeds/feed.sample.json"),
        "[1]",
        "the file called for it stays"
    );
    assert!(!dir.join("Services/feed.sample.json").exists());
    let after = &index_of(&dir)["items"]["Services/feed.url"];
    assert_eq!(
        (&after["id"], &after["sha256"]),
        (&before["id"], &before["sha256"])
    );
    assert_eq!(notice(&h).as_deref(), Some("Moved “feed” into “Services”"));
    assert_eq!(
        h.app.workspace_pane.selected_path(),
        Some("Services/feed.url")
    );
    forget_settings(&file);
}

#[test]
fn a_web_address_carried_over_the_query_pane_is_not_lit_there_and_is_not_opened() {
    let (mut h, file) = start_with_feeds("url-carry");
    let commands = keep_commands(&mut h);
    let query = h.query_box().center();
    for name in ["feed", "bad"] {
        let from = row_at(&h, name);
        pick_up(&mut h, from, query);
        assert!(lit(&h).is_empty(), "{name}: {:?}", lit(&h));
        assert!(
            !is_drawn_containing(&h, "Open “"),
            "{name}: {:?}",
            h.texts()
        );
        let_go(&mut h, query);
        assert_eq!(h.app.query_text, "", "{name}");
    }
    assert!(asked_for(&commands).is_empty());
    assert!(h.app.workspace_pane.take_actions().is_empty());
    assert!(folder_of(&file).join("Feeds/feed.url").is_file());
    forget_settings(&file);
}

// ---- export and import --------------------------------------------------------------------------------

#[test]
fn the_export_counts_the_web_addresses_and_not_a_sample_that_is_called_for_one() {
    let (mut h, file) = start_with_feeds("url-export");
    let to = file
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("exported feeds");
    click_button(&mut h, "menu");
    press(&mut h, "Export the workspace…");
    assert!(
        is_drawn_containing(&h, "Everything: 7 items in 4 folders"),
        "{:?}",
        h.texts()
    );
    // The two samples there are (the query's, the schema's); the file beside the address is nobody's.
    assert!(
        is_drawn_containing(&h, "include the samples (2 files"),
        "{:?}",
        h.texts()
    );
    press_one_of(
        &mut h,
        &["a new folder, or an empty one", "jsonquery-workspace-*"],
    );
    type_over(&mut h, &to.display().to_string());
    press_last(&mut h, "Export");
    assert_eq!(read(&to, "Feeds/feed.url"), shortcut_text(ADDRESS));
    assert!(!to.join("Feeds/feed.sample.json").exists());
    let exported = index_of(&to);
    assert_eq!(
        exported["items"]["Feeds/feed.url"]["id"],
        index_of(&folder_of(&file))["items"]["Feeds/feed.url"]["id"],
        "with the id it had"
    );
    assert!(notice(&h).is_some_and(|t| t.starts_with("Exported 7 items and 2 samples")));
    let _ = std::fs::remove_dir_all(&to);
    forget_settings(&file);
}

#[test]
fn a_folder_with_web_addresses_in_it_is_told_and_imported_with_them() {
    let (mut h, file) = start_with_feeds("url-import");
    let dir = folder_of(&file);
    let from = file.parent().unwrap().parent().unwrap().join("team feeds");
    put(
        &from,
        "weekly.url",
        &shortcut_text("https://example.com/weekly.json"),
    );
    put(&from, "Reports/q.jq", ".q");
    click_button(&mut h, "menu");
    press(&mut h, "Import a workspace…");
    press(&mut h, "a folder of queries, schemas…");
    h.type_text(&from.display().to_string());
    h.key(egui::Key::Tab, egui::Modifiers::NONE);
    h.settle();
    assert!(
        is_drawn_containing(&h, "2 items in 1 folder: 1 query, 1 web address"),
        "{:?}",
        h.texts()
    );
    press_last(&mut h, "Import");
    assert_eq!(
        read(&dir, "team feeds/weekly.url"),
        shortcut_text("https://example.com/weekly.json")
    );
    assert!(notice(&h).is_some_and(|t| t.starts_with("Imported 2 items")));
    assert!(index_of(&dir)["items"]["team feeds/weekly.url"]["id"].is_string());
    let _ = std::fs::remove_dir_all(&from);
    forget_settings(&file);
}
