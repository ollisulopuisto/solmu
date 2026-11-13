//! Line-break balancing for chat bubbles: pure decision logic, no egui.
//!
//! Instead of egui's greedy first-fit wrap, body text is broken by a
//! Knuth-Plass-style dynamic program that minimises raggedness (squared
//! slack per line) with demerits for hyphenated breaks, extra demerits for
//! consecutive hyphenated lines, and a runt penalty for a very short final
//! line. Justification-style stretch/shrink badness is the wrong model for
//! ragged-right chat text, so only slack is scored.
//!
//! Rules baked in (from the line-wrap design sheets):
//!
//! * URLs are never hyphenated. A URL that fits within the width cap stays
//!   whole: it moves to its own line when needed, and the bubble widens to
//!   it. A URL wider than the cap breaks at punctuation only, without a
//!   hyphen glyph.
//! * Words longer than [`MIN_HYPHENATED_CHARS`] may hyphenate at the points
//!   the hyphenator offers, but never leaving fewer than
//!   [`MIN_HYPHEN_FRAGMENT`] letters on either side. A compound word (the
//!   hyphenator marks compound joints) breaks at its joints only.
//! * A word wider than the line with no usable hyphen point falls back to
//!   hyphenless character splits, as plain greedy wrapping does.
//!
//! Measuring text lives in [`crate::markup`]; hyphenation lives in
//! [`crate::hyphenate`]. This module only sees per-character widths.

/// Words hyphenate only when longer than this many letters.
pub const MIN_HYPHENATED_CHARS: usize = 10;
/// A hyphen break must leave at least this many letters on each side.
pub const MIN_HYPHEN_FRAGMENT: usize = 3;

/// Demerit of ending a line on a hyphen (already squared, in points²).
/// Free at compound joints.
const HYPHEN_PENALTY: f32 = 50.0 * 50.0;
/// Extra flat demerit when consecutive lines end on hyphens.
const CONSECUTIVE_HYPHEN_PENALTY: f32 = 10_000.0;
/// Demerit of breaking a URL at punctuation (already squared, in points²).
const URL_BREAK_PENALTY: f32 = 10.0 * 10.0;
/// Demerit of a hyphenless mid-word character split (already squared, in
/// points²). High enough to lose to every other break, cheap enough to
/// stack when a very long word needs several.
const FORCED_SPLIT_PENALTY: f32 = 1_000.0 * 1_000.0;
/// A final line narrower than this share of the wrap width is a runt and
/// pays for the missing width (quadratically, like slack).
const RUNT_SHARE: f32 = 0.25;
/// Overflow (an indivisible piece wider than the line) pays this multiple
/// of its squared excess, so it loses to any available break and wins only
/// when nothing can break.
const OVERFLOW_PENALTY: f32 = 1_000.0;

/// A hyphenation point inside a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hyphen {
    /// Character index within the word the break falls after.
    pub at: usize,
    /// Whether this point is a compound-word joint (`käyttö|liittymä`).
    pub joint: bool,
}

/// What one whitespace-delimited run of text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Word,
    /// A run of spaces or tabs.
    Space,
    /// A hard line break from the text itself.
    Newline,
    /// A word covered by a detected URL span.
    Url,
}

/// One token of body text with its measured shape.
#[derive(Clone, Debug)]
pub struct Item {
    /// Byte range in the original text.
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
    /// The token's characters, aligned with `widths`.
    pub chars: Vec<char>,
    /// Advance width per character, in order.
    pub widths: Vec<f32>,
    /// Usable hyphenation points, character indices within this item.
    /// Words only, already filtered by the fragment and joint rules.
    pub hyphens: Vec<Hyphen>,
}

impl Item {
    /// Sum of the character advances.
    pub fn width(&self) -> f32 {
        self.widths.iter().sum()
    }

    fn len(&self) -> usize {
        self.widths.len()
    }

    /// Byte offset of character `c` within the original text.
    fn byte_of(&self, c: usize) -> usize {
        self.start
            + self.chars[..c]
                .iter()
                .map(|ch| ch.len_utf8())
                .sum::<usize>()
    }
}

/// Splits `text` into tokens. `widths` carries one advance width per
/// character of `text`; `urls` marks byte ranges detected as URLs;
/// `hyphenate` offers hyphenation points for a single word.
pub fn tokenize(
    text: &str,
    widths: &[f32],
    urls: &[(usize, usize)],
    hyphenate: &dyn Fn(&str) -> Vec<Hyphen>,
) -> Vec<Item> {
    debug_assert_eq!(widths.len(), text.chars().count());
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut items = Vec::new();
    let mut pos = 0;
    while pos < chars.len() {
        let kind = kind_of(chars[pos].1);
        let mut next = pos + 1;
        while next < chars.len() && kind != Kind::Newline && kind_of(chars[next].1) == kind {
            next += 1;
        }
        let end_byte = chars.get(next).map(|(b, _)| *b).unwrap_or(text.len());
        let mut item = Item {
            start: chars[pos].0,
            end: end_byte,
            kind,
            chars: chars[pos..next].iter().map(|(_, c)| *c).collect(),
            widths: widths[pos..next].to_vec(),
            hyphens: Vec::new(),
        };
        if kind == Kind::Word {
            if urls.iter().any(|(s, e)| *s <= item.start && item.end <= *e) {
                item.kind = Kind::Url;
            } else if item.len() > MIN_HYPHENATED_CHARS {
                let word = &text[item.start..item.end];
                let mut points = hyphenate(word);
                points.retain(|h| {
                    h.at >= MIN_HYPHEN_FRAGMENT && h.at <= item.len() - MIN_HYPHEN_FRAGMENT
                });
                points.sort_by_key(|h| h.at);
                points.dedup_by_key(|h| h.at);
                // Compounds break at their joints only (design sheet 4,
                // decision ii); a word without marked joints may use all
                // remaining points.
                if points.iter().any(|h| h.joint) {
                    points.retain(|h| h.joint);
                }
                item.hyphens = points;
            }
        }
        items.push(item);
        pos = next;
    }
    items
}

