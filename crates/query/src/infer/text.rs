//! What the strings seen at one place of a document have in common: the kinds
//! of text that every one of them is (`kinds.rs`), the shape they all have —
//! letters, digits and the same marks between them, as `ORD-2024-0017` and
//! `INV-2023-0402` have — and whether each is a list: items of one kind, again and
//! again, with a comma, a semicolon or a bar between them (`10.0.0.1, 10.0.0.2`).
//! From that comes the expression the schema says they match.
//!
//! It is worked out as the strings come, one at a time, and two places can be
//! put together ([`Strings::merge`]) — the values of a map are one place, for
//! one.

use super::kinds::{escape, Kind};
use super::Draft;

/// How many different strings are remembered: to make an `enum` of, and to know
/// whether the strings differ.
pub(super) const MAX_VALUES: usize = 10;

/// A string longer than this (in bytes) is counted, and not looked at.
const LONGEST: usize = 4096;

/// A shape is of strings no longer than this (in characters), and of this many
/// runs at most: what is longer is text, and not a code.
const LONGEST_SHAPE: usize = 64;
const MOST_RUNS: usize = 16;

/// What can be between the items of a list, in the order they are tried.
const SEPARATORS: [char; 3] = [',', ';', '|'];

/// Counts and flags the expressions are made from.
#[derive(Clone, Debug, Default)]
pub(super) struct Facts {
    /// The fewest and the most characters a string had.
    pub min_chars: usize,
    pub max_chars: usize,
    /// A string started with a minus.
    pub signed: bool,
    /// Letters a to z, and A to Z, were in the strings.
    pub lower: bool,
    pub upper: bool,
    /// The fewest and the most digits after the last full stop.
    pub min_fraction: usize,
    pub max_fraction: usize,
    /// How many different strings there were — [`MAX_VALUES`] and one for
    /// more than that.
    pub distinct: usize,
}

impl Facts {
    fn of(text: &str) -> Self {
        let chars = text.chars().count();
        let fraction = text.rsplit_once('.').map_or(0, |(_, after)| {
            after.bytes().take_while(u8::is_ascii_digit).count()
        });
        Self {
            min_chars: chars,
            max_chars: chars,
            signed: text.starts_with('-'),
            lower: text.bytes().any(|b| b.is_ascii_lowercase()),
            upper: text.bytes().any(|b| b.is_ascii_uppercase()),
            min_fraction: fraction,
            max_fraction: fraction,
            distinct: 0,
        }
    }

    fn merge(&mut self, other: &Facts) {
        self.min_chars = self.min_chars.min(other.min_chars);
        self.max_chars = self.max_chars.max(other.max_chars);
        self.signed |= other.signed;
        self.lower |= other.lower;
        self.upper |= other.upper;
        self.min_fraction = self.min_fraction.min(other.min_fraction);
        self.max_fraction = self.max_fraction.max(other.max_fraction);
    }

    /// The hex digits, in the case the strings have their letters in.
    pub fn hex_class(&self) -> &'static str {
        match (self.lower, self.upper) {
            (true, true) => "[0-9a-fA-F]",
            (false, true) => "[0-9A-F]",
            _ => "[0-9a-f]",
        }
    }

    /// How many times what goes before is there, for something that came between
    /// `min` and `max` times: a number when it is always the same and that says
    /// something — several different strings agree on it, and it is more than
    /// one (a lone digit of three strings may as well be two in the next) — and
    /// otherwise one or more. With `bounds`, the least and the most seen.
    pub fn count(&self, min: usize, max: usize, bounds: bool) -> String {
        if min == max && (bounds || (self.distinct >= 2 && min >= 2)) {
            return if min == 1 {
                String::new()
            } else {
                format!("{{{min}}}")
            };
        }
        if bounds {
            return format!("{{{min},{max}}}");
        }
        "+".to_owned()
    }
}

/// The different strings seen, while they are few.
#[derive(Clone, Debug)]
enum Values {
    /// Each with how many times it was seen, in the order they came.
    Few(Vec<(String, usize)>),
    Many,
}

impl Values {
    fn add(&mut self, text: &str, times: usize) {
        let Values::Few(values) = self else {
            return;
        };
        if let Some((_, n)) = values.iter_mut().find(|(v, _)| v == text) {
            *n += times;
        } else if values.len() < MAX_VALUES && text.len() <= LONGEST {
            values.push((text.to_owned(), times));
        } else {
            *self = Values::Many;
        }
    }

