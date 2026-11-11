//! macOS-style text replacement for the composer.
//!
//! System text replacements only fire inside Cocoa text views. The composer
//! is an egui canvas and winit's `NSTextInputClient` ignores substitution
//! ranges, so the user's own replacement table (System Settings > Keyboard >
//! Text Replacements) is read from the global preferences domain and applied
//! here when a delimiter is typed.

use std::ops::Range;

/// One trigger/replacement pair, such as `omw` -> `On my way!`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    pub replace: String,
    pub with: String,
}

/// A typed character that commits a pending trigger: space, return or
/// punctuation, matching the macOS expander's word breaks.
pub fn is_delimiter(character: char) -> bool {
    character.is_whitespace() || character.is_ascii_punctuation()
}

/// The user's replacement table. On macOS it is read from the global
/// preferences domain; elsewhere there is no system table yet.
pub fn system_replacements() -> Vec<Replacement> {
    system_replacements_impl()
}

#[cfg(target_os = "macos")]
fn system_replacements_impl() -> Vec<Replacement> {
    let output = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "-g", "NSUserDictionaryReplacementItems"])
        .output();
    match output {
        Ok(done) if done.status.success() => String::from_utf8(done.stdout)
            .map(|text| parse_replacements(&text))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

#[cfg(not(target_os = "macos"))]
fn system_replacements_impl() -> Vec<Replacement> {
    Vec::new()
}

/// The trigger to expand in `text` before byte index `cursor`, as the
/// longest suffix match at a word boundary, plus its replacement. Two
/// candidates are tried: the text before the cursor as-is, so a trigger
/// ending in the delimiter itself (such as `shrug,,`) expands on its final
/// character, and the text before the trailing delimiter, so a plain trigger
/// expands on the following space, return or punctuation mark. A typed
/// delimiter stays in place after the replacement.
pub fn expand_at(
    table: &[Replacement],
    text: &str,
    cursor: usize,
) -> Option<(Range<usize>, String)> {
    let before = text.get(..cursor)?;
    let mut candidates: Vec<&str> = vec![before];
    if let Some(delimiter) = before.chars().next_back().filter(|c| is_delimiter(*c)) {
        candidates.push(&before[..before.len() - delimiter.len_utf8()]);
    }
    let mut best: Option<(Range<usize>, &str)> = None;
    for candidate in candidates {
        for entry in table {
            let trigger = entry.replace.as_str();
            if trigger.is_empty() || !candidate.ends_with(trigger) {
                continue;
            }
            let start = candidate.len() - trigger.len();
            // A letter or digit right before the trigger makes it part of a
            // longer word, not the trigger.
            if candidate[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            {
                continue;
            }
            let range = start..start + trigger.len();
            if best
                .as_ref()
                .is_none_or(|(known, _)| range.len() > known.len())
            {
                best = Some((range, entry.with.as_str()));
            }
        }
    }
    best.map(|(range, with)| (range, with.to_owned()))
}

/// Parses the output of `defaults read -g NSUserDictionaryReplacementItems`
/// (an old-style plist fragment) into replacement pairs, skipping entries
/// switched off (`on = 0`). Malformed input yields what parsed so far.
pub fn parse_replacements(input: &str) -> Vec<Replacement> {
    let Some(value) = Parser::new(input).value() else {
        return Vec::new();
    };
    let Value::Array(entries) = value else {
        return Vec::new();
    };
    let mut table = Vec::new();
    for entry in entries {
        let Value::Dict(fields) = entry else {
            continue;
        };
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| key == name)
                .and_then(|(_, value)| match value {
                    Value::Atom(atom) => Some(atom.as_str()),
                    _ => None,
                })
        };
        if field("on") == Some("0") {
            continue;
        }
        if let (Some(replace), Some(with)) = (field("replace"), field("with")) {
            table.push(Replacement {
                replace: replace.to_owned(),
                with: with.to_owned(),
            });
        }
    }
    table
}

/// Old-style plist value: arrays, dictionaries and atoms (quoted or bare).
enum Value {
    Atom(String),
    Array(Vec<Value>),
    Dict(Vec<(String, Value)>),
}