fn kind_of(ch: char) -> Kind {
    if ch == '\n' || ch == '\r' {
        Kind::Newline
    } else if ch.is_whitespace() {
        Kind::Space
    } else {
        Kind::Word
    }
}

/// Where the balanced layout breaks the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineBreak {
    /// Byte position in the original text the break falls before. For a
    /// hyphenated break this sits inside the broken word.
    pub byte: usize,
    /// Whether the line above ends with an inserted hyphen glyph.
    pub hyphen: bool,
}

/// The result of balancing one text at one width.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Breaks in text order, one per line after the first.
    pub breaks: Vec<LineBreak>,
    /// Lines the text takes with these breaks.
    pub lines: usize,
    /// Longest line, including any trailing hyphen glyph: the natural
    /// width a bubble takes at this wrap width.
    pub width: f32,
}

/// How a line may end after a unit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Break {
    /// No break allowed.
    None,
    /// After a space: free.
    Free,
    /// A hard newline: a break must be taken.
    Mandatory,
    /// A hyphenation point; free at compound joints.
    Hyphen { joint: bool },
    /// Inside an over-wide URL, after punctuation, without a hyphen glyph.
    UrlPunct,
    /// Hyphenless mid-word character split, the last resort.
    Forced,
}

impl Break {
    fn penalty(self) -> f32 {
        match self {
            Break::None | Break::Free | Break::Mandatory => 0.0,
            Break::Hyphen { joint: true } => 0.0,
            Break::Hyphen { joint: false } => HYPHEN_PENALTY,
            Break::UrlPunct => URL_BREAK_PENALTY,
            Break::Forced => FORCED_SPLIT_PENALTY,
        }
    }

    fn hyphen_glyph(self) -> bool {
        matches!(self, Break::Hyphen { .. })
    }
}

/// One indivisible piece of a line: a space run, a newline, a word segment
/// between hyphenation points, a URL segment, or a single character of an
/// over-wide word.
#[derive(Clone, Copy, Debug)]
struct Unit {
    /// Item index in the token list.
    item: usize,
    /// Character index just past the unit within the item.
    end: usize,
    width: f32,
    /// How a line may end right after this unit.
    brk: Break,
}

/// Pushes one segment as a unit, subdividing it into single characters
/// with forced breaks when it alone exceeds the wrap width.
fn push_segment(
    units: &mut Vec<Unit>,
    item: &Item,
    index: usize,
    first: usize,
    last: usize,
    brk: Break,
    max_width: f32,
) {
    let width: f32 = item.widths[first..last].iter().sum();
    if width <= max_width || last - first <= 1 {
        units.push(Unit {
            item: index,
            end: last,
            width,
            brk,
        });
        return;
    }
    for c in first..last {
        units.push(Unit {
            item: index,
            end: c + 1,
            width: item.widths[c],
            brk: if c + 1 == last { brk } else { Break::Forced },
        });
    }
}

/// Characters after which an over-wide URL may break.
const URL_BREAK_AFTER: &[char] = &[
    '/', '.', '-', '_', '?', '&', '=', '#', '%', '+', '~', ':', ';', ',', '!',
];

