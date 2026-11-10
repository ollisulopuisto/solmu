//! Validation at the boundary between message content and desktop handlers.

use std::path::Path;

/// Reject single or nested percent-encoded ASCII control characters (`%00`..=`%1F`, `%7F`),
/// e.g. `%2500` (double-encoded null) that could inject arguments into desktop handlers.
fn has_encoded_control(value: &str) -> bool {
    let mut current = value.as_bytes().to_vec();
    // Decode bounded nested percent-encoding layers to detect hidden control characters.
    for _ in 0..16 {
        if current.windows(3).any(|w| {
            w[0] == b'%'
                && (w[1] as char).to_digit(16).and_then(|h1| {
                    (w[2] as char)
                        .to_digit(16)
                        .map(|h2| matches!((h1 << 4) | h2, 0..=0x1f | 0x7f))
                }) == Some(true)
        }) {
            return true;
        }
        let mut next = Vec::with_capacity(current.len());
        let mut i = 0;
        let mut changed = false;
        while i < current.len() {
            if current[i] == b'%'
                && let Some(b) = current.get(i + 1..i + 3).and_then(|h| {
                    std::str::from_utf8(h)
                        .ok()
                        .and_then(|s| u8::from_str_radix(s, 16).ok())
                })
            {
                next.push(b);
                i += 3;
                changed = true;
                continue;
            }
            next.push(current[i]);
            i += 1;
        }
        if !changed {
            return false;
        }
        current = next;
    }
    // Decoding was still progressing at the layer bound: a deeper-nested
    // control could survive, so reject rather than pass it through.
    true
}

/// Preview metadata may supply a bare host, but never a desktop URI scheme.
pub fn preview_url(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.contains('\\') || has_encoded_control(value) {
        return None;
    }
    let value = value.trim();
    let url = match reqwest::Url::parse(value) {
        Ok(url) => url,
        Err(_) => reqwest::Url::parse(&format!("https://{value}")).ok()?,
    };
    // Reject userinfo (credentials) to prevent phishing and spoofing via userinfo authority tricks.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then(|| url.to_string())
}

/// Check again at the action boundary, including previews already in archives.
pub fn external_url(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) || value.contains('\\') || has_encoded_control(value) {
        return None;
    }
    let value = value.trim();
    let url = reqwest::Url::parse(value).ok()?;
    // Reject userinfo (credentials) to prevent phishing and credential injection.
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url.to_string()),
        "mailto" if !url.path().is_empty() => Some(url.to_string()),
        _ => None,
    }
}

/// Image formats decoded by Solmu's in-process preview.
pub fn can_preview_image(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "webp"
    )
}

/// The code of a WhatsApp group invite link such as
/// `https://chat.whatsapp.com/AbCd123` or `whatsapp://chat/?code=AbCd123`,
/// which Solmu opens itself.
pub fn group_invite_code(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = reqwest::Url::parse(trimmed)
        && matches!(url.scheme(), "whatsapp" | "whatsapp-send")
    {
        let host = url.host_str().unwrap_or_default();
        if host.eq_ignore_ascii_case("chat") {
            for (key, val) in url.query_pairs() {
                if key == "code" {
                    if is_valid_group_invite_code(&val) {
                        return Some(val.into_owned());
                    }
                    if let Some(code) = http_group_invite_code(&val) {
                        return Some(code);
                    }
                }
            }
        } else if host.eq_ignore_ascii_case("invite")
            && let Some(segments) = url.path_segments()
        {
            let mut iter = segments.filter(|s| !s.is_empty());
            if let Some(code) = iter.next()
                && iter.next().is_none()
                && is_valid_group_invite_code(code)
            {
                return Some(code.to_owned());
            }
        }
        return None;
    }
    http_group_invite_code(trimmed)
}

