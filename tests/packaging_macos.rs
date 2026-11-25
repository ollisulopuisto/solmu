//! The macOS bundle carries the launch agent that starts Solmu in the tray
//! at login, exactly as fastframe-autostart registers it.
use std::fs;
use std::path::PathBuf;

/// A packaging file as committed. A Windows checkout may turn its line
/// endings into CRLF; the bundle is built on macOS, from LF.
fn packaging(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("packaging/macos")
        .join(file);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .replace("\r\n", "\n")
}

#[test]
fn the_bundle_carries_the_agent_that_starts_in_the_tray() {
    // The identifier from before the rename, which the linked device and the
    // keychain entries still go by.
    let id = "me.paolino.fastsapp";
    assert!(packaging("Info.plist").contains(&format!("<string>{id}</string>")));
    assert_eq!(
        packaging("autostart.plist"),
        fastframe_autostart::macos_agent_plist(id, fastframe_autostart::START_HIDDEN),
        "the committed agent is the one the crate expects"
    );
    let bundle = packaging("bundle.sh");
    assert!(bundle.contains(&format!("identifier={id}\n")));
    assert!(
        bundle.contains(
            r#""$here/autostart.plist" "$app/Contents/Library/LaunchAgents/$identifier.autostart.plist""#
        ),
        "bundle.sh copies it where SMAppService looks: {}",
        fastframe_autostart::macos_agent_plist_name(id)
    );
}

#[test]
fn whatsapp_links_are_registered_and_forwarded_without_a_shell() {
    let plist = packaging("Info.plist");
    assert!(plist.contains("CFBundleURLTypes"));
    assert!(plist.contains("<string>whatsapp</string>"));
    let launcher = packaging("BeeperLauncher.swift");
    assert!(launcher.contains("func application(_ application: NSApplication, open urls: [URL])"));
    assert!(launcher.contains("helper.arguments = [url]"));
    assert!(launcher.contains("forwarder.flushURLs()"));
    assert!(launcher.contains("url.scheme?.lowercased() == \"whatsapp\""));
}

#[cfg(target_os = "macos")]
#[test]
fn the_beeper_url_launcher_typechecks_on_macos() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packaging/macos/BeeperLauncher.swift");
    // Nix provides a pinned compiler explicitly; ordinary macOS builds keep
    // using the active Xcode toolchain. Missing compilers remain a hard failure.
    let mut command = match std::env::var_os("SOLMU_SWIFTC") {
        Some(compiler) => std::process::Command::new(compiler),
        None => {
            let mut command = std::process::Command::new("xcrun");
            command.arg("swiftc");
            command
        }
    };
    let output = command
        .arg("-typecheck")
        .arg(path)
        .output()
        .expect("Xcode Swift compiler");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nix_darwin_declares_the_swift_compiler_for_launcher_checks() {
    let flake =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("flake.nix")).unwrap();
    assert!(flake.contains("nativeCheckInputs = pkgs.lib.optionals isDarwin [ pkgs.swift ];"));
    assert!(flake.contains(r#"SOLMU_SWIFTC = "${pkgs.swift}/bin/swiftc";"#));
}
