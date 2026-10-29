//! The kinds of text a string is recognised as — a date, a uuid, an address, a
//! number written as text… Each is a regular expression, which is what the
//! schema says as the string's `pattern`, and, where JSON Schema has a name for
//! it, a `format`.
//!
//! The expressions are written in the part of the syntax that ECMA-262 (what a
//! `pattern` of JSON Schema is) and the `regex` crate (what finds them here) read
//! the same way: `[0-9]` and not `\d` (which is any digit of Unicode to the
//! crate), no look-around, no backslash before a character that needs none. A
//! string is of a kind when the expression matches all of it and, for a few, a
//! check the expression can't make holds too (that the 30th of February is no
//! date). The tests hold each expression to the samples it is for, in the crate
//! and in the validator of the Validate page.

use std::sync::LazyLock;

use regex::Regex;

use super::text::Facts;
use super::Draft;

/// The kinds, in the order they are tried: when the strings at a place are of
/// more than one, the first is what they are said to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    DateTime,
    LocalDateTime,
    Date,
    Time,
    LocalTime,
    Duration,
    Uuid,
    Email,
    Url,
    Ipv4,
    Ipv6,
    Hostname,
    Mac,
    Semver,
    Phone,
    HexColour,
    Jwt,
    Integer,
    Decimal,
    NumberText,
    BooleanText,
    LanguageTag,
    Code,
    Slug,
    Hex,
}

/// The digits of a hexadecimal number, of either case.
const HEX: &str = "[0-9a-fA-F]";

/// An octet of an IPv4 address, 0 to 255 without a leading zero.
const OCTET: &str = "(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])";

/// The month and the day of a date: 01–12 and 01–31 (whether the day is in the
/// month is the check's to say).
const MONTH_DAY: &str = "-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12][0-9]|3[01])";

/// Hours and minutes: 00:00 to 23:59.
const HOUR_MINUTE: &str = "(?:[01][0-9]|2[0-3]):[0-5][0-9]";

/// Top-level domains a host name is taken for one by: a word with a dot in it is
/// a file name ("photo.jpg") as often as a host, and a short list of the domains
/// that are common keeps the two apart. (The pattern written takes any.)
const COMMON_TLDS: &[&str] = &[
    "ai", "app", "ar", "at", "au", "be", "biz", "br", "ca", "ch", "cloud", "cn", "co", "com", "cz",
    "de", "dev", "dk", "edu", "es", "eu", "fi", "fr", "gov", "gr", "hk", "hu", "ie", "il", "in",
    "info", "int", "internal", "io", "it", "jp", "kr", "local", "me", "mil", "mx", "net", "nl",
    "no", "nz", "online", "org", "pl", "pt", "ro", "ru", "se", "sg", "site", "tech", "test", "tr",
    "tw", "ua", "uk", "us", "xyz", "za",
];

impl Kind {
    /// Every kind, in the order they are tried — which is also the order of the
    /// variants, so that a kind's number is its place here.
    pub const ALL: [Kind; 25] = [
        Kind::DateTime,
        Kind::LocalDateTime,
        Kind::Date,
        Kind::Time,
        Kind::LocalTime,
        Kind::Duration,
        Kind::Uuid,
        Kind::Email,
        Kind::Url,
        Kind::Ipv4,
        Kind::Ipv6,
        Kind::Hostname,
        Kind::Mac,
        Kind::Semver,
        Kind::Phone,
        Kind::HexColour,
        Kind::Jwt,
        Kind::Integer,
        Kind::Decimal,
        Kind::NumberText,
        Kind::BooleanText,
        Kind::LanguageTag,
        Kind::Code,
        Kind::Slug,
        Kind::Hex,
    ];

    /// The set of every kind, one bit each.
    pub const EVERY: u32 = (1 << Self::ALL.len()) - 1;

    pub fn bit(self) -> u32 {
        1 << self as u32
    }

