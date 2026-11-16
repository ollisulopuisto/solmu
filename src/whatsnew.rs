//! "What's New": parses the bundled CHANGELOG.md and picks the sections
//! to show after an upgrade or on demand from Help.
//!
//! Sections newer than the last acknowledged version show automatically
//! once after an upgrade; the latest sections show on demand. The
//! `Unreleased` section is working notes for the next release and never
//! shows in the app.

/// One changelog section: a released version and its bullet lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub version: [u32; 3],
    pub items: Vec<String>,
}

/// The changelog bundled at build time.
pub const CHANGELOG: &str = include_str!("../CHANGELOG.md");

/// Parses `major.minor.patch`, tolerating a leading `v` and extra
/// suffixes after the patch number.
pub fn version(text: &str) -> Option<[u32; 3]> {
    let text = text.trim().trim_start_matches('v');
    let mut parts = text.split(['.', '-', '+']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some([major, minor, patch])
}

/// The running build's version.
pub fn current() -> [u32; 3] {
    version(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is major.minor.patch")
}

/// Released sections, newest first. `Unreleased` is skipped.
pub fn sections() -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in CHANGELOG.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some(version) = version(heading) {
                out.push(Section {
                    version,
                    items: Vec::new(),
                });
            }
            // Any other heading (Unreleased, comments) ends the section
            // above without starting one.
        } else if let Some(item) = line.strip_prefix("- ")
            && let Some(section) = out.last_mut()
        {
            section.items.push(item.trim().to_owned());
        }
    }
    out
}

/// Sections released after `last`, newest first. `None` (no version
/// acknowledged yet) means everything is new.
pub fn newer_than(sections: &[Section], last: Option<[u32; 3]>) -> Vec<&Section> {
    sections
        .iter()
        .filter(|section| last.is_none_or(|last| section.version > last))
        .collect()
}

/// The `count` most recent sections, for browsing from Help.
pub fn latest(sections: &[Section], count: usize) -> Vec<&Section> {
    sections.iter().take(count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_parses_and_matches_cargo() {
        assert_eq!(
            current(),
            version(env!("CARGO_PKG_VERSION")).unwrap(),
            "CARGO_PKG_VERSION must stay major.minor.patch"
        );
    }

    #[test]
    fn current_version_is_calver_year_month_number() {
        let [year, month, number] = current();
        assert!(year >= 26, "the year is two digits, from 2026: {year}");
        assert!((1..=12).contains(&month), "the month is 1-12: {month}");
        assert!(number >= 1, "the number counts releases in the month from 1");
    }

    #[test]
    fn parses_versions_with_prefixes_and_suffixes() {
        assert_eq!(version("0.20.2"), Some([0, 20, 2]));
        assert_eq!(version("v1.2.3"), Some([1, 2, 3]));
        assert_eq!(version("1.2.3-rc1"), Some([1, 2, 3]));
        assert_eq!(version("Unreleased"), None);
    }

    #[test]
    fn sections_skip_unreleased_and_keep_newest_first() {
        let parsed = sections();
        assert!(parsed.iter().all(|s| s.items.iter().all(|i| !i.is_empty())));
        for pair in parsed.windows(2) {
            assert!(pair[0].version >= pair[1].version, "newest first");
        }
    }

    #[test]
    fn newer_than_filters_and_none_means_everything() {
        let sections = vec![
            Section {
                version: [0, 21, 0],
                items: vec!["a".into()],
            },
            Section {
                version: [0, 20, 2],
                items: vec!["b".into()],
            },
        ];
        assert_eq!(newer_than(&sections, Some([0, 20, 2])).len(), 1);
        assert_eq!(newer_than(&sections, Some([0, 21, 0])).len(), 0);
        assert_eq!(newer_than(&sections, None).len(), 2);
        assert_eq!(latest(&sections, 1)[0].version, [0, 21, 0]);
    }
}