fn http_group_invite_code(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let url_to_parse =
        if lower.starts_with("chat.whatsapp.com/") || lower.starts_with("www.chat.whatsapp.com/") {
            format!("https://{trimmed}")
        } else {
            trimmed.to_owned()
        };
    let url = reqwest::Url::parse(&url_to_parse).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    if !host.eq_ignore_ascii_case("chat.whatsapp.com")
        && !host.eq_ignore_ascii_case("www.chat.whatsapp.com")
    {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
    let first = segments.next()?;
    let code = if first == "invite" {
        segments.next()?
    } else {
        first
    };
    (segments.next().is_none() && is_valid_group_invite_code(code)).then(|| code.to_owned())
}

fn is_valid_group_invite_code(code: &str) -> bool {
    (10..=40).contains(&code.len())
        && code
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

/// A WhatsApp call link such as `https://call.whatsapp.com/voice/AbCd123`:
/// its token, and whether it is a video call's (`/video/`) rather than a
/// voice call's (`/voice/`, or `/audio/` as whatsapp-rust writes it).
/// Solmu joins these itself.
pub fn call_link(value: &str) -> Option<(String, bool)> {
    let url = reqwest::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.host_str()?.eq_ignore_ascii_case("call.whatsapp.com")
    {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
    let video = match segments.next()? {
        "voice" | "audio" => false,
        "video" => true,
        _ => return None,
    };
    let token = segments.next()?;
    (segments.next().is_none()
        && (8..=64).contains(&token.len())
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')))
    .then(|| (token.to_owned(), video))
}

/// Only common document/media formats go to their desktop application.
/// Unknown files, scripts, installers and application bundles can be revealed
/// in their folder instead. Sender-provided MIME types cannot grant permission.
pub(crate) fn is_reserved_dos_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || (lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.len() == 4
            && lower.as_bytes()[3].is_ascii_digit()
}

pub fn can_open_attachment(path: &Path) -> bool {
    // Reject filenames with trailing whitespace or periods which can mask extensions or alter execution.
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    if name.ends_with('.') || name.ends_with(' ') || name.contains(':') {
        return false;
    }
    // Reject Windows DOS device names (e.g. CON, PRN, AUX, NUL, COM1..9, LPT1..9)
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if is_reserved_dos_name(stem) {
        return false;
    }
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "bmp"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "avif"
            | "mp4"
            | "m4v"
            | "mov"
            | "webm"
            | "mkv"
            | "3gp"
            | "mp3"
            | "m4a"
            | "aac"
            | "ogg"
            | "opus"
            | "wav"
            | "flac"
            | "pdf"
            | "txt"
            | "log"
            | "csv"
            | "docx"
            | "xlsx"
            | "pptx"
            | "odt"
            | "ods"
            | "odp"
            | "rtf"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_links_are_recognised_and_nothing_else() {
        let token = "AbCdEf1234567890";
        assert_eq!(
            call_link(&format!("https://call.whatsapp.com/voice/{token}")),
            Some((token.to_owned(), false))
        );
        assert_eq!(
            call_link(&format!("https://call.whatsapp.com/audio/{token}?utm=1")),
            Some((token.to_owned(), false))
        );
        assert_eq!(
            call_link(&format!("https://CALL.whatsapp.com/video/{token}/")),
            Some((token.to_owned(), true))
        );
        for link in [
            "https://call.whatsapp.com/",
            "https://call.whatsapp.com/voice/",
            "https://call.whatsapp.com/voice/short",
            "https://call.whatsapp.com/screen/AbCdEf1234567890",
            "https://call.whatsapp.com.evil.example/voice/AbCdEf1234567890",
            "https://call.whatsapp.com/voice/AbCdEf1234567890/extra",
            "https://call.whatsapp.com/voice/AbCd%20Ef1234567890",
            "https://chat.whatsapp.com/AbCdEf1234567890",
            "whatsapp://call.whatsapp.com/voice/AbCdEf1234567890",
        ] {
            assert_eq!(call_link(link), None, "{link}");
        }
    }

    #[test]
    fn group_invite_links_are_recognised_and_nothing_else() {
        let code = "AbCdEf1234567890XyZ";
        for link in [
            format!("https://chat.whatsapp.com/{code}"),
            format!("https://www.chat.whatsapp.com/{code}"),
            format!("https://chat.whatsapp.com/invite/{code}"),
            format!("http://CHAT.whatsapp.com/{code}/"),
            format!("https://chat.whatsapp.com/{code}?utm=1"),
            format!("chat.whatsapp.com/{code}"),
            format!("www.chat.whatsapp.com/{code}"),
            format!("whatsapp://chat/?code={code}"),
            format!("whatsapp://chat/?code=https%3A%2F%2Fchat.whatsapp.com%2F{code}"),
            format!("whatsapp://chat/?code=https://chat.whatsapp.com/{code}"),
            format!("whatsapp://chat?code={code}"),
            format!("whatsapp-send://chat?code={code}"),
            format!("whatsapp://invite/{code}"),
        ] {
            assert_eq!(group_invite_code(&link).as_deref(), Some(code), "{link}");
        }
        for link in [
            "https://chat.whatsapp.com/",
            "https://chat.whatsapp.com/short",
            "https://evil.example/AbCdEf1234567890XyZ",
            "https://chat.whatsapp.com.evil.example/AbCdEf1234567890XyZ",
            "https://chat.whatsapp.com/AbCdEf1234567890XyZ/extra",
            "https://chat.whatsapp.com/AbCd-Ef1234567890XyZ",
            "whatsapp://chat.whatsapp.com/AbCdEf1234567890XyZ",
            "whatsapp://chat/?code=https://evil.example/AbCdEf1234567890XyZ",
            "whatsapp://chat/?code=https%3A%2F%2Fevil.example%2FAbCdEf1234567890XyZ",
        ] {
            assert_eq!(group_invite_code(link), None, "{link}");
        }
    }

    #[test]
    fn previews_and_actions_reject_desktop_handlers() {
        for value in [
            "file:///tmp/example",
            "ms-appinstaller://example",
            "javascript:alert(1)",
            "data:text/html,example",
            "shell:AppsFolder",
            "https:\\example.com",
            "https://example.com\n",
            "mailto:test@example.com",
        ] {
            assert_eq!(preview_url(value), None, "{value}");
            if !value.starts_with("mailto:") {
                assert_eq!(external_url(value), None, "{value}");
            }
        }
        assert_eq!(
            preview_url("example.com/page"),
            Some("https://example.com/page".into())
        );
        assert_eq!(
            preview_url("HTTPS://example.com"),
            Some("https://example.com/".into())
        );
        assert_eq!(
            external_url("mailto:test@example.com"),
            Some("mailto:test@example.com".into())
        );
        assert_eq!(
            external_url("  https://example.com/trimmed  "),
            Some("https://example.com/trimmed".into())
        );

        // Percent-encoded control characters are rejected; valid percent encodings are kept
        for bad in [
            "https://example.com/%00evil",
            "https://example.com/%0d%0ainjection",
            "https://example.com/%1b]0;cmd%07",
            "https://example.com/%7f",
            "mailto:user@example.com%00payload",
            "mailto:user@example.com%0D%0Acc:admin@example.com",
            // Nested / double / multi percent-encoded control characters
            "https://example.com/%2500evil",
            "https://example.com/%250d%250ainjection",
            "https://example.com/%252500nested",
            "https://example.com/%25252500quad",
            "https://example.com/%2525250dquad_cr",
            "https://example.com/%2525252500deep",
            "https://example.com/%257f",
            "mailto:user@example.com%2500payload",
            "mailto:user@example.com%25252500payload",
            "mailto:user@example.com%250D%250Acc:admin@example.com",
        ] {
            assert_eq!(preview_url(bad), None, "{bad}");
            assert_eq!(external_url(bad), None, "{bad}");
        }

        // Decode-cap boundary: nested(l) applies l extra percent-encoding layers
        // around %00. The loop checks 16 layers, so l=15 is still caught inside
        // the budget and l=16/17 must be rejected by failing closed at the bound
        // rather than passing a deeper-nested control through.
        fn nested(layers: u32) -> String {
            let mut s = "%00".to_string();
            for _ in 0..layers {
                s = s.replace('%', "%25");
            }
            s
        }
        for layers in [15u32, 16, 17, 40] {
            let bad = format!("https://example.com/{}", nested(layers));
            assert_eq!(preview_url(&bad), None, "{bad}");
            assert_eq!(external_url(&bad), None, "{bad}");
        }
        assert_eq!(
            external_url("https://example.com/hello%20world"),
            Some("https://example.com/hello%20world".into())
        );
        assert_eq!(
            external_url("https://example.com/hello%2520world"),
            Some("https://example.com/hello%2520world".into())
        );
    }

    #[test]
    fn only_supported_raster_images_open_in_the_native_preview() {
        for name in [
            "photo.JPG",
            "photo.jpeg",
            "photo.png",
            "photo.webp",
            "photo.gif",
        ] {
            assert!(can_preview_image(Path::new(name)), "{name}");
        }
        for name in [
            "photo.bmp",
            "photo.heic",
            "photo.avif",
            "photo.jpg.exe",
            "photo",
        ] {
            assert!(!can_preview_image(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn only_recognized_document_and_media_extensions_can_launch() {
        for name in [
            "photo.JPG",
            "photo.jpeg",
            "photo.png",
            "photo.gif",
            "photo.webp",
            "photo.bmp",
            "photo.tif",
            "photo.tiff",
            "photo.heic",
            "photo.heif",
            "photo.avif",
            "movie.mp4",
            "movie.m4v",
            "movie.mov",
            "movie.webm",
            "movie.mkv",
            "movie.3gp",
            "track.mp3",
            "track.m4a",
            "track.aac",
            "voice.ogg",
            "voice.opus",
            "sound.wav",
            "audio.flac",
            "document.pdf",
            "notes.txt",
            "debug.log",
            "table.csv",
            "document.docx",
            "sheet.xlsx",
            "presentation.pptx",
            "document.odt",
            "sheet.ods",
            "slides.odp",
            "document.rtf",
        ] {
            assert!(can_open_attachment(Path::new(name)), "{name}");
        }
        for name in [
            "invoice.pdf.exe",
            "setup.msi",
            "script.ps1",
            "script.js",
            "script.bat",
            "script.cmd",
            "script.vbs",
            "screen.scr",
            "link.lnk",
            "link.url",
            "app.desktop",
            "app.AppImage",
            "app.app",
            "script.command",
            "no-extension",
            "macro.docm",
            "file.pdf ",
            "file.pdf:payload.exe",
            "file.unknown",
            "payload.sh",
            "binary.bin",
            "library.so",
            "library.dylib",
            "library.dll",
            "malware.pif",
            "malware.com",
            "malware.hta",
            "malware.cpl",
            "malware.wsf",
            "malware.vbe",
            "document.docx.",
            "document.pdf ",
            "document.pdf:stream",
            "file.png:$DATA",
            "CON.txt",
            "con.png",
            "prn.pdf",
            "AUX.docx",
            "NUL.jpg",
            "com1.txt",
            "lpt1.pdf",
        ] {
            assert!(!can_open_attachment(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn external_and_preview_url_edge_cases() {
        // Invalid or empty URLs
        assert_eq!(external_url(""), None);
        assert_eq!(external_url("   "), None);
        assert_eq!(preview_url(""), None);
        assert_eq!(preview_url("   "), None);

        // Control characters
        assert_eq!(external_url("https://example.com/\0"), None);
        assert_eq!(external_url("https://example.com/\tpath"), None);
        assert_eq!(external_url("https://example.com/\r\npath"), None);
        assert_eq!(preview_url("https://example.com/\0"), None);
        assert_eq!(preview_url("https://example.com/\tpath"), None);

        // Disallowed schemes
        for scheme in [
            "ftp://example.com",
            "ssh://example.com",
            "tel:+1234567890",
            "smb://example.com",
            "view-source:https://example.com",
        ] {
            assert_eq!(external_url(scheme), None, "{scheme}");
            assert_eq!(preview_url(scheme), None, "{scheme}");
        }

        // Mailto without recipient or with backslash
        assert_eq!(external_url("mailto:"), None);
        assert_eq!(external_url("mailto:\\evil@example.com"), None);

        // URLs with credentials/userinfo are rejected to prevent phishing/credential injection
        for url_with_userinfo in [
            "https://user:password@example.com",
            "https://admin@example.com/login",
            "http://user:pass@127.0.0.1/",
            "https://evil.com@legit.example.com",
        ] {
            assert_eq!(external_url(url_with_userinfo), None, "{url_with_userinfo}");
            assert_eq!(preview_url(url_with_userinfo), None, "{url_with_userinfo}");
        }

        // External url accepts valid http, https, and mailto
        assert_eq!(
            external_url("https://example.com:8080/path?query=val#hash"),
            Some("https://example.com:8080/path?query=val#hash".into())
        );
        assert_eq!(
            external_url("http://example.org/"),
            Some("http://example.org/".into())
        );
        assert_eq!(
            external_url("mailto:user.name+tag@example.co.uk"),
            Some("mailto:user.name+tag@example.co.uk".into())
        );

        // Preview URL bare domain parsing vs scheme
        assert_eq!(
            preview_url("sub.domain.example.org/test"),
            Some("https://sub.domain.example.org/test".into())
        );
    }

    #[test]
    fn group_invite_and_call_link_edge_cases() {
        // Empty or whitespace
        assert_eq!(group_invite_code(""), None);
        assert_eq!(group_invite_code("   "), None);
        assert_eq!(call_link(""), None);
        assert_eq!(call_link("   "), None);

        // Invalid host or path for invite
        assert_eq!(
            group_invite_code("https://other.com/AbCdEf1234567890XyZ"),
            None
        );
        assert_eq!(
            group_invite_code("whatsapp://other/?code=AbCdEf1234567890XyZ"),
            None
        );

        // Invalid characters in call token
        assert_eq!(
            call_link("https://call.whatsapp.com/voice/invalid!token!123"),
            None
        );
        assert_eq!(
            call_link("https://call.whatsapp.com/video/invalid$token$123"),
            None
        );

        // Unknown call media type
        assert_eq!(
            call_link("https://call.whatsapp.com/stream/AbCdEf1234567890"),
            None
        );
    }
}
