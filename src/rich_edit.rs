//! Reconstruct editable Markdown from the API's rich-text body, never from
//! the display renderer's WhatsApp-style markers or bullet glyphs.
use scraper::{ElementRef, Html, Node};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    Unavailable,
}
impl From<&str> for Error {
    fn from(_: &str) -> Self {
        Self::Unsupported
    }
}
impl From<String> for Error {
    fn from(_: String) -> Self {
        Self::Unsupported
    }
}

const LIMIT: usize = 256 * 1024;

pub fn markdown(rich: &str) -> Result<String, Error> {
    if rich.len() > LIMIT {
        return Err("Message is too large to prepare safely for editing".into());
    }
    let doc = Html::parse_fragment(rich);
    validate(doc.root_element(), 0, false)?;
    let source = children(doc.root_element(), 0)?
        .trim_matches('\n')
        .to_owned();
    let mut rendered = String::new();
    pulldown_cmark::html::push_html(
        &mut rendered,
        pulldown_cmark::Parser::new_ext(&source, pulldown_cmark::Options::ENABLE_STRIKETHROUGH),
    );
    let roundtrip = Html::parse_fragment(&rendered);
    if signature(doc.root_element(), 0)?.trim() != signature(roundtrip.root_element(), 0)?.trim() {
        return Err("This message's rich formatting cannot be reconstructed without a change; no edit started".into());
    }
    Ok(source)
}

fn validate(e: ElementRef<'_>, depth: usize, in_pre: bool) -> Result<(), Error> {
    if depth > 64 {
        return Err(Error::Unsupported);
    }
    let tag = e.value().name();
    for (name, value) in e.value().attrs() {
        let supported = match (tag, name) {
            ("a", "href" | "title") | ("ol", "start") => true,
            ("code", "class") if in_pre => value.split_whitespace().all(|v| {
                v.strip_prefix("language-").is_some_and(|s| {
                    !s.is_empty()
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
                })
            }),
            _ => false,
        };
        if !supported {
            return Err(Error::Unsupported);
        }
    }
    if in_pre && tag != "code" {
        return Err(Error::Unsupported);
    }
    for child in e.children().filter_map(ElementRef::wrap) {
        if tag == "code" {
            return Err(Error::Unsupported);
        }
        validate(child, depth + 1, tag == "pre")?;
    }
    Ok(())
}

