//! List line identity used by hanging-indent layout. Never rewrites message text.

/// End byte of a visible list prefix, including indentation and its following
/// whitespace. Ordered markers require a separating space so versions, URLs and
/// decimal numbers remain ordinary prose.
pub fn prefix_end(line: &str) -> Option<usize> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let indent = line.len() - trimmed.len();
    let marker = ["•", "◦", "-", "*"]
        .into_iter()
        .find_map(|m| trimmed.strip_prefix(m).map(|rest| (m.len(), rest)))
        .or_else(|| {
            let end = trimmed.find(|c: char| !c.is_ascii_digit())?;
            if end == 0 || end > 18 {
                return None;
            }
            let rest = trimmed[end..].strip_prefix('.')?;
            Some((end + 1, rest))
        })?;
    if !marker.1.starts_with([' ', '\t']) {
        return None;
    }
    let rest = marker.1.trim_start_matches([' ', '\t']);
    if rest.is_empty() {
        return None;
    }
    Some(indent + marker.0 + marker.1.len() - rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefixes_cover_nested_bullets_and_wide_ordered_markers() {
        for (line, expected) in [
            ("•  first", 5),
            ("  - nested", 4),
            ("12. twelve", 4),
            ("1.\tfirst", 3),
        ] {
            assert_eq!(prefix_end(line), Some(expected), "{line}");
            assert!(!line[expected..].starts_with(char::is_whitespace));
        }
    }
    #[test]
    fn prose_decimals_empty_items_and_overlong_numbers_are_not_lists() {
        for line in [
            "1.2",
            "v1. next",
            "*bold*",
            "-",
            "• ",
            "1234567890123456789. long",
            "https://example.test/",
        ] {
            assert_eq!(prefix_end(line), None, "{line}");
        }
    }
}

/// Shift only wrapped continuation rows. `inserted` marks the balancer's added
/// newlines in byte coordinates, so real paragraph boundaries stay boundaries.
pub fn hanging(
    galley: &mut std::sync::Arc<egui::Galley>,
    inserted: &[(usize, usize)],
    indent: f32,
) {
    if indent <= 0.0 {
        return;
    }
    let job = galley.job.clone();
    let prefixes = paragraph_prefixes(&job.text, inserted);
    let rows = &mut std::sync::Arc::make_mut(galley).rows;
    for placed in rows.iter_mut() {
        let Some(first) = placed.row.glyphs.first().map(|g| g.cluster as usize) else {
            continue;
        };
        if galley_sections_mono(&job.sections, first) {
            continue;
        }
        // Visual rows need not have monotonically increasing glyph clusters.
        // Index logical paragraphs once, then look up each row independently.
        let index = prefixes
            .partition_point(|(start, _)| *start <= first)
            .saturating_sub(1);
        if prefixes[index].1.is_some_and(|content| first >= content) {
            placed.pos.x += indent;
        }
    }
    let galley = std::sync::Arc::make_mut(galley);
    galley.rect.max.x = galley
        .rows
        .iter()
        .map(|r| r.pos.x + r.row.size.x)
        .fold(galley.rect.width(), f32::max)
        + galley.rect.min.x;
    galley.mesh_bounds = galley.rows.iter().fold(egui::Rect::NOTHING, |bounds, row| {
        bounds.union(row.row.visuals.mesh_bounds.translate(row.pos.to_vec2()))
    });
}