/// Balances `items` to lines no wider than `max_width`, returning the
/// breaks, line count and natural width. `hyphen_width` is the advance of
/// the inserted `-` glyph.
pub fn balance(items: &[Item], max_width: f32, hyphen_width: f32) -> Plan {
    let mut units: Vec<Unit> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item.kind {
            Kind::Space => units.push(Unit {
                item: index,
                end: item.len(),
                width: item.width(),
                brk: Break::Free,
            }),
            Kind::Newline => units.push(Unit {
                item: index,
                end: item.len(),
                width: 0.0,
                brk: Break::Mandatory,
            }),
            Kind::Word => {
                let mut first = 0;
                for hyphen in &item.hyphens {
                    push_segment(
                        &mut units,
                        item,
                        index,
                        first,
                        hyphen.at,
                        Break::Hyphen {
                            joint: hyphen.joint,
                        },
                        max_width,
                    );
                    first = hyphen.at;
                }
                push_segment(
                    &mut units,
                    item,
                    index,
                    first,
                    item.len(),
                    Break::None,
                    max_width,
                );
            }
            Kind::Url => {
                // A URL that fits stays whole, so the bubble can widen to
                // it; a wider one breaks after punctuation only.
                if item.width() <= max_width {
                    units.push(Unit {
                        item: index,
                        end: item.len(),
                        width: item.width(),
                        brk: Break::None,
                    });
                } else {
                    let mut first = 0;
                    for (c, ch) in item.chars.iter().enumerate() {
                        if URL_BREAK_AFTER.contains(ch) && c + 1 < item.len() {
                            push_segment(
                                &mut units,
                                item,
                                index,
                                first,
                                c + 1,
                                Break::UrlPunct,
                                max_width,
                            );
                            first = c + 1;
                        }
                    }
                    push_segment(
                        &mut units,
                        item,
                        index,
                        first,
                        item.len(),
                        Break::None,
                        max_width,
                    );
                }
            }
        }
    }

    let n = units.len();
    if n == 0 {
        return Plan {
            breaks: Vec::new(),
            lines: 0,
            width: 0.0,
        };
    }
    // prefix[p] = total width of units[..p].
    let mut prefix = vec![0.0f32; n + 1];
    for (p, unit) in units.iter().enumerate() {
        prefix[p + 1] = prefix[p] + unit.width;
    }
    // A break is allowed after unit p-1 (consuming p units) when the unit
    // says so; the end of the text always allows it.
    let allowed = |p: usize| -> bool { p == 0 || p == n || units[p - 1].brk != Break::None };
    // line_width(p, q): units[p..q] as a line, excluding a trailing space,
    // including an inserted hyphen glyph.
    let line_width = |p: usize, q: usize| -> f32 {
        let mut width = prefix[q] - prefix[p];
        let last = &units[q - 1];
        if items[last.item].kind == Kind::Space {
            width -= last.width;
        }
        if last.brk.hyphen_glyph() {
            width += hyphen_width;
        }
        width
    };
    // Prefix count of mandatory breaks: the no-crossing check inside the
    // dynamic program stays O(1) per candidate instead of rescanning the
    // line, keeping the whole pass O(n^2).
    let mut mandatory = vec![0usize; n + 1];
    for (index, unit) in units.iter().enumerate() {
        mandatory[index + 1] = mandatory[index] + usize::from(unit.brk == Break::Mandatory);
    }
    // dp[p][h] = lowest cost laying out units[..p] ending in a break at p,
    // where h records whether that line ended on a hyphen.
    let mut dp = vec![[f32::INFINITY; 2]; n + 1];
    let mut parent = vec![[usize::MAX; 2]; n + 1];
    dp[0] = [0.0, f32::INFINITY];
    for q in 1..=n {
        if !allowed(q) {
            continue;
        }
        let last_line = q == n || units[q - 1].brk == Break::Mandatory;
        for p in 0..q {
            if !allowed(p) {
                continue;
            }
            // Never cross a mandatory break inside a line.
            if mandatory[q.saturating_sub(1)] - mandatory[p] > 0 {
                continue;
            }
            let width = line_width(p, q);
            let slack = max_width - width;
            let mut cost = if slack < 0.0 {
                slack * slack * OVERFLOW_PENALTY
            } else if last_line {
                (RUNT_SHARE * max_width - width).max(0.0).powi(2)
            } else {
                slack * slack
            };
            let brk = units[q - 1].brk;
            cost += brk.penalty();
            let ends_hyphen = brk.hyphen_glyph();
            for h in 0..2 {
                if dp[p][h].is_infinite() {
                    continue;
                }
                let mut total = dp[p][h] + cost;
                if h == 1 && ends_hyphen {
                    total += CONSECUTIVE_HYPHEN_PENALTY;
                }
                let hn = usize::from(ends_hyphen);
                if total < dp[q][hn] {
                    dp[q][hn] = total;
                    parent[q][hn] = p * 2 + h;
                }
            }
        }
    }
    let end = usize::from(dp[n][1] < dp[n][0]);
    debug_assert!(
        dp[n][end].is_finite(),
        "every text is wrappable through forced splits"
    );
    // Walk the parents back, collecting breaks and the widest line.
    let mut breaks = Vec::new();
    let mut width: f32 = 0.0;
    let mut lines = 0;
    let mut q = n;
    let mut h = end;
    while q > 0 {
        let packed = parent[q][h];
        assert!(packed != usize::MAX, "reachable state has a parent");
        let p = packed / 2;
        width = width.max(line_width(p, q));
        lines += 1;
        if p > 0 {
            let unit = &units[p - 1];
            breaks.push(LineBreak {
                // The next line starts right after the broken unit: past
                // the space, the hyphenation point, or the newline.
                byte: items[unit.item].byte_of(unit.end),
                hyphen: unit.brk.hyphen_glyph(),
            });
        }
        q = p;
        h = packed % 2;
    }
    breaks.reverse();
    Plan {
        breaks,
        lines,
        width,
    }
}

#[cfg(test)]
#[path = "wrap/tests.rs"]
mod tests;
