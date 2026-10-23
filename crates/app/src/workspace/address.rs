//! The web address of a `.url` item: what an address is, where a file keeps it, what
//! is written, and the name it is saved under to begin with.
//!
//! A `.url` file is a Windows "Internet Shortcut", which other programs can read and
//! write too:
//!
//! ```text
//! [InternetShortcut]
//! URL=https://example.com/data.json
//! ```
//!
//! This app writes exactly that, with `\r\n` after each line, in UTF-8 and without a
//! byte-order mark ([`shortcut_text`]). It reads leniently ([`address_in`]): the
//! address is the value of the first line whose key is `URL`, whatever the case of
//! the key, in any section or none; a file that has no such line and is a single line
//! is that address; a byte-order mark and either kind of line end are accepted.
//!
//! What may be an address is one rule ([`web_address`]), used wherever one is read,
//! typed or about to be loaded.

use super::names::{cleaned_name, ends_with_ignoring_case};

/// The most characters an address may have.
const MAX_CHARS: usize = 4096;

/// The address that `raw` is, trimmed, if it is one: 1 to 4096 characters, none of
/// them a space or a control character, `http` or `https` (whatever the case) and
/// `://`, and then a host, which is at least one character that is not `/`, `?` or
/// `#`. Anything else (`ftp://…`, `file://…`, `javascript:…`, `https://`, `example.com`)
/// is not a web address.
pub fn web_address(raw: &str) -> Option<&str> {
    let address = raw.trim();
    if address.is_empty()
        || address.chars().count() > MAX_CHARS
        || address.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return None;
    }
    let host_and_after = ["http://", "https://"].iter().find_map(|scheme| {
        let start = address.get(..scheme.len())?;
        start
            .eq_ignore_ascii_case(scheme)
            .then(|| &address[scheme.len()..])
    })?;
    host_and_after
        .chars()
        .next()
        .is_some_and(|c| !matches!(c, '/' | '?' | '#'))
        .then_some(address)
}

/// The web address that the text of a `.url` file holds, if it holds one: the value of
/// the first line whose key is `URL` (the key in any case, the key and the value
/// trimmed, sections of the file not looked at); or, when there is no such line and
/// the text is a single line, that line. A byte-order mark at the start is ignored.
/// The first `URL` line is the one: if what it holds is not an address, the file has
/// none.
pub fn address_in(text: &str) -> Option<&str> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let keyed = text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        key.trim().eq_ignore_ascii_case("url").then_some(value)
    });
    // (A file of several lines is not an address: it has a line end inside it.)
    web_address(keyed.unwrap_or_else(|| text.trim()))
}

/// What a `.url` file is made of for `address`.
pub fn shortcut_text(address: &str) -> String {
    format!("[InternetShortcut]\r\nURL={address}\r\n")
}

/// The name an address is saved under to begin with: the last part of its path that
/// is not empty, without a `.json`, `.ndjson` or `.jsonl` at its end, or, when it has
/// none, its host (without who it is for, nor the port); what a name may not have is
/// replaced by `-` ([`cleaned_name`]).
pub fn default_name(address: &str) -> String {
    let rest = address.trim();
    let rest = rest.split_once("://").map_or(rest, |(_, rest)| rest);
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    // (the path ends where the query or the fragment begins)
    let path = tail.split(['?', '#']).next().unwrap_or_default();
    let last = path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or_default();
    let name = cleaned_name(without_document_end(last));
    if name.is_empty() {
        cleaned_name(host_of(authority))
    } else {
        name
    }
}

/// `part` without a `.json`, `.ndjson` or `.jsonl` at its end, whatever the case.
fn without_document_end(part: &str) -> &str {
    [".json", ".ndjson", ".jsonl"]
        .iter()
        .find(|end| ends_with_ignoring_case(part, end))
        .map_or(part, |end| &part[..part.len() - end.len()])
}