    fn merge(&mut self, other: &Values) {
        match other {
            Values::Many => *self = Values::Many,
            Values::Few(values) => {
                for (text, times) in values {
                    self.add(text, *times);
                }
            }
        }
    }

    fn distinct(&self) -> usize {
        match self {
            Values::Few(values) => values.len(),
            Values::Many => MAX_VALUES + 1,
        }
    }
}

/// What a character of a shape is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Digit,
    Upper,
    Lower,
    /// Upper case in one string, lower in another.
    Letter,
    /// This character itself: a dash, a slash, a full stop…
    Mark(char),
}

/// Characters of one class, one after the other: `ORD` of `ORD-17`.
#[derive(Clone, Debug)]
struct Run {
    class: Class,
    /// The fewest and the most there were.
    min: usize,
    max: usize,
    /// The letters, when they were the same in every string.
    text: Option<String>,
}

/// The runs every string is made of, in the same order.
#[derive(Clone, Debug, Default)]
enum Shape {
    #[default]
    Unseen,
    Of {
        runs: Vec<Run>,
        /// The first string, to show what it looks like.
        example: String,
    },
    /// The strings are not of one shape.
    Mixed,
}

impl Shape {
    fn of(text: &str) -> Shape {
        if text.is_empty() || text.len() > LONGEST_SHAPE * 4 {
            return Shape::Mixed;
        }
        let mut runs: Vec<Run> = Vec::new();
        for c in text.chars() {
            let class = match c {
                '0'..='9' => Class::Digit,
                'A'..='Z' => Class::Upper,
                'a'..='z' => Class::Lower,
                c if c.is_ascii_punctuation() => Class::Mark(c),
                // White space is text, and a character outside ASCII is a word.
                _ => return Shape::Mixed,
            };
            match runs.last_mut() {
                Some(run) if run.class == class => {
                    run.min += 1;
                    run.max += 1;
                    if let Some(text) = &mut run.text {
                        text.push(c);
                    }
                }
                _ => runs.push(Run {
                    class,
                    min: 1,
                    max: 1,
                    text: (class != Class::Digit).then(|| c.to_string()),
                }),
            }
        }
        if runs.len() > MOST_RUNS || text.chars().count() > LONGEST_SHAPE {
            return Shape::Mixed;
        }
        Shape::Of {
            runs,
            example: text.to_owned(),
        }
    }

    fn merge(&mut self, other: Shape) {
        let (mine, theirs) = match (&mut *self, other) {
            (_, Shape::Unseen) | (Shape::Mixed, _) => return,
            (Shape::Unseen, other) => {
                *self = other;
                return;
            }
            (_, Shape::Mixed) => {
                *self = Shape::Mixed;
                return;
            }
            (Shape::Of { runs, .. }, Shape::Of { runs: theirs, .. }) => (runs, theirs),
        };
        if mine.len() != theirs.len() {
            *self = Shape::Mixed;
            return;
        }
        for (run, other) in mine.iter_mut().zip(theirs) {
            run.class = match (run.class, other.class) {
                (a, b) if a == b => a,
                (
                    Class::Upper | Class::Lower | Class::Letter,
                    Class::Upper | Class::Lower | Class::Letter,
                ) => Class::Letter,
                _ => {
                    *self = Shape::Mixed;
                    return;
                }
            };
            run.min = run.min.min(other.min);
            run.max = run.max.max(other.max);
            if run.text != other.text {
                run.text = None;
            }
        }
    }

    /// The expression of the shape, without `^` and `$`, when it is one worth
    /// saying: one with digits in it and something else, as a code or a number
    /// of a document has (a word alone, or a name, is not one). Letters that
    /// every one of several different strings has are written as they are.
    fn pattern(&self, distinct: usize, bounds: bool) -> Option<String> {
        let Shape::Of { runs, .. } = self else {
            return None;
        };
        if runs.len() < 2 || !runs.iter().any(|r| r.class == Class::Digit) {
            return None;
        }
        let facts = Facts {
            distinct,
            ..Facts::default()
        };
        let mut out = String::new();
        for run in runs {
            let class = match run.class {
                Class::Mark(c) => {
                    // A mark is the same mark: its count is said whenever it is the same.
                    out.push_str(&escape(&c.to_string()));
                    if run.min != run.max || bounds {
                        out.push_str(&facts.count(run.min, run.max, bounds));
                    } else if run.min > 1 {
                        out.push_str(&format!("{{{}}}", run.min));
                    }
                    continue;
                }
                Class::Digit => "[0-9]",
                Class::Upper => "[A-Z]",
                Class::Lower => "[a-z]",
                Class::Letter => "[A-Za-z]",
            };
            match &run.text {
                Some(text) if distinct >= 2 => out.push_str(&escape(text)),
                _ => {
                    out.push_str(class);
                    // One letter in every one of several strings is one letter (where
                    // one digit may be two in the next).
                    let one_letter = run.class != Class::Digit && run.max == 1 && distinct >= 2;
                    if !one_letter {
                        out.push_str(&facts.count(run.min, run.max, bounds));
                    }
                }
            }
        }
        Some(out)
    }

