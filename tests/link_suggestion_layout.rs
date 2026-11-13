//! Portable source contract; native layout still needs the owner's pixel review.
#[test]
fn chat_suggestions_stay_bounded_after_the_filters_menu_is_retired() {
    let header = include_str!("../src/ui/chats.rs");
    assert!(!header.contains("let suggestions = app.link_suggestions();"));
    let source = include_str!("../src/ui/merged.rs");
    let cards = source
        .split("fn suggestion_cards(")
        .nth(1)
        .expect("chat-level suggestion cards remain available")
        .split("\n#[cfg(test)]")
        .next()
        .unwrap();
    let compact: String = cards.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains(".filter(|s|s.involves(&chat.id)).take(3)"));
    assert!(cards.contains("app.accept_link_suggestion("));
    assert!(cards.contains("app.dismiss_link_suggestion("));
    assert!(cards.contains("Same person?"));
}