/// The host in `authority` (`user:secret@example.com:8080`): not who it is for, which
/// may be a password, and not the port.
fn host_of(authority: &str) -> &str {
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{check_item_name, ItemKind};

    #[test]
    fn an_http_or_https_address_with_a_host_is_one() {
        for address in [
            "http://example.com",
            "https://example.com",
            "https://example.com/",
            "https://example.com/data/orders.json",
            "https://example.com/a?b=c&d=e#f",
            "https://example.com?x=1",
            "https://example.com#top",
            "http://localhost:8080/data.json",
            "http://127.0.0.1:8000/a.json",
            "https://user:secret@example.com/data.json",
            "https://[::1]:8443/x",
            "https://münchen.example/ä.json",
            "https://example.com/a%20b.json",
            // The scheme is read whatever its case, and kept as it was written.
            "HTTPS://Example.com/A.json",
            "Http://example.com",
            // Only a host is wanted of the host: anything that is not `/`, `?` or `#`.
            "https://:80/x",
            "https://@/x",
            "https://x",
        ] {
            assert_eq!(web_address(address), Some(address), "{address}");
        }
    }

    #[test]
    fn what_is_not_an_http_or_https_address_with_a_host_is_not_one() {
        for address in [
            "",
            "example.com",
            "example.com/data.json",
            "//example.com/data.json",
            "www.example.com",
            "ftp://example.com/data.json",
            "file:///etc/passwd",
            "file://host/share/a.json",
            "javascript:alert(1)",
            "data:application/json,{}",
            "mailto:me@example.com",
            "ws://example.com",
            "httpss://example.com",
            "xhttp://example.com",
            "http//example.com",
            "http:/example.com",
            "http:example.com",
            "https:\\\\example.com",
            "https//example.com",
            "https:example.com",
            "ttps://example.com",
            // A scheme with no host after it.
            "http://",
            "https://",
            "https:///x",
            "https://?q=1",
            "https://#frag",
            "http:///",
        ] {
            assert_eq!(web_address(address), None, "{address:?}");
        }
    }

    #[test]
    fn an_address_is_trimmed_and_has_nothing_inside_it_that_is_a_space_or_a_control() {
        assert_eq!(
            web_address("  https://example.com/a.json \r\n"),
            Some("https://example.com/a.json")
        );
        assert_eq!(web_address("\t\u{a0}https://x\u{2003}"), Some("https://x"));
        for address in [
            "https://exa mple.com",
            "https://example.com/a b.json",
            "https://example.com/a\tb",
            "https://example.com/a\nb",
            "https://example.com/a\r\nb",
            "https://example.com/a\u{a0}b",
            "https://example.com/a\u{2003}b",
            "https://example.com/a\u{0}b",
            "https://example.com/a\u{7f}b",
            "https://example.com/a\u{85}b",
            "https://example.com/\u{1b}[0m",
            // (A space inside it, once it is trimmed.)
            " https://example.com/ https://example.com/",
        ] {
            assert_eq!(web_address(address), None, "{address:?}");
        }
        assert_eq!(web_address("   "), None);
        assert_eq!(web_address("\n"), None);
    }

    #[test]
    fn an_address_has_up_to_4096_characters_and_they_are_counted_as_characters() {
        let long = |n: usize, tail: char| {
            let mut address = "https://example.com/".to_owned();
            address.extend(std::iter::repeat_n(tail, n - address.chars().count()));
            address
        };
        assert!(web_address(&long(4096, 'a')).is_some());
        assert!(web_address(&long(4097, 'a')).is_none());
        // Two bytes to the letter: far more than 4096 bytes, and still 4096 characters.
        let wide = long(4096, 'é');
        assert!(wide.len() > 4096 && wide.chars().count() == 4096);
        assert!(web_address(&wide).is_some());
        assert!(web_address(&long(4097, 'é')).is_none());
        // The room that is left by the white space round it counts for nothing.
        assert!(web_address(&format!("  {}  ", long(4096, 'a'))).is_some());
    }

    #[test]
    fn the_address_in_a_file_is_the_value_of_its_url_line() {
        let written = shortcut_text("https://example.com/data.json");
        assert_eq!(
            address_in(&written),
            Some("https://example.com/data.json"),
            "what is written is what is read"
        );
        // Line ends of either kind, the file's last line with or without one.
        for text in [
            "[InternetShortcut]\nURL=https://example.com/a.json\n",
            "[InternetShortcut]\r\nURL=https://example.com/a.json\r\n",
            "[InternetShortcut]\nURL=https://example.com/a.json",
            "[InternetShortcut]\r\nURL=https://example.com/a.json",
            "URL=https://example.com/a.json",
            "\nURL=https://example.com/a.json\n\n",
        ] {
            assert_eq!(
                address_in(text),
                Some("https://example.com/a.json"),
                "{text:?}"
            );
        }
        // A byte-order mark.
        assert_eq!(
            address_in("\u{feff}[InternetShortcut]\r\nURL=https://example.com/a.json\r\n"),
            Some("https://example.com/a.json")
        );
        assert_eq!(
            address_in("\u{feff}https://example.com/a.json"),
            Some("https://example.com/a.json")
        );
    }

    #[test]
    fn the_key_is_read_whatever_its_case_and_with_spaces_round_it_and_the_value() {
        for text in [
            "url=https://example.com/a",
            "Url=https://example.com/a",
            "URL = https://example.com/a",
            "  url\t=\thttps://example.com/a  ",
            "[InternetShortcut]\r\n  URL  =   https://example.com/a   \r\n",
        ] {
            assert_eq!(address_in(text), Some("https://example.com/a"), "{text:?}");
        }
        // The key is `URL` and nothing longer or shorter, and the value starts at the first `=`.
        assert_eq!(
            address_in("BASEURL=https://example.com/a\nIconIndex=0"),
            None
        );
        assert_eq!(address_in("URLS=https://example.com/a\nx=y"), None);
        assert_eq!(address_in("MYURL=https://example.com/a\nx=y"), None);
        assert_eq!(
            address_in("URL=https://example.com/a?b=c&d=e"),
            Some("https://example.com/a?b=c&d=e"),
            "an = in the value is the value's"
        );
    }

    #[test]
    fn the_sections_of_a_file_and_its_other_keys_are_not_looked_at() {
        let text = "[Other]\r\nName=thing\r\n[InternetShortcut]\r\nIconFile=x.ico\r\n\
                    IconIndex=0\r\nURL=https://example.com/a.json\r\n[Last]\r\nURL=https://other.example\r\n";
        assert_eq!(address_in(text), Some("https://example.com/a.json"));
        // In a section of another name, or in none at all.
        assert_eq!(
            address_in("[Whatever]\nURL=https://example.com/a"),
            Some("https://example.com/a")
        );
        assert_eq!(
            address_in("; a comment\n# another\n\nURL=https://example.com/a"),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn the_first_url_line_is_the_one_even_when_what_it_holds_is_not_an_address() {
        assert_eq!(
            address_in("URL=https://example.com/first\nURL=https://example.com/second"),
            Some("https://example.com/first")
        );
        assert_eq!(
            address_in("URL=ftp://example.com/first\nURL=https://example.com/second"),
            None
        );
        assert_eq!(
            address_in("URL=\nURL=https://example.com/second"),
            None,
            "an empty value is not an address either"
        );
    }

    #[test]
    fn a_file_with_no_url_line_that_is_a_single_line_is_that_address() {
        for text in [
            "https://example.com/a.json",
            "https://example.com/a.json\n",
            "https://example.com/a.json\r\n",
            "\r\n  https://example.com/a.json  \r\n\r\n",
            "\u{feff}https://example.com/a.json\r\n",
        ] {
            assert_eq!(
                address_in(text),
                Some("https://example.com/a.json"),
                "{text:?}"
            );
        }
        // It has an = in it and is still the address: the key is not `URL`.
        assert_eq!(
            address_in("https://example.com/a?URL=1"),
            Some("https://example.com/a?URL=1")
        );
    }

    #[test]
    fn a_file_with_no_address_in_it_has_none() {
        for text in [
            "",
            "\n",
            "   ",
            "\u{feff}",
            "[InternetShortcut]",
            "[InternetShortcut]\r\nIconIndex=0\r\n",
            "[InternetShortcut]\r\nURL=\r\n",
            "[InternetShortcut]\r\nURL=example.com\r\n",
            "[InternetShortcut]\r\nURL=ftp://example.com/a\r\n",
            "[InternetShortcut]\r\nURL=https://\r\n",
            "[InternetShortcut]\r\nURL=https://exa mple.com/a\r\n",
            // Several lines, none of them the key: the file is not an address on its own.
            "https://example.com/a\nhttps://example.com/b",
            "https://example.com/a\r\nand a word",
            "example.com",
            "ftp://example.com/a",
            "not a web address at all",
        ] {
            assert_eq!(address_in(text), None, "{text:?}");
        }
    }

    #[test]
    fn what_is_written_is_a_windows_internet_shortcut_in_utf8_with_no_mark() {
        let text = shortcut_text("https://example.com/data.json?a=1");
        assert_eq!(
            text, "[InternetShortcut]\r\nURL=https://example.com/data.json?a=1\r\n",
            "exactly the lines, each with \\r\\n after it"
        );
        assert!(!text.starts_with('\u{feff}'));
        assert_eq!(text.as_bytes()[0], b'[');
        // Whatever the address is, it comes back as it went in.
        for address in [
            "https://example.com/a.json",
            "http://localhost:8080/x?y=z&w=%20#frag",
            "https://münchen.example/ä=ö",
            "HTTPS://Example.com/=",
        ] {
            assert_eq!(
                address_in(&shortcut_text(address)),
                Some(address),
                "{address}"
            );
        }
    }

    #[test]
    fn the_name_to_begin_with_is_the_last_part_of_the_path_without_a_document_end() {
        for (address, name) in [
            ("https://example.com/data/orders.json", "orders"),
            ("https://example.com/orders.ndjson", "orders"),
            ("https://example.com/orders.jsonl", "orders"),
            ("https://example.com/orders.JSON", "orders"),
            ("https://example.com/orders.NdJson", "orders"),
            (
                "https://example.com/a/b/c/orders.json?limit=10&x=y#top",
                "orders",
            ),
            ("https://example.com/api/v1/orders", "orders"),
            ("https://example.com/api/v1/orders/", "orders"),
            ("https://example.com/api/v1//orders//", "orders"),
            ("https://example.com/orders.v2.json", "orders.v2"),
            // Only one end is taken, and only at the end.
            ("https://example.com/data.json.gz", "data.json.gz"),
            ("https://example.com/data.json.json", "data.json"),
            ("https://example.com/my.jsonfile", "my.jsonfile"),
            // Not decoded.
            ("https://example.com/my%20orders.json", "my%20orders"),
            ("http://example.com/Ünï.json", "Ünï"),
            ("http://example.com/orders?file=other.json", "orders"),
            ("http://example.com/orders#other.json", "orders"),
        ] {
            assert_eq!(default_name(address), name, "{address}");
        }
    }

    #[test]
    fn with_no_path_the_name_to_begin_with_is_the_host_and_not_who_it_is_for_nor_the_port() {
        for (address, name) in [
            ("https://example.com", "example.com"),
            ("https://example.com/", "example.com"),
            ("https://example.com?x=1", "example.com"),
            ("https://example.com#x", "example.com"),
            ("http://localhost:8080/", "localhost"),
            ("http://127.0.0.1:8000", "127.0.0.1"),
            ("https://user:secret@example.com:8443/", "example.com"),
            ("https://user@example.com", "example.com"),
            ("https://a@b@example.com/", "example.com"),
            ("https://example.com:/", "example.com"),
            // A path with nothing in it that can be a name.
            ("https://example.com/.json", "example.com"),
            ("https://example.com/a/..", "example.com"),
            ("https://example.com/..", "example.com"),
            ("https://example.com/./", "example.com"),
            ("https://[::1]:8080/", "[--1]"),
            ("https://[::1]/", "[--1]"),
            // Without a scheme, as far as it can be made out while it is being typed.
            ("example.com/data.json", "data"),
            ("example.com", "example.com"),
            ("", ""),
            ("https://", ""),
            ("https:///", ""),
        ] {
            assert_eq!(default_name(address), name, "{address:?}");
        }
        assert!(
            !default_name("https://user:secret@example.com/").contains("secret"),
            "a password is not a name"
        );
    }

    #[test]
    fn what_a_name_may_not_have_is_replaced_in_the_name_to_begin_with() {
        for (address, name) in [
            ("https://example.com/a:b.json", "a-b"),
            ("https://example.com/a*b?c.json", "a-b"),
            ("https://example.com/a\"b<c>d|e.json", "a-b-c-d-e"),
            ("https://example.com/a\\b.json", "a-b"),
            ("https://example.com/.hidden.json", "hidden"),
            ("https://example.com/ends.", "ends"),
            ("https://example.com/..dots..json", "dots"),
            ("https://example.com/%2F.json", "%2F"),
        ] {
            assert_eq!(default_name(address), name, "{address}");
        }
        // Every character there is: what comes out is a name the rules take (or nothing),
        // whatever the address was.
        for code in 0_u32..0x250 {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            for address in [
                format!("https://example.com/a{c}b.json"),
                format!("https://example.com/{c}"),
                format!("https://ex{c}ample.com"),
                format!("https://example.com/x/{c}{c}.json"),
            ] {
                let name = default_name(&address);
                if name.is_empty() {
                    continue;
                }
                assert!(
                    check_item_name(&name, ItemKind::Url).is_ok(),
                    "{address:?} gave {name:?}"
                );
            }
        }
    }
}
