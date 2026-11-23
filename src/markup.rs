//! WhatsApp message markup for egui.
//!
//! `*bold*`, `_italic_`, `~struck~`, `` `mono` ``, fenced blocks, `> `
//! quotes, `* ` lists, links, phone numbers, email addresses, and named
//! `@mentions`. Emoji go through [`crate::emoji`].

use std::ops::Range;
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Color32, FontId, Galley, Pos2, Rect, Stroke, TextFormat};
use fastframe_text::snap_to_pixels;

use crate::bidi;
use crate::emoji;
use crate::theme;

/// Balanced wrapping is quadratic in the number of break opportunities. Large
/// pasted logs use egui's ordinary greedy wrapping instead, including during
/// width planning. This is a work limit, not a message or copy limit.
pub const BALANCED_BODY_MAX_BYTES: usize = 8 * 1024;

/// WhatsApp mention id and display name.
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub user: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub size: f32,
    pub color: Color32,
    pub secondary: Color32,
    pub link: Color32,
    pub mention: Color32,
}

/// Message text prepared for painting.
/// What decorates one laid-out row: a fenced code panel, a quote panel,
/// or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Ordinary body text.
    Normal,
    /// A line inside a fenced code block: the block's index into
    /// [`code_blocks`], so two adjacent fences stay two panels and each
    /// Copy code button reaches its own block.
    Code(usize),
    /// A `> ` quoted line.
    Quote,
}

pub struct Text {
    pub galley: Arc<Galley>,
    /// Character ranges linked to web addresses.
    pub links: Vec<(Range<usize>, String)>,
    /// Whether the message is emoji-only and should use a larger size.
    pub big: bool,
    /// Everything else about the layout, shared so a cached body rebuilds
    /// its frame's `Text` (a fresh egui galley) without cloning these.
    shared: Arc<Shared>,
}

/// The parts of a prepared body that survive a font reload: everything but
/// the galley, which egui re-shapes from the job every pass.
struct Shared {
    placements: Vec<String>,
    list_indent: f32,
    /// The link ranges in `unbroken_text` coordinates: what copy should
    /// read. Equals `links` for a greedy layout.
    copy_links: Vec<(Range<usize>, String)>,
    accessible_text: String,
    /// Balanced layout only: the laid-out text before the balancer's
    /// inserted hyphens and newlines, for copy and transcript formatting.
    unbroken: Option<String>,
    /// Byte ranges in the galley text holding the balancer's inserted
    /// characters (a "\n" or "-\n" per break), so transcript formatting
    /// can drop them.
    inserted: Vec<(usize, usize)>,
    /// Per galley row: which rows belong to a fenced code block or a quote,
    /// so the painter can give them an inset panel. Rows survive a font
    /// reload like everything else here: a balanced body is broken into
    /// hard lines with unlimited wrap, so its row structure does not depend
    /// on font metrics, and the greedy path finishes fresh every call.
    row_kinds: Vec<RowKind>,
}

/// Reserve an explicit code header and inset without adding copyable spaces or
/// blank lines. Both cached and fresh layouts use these same row coordinates.
fn spaced_code_galley(mut galley: Arc<Galley>, kinds: &[RowKind]) -> Arc<Galley> {
    if !kinds.iter().any(|kind| matches!(kind, RowKind::Code(_))) {
        return galley;
    }
    let galley_mut = Arc::make_mut(&mut galley);
    let mut shift = 0.0;
    let mut width = galley_mut
        .rect
        .width()
        .max(100.0_f32.min(galley_mut.job.wrap.max_width + 20.0));
    let mut previous = RowKind::Normal;
    for (index, row) in galley_mut.rows.iter_mut().enumerate() {
        let kind = kinds[index];
        if matches!(previous, RowKind::Code(_)) && kind != previous {
            shift += 10.0;
        }
        if matches!(kind, RowKind::Code(_)) && kind != previous {
            shift += 20.0;
        }
        row.pos.y += shift;
        if matches!(kind, RowKind::Code(_)) {
            row.pos.x += 10.0;
            width = width.max(row.pos.x + row.row.size.x + 10.0);
        }
        previous = kind;
    }
    if matches!(previous, RowKind::Code(_)) {
        shift += 10.0;
    }
    galley_mut.rect.max.y += shift;
    galley_mut.rect.max.x = galley_mut.rect.min.x + width;
    galley_mut.mesh_bounds = galley_mut.rows.iter().fold(Rect::NOTHING, |bounds, row| {
        bounds.union(row.row.visuals.mesh_bounds.translate(row.pos.to_vec2()))
    });
    galley
}

impl Text {
    /// Emoji sequences represented by placeholder glyphs.
    pub fn placements(&self) -> &[String] {
        &self.shared.placements
    }

    /// Returns the link at a character index.
    pub fn link_at(&self, character: usize) -> Option<&str> {
        self.links
            .iter()
            .find(|(range, _)| range.contains(&character))
            .map(|(_, url)| url.as_str())
    }

    /// The text without the balancer's inserted hyphens and line breaks:
    /// what copy and transcript formatting should see. Equals the galley's
    /// text for a greedy layout.
    pub fn unbroken_text(&self) -> &str {
        self.shared
            .unbroken
            .as_deref()
            .unwrap_or_else(|| self.galley.text())
    }

    /// The text a link covers, read from the unbroken text: what copying
    /// the link puts on the clipboard. Indexed like `links`.
    pub fn link_copy_text(&self, index: usize) -> String {
        let (range, _) = &self.shared.copy_links[index];
        self.unbroken_text()
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect()
    }

    /// The galley text when it carries the balancer's inserted breaks and
    /// so differs from `unbroken_text`: what a drag selection copies.
    pub fn broken_text(&self) -> Option<&str> {
        self.shared.unbroken.is_some().then(|| self.galley.text())
    }

    /// Byte ranges in the galley text holding the balancer's inserted
    /// characters. Empty for a greedy layout.
    pub fn inserted(&self) -> &[(usize, usize)] {
        &self.shared.inserted
    }

    /// Which decoration each galley row gets, aligned to `galley.rows`.
    pub fn row_kinds(&self) -> &[RowKind] {
        &self.shared.row_kinds
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    mono: bool,
    /// A fenced code block's line, as opposed to inline `code`.
    block: bool,
    /// The fenced block's index into [`code_blocks`]: every content line of
    /// one fence shares it, adjacent fences stay distinct, and empty fences
    /// (which [`code_blocks`] drops) consume no index. Meaningful only when
    /// `block` is true.
    block_id: usize,
    link: Option<String>,
    mention: bool,
    /// Paints a quoted line's bar and indent.
    quote: bool,
}

/// Size multiplier for an emoji-only message: one emoji largest, two or
/// three big, anything else body size. Bubbles drop at the same threshold
/// (`bubbleless` in ui::conversation), like a sticker.
fn emoji_scale(text: &str) -> f32 {
    match emoji::only_emoji(text) {
        Some(1) => 3.0,
        Some(2..=3) => 2.4,
        _ => 1.0,
    }
}

/// The parts of a laid-out message before the galley.
struct Built {
    job: LayoutJob,
    list_indent: f32,
    placements: Vec<String>,
    links: Vec<(Range<usize>, String)>,
    big: bool,
    accessible_text: String,
    /// Character ranges of fenced code block lines in the job text, each
    /// paired with its block's index into [`code_blocks`]. One fence emits
    /// one range per content line, so the index, not the range position,
    /// identifies the block.
    blocks: Vec<(usize, Range<usize>)>,
    /// Character ranges of quoted lines in the job text.
    quotes: Vec<Range<usize>>,
}

/// Lays text out within `max_width`. `grow_emoji` is false for text that
/// rides inside another message's bubble, like a media caption: a caption
/// that happens to be one emoji stays at body size, because the growth (and
/// the bubble drop that pairs with it) belongs to standalone messages only.
pub fn layout(
    ui: &egui::Ui,
    text: &str,
    mentions: &[Mention],
    style: &Style,
    max_width: f32,
    grow_emoji: bool,
) -> Text {
    finish(
        ui,
        build(ui, text, mentions, style, max_width, grow_emoji),
        None,
        None,
        Vec::new(),
    )
}

fn finish(
    ui: &egui::Ui,
    built: Built,
    unbroken: Option<String>,
    copy_links: Option<Vec<(Range<usize>, String)>>,
    inserted: Vec<(usize, usize)>,
) -> Text {
    // The block and quote ranges are character-indexed; galley sections use
    // byte ranges, so convert with the final job text.
    let mut galley = bidi::layout_job(ui, built.job);
    crate::list_layout::hanging(&mut galley, &inserted, built.list_indent);
    let row_kinds = row_kinds(&galley, &built.blocks, &built.quotes);
    let galley = spaced_code_galley(galley, &row_kinds);
    let copy_links = copy_links.unwrap_or_else(|| built.links.clone());
    Text {
        galley,
        links: built.links,
        big: built.big,
        shared: Arc::new(Shared {
            placements: built.placements,
            list_indent: built.list_indent,
            copy_links,
            accessible_text: built.accessible_text,
            unbroken,
            inserted,
            row_kinds,
        }),
    }
}

/// Per galley row: which rows belong to a fenced code block or a quote, so
/// the painter can give them an inset panel. The ranges are
/// character-indexed in the galley's text; each glyph's `cluster` is the
/// byte offset of its source character, so convert with that text.
///
/// Fence markers paint no row of their own: parse drops the ``` lines, so
/// each leaves a blank row behind. A blank row carries no glyphs and
/// cannot be hit-tested by cluster, so it is placed by the character
/// position of its newline against the block's content bounds: the blank
/// lines between a fence's content lines fall inside, the dropped markers'
/// rows fall outside and stay Normal.
fn list_indent(ui: &egui::Ui, job: &LayoutJob, size: f32, cap: f32) -> f32 {
    let text = &job.text;
    if crate::bidi::message_rtl(text) {
        return 0.0;
    }
    text.split_inclusive('\n')
        .scan(0, |start, line| {
            let at = *start;
            *start += line.len();
            Some((at, line))
        })
        .filter(|(start, _)| {
            !job.sections.iter().any(|section| {
                section.byte_range.start.0 <= *start
                    && *start < section.byte_range.end.0
                    && section.format.font_id.family == egui::FontFamily::Monospace
            })
        })
        .filter_map(|(_, line)| crate::list_layout::prefix_end(line).map(|end| &line[..end]))
        .map(|prefix| {
            ui.painter()
                .layout_no_wrap(prefix.into(), theme::regular(size), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        .min(cap.max(0.0) / 3.0)
}

fn row_kinds(
    galley: &Galley,
    blocks: &[(usize, Range<usize>)],
    quotes: &[Range<usize>],
) -> Vec<RowKind> {
    let text = &galley.job.text;
    let char_bytes: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain([text.len()])
        .collect();
    let to_byte = |char: usize| char_bytes[char.min(char_bytes.len() - 1)];
    let byte_to_char = |byte: usize| char_bytes.partition_point(|b| *b < byte);
    // Content bounds per block, character-indexed. The end is inclusive
    // for the blank-row check: an empty content line is exactly its span's
    // empty range, and its terminating newline sits at that end.
    let mut bounds: Vec<(usize, usize, usize)> = Vec::new();
    for (id, range) in blocks {
        match bounds.iter_mut().find(|(b, _, _)| b == id) {
            Some((_, start, end)) => {
                *start = (*start).min(range.start);
                *end = (*end).max(range.end);
            }
            None => bounds.push((*id, range.start, range.end)),
        }
    }
    // Rows partition the galley text in order; track the character
    // position each row starts at so blank rows can be placed.
    let mut cursor = 0usize;
    galley
        .rows
        .iter()
        .map(|row| {
            if row.row.glyphs.is_empty() {
                // A blank line is exactly its terminating newline.
                let at = cursor;
                cursor += 1;
                return bounds
                    .iter()
                    .find(|(_, start, end)| *start <= at && at <= *end)
                    .map_or(RowKind::Normal, |(id, _, _)| RowKind::Code(*id));
            }
            let mut lo = usize::MAX;
            let mut hi = 0usize;
            for glyph in &row.row.glyphs {
                let offset = glyph.cluster as usize;
                lo = lo.min(offset);
                hi = hi.max(offset + glyph.chr.len_utf8());
            }
            cursor = byte_to_char(hi) + usize::from(row.ends_with_newline);
            let hit = |start: usize, end: usize| lo < end && start < hi;
            if let Some((index, _)) = blocks
                .iter()
                .find(|(_, range)| hit(to_byte(range.start), to_byte(range.end)))
            {
                RowKind::Code(*index)
            } else if quotes
                .iter()
                .any(|range| hit(to_byte(range.start), to_byte(range.end)))
            {
                RowKind::Quote
            } else {
                RowKind::Normal
            }
        })
        .collect()
}

fn build(
    ui: &egui::Ui,
    text: &str,
    mentions: &[Mention],
    style: &Style,
    max_width: f32,
    grow_emoji: bool,
) -> Built {
    let scale = if grow_emoji { emoji_scale(text) } else { 1.0 };
    let big = scale > 1.0;
    let size = style.size * scale;
    let mut job = LayoutJob::default();
    job.wrap.max_width = max_width;
    let mut placements = Vec::new();
    let mut links = Vec::new();
    let mut blocks = Vec::new();
    let mut quotes = Vec::new();
    let mut characters = 0;
    for span in display_spans(parse(text, mentions)) {
        let font_id = if span.mono {
            FontId::monospace(size * 0.95)
        } else if span.bold || span.mention {
            theme::bold(size)
        } else {
            theme::regular(size)
        };
        let color = if span.link.is_some() {
            style.link
        } else if span.mention {
            style.mention
        } else if span.quote && span.text.starts_with('▎') {
            style.secondary
        } else {
            style.color
        };
        let format = TextFormat {
            font_id,
            line_height: span.block.then_some(size * 1.4),
            color,
            italics: span.italic,
            underline: if span.link.is_some() {
                Stroke::new(1.0, style.link)
            } else {
                Stroke::NONE
            },
            strikethrough: if span.strike {
                Stroke::new(1.0, color)
            } else {
                Stroke::NONE
            },
            // Inline `code` gets a quiet inset; fenced blocks paint a panel
            // per row instead (see RowKind).
            background: if span.mono && !span.block {
                style.secondary.gamma_multiply(0.12)
            } else {
                Color32::TRANSPARENT
            },
            ..Default::default()
        };
        let before = characters;
        let after = before + emoji::append(ui, &mut job, &mut placements, &span.text, &format);
        if let Some(url) = span.link {
            links.push((before..after, url));
        }
        if span.block {
            blocks.push((span.block_id, before..after));
        }
        if span.quote {
            quotes.push(before..after);
        }
        characters = after;
    }
    if job.text.is_empty() {
        job.append(
            " ",
            0.0,
            TextFormat::simple(theme::regular(size), style.color),
        );
    }
    let list_indent = list_indent(ui, &job, size, max_width);
    // Reserve both decorations before either greedy shaping or balancing.
    // The previous assignment replaced the code inset with the list cap.
    let code_inset = if blocks.is_empty() { 0.0 } else { 20.0 };
    job.wrap.max_width = (max_width - list_indent - code_inset).max(1.0);
    Built {
        job,
        list_indent,
        placements,
        links,
        big,
        accessible_text: plain(text, mentions),
        blocks,
        quotes,
    }
}

/// Body text measured for the wrap balancer: tokens with per-character
/// widths plus the hyphen glyph's advance.
pub struct Measured {
    /// The text's tokens, ready for `crate::wrap::balance` (private, so no doc link).
    pub items: Vec<crate::wrap::Item>,
    /// Advance width of an inserted `-` at body size.
    pub hyphen: f32,
}

/// Measures `text` for balancing at `max_width` without laying it out.
pub fn measure(
    ui: &egui::Ui,
    text: &str,
    mentions: &[Mention],
    style: &Style,
    max_width: f32,
    grow_emoji: bool,
) -> Measured {
    let built = build(ui, text, mentions, style, max_width, grow_emoji);
    measure_job(ui, &built.job, &built.links, style.size, max_width)
}

/// Splits the job text into balancer tokens. Token widths are exact
/// (shaped per section piece); per-character widths are shaped for the
/// tokens that can break inside (long words with hyphenation points,
/// anything wider than the cap) and interpolated for the rest, whose
/// interiors the balancer never inspects.
fn measure_job(
    ui: &egui::Ui,
    job: &LayoutJob,
    links: &[(Range<usize>, String)],
    size: f32,
    max_width: f32,
) -> Measured {
    let text = job.text.as_str();
    let n_chars = text.chars().count();
    // Byte offset of each character plus the end, for char<->byte mapping.
    let char_bytes: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain([text.len()])
        .collect();
    // URL byte spans: link ranges whose visible text is a bare address.
    let urls: Vec<(usize, usize)> = links
        .iter()
        .filter(|(range, _)| {
            let start = char_bytes.get(range.start).copied().unwrap_or(text.len());
            let end = char_bytes.get(range.end).copied().unwrap_or(text.len());
            let shown = &text[start..end];
            shown.contains("://") || shown.starts_with("www.")
        })
        .map(|(range, _)| {
            (
                char_bytes.get(range.start).copied().unwrap_or(text.len()),
                char_bytes.get(range.end).copied().unwrap_or(text.len()),
            )
        })
        .collect();
    // Section font per byte offset.
    let font_at = |byte: usize| -> FontId {
        job.sections
            .iter()
            .find(|section| {
                let start: usize = section.byte_range.start.into();
                let end: usize = section.byte_range.end.into();
                start <= byte && byte < end
            })
            .map(|section| section.format.font_id.clone())
            .unwrap_or_else(|| theme::regular(size))
    };
    let piece_width = |from: usize, to: usize, font: &FontId| -> f32 {
        if from >= to {
            return 0.0;
        }
        ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text[from..to].to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        })
    };
    let mut widths = vec![0.0f32; n_chars];
    let mut pos = 0;
    while pos < n_chars {
        let start = char_bytes[pos];
        let first = text[start..].chars().next().unwrap();
        let mut next = pos + 1;
        if !matches!(first, '\n' | '\r') {
            while next < n_chars {
                let c = text[char_bytes[next]..].chars().next().unwrap();
                if c.is_whitespace() != first.is_whitespace() || matches!(c, '\n' | '\r') {
                    break;
                }
                next += 1;
            }
        }
        let end = char_bytes[next];
        // Measure the token in section pieces.
        let mut cuts: Vec<usize> = vec![start, end];
        for section in &job.sections {
            for b in [
                usize::from(section.byte_range.start),
                usize::from(section.byte_range.end),
            ] {
                if b > start && b < end && !cuts.contains(&b) {
                    cuts.push(b);
                }
            }
        }
        cuts.sort_unstable();
        let mut pieces: Vec<(usize, usize, FontId, f32)> = Vec::new();
        for pair in cuts.windows(2) {
            let (b0, b1) = (pair[0], pair[1]);
            let font = font_at(b0);
            let width = piece_width(b0, b1, &font);
            pieces.push((b0, b1, font, width));
        }
        let token_chars = next - pos;
        let token_width: f32 = pieces.iter().map(|p| p.3).sum();
        let is_url = urls.iter().any(|(s, e)| *s <= start && end <= *e);
        // Code never hyphenates: an inserted "-" would paint as code that
        // is not in the text (and at the code font's width, which the
        // planner's regular-font hyphen width would undercount).
        let mono = font_at(start).family == egui::FontFamily::Monospace;
        let hyphenated = !is_url
            && !mono
            && !first.is_whitespace()
            && token_chars > crate::wrap::MIN_HYPHENATED_CHARS
            && !text[start..end].contains(emoji::PLACEHOLDER)
            && !crate::hyphenate::hyphenate(&text[start..end]).is_empty();
        if token_width > max_width || hyphenated {
            // Per-character widths from cumulative piece prefixes.
            let mut ci = pos;
            for (b0, b1, font, _) in &pieces {
                let piece = &text[*b0..*b1];
                let mut prev = 0.0f32;
                for (k, ch) in piece.char_indices() {
                    let w = piece_width(*b0, b0 + k + ch.len_utf8(), font);
                    widths[ci] = (w - prev).max(0.0);
                    prev = w;
                    ci += 1;
                }
            }
        } else {
            let share = token_width / token_chars.max(1) as f32;
            for w in &mut widths[pos..next] {
                *w = share;
            }
        }
        pos = next;
    }
    let items = crate::wrap::tokenize(text, &widths, &urls, &|at, word| {
        if word.contains(emoji::PLACEHOLDER) || font_at(at).family == egui::FontFamily::Monospace {
            Vec::new()
        } else {
            crate::hyphenate::hyphenate(word)
        }
    });
    let hyphen = ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap("-".to_owned(), theme::regular(size), Color32::WHITE)
            .size()
            .x
    });
    Measured { items, hyphen }
}

