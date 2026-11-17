//! The full package identity, including the manual build suffix.
use std::sync::OnceLock;

static VERSION: OnceLock<String> = OnceLock::new();

/// Load once during startup, before the event loop or any UI rendering.
pub fn initialize() {
    VERSION.get_or_init(|| {
        let packaged = std::env::current_exe().ok().and_then(|exe| {
            let contents = exe.parent()?.parent()?;
            // Only app bundles carry this packaging-owned display metadata.
            (contents.file_name()? == "Contents")
                .then(|| std::fs::read_to_string(contents.join("Resources/build-version.txt")).ok())
                .flatten()
        });
        choose(packaged.as_deref()).to_owned()
    });
}

/// Rendering only reads memory, even if an embedding did not initialize us.
pub fn display() -> &'static str {
    VERSION
        .get()
        .map(String::as_str)
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn choose(packaged: Option<&str>) -> &str {
    packaged
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_package_identity_survives_base_version_and_whitespace() {
        assert_eq!(choose(Some("26.11.1-manual.53\n")), "26.11.1-manual.53");
        assert_eq!(choose(Some("2026.11.17.53")), "2026.11.17.53");
        assert_eq!(choose(Some("26.11.2")), "26.11.2");
        for missing in [None, Some(""), Some("\n")] {
            assert_eq!(choose(missing), env!("CARGO_PKG_VERSION"));
        }
    }
}