    fn example(&self) -> &str {
        match self {
            Shape::Of { example, .. } => example,
            _ => "",
        }
    }
}

/// Which characters the items of a list are made of, when they are a word each:
/// letters, digits, `_`, `.` and `-`.
#[derive(Clone, Copy, Debug, Default)]
struct Word {
    broken: bool,
    upper: bool,
    lower: bool,
    digit: bool,
    underscore: bool,
    dot: bool,
    dash: bool,
}

impl Word {
    fn add(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                'A'..='Z' => self.upper = true,
                'a'..='z' => self.lower = true,
                '0'..='9' => self.digit = true,
                '_' => self.underscore = true,
                '.' => self.dot = true,
                '-' => self.dash = true,
                _ => self.broken = true,
            }
        }
    }

    fn merge(&mut self, other: Word) {
        self.broken |= other.broken;
        self.upper |= other.upper;
        self.lower |= other.lower;
        self.digit |= other.digit;
        self.underscore |= other.underscore;
        self.dot |= other.dot;
        self.dash |= other.dash;
    }

    /// The class of the characters, as `[A-Za-z0-9_.-]` has all of them.
    fn class(&self) -> String {
        let mut class = String::from("[");
        for (has, part) in [
            (self.upper, "A-Z"),
            (self.lower, "a-z"),
            (self.digit, "0-9"),
            (self.underscore, "_"),
            (self.dot, "."),
            (self.dash, "-"),
        ] {
            if has {
                class.push_str(part);
            }
        }
        class.push(']');
        class
    }
}

/// The strings, taken as lists with `separator` between their items. A list is
/// started by the first string that has the separator in it; each string before
/// that was one item, so the items so far are those strings. An empty string is
/// a list with no items.
#[derive(Clone, Debug)]
struct List {
    separator: char,
    items: Box<Strings>,
    /// A string had two items or more.
    several: bool,
    /// An item after a separator had a space before it.
    spaced: bool,
    /// A string is no list of this: an item was empty, or the items have nothing
    /// in common.
    broken: bool,
}

impl List {
    /// A list whose items so far are `strings`, one string each.
    fn starting(separator: char, strings: Strings) -> Self {
        let mut list = Self {
            separator,
            items: Box::new(Strings::plain()),
            several: false,
            spaced: false,
            broken: false,
        };
        list.take_whole(&strings);
        list
    }

    /// Strings that do not have the separator in them, each an item.
    fn take_whole(&mut self, strings: &Strings) {
        if !self.broken {
            self.items.merge(strings);
            self.broken = self.items.hopeless();
        }
    }

    fn add(&mut self, text: &str) {
        if self.broken {
            return;
        }
        let mut items = 0;
        for (i, item) in text.split(self.separator).enumerate() {
            let item = match item.strip_prefix(' ') {
                Some(rest) if i > 0 => {
                    self.spaced = true;
                    rest
                }
                _ => item,
            };
            if item.is_empty() {
                self.broken = true;
                return;
            }
            self.items.add(item);
            items += 1;
        }
        self.several |= items >= 2;
        self.broken = self.items.hopeless();
    }

    fn merge(&mut self, other: &List) {
        self.several |= other.several;
        self.spaced |= other.spaced;
        self.broken |= other.broken;
        if !self.broken {
            self.items.merge(&other.items);
            self.broken = self.items.hopeless();
        }
    }

    /// Its separator's bit, in [`Strings::unlisted`].
    fn bit(&self) -> u8 {
        separator_bit(self.separator)
    }

    fn name(&self) -> &'static str {
        match self.separator {
            ',' => "comma",
            ';' => "semicolon",
            _ => "bar",
        }
    }
}

/// A separator's bit, in [`Strings::unlisted`].
fn separator_bit(separator: char) -> u8 {
    1 << SEPARATORS.iter().position(|s| *s == separator).unwrap_or(0)
}

/// What a place's strings were found to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pattern {
    /// The expression, with `^` and `$`.
    pub expression: String,
    /// The `format` of JSON Schema for it, if the draft has one.
    pub format: Option<&'static str>,
    /// What it is, in a few words: "uuid", "comma-separated: email".
    pub what: String,
    pub source: Source,
}