/// What a prepared balanced layout depends on: the text and resolved
/// mention names (hashed, then verified against the stored inputs before a
/// hit is served), body size, wrap width, colours, the emoji sizing switch
/// and the font generation, which [`crate::theme::set_font`] bumps on a
/// typeface change.
#[derive(Clone, PartialEq, Eq, Hash)]
struct PreparedKey {
    text_hash: u64,
    mentions_hash: u64,
    size: u32,
    max_width: u32,
    colors: [u32; 4],
    grow_emoji: bool,
    font_generation: u32,
    pixels_per_point: u32,
}

impl PreparedKey {
    fn new(
        text: &str,
        mentions: &[Mention],
        style: &Style,
        max_width: f32,
        grow_emoji: bool,
        pixels_per_point: f32,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let text_hash = hasher.finish();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for mention in mentions {
            mention.user.hash(&mut hasher);
            mention.name.hash(&mut hasher);
        }
        let mentions_hash = hasher.finish();
        let color = |color: Color32| u32::from_ne_bytes(color.to_srgba_unmultiplied());
        Self {
            text_hash,
            mentions_hash,
            size: style.size.to_bits(),
            max_width: max_width.to_bits(),
            colors: [
                color(style.color),
                color(style.secondary),
                color(style.link),
                color(style.mention),
            ],
            grow_emoji,
            font_generation: crate::theme::font_generation(),
            pixels_per_point: pixels_per_point.to_bits(),
        }
    }
}

/// Bound a large body's shaping paragraphs without breaking a Unicode grapheme.
/// These display-only newlines use the same offset/copy mapping as balanced
/// breaks. Existing short log lines do not change at all.
fn bounded_shape_breaks(text: &str) -> Vec<crate::wrap::LineBreak> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut breaks = Vec::new();
    let mut count = 0;
    for (byte, cluster) in text.grapheme_indices(true) {
        if cluster.contains(['\n', '\r']) {
            count = 0;
            continue;
        }
        if count == 1024 {
            breaks.push(crate::wrap::LineBreak {
                byte,
                hyphen: false,
            });
            count = 0;
        }
        count += 1;
    }
    breaks
}

/// A prepared body: the final job and everything around it, but no galley.
/// The galley comes from egui's own cache on every pass (`draw`), so painted
/// glyphs always match the live font atlas, across reloads and resets. The
/// raw inputs ride along so a hash collision in [`PreparedKey`] can never
/// serve another body's layout.
struct Prepared {
    text: Box<str>,
    mentions: Box<[Mention]>,
    job: LayoutJob,
    links: Vec<(Range<usize>, String)>,
    big: bool,
    shared: Arc<Shared>,
}

impl Prepared {
    fn build(
        ui: &egui::Ui,
        text: &str,
        mentions: &[Mention],
        style: &Style,
        max_width: f32,
        grow_emoji: bool,
    ) -> Self {
        let built = build(ui, text, mentions, style, max_width, grow_emoji);
        let whole = |built: Built| {
            let galley = bidi::layout_job(ui, built.job.clone());
            let row_kinds = row_kinds(&galley, &built.blocks, &built.quotes);
            Self {
                text: text.into(),
                mentions: mentions.into(),
                job: built.job,
                links: built.links.clone(),
                big: built.big,
                shared: Arc::new(Shared {
                    placements: built.placements,
                    list_indent: built.list_indent,
                    copy_links: built.links,
                    accessible_text: built.accessible_text,
                    unbroken: None,
                    inserted: Vec::new(),
                    row_kinds,
                }),
            }
        };
        if text.len() > BALANCED_BODY_MAX_BYTES {
            // Some shapers also become expensive on one enormous paragraph.
            // Partition only the display job, never stored/copied content.
            let breaks = bounded_shape_breaks(&built.job.text);
            if breaks.is_empty() {
                return whole(built);
            }
            let unbroken = built.job.text.clone();
            let copy_links = built.links.clone();
            let (job, links, blocks, quotes, inserted) = insert_breaks(
                built.job,
                &built.links,
                &built.blocks,
                &built.quotes,
                &breaks,
            );
            let galley = bidi::layout_job(ui, job.clone());
            let row_kinds = row_kinds(&galley, &blocks, &quotes);
            return Self {
                text: text.into(),
                mentions: mentions.into(),
                job,
                links,
                big: built.big,
                shared: Arc::new(Shared {
                    placements: built.placements,
                    list_indent: built.list_indent,
                    copy_links,
                    accessible_text: built.accessible_text,
                    unbroken: Some(unbroken),
                    inserted,
                    row_kinds,
                }),
            };
        }
        if built.big || max_width <= 0.0 {
            // Emoji-only text never balances; a zero cap leaves nothing to do.
            return whole(built);
        }
        let cap = built.job.wrap.max_width;
        let breaks = balanced_breaks(ui, &built, style.size, cap);
        if breaks.is_empty() {
            return whole(built);
        }
        let unbroken = built.job.text.clone();
        let copy_links = built.links.clone();
        let (mut job, links, blocks, quotes, inserted) = insert_breaks(
            built.job,
            &built.links,
            &built.blocks,
            &built.quotes,
            &breaks,
        );
        // Prose keeps its planned breaks. Fenced code must also retain a
        // hard shaping cap: token measurements can omit trailing whitespace,
        // but those glyphs still take room before the panel's right inset.
        if blocks.is_empty() {
            job.wrap.max_width = f32::INFINITY;
        } else {
            job.wrap.max_width = cap;
        }
        let galley = bidi::layout_job(ui, job.clone());
        let row_kinds = row_kinds(&galley, &blocks, &quotes);
        Self {
            text: text.into(),
            mentions: mentions.into(),
            job,
            links,
            big: built.big,
            shared: Arc::new(Shared {
                placements: built.placements,
                list_indent: built.list_indent,
                copy_links,
                accessible_text: built.accessible_text,
                unbroken: Some(unbroken),
                inserted,
                row_kinds,
            }),
        }
    }

    /// Shapes this pass's galley. egui caches galleys by job and drops them
    /// when the atlas changes, so a font reload never paints stale glyphs.
    fn draw(&self, ui: &egui::Ui) -> Text {
        Text {
            galley: {
                let mut galley = bidi::layout_job(ui, self.job.clone());
                crate::list_layout::hanging(
                    &mut galley,
                    &self.shared.inserted,
                    self.shared.list_indent,
                );
                spaced_code_galley(galley, &self.shared.row_kinds)
            },
            links: self.links.clone(),
            big: self.big,
            shared: self.shared.clone(),
        }
    }
}

/// Prepared bodies plus the font generation they were built under. A
/// generation change clears the map: the old jobs name the old fonts.
#[derive(Default)]
struct PreparedCache {
    generation: u32,
    entries: std::collections::HashMap<PreparedKey, Arc<Prepared>>,
}

/// How many prepared bodies are cached. Covers a long chat's loaded set;
/// past the cap new bodies are laid out fresh every frame rather than
/// evicting, because evicting under a full-timeline scan (an active text
/// selection lays out every loaded row) thrashes the cache into permanent
/// misses. An over-cap chat keeps a stable cached subset instead.
const PREPARED_CAP: usize = 1024;

// Prepared jobs depend on this context's font definitions and measurements.
// Separate contexts must not share entries or compete for admission slots.
fn prepared_cache(ctx: &egui::Context) -> Arc<std::sync::Mutex<PreparedCache>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<std::sync::Mutex<PreparedCache>>>(egui::Id::new(
            "solmu-prepared-body-cache",
        ))
        .clone()
    })
}

/// Lays text out like [`layout`], but with line breaks from the wrap
/// balancer (`crate::wrap`, private, so no doc link) instead of greedy first-fit: raggedness is
/// minimised, Finnish words may hyphenate, and URLs stay whole when they
/// fit the cap. The inserted hyphens and newlines are layout-time only;
/// [`Text::unbroken_text`] carries the text without them for copying.
///
/// A body is prepared once per input set and then served from a cache: the
/// conversation view calls this every frame for every visible bubble while
/// texts, widths and fonts rarely change. An edit, a renamed mention, a new
/// width or a new typeface misses and is prepared again. The galley is
/// re-shaped from the cached job on every call (through egui's cache), so
/// painted glyphs track the live font atlas.
pub fn layout_balanced(
    ui: &egui::Ui,
    text: &str,
    mentions: &[Mention],
    style: &Style,
    max_width: f32,
    grow_emoji: bool,
) -> Arc<Text> {
    let key = PreparedKey::new(
        text,
        mentions,
        style,
        max_width,
        grow_emoji,
        ui.pixels_per_point(),
    );
    let cache = prepared_cache(ui.ctx());
    {
        let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        if cache.generation != key.font_generation {
            cache.entries.clear();
            cache.generation = key.font_generation;
        }
        if let Some(prepared) = cache.entries.get(&key)
            && prepared.text.as_ref() == text
            && prepared.mentions.as_ref() == mentions
        {
            return Arc::new(prepared.draw(ui));
        }
    }
    let prepared = Prepared::build(ui, text, mentions, style, max_width, grow_emoji);
    let laid = Arc::new(prepared.draw(ui));
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    if cache.entries.len() < PREPARED_CAP {
        cache.entries.insert(key, Arc::new(prepared));
    }
    laid
}

/// Cache key: text, body size, wrap width, font generation, and a hash of
/// everything else that shapes measurement: section fonts and link ranges.
type BreaksKey = (String, u32, u32, u32, u32, u64);

/// Hashes the parts of a built job beyond its plain text that change token
/// widths or break opportunities: each section's font family and size (bold
/// and monospace render different widths for identical text) and the link
/// spans (link text never hyphenates and keeps its own break rules).
fn format_hash(built: &Built) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for section in &built.job.sections {
        section.byte_range.start.hash(&mut hasher);
        section.byte_range.end.hash(&mut hasher);
        section.format.font_id.family.hash(&mut hasher);
        section.format.font_id.size.to_bits().hash(&mut hasher);
    }
    for (range, _) in &built.links {
        range.start.hash(&mut hasher);
        range.end.hash(&mut hasher);
    }
    hasher.finish()
}

/// The balancer's breaks for this text at this width, cached: body layout
/// runs every frame while the text, fonts and width rarely change.
fn balanced_breaks(
    ui: &egui::Ui,
    built: &Built,
    size: f32,
    max_width: f32,
) -> Arc<Vec<crate::wrap::LineBreak>> {
    let key: BreaksKey = (
        built.job.text.clone(),
        size.to_bits(),
        max_width.to_bits(),
        crate::theme::font_generation(),
        ui.pixels_per_point().to_bits(),
        format_hash(built),
    );
    type Cache =
        std::sync::Mutex<std::collections::HashMap<BreaksKey, Arc<Vec<crate::wrap::LineBreak>>>>;
    static CACHE: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(breaks) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return breaks.clone();
    }
    let measured = measure_job(ui, &built.job, &built.links, size, max_width);
    let plan = crate::wrap::balance(&measured.items, max_width, measured.hyphen);
    let breaks = Arc::new(plan.breaks);
    let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
    if cache.len() >= 512 {
        cache.clear();
    }
    cache.insert(key, breaks.clone());
    breaks
}

/// Rebuilds the job text with the balancer's breaks inserted as hard
/// newlines (plus a hyphen glyph for hyphenated breaks), shifting section
/// and link ranges. Breaks the text already makes itself (at its own
/// newlines) are skipped.
/// The job, link, code-block and quote ranges (all character-indexed) and
/// inserted-character ranges after break insertion.
type Broken = (
    LayoutJob,
    Vec<(Range<usize>, String)>,
    Vec<(usize, Range<usize>)>,
    Vec<Range<usize>>,
    Vec<(usize, usize)>,
);

