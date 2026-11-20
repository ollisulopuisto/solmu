//! Shared local chat-name search data. Display titles stay unchanged.
pub fn terms(title: &str, members: impl IntoIterator<Item = String>) -> String {
    let mut text = title.to_owned();
    for name in members {
        if !name.trim().is_empty() {
            text.push('\n');
            text.push_str(&name);
        }
    }
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_group_matches_full_member_names_not_just_its_subject() {
        let searchable = terms(
            "Weekend plans",
            ["Hanna Mäkinen".into(), "Lauri Ojanen".into()],
        );
        assert!(searchable.contains("Hanna Mäkinen"));
        assert!(searchable.contains("Lauri Ojanen"));
        assert!(searchable.starts_with("Weekend plans"));
    }
}
