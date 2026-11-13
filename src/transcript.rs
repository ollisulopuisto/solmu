//! WhatsApp-style text for selections across messages:
//! `[22:41, 8/18/2026] Ada: message`.
//!
//! Each frame registers its message bodies and headers. The output hook adds
//! those headers when a selection spans multiple messages.

/// Rewrites copied message text in `output_hook`, after egui's selection hook.
pub struct CopyAnnotator {
    /// Message bodies drawn during the frame, in order.
    pub rows: std::sync::Arc<std::sync::Mutex<Vec<Row>>>,
}

impl egui::plugin::Plugin for CopyAnnotator {
    fn debug_name(&self) -> &'static str {
        "solmu-transcript"
    }

    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        let rows = self.rows.lock().unwrap_or_else(|p| p.into_inner());
        if rows.is_empty() {
            return;
        }
        for command in &mut output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command
                && let Some(refined) = refine(text, &rows)
            {
                *text = refined;
            }
        }
    }
}

/// One drawn message body with its copy header and emoji placeholders.
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub header: String,
    pub body: String,
    pub placements: Vec<String>,
    /// Non-text message description:
    /// "\[photo\]", "\[video\]", "\[document: notes.pdf\]", and so on.
    pub marker: Option<String>,
    /// Preformatted reactions, such as " (❤️ Roberta)".
    pub reactions: String,
    /// Preformatted reply context:
    /// `(replying to Roberta: "…")`.
    pub quote: Option<String>,
    /// The laid-out body with the wrap balancer's inserted hyphens and line
    /// breaks: what egui's selection actually copies. Set when it differs
    /// from `body`.
    pub broken: Option<String>,
    /// Byte ranges in `broken` holding the balancer's inserted characters
    /// (a newline, or a hyphen and newline, per break), dropped from output.
    pub inserted: Vec<(usize, usize)>,
}

impl Row {
    /// Formats one transcript line from its available parts.
    pub(crate) fn line(&self, body: &str) -> String {
        let mut line = self.header.clone();
        if let Some(quote) = &self.quote {
            line.push_str(quote);
            line.push(' ');
        }
        if let Some(marker) = &self.marker {
            line.push_str(marker);
            if !body.is_empty() {
                line.push(' ');
            }
        }
        line.push_str(body);
        line.push_str(&self.reactions);
        line
    }

    /// The text a selection copy matches against: the laid-out body when
    /// the balancer rewrote it, the plain body otherwise.
    fn match_text(&self) -> &str {
        self.broken.as_deref().unwrap_or(&self.body)
    }

    /// `restored` plus the balancer's inserted break characters removed:
    /// what the clipboard should carry. `start` is the segment's byte
    /// offset in the match text. Inserted ranges never cover placeholders.
    fn cleaned(&self, start: usize, segment: &str) -> String {
        if self.inserted.is_empty() {
            return self.restored(start, segment);
        }
        let mut next = self.match_text()[..start]
            .chars()
            .filter(|&c| c == crate::emoji::PLACEHOLDER)
            .count();
        let mut out = String::with_capacity(segment.len());
        for (offset, character) in segment.char_indices() {
            let at = start + offset;
            if self
                .inserted
                .iter()
                .any(|&(pos, len)| at >= pos && at < pos + len)
            {
                continue;
            }
            if character == crate::emoji::PLACEHOLDER {
                match self.placements.get(next) {
                    Some(emoji) => out.push_str(emoji),
                    None => out.push(character),
                }
                next += 1;
            } else {
                out.push(character);
            }
        }
        out
    }

    /// Returns the body from `start` with emoji placeholders restored.
    fn restored(&self, start: usize, segment: &str) -> String {
        if self.placements.is_empty() {
            return segment.to_owned();
        }
        let mut next = self.match_text()[..start]
            .chars()
            .filter(|&c| c == crate::emoji::PLACEHOLDER)
            .count();
        let mut out = String::with_capacity(segment.len());
        for character in segment.chars() {
            if character == crate::emoji::PLACEHOLDER {
                match self.placements.get(next) {
                    Some(emoji) => out.push_str(emoji),
                    None => out.push(character),
                }
                next += 1;
            } else {
                out.push(character);
            }
        }
        out
    }
}

/// Restores emoji and adds headers for multi-message selections. Returns
/// `None` for unrelated or unchanged copied text.
pub fn refine(copied: &str, rows: &[Row]) -> Option<String> {
    if let Some(annotated) = annotate(copied, rows) {
        return Some(annotated);
    }
    // Single-message selections need emoji restored and the balancer's
    // inserted hyphens and line breaks dropped.
    let needs_cleanup = copied.contains(crate::emoji::PLACEHOLDER)
        || rows.iter().any(|row| !row.inserted.is_empty());
    if !needs_cleanup {
        return None;
    }
    for row in rows {
        // A phrase can repeat inside one message: prefer the occurrence
        // whose span overlaps the balancer's inserted characters, since
        // those are what this cleanup strips. Without the preference a
        // literal "-\n" the sender typed in an earlier occurrence could be
        // cleaned as if it were the inserted one.
        let text = row.match_text();
        let mut first = None;
        let mut from = 0;
        while from <= text.len() {
            let Some(offset) = text[from..].find(copied) else {
                break;
            };
            let start = from + offset;
            let end = start + copied.len();
            if row
                .inserted
                .iter()
                .any(|&(pos, len)| pos < end && start < pos + len)
            {
                first = Some(start);
                break;
            }
            first = first.or(Some(start));
            from = start + copied.chars().next().map_or(1, char::len_utf8);
        }
        if let Some(start) = first {
            return Some(row.cleaned(start, copied));
        }
    }
    None
}