/// Where a [`Pattern`] came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Source {
    /// Every string is of this kind.
    Kind(Kind),
    /// Every string is a list.
    List,
    /// Every string is of one shape.
    Shape,
}

/// The strings seen at one place.
#[derive(Clone, Debug)]
pub(super) struct Strings {
    pub count: usize,
    /// How many of them were empty: of no kind and no shape, and a list with no
    /// items. The facts, kinds, shape and word are of the others.
    empty: usize,
    facts: Facts,
    /// The kinds every string so far is of, one bit each.
    kinds: u32,
    shape: Shape,
    /// What the items of a list are made of, when each is a word.
    word: Word,
    values: Values,
    /// The strings as lists, one for each separator that a string has had; none
    /// for strings that are not taken as lists (the items of a list, keys).
    lists: Option<Vec<List>>,
    /// The separators the strings are no lists of, one bit each: a list of it
    /// broke, and is not started again.
    unlisted: u8,
}

impl Default for Strings {
    fn default() -> Self {
        Self {
            count: 0,
            empty: 0,
            facts: Facts::default(),
            kinds: Kind::EVERY,
            shape: Shape::Unseen,
            word: Word::default(),
            values: Values::Few(Vec::new()),
            lists: Some(Vec::new()),
            unlisted: 0,
        }
    }
}

impl Strings {
    /// For strings that are not to be taken as lists: the items of a list, and
    /// the keys of objects.
    pub fn plain() -> Self {
        Self {
            lists: None,
            ..Self::default()
        }
    }

    /// The same strings, not taken as lists: the items of a list started now.
    /// The empty ones are lists with no items, and so not among them.
    fn as_items(&self) -> Strings {
        let values = match &self.values {
            Values::Few(values) => Values::Few(
                values
                    .iter()
                    .filter(|(v, _)| !v.is_empty())
                    .cloned()
                    .collect(),
            ),
            Values::Many => Values::Many,
        };
        Strings {
            count: self.count - self.empty,
            empty: 0,
            facts: self.facts.clone(),
            kinds: self.kinds,
            shape: self.shape.clone(),
            word: self.word,
            values,
            lists: None,
            unlisted: 0,
        }
    }

    pub fn add(&mut self, text: &str) {
        // A list with no items, and nothing to learn from.
        if text.is_empty() {
            self.count += 1;
            self.empty += 1;
            self.values.add(text, 1);
            return;
        }
        // A separator seen for the first time starts a list of it.
        if let Some(lists) = &self.lists {
            let new: Vec<char> = SEPARATORS
                .into_iter()
                .filter(|&separator| {
                    self.unlisted & separator_bit(separator) == 0
                        && text.len() <= LONGEST
                        && text.contains(separator)
                        && !lists.iter().any(|list| list.separator == separator)
                })
                .collect();
            for separator in new {
                let list = List::starting(separator, self.as_items());
                if let Some(lists) = &mut self.lists {
                    lists.push(list);
                }
            }
        }
        let facts = Facts::of(text);
        if self.count == self.empty {
            self.facts = facts;
        } else {
            self.facts.merge(&facts);
        }
        self.count += 1;
        self.values.add(text, 1);
        if text.len() > LONGEST {
            self.kinds = 0;
            self.shape = Shape::Mixed;
            self.word.broken = true;
            self.values = Values::Many;
            if let Some(lists) = &mut self.lists {
                lists.clear();
            }
            self.unlisted = u8::MAX;
            return;
        }
        for kind in Kind::ALL {
            if self.kinds & kind.bit() != 0 && !kind.matches(text) {
                self.kinds &= !kind.bit();
            }
        }
        if !matches!(self.shape, Shape::Mixed) {
            self.shape.merge(Shape::of(text));
        }
        if !self.word.broken {
            self.word.add(text);
        }
        if let Some(lists) = &mut self.lists {
            for list in lists.iter_mut() {
                list.add(text);
                if list.broken {
                    self.unlisted |= list.bit();
                }
            }
            lists.retain(|list| !list.broken);
        }
    }