struct Parser<'a> {
    rest: &'a [u8],
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            rest: input.as_bytes(),
        }
    }

    fn skip_space(&mut self) {
        while let Some(&byte) = self.rest.first() {
            if byte.is_ascii_whitespace() {
                self.rest = &self.rest[1..];
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_space();
        if self.rest.first() == Some(&byte) {
            self.rest = &self.rest[1..];
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Option<Value> {
        self.skip_space();
        match self.rest.first()? {
            b'(' => self.array(),
            b'{' => self.dict(),
            _ => self.atom().map(Value::Atom),
        }
    }

    fn array(&mut self) -> Option<Value> {
        if !self.eat(b'(') {
            return None;
        }
        let mut items = Vec::new();
        loop {
            self.skip_space();
            match self.rest.first()? {
                b')' => {
                    self.rest = &self.rest[1..];
                    return Some(Value::Array(items));
                }
                b',' => {
                    self.rest = &self.rest[1..];
                }
                _ => items.push(self.value()?),
            }
        }
    }

    fn dict(&mut self) -> Option<Value> {
        if !self.eat(b'{') {
            return None;
        }
        let mut fields = Vec::new();
        loop {
            self.skip_space();
            if self.rest.first()? == &b'}' {
                self.rest = &self.rest[1..];
                return Some(Value::Dict(fields));
            }
            let key = self.atom()?;
            if !self.eat(b'=') {
                return None;
            }
            let value = self.value()?;
            if !self.eat(b';') {
                return None;
            }
            fields.push((key, value));
        }
    }

    /// A quoted string with escapes, or a bare atom up to the next
    /// whitespace or structural character.
    fn atom(&mut self) -> Option<String> {
        self.skip_space();
        if self.rest.first() == Some(&b'"') {
            return self.quoted();
        }
        let end = self
            .rest
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || b"(){}=;,\"".contains(byte))
            .unwrap_or(self.rest.len());
        if end == 0 {
            return None;
        }
        let (atom, rest) = self.rest.split_at(end);
        self.rest = rest;
        std::str::from_utf8(atom).ok().map(str::to_owned)
    }

    fn quoted(&mut self) -> Option<String> {
        self.rest = &self.rest[1..];
        let mut out = String::new();
        loop {
            let (&byte, rest) = self.rest.split_first()?;
            self.rest = rest;
            match byte {
                b'"' => return Some(out),
                b'\\' => out.push(self.escape()?),
                _ => {
                    // UTF-8 passes through byte by byte.
                    let mut buf = [0u8; 4];
                    let bytes = if byte < 0x80 {
                        buf[..1].copy_from_slice(&[byte]);
                        &buf[..1]
                    } else {
                        let width = match byte {
                            0xC0..=0xDF => 2,
                            0xE0..=0xEF => 3,
                            _ => 4,
                        };
                        if self.rest.len() < width - 1 {
                            return None;
                        }
                        buf[0] = byte;
                        buf[1..width].copy_from_slice(&self.rest[..width - 1]);
                        self.rest = &self.rest[width - 1..];
                        &buf[..width]
                    };
                    out.push_str(std::str::from_utf8(bytes).ok()?);
                }
            }
        }
    }

    fn escape(&mut self) -> Option<char> {
        let (&byte, rest) = self.rest.split_first()?;
        self.rest = rest;
        match byte {
            b'n' => Some('\n'),
            b'r' => Some('\r'),
            b't' => Some('\t'),
            b'\\' => Some('\\'),
            b'"' => Some('"'),
            b'U' => {
                if self.rest.len() < 4 {
                    return None;
                }
                let (hex, rest) = self.rest.split_at(4);
                self.rest = rest;
                let code = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                char::from_u32(code)
            }
            b'0'..=b'7' => {
                // Octal escape, up to three digits.
                let mut value = u32::from(byte - b'0');
                for _ in 0..2 {
                    match self.rest.first() {
                        Some(digit @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(digit - b'0');
                            self.rest = &self.rest[1..];
                        }
                        _ => break,
                    }
                }
                char::from_u32(value)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<Replacement> {
        vec![
            Replacement {
                replace: "omw".into(),
                with: "On my way!".into(),
            },
            Replacement {
                replace: "shrug,,".into(),
                with: "¯\\_(ツ)_/¯".into(),
            },
        ]
    }

    #[test]
    fn parse_typical_defaults_output() {
        let input = "(\n        {\n        on = 1;\n        replace = omw;\n        with = \"On my way!\";\n    },\n        {\n        on = 1;\n        replace = \"shrug,,\";\n        with = \"\\U00af\\\\_(\\U30c4)_/\\U00af\";\n    }\n)\n";
        assert_eq!(parse_replacements(input), table());
    }

    #[test]
    fn parse_skips_switched_off_entries() {
        let input = "({ on = 0; replace = omw; with = \"On my way!\"; })";
        assert_eq!(parse_replacements(input), Vec::new());
    }

    #[test]
    fn parse_handles_escapes_and_bare_atoms() {
        let input = "({ replace = \"a\\\"b\"; with = \"c\\\\d\\ne\\050\\U00e4\"; })";
        assert_eq!(
            parse_replacements(input),
            vec![Replacement {
                replace: "a\"b".into(),
                with: "c\\d\ne(ä".into(),
            }]
        );
    }

    #[test]
    fn parse_rejects_garbage_without_panicking() {
        assert_eq!(parse_replacements(""), Vec::new());
        assert_eq!(parse_replacements("not a plist"), Vec::new());
        assert_eq!(parse_replacements("({ replace = x;"), Vec::new());
        assert_eq!(parse_replacements("{ replace = x; with = y; }"), Vec::new());
    }

    #[test]
    fn plain_trigger_expands_on_a_following_delimiter() {
        let (range, with) = expand_at(&table(), "ok omw ", 7).expect("omw expands");
        assert_eq!(range, 3..6);
        assert_eq!(with, "On my way!");
    }

    #[test]
    fn trigger_inside_a_longer_word_does_not_expand() {
        assert_eq!(expand_at(&table(), "xomw ", 5), None);
        assert_eq!(expand_at(&table(), "nomw, ", 6), None);
    }

    #[test]
    fn trigger_ending_in_punctuation_expands_on_its_last_character() {
        let (range, with) = expand_at(&table(), "shrug,,", 7).expect("expands at the 2nd comma");
        assert_eq!(range, 0..7);
        assert_eq!(with, "¯\\_(ツ)_/¯");
        let (range, _) = expand_at(&table(), "well shrug,, ", 13).expect("and after a space");
        assert_eq!(range, 5..12);
    }

    #[test]
    fn the_longest_trigger_wins() {
        let table = vec![
            Replacement {
                replace: "a".into(),
                with: "A".into(),
            },
            Replacement {
                replace: "b a".into(),
                with: "BA".into(),
            },
        ];
        let (range, with) = expand_at(&table, "b a ", 4).expect("longest match");
        assert_eq!(range, 0..3);
        assert_eq!(with, "BA");
    }

    #[test]
    fn expansion_works_in_the_middle_of_text() {
        let (range, _) = expand_at(&table(), "say omw there", 8).expect("mid-text cursor");
        assert_eq!(range, 4..7);
    }

    #[test]
    fn punctuation_before_a_trigger_is_a_boundary() {
        let (range, _) = expand_at(&table(), "(omw ", 5).expect("boundary before");
        assert_eq!(range, 1..4);
    }

    #[test]
    fn unknown_triggers_do_not_expand() {
        assert_eq!(expand_at(&table(), "hello ", 6), None);
        assert_eq!(expand_at(&[], "omw ", 4), None);
    }

    #[test]
    fn unicode_triggers_and_replacements_work() {
        let table = vec![Replacement {
            replace: "åh".into(),
            with: "Åhå".into(),
        }];
        let text = "jaha åh ";
        let (range, with) = expand_at(&table, text, text.len()).expect("unicode trigger");
        assert_eq!(&text[range], "åh");
        assert_eq!(with, "Åhå");
    }

    #[test]
    fn delimiters_are_whitespace_or_ascii_punctuation() {
        assert!(is_delimiter(' '));
        assert!(is_delimiter('\n'));
        assert!(is_delimiter(','));
        assert!(is_delimiter('.'));
        assert!(!is_delimiter('a'));
        assert!(!is_delimiter('ä'));
    }
}