    /// What the list of patterns found calls it.
    pub fn name(self) -> &'static str {
        match self {
            Kind::DateTime => "date-time",
            Kind::LocalDateTime => "date-time without zone",
            Kind::Date => "date",
            Kind::Time => "time",
            Kind::LocalTime => "time without zone",
            Kind::Duration => "duration",
            Kind::Uuid => "uuid",
            Kind::Email => "email",
            Kind::Url => "URL",
            Kind::Ipv4 => "IPv4 address",
            Kind::Ipv6 => "IPv6 address",
            Kind::Hostname => "host name",
            Kind::Mac => "MAC address",
            Kind::Semver => "version (semver)",
            Kind::Phone => "phone number",
            Kind::HexColour => "hex colour",
            Kind::Jwt => "JWT",
            Kind::Integer => "integer as text",
            Kind::Decimal => "decimal as text",
            Kind::NumberText => "number as text",
            Kind::BooleanText => "true or false as text",
            Kind::LanguageTag => "language tag",
            Kind::Code => "code",
            Kind::Slug => "slug",
            Kind::Hex => "hex string",
        }
    }

    /// The `format` JSON Schema has for it, and the first draft that has it.
    pub fn format(self) -> Option<(&'static str, Draft)> {
        match self {
            Kind::DateTime => Some(("date-time", Draft::D4)),
            Kind::Date => Some(("date", Draft::D7)),
            Kind::Time => Some(("time", Draft::D7)),
            Kind::Duration => Some(("duration", Draft::D201909)),
            Kind::Uuid => Some(("uuid", Draft::D201909)),
            Kind::Email => Some(("email", Draft::D4)),
            Kind::Url => Some(("uri", Draft::D4)),
            Kind::Ipv4 => Some(("ipv4", Draft::D4)),
            Kind::Ipv6 => Some(("ipv6", Draft::D4)),
            Kind::Hostname => Some(("hostname", Draft::D4)),
            _ => None,
        }
    }

    /// The expression, without the `^` and `$` around it; an alternative at its
    /// top level is in a group, so that it can be put in a longer one as it is.
    fn source(self) -> String {
        let ipv6 = {
            let h = HEX;
            let v4 = format!("(?:{OCTET}\\.){{3}}{OCTET}");
            [
                format!("(?:{h}{{1,4}}:){{7}}{h}{{1,4}}"),
                format!("(?:{h}{{1,4}}:){{1,7}}:"),
                format!("(?:{h}{{1,4}}:){{1,6}}:{h}{{1,4}}"),
                format!("(?:{h}{{1,4}}:){{1,5}}(?::{h}{{1,4}}){{1,2}}"),
                format!("(?:{h}{{1,4}}:){{1,4}}(?::{h}{{1,4}}){{1,3}}"),
                format!("(?:{h}{{1,4}}:){{1,3}}(?::{h}{{1,4}}){{1,4}}"),
                format!("(?:{h}{{1,4}}:){{1,2}}(?::{h}{{1,4}}){{1,5}}"),
                format!("{h}{{1,4}}:(?::{h}{{1,4}}){{1,6}}"),
                format!(":(?:(?::{h}{{1,4}}){{1,7}}|:)"),
                format!("(?:{h}{{1,4}}:){{6}}{v4}"),
                format!("::(?:ffff(?::0{{1,4}})?:)?{v4}"),
                format!("(?:{h}{{1,4}}:){{1,4}}:{v4}"),
            ]
            .join("|")
        };
        let label = "[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?";
        match self {
            Kind::DateTime => format!(
                "[0-9]{{4}}{MONTH_DAY}T{HOUR_MINUTE}:[0-5][0-9](?:\\.[0-9]+)?(?:Z|[+-]{HOUR_MINUTE})"
            ),
            Kind::LocalDateTime => {
                format!("[0-9]{{4}}{MONTH_DAY}[T ]{HOUR_MINUTE}(?::[0-5][0-9](?:\\.[0-9]+)?)?")
            }
            Kind::Date => format!("[0-9]{{4}}{MONTH_DAY}"),
            Kind::Time => {
                format!("{HOUR_MINUTE}:[0-5][0-9](?:\\.[0-9]+)?(?:Z|[+-]{HOUR_MINUTE})")
            }
            Kind::LocalTime => format!("{HOUR_MINUTE}(?::[0-5][0-9](?:\\.[0-9]+)?)?"),
            Kind::Duration => "P(?:[0-9]+Y)?(?:[0-9]+M)?(?:[0-9]+W)?(?:[0-9]+D)?\
                               (?:T(?:[0-9]+H)?(?:[0-9]+M)?(?:[0-9]+(?:\\.[0-9]+)?S)?)?"
                .to_owned(),
            Kind::Uuid => format!("{HEX}{{8}}-{HEX}{{4}}-{HEX}{{4}}-{HEX}{{4}}-{HEX}{{12}}"),
            Kind::Email => "[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\\.[A-Za-z0-9-]+)*\\.[A-Za-z]{2,}"
                .to_owned(),
            Kind::Url => "(?:https?|ftps?|wss?)://[^\\s/?#]+(?:[/?#][^\\s]*)?".to_owned(),
            Kind::Ipv4 => format!("(?:{OCTET}\\.){{3}}{OCTET}"),
            Kind::Ipv6 => format!("(?:{ipv6})"),
            Kind::Hostname => format!("{label}(?:\\.{label})*\\.[A-Za-z]{{2,63}}"),
            Kind::Mac => format!("{HEX}{{2}}(?:[:-]{HEX}{{2}}){{5}}"),
            Kind::Semver => "(?:0|[1-9][0-9]*)\\.(?:0|[1-9][0-9]*)\\.(?:0|[1-9][0-9]*)\
                             (?:-[0-9A-Za-z.-]+)?(?:\\+[0-9A-Za-z.-]+)?"
                .to_owned(),
            Kind::Phone => "\\+[0-9](?:[ .()-]{0,2}[0-9]){6,14}".to_owned(),
            Kind::HexColour => format!("#(?:{HEX}{{3}}|{HEX}{{4}}|{HEX}{{6}}|{HEX}{{8}})"),
            Kind::Jwt => "eyJ[A-Za-z0-9_-]*\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]*".to_owned(),
            Kind::Integer => "-?[0-9]+".to_owned(),
            Kind::Decimal => "-?[0-9]+\\.[0-9]+".to_owned(),
            Kind::NumberText => "-?[0-9]+(?:\\.[0-9]+)?".to_owned(),
            Kind::BooleanText => "(?:true|false)".to_owned(),
            Kind::LanguageTag => "[a-z]{2,3}[-_][A-Z]{2}".to_owned(),
            Kind::Code => "[A-Z]{2,3}".to_owned(),
            Kind::Slug => "[a-z0-9]+(?:-[a-z0-9]+)+".to_owned(),
            Kind::Hex => format!("{HEX}+"),
        }
    }

    /// Whether `text`, all of it, is of this kind.
    pub fn matches(self, text: &str) -> bool {
        EXPRESSIONS[self as usize].is_match(text) && self.holds(text)
    }

    /// What the expression can't say.
    fn holds(self, text: &str) -> bool {
        match self {
            Kind::DateTime | Kind::LocalDateTime | Kind::Date => is_a_day(text),
            // "P" and "PT" say no time at all.
            Kind::Duration => text.len() > 1 && !text.ends_with('T'),
            Kind::Ipv6 => text.parse::<std::net::Ipv6Addr>().is_ok(),
            Kind::Hostname => {
                let tld = text.rsplit('.').next().unwrap_or("");
                text.len() <= 253 && COMMON_TLDS.contains(&tld.to_ascii_lowercase().as_str())
            }
            _ => true,
        }
    }

    /// Whether the strings at a place, as `facts` sums them up, are this kind
    /// as a whole — for the kinds that every string can be one of and the lot
    /// still not: three letters in capitals are a code when they are all as long,
    /// and digits and the letters a to f are a hex string when they are long and
    /// some of them are letters.
    pub fn fits(self, facts: &Facts) -> bool {
        match self {
            Kind::Code => facts.min_chars == facts.max_chars && facts.distinct >= 2,
            Kind::Hex => facts.min_chars >= 8 && (facts.lower || facts.upper),
            _ => true,
        }
    }

    /// The expression for strings that `facts` sums up, without `^` and `$`:
    /// the hex digits in the case the strings have them in, the sign of a number
    /// only if one had one, and a length that every string has when that says
    /// something (see [`Facts::count`]).
    pub fn pattern(self, facts: &Facts, bounds: bool) -> String {
        let sign = if facts.signed { "-?" } else { "" };
        match self {
            Kind::Uuid | Kind::Mac | Kind::HexColour => {
                self.source().replace(HEX, facts.hex_class())
            }
            Kind::Integer if facts.signed => "-?[0-9]+".to_owned(),
            Kind::Integer => format!(
                "[0-9]{}",
                facts.count(facts.min_chars, facts.max_chars, bounds)
            ),
            Kind::Decimal => {
                let digits = facts.count(facts.min_fraction, facts.max_fraction, bounds);
                format!("{sign}[0-9]+\\.[0-9]{digits}")
            }
            Kind::NumberText => format!("{sign}[0-9]+(?:\\.[0-9]+)?"),
            Kind::Code => format!("[A-Z]{{{}}}", facts.min_chars),
            Kind::Hex => format!(
                "{}{}",
                facts.hex_class(),
                facts.count(facts.min_chars, facts.max_chars, bounds)
            ),
            _ => self.source(),
        }
    }
}