/// One prefix lookup per real paragraph, never one backward scan per visual
/// row. Inserted byte ranges are sorted/nonoverlapping, as insert_breaks emits
/// them. Even an enormous single list item has only one logical prefix.
fn paragraph_prefixes(text: &str, inserted: &[(usize, usize)]) -> Vec<(usize, Option<usize>)> {
    // Preserve the old first-display-line check. If that line contains only
    // a marker and spaces, a shaping/balancing break must not turn it into
    // a newly indented item. This still scans only once per real paragraph.
    let prefix = |start: usize, end: usize| {
        prefix_end(text[start..end].split('\n').next().unwrap_or_default())
            .map(|prefix| start + prefix)
    };
    let mut prefixes = Vec::new();
    let mut start = 0;
    for (at, _) in text.match_indices('\n') {
        let index = inserted.partition_point(|(pos, len)| pos + len <= at);
        if inserted
            .get(index)
            .is_some_and(|(pos, len)| *pos <= at && at < pos + len)
        {
            continue;
        }
        prefixes.push((start, prefix(start, at)));
        start = at + 1;
    }
    prefixes.push((start, prefix(start, text.len())));
    prefixes
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    #[test]
    fn wrapped_rows_hang_under_content_without_changing_copy_text() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let source = "•  a list item long enough to wrap through several lines\nordinary prose";
            let mut galley = ui.painter().layout(
                source.into(),
                egui::FontId::proportional(16.0),
                egui::Color32::WHITE,
                125.0,
            );
            let text = galley.text().to_owned();
            let before: Vec<_> = galley.rows.iter().map(|r| r.pos.x).collect();
            hanging(&mut galley, &[], 18.0);
            assert_eq!(galley.text(), text);
            assert_eq!(galley.rows[0].pos.x, before[0]);
            assert!(galley.rows[1].pos.x >= before[1] + 18.0);
            let prose = galley
                .rows
                .iter()
                .position(|r| {
                    r.row
                        .glyphs
                        .first()
                        .is_some_and(|g| g.cluster as usize >= source.find("ordinary").unwrap())
                })
                .unwrap();
            assert_eq!(galley.rows[prose].pos.x, before[prose]);
        });
        output.textures_delta.clear();
    }
    #[test]
    fn balanced_inserted_newlines_do_not_end_a_list_item() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let text = "12. first\ncontinuation\nplain";
            let mut galley = ui.painter().layout_no_wrap(
                text.into(),
                egui::FontId::proportional(16.0),
                egui::Color32::WHITE,
            );
            hanging(&mut galley, &[(9, 1)], 26.0);
            assert_eq!(galley.rows[0].pos.x, 0.0);
            assert_eq!(galley.rows[1].pos.x, 26.0);
            assert_eq!(galley.rows[2].pos.x, 0.0);
        });
        output.textures_delta.clear();
    }
}

fn galley_sections_mono(sections: &[egui::text::LayoutSection], at: usize) -> bool {
    // LayoutJob sections are ordered ranges. Do not rescan all earlier spans
    // for every row of a richly formatted list item.
    let index = sections.partition_point(|section| section.byte_range.end.0 <= at);
    sections
        .get(index)
        .filter(|section| section.byte_range.start.0 <= at)
        .is_some_and(|section| section.format.font_id.family == egui::FontFamily::Monospace)
}

#[cfg(test)]
mod large_list_tests {
    use super::*;

    #[test]
    fn marker_only_display_row_keeps_legacy_continuation_alignment() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for marker in ["12. ", "- ", "•  "] {
                let text = format!("{marker}\nitem text\nordinary");
                for inserted in [vec![], vec![(marker.len(), 1)]] {
                    let mut galley = ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(16.0), egui::Color32::WHITE);
                    let before: Vec<_> = galley.rows.iter().map(|row| row.pos.x).collect();
                    hanging(&mut galley, &inserted, 26.0);
                    assert_eq!(galley.text(), text);
                    assert_eq!(galley.rows.iter().map(|row| row.pos.x).collect::<Vec<_>>(), before, "marker-only display row should not gain indentation: {marker:?}, {inserted:?}");
                }
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn display_breaks_keep_one_prefix_until_a_real_paragraph_boundary() {
        let text = format!("12. {}\ncontinued\nplain\n- second", "ää ".repeat(5000));
        let display = text.find("\ncontinued").unwrap();
        let plain = text.find("plain").unwrap();
        let second = text.find("- second").unwrap();
        assert_eq!(
            paragraph_prefixes(&text, &[(display, 1)]),
            vec![(0, Some(4)), (plain, None), (second, Some(second + 2))]
        );
    }

    #[test]
    fn large_wrapped_list_keeps_every_continuation_indent_and_copy_byte() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let text = format!("12. {}", "list words ".repeat(3000));
            let mut galley = ui.painter().layout(
                text.clone(),
                egui::FontId::proportional(16.0),
                egui::Color32::WHITE,
                140.0,
            );
            let before: Vec<_> = galley.rows.iter().map(|row| row.pos.x).collect();
            assert!(before.len() > 500);
            hanging(&mut galley, &[], 26.0);
            assert_eq!(galley.text(), text);
            assert_eq!(galley.rows[0].pos.x, before[0]);
            assert!(
                galley
                    .rows
                    .iter()
                    .zip(before)
                    .skip(1)
                    .all(|(row, x)| row.pos.x == x + 26.0)
            );
        });
        output.textures_delta.clear();
    }
}