fn insert_breaks(
    mut job: LayoutJob,
    links: &[(Range<usize>, String)],
    blocks: &[(usize, Range<usize>)],
    quotes: &[Range<usize>],
    breaks: &[crate::wrap::LineBreak],
) -> Broken {
    let text = std::mem::take(&mut job.text);
    let char_bytes: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain([text.len()])
        .collect();
    let byte_to_char = |byte: usize| -> usize { char_bytes.partition_point(|b| *b < byte) };
    // Insertions in character coordinates, in order.
    let mut insertions: Vec<(usize, &str)> = Vec::new();
    for brk in breaks {
        let at = byte_to_char(brk.byte);
        let preceded_by_newline =
            text[..brk.byte].ends_with('\n') || text[..brk.byte].ends_with('\r');
        if preceded_by_newline {
            continue;
        }
        insertions.push((at, if brk.hyphen { "-\n" } else { "\n" }));
    }
    if insertions.is_empty() {
        job.text = text;
        return (
            job,
            links.to_vec(),
            blocks.to_vec(),
            quotes.to_vec(),
            Vec::new(),
        );
    }
    // shift[i] = inserted characters before original character i.
    let n_chars = char_bytes.len() - 1;
    let mut shift = vec![0usize; n_chars + 1];
    let mut total = 0;
    let mut insertion = insertions.iter().peekable();
    for (i, slot) in shift.iter_mut().enumerate() {
        while let Some((at, inserted)) = insertion.peek() {
            if *at == i {
                total += inserted.len();
                insertion.next();
            } else {
                break;
            }
        }
        *slot = total;
    }
    let mut new_text = String::with_capacity(text.len() + total);
    let mut inserted_ranges: Vec<(usize, usize)> = Vec::with_capacity(insertions.len());
    let mut insertion = insertions.iter().peekable();
    for (i, ch) in text.chars().enumerate() {
        while let Some((at, inserted)) = insertion.peek() {
            if *at == i {
                inserted_ranges.push((new_text.len(), inserted.len()));
                new_text.push_str(inserted);
                insertion.next();
            } else {
                break;
            }
        }
        new_text.push(ch);
    }
    for (_, inserted) in insertion {
        inserted_ranges.push((new_text.len(), inserted.len()));
        new_text.push_str(inserted);
    }
    // New byte offset of a new character index.
    let new_bytes: Vec<usize> = new_text
        .char_indices()
        .map(|(b, _)| b)
        .chain([new_text.len()])
        .collect();
    let map_range = |start: usize, end: usize| -> Range<usize> {
        let s = start + shift[start.min(n_chars)];
        let e = end + shift[end.min(n_chars)];
        new_bytes[s]..new_bytes[e.min(new_bytes.len() - 1)]
    };
    for section in &mut job.sections {
        let range = map_range(
            byte_to_char(usize::from(section.byte_range.start)),
            byte_to_char(usize::from(section.byte_range.end)),
        );
        section.byte_range = range.start.into()..range.end.into();
    }
    // Links are character ranges (sections above are bytes), so they keep
    // character coordinates out of the shift table.
    let links = links
        .iter()
        .map(|(range, url)| {
            let start = range.start + shift[range.start.min(n_chars)];
            let end = range.end + shift[range.end.min(n_chars)];
            (start..end, url.clone())
        })
        .collect();
    let shift_ranges = |ranges: &[Range<usize>]| -> Vec<Range<usize>> {
        ranges
            .iter()
            .map(|range| {
                let start = range.start + shift[range.start.min(n_chars)];
                let end = range.end + shift[range.end.min(n_chars)];
                start..end
            })
            .collect()
    };
    let blocks = blocks
        .iter()
        .map(|(id, range)| {
            let start = range.start + shift[range.start.min(n_chars)];
            let end = range.end + shift[range.end.min(n_chars)];
            (*id, start..end)
        })
        .collect();
    let quotes = shift_ranges(quotes);
    job.text = new_text;
    (job, links, blocks, quotes, inserted_ranges)
}

/// Paints laid-out text at `pos`.
pub fn paint(ui: &egui::Ui, text: &Text, pos: Pos2, fallback: Color32) {
    ui.painter().galley(pos, text.galley.clone(), fallback);
    emoji::paint(ui, &text.galley, pos, &text.shared.placements);
}

/// Paints selectable text. The response must sense clicks and drags. Set
/// `visible` false for off-screen galleys kept only for selection state.
pub fn paint_selectable(
    ui: &egui::Ui,
    text: &Text,
    response: &egui::Response,
    pos: Pos2,
    fallback: Color32,
    visible: bool,
) {
    paint_selectable_height(ui, text, response, pos, fallback, visible, None);
}

/// Keep complete copy contents but use the displayed body's height for label
/// assembly. Hidden body rows must not overlap the following message's label.
#[allow(clippy::too_many_arguments)]
pub fn paint_selectable_height(
    ui: &egui::Ui,
    text: &Text,
    response: &egui::Response,
    pos: Pos2,
    fallback: Color32,
    visible: bool,
    displayed_height: Option<f32>,
) {
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            ui.is_enabled(),
            &text.shared.accessible_text,
        )
    });
    // egui treats non-overlapping text bounds as separate columns. Incoming
    // and outgoing bubbles are one transcript even when both contain short
    // text. Give selection a shared column while keeping every glyph in its
    // original screen position; layout, hit targets, and link offsets stay put.
    let mut galley = (*text.galley).clone();
    let column = ui.clip_rect().x_range();
    // Both ends on whole physical pixels: egui rounds where it paints the
    // galley, not this shift, so a fractional offset left every glyph of a
    // bubble between pixels and blurred it (0.21 px at 133%).
    let ppp = ui.pixels_per_point();
    let pos = Pos2::new(snap_to_pixels(pos.x, ppp), pos.y);
    let offset = pos.x - snap_to_pixels(column.min, ppp);
    for row in &mut galley.rows {
        row.pos.x += offset;
    }
    if let Some(height) = displayed_height {
        galley.rect.max.y = galley.rect.min.y + height;
    }
    galley.rect.min.x = 0.0;
    galley.rect.max.x = column.span();
    galley.mesh_bounds = galley.mesh_bounds.translate(egui::vec2(offset, 0.0));
    let selection_pos = Pos2::new(snap_to_pixels(column.min, ppp), pos.y);
    egui::text_selection::LabelSelectionState::label_text_selection(
        ui,
        response,
        selection_pos,
        Arc::new(galley),
        fallback,
        egui::Stroke::NONE,
    );
    if visible {
        emoji::paint(ui, &text.galley, pos, &text.shared.placements);
    }
}

/// The web addresses in `text`, each once, in reading order, found the way
/// a bubble underlines them. A clean URL used as a link label resolves to
/// the full destination once, just like a named title. Mail addresses do not open
/// in a browser.
pub fn links(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for span in display_spans(parse(text, &[])) {
        if let Some(link) = span.link
            && !link.starts_with("mailto:")
            && !found.contains(&link)
        {
            found.push(link);
        }
    }
    found
}

/// Plain text with resolved mentions, used in previews.
pub fn plain(text: &str, mentions: &[Mention]) -> String {
    parse(text, mentions)
        .into_iter()
        .map(|span| span.text)
        .collect()
}

/// Replaces mention ids with names without parsing other markup.
pub fn name_mentions(text: &str, mentions: &[Mention]) -> String {
    if mentions.is_empty() {
        return text.to_owned();
    }
    plain(text, mentions)
}

/// Different recipients with the same name need distinct editable tokens.
pub fn distinguish_mention_names(mentions: &mut [Mention]) {
    let names: Vec<_> = mentions
        .iter()
        .map(|mention| mention.name.clone())
        .collect();
    let mut used: std::collections::HashSet<String> = names.iter().cloned().collect();
    for (index, mention) in mentions.iter_mut().enumerate() {
        if names.iter().filter(|name| **name == names[index]).count() > 1 {
            let mut suffix = index + 1;
            loop {
                let candidate = format!("{} ({suffix})", names[index]);
                if used.insert(candidate.clone()) {
                    mention.name = candidate;
                    break;
                }
                suffix += 1;
            }
        }
    }
}

/// Restores display-name mentions in an editable draft, preserving its markup.
pub fn editable_mentions(text: &str, mentions: &[Mention]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        if text[at..].starts_with('@')
            && let Some((end, mention)) = mention_at(text, at, mentions)
        {
            out.push('@');
            out.push_str(&mention.name);
            at = end;
        } else {
            let character = text[at..].chars().next().expect("a character remains");
            out.push(character);
            at += character.len_utf8();
        }
    }
    out
}

/// Bare fences close a block. A single language token is opening metadata,
/// never code content. Do not consume WhatsApp's inline ```text``` form.
pub(crate) fn opens_fence(line: &str) -> bool {
    let Some(info) = line.trim().strip_prefix("```") else {
        return false;
    };
    info.is_empty()
        || info
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '#' | '.'))
}

fn parse(text: &str, mentions: &[Mention]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut in_block = false;
    // Fences are numbered like code_blocks numbers its entries: only a fence
    // whose joined contents are nonempty gets an index. An empty fence emits
    // no panel and no copy entry, so its spans lose the block flag here.
    let mut fence_ord = 0;
    let mut fence_lines = 0;
    let mut fence_first_empty = false;
    let mut fence_start = 0;
    let mut kept: Vec<usize> = Vec::new();
    fn close_fence(
        spans: &mut [Span],
        fence_start: usize,
        fence_lines: usize,
        fence_first_empty: bool,
        fence_ord: usize,
        kept: &mut Vec<usize>,
    ) {
        // code_blocks keeps a fence whose lines join to a nonempty string:
        // two or more lines (the join holds a newline), or one nonempty line.
        if fence_lines >= 2 || (fence_lines == 1 && !fence_first_empty) {
            kept.push(fence_ord);
        } else {
            for span in &mut spans[fence_start..] {
                span.block = false;
            }
        }
    }
    let mut first = true;
    for line in text.split('\n') {
        if !first {
            spans.push(Span {
                text: "\n".to_owned(),
                ..Default::default()
            });
        }
        first = false;
        let trimmed = line.trim_end();
        // Only a bare fence closes. Language-labelled lines inside code
        // remain literal, matching the copy extractor below.
        if trimmed.trim() == "```" || (!in_block && opens_fence(trimmed)) {
            if in_block {
                close_fence(
                    &mut spans,
                    fence_start,
                    fence_lines,
                    fence_first_empty,
                    fence_ord,
                    &mut kept,
                );
                fence_ord += 1;
            } else {
                fence_lines = 0;
                fence_first_empty = false;
                fence_start = spans.len();
            }
            in_block = !in_block;
            continue;
        }
        if in_block {
            if fence_lines == 0 {
                fence_first_empty = line.is_empty();
            }
            fence_lines += 1;
            spans.push(Span {
                text: fenced_line(line).into_owned(),
                mono: true,
                block: true,
                block_id: fence_ord,
                ..Default::default()
            });
            continue;
        }
        let mut quote = false;
        let mut content = line;
        if let Some(rest) = content
            .strip_prefix("> ")
            .or_else(|| content.strip_prefix(">"))
        {
            quote = true;
            content = rest;
            spans.push(Span {
                text: "▎ ".to_owned(),
                quote: true,
                ..Default::default()
            });
        }
        let indent = content.len() - content.trim_start().len();
        let body = &content[indent..];
        let bullet = ["* ", "- ", "• ", "◦ "]
            .into_iter()
            .find_map(|marker| body.strip_prefix(marker));
        if let Some(rest) = bullet {
            spans.push(Span {
                text: format!("{}•  ", &content[..indent]),
                quote,
                ..Default::default()
            });
            content = rest;
        }
        for mut span in inline(content) {
            span.quote = quote;
            if span.mono {
                spans.push(span);
            } else {
                spans.extend(link_and_mention(span, mentions));
            }
        }
    }
    if in_block {
        close_fence(
            &mut spans,
            fence_start,
            fence_lines,
            fence_first_empty,
            fence_ord,
            &mut kept,
        );
    }
    // Remap fence ordinals to code_blocks indices; spans of dropped fences
    // lost the block flag in close_fence.
    for span in &mut spans {
        if span.block {
            span.block_id = kept
                .iter()
                .position(|ord| *ord == span.block_id)
                .expect("block span of a kept fence");
        }
    }
    spans
}

/// A fenced block's content line. The bridge mapper escapes a content
/// line that would collide with the fence by backslashing its backticks;
/// that exact escape, and only it, resolves back, so a native backslash
/// line keeps its bytes and indentation survives.
fn fenced_line(line: &str) -> std::borrow::Cow<'_, str> {
    let resolved = line.find("\\```").and_then(|at| {
        let candidate = format!("{}{}", &line[..at], &line[at + 1..]);
        (candidate.trim() == "```").then_some(candidate)
    });
    match resolved {
        Some(resolved) => std::borrow::Cow::Owned(resolved),
        None => std::borrow::Cow::Borrowed(line),
    }
}

/// Runs of equal row kinds: (kind, first row, one past the last row).
fn row_kind_spans(laid: &Text) -> Vec<(RowKind, usize, usize)> {
    let rows = &laid.galley.rows;
    let kinds = laid.row_kinds();
    let mut spans = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        let kind = kinds[start];
        let mut end = start + 1;
        while end < rows.len() && kinds[end] == kind {
            end += 1;
        }
        spans.push((kind, start, end));
        start = end;
    }
    spans
}

/// The inset panel behind one span of rows, or None when the span starts
/// past the clip.
fn panel_rect(
    body: Rect,
    origin: Pos2,
    laid: &Text,
    start: usize,
    end: usize,
    clip_height: Option<f32>,
) -> Option<Rect> {
    const PAD: f32 = 3.0;
    let rows = &laid.galley.rows;
    let code = matches!(laid.row_kinds()[start], RowKind::Code(_));
    let top = rows[start].pos.y;
    let bottom = rows[end - 1].pos.y + rows[end - 1].row.size.y;
    if clip_height.is_some_and(|clip| top >= clip) {
        return None;
    }
    let bottom = clip_height.map_or(bottom, |clip| bottom.min(clip));
    Some(Rect::from_min_max(
        Pos2::new(body.min.x, origin.y + top - if code { 8.0 } else { PAD }),
        Pos2::new(body.max.x, origin.y + bottom + if code { 8.0 } else { PAD }),
    ))
}

/// Geometry of each code panel (block index, rect), before painting.
pub fn code_panel_rects(
    body: Rect,
    origin: Pos2,
    laid: &Text,
    clip_height: Option<f32>,
) -> Vec<(usize, Rect)> {
    row_kind_spans(laid)
        .into_iter()
        .filter_map(|(kind, start, end)| {
            if let RowKind::Code(index) = kind {
                panel_rect(body, origin, laid, start, end, clip_height).map(|panel| (index, panel))
            } else {
                None
            }
        })
        .collect()
}

/// Copy control outside and above the code panel, with a four-point gap.
pub fn copy_button_rect(panel: Rect) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            (panel.max.x - 82.0).max(panel.min.x + 6.0),
            panel.min.y - 24.0,
        ),
        Pos2::new(panel.max.x - 6.0, panel.min.y - 4.0),
    )
}

/// Render a real accessible button in the separate20pt control band.
pub fn put_copy_button(
    ui: &mut egui::Ui,
    panel: Rect,
    label: &str,
    color: Color32,
) -> egui::Response {
    // Limit the themed minimum height so selection stays clear of the header.
    let rect = copy_button_rect(panel);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    child.spacing_mut().interact_size.y = 20.0;
    child.spacing_mut().button_padding = egui::vec2(4.0, 0.0);
    child.add(egui::Button::new(egui::RichText::new(label).small().color(color)).frame(false))
}

/// Paints the quiet inset panels behind each run of [`RowKind::Code`] or
/// [`RowKind::Quote`] rows: a low-alpha fill across the body width plus a
/// pale accent bar on the leading edge. Returns each code panel with its
/// index into [`code_blocks`], so callers can add a Copy code button per
/// panel. `clip_height` limits painting to the visible rows of a collapsed
/// message, in galley coordinates like [`paint_selectable_height`].
pub fn paint_row_panels(
    painter: &egui::Painter,
    body: Rect,
    origin: Pos2,
    laid: &Text,
    clip_height: Option<f32>,
    fill: Color32,
    bar: Color32,
) -> Vec<(usize, Rect)> {
    const BAR: f32 = 2.5;
    let mut panels = Vec::new();
    for (kind, start, end) in row_kind_spans(laid) {
        if kind == RowKind::Normal {
            continue;
        }
        let Some(panel) = panel_rect(body, origin, laid, start, end, clip_height) else {
            continue;
        };
        painter.rect_filled(panel, 4.0, fill);
        painter.rect_filled(
            Rect::from_min_max(panel.min, Pos2::new(panel.min.x + BAR, panel.max.y)),
            1.0,
            bar,
        );
        if let RowKind::Code(index) = kind {
            panels.push((index, panel));
        }
    }
    panels
}

/// Display/copy content with optional explicit opening-fence metadata.
#[derive(Debug, PartialEq, Eq)]
pub struct CodeBlock {
    pub text: String,
    pub language: Option<String>,
}

/// The exact contents of each fenced block, excluding metadata and prose.
/// Unterminated blocks run to the end, matching their painted contents.
#[cfg_attr(not(test), allow(dead_code))] // Compatibility helper for exact-content callers.
pub fn code_blocks(text: &str) -> Vec<String> {
    code_block_details(text)
        .into_iter()
        .map(|block| block.text)
        .collect()
}