// Semantic HTML signature, independent of synonymous tags and serialization.
// Keep code whitespace, list numbering, links and style boundaries. This is a
// CommonMark safety oracle, not a claim about a particular bridge renderer.
fn signature(e: ElementRef<'_>, depth: usize) -> Result<String, Error> {
    if depth > 64 {
        return Err("Message formatting is nested too deeply".into());
    }
    let tag = match e.value().name() {
        "b" => "strong",
        "i" => "em",
        "s" | "strike" => "del",
        "div" => "p",
        t => t,
    };
    if tag == "pre" {
        let text: String = e.text().collect();
        return Ok(format!(
            "<pre>{:?}</pre>",
            text.strip_suffix('\n').unwrap_or(&text)
        ));
    }
    if tag == "code" {
        return Ok(format!("<code>{:?}</code>", e.text().collect::<String>()));
    }
    let mut body = String::new();
    for child in e.children() {
        match child.value() {
            Node::Text(text) => {
                if text.trim().is_empty()
                    && text.contains('\n')
                    && matches!(tag, "html" | "body" | "ul" | "ol" | "blockquote")
                {
                    continue;
                }
                if text.trim().is_empty() && text.contains('\n') {
                    continue;
                }
                // CommonMark serializes a newline after <br> and before
                // nested block lists. Those are formatting whitespace, not
                // extra inline spaces. Do not drop spaces between words.
                let after_break = child
                    .prev_sibling()
                    .and_then(ElementRef::wrap)
                    .is_some_and(|e| e.value().name() == "br");
                let before_block =
                    child
                        .next_sibling()
                        .and_then(ElementRef::wrap)
                        .is_some_and(|e| {
                            matches!(
                                e.value().name(),
                                "ul" | "ol" | "blockquote" | "p" | "div" | "pre"
                            )
                        });
                let text = if after_break {
                    text.trim_start_matches(['\n', '\r'])
                } else {
                    text.as_ref()
                };
                let text = if before_block {
                    text.trim_end_matches(['\n', '\r', ' ', '\t'])
                } else {
                    text
                };
                let mut space = false;
                for ch in text.chars() {
                    if matches!(ch, ' ' | '\n' | '\r' | '\t' | '\u{c}') {
                        if !space {
                            body.push(' ');
                        }
                        space = true;
                    } else {
                        body.push(ch);
                        space = false;
                    }
                }
            }
            Node::Element(_) => {
                body.push_str(&signature(ElementRef::wrap(child).unwrap(), depth + 1)?)
            }
            Node::Comment(_) => {}
            _ => {}
        }
    }
    Ok(match tag {
        "html" | "body" | "span" => body,
        "p" => format!("\n\n{}\n\n", body.trim()),
        "br" => "<br>".into(),
        "a" => format!(
            "<a {:?} {:?}>{body}</a>",
            e.value().attr("href"),
            e.value().attr("title")
        ),
        "ol" => format!("<ol {}>{body}</ol>", e.value().attr("start").unwrap_or("1")),
        _ => format!("<{tag}>{body}</{tag}>"),
    })
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn children(element: ElementRef<'_>, depth: usize) -> Result<String, Error> {
    if depth > 64 {
        return Err("Message formatting is nested too deeply to edit safely".into());
    }
    let mut out = String::new();
    for child in element.children() {
        match child.value() {
            Node::Text(text) => {
                // Pretty-print newlines following a <br> are HTML whitespace,
                // not a second message line break.
                let text = if out.ends_with('\n') {
                    text.trim_start_matches(['\n', '\r'])
                } else {
                    text.as_ref()
                };
                out.push_str(&escape(text));
            }
            Node::Element(_) => {
                let child = ElementRef::wrap(child).unwrap();
                let rendered = element_markdown(child, depth + 1)?;
                if element.value().name() == "li" && matches!(child.value().name(), "ul" | "ol") {
                    let loose = element
                        .children()
                        .filter_map(ElementRef::wrap)
                        .any(|e| e.value().name() == "p");
                    out.truncate(out.trim_end_matches('\n').len());
                    out.push_str(if loose { "\n\n" } else { "\n" });
                    out.push_str(rendered.trim_start_matches('\n'));
                } else {
                    if rendered.starts_with("\n\n") && out.ends_with('\n') {
                        out.truncate(out.trim_end_matches('\n').len());
                    }
                    out.push_str(&rendered);
                }
            }
            Node::Comment(_) => {}
            _ => return Err("Unsupported message markup; no edit started".into()),
        }
    }
    Ok(out)
}

fn styled(text: String, marker: &str) -> String {
    let middle = text.trim();
    if middle.is_empty() {
        return text;
    }
    let left = text.len() - text.trim_start().len();
    let right = text.trim_end().len();
    format!(
        "{}{marker}{middle}{marker}{}",
        &text[..left],
        &text[right..]
    )
}

fn ticks(text: &str, min: usize) -> String {
    let longest = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(min.max(longest + 1))
}

fn element_markdown(e: ElementRef<'_>, depth: usize) -> Result<String, Error> {
    let tag = e.value().name();
    // These cannot be represented by the supported Markdown subset. Never
    // silently discard a style, active element, attachment or mention target.
    if e.value().attr("style").is_some()
        || e.value().attr("data-mention").is_some()
        || e.value().attr("data-user-id").is_some()
    {
        return Err("This message has unsupported rich formatting; no edit started".into());
    }
    match tag {
        "pre" => {
            let text: String = e.text().collect();
            let fence = ticks(&text, 3);
            let lang = e.children().filter_map(ElementRef::wrap).find(|c| c.value().name() == "code")
                .and_then(|c| c.value().attr("class"))
                .and_then(|c| c.split_whitespace().find_map(|c| c.strip_prefix("language-")))
                .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c)))
                .unwrap_or("");
            Ok(format!("\n\n{fence}{lang}\n{}{fence}\n\n", if text.ends_with('\n') { text } else { format!("{text}\n") }))
        }
        "code" => {
            if e.children().any(|c| matches!(c.value(), Node::Element(_))) {
                return Err("Unsupported nested code formatting; no edit started".into());
            }
            let text: String = e.text().collect();
            if text.contains('\n') { return Err("Multiline inline code cannot be edited without changing it".into()); }
            let fence = ticks(&text, 1);
            let pad = if text.starts_with('`') || text.ends_with('`') || (text.starts_with(' ') && text.ends_with(' ') && !text.trim().is_empty()) { " " } else { "" };
            Ok(format!("{fence}{pad}{text}{pad}{fence}"))
        }
        "ul" | "ol" => {
            let mut index = e.value().attr("start").unwrap_or("1").parse::<u32>()
                .map_err(|_| "Unsupported ordered-list start; no edit started")?;
            let mut out = String::from("\n\n");
            for child in e.children() {
                if matches!(child.value(), Node::Text(t) if t.trim().is_empty()) { continue; }
                let item = ElementRef::wrap(child).filter(|c| c.value().name() == "li")
                    .ok_or("Unsupported list contents; no edit started")?;
                if item.value().attr("value").is_some() { return Err("Custom list numbering cannot be preserved; no edit started".into()); }
                let body = children(item, depth + 1)?;
                let marker = if tag == "ul" { "- ".to_owned() } else { format!("{index}. ") };
                index = index.checked_add(1).ok_or("List numbering is too large")?;
                let indent = " ".repeat(marker.len());
                for (n, line) in body.trim_matches('\n').split('\n').enumerate() {
                    out.push_str(if n == 0 { &marker } else { &indent });
                    out.push_str(line);
                    out.push('\n');
                }
            }
            out.push('\n');
            Ok(out)
        }
        "script" | "style" | "iframe" | "object" | "img" | "table" | "u" | "input" =>
            Err("This message contains formatting that cannot round-trip through Markdown; no edit started".into()),
        _ => {
            let body = children(e, depth)?;
            Ok(match tag {
                "html" | "body" | "span" => body,
                "strong" | "b" => styled(body, "**"),
                "em" | "i" => styled(body, "_"),
                "del" | "s" | "strike" => styled(body, "~~"),
                "p" | "div" => format!("\n\n{}\n\n", body.trim_matches('\n')),
                "br" => "  \n".into(),
                "blockquote" => format!("\n\n{}\n\n", body.trim_matches('\n').split('\n').map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")),
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => format!("\n\n{} {}\n\n", "#".repeat(tag.as_bytes()[1] as usize - b'0' as usize), body.trim()),
                "hr" => "\n\n---\n\n".into(),
                "a" => {
                    let url = e.value().attr("href").ok_or("Link has no destination; no edit started")?;
                    if !(url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:"))
                        || url.starts_with("https://matrix.to/#/@")
                        || url.chars().any(|c| c.is_control() || c.is_whitespace() || "<>".contains(c)) {
                        return Err("Unsupported link or mention destination; no edit started".into());
                    }
                    let title = e.value().attr("title").map(|s| format!(" \"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))).unwrap_or_default();
                    format!("[{body}](<{url}>{title})")
                }
                _ => return Err(Error::Unsupported),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_hard_break_and_mixed_nested_list_are_editable() {
        assert_eq!(markdown("<p>one<br>two</p>").unwrap(), "one  \ntwo");
        assert_eq!(
            markdown("<ol start=3><li>outer<ul><li>inner</li></ul></li><li>next</li></ol>")
                .unwrap(),
            "3. outer\n   - inner\n4. next"
        );
    }
    #[test]
    fn inline_spaces_are_not_all_discarded_by_signature() {
        assert_ne!(
            signature(Html::parse_fragment("<p>one two</p>").root_element(), 0).unwrap(),
            signature(Html::parse_fragment("<p>onetwo</p>").root_element(), 0).unwrap()
        );
    }
    #[test]
    fn commonmark_rich_round_trips_are_identical() {
        fn rich(source: &str) -> String {
            let mut html = String::new();
            pulldown_cmark::html::push_html(
                &mut html,
                pulldown_cmark::Parser::new_ext(
                    source,
                    pulldown_cmark::Options::ENABLE_STRIKETHROUGH,
                ),
            );
            html
        }
        for source in [
            "hello\n\n- one\n- two",
            "**bold** and _italic_ and ~~strike~~",
            "**bold _italic_**",
            "[a link](https://example.com/a_(b))",
            "a  \nb\n\nsecond paragraph",
            "3. three\n4. four",
            "- outer\n  - inner\n  - another\n- next",
            "> quote\n>\n> another\n\nafter",
            "literal \\*stars\\* \\_underscore\\_ \\<tag\\> &amp;",
            "`a*b` and ``a`b``",
            "```rust\nlet x = 1;\n```",
            "````\n```\n````",
            "# heading\n\n---",
        ] {
            let original = rich(source);
            let edit = markdown(&original).unwrap();
            assert_eq!(rich(&edit), original, "source={source:?}, edit={edit:?}");
        }
    }
    #[test]
    fn lists_remain_editable_markdown_not_display_bullets() {
        assert_eq!(
            markdown("<p>hello</p><ul><li>one</li><li>two</li></ul>").unwrap(),
            "hello\n\n- one\n- two"
        );
    }
    #[test]
    fn styles_links_literals_and_code_do_not_become_each_other() {
        assert_eq!(
            markdown("<strong>bold</strong> <em>italic</em> <del>old</del> *literal*").unwrap(),
            "**bold** _italic_ ~~old~~ \\*literal\\*"
        );
        assert_eq!(markdown("<code>a`b</code>").unwrap(), "``a`b``");
        assert_eq!(
            markdown("<a href=\"https://example.com/a_(b)\">label</a>").unwrap(),
            "[label](<https://example.com/a_(b)>)"
        );
        assert_eq!(
            markdown("&amp;lt; &lt;tag&gt;").unwrap(),
            "\\&lt\\; \\<tag\\>"
        );
    }
    #[test]
    fn lossy_constructs_fail_instead_of_silently_changing_the_message() {
        for rich in [
            "<u>underlined</u>",
            "<table><tr><td>x</td></tr></table>",
            "<span style=\"color:red\">x</span>",
            "<a href=\"https://matrix.to/#/@user:server\">Name</a>",
            "<script>x()</script>",
            "<pre>first<br>second</pre>",
            "<pre><a href=\"https://example.com\">link</a></pre>",
            "<pre><code style=\"color:red\">code</code></pre>",
            "<ul><li style=\"color:red\">item</li></ul>",
            "<ol reversed start=\"3\"><li>three</li><li>two</li></ol>",
        ] {
            assert!(markdown(rich).is_err(), "{rich}");
        }
    }
}
