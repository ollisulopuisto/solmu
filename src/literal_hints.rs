/// Remove only bridge literal-marker hints, preserving native separators.
/// Borrow ordinary text so unhinted search keeps its allocation-free path.
pub fn without_literal_hints(text: &str) -> std::borrow::Cow<'_, str> {
    if !["*\u{2063}", "_\u{2063}", "~\u{2063}", "`\u{2063}"]
        .iter()
        .any(|pair| text.contains(pair))
    {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        out.push(ch);
        if matches!(ch, '*' | '_' | '~' | '`') && chars.peek() == Some(&'\u{2063}') {
            chars.next();
        }
    }
    std::borrow::Cow::Owned(out)
}