/// Adds a header per message when copied text spans multiple rows.
///
/// Matches egui's copied tail, complete middle bodies, and final head.
pub fn annotate(copied: &str, rows: &[Row]) -> Option<String> {
    for start in 0..rows.len() {
        if let Some(lines) = walk(copied, &rows[start..])
            && lines.len() >= 2
        {
            return Some(lines.join("\n"));
        }
    }
    None
}

/// Parses a copied tail followed by later rows into annotated lines.
fn walk(copied: &str, rows: &[Row]) -> Option<Vec<String>> {
    let row = rows.first()?;
    let text = row.match_text();
    // Try the longest tail first because the selection continues into the next row.
    for cut in 0..text.len() {
        if !text.is_char_boundary(cut) {
            continue;
        }
        let part = &text[cut..];
        let Some(rest) = copied.strip_prefix(part) else {
            continue;
        };
        let line = row.line(&row.cleaned(cut, part));
        if rest.is_empty() {
            return Some(vec![line]);
        }
        if let Some(mut tail) = follow(rest, &rows[1..]) {
            tail.insert(0, line);
            return Some(tail);
        }
    }
    None
}

/// Parses complete middle bodies and the final partial body after a separator.
/// Non-text rows crossed by the selection contribute their message kind.
fn follow(copied: &str, rows: &[Row]) -> Option<Vec<String>> {
    let remaining = copied
        .strip_prefix("\n\n")
        .or_else(|| copied.strip_prefix('\n'))?;
    if remaining.is_empty() {
        return None;
    }
    let mut passed: Vec<String> = Vec::new();
    for (skipped, row) in rows.iter().enumerate() {
        let text = row.match_text();
        if text.is_empty() {
            if row.marker.is_some() {
                passed.push(row.line(""));
            }
            continue;
        }
        // The remaining text is the final body's prefix.
        if text.starts_with(remaining) {
            passed.push(row.line(&row.cleaned(0, remaining)));
            return Some(passed);
        }
        // This complete body is followed by more copied text.
        if let Some(rest) = remaining.strip_prefix(text)
            && let Some(tail) = follow(rest, &rows[skipped + 1..])
        {
            passed.push(row.line(&row.cleaned(0, text)));
            passed.extend(tail);
            return Some(passed);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(header: &str, body: &str) -> Row {
        Row {
            header: header.to_owned(),
            body: body.to_owned(),
            placements: body
                .chars()
                .filter(|&c| c == crate::emoji::PLACEHOLDER)
                .map(|_| "🎉".to_owned())
                .collect(),
            ..Default::default()
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            row("[22:41, 8/18/2026] Carmine: ", "the analyzer is crazy good"),
            row(
                "[18:21, 8/30/2026] Ada: ",
                "Hello from France!\nSee you soon",
            ),
            row("[18:27, 8/30/2026] Carmine: ", "Sure I'll take a look"),
        ]
    }

    /// A phrase repeated in one message, once with a literal "-\n" the
    /// sender typed and once with the balancer's inserted hyphenation:
    /// cleanup targets the inserted occurrence, not the first match.
    #[test]
    fn refine_prefers_the_occurrence_overlapping_inserted_breaks() {
        let body = "käyttö-\nliittymä ja käyttöliittymä loppuun";
        let inserted_at = body.find("käyttöliittymä loppuun").unwrap() + "käyttö".len();
        let mut broken = body.to_owned();
        broken.insert_str(inserted_at, "-\n");
        let row = Row {
            body: body.to_owned(),
            broken: Some(broken),
            inserted: vec![(inserted_at, 2)],
            ..Default::default()
        };
        // The selection copied the balanced occurrence; the inserted "-\n"
        // disappears and the literal one in the first occurrence is untouched.
        assert_eq!(
            refine("käyttö-\nliittymä", &[row]).as_deref(),
            Some("käyttöliittymä")
        );
    }

    #[test]
    fn a_span_of_messages_gets_a_header_each() {
        let copied = "crazy good\nHello from France!\nSee you soon\nSure";
        assert_eq!(
            annotate(copied, &rows()).as_deref(),
            Some(
                "[22:41, 8/18/2026] Carmine: crazy good\n\
                 [18:21, 8/30/2026] Ada: Hello from France!\nSee you soon\n\
                 [18:27, 8/30/2026] Carmine: Sure"
            )
        );
        // egui may insert a blank line between bubbles.
        let gapped = "good\n\nHello from France!\nSee you soon";
        assert_eq!(
            annotate(gapped, &rows()).as_deref(),
            Some(
                "[22:41, 8/18/2026] Carmine: good\n\
                 [18:21, 8/30/2026] Ada: Hello from France!\nSee you soon"
            )
        );
    }

    #[test]
    fn balanced_breaks_match_and_drop_out_again() {
        // The balancer broke "the analyzer is crazy good" before "good" and
        // hyphenated "compoundword" after "compo": the selection copies the
        // broken text, the transcript drops the inserted characters.
        let mut first = row("[18:21, 8/30/2026] Ada: ", "the analyzer is crazy good");
        first.broken = Some("the analyzer is crazy \ngood".to_owned());
        first.inserted = vec![(22, 1)];
        let mut second = row("[18:22, 8/30/2026] Carmine: ", "a compoundword indeed");
        second.broken = Some("a compo-\nundword indeed".to_owned());
        second.inserted = vec![(7, 2)];
        let copied = "crazy \ngood\na compo-\nundword indeed";
        assert_eq!(
            annotate(copied, &[first.clone(), second]).as_deref(),
            Some(
                "[18:21, 8/30/2026] Ada: crazy good\n\
                 [18:22, 8/30/2026] Carmine: a compoundword indeed"
            )
        );
        // A single-message copy drops the break too.
        assert_eq!(
            refine("crazy \ngood", &[first]).as_deref(),
            Some("crazy good")
        );
    }

    #[test]
    fn within_one_message_nothing_changes() {
        assert_eq!(annotate("analyzer is crazy", &rows()), None);
        assert_eq!(annotate("the analyzer is crazy good", &rows()), None);
        assert_eq!(annotate("something else entirely", &rows()), None);
        assert_eq!(annotate("", &rows()), None);
        assert_eq!(annotate("anything", &[]), None);
    }

    #[test]
    fn emoji_come_back_out_of_their_placeholders() {
        let placeholder = crate::emoji::PLACEHOLDER;
        let rows = vec![Row {
            header: "[9:00, 1/2/2026] Ada: ".to_owned(),
            body: format!("well {placeholder} done {placeholder}"),
            placements: vec!["🎂".to_owned(), "🎈".to_owned()],
            ..Default::default()
        }];
        // Single-message copy: restore emoji without a header.
        assert_eq!(
            refine(&format!("{placeholder} done {placeholder}"), &rows).as_deref(),
            Some("🎂 done 🎈")
        );
        assert_eq!(
            refine(&format!("done {placeholder}"), &rows).as_deref(),
            Some("done 🎈"),
            "the offset counts placeholders before the selection"
        );
        assert_eq!(refine("well", &rows), None, "nothing to do");
        // Multi-message copy: restore emoji and add headers.
        let mut both = rows.clone();
        both.push(Row {
            header: "[9:01, 1/2/2026] Ada: ".to_owned(),
            body: "and again".to_owned(),
            placements: Vec::new(),
            ..Default::default()
        });
        assert_eq!(
            refine(&format!("done {placeholder}\nand again"), &both).as_deref(),
            Some("[9:00, 1/2/2026] Ada: done 🎈\n[9:01, 1/2/2026] Ada: and again")
        );
    }

    #[test]
    fn a_swept_over_picture_is_named_and_an_unknown_stays_silent() {
        let mut photo = row("[9:01, 1/2/2026] Ada: ", "");
        photo.marker = Some("[photo]".to_owned());
        let with_gap = vec![
            row("[9:00, 1/2/2026] Ada: ", "before the picture"),
            photo,
            row("[9:02, 1/2/2026] Ada: ", "after the picture"),
        ];
        let copied = "the picture\n\nafter the picture";
        assert_eq!(
            annotate(copied, &with_gap).as_deref(),
            Some(
                "[9:00, 1/2/2026] Ada: the picture\n[9:01, 1/2/2026] Ada: [photo]\n[9:02, 1/2/2026] Ada: after the picture"
            )
        );
        let silent = vec![
            row("[9:00, 1/2/2026] Ada: ", "before the picture"),
            row("[9:01, 1/2/2026] Ada: ", ""),
            row("[9:02, 1/2/2026] Ada: ", "after the picture"),
        ];
        assert_eq!(
            annotate(copied, &silent).as_deref(),
            Some("[9:00, 1/2/2026] Ada: the picture\n[9:02, 1/2/2026] Ada: after the picture")
        );
    }

    #[test]
    fn reactions_quotes_and_kinds_ride_along() {
        let mut first = row("[9:00, 1/2/2026] Ada: ", "look at this");
        first.marker = Some("[photo]".to_owned());
        first.reactions = " (❤️ You)".to_owned();
        let mut second = row("[9:05, 1/2/2026] You: ", "lovely");
        second.quote = Some("(replying to Ada: \"look at this\")".to_owned());
        let copied = "at this\nlovely";
        assert_eq!(
            annotate(copied, &[first, second]).as_deref(),
            Some(
                "[9:00, 1/2/2026] Ada: [photo] at this (❤️ You)\n[9:05, 1/2/2026] You: (replying to Ada: \"look at this\") lovely"
            )
        );
    }
}