/// Metadata shares the extractor's exact empty-block/index rules.
pub fn code_block_details(text: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let mut current: Option<(Vec<String>, Option<String>)> = None;
    for line in text.split('\n') {
        let trimmed = line.trim();
        if trimmed == "```" || (current.is_none() && opens_fence(line)) {
            match current.take() {
                Some((lines, language)) => blocks.push(CodeBlock {
                    text: lines.join("\n"),
                    language,
                }),
                None => {
                    current = Some((
                        Vec::new(),
                        trimmed
                            .strip_prefix("```")
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                    ))
                }
            }
            continue;
        }
        if let Some((lines, _)) = current.as_mut() {
            lines.push(fenced_line(line).into_owned());
        }
    }
    if let Some((lines, language)) = current {
        blocks.push(CodeBlock {
            text: lines.join("\n"),
            language,
        });
    }
    blocks.retain(|block| !block.text.is_empty());
    blocks
}

/// A passive label across the panel edge. Its bottom meets, never covers,
/// the first code row; the existing header band reserves the upper half.
pub fn put_code_language(
    ui: &egui::Ui,
    panel: Rect,
    language: &str,
    fill: Color32,
    text: Color32,
) -> Option<Rect> {
    let max_width = (copy_button_rect(panel).left() - panel.left() - 20.0).min(160.0);
    if max_width < 28.0 {
        return None;
    }
    let mut job = LayoutJob::simple(
        language.to_owned(),
        FontId::monospace(12.0),
        text,
        max_width - 14.0,
    );
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let rect = Rect::from_min_size(
        Pos2::new(panel.left() + 8.0, panel.top() - 12.0),
        egui::vec2(galley.size().x + 14.0, 20.0),
    );
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().galley(
        Pos2::new(rect.left() + 7.0, rect.center().y - galley.size().y / 2.0),
        galley,
        text,
    );
    Some(rect)
}

/// Collapse a trailing `Title (url)` only for painting. The source, plain
/// text and copy/export paths retain the complete address. Each line is
/// independent, so an unrelated preceding message line cannot become a title.
fn display_spans(spans: Vec<Span>) -> Vec<Span> {
    let mut out = Vec::new();
    let mut line = Vec::new();
    for span in spans {
        if span.text == "\n" {
            out.extend(named_link_line(std::mem::take(&mut line)));
            out.push(span);
        } else {
            line.push(span);
        }
    }
    out.extend(named_link_line(line));
    out
}

fn named_link_line(mut line: Vec<Span>) -> Vec<Span> {
    let n = line.len();
    if n < 3 || line[n - 1].text != ")" {
        return line;
    }
    let Some(url) = line[n - 2]
        .link
        .clone()
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
    else {
        return line;
    };
    if !line[n - 3].text.ends_with(" (") || line[..n - 2].iter().any(|span| span.mono) {
        return line;
    }
    // Bridges can label a tracked destination with its clean URL (for
    // example an Instagram reel). Treat that address as a named link too,
    // but only when the complete label is the same resource without a query
    // or fragment. Never hide an unrelated second URL or discard its target.
    let has_link = line[..n - 2].iter().any(|span| span.link.is_some());
    let title: String = line[..n - 2]
        .iter()
        .map(|span| span.text.as_str())
        .collect();
    if title.trim_end_matches(" (").trim().is_empty() {
        return line;
    }
    if has_link {
        let label = title.strip_suffix(" (").unwrap_or(&title).trim();
        let resource = url.split(['?', '#']).next().unwrap_or(&url);
        if label.contains(['?', '#'])
            || label.trim_end_matches('/') != resource.trim_end_matches('/')
        {
            return line;
        }
    }
    line.truncate(n - 2);
    let last = line.last_mut().expect("title span");
    last.text.truncate(last.text.len() - 2);
    for span in &mut line {
        span.link = Some(url.clone());
    }
    line
}

/// Inline markers within one line.
fn inline(line: &str) -> Vec<Span> {
    let chars: Vec<char> = line.chars().collect();
    let bytes: Vec<usize> = line.char_indices().map(|(at, _)| at).collect();
    let mut spans = Vec::new();
    let mut run = String::new();
    // bold, italic, strike
    let mut flags = [false; 3];
    let flush = |run: &mut String, spans: &mut Vec<Span>, flags: [bool; 3]| {
        if !run.is_empty() {
            spans.push(Span {
                text: std::mem::take(run),
                bold: flags[0],
                italic: flags[1],
                strike: flags[2],
                ..Default::default()
            });
        }
    };
    let mut i = 0;
    let triple = |at: usize| {
        at + 2 < chars.len() && chars[at] == '`' && chars[at + 1] == '`' && chars[at + 2] == '`'
    };
    while i < chars.len() {
        let c = chars[i];
        // The HTML mapper distinguishes literal markers from semantic
        // markup with a following nonprinting separator. Consume both as
        // one literal character before testing code or emphasis syntax.
        if matches!(c, '*' | '_' | '~' | '`') && chars.get(i + 1) == Some(&'\u{2063}') {
            run.push(c);
            i += 2;
            continue;
        }
        // The mapper's zero-width-space hint lets a marker open or close
        // mid-word: next to a marker it counts as a boundary but never
        // reaches the text. A zero-width space anywhere else is native
        // content and stays.
        if c == '\u{200B}' {
            let by_marker = (i > 0 && matches!(chars[i - 1], '*' | '_' | '~' | '`'))
                || (i + 1 < chars.len() && matches!(chars[i + 1], '*' | '_' | '~' | '`'));
            if by_marker {
                i += 1;
                continue;
            }
        }
        // URL syntax wins over WhatsApp emphasis markers. Query strings and
        // paths commonly contain paired underscores which must remain bytes
        // of the click target, not italic delimiters.
        if c.is_ascii_alphanumeric() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            // Borrow the original suffix. Copying the remaining line at every
            // word made a long unbroken prose paragraph quadratic to parse.
            let rest = &line[bytes[i]..];
            if let Some((end, _)) = link_at(rest, 0) {
                let token = &rest[..end];
                // Decode literal hints while keeping URLs in one run. This
                // preserves their underscores and avoids rescanning a long
                // hinted URL once per character.
                let mut token_chars = token.chars().peekable();
                while let Some(ch) = token_chars.next() {
                    run.push(ch);
                    if matches!(ch, '*' | '_' | '~' | '`')
                        && token_chars.peek() == Some(&'\u{2063}')
                    {
                        token_chars.next();
                    }
                }
                i += token.chars().count();
                continue;
            }
        }
        // WhatsApp inline monospace: ```text```.
        if triple(i)
            && let Some(close) = code_close(&chars, i + 3, 3)
            && close > i + 3
        {
            flush(&mut run, &mut spans, flags);
            spans.push(Span {
                text: unescape_code(&chars[i + 3..close]),
                mono: true,
                ..Default::default()
            });
            i = close + 3;
            continue;
        }
        if c == '`'
            && let Some(close) = code_close(&chars, i + 1, 1)
            && close > i + 1
        {
            flush(&mut run, &mut spans, flags);
            spans.push(Span {
                text: unescape_code(&chars[i + 1..close]),
                mono: true,
                ..Default::default()
            });
            i = close + 1;
            continue;
        }
        if matches!(c, '*' | '_' | '~') {
            let index = match c {
                '*' => 0,
                '_' => 1,
                _ => 2,
            };
            if flags[index] {
                if is_closer(&chars, i) {
                    flush(&mut run, &mut spans, flags);
                    flags[index] = false;
                    i += 1;
                    continue;
                }
            } else if is_opener(&chars, i) && has_closer(&chars, i + 1, c) {
                flush(&mut run, &mut spans, flags);
                flags[index] = true;
                i += 1;
                continue;
            }
        }
        run.push(c);
        i += 1;
    }
    flush(&mut run, &mut spans, flags);
    spans
}

/// Finds the closing marker run of an inline code span. The bridge
/// mapper's `\\`` and `\\\\` escapes are content, not syntax; if that
/// reading finds no close, the text was native and unescaped, so the
/// plain scan decides - code ending in a backslash still closes.
fn code_close(chars: &[char], from: usize, width: usize) -> Option<usize> {
    let mut j = from;
    while j + width <= chars.len() {
        if chars[j] == '\\' && j + 1 < chars.len() && matches!(chars[j + 1], '`' | '\\') {
            j += 2;
            continue;
        }
        if (j..j + width).all(|at| chars[at] == '`') {
            return Some(j);
        }
        j += 1;
    }
    // Native text is not escaped: when the escaped reading finds no
    // close, the plain scan decides, so code ending in a backslash
    // still closes at its backtick.
    (from..chars.len())
        .find(|&j| j + width <= chars.len() && (j..j + width).all(|at| chars[at] == '`'))
}

/// Resolves the `\\`` and `\\\\` escapes inline code content can carry.
fn unescape_code(chars: &[char]) -> String {
    let mut out = String::new();
    let mut j = 0;
    while j < chars.len() {
        if chars[j] == '\\' && j + 1 < chars.len() && matches!(chars[j + 1], '`' | '\\') {
            out.push(chars[j + 1]);
            j += 2;
        } else {
            out.push(chars[j]);
            j += 1;
        }
    }
    out
}

/// A slash or backslash. An underscore against one is text, not emphasis:
/// the shrug `¯\_(ツ)_/¯` keeps its underscores. Brackets do not block, so
/// `_(hello)_` stays italic, as Beeper's `<em>(hello)</em>` arrives.
fn blocks_underscore(c: char) -> bool {
    matches!(c, '\\' | '/')
}

/// An underscore with a slash or backslash on either side.
fn against_slash(chars: &[char], i: usize) -> bool {
    chars[i] == '_'
        && ((i > 0 && blocks_underscore(chars[i - 1]))
            || chars
                .get(i + 1)
                .is_some_and(|next| blocks_underscore(*next)))
}

fn is_opener(chars: &[char], i: usize) -> bool {
    if against_slash(chars, i) || chars.get(i + 1) == Some(&'\u{2063}') {
        return false;
    }
    let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
    let after_ok = chars
        .get(i + 1)
        .is_some_and(|next| !next.is_whitespace() && *next != chars[i]);
    before_ok && after_ok
}

fn is_closer(chars: &[char], i: usize) -> bool {
    if against_slash(chars, i) || chars.get(i + 1) == Some(&'\u{2063}') {
        return false;
    }
    let before_ok = i > 0 && !chars[i - 1].is_whitespace() && chars[i - 1] != chars[i];
    let after_ok = chars.get(i + 1).is_none_or(|next| !next.is_alphanumeric());
    before_ok && after_ok
}

fn has_closer(chars: &[char], from: usize, marker: char) -> bool {
    (from..chars.len()).any(|j| chars[j] == marker && is_closer(chars, j))
}

/// Splits a plain span around web addresses and mentions.
fn link_and_mention(span: Span, mentions: &[Mention]) -> Vec<Span> {
    let mut out = Vec::new();
    let text = span.text.as_str();
    let mut plain_start = 0;
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        let at_boundary = i == 0
            || !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric());
        if at_boundary {
            if (bytes[i].is_ascii_digit() || matches!(bytes[i], b'+' | b'('))
                && let Some((end, digits, valid)) = phone_at(text, i)
            {
                if valid {
                    push_plain(&mut out, &span, &text[plain_start..i]);
                    out.push(Span {
                        text: text[i..end].to_owned(),
                        link: Some(format!("tel:{digits}")),
                        bold: span.bold,
                        italic: span.italic,
                        strike: span.strike,
                        quote: span.quote,
                        ..Default::default()
                    });
                    plain_start = end;
                    i = end;
                    continue;
                }
                if digits.is_empty() && end > i {
                    i = end;
                    continue;
                }
                // Don't retry a suffix of an oversized number as a new link.
                // Let the email parser inspect the whole token when it ends in @.
                if digits.len() > 15 && !text[end..].starts_with('@') {
                    i = end;
                    continue;
                }
            }
            if bytes[i] == b'@'
                && let Some((end, mention)) = mention_at(text, i, mentions)
            {
                push_plain(&mut out, &span, &text[plain_start..i]);
                out.push(Span {
                    text: format!("@{}", mention.name),
                    mention: true,
                    quote: span.quote,
                    ..Default::default()
                });
                plain_start = end;
                i = end;
                continue;
            }
            // Links start with a letter or digit. Skip punctuation runs.
            if bytes[i].is_ascii_alphanumeric()
                && let Some((end, url)) = link_at(text, i)
                && !bare_after_word_character(text, i, &text[i..end])
            {
                push_plain(&mut out, &span, &text[plain_start..i]);
                out.push(Span {
                    text: text[i..end].to_owned(),
                    link: Some(url),
                    bold: span.bold,
                    italic: span.italic,
                    strike: span.strike,
                    quote: span.quote,
                    ..Default::default()
                });
                plain_start = end;
                i = end;
                continue;
            }
        }
        i += 1;
    }
    push_plain(&mut out, &span, &text[plain_start..]);
    out
}

/// Finds a phone candidate with optional spaces, hyphens, and parentheses.
/// Dots are excluded so formatted tax IDs such as XXX.XXX.XXX-XX stay plain.
/// The validity flag lets the caller skip an oversized number as one token.
fn phone_at(text: &str, at: usize) -> Option<(usize, String, bool)> {
    // Reject an ISO date before scanning through the space after it; the next
    // token may be a time or a separate phone number.
    let date_at = at + usize::from(text[at..].starts_with('('));
    let iso_date = text
        .as_bytes()
        .get(date_at..date_at + 10)
        .is_some_and(|date| {
            date[4] == b'-'
                && date[7] == b'-'
                && date[..4].iter().all(u8::is_ascii_digit)
                && date[5..7].iter().all(u8::is_ascii_digit)
                && date[8..].iter().all(u8::is_ascii_digit)
        });
    if iso_date {
        return Some((date_at + 10, String::new(), false));
    }

    let mut end = at;
    let mut digits = String::new();
    for (offset, c) in text[at..].char_indices() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if matches!(c, '+' | ' ' | '-' | '(' | ')') {
            if c == '+' && offset != 0 {
                break;
            }
        } else {
            break;
        }
        end = at + offset + c.len_utf8();
    }
    while end > at && text[..end].ends_with([' ', '-', '(']) {
        end -= text[..end].chars().next_back()?.len_utf8();
    }
    let following = text[end..].chars().next();
    let valid = digits.len() >= 7
        && digits.len() <= 15
        && following.is_none_or(|c| !c.is_ascii_alphanumeric() && c != '@');
    (!digits.is_empty()).then_some((
        end,
        if text[at..].starts_with('+') {
            format!("+{digits}")
        } else {
            digits
        },
        valid,
    ))
}

fn push_plain(out: &mut Vec<Span>, template: &Span, text: &str) {
    if !text.is_empty() {
        out.push(Span {
            text: text.to_owned(),
            link: None,
            mention: false,
            ..template.clone()
        });
    }
}

