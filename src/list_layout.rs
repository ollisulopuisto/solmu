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
    let text = galley.job.text.clone();
    let sections = galley.job.sections.clone();
    let rows = &mut std::sync::Arc::make_mut(galley).rows;
    for placed in rows.iter_mut() {
        let Some(first) = placed.row.glyphs.first().map(|g| g.cluster as usize) else {
            continue;
        };
        if galley_sections_mono(&sections, first) {
            continue;
        }
        let start = text[..first]
            .char_indices()
            .rev()
            .find_map(|(at, ch)| {
                (ch == '\n' && !inserted.iter().any(|(a, n)| *a <= at && at < a + n))
                    .then_some(at + 1)
            })
            .unwrap_or(0);
        let end = text[start..].find('\n').map_or(text.len(), |n| start + n);
        if prefix_end(&text[start..end]).is_some_and(|prefix| first >= start + prefix) {
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
    sections
        .iter()
        .find(|section| section.byte_range.start.0 <= at && at < section.byte_range.end.0)
        .is_some_and(|section| section.format.font_id.family == egui::FontFamily::Monospace)
}