    pub fn merge(&mut self, other: &Strings) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            let lists = self.lists.is_some();
            *self = other.clone();
            if !lists {
                self.lists = None;
            }
            return;
        }
        self.merge_lists(other);
        if self.count == self.empty {
            self.facts = other.facts.clone();
        } else if other.count > other.empty {
            self.facts.merge(&other.facts);
        }
        self.count += other.count;
        self.empty += other.empty;
        self.kinds &= other.kinds;
        self.shape.merge(other.shape.clone());
        self.word.merge(other.word);
        self.values.merge(&other.values);
    }

    /// Put the lists of `other` with these, before the rest of it is: a list one
    /// side has and the other has not is that side's list, with the other side's
    /// strings as items, whole.
    fn merge_lists(&mut self, other: &Strings) {
        let (Some(mine), Some(theirs)) = (&self.lists, &other.lists) else {
            return;
        };
        let unlisted = self.unlisted | other.unlisted;
        let mut merged = Vec::new();
        for separator in SEPARATORS {
            if unlisted & separator_bit(separator) != 0 {
                continue;
            }
            let ours = mine.iter().find(|l| l.separator == separator);
            let their = theirs.iter().find(|l| l.separator == separator);
            let list = match (ours, their) {
                (Some(ours), Some(their)) => {
                    let mut list = ours.clone();
                    list.merge(their);
                    list
                }
                (Some(ours), None) => {
                    let mut list = ours.clone();
                    list.take_whole(&other.as_items());
                    list
                }
                (None, Some(their)) => {
                    let mut list = List::starting(separator, self.as_items());
                    list.merge(their);
                    list
                }
                (None, None) => continue,
            };
            merged.push(list);
        }
        for list in merged.iter().filter(|l| l.broken) {
            self.unlisted |= list.bit();
        }
        merged.retain(|list| !list.broken);
        self.unlisted |= unlisted;
        self.lists = Some(merged);
    }

    /// No kind, shape or word is left that the strings could all be.
    fn hopeless(&self) -> bool {
        self.kinds == 0 && matches!(self.shape, Shape::Mixed) && self.word.broken
    }

    fn facts(&self) -> Facts {
        Facts {
            distinct: self.values.distinct(),
            ..self.facts.clone()
        }
    }

    /// The different strings, when there are a few (each no longer than
    /// `longest` characters), in the order they came, each with how many times
    /// it was seen.
    pub fn few_values(&self, longest: usize) -> Option<Vec<(&str, usize)>> {
        let Values::Few(values) = &self.values else {
            return None;
        };
        values
            .iter()
            .all(|(v, _)| v.chars().count() <= longest)
            .then(|| values.iter().map(|(v, n)| (v.as_str(), *n)).collect())
    }

    /// The first kind the strings all are, as a whole.
    fn kind(&self, facts: &Facts) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|kind| self.kinds & kind.bit() != 0 && kind.fits(facts))
    }

    /// The fewest and the most characters a string had.
    pub fn lengths(&self) -> (usize, usize) {
        let least = if self.empty > 0 {
            0
        } else {
            self.facts.min_chars
        };
        (least, self.facts.max_chars)
    }

    /// What the strings are, when they are something: of a kind, lists of
    /// something, or of a shape. A list is empty, or one item or more; an empty
    /// string is of no kind and no shape. With `bounds`, lengths that vary are
    /// given as the least and the most seen.
    pub fn pattern(&self, bounds: bool, draft: Draft) -> Option<Pattern> {
        let facts = self.facts();
        if let Some(kind) = self.kind(&facts).filter(|_| self.empty == 0) {
            return Some(Pattern {
                expression: format!("^{}$", kind.pattern(&facts, bounds)),
                format: kind
                    .format()
                    .filter(|(_, since)| draft >= *since)
                    .map(|(format, _)| format),
                what: kind.name().to_owned(),
                source: Source::Kind(kind),
            });
        }
        for list in self.lists.iter().flatten() {
            if !list.several {
                continue;
            }
            let Some((item, what)) = list.items.item(bounds) else {
                continue;
            };
            let space = if list.spaced { " ?" } else { "" };
            let separator = escape(&list.separator.to_string());
            return Some(Pattern {
                expression: format!("^(?:{item}(?:{separator}{space}{item})*)?$"),
                format: None,
                what: format!("{}-separated: {what}", list.name()),
                source: Source::List,
            });
        }
        if self.empty > 0 {
            return None;
        }
        let shape = self.shape.pattern(facts.distinct, bounds)?;
        Some(Pattern {
            expression: format!("^{shape}$"),
            format: None,
            what: format!("like {}", self.shape.example()),
            source: Source::Shape,
        })
    }

    /// What the items of a list are: of a kind, of a shape, or words.
    fn item(&self, bounds: bool) -> Option<(String, String)> {
        let facts = self.facts();
        if let Some(kind) = self.kind(&facts) {
            return Some((kind.pattern(&facts, bounds), kind.name().to_owned()));
        }
        if let Some(shape) = self.shape.pattern(facts.distinct, bounds) {
            return Some((shape, format!("like {}", self.shape.example())));
        }
        if self.word.broken || self.count == 0 {
            return None;
        }
        let count = if bounds {
            facts.count(facts.min_chars, facts.max_chars, true)
        } else {
            "+".to_owned()
        };
        Some((format!("{}{count}", self.word.class()), "words".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(texts: &[&str]) -> Strings {
        let mut strings = Strings::default();
        for text in texts {
            strings.add(text);
        }
        strings
    }

    /// The expression and what it is, for these strings.
    fn found(texts: &[&str]) -> Option<(String, String)> {
        strings(texts)
            .pattern(false, Draft::D202012)
            .map(|p| (p.expression, p.what))
    }

    fn expression(texts: &[&str]) -> Option<String> {
        found(texts).map(|(e, _)| e)
    }

    #[test]
    fn strings_of_a_kind_have_its_expression_and_its_format() {
        let p = strings(&["123e4567-e89b-12d3-a456-426614174000"])
            .pattern(false, Draft::D202012)
            .unwrap();
        assert_eq!(p.what, "uuid");
        assert_eq!(p.format, Some("uuid"));
        assert_eq!(
            p.expression, "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
            "in the case they are written in"
        );
        // A draft that has no such format has the expression alone.
        let p = strings(&["123e4567-e89b-12d3-a456-426614174000"])
            .pattern(false, Draft::D7)
            .unwrap();
        assert_eq!(p.format, None);
        let p = strings(&["2024-01-15"]).pattern(false, Draft::D6).unwrap();
        assert_eq!((p.what.as_str(), p.format), ("date", None));
        let p = strings(&["2024-01-15"]).pattern(false, Draft::D7).unwrap();
        assert_eq!(p.format, Some("date"));
    }

    #[test]
    fn strings_of_different_kinds_are_of_the_kind_they_share_or_of_none() {
        // Neither expression takes the other: no kind.
        assert_eq!(found(&["2024-01-15", "2024-01-15T10:00:00Z"]), None);
        // Whole numbers and decimals are numbers.
        assert_eq!(
            found(&["1", "2.5", "-3"]),
            Some((
                "^-?[0-9]+(?:\\.[0-9]+)?$".to_owned(),
                "number as text".to_owned()
            ))
        );
    }

    #[test]
    fn a_length_is_said_when_several_different_strings_agree_on_it() {
        assert_eq!(
            expression(&["90210", "10001", "60601"]).unwrap(),
            "^[0-9]{5}$"
        );
        // One string says nothing of the next.
        assert_eq!(expression(&["90210"]).unwrap(), "^[0-9]+$");
        // The same one again neither.
        assert_eq!(expression(&["90210", "90210"]).unwrap(), "^[0-9]+$");
        // Lengths that differ are one or more…
        assert_eq!(expression(&["7", "1234"]).unwrap(), "^[0-9]+$");
        // …and a single digit is no width.
        assert_eq!(expression(&["1", "2", "3"]).unwrap(), "^[0-9]+$");
        // Prices, with two decimals.
        assert_eq!(
            expression(&["12.50", "3.99"]).unwrap(),
            "^[0-9]+\\.[0-9]{2}$"
        );
        assert_eq!(
            expression(&["-1.5", "2.25"]).unwrap(),
            "^-?[0-9]+\\.[0-9]+$"
        );
        // With bounds, the least and the most.
        let s = strings(&["7", "1234"]);
        assert_eq!(
            s.pattern(true, Draft::D202012).unwrap().expression,
            "^[0-9]{1,4}$"
        );
    }

    #[test]
    fn codes_and_hex_strings_are_known_by_all_of_them() {
        assert_eq!(expression(&["USD", "EUR", "GBP"]).unwrap(), "^[A-Z]{3}$");
        assert_eq!(expression(&["USD"]), None, "one is a word in capitals");
        assert_eq!(expression(&["USD", "EURO"]), None);
        let sha = [
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "60303ae22b998861bce3b28f33eec1be758a213c86c93c076dbe9f558c11c752",
        ];
        assert_eq!(expression(&sha).unwrap(), "^[0-9a-f]{64}$");
        assert_eq!(expression(&["ada", "bad", "cab"]), None, "words of a to f");
    }

    #[test]
    fn strings_of_one_shape_have_its_expression() {
        assert_eq!(
            found(&["ORD-2024-00017"]).unwrap(),
            (
                "^[A-Z]+-[0-9]+-[0-9]+$".to_owned(),
                "like ORD-2024-00017".to_owned()
            )
        );
        // Several say which letters are always there, and the widths.
        assert_eq!(
            expression(&["ORD-2024-00017", "ORD-2023-00102", "ORD-2024-00230"]).unwrap(),
            "^ORD-[0-9]{4}-[0-9]{5}$"
        );
        assert_eq!(
            expression(&["AB12", "cd345"]).unwrap(),
            "^[A-Za-z]{2}[0-9]+$",
            "letters of either case are letters"
        );
        assert_eq!(
            expression(&["v1.2", "v10.4"]).unwrap(),
            "^v[0-9]+\\.[0-9]+$"
        );
        // A name is not a code, nor a sentence with a number in it.
        assert_eq!(expression(&["Ada Lovelace"]), None);
        assert_eq!(expression(&["Flat 3"]), None);
        assert_eq!(expression(&["hello", "world"]), None);
        // Shapes that differ are no shape.
        assert_eq!(expression(&["A-1", "A1"]), None);
    }

    #[test]
    fn a_list_with_commas_is_items_of_a_kind_again_and_again() {
        assert_eq!(
            found(&["1,2,3", "40,5"]).unwrap(),
            (
                "^(?:[0-9]+(?:,[0-9]+)*)?$".to_owned(),
                "comma-separated: integer as text".to_owned()
            )
        );
        assert_eq!(
            found(&["ada@example.com; bob@example.org", "cy@example.net"]).unwrap(),
            (
                "^(?:[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\\.[A-Za-z0-9-]+)*\\.[A-Za-z]{2,}(?:; ?[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\\.[A-Za-z0-9-]+)*\\.[A-Za-z]{2,})*)?$".to_owned(),
                "semicolon-separated: email".to_owned()
            )
        );
        assert_eq!(
            found(&["red|green|blue"]).unwrap(),
            (
                "^(?:[a-z]+(?:\\|[a-z]+)*)?$".to_owned(),
                "bar-separated: words".to_owned()
            )
        );
        assert_eq!(
            found(&["10.0.0.1, 10.0.0.2"]).unwrap().1,
            "comma-separated: IPv4 address"
        );
        assert_eq!(
            found(&["en-US,fr-FR", "de-DE"]).unwrap().1,
            "comma-separated: language tag"
        );
        assert_eq!(
            found(&["A1,B22"]).unwrap(),
            (
                "^(?:[A-Z][0-9]+(?:,[A-Z][0-9]+)*)?$".to_owned(),
                "comma-separated: like A1".to_owned()
            )
        );
    }

    #[test]
    fn a_list_started_late_has_the_strings_before_it_as_items() {
        assert_eq!(
            found(&["a", "b", "c,d"]).unwrap().1,
            "comma-separated: words"
        );
        assert_eq!(
            expression(&["7", "12", "3;45"]).unwrap(),
            "^(?:[0-9]+(?:;[0-9]+)*)?$"
        );
        // A string before that is no item makes it no list.
        assert_eq!(found(&["a b", "c,d"]), None);
        // A list that broke is not started again by a string that would do.
        assert_eq!(found(&["a,,b", "c,d"]), None);
        // Nor is a string after it any different.
        assert_eq!(found(&["c,d", "a,,b", "e,f"]), None);
    }

    #[test]
    fn a_list_only_one_of_two_places_has_takes_the_strings_of_the_other_as_items() {
        let mut a = strings(&["3"]);
        a.merge(&strings(&["1,2"]));
        assert_eq!(
            a.pattern(false, Draft::D202012).unwrap().what,
            "comma-separated: integer as text"
        );
        let mut b = strings(&["x y"]);
        b.merge(&strings(&["1,2"]));
        assert_eq!(b.pattern(false, Draft::D202012), None);
        // One side's broken list stays broken.
        let mut c = strings(&["a,,b"]);
        c.merge(&strings(&["c,d"]));
        assert_eq!(c.pattern(false, Draft::D202012), None);
    }

    #[test]
    fn an_empty_string_is_a_list_with_no_items() {
        let ips = found(&["10.0.0.1, 10.0.0.2", "10.0.0.3"]).unwrap();
        assert_eq!(ips.1, "comma-separated: IPv4 address");
        assert!(
            ips.0.starts_with("^(?:(?:(?:25[0-5]|") && ips.0.ends_with("))*)?$"),
            "{}",
            ips.0
        );
        for texts in [
            &["10.0.0.1, 10.0.0.2", "", "10.0.0.3"][..],
            &["", "10.0.0.3", "10.0.0.1, 10.0.0.2"],
            &["10.0.0.3", "", "", "10.0.0.1, 10.0.0.2"],
        ] {
            assert_eq!(found(texts).as_ref(), Some(&ips), "{texts:?}");
        }
        // Nor does it count among the items: two different ones agree on a width.
        assert_eq!(
            expression(&["", "12", "34,56"]).unwrap(),
            "^(?:[0-9]{2}(?:,[0-9]{2})*)?$"
        );
        assert_eq!(
            expression(&["", "", "12,34"]).unwrap(),
            "^(?:[0-9]{2}(?:,[0-9]{2})*)?$"
        );
        assert_eq!(
            expression(&["", "12", "12,12"]).unwrap(),
            "^(?:[0-9]+(?:,[0-9]+)*)?$"
        );
        // Nor do the empty ones of a place it was put together with.
        let mut c = strings(&["", ""]);
        c.merge(&strings(&["12"]));
        c.add("34,56");
        assert_eq!(
            c.pattern(false, Draft::D202012).unwrap().expression,
            "^(?:[0-9]{2}(?:,[0-9]{2})*)?$"
        );
        let mut d = strings(&["12"]);
        d.merge(&strings(&["", ""]));
        d.add("34,56");
        assert_eq!(
            d.pattern(false, Draft::D202012).unwrap().expression,
            "^(?:[0-9]{2}(?:,[0-9]{2})*)?$"
        );
        // A place put together with one of empty strings only.
        let mut a = strings(&["", ""]);
        a.merge(&strings(&["1,2"]));
        assert_eq!(
            a.pattern(false, Draft::D202012).unwrap().what,
            "comma-separated: integer as text"
        );
        let mut b = strings(&["3"]);
        b.merge(&strings(&["", "1,2"]));
        assert_eq!(
            b.pattern(false, Draft::D202012).unwrap().what,
            "comma-separated: integer as text"
        );
        assert_eq!(b.lengths(), (0, 3));
        // Empty strings and no list are no kind and no shape, and nothing at all.
        assert_eq!(found(&["", "10.0.0.1"]), None);
        assert_eq!(found(&["ORD-17", ""]), None);
        assert_eq!(found(&["", ""]), None);
        assert_eq!(strings(&["", "ab"]).lengths(), (0, 2));
    }

    #[test]
    fn what_is_not_a_list_is_not_taken_for_one() {
        // No string has two items.
        assert_eq!(found(&["red", "green"]), None);
        // An empty item.
        assert_eq!(found(&["a,,b"]), None);
        // Text, with commas.
        assert_eq!(found(&["Hello, how are you"]), None);
        // A kind of the whole string comes first: a URL with a comma in it.
        assert_eq!(found(&["https://example.com/?a=1,2"]).unwrap().1, "URL");
    }

    #[test]
    fn two_places_put_together_are_what_their_strings_are_together() {
        let mut a = strings(&["123e4567-e89b-12d3-a456-426614174000"]);
        a.merge(&strings(&["C56A4180-65AA-42EC-A945-5FD21DEC0538"]));
        assert_eq!(a.count, 2);
        assert_eq!(
            a.pattern(false, Draft::D202012).unwrap().expression,
            "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
        );
        let mut b = strings(&["1,2"]);
        b.merge(&strings(&["x y"]));
        assert_eq!(b.pattern(false, Draft::D202012), None);
        let mut c = strings(&["1,2"]);
        c.merge(&strings(&["3"]));
        assert_eq!(
            c.pattern(false, Draft::D202012).unwrap().what,
            "comma-separated: integer as text"
        );
        // An empty place takes the other as it is.
        let mut d = Strings::default();
        d.merge(&strings(&["2024-01-15"]));
        assert_eq!(d.pattern(false, Draft::D202012).unwrap().what, "date");
    }

    #[test]
    fn a_few_values_are_remembered_and_many_are_not() {
        let s = strings(&["a", "b", "a"]);
        assert_eq!(s.few_values(64), Some(vec![("a", 2), ("b", 1)]));
        assert_eq!(s.few_values(0), None, "longer than asked");
        let many: Vec<String> = (0..=MAX_VALUES).map(|i| i.to_string()).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(strings(&many).few_values(64), None);
    }

    #[test]
    fn a_long_string_is_counted_and_not_looked_at() {
        let long = "1".repeat(LONGEST + 1);
        let s = strings(&[&long]);
        assert_eq!(s.count, 1);
        assert_eq!(s.pattern(false, Draft::D202012), None);
        assert_eq!(s.lengths(), (LONGEST + 1, LONGEST + 1));
    }
}