fn mention_at<'m>(text: &str, at: usize, mentions: &'m [Mention]) -> Option<(usize, &'m Mention)> {
    if let Some(end) = crate::mentions::everyone_token_end(text, at)
        && let Some(mention) = mentions
            .iter()
            .find(|mention| mention.user == crate::mentions::ALL_USER)
    {
        return Some((end, mention));
    }
    let rest = &text[at + 1..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() >= 5 {
        let mention = mentions.iter().find(|mention| mention.user == digits)?;
        return Some((at + 1 + digits.len(), mention));
    }
    // A bridge mention carries the person's name (Signal): match the
    // longest listed name verbatim, bounded so "@Ann" can't chip "Anna".
    // Digits-only users stay on the WhatsApp matcher above.
    mentions
        .iter()
        .filter(|mention| {
            !mention.user.is_empty() && !mention.user.chars().all(|c| c.is_ascii_digit())
        })
        .filter(|mention| rest.starts_with(mention.user.as_str()))
        .filter(|mention| {
            text[at + 1 + mention.user.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
        })
        .max_by_key(|mention| mention.user.len())
        .map(|mention| (at + 1 + mention.user.len(), mention))
}

/// Parses a web or email address at `at` and returns its end and target.
fn link_at(text: &str, at: usize) -> Option<(usize, String)> {
    let rest = &text[at..];
    // Brackets opened inside the address belong to it, as in
    // `https://en.wikipedia.org/wiki/Rust_(programming_language)`; a closing
    // one without its opener closes the text around it: `(see example.com)`.
    let (mut parens, mut squares) = (0usize, 0usize);
    let token_end = rest
        .find(|c: char| {
            let depth = match c {
                '(' | ')' => &mut parens,
                '[' | ']' => &mut squares,
                _ => return c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '{' | '}'),
            };
            if matches!(c, '(' | '[') {
                *depth += 1;
                false
            } else if *depth > 0 {
                *depth -= 1;
                false
            } else {
                true
            }
        })
        .unwrap_or(rest.len());
    let mut token = &rest[..token_end];
    while let Some(stripped) = token.strip_suffix(['.', ',', ';', ':', '!', '?', '*', '_', '~']) {
        token = stripped;
    }
    if token.len() < 4 {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    let end = at + token.len();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return (token.len() > 8).then(|| (end, token.to_owned()));
    }
    if lower.starts_with("www.") {
        return Some((end, format!("https://{token}")));
    }
    if lower.starts_with("mailto:") {
        return Some((end, token.to_owned()));
    }
    if let Some((user, host)) = token.split_once('@')
        && !user.is_empty()
        && !user.contains('/')
        && is_domain(host, false)
    {
        return Some((end, format!("mailto:{token}")));
    }
    let (host, _) = token.split_once('/').unwrap_or((token, ""));
    if is_domain(host, true) {
        return Some((end, format!("https://{token}")));
    }
    None
}

/// A bare name (no scheme, no `www.`) right after an underscore is part of a
/// longer identifier such as `run_widths.rs`, not an address. Explicit
/// `http(s)://` and `www.` links still work after one.
fn bare_after_word_character(text: &str, at: usize, token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    let explicit = lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("www.")
        || lower.starts_with("mailto:");
    !explicit && text[..at].ends_with('_')
}

/// Whether a token is a hostname without a scheme. `bare` is a web address
/// typed with no scheme, `www.` or `@`: only those reject a mixed-case ending.
fn is_domain(host: &str, bare: bool) -> bool {
    let host = host.split_once(':').map_or(host, |(name, _)| name);
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
        return false;
    }
    if !labels.iter().all(|label| {
        label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    }) {
        return false;
    }
    let tld = labels[labels.len() - 1];
    // A capital after the dot ("ok.Today") is the next sentence, not an address;
    // all-capitals ("EXAMPLE.COM") still is one.
    let mixed_case =
        tld.chars().any(|c| c.is_ascii_lowercase()) && tld.chars().any(|c| c.is_ascii_uppercase());
    if !tld.chars().all(|c| c.is_ascii_alphabetic()) || (bare && mixed_case) {
        return false;
    }
    let tld = tld.to_ascii_lowercase();
    match tld.len() {
        0..=1 => false,
        2 => COUNTRY_TLDS.split(' ').any(|code| code == tld),
        _ => KNOWN_TLDS.contains(&tld.as_str()),
    }
}

/// Country codes a bare name may end in. Left out on purpose, because they are
/// also file extensions: rs, py, sh, md, cc, so, ml. Written with an explicit
/// scheme or `www.` they still link.
const COUNTRY_TLDS: &str = "ac ad ae af ag ai al am ao aq ar as at au aw ax az ba bb bd be bf bg \
    bh bi bj bm bn bo br bs bt bw by bz ca cd cf cg ch ci ck cl cm cn co \
    cr cu cv cw cx cy cz de dj dk dm do dz ec ee eg er es et eu fi fj fk \
    fm fo fr ga gd ge gf gg gh gi gl gm gn gp gq gr gs gt gu gw gy hk hm \
    hn hr ht hu id ie il im in io iq ir is it je jm jo jp ke kg kh ki km \
    kn kp kr kw ky kz la lb lc li lk lr ls lt lu lv ly ma mc me mg mh mk \
    mm mn mo mp mq mr ms mt mu mv mw mx my mz na nc ne nf ng ni nl no np \
    nr nu nz om pa pe pf pg ph pk pl pm pn pr ps pt pw qa re ro ru rw sa \
    sb sc sd se sg si sk sl sm sn sr ss st su sv sx sy sz tc td tf tg th \
    tj tk tl tm tn to tr tt tv tw tz ua ug uk us uy uz va vc ve vg vi vn \
    vu wf ws ye yt za zm zw";

const KNOWN_TLDS: &[&str] = &[
    "com",
    "net",
    "org",
    "edu",
    "gov",
    "mil",
    "int",
    "info",
    "biz",
    "name",
    "pro",
    "app",
    "dev",
    "io",
    "ai",
    "xyz",
    "site",
    "online",
    "tech",
    "cloud",
    "page",
    "shop",
    "store",
    "blog",
    "news",
    "berlin",
    "rocks",
    "world",
    "live",
    "life",
    "media",
    "email",
    "digital",
    "design",
    "studio",
    "agency",
    "club",
    "social",
    "space",
    "wiki",
    "zone",
    "today",
    "art",
    "fun",
    "one",
    "top",
    "link",
    "lol",
    "eu",
    "asia",
    "travel",
    "museum",
    "coop",
    "jobs",
    "mobi",
    "tel",
    "aero",
    "photos",
    "pics",
    "video",
    "music",
    "games",
    "team",
    "run",
    "ninja",
    "guru",
    "expert",
    "chat",
    "codes",
    "cool",
    "events",
    "health",
    "house",
    "land",
    "law",
    "money",
    "network",
    "party",
    "pub",
    "rest",
    "school",
    "science",
    "software",
    "solutions",
    "systems",
    "tools",
    "toys",
    "works",
    "academy",
    "bar",
    "beer",
    "bio",
    "cafe",
    "camp",
    "care",
    "center",
    "city",
    "coffee",
    "company",
    "fit",
    "fitness",
    "gallery",
    "group",
    "help",
    "host",
    "kitchen",
    "love",
    "market",
    "pizza",
    "press",
    "recipes",
    "review",
    "sale",
    "style",
    "tips",
    "vision",
    "watch",
    "wine",
    "yoga",
];

#[cfg(test)]
mod tests {
    use super::emoji_scale;

    /// One emoji draws largest, two or three big, four or more and mixed
    /// text at body size. `bubbleless` frees the same one-to-three range.
    #[test]
    fn emoji_only_messages_scale_by_count() {
        assert_eq!(emoji_scale("\u{1F4A3}"), 3.0);
        assert_eq!(emoji_scale("\u{1F602} \u{1F44D}"), 2.4);
        assert_eq!(emoji_scale("\u{1F602} \u{1F64F} \u{1F44D}"), 2.4);
        assert_eq!(emoji_scale("\u{1F602}\u{1F602}\u{1F602}\u{1F602}"), 1.0);
        assert_eq!(emoji_scale("ok \u{1F602}"), 1.0);
        assert_eq!(emoji_scale("hello"), 1.0);
        assert_eq!(emoji_scale(""), 1.0);
    }

