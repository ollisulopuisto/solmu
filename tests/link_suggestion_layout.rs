//! Portable source contract; native layout still needs the owner's pixel review.
#[test]
fn suggestion_menu_only_builds_visible_fixed_height_rows() {
    let source = include_str!("../src/ui/chats.rs");
    let menu = source
        .split("let suggestions = app.link_suggestions();")
        .nth(1)
        .unwrap()
        .split("Action::ToggleSidebar")
        .next()
        .unwrap();
    let compact: String = menu.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains(".show_rows(ui,row_height,suggestions.len()"));
    assert!(menu.contains("&suggestions[range]"));
    assert!(menu.contains(".truncate()"));
    assert!(menu.contains(".on_hover_text("));
    assert!(menu.contains("names.get(&member.chat)"));
    assert!(!menu.contains("app.chat("));
}
