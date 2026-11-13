//! Tests for the balancing rules, encoded from the line-wrap design
//! sheets. Metrics are synthetic (10pt per character, 10pt hyphen), so the
//! expectations are exact and font-independent.

use super::*;

const PX: f32 = 10.0;

fn widths_of(text: &str) -> Vec<f32> {
    text.chars().map(|_| PX).collect()
}

fn no_hyphens(_: &str) -> Vec<Hyphen> {
    Vec::new()
}

/// A hyphenator scripted per word: (word, [(char index, joint)]).
fn scripted(points: Vec<(&'static str, Vec<(usize, bool)>)>) -> impl Fn(&str) -> Vec<Hyphen> {
    move |word| {
        points
            .iter()
            .find(|(w, _)| *w == word)
            .map(|(_, points)| {
                points
                    .iter()
                    .map(|&(at, joint)| Hyphen { at, joint })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn plan(text: &str, urls: &[(usize, usize)], hyph: &dyn Fn(&str) -> Vec<Hyphen>, cap: f32) -> Plan {
    let items = tokenize(text, &widths_of(text), urls, hyph);
    balance(&items, cap, PX)
}

fn hyphen_break_chars(text: &str, plan: &Plan) -> Vec<usize> {
    plan.breaks
        .iter()
        .filter(|b| b.hyphen)
        .map(|b| text[..b.byte].chars().count())
        .collect()
}

#[test]
fn url_that_fits_stays_whole_and_widens_the_bubble() {
    // Sheet 5 / redrawn 5b: a URL within the 560pt cap moves to its own
    // line and the bubble widens to it. 38-char URL = 380pt, cap 400.
    let url = "https://github.com/ollisulopuisto/solmu";
    let text = format!("kato {url} juttu");
    let urls = [(5, 5 + url.len())];
    let plan = plan(&text, &urls, &no_hyphens, 400.0);
    assert_eq!(plan.lines, 3);
    assert_eq!(
        plan.breaks,
        vec![
            LineBreak {
                byte: 5,
                hyphen: false
            },
            LineBreak {
                byte: 5 + url.len() + 1,
                hyphen: false
            },
        ]
    );
    assert_eq!(plan.width, 390.0);
}

#[test]
fn fitting_url_joins_the_line() {
    let url = "http://a.fi";
    let text = format!("kato {url} juttu");
    let urls = [(5, 5 + url.len())];
    let plan = plan(&text, &urls, &no_hyphens, 400.0);
    assert_eq!(plan.lines, 1);
    assert!(plan.breaks.is_empty());
}

#[test]
fn over_wide_url_breaks_at_punctuation_without_a_hyphen() {
    // Sheet 5 case 2: a URL past the cap breaks at punctuation, never with
    // an inserted hyphen.
    let url = "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/bb";
    let text = format!("kato {url}");
    let urls = [(5, 5 + url.len())];
    let plan = plan(&text, &urls, &no_hyphens, 200.0);
    assert!(plan.lines >= 3, "the over-wide URL takes several lines");
    let mut punct_breaks = 0;
    for brk in &plan.breaks {
        assert!(!brk.hyphen, "URLs never gain a hyphen glyph");
        if brk.byte > 5 && brk.byte < 5 + url.len() {
            let before = text.as_bytes()[brk.byte - 1] as char;
            if URL_BREAK_AFTER.contains(&before) {
                punct_breaks += 1;
            }
            // Otherwise a punctuation-free segment wider than the cap fell
            // back to a hyphenless character split, as greedy does today.
        }
    }
    assert!(punct_breaks > 0, "punctuation breaks are used when offered");
}

#[test]
fn unpunctuated_over_wide_url_char_splits_hyphenless() {
    // The greedy fallback: no punctuation, no hyphenation, split at
    // characters without a hyphen.
    let url = "https://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let panic_on_url = |word: &str| -> Vec<Hyphen> {
        panic!("URLs are never hyphenated: {word}");
    };
    let text = url.to_owned();
    let urls = [(0, url.len())];
    let plan = plan(&text, &urls, &panic_on_url, 100.0);
    // Breaks fall after the slashes (punctuation) or at characters, but
    // never carry a hyphen.
    assert!(plan.lines >= 3);
    assert!(plan.breaks.iter().all(|b| !b.hyphen));
}

#[test]
fn compound_words_break_at_joints_only() {
    // Sheet 4, decision (ii): compounds break at voikko joints only, even
    // when the hyphenator offers syllable points between them.
    let word = "kayttoliittymasuunnittelu"; // ASCII käyttöliittymäsuunnittelu
    let hyph = scripted(vec![(
        word,
        vec![(4, false), (6, true), (9, false), (14, true), (18, false)],
    )]);
    let text = format!("aa {word} bb");
    let plan = plan(&text, &[], &hyph, 150.0);
    let breaks = hyphen_break_chars(&text, &plan);
    assert!(!breaks.is_empty(), "the 250pt word must break at 150pt");
    for at in breaks {
        let within = at - 3; // "aa " precedes the word
        assert!(
            within == 6 || within == 14,
            "broke at {within}, not a joint (6 or 14)"
        );
    }
}

#[test]
fn non_compound_keeps_three_letter_fragments() {
    // Sheet 4, decision (ii): non-compounds hyphenate at any offered point
    // leaving >= 3 letters on both sides; "ti-lanteeseen" (2 letters) is
    // rejected.
    let word = "tilanteeseen"; // 12 chars
    let hyph = scripted(vec![(word, vec![(2, false), (5, false), (8, false)])]);
    let plan = plan(word, &[], &hyph, 70.0);
    assert_eq!(hyphen_break_chars(word, &plan), vec![5]);
    assert_eq!(plan.lines, 2);
}

#[test]
fn words_of_ten_letters_or_less_never_hyphenate() {
    let word = "aaaaaaaaaa"; // exactly 10: over the threshold nothing happens
    let hyph = scripted(vec![(word, vec![(4, false), (6, false)])]);
    let plan = plan(word, &[], &hyph, 50.0);
    assert_eq!(plan.lines, 2);
    assert!(plan.breaks.iter().all(|b| !b.hyphen));
}

#[test]
fn joint_breaks_are_free_and_preferred() {
    let word = "aaaaabbbbbccccc";
    let hyph = scripted(vec![(word, vec![(5, false), (10, true)])]);
    let plan = plan(word, &[], &hyph, 110.0);
    // The joint at 10 fills the line exactly and costs nothing.
    assert_eq!(hyphen_break_chars(word, &plan), vec![10]);
    assert_eq!(plan.width, 110.0);
}

#[test]
fn consecutive_hyphenated_lines_are_avoided() {
    let first = "wwwwwwwwwwwwwwwwwwww"; // 20
    let second = "xxxxxxxxxxxxxxxxxxxx"; // 20
    let hyph = scripted(vec![
        (first, vec![(10, false)]),
        (second, vec![(10, false)]),
    ]);
    let text = format!("{first} {second}");
    let plan = plan(&text, &[], &hyph, 110.0);
    // Two consecutive hyphenated lines would cost the flat 10000 demerit;
    // taking four lines is cheaper.
    assert_eq!(plan.lines, 4);
}

#[test]
fn a_hyphen_is_taken_to_avoid_a_ragged_line() {
    // The sheet-3 effect: hyphenating the long word fills the first line
    // (slack 0, one 2500 demerit) instead of leaving "aaaa" alone at 4900
    // slack² with the long word overflowing or wrapping whole.
    let word = "bbbbbbbbbbb"; // 11 chars
    let hyph = scripted(vec![(word, vec![(5, false)])]);
    let text = format!("aaaa {word} cc");
    let plan = plan(&text, &[], &hyph, 110.0);
    assert_eq!(plan.lines, 2);
    assert_eq!(hyphen_break_chars(&text, &plan), vec![10]);
}

#[test]
fn hard_newlines_are_kept() {
    let one = plan("aaa\nbb", &[], &no_hyphens, 100.0);
    assert_eq!(one.lines, 2);
    assert_eq!(
        one.breaks,
        vec![LineBreak {
            byte: 4,
            hyphen: false
        }]
    );
    let two = plan("a\n\nb", &[], &no_hyphens, 100.0);
    assert_eq!(two.lines, 3);
    let plan = plan("aaa\nbb", &[], &no_hyphens, 100.0);
    assert_eq!(plan.width, 30.0);
}

#[test]
fn space_runs_do_not_pad_the_width() {
    let plan = plan("aa  bb", &[], &no_hyphens, 100.0);
    assert_eq!(plan.lines, 1);
    assert_eq!(plan.width, 60.0);
}

#[test]
fn hyphen_glyph_counts_in_the_line_width() {
    let word = "tilanteeseen";
    let hyph = scripted(vec![(word, vec![(5, false)])]);
    let plan = plan(word, &[], &hyph, 70.0);
    // "tees een" (7 chars) is wider than "tilan-" (6 incl. the hyphen).
    assert_eq!(plan.width, 70.0);
}

#[test]
fn empty_text_has_no_lines() {
    let plan = plan("", &[], &no_hyphens, 100.0);
    assert_eq!(plan.lines, 0);
    assert_eq!(plan.width, 0.0);
}

#[test]
fn unbreakable_word_overflows_instead_of_vanishing() {
    // A single character past the cap: the line overflows rather than the
    // DP finding no solution.
    let plan = plan("aaa", &[], &no_hyphens, 20.0);
    assert_eq!(plan.lines, 1);
    assert_eq!(plan.width, 30.0);
}