    /// A single-emoji media caption stays at body size: it lives inside the
    /// photo's bubble, so the standalone-message growth would look wrong.
    #[test]
    fn emoji_only_captions_stay_body_size() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let grown = layout(ui, "\u{1F602}", &[], &style, 400.0, true);
            assert!(grown.big);
            assert!(grown.galley.size().y > 16.0 * 2.0);
            let caption = layout(ui, "\u{1F602}", &[], &style, 400.0, false);
            assert!(!caption.big);
            assert_eq!(
                caption.galley.size().y,
                layout(ui, "ok", &[], &style, 400.0, false).galley.size().y
            );
        });
        output.textures_delta.clear();
    }

    /// The break cache distinguishes formatting: bold and monospace change
    /// token widths for identical plain text, so the cache key hashes each
    /// section's font alongside the text, and the link spans whose break
    /// rules differ from plain words.
    #[test]
    fn break_cache_keys_track_formatting() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let plain = super::build(ui, "hello world", &[], &style, 200.0, false);
            let bold = super::build(ui, "*hello world*", &[], &style, 200.0, false);
            assert_eq!(plain.job.text, bold.job.text);
            assert_ne!(super::format_hash(&plain), super::format_hash(&bold));
            let mono = super::build(ui, "`hello world`", &[], &style, 200.0, false);
            assert_eq!(plain.job.text, mono.job.text);
            assert_ne!(super::format_hash(&plain), super::format_hash(&mono));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn wrapped_named_links_keep_the_target_on_each_title_character() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let text = layout(
                ui,
                "A long title across rows (https://example.com/path?q=a_b_c)",
                &[],
                &Style {
                    size: 16.0,
                    color: Color32::WHITE,
                    secondary: Color32::GRAY,
                    link: Color32::LIGHT_BLUE,
                    mention: Color32::WHITE,
                },
                90.0,
                true,
            );
            assert!(text.galley.rows.len() > 1);
            assert_eq!(text.galley.text(), "A long title across rows");
            for character in 0..text.galley.text().chars().count() {
                assert_eq!(
                    text.link_at(character),
                    Some("https://example.com/path?q=a_b_c")
                );
            }
            assert_eq!(
                text.shared.accessible_text,
                "A long title across rows (https://example.com/path?q=a_b_c)"
            );
        });
        output.textures_delta.clear();
    }

    #[test]
    fn named_bridge_links_hide_only_the_display_address() {
        let cases = [
            (
                "Epic Games (Game, PC) - Idle Champions of the Forgotten Realms",
                "https://store.epicgames.com/en-US/p/idle-champions-of-the-forgotten-realms",
            ),
            (
                "Amazon Prime (Game, PC) - Squirrel with a Gun",
                "https://luna.amazon.com/claims/squirrel-with-a-gun-aga/dp/amzn1.pg.item.70eff815-88f4-47f9-83e0-8a8d2837c7ad?g=s&ingress=amzn&ref_=SM_SquirrelwithaGun_S01_FGWP_CRWN",
            ),
            (
                "Amazon Prime (Game, PC) - WRC Generations",
                "https://luna.amazon.com/claims/wrc-generations-epic/dp/amzn1.pg.item.59a0703f-be28-476e-9160-8e56e257dee5?g=s&ingress=amzn&ref_=SM_WRCGenerations_S01_FGWP_CRWN",
            ),
            (
                "Sponsored message - VIEW BOT",
                "https://t.me/kindra_neon_city_bot?start=ad_5FAzm4YUZWZk",
            ),
        ];
        for (title, url) in cases {
            let source = format!("{title} ({url})");
            let spans = display_spans(parse(&source, &[]));
            assert_eq!(
                spans.iter().map(|s| s.text.as_str()).collect::<String>(),
                title
            );
            assert!(spans.iter().all(|s| s.link.as_deref() == Some(url)));
            assert_eq!(plain(&source, &[]), source, "copy/export keeps the URL");
            assert_eq!(links(&source), vec![url], "query bytes stay intact");
        }
        for source in [
            "https://example.com/a_b_c",
            "(https://example.com)",
            "https://example.com (https://other.com)",
            "Title\n(https://example.com)",
        ] {
            let before = plain(source, &[]);
            let displayed: String = display_spans(parse(source, &[]))
                .iter()
                .map(|s| s.text.as_str())
                .collect();
            assert_eq!(displayed, before, "no invented title: {source}");
        }
    }

    #[test]
    fn wrapped_clean_url_label_uses_the_full_target_on_every_character() {
        let ctx = egui::Context::default();
        let target = "https://www.instagram.com/reel/DemoReel/?id=123_456&is_sponsored=false";
        let source = format!("https://www.instagram.com/reel/DemoReel/ ({target})");
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let text = layout(
                ui,
                &source,
                &[],
                &Style {
                    size: 16.0,
                    color: Color32::WHITE,
                    secondary: Color32::GRAY,
                    link: Color32::LIGHT_BLUE,
                    mention: Color32::WHITE,
                },
                90.0,
                true,
            );
            assert!(text.galley.rows.len() > 1);
            assert_eq!(
                text.galley.text(),
                "https://www.instagram.com/reel/DemoReel/"
            );
            for character in 0..text.galley.text().chars().count() {
                assert_eq!(text.link_at(character), Some(target));
            }
            assert_eq!(text.shared.accessible_text, source);
        });
        output.textures_delta.clear();
    }

    #[test]
    fn clean_url_labels_hide_tracked_destinations_only_for_the_same_resource() {
        for (label, target) in [
            (
                "https://www.instagram.com/reel/DemoReel/",
                "https://www.instagram.com/reel/DemoReel/?id=123_456&is_sponsored=false&is_ineligible_for_clips_chaining=false",
            ),
            (
                "https://example.com/story",
                "https://example.com/story/?utm_source=share",
            ),
            (
                "https://example.com/story",
                "https://example.com/story#section",
            ),
        ] {
            let source = format!("{label} ({target})");
            let spans = display_spans(parse(&source, &[]));
            assert_eq!(
                spans.iter().map(|s| s.text.as_str()).collect::<String>(),
                label
            );
            assert!(spans.iter().all(|s| s.link.as_deref() == Some(target)));
            assert_eq!(
                links(&source),
                vec![target],
                "Copy URL keeps the full target once"
            );
            assert_eq!(
                plain(&source, &[]),
                source,
                "copy/export retains both original addresses"
            );
        }
        for source in [
            "https://example.com/a (https://example.com/b?tracking=1)",
            "https://example.com/a (https://other.example.com/a?tracking=1)",
            "https://example.com/a?q=one (https://example.com/a?q=two)",
            "See https://example.com/a (https://example.com/a?tracking=1)",
            "https://example.com/a\n(https://example.com/a?tracking=1)",
            "`https://example.com/a` (https://example.com/a?tracking=1)",
        ] {
            let spans = display_spans(parse(source, &[]));
            assert_eq!(
                spans.iter().map(|s| s.text.as_str()).collect::<String>(),
                plain(source, &[])
            );
        }
    }

    #[test]
    fn bridge_name_mentions_chip_verbatim() {
        let mentions = [Mention {
            user: "Mikael Kasurinen".into(),
            name: "Mikael Kasurinen".into(),
        }];
        let spans = parse("hey @Mikael Kasurinen look", &mentions);
        let chipped: Vec<_> = spans.iter().filter(|span| span.mention).collect();
        assert_eq!(chipped.len(), 1, "{spans:?}");
        assert_eq!(chipped[0].text, "@Mikael Kasurinen");
        // A bare prefix of a listed name stays plain.
        let spans = parse("hey @Mikael look", &mentions);
        assert!(spans.iter().all(|span| !span.mention), "{spans:?}");
        // Digits-only users still match as numbers, not name tokens.
        let digits = [Mention {
            user: "35850123".into(),
            name: "Ada".into(),
        }];
        let spans = parse("hey @35850123 look", &digits);
        assert_eq!(spans.iter().filter(|span| span.mention).count(), 1);
    }

    use super::*;

    fn kinds(text: &str) -> Vec<(String, bool, bool, bool, bool)> {
        parse(text, &[])
            .into_iter()
            .map(|span| (span.text, span.bold, span.italic, span.strike, span.mono))
            .collect()
    }

    #[test]
    fn inline_markers_pair_up() {
        assert_eq!(
            kinds("a *bold* and _it_ and ~no~ and `code`"),
            vec![
                ("a ".into(), false, false, false, false),
                ("bold".into(), true, false, false, false),
                (" and ".into(), false, false, false, false),
                ("it".into(), false, true, false, false),
                (" and ".into(), false, false, false, false),
                ("no".into(), false, false, true, false),
                (" and ".into(), false, false, false, false),
                ("code".into(), false, false, false, true),
            ]
        );
    }

    /// The Beeper adapter's `plain_rich_text` maps normalized bridge HTML
    /// (`<strong>`, `<em>`, `<del>`, `<code>`, `<pre>`, `<blockquote>`) to
    /// exactly these strings; keep the renderer in lockstep with the mapper.
    #[test]
    fn beeper_emitted_styles_parse_and_strip() {
        assert_eq!(
            kinds("*bold* _and_ ~gone~ `x`"),
            vec![
                ("bold".into(), true, false, false, false),
                (" ".into(), false, false, false, false),
                ("and".into(), false, true, false, false),
                (" ".into(), false, false, false, false),
                ("gone".into(), false, false, true, false),
                (" ".into(), false, false, false, false),
                ("x".into(), false, false, false, true),
            ]
        );
        assert_eq!(plain("*bold* _and_ ~gone~ `x`", &[]), "bold and gone x");
        assert_eq!(plain("> quoted", &[]), "\u{258e} quoted");
        // Fence lines toggle the block; their line breaks remain.
        assert_eq!(
            plain("```\nline one\nline two\n```", &[]),
            "\nline one\nline two\n"
        );
        // Raw crossing markers stay literal and readable.
        assert_eq!(plain("*a_b*c_", &[]), "*a_b*c_");
    }

    /// The mapper's zero-width-space hint lets a marker open or close
    /// mid-word without showing; the hint itself never reaches the text.
    #[test]
    fn boundary_hints_style_mid_word_markers() {
        let spans = parse("a\u{200B}*b*\u{200B}c", &[]);
        let bold: Vec<_> = spans.iter().filter(|span| span.bold).collect();
        assert_eq!(bold.len(), 1);
        assert_eq!(bold[0].text, "b");
        assert_eq!(plain("a\u{200B}*b*\u{200B}c", &[]), "abc");
        // Without the hint the same markers stay literal, as before.
        assert_eq!(plain("a*b*c", &[]), "a*b*c");
        assert_eq!(plain("2 * 3 = 6", &[]), "2 * 3 = 6");
    }

    /// Escapes keep bridge-mapped code content from closing its own span
    /// or fence; native text is not escaped, so a zero-width space away
    /// from markers, a backslash line, and code ending in a backslash all
    /// keep their bytes.
    #[test]
    fn code_content_escapes_delimiters() {
        assert_eq!(plain("`foo\\`bar\\`baz`", &[]), "foo`bar`baz");
        let spans = parse("`foo\\`bar`", &[]);
        let mono: Vec<_> = spans.iter().filter(|span| span.mono).collect();
        assert_eq!(mono.len(), 1);
        assert_eq!(mono[0].text, "foo`bar");
        // A backslash escapes itself so content can carry one.
        assert_eq!(plain("`a\\\\b`", &[]), "a\\b");
        // A fence line inside a fenced block arrives backslash-escaped.
        assert_eq!(
            plain("```\nfirst\n\\```\nsecond\n```", &[]),
            "\nfirst\n```\nsecond\n"
        );
        // A native backslash line keeps its backslash.
        assert_eq!(plain("```\n\\home\\user\n```", &[]), "\n\\home\\user\n");
        // An escaped indented fence resolves and keeps its indentation.
        assert_eq!(plain("```\n  \\```\n```", &[]), "\n  ```\n");
        // Native code ending in a backslash still closes at its backtick.
        let spans = parse("`a\\`", &[]);
        let mono: Vec<_> = spans.iter().filter(|span| span.mono).collect();
        assert_eq!(mono.len(), 1);
        assert_eq!(mono[0].text, "a\\");
        // A native zero-width space away from markers survives.
        assert_eq!(plain("a\u{200B}b", &[]), "a\u{200B}b");
    }

    #[test]
    fn rich_literal_hint_keeps_markers_copy_links_and_surrounding_styles() {
        let text = "*\u{2063}literal*\u{2063} _\u{2063}literal_\u{2063} ~\u{2063}literal~\u{2063} `\u{2063}literal`\u{2063}";
        assert_eq!(plain(text, &[]), "*literal* _literal_ ~literal~ `literal`");
        assert!(
            parse(text, &[])
                .iter()
                .all(|span| !span.bold && !span.italic && !span.strike && !span.mono)
        );
        let styled = parse("*real *\u{2063}literal*\u{2063}*", &[]);
        assert!(styled.iter().all(|span| span.bold));
        assert_eq!(
            styled
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "real *literal*"
        );
        assert_eq!(plain("a\u{2063}b", &[]), "a\u{2063}b");
        assert_eq!(plain("*\u{2063} first", &[]), "* first");
        assert_eq!(plain("`\u{2063}`\u{2063}`\u{2063}", &[]), "```");
        let link = "https://example.com/a_\u{2063}b_\u{2063}c";
        assert_eq!(plain(link, &[]), "https://example.com/a_b_c");
        assert_eq!(links(link), vec!["https://example.com/a_b_c"]);
        let shrug = "¯\\_(ツ)_/¯";
        assert_eq!(plain(shrug, &[]), shrug);
    }

    #[test]
    fn the_shrug_keeps_its_underscores_and_backslash() {
        let shrug = "¯\\_(ツ)_/¯";
        assert_eq!(plain(shrug, &[]), shrug);
        assert!(parse(shrug, &[]).iter().all(|span| !span.italic));
        let sentence = format!("well {shrug} ok");
        assert_eq!(plain(&sentence, &[]), sentence);
        // A later emphasis does not pair with the shrug's underscores.
        let both = format!("{shrug} and _word_");
        assert_eq!(plain(&both, &[]), format!("{shrug} and word"));
        // Emphasis around words still works, also in brackets and before
        // punctuation, and around a bracketed phrase (Beeper's <em>(hi)</em>).
        assert!(parse("_note_", &[]).iter().any(|span| span.italic));
        assert!(
            parse("(_note_)", &[])
                .iter()
                .any(|span| span.italic && span.text == "note")
        );
        assert!(parse("_wow!_", &[]).iter().any(|span| span.italic));
        assert_eq!(plain("_(x)_", &[]), "(x)");
        assert!(parse("_(x)_", &[]).iter().any(|span| span.italic));
    }

    /// What the composer inserts for `shrug,,` and `flip,,` (Olli's Text
    /// Replacements, the first with its leading space) must survive the
    /// markup renderer byte for byte, whatever it sits next to.
    #[test]
    fn expanded_replacements_are_not_mangled_by_markup() {
        let shrug = "¯\\_(ツ)_/¯";
        let spaced = " ¯\\_(ツ)_/¯";
        let flip = "(╯°□°）╯︵ ┻━┻";
        let unstyled = |text: &str| {
            parse(text, &[])
                .iter()
                .all(|s| !s.italic && !s.bold && !s.strike && !s.mono)
        };
        for text in [
            shrug.to_owned(),
            spaced.to_owned(),
            format!("{shrug}{shrug}"),
            format!("{shrug} {shrug}"),
            format!("a{spaced}"),
            format!("hello{spaced}\nsecond line {shrug}"),
            format!("({shrug})"),
            format!("{shrug}."),
            format!("{shrug}!"),
            format!("{shrug}?"),
            flip.to_owned(),
            format!("{flip} {shrug}"),
        ] {
            assert_eq!(plain(&text, &[]), text, "{text:?}");
            assert!(unstyled(&text), "{text:?} gained styling");
        }
        // Quote and bullet lines add their own marker; the shrug after it
        // is unchanged and unstyled.
        for text in [format!("> {shrug}"), format!("\u{2022} {shrug}")] {
            assert!(plain(&text, &[]).ends_with(shrug), "{text:?}");
            assert!(unstyled(&text), "{text:?} gained styling");
        }
        // Real emphasis next to it still works and does not pair with the
        // shrug's underscores, in either order.
        for text in [
            format!("{shrug} _word_"),
            format!("_word_ {shrug}"),
            format!("*bold* {shrug} ~gone~"),
        ] {
            let shown = plain(&text, &[]);
            assert!(shown.contains(shrug), "{text:?} -> {shown:?}");
            assert!(!unstyled(&text), "{text:?} lost its emphasis");
        }
        assert_eq!(
            plain(&format!("_word_ {shrug}"), &[]),
            format!("word {shrug}")
        );
    }

    #[test]
    fn stray_markers_stay_literal() {
        assert_eq!(plain("2 * 3 = 6", &[]), "2 * 3 = 6");
        assert_eq!(plain("snake_case_name", &[]), "snake_case_name");
        assert_eq!(plain("*unclosed", &[]), "*unclosed");
        assert_eq!(plain("* Guestlist", &[]), "•  Guestlist");
    }

    #[test]
    fn blocks_and_lists() {
        let spans = parse("> quoted\n- one\n```\nlet x = 1;\n```", &[]);
        assert_eq!(spans[0].text, "▎ ");
        assert!(spans[0].quote);
        assert_eq!(spans[1].text, "quoted");
        assert!(spans.iter().any(|span| span.text == "•  "));
        assert!(
            spans
                .iter()
                .any(|span| span.mono && span.text == "let x = 1;")
        );
    }

    #[test]
    fn addresses_are_found_with_and_without_a_scheme() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert_eq!(links("see https://a.b/c?d=1."), vec!["https://a.b/c?d=1"]);
        assert_eq!(
            links("go to spotifast.rocks!"),
            vec!["https://spotifast.rocks"]
        );
        assert_eq!(
            links("mail hello@section8berlin.com or dm"),
            vec!["mailto:hello@section8berlin.com"]
        );
        assert_eq!(
            links("(www.rust-lang.org)"),
            vec!["https://www.rust-lang.org"]
        );
        assert!(links("version 0.3.0 of main.rs and e.g. this").is_empty());
        assert!(links("abchttp://x").is_empty());
    }

    #[test]
    fn addresses_keep_the_brackets_they_open() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert_eq!(
            links("https://example.com/testing(testing)"),
            vec!["https://example.com/testing(testing)"]
        );
        assert_eq!(
            links("read https://en.wikipedia.org/wiki/Foo_(bar)."),
            vec!["https://en.wikipedia.org/wiki/Foo_(bar)"]
        );
        assert_eq!(
            links("(see https://en.wikipedia.org/wiki/Foo_(bar))"),
            vec!["https://en.wikipedia.org/wiki/Foo_(bar)"]
        );
        assert_eq!(
            links("(see https://example.com)"),
            vec!["https://example.com"]
        );
        assert_eq!(
            links("[https://example.com/a[1]]"),
            vec!["https://example.com/a[1]"]
        );
        assert_eq!(
            links("example.com/a(b)c, then"),
            vec!["https://example.com/a(b)c"]
        );
        let spans = parse("(see https://example.com)", &[]);
        assert_eq!(spans.last().map(|span| span.text.as_str()), Some(")"));
    }

    #[test]
    fn code_names_are_not_linked_after_an_underscore() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert!(links("run_widths.rs").is_empty());
        assert!(links("see run_widths.rs and main.py, util.js, notes.md, x.sh").is_empty());
        assert!(links("my_site.com").is_empty());
        // Explicit links and ordinary bare domains keep working.
        assert_eq!(
            links("run_ https://widths.rs/x"),
            vec!["https://widths.rs/x"]
        );
        assert_eq!(links("_www.example.com"), vec!["https://www.example.com"]);
        assert_eq!(links("go to example.com"), vec!["https://example.com"]);
    }

    #[test]
    fn bare_names_link_only_with_a_real_lowercase_or_capital_ending() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        // File names and run-on sentences are not addresses.
        assert!(links("see tests.kt and build.ex and mod.el").is_empty());
        assert!(links("ok.Today no.Help joo.Love").is_empty());
        assert!(links("main.rs lib.py run.sh notes.md").is_empty());
        // Real ones still link, including country codes and all capitals.
        assert_eq!(links("vedur.is"), vec!["https://vedur.is"]);
        assert_eq!(links("yle.fi/uutiset"), vec!["https://yle.fi/uutiset"]);
        assert_eq!(links("EXAMPLE.COM"), vec!["https://EXAMPLE.COM"]);
        // Explicit schemes and www. keep working for any ending.
        assert_eq!(links("https://tokio.rs/x"), vec!["https://tokio.rs/x"]);
        assert_eq!(links("www.example.rs"), vec!["https://www.example.rs"]);
        // An email host may be mixed case.
        assert_eq!(links("hello@Example.Com"), vec!["mailto:hello@Example.Com"]);
    }

    #[test]
    fn phone_numbers_become_links_without_linking_tax_ids() {
        let links = |text: &str| -> Vec<String> {
            parse(text, &[])
                .into_iter()
                .filter_map(|span| span.link)
                .collect()
        };
        assert_eq!(
            links("CPF 000.000.000-00; call 000 0000-0000 or +00 (00) 00000-0000."),
            vec!["tel:00000000000", "tel:+0000000000000"]
        );
        assert_eq!(links("Call (212) 555-1212"), vec!["tel:2125551212"]);
        assert_eq!(
            parse("Call (212) 555-1212", &[])
                .into_iter()
                .find(|span| span.link.is_some())
                .unwrap()
                .text,
            "(212) 555-1212"
        );
        assert_eq!(links("Call +1 (212) 555-1212"), vec!["tel:+12125551212"]);
        let spans = parse("Call +1 (212) 555-1212", &[]);
        assert_eq!(
            spans.iter().find(|span| span.link.is_some()).unwrap().text,
            "+1 (212) 555-1212"
        );
        assert!(links("Date 2026-10-01").is_empty());
        assert!(links("Date (2026-10-01)").is_empty());
        assert!(links("Date 2026-10-01 12:30").is_empty());
        assert_eq!(links("Date 2026-10-01 1234567"), vec!["tel:1234567"]);
        assert!(links("Reference 12345678-90123456").is_empty());
        assert_eq!(
            links("1234567@example.com"),
            vec!["mailto:1234567@example.com"]
        );
        assert_eq!(
            links("1234567890123456@example.com"),
            vec!["mailto:1234567890123456@example.com"]
        );
        assert!(links("id 000000; long id 0000000000000000").is_empty());
        assert!(links("abc00000000000").is_empty());
    }

    #[test]
    fn mentions_become_names() {
        let mentions = [Mention {
            user: "174057861464188".into(),
            name: "+49 176 31141665".into(),
        }];
        let spans = parse("20:00 @174057861464188 (no pronouns)", &mentions);
        let mention = spans.iter().find(|span| span.mention).expect("mention");
        assert_eq!(mention.text, "@+49 176 31141665");
        assert_eq!(
            plain("hi @174057861464188", &mentions),
            "hi @+49 176 31141665"
        );
        assert_eq!(plain("hi @123", &mentions), "hi @123");
    }

    #[test]
    fn everyone_is_highlighted_only_with_its_mention() {
        let everyone = [Mention {
            user: "all".into(),
            name: "all".into(),
        }];
        let spans = parse("heading over @all tonight", &everyone);
        let mention = spans.iter().find(|span| span.mention).expect("mention");
        assert_eq!(mention.text, "@all");
        assert!(
            parse("heading over @all tonight", &[])
                .iter()
                .all(|span| !span.mention)
        );
        assert!(
            parse("mail@all.com and @alligator", &everyone)
                .iter()
                .all(|span| !span.mention)
        );
        assert_eq!(plain("ping @all", &everyone), "ping @all");
    }

    fn body_style() -> Style {
        Style {
            size: 16.0,
            color: Color32::WHITE,
            secondary: Color32::GRAY,
            link: Color32::LIGHT_BLUE,
            mention: Color32::WHITE,
        }
    }

    /// Each fixture owns its fonts and cache. Other suites can still bump
    /// the legacy global font generation, so retries below reject those runs.
    fn prepared_test_context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        ctx
    }

    fn clear_prepared_cache(ctx: &egui::Context) {
        prepared_cache(ctx)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entries
            .clear();
    }

    /// Unchanged inputs reuse the prepared layout instead of rebuilding the
    /// markup and re-inserting the balancer's breaks every frame.
    #[test]
    fn a_balanced_layout_is_prepared_once_per_input_set() {
        let ctx = prepared_test_context();
        for _ in 0..3 {
            clear_prepared_cache(&ctx);
            let generation = crate::theme::font_generation();
            let mut laid = None;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let text = "words that wrap and balance over several lines ".repeat(20);
                let first = layout_balanced(ui, &text, &[], &body_style(), 300.0, true);
                let second = layout_balanced(ui, &text, &[], &body_style(), 300.0, true);
                let narrower = layout_balanced(ui, &text, &[], &body_style(), 200.0, true);
                let larger = layout_balanced(
                    ui,
                    &text,
                    &[],
                    &Style {
                        size: 18.0,
                        ..body_style()
                    },
                    300.0,
                    true,
                );
                laid = Some((first, second, narrower, larger));
            });
            output.textures_delta.clear();
            if crate::theme::font_generation() != generation {
                continue;
            }
            let (first, second, narrower, larger) = laid.expect("laid out");
            assert!(
                std::sync::Arc::ptr_eq(&first.shared, &second.shared),
                "unchanged inputs reuse the prepared layout"
            );
            assert!(
                !std::sync::Arc::ptr_eq(&first.shared, &narrower.shared),
                "a new wrap width lays out again"
            );
            assert_eq!(first.unbroken_text(), narrower.unbroken_text());
            assert!(
                !std::sync::Arc::ptr_eq(&first.shared, &larger.shared),
                "a new body size lays out again"
            );
            return;
        }
        panic!("the font generation kept changing under the test");
    }

    /// The cached copy matches a fresh build, and an edited body or a
    /// renamed mention never reuses it.
    #[test]
    fn a_prepared_layout_matches_fresh_and_tracks_edits() {
        let ctx = prepared_test_context();
        for _ in 0..3 {
            clear_prepared_cache(&ctx);
            let generation = crate::theme::font_generation();
            let mut laid = None;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let text = "hey @35850123, lunch tomorrow? ".repeat(8);
                let mentions = [Mention {
                    user: "35850123".into(),
                    name: "Ada".into(),
                }];
                let fresh = Prepared::build(ui, &text, &mentions, &body_style(), 300.0, true);
                let fresh_drawn = fresh.draw(ui);
                let cached = layout_balanced(ui, &text, &mentions, &body_style(), 300.0, true);
                let again = layout_balanced(ui, &text, &mentions, &body_style(), 300.0, true);
                let edited = format!("{text}!");
                let relaid = layout_balanced(ui, &edited, &mentions, &body_style(), 300.0, true);
                let renamed = [Mention {
                    user: "35850123".into(),
                    name: "Ada Lovelace".into(),
                }];
                let renamed_layout =
                    layout_balanced(ui, &text, &renamed, &body_style(), 300.0, true);
                laid = Some((fresh_drawn, cached, again, relaid, renamed_layout, edited));
            });
            output.textures_delta.clear();
            if crate::theme::font_generation() != generation {
                continue;
            }
            let (fresh_drawn, cached, again, relaid, renamed_layout, edited) =
                laid.expect("laid out");
            assert!(std::sync::Arc::ptr_eq(&cached.shared, &again.shared));
            assert_eq!(fresh_drawn.galley.text(), again.galley.text());
            assert_eq!(fresh_drawn.galley.rows.len(), again.galley.rows.len());
            assert_eq!(fresh_drawn.unbroken_text(), again.unbroken_text());
            assert_eq!(fresh_drawn.inserted(), again.inserted());
            assert!(
                !std::sync::Arc::ptr_eq(&cached.shared, &relaid.shared),
                "an edited body lays out again"
            );
            assert_eq!(
                relaid.unbroken_text(),
                edited.replace("@35850123", "@Ada"),
                "copy text preserves the edit and resolves the mention name"
            );
            assert!(
                !std::sync::Arc::ptr_eq(&cached.shared, &renamed_layout.shared),
                "a renamed mention lays out again"
            );
            assert_ne!(cached.galley.text(), renamed_layout.galley.text());
            return;
        }
        panic!("the font generation kept changing under the test");
    }

    /// A font reload drops the prepared jobs (they name the old fonts) and
    /// the body is prepared again under the new generation; the re-shaped
    /// galley's metrics match a from-scratch layout under the current fonts.
    /// Only Inter is available headless, so glyph-metric differences between
    /// typefaces cannot manifest here: the reload handling is proven through
    /// the rebuild, and the galley always comes from egui's own per-pass
    /// cache, which tracks the live atlas.
    #[test]
    fn a_font_reload_rebuilds_a_cached_body() {
        let ctx = prepared_test_context();
        let (first, again) = {
            let mut pair = None;
            for _ in 0..3 {
                clear_prepared_cache(&ctx);
                let generation = crate::theme::font_generation();
                let mut laid = None;
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    let first = layout_balanced(
                        ui,
                        "font *reload* check body text",
                        &[],
                        &body_style(),
                        300.0,
                        true,
                    );
                    let again = layout_balanced(
                        ui,
                        "font *reload* check body text",
                        &[],
                        &body_style(),
                        300.0,
                        true,
                    );
                    laid = Some((first, again));
                });
                output.textures_delta.clear();
                if crate::theme::font_generation() == generation {
                    pair = laid;
                    break;
                }
            }
            pair.expect("the font generation kept changing under the test")
        };
        assert!(
            std::sync::Arc::ptr_eq(&first.shared, &again.shared),
            "without a reload the prepared body is reused"
        );
        crate::theme::set_font(&ctx, crate::settings::FontChoice::Inter);
        crate::theme::set_font(&ctx, crate::settings::FontChoice::System);
        let mut laid = None;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let relaid = layout_balanced(
                ui,
                "font *reload* check body text",
                &[],
                &body_style(),
                300.0,
                true,
            );
            let fresh = Prepared::build(
                ui,
                "font *reload* check body text",
                &[],
                &body_style(),
                300.0,
                true,
            )
            .draw(ui);
            laid = Some((relaid, fresh));
        });
        output.textures_delta.clear();
        let (relaid, fresh) = laid.expect("laid out");
        assert!(
            !std::sync::Arc::ptr_eq(&first.shared, &relaid.shared),
            "a font reload rebuilds the prepared job under the new generation"
        );
        assert_eq!(first.galley.text(), relaid.galley.text());
        assert_eq!(
            relaid.galley.size(),
            fresh.galley.size(),
            "metrics match a fresh layout under the current fonts"
        );
        assert_eq!(relaid.galley.rows.len(), fresh.galley.rows.len());
    }

    #[test]
    fn prepared_caches_belong_to_their_font_context() {
        let first = prepared_test_context();
        let second = prepared_test_context();
        assert!(!Arc::ptr_eq(
            &prepared_cache(&first),
            &prepared_cache(&second)
        ));
        assert!(Arc::ptr_eq(
            &prepared_cache(&first),
            &prepared_cache(&first)
        ));
    }

    /// More bodies than the old small cap: an active text selection lays
    /// out every loaded row each frame, and evicting under that scan
    /// thrashes the cache into permanent misses. The cache instead keeps a
    /// stable subset, and later passes reuse it.
    #[test]
    fn a_full_timeline_scan_keeps_its_cached_subset() {
        let ctx = prepared_test_context();
        for _ in 0..3 {
            clear_prepared_cache(&ctx);
            let generation = crate::theme::font_generation();
            let mut hits = 0;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let texts: Vec<String> = (0..(PREPARED_CAP + 76))
                    .map(|i| format!("scan body {i} with a few words to wrap"))
                    .collect();
                let first: Vec<_> = texts
                    .iter()
                    .map(|t| layout_balanced(ui, t, &[], &body_style(), 300.0, true))
                    .collect();
                let second: Vec<_> = texts
                    .iter()
                    .map(|t| layout_balanced(ui, t, &[], &body_style(), 300.0, true))
                    .collect();
                hits = first
                    .iter()
                    .zip(&second)
                    .filter(|(a, b)| std::sync::Arc::ptr_eq(&a.shared, &b.shared))
                    .count();
            });
            output.textures_delta.clear();
            if crate::theme::font_generation() != generation {
                continue;
            }
            assert_eq!(hits, PREPARED_CAP, "a scan reuses the stable subset");
            return;
        }
        panic!("the font generation kept changing under the test");
    }
}