/// The expressions of the kinds, anchored, in the order of [`Kind::ALL`].
static EXPRESSIONS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    Kind::ALL
        .iter()
        .map(|kind| {
            Regex::new(&format!("^{}$", kind.source()))
                .unwrap_or_else(|e| panic!("the expression of {kind:?}: {e}"))
        })
        .collect()
});

/// Whether the date a text starts with (`YYYY-MM-DD`, which the expression has
/// let through) is a day of the calendar: no 31st of April, a 29th of February
/// only in a leap year.
fn is_a_day(text: &str) -> bool {
    let number = |range: std::ops::Range<usize>| -> Option<u32> { text.get(range)?.parse().ok() };
    let (Some(year), Some(month), Some(day)) = (number(0..4), number(5..7), number(8..10)) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=days).contains(&day)
}

/// `text` as it is in an expression: the characters that mean something there
/// with a backslash before them, and no other (ECMA-262 refuses a backslash
/// before a character that needs none, in the mode a schema's pattern can be read
/// in).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The kinds `text` is of.
    fn kinds_of(text: &str) -> Vec<Kind> {
        Kind::ALL.into_iter().filter(|k| k.matches(text)).collect()
    }

    /// The first kind `text` is of.
    fn kind_of(text: &str) -> Option<Kind> {
        kinds_of(text).first().copied()
    }

    #[test]
    fn the_kinds_are_in_the_order_of_their_numbers() {
        for (i, kind) in Kind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, i, "{kind:?}");
        }
        assert_eq!(Kind::EVERY.count_ones() as usize, Kind::ALL.len());
    }

    #[test]
    fn every_expression_is_read_by_the_crate() {
        // A panic here names the kind.
        assert_eq!(EXPRESSIONS.len(), Kind::ALL.len());
    }

    /// Samples of each kind, and texts that look like one and are not.
    pub(crate) fn samples() -> Vec<(Kind, Vec<&'static str>, Vec<&'static str>)> {
        vec![
            (
                Kind::DateTime,
                vec![
                    "2024-01-15T10:30:00Z",
                    "2024-02-29T23:59:59.123+05:30",
                    "1999-12-31T00:00:00-08:00",
                ],
                vec![
                    "2024-01-15T10:30:00",
                    "2023-02-29T10:30:00Z",
                    "2024-13-01T10:30:00Z",
                    "2024-01-15T24:00:00Z",
                    "2024-01-15 10:30:00Z",
                ],
            ),
            (
                Kind::LocalDateTime,
                vec!["2024-01-15T10:30:00", "2024-01-15 10:30", "2024-04-30 08:00:01.5"],
                vec!["2024-04-31 10:30", "2024-01-15T10", "2024-01-15"],
            ),
            (
                Kind::Date,
                vec!["2024-01-15", "2000-02-29", "1970-12-31"],
                vec!["2024-1-15", "1900-02-29", "2024-06-31", "15/01/2024", "2024-00-10"],
            ),
            (
                Kind::Time,
                vec!["10:30:00Z", "23:59:59.5+01:00", "00:00:00-03:30"],
                vec!["10:30:00", "25:00:00Z", "10:60:00Z"],
            ),
            (
                Kind::LocalTime,
                vec!["10:30", "23:59:59", "07:05:00.250"],
                vec!["7:05", "24:00", "10:30:00Z"],
            ),
            (
                Kind::Duration,
                vec!["P1D", "PT30M", "P1Y2M3DT4H5M6S", "PT0.5S", "P2W"],
                vec!["P", "PT", "1D", "P1H"],
            ),
            (
                Kind::Uuid,
                vec![
                    "123e4567-e89b-12d3-a456-426614174000",
                    "C56A4180-65AA-42EC-A945-5FD21DEC0538",
                ],
                vec![
                    "123e4567e89b12d3a456426614174000",
                    "123e4567-e89b-12d3-a456-42661417400",
                    "g23e4567-e89b-12d3-a456-426614174000",
                ],
            ),
            (
                Kind::Email,
                vec!["ada@example.com", "first.last+tag@mail.co.uk", "x_y%z@a-b.io"],
                vec!["ada@example", "ada.example.com", "@example.com", "a b@example.com"],
            ),
            (
                Kind::Url,
                vec![
                    "https://example.com",
                    "http://localhost:8080/a/b?c=d#e",
                    "wss://example.com/socket",
                ],
                vec!["example.com", "https://", "https://exa mple.com", "mailto:a@b.c"],
            ),
            (
                Kind::Ipv4,
                vec!["192.168.0.1", "10.0.0.255", "0.0.0.0", "255.255.255.255"],
                vec!["256.1.1.1", "1.2.3", "01.2.3.4", "1.2.3.4.5"],
            ),
            (
                Kind::Ipv6,
                vec![
                    "::1",
                    "2001:db8::8a2e:370:7334",
                    "2001:0db8:85a3:0000:0000:8a2e:0370:7334",
                    "fe80::",
                    "::ffff:192.168.0.1",
                    "1:2:3:4:5:6:1.2.3.4",
                ],
                vec!["1:2:3", "2001:db8::8a2e::7334", "12345::1", ":::"],
            ),
            (
                Kind::Hostname,
                vec!["example.com", "api.eu.example.io", "my-host.internal"],
                vec!["photo.jpg", "readme", "-bad.com", "a_b.com"],
            ),
            (
                Kind::Mac,
                vec!["00:1A:2B:3C:4D:5E", "00-1a-2b-3c-4d-5e"],
                vec!["00:1A:2B:3C:4D", "001A2B3C4D5E", "00:1A:2B:3C:4D:5G"],
            ),
            (
                Kind::Semver,
                vec!["1.2.3", "0.10.0-beta.1", "2.0.0+build.5", "1.0.0-rc.1+sha.5114f85"],
                vec!["1.2", "01.2.3", "v1.2.3", "1.2.3.4"],
            ),
            (
                Kind::Phone,
                vec!["+14155552671", "+44 20 7946 0958", "+1 (415) 555-2671"],
                vec!["4155552671", "+1 415", "+1-415-555-2671-1234-5678-9"],
            ),
            (
                Kind::HexColour,
                vec!["#fff", "#A1B2C3", "#a1b2c3d4", "#abcd"],
                vec!["fff", "#ff", "#abcde", "#ggg"],
            ),
            (
                Kind::Jwt,
                vec![
                    "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
                ],
                vec!["abc.def.ghi", "eyJhbGciOiJIUzI1NiJ9"],
            ),
            (
                Kind::Integer,
                vec!["0", "42", "-17", "00123"],
                vec!["1.5", "1e3", "+1", "12a", ""],
            ),
            (
                Kind::Decimal,
                vec!["1.5", "-0.25", "12.50"],
                vec!["1", ".5", "1.", "1,5"],
            ),
            (
                Kind::NumberText,
                vec!["1", "1.5", "-3"],
                vec!["1.", "abc", "1e5"],
            ),
            (
                Kind::BooleanText,
                vec!["true", "false"],
                vec!["True", "yes", "1"],
            ),
            (
                Kind::LanguageTag,
                vec!["en-US", "fr-FR", "pt_BR", "ast-ES"],
                vec!["en", "EN-us", "english-US"],
            ),
            (Kind::Code, vec!["USD", "EUR"], vec!["usd", "EURO", "A"]),
            (
                Kind::Slug,
                vec!["my-first-post", "release-2-0"],
                vec!["My-Post", "post", "a--b", "-a"],
            ),
            (
                Kind::Hex,
                vec!["9f86d081884c7d659a2feaa0c55ad015", "DEADBEEF"],
                vec!["xyz", "12 34"],
            ),
        ]
    }

    #[test]
    fn each_kind_takes_its_samples_and_refuses_the_look_alikes() {
        for (kind, good, bad) in samples() {
            for text in good {
                assert!(kind.matches(text), "{kind:?} takes {text:?}");
            }
            for text in bad {
                assert!(!kind.matches(text), "{kind:?} refuses {text:?}");
            }
        }
    }

    #[test]
    fn a_text_is_first_of_the_kind_that_says_most_about_it() {
        assert_eq!(kind_of("2024-01-15"), Some(Kind::Date), "not a slug");
        assert_eq!(
            kind_of("123e4567-e89b-12d3-a456-426614174000"),
            Some(Kind::Uuid)
        );
        assert_eq!(
            kind_of("42"),
            Some(Kind::Integer),
            "before a number and a hex string"
        );
        assert_eq!(kind_of("1.5"), Some(Kind::Decimal));
        assert_eq!(kind_of("example.com"), Some(Kind::Hostname));
        assert_eq!(kind_of("Hello world"), None);
        assert_eq!(kind_of("Ada Lovelace"), None);
        // Three letters a to f are a hex string, taken alone: whether the strings of
        // a place are one is for them all to say (see `Kind::fits`).
        assert_eq!(kind_of("ada"), Some(Kind::Hex));
    }

    #[test]
    fn an_ipv6_address_is_one_the_standard_library_reads() {
        // Whatever both take is an address; a few forms the expression leaves out are
        // not taken, rather than taken wrongly.
        for text in ["1:2::3:1.2.3.4", "::ffff:1.2.3.4", "1::", "::"] {
            let parsed = text.parse::<std::net::Ipv6Addr>().is_ok();
            if Kind::Ipv6.matches(text) {
                assert!(parsed, "{text}");
            }
        }
        for n in 0u128..2000 {
            let address =
                std::net::Ipv6Addr::from(n.wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835));
            let text = address.to_string();
            assert!(Kind::Ipv6.matches(&text), "{text}");
        }
    }

    #[test]
    fn a_day_is_one_of_the_calendar() {
        assert!(is_a_day("2024-02-29"));
        assert!(!is_a_day("2023-02-29"));
        assert!(is_a_day("2000-02-29"));
        assert!(!is_a_day("1900-02-29"));
        assert!(!is_a_day("2024-04-31"));
        assert!(is_a_day("2024-12-31"));
        assert!(!is_a_day("2024-12-00"));
        assert!(!is_a_day("short"));
    }

    #[test]
    fn only_what_means_something_is_escaped() {
        assert_eq!(escape("a.b"), "a\\.b");
        assert_eq!(escape("ORD-"), "ORD-");
        assert_eq!(
            escape("(x)+[y]{z}|^$*?\\"),
            "\\(x\\)\\+\\[y\\]\\{z\\}\\|\\^\\$\\*\\?\\\\"
        );
        assert_eq!(escape("#/_:@ ,;~&%"), "#/_:@ ,;~&%");
    }
}