#[cfg(test)]
mod monospace_tests {
    use super::*;

    fn mono(text: &str) -> Vec<(String, bool)> {
        parse(text, &[])
            .into_iter()
            .map(|span| (span.text, span.mono))
            .collect()
    }

    #[test]
    fn triple_backticks_mark_monospace_on_a_line() {
        assert_eq!(
            mono("say ```hello``` now"),
            vec![
                ("say ".into(), false),
                ("hello".into(), true),
                (" now".into(), false),
            ]
        );
        assert_eq!(mono("```hello```"), vec![("hello".into(), true)]);
        // Preserve all text after an unmatched marker.
        assert_eq!(
            mono("```a```\nb"),
            vec![
                ("a".into(), true),
                ("\n".into(), false),
                ("b".into(), false)
            ]
        );
    }

    #[test]
    fn a_fence_line_still_opens_a_block() {
        let spans = mono("```\ncode\n```\nafter");
        assert!(spans.iter().any(|(text, mono)| text == "code" && *mono));
        assert!(spans.iter().any(|(text, mono)| text == "after" && !*mono));
    }

    #[test]
    fn web_links_are_listed_once_without_mail_addresses() {
        assert_eq!(
            links("see https://a.b/c, then spotifast.rocks and https://a.b/c again"),
            vec!["https://a.b/c", "https://spotifast.rocks"]
        );
        assert!(links("mail hello@section8berlin.com or nothing").is_empty());
        assert!(links("version 0.3.0 of main.rs").is_empty());
    }

    /// Fenced lines are flagged as block code; inline `code` is not.
    #[test]
    fn fenced_lines_are_block_code_and_inline_code_is_not() {
        let spans = parse("before\n```\nlet x = 1;\n```\nafter `inl`", &[]);
        let block: Vec<_> = spans.iter().filter(|span| span.block).collect();
        assert_eq!(block.len(), 1);
        assert_eq!(block[0].text, "let x = 1;");
        assert!(block[0].mono);
        let inline = spans
            .iter()
            .find(|span| span.text == "inl")
            .expect("inline code span");
        assert!(inline.mono && !inline.block);
    }

    /// Two fences with nothing between them are two blocks and two row
    /// runs, so each panel's Copy code button reaches its own block.
    #[test]
    fn adjacent_fences_stay_two_code_runs() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "```\nfirst\n```\n```\nsecond\n```";
            let laid = layout_balanced(ui, text, &[], &style, 220.0, false);
            let kinds = laid.row_kinds();
            assert_eq!(
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, RowKind::Code(0)))
                    .count(),
                1,
                "first block: {kinds:?}"
            );
            assert_eq!(
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, RowKind::Code(1)))
                    .count(),
                1,
                "second block: {kinds:?}"
            );
            assert_eq!(code_blocks(text), ["first", "second"]);
        });
        output.textures_delta.clear();
    }

    /// Every line of a multiline fence carries the same block index, a blank
    /// line stays inside its block, an empty fence consumes no index, and
    /// each panel's Copy code button reaches its own block.
    #[test]
    fn multiline_fences_keep_one_block_index_each() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "```\nalpha\n\nbeta\n```\nprose\n```\n```\nmore\n```\ngamma\ndelta\n```";
            let laid = layout_balanced(ui, text, &[], &style, 220.0, false);
            let kinds = laid.row_kinds();
            // alpha, the blank line and beta are block 0; gamma and delta 1.
            assert_eq!(
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, RowKind::Code(0)))
                    .count(),
                3,
                "first block: {kinds:?}"
            );
            assert_eq!(
                kinds
                    .iter()
                    .filter(|kind| matches!(kind, RowKind::Code(1)))
                    .count(),
                2,
                "second block: {kinds:?}"
            );
            assert_eq!(code_blocks(text), ["alpha\n\nbeta", "gamma\ndelta"]);
        });
        output.textures_delta.clear();
    }

    /// Blank lines belong to the copy text, and an empty fence, which
    /// paints no row and gets no panel, gets no copy entry either, so panel
    /// and block indices stay aligned.
    #[test]
    fn code_blocks_keep_blank_lines_and_skip_empty_fences() {
        // A leading blank line is part of the block.
        assert_eq!(code_blocks("```\n\nfirst\n```"), ["\nfirst"]);
        // An empty fence paints no panel, so it has no copy entry either.
        assert_eq!(
            code_blocks("```\n```\ntext\n```\necho hi\n```"),
            ["echo hi"]
        );
    }

    /// Copy code takes the exact block: no fences, no surrounding prose,
    /// the bridge's fence escape resolved, several blocks in order, and an
    /// unterminated fence running to the end like it paints.
    #[test]
    fn code_blocks_copy_the_exact_block_contents() {
        assert_eq!(
            code_blocks("run this:\n```\ncd ~/koodi\ncargo build\n```\nthen wait"),
            vec!["cd ~/koodi\ncargo build"]
        );
        assert_eq!(
            code_blocks("```\nfirst\n```\nmiddle\n```\nsecond\n```"),
            vec!["first", "second"]
        );
        // A backslashed line is content, and unless it is the bridge's
        // exact fence escape it keeps its bytes, backslash included.
        assert_eq!(code_blocks("```\na\\```b\n```"), vec!["a\\```b"]);
        assert_eq!(code_blocks("```\n\\```\n```"), vec!["```"]);
        // Indentation and inner blank lines survive byte for byte.
        assert_eq!(
            code_blocks("```\n  indented\n\n  more\n```"),
            vec!["  indented\n\n  more"]
        );
        assert_eq!(code_blocks("```\nnever closed"), vec!["never closed"]);
        assert!(code_blocks("no code here").is_empty());
        // An empty fence paints no panel, so it has no copy entry.
        assert!(code_blocks("```\n```").is_empty());
    }

    /// Panel rows follow the galley: quote and code rows are marked, plain
    /// rows are not, and the marks survive the balancer's inserted breaks.
    #[test]
    fn row_kinds_track_quote_and_code_rows() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "> wait for the fixes\nuse a local build:\n```\ncargo build --locked --release --target aarch64-apple-darwin and some more words to wrap\n```\ndone";
            let laid = layout_balanced(ui, text, &[], &style, 220.0, false);
            assert_eq!(laid.row_kinds().len(), laid.galley.rows.len());
            assert_eq!(laid.row_kinds()[0], RowKind::Quote);
            let code_rows = laid
                .row_kinds()
                .iter()
                .filter(|kind| matches!(kind, RowKind::Code(_)))
                .count();
            assert!(
                code_rows >= 2,
                "the long code line wraps and every wrapped row stays code: {code_rows}"
            );
            assert_eq!(laid.row_kinds().last(), Some(&RowKind::Normal));
        });
        output.textures_delta.clear();
    }

    /// Code lines never paint wider than their room: a fenced block with a
    /// very long line wraps to the layout width instead of overflowing the
    /// bubble.
    #[test]
    fn fenced_code_rows_fit_the_layout_width() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let long = "x".repeat(300);
            let text = format!("```\ncmd {long} end\n```");
            let laid = layout_balanced(ui, &text, &[], &style, 200.0, false);
            for (index, row) in laid.galley.rows.iter().enumerate() {
                // The planner sums advances while the galley snaps glyph
                // positions to pixels, so a shaped row can exceed the
                // planned width by a fraction of a pixel, never more.
                assert!(
                    row.row.size.x <= 200.5,
                    "row {index} is {} wide in a 200-wide room",
                    row.row.size.x
                );
            }
        });
        output.textures_delta.clear();
    }

    /// A long code line soft-wraps character by character: a hyphenation
    /// "-" inside a fenced block would paint as code that was never typed.
    #[test]
    fn a_long_code_line_wraps_without_a_hyphen_glyph() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let long = "x".repeat(300);
            let text = format!("```\ncmd {long} end\n```");
            let laid = layout_balanced(ui, &text, &[], &style, 200.0, false);
            let hyphens = laid
                .galley
                .rows
                .iter()
                .flat_map(|row| row.row.glyphs.iter())
                .filter(|glyph| glyph.chr == '-')
                .count();
            assert_eq!(hyphens, 0, "code rows paint no inserted hyphen");
        });
        output.textures_delta.clear();
    }

    /// Copy code sits outside and above the panel, separate from code selection.
    #[test]
    fn code_panels_have_inset_header_bottom_space_and_unchanged_copy() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "```\nfirst line\nsecond line\n```";
            for laid in [
                Arc::new(layout(ui, text, &[], &style, 220.0, false)),
                layout_balanced(ui, text, &[], &style, 220.0, false),
            ] {
                let body = Rect::from_min_size(Pos2::ZERO, laid.galley.size());
                let (_, panel) = code_panel_rects(body, Pos2::ZERO, &laid, None)[0];
                let rows: Vec<_> = laid
                    .galley
                    .rows
                    .iter()
                    .zip(laid.row_kinds())
                    .filter(|(_, kind)| matches!(kind, RowKind::Code(_)))
                    .map(|(row, _)| row)
                    .collect();
                assert!(rows[0].pos.x - panel.left() >= 9.5);
                assert!(rows[0].pos.y - copy_button_rect(panel).bottom() >= 5.5);
                let last = rows.last().unwrap();
                assert!(panel.bottom() - last.pos.y - last.row.size.y >= 7.5);
                assert!(rows[1].pos.y - rows[0].pos.y >= 21.0);
                assert_eq!(code_blocks(text), vec!["first line\nsecond line"]);
                assert!(
                    panel.top() >= 0.0,
                    "header must fit inside allocated text body"
                );
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn zoom_rebuilds_fenced_row_classifications_at_the_same_point_width() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let text = "```\nlet answer = value + several_more_values;\n```";
        for scale in [1.0, 0.85, 1.25, 1.5, 2.0] {
            ctx.set_pixels_per_point(scale);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let style = Style {
                    size: 16.0,
                    color: Color32::WHITE,
                    secondary: Color32::GRAY,
                    link: Color32::BLUE,
                    mention: Color32::WHITE,
                };
                for cap in [139.0, 140.0, 141.0, 220.0] {
                    let cached = layout_balanced(ui, text, &[], &style, cap, false);
                    let fresh = Prepared::build(ui, text, &[], &style, cap, false).draw(ui);
                    assert_eq!(cached.galley.rows.len(), cached.row_kinds().len());
                    assert_eq!(cached.row_kinds(), fresh.row_kinds());
                    assert_eq!(cached.galley.rect, fresh.galley.rect);
                }
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn wrapped_code_insets_stay_inside_the_requested_width() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "Before\n```\na fairly long line of code with several words and punctuation();\n```\nAfter";
            for cap in [140.0, 220.0, 300.0] {
                for laid in [
                    Arc::new(layout(ui, text, &[], &style, cap, false)),
                    layout_balanced(ui, text, &[], &style, cap, false),
                ] {
                    assert!(laid.galley.size().x <= cap + 0.5,
                        "code inset overflow: {} > {cap}", laid.galley.size().x);
                    for (row, kind) in laid.galley.rows.iter().zip(laid.row_kinds()) {
                        if matches!(kind, RowKind::Code(_)) {
                            assert!(row.pos.x + row.row.size.x + 10.0 <= cap + 0.5);
                        }
                    }
                    assert_eq!(code_blocks(text), vec!["a fairly long line of code with several words and punctuation();"]);
                }
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_copy_button_never_covers_code_text() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            // Short lines: the widest code row reaches the panel's right
            // edge, where the button used to paint over the text.
            let text = "```\ncd ~/koodi/solmu\ncargo build\n```";
            let laid = layout_balanced(ui, text, &[], &style, 200.0, false);
            let body = Rect::from_min_size(Pos2::ZERO, laid.galley.size());
            for (_, panel) in code_panel_rects(body, Pos2::ZERO, &laid, None) {
                let button = copy_button_rect(panel);
                for (row, kind) in laid.galley.rows.iter().zip(laid.row_kinds().iter()) {
                    if !matches!(kind, RowKind::Code(_)) || row.row.glyphs.is_empty() {
                        continue;
                    }
                    let text_rect = Rect::from_min_size(row.pos, row.row.size);
                    assert!(
                        !button.intersects(text_rect),
                        "copy button {button:?} covers code row {text_rect:?}"
                    );
                }
            }
        });
        output.textures_delta.clear();
    }

    /// The Copy code button's actual response rect, not just its
    /// layout target: a themed Button expands to the theme's minimum
    /// interact height under Ui::put and leaks from the reserved band
    /// into the first code row, where it intercepts selection drags.
    /// The response must stay inside the20pt band above the panel.
    #[test]
    fn the_copy_button_response_stays_in_its_band() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "```\ncd ~/koodi/solmu\ncargo build\n```";
            let laid = layout_balanced(ui, text, &[], &style, 200.0, false);
            let body = Rect::from_min_size(Pos2::ZERO, laid.galley.size());
            for (_, panel) in code_panel_rects(body, Pos2::ZERO, &laid, None) {
                let response = put_copy_button(ui, panel, "Copy code", Color32::WHITE);
                let band = copy_button_rect(panel);
                assert!(band.bottom() <= panel.top() - 3.5);
                assert!(!band.intersects(panel));
                assert!(
                    band.contains_rect(response.rect) && response.rect.height() <= 20.0,
                    "copy button response {:?} outgrew its band {band:?}",
                    response.rect
                );
                for (row, kind) in laid.galley.rows.iter().zip(laid.row_kinds().iter()) {
                    if !matches!(kind, RowKind::Code(_)) || row.row.glyphs.is_empty() {
                        continue;
                    }
                    let text_rect = Rect::from_min_size(row.pos, row.row.size);
                    assert!(
                        !response.rect.intersects(text_rect),
                        "copy button response {:?} reaches code row {text_rect:?}",
                        response.rect
                    );
                }
            }
        });
        output.textures_delta.clear();
    }

    /// The Copy code control must be a real labelled widget: a bare
    /// allocate_rect plus painted text keeps the geometry but gives a
    /// screen reader an unnamed hit region (Devin finding on db154ed).
    #[test]
    fn the_copy_button_has_an_accessible_label() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut id = None;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "```\ncd ~/koodi/solmu\ncargo build\n```";
            let laid = layout_balanced(ui, text, &[], &style, 200.0, false);
            let body = Rect::from_min_size(Pos2::ZERO, laid.galley.size());
            for (_, panel) in code_panel_rects(body, Pos2::ZERO, &laid, None) {
                id = Some(put_copy_button(ui, panel, "Copy code", Color32::WHITE).id);
            }
        });
        let update = output
            .platform_output
            .accesskit_update
            .expect("an accesskit tree update");
        let labelled = update
            .nodes
            .iter()
            .any(|(_, node)| node.label().is_some_and(|l| l == "Copy code"));
        assert!(labelled, "copy button exposes no accessible label");
        output.textures_delta.clear();
    }

    /// A collapse clip can cut a code block mid-panel: the clamped
    /// panel's bottom sits at the clip, so a button hanging below the
    /// panel lands outside the visible body and Copy code is
    /// unreachable. The button must hang off the panel's top, which
    /// stays visible whenever any of the block is.
    #[test]
    fn the_copy_button_stays_visible_on_a_clipped_block() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "intro\n```\nline one\nline two\nline three\nline four\nline five\n```";
            let laid = layout_balanced(ui, text, &[], &style, 200.0, false);
            // Cut the block mid-panel: rows past the clip are invisible.
            let clip = laid.galley.rows[4].pos.y + 1.0;
            let body = Rect::from_min_size(Pos2::ZERO, laid.galley.size());
            let panels = code_panel_rects(body, Pos2::ZERO, &laid, Some(clip));
            assert_eq!(panels.len(), 1, "expected one clipped code panel");
            for (_, panel) in panels {
                let button = copy_button_rect(panel);
                assert!(
                    button.max.y <= clip,
                    "copy button {button:?} falls below the clip {clip}"
                );
            }
        });
        output.textures_delta.clear();
    }

    /// A blank line inside a fenced block keeps the block's rows together:
    /// the content rows and the blank between them stay code, so the panel
    /// and the Copy code button cover the block as one piece. The dropped
    /// fence markers' own blank rows stay plain.
    #[test]
    fn a_blank_line_inside_a_fence_stays_code() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::LIGHT_BLUE,
                mention: Color32::WHITE,
            };
            let text = "```\nfirst\n\nthird\n```\nafter";
            let laid = layout_balanced(ui, text, &[], &style, 220.0, false);
            let kinds = laid.row_kinds();
            assert_eq!(
                kinds,
                [
                    RowKind::Normal,
                    RowKind::Code(0),
                    RowKind::Code(0),
                    RowKind::Code(0),
                    RowKind::Normal,
                    RowKind::Normal,
                ],
                "content and the in-fence blank are code, markers are not: {kinds:?}"
            );
        });
        output.textures_delta.clear();
    }
}

#[cfg(test)]
mod compact_list_tests {
    use super::*;
    #[test]
    fn compact_list_balancing_keeps_copy_and_link_targets() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style { size: 16.0, color: Color32::WHITE, secondary: Color32::GRAY, link: Color32::BLUE, mention: Color32::GREEN };
            let source = "•  *Open settings.* Follow the longer instructions at https://example.test/ before closing the app.\n•  Second item.";
            let laid = layout_balanced(ui, source, &[], &style, 250.0, false);
            assert_eq!(laid.unbroken_text(), plain(source, &[]));
            assert!(laid.galley.rows.iter().skip(1).any(|row| row.pos.x > 0.0));
            assert!(laid.shared.list_indent > 0.0);
            assert!(laid.links.iter().any(|(_,url)| url == "https://example.test/"));
            let cached = layout_balanced(ui, source, &[], &style, 250.0, false);
            assert_eq!(laid.galley.rect, cached.galley.rect);
            assert_eq!(laid.inserted(), cached.inserted());
            assert_eq!(laid.shared.list_indent, cached.shared.list_indent);
        });
        output.textures_delta.clear();
    }
    #[test]
    fn fenced_code_markers_do_not_reserve_prose_indentation() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::BLUE,
                mention: Color32::GREEN,
            };
            for source in [
                "```\n- option long enough to wrap over multiple rows\n```",
                "```\n12. option\n```\nordinary prose",
            ] {
                let built = build(ui, source, &[], &style, 180.0, false);
                assert_eq!(built.list_indent, 0.0, "{source}");
                // Only the code panel's20pt inset, never list indentation.
                assert_eq!(built.job.wrap.max_width, 160.0);
            }
            let built = build(
                ui,
                "```\n- option\n```\n•  prose item long enough to wrap",
                &[],
                &style,
                180.0,
                false,
            );
            assert!(built.list_indent > 0.0, "real prose lists still hang");
        });
        output.textures_delta.clear();
    }
    #[test]
    fn hanging_does_not_change_ordinary_or_rtl_text_layout() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert_eq!(
                list_indent(
                    ui,
                    &LayoutJob::simple_singleline(
                        "ordinary prose".into(),
                        theme::regular(16.0),
                        Color32::WHITE
                    ),
                    16.0,
                    250.0
                ),
                0.0
            );
            assert_eq!(
                list_indent(
                    ui,
                    &LayoutJob::simple_singleline(
                        "•  שלום עולם".into(),
                        theme::regular(16.0),
                        Color32::WHITE
                    ),
                    16.0,
                    250.0
                ),
                0.0
            );
        });
        output.textures_delta.clear();
    }
}

#[cfg(test)]
mod large_body_tests {
    use super::*;

    #[test]
    fn large_body_code_copy_survives_a_matching_synthetic_break() {
        let line = "a".repeat(1024);
        let literal = format!("{line}\nb");
        let text = format!("{line}b\n```\n{literal}\n```\n{}", "padding ".repeat(900));
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let rows = Arc::new(std::sync::Mutex::new(Vec::new()));
        ctx.add_plugin(crate::transcript::CopyAnnotator { rows: rows.clone() });
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let laid = layout_balanced(
                ui,
                &text,
                &[],
                &Style {
                    size: 16.0,
                    color: Color32::WHITE,
                    secondary: Color32::GRAY,
                    link: Color32::BLUE,
                    mention: Color32::WHITE,
                },
                400.0,
                false,
            );
            assert!(!laid.inserted().is_empty());
            rows.lock().unwrap().push(crate::transcript::Row {
                body: laid.unbroken_text().into(),
                broken: laid.broken_text().map(str::to_owned),
                inserted: laid.inserted().to_vec(),
                ..Default::default()
            });
            assert_eq!(
                code_blocks(&text).as_slice(),
                std::slice::from_ref(&literal)
            );
            assert!(
                crate::transcript::refine(&literal, &rows.lock().unwrap())
                    .is_some_and(|copy| copy != literal),
                "fixture must collide with selection cleanup"
            );
            crate::transcript::copy_literal(ui.ctx(), literal.clone());
        });
        assert!(
            output.platform_output.commands.iter().any(
                |command| matches!(command, egui::OutputCommand::CopyText(copy) if copy == &literal)
            ),
            "Copy code must keep every real newline"
        );
        output.textures_delta.clear();
    }

    #[test]
    fn pasted_log_uses_greedy_wrap_without_losing_copy_or_links() {
        let text = format!(
            "{}https://example.com/end",
            "synthetic function in Framework + 684 [0x18836fa34] waiting for an event\n"
                .repeat(6000)
        );
        assert!(text.len() > 400_000);
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let style = Style {
            size: 16.0,
            color: Color32::WHITE,
            secondary: Color32::GRAY,
            link: Color32::BLUE,
            mention: Color32::WHITE,
        };
        // Other parallel UI tests reinstall fonts process-wide. A changed
        // generation must invalidate this cache, so assert reuse only across
        // an unchanged epoch, just like the existing cache tests above.
        let mut stable = false;
        for _ in 0..8 {
            let generation = crate::theme::font_generation();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let laid = layout_balanced(ui, &text, &[], &style, 400.0, true);
                assert!(
                    laid.broken_text().is_none(),
                    "large bodies must not run the quadratic balancer"
                );
                assert!(laid.inserted().is_empty());
                assert_eq!(laid.unbroken_text(), text);
                assert!(laid.galley.rows.len() > 6000);
                assert_eq!(laid.links.last().unwrap().1, "https://example.com/end");
                let cached = layout_balanced(ui, &text, &[], &style, 400.0, true);
                if crate::theme::font_generation() == generation {
                    assert!(Arc::ptr_eq(&laid.shared, &cached.shared));
                    stable = true;
                }
            });
            output.textures_delta.clear();
            if stable {
                break;
            }
        }
        assert!(
            stable,
            "the font generation kept changing during cache checks"
        );
    }

    #[test]
    fn shaping_boundaries_keep_graphemes_and_reset_at_hard_breaks() {
        let exact = "a".repeat(1024);
        assert!(bounded_shape_breaks(&exact).is_empty());
        let text = format!("{exact}e\u{301}z\n{exact}界");
        let breaks = bounded_shape_breaks(&text);
        assert_eq!(breaks.len(), 2);
        assert_eq!(breaks[0].byte, 1024);
        assert_eq!(breaks[1].byte, text.len() - "界".len());
        assert!(
            breaks
                .iter()
                .all(|b| text.is_char_boundary(b.byte) && !b.hyphen)
        );
        assert!(text[breaks[0].byte..].starts_with("e\u{301}"));
    }

    #[test]
    fn long_single_line_and_unicode_keep_original_bytes() {
        let text = format!(
            "{}https://example.com/end",
            "ääkkönen synthetic words with no newline ".repeat(11000)
        );
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let laid = layout_balanced(
                ui,
                &text,
                &[],
                &Style {
                    size: 16.0,
                    color: Color32::WHITE,
                    secondary: Color32::GRAY,
                    link: Color32::BLUE,
                    mention: Color32::WHITE,
                },
                400.0,
                false,
            );
            assert!(
                !laid.inserted().is_empty(),
                "shaping paragraphs are bounded"
            );
            assert_eq!(laid.unbroken_text(), text);
            assert_eq!(
                laid.link_copy_text(laid.links.len() - 1),
                "https://example.com/end"
            );
        });
        output.textures_delta.clear();
    }
}

#[cfg(test)]
mod fence_language_regression {
    use super::*;
    #[test]
    fn language_label_is_metadata_for_display_and_copy() {
        let command = "git pull --ff-only && npm ci && WRANGLER_SEND_METRICS=false npm run deploy";
        for label in ["zsh", "bash", "rust", "c++", "c#", "shell-session"] {
            let text = format!("```{label}\n{command}\n```");
            assert_eq!(plain(&text, &[]).trim(), command);
            assert_eq!(code_blocks(&text), [command]);
            let spans = parse(&text, &[]);
            assert!(
                spans
                    .iter()
                    .any(|s| s.block && s.block_id == 0 && s.text == command)
            );
        }
    }
    #[test]
    fn metadata_indices_and_pill_geometry_follow_visible_blocks() {
        let text = "```zsh\n```\n```rust\nlet answer = 42;\n```\n```\nplain\n```";
        let details = code_block_details(text);
        assert_eq!(details[0].language.as_deref(), Some("rust"));
        assert_eq!(details[1].language, None);
        assert_eq!(
            details.iter().map(|b| b.text.clone()).collect::<Vec<_>>(),
            code_blocks(text)
        );
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let style = Style {
                size: 16.0,
                color: Color32::WHITE,
                secondary: Color32::GRAY,
                link: Color32::BLUE,
                mention: Color32::WHITE,
            };
            for width in [140.0, 220.0, 480.0] {
                let laid = layout_balanced(ui, text, &[], &style, width, false);
                for (index, panel) in code_panel_rects(
                    Rect::from_min_size(Pos2::ZERO, laid.galley.size()),
                    Pos2::ZERO,
                    &laid,
                    None,
                ) {
                    if let Some(language) = &details[index].language {
                        if let Some(pill) = put_code_language(
                            ui,
                            panel,
                            language,
                            Color32::DARK_GRAY,
                            Color32::WHITE,
                        ) {
                            assert!(!pill.intersects(copy_button_rect(panel)));
                            assert!(pill.right() <= panel.right());
                            let first = laid
                                .galley
                                .rows
                                .iter()
                                .zip(laid.row_kinds())
                                .find(|(_, kind)| **kind == RowKind::Code(index))
                                .unwrap()
                                .0;
                            assert!(pill.bottom() <= first.pos.y + 0.1);
                        }
                    }
                }
            }
        });
        output.textures_delta.clear();
    }
    #[test]
    fn labels_only_open_fences_and_inline_triples_stay_inline() {
        let text = "```zsh\nfirst\n```rust\nlast\n```\n```python\nsecond\n```";
        assert_eq!(code_blocks(text), ["first\n```rust\nlast", "second"]);
        assert_eq!(plain("```hello```", &[]), "hello");
        assert!(code_blocks("```hello```").is_empty());
        assert_eq!(code_blocks("```zsh\n```\n```bash\nnext\n```"), ["next"]);
        assert_eq!(code_blocks("```zsh\nnot closed"), ["not closed"]);
    }
}
