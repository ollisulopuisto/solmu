//! Local presentation overrides. This module has no backend or network access.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub account: String,
    pub origin: Option<String>,
    pub backend_account: Option<String>,
    pub chat: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Appearance {
    Initials { text: String, colour: [u8; 3] },
    Image { file: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub identities: Vec<Identity>,
    pub appearance: Appearance,
}

/// Exact tuple matching avoids account collisions and stale source replacements.
pub fn resolve<'a>(entries: &'a [Entry], identities: &[Identity]) -> Option<&'a Appearance> {
    entries
        .iter()
        .rev()
        .find(|entry| entry.identities.iter().any(|key| identities.contains(key)))
        .map(|entry| &entry.appearance)
}

pub fn replace(
    entries: &mut Vec<Entry>,
    identities: Vec<Identity>,
    appearance: Option<Appearance>,
) {
    entries.retain_mut(|entry| {
        entry.identities.retain(|key| !identities.contains(key));
        !entry.identities.is_empty()
    });
    if let Some(appearance) = appearance {
        entries.push(Entry {
            identities,
            appearance,
        });
    }
}

pub fn initials(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .take(3)
        .collect()
}

pub fn image_path(state: &Path, file: &str) -> Option<PathBuf> {
    let hash = file.strip_suffix(".png")?;
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| state.join("custom-avatars").join(file))
}

/// Read only a bounded local file. The picker never accepts URLs.
pub fn import(path: &Path) -> Result<(String, Vec<u8>), String> {
    let metadata = std::fs::metadata(path).map_err(|_| "Could not open image")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("Choose an image file up to 10 MB".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "Could not open image")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read image")?;
    let png = normalize(&bytes)?;
    let hash: String = Sha256::digest(&png)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((format!("{hash}.png"), png))
}

pub fn normalize(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Image is larger than 10 MB".into());
    }
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unsupported image")?;
    let (width, height) = reader.into_dimensions().map_err(|_| "Invalid image")?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err("Image dimensions are too large".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unsupported image")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_PIXELS * 4);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| "Invalid image")?;
    use image::ImageDecoder;
    let orientation = decoder
        .orientation()
        .map_err(|_| "Invalid image orientation")?;
    let mut decoded =
        image::DynamicImage::from_decoder(decoder).map_err(|_| "Could not decode image")?;
    decoded.apply_orientation(orientation);
    // Crop before scaling: scaling a thin rectangle to fill a square can
    // otherwise allocate gigabytes despite bounded decoded dimensions.
    let side = decoded.width().min(decoded.height());
    let square = decoded
        .crop_imm(
            (decoded.width() - side) / 2,
            (decoded.height() - side) / 2,
            side,
            side,
        )
        .resize_exact(512, 512, image::imageops::FilterType::Lanczos3);
    let mut png = Cursor::new(Vec::new());
    square
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "Could not prepare avatar")?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(w: u32, h: u32) -> Vec<u8> {
        let image = image::DynamicImage::new_rgba8(w, h);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }
    #[test]
    fn imports_are_decoded_bounded_and_reencoded_not_passed_through() {
        assert!(normalize(b"not an image").is_err());
        assert!(normalize(&vec![0; MAX_BYTES as usize + 1]).is_err());
        assert!(normalize(&png(MAX_DIMENSION + 1, 1)).is_err());
        let mut source = png(900, 600);
        source.extend_from_slice(b"private trailing metadata must not survive");
        let clean = normalize(&source).unwrap();
        let decoded = image::load_from_memory(&clean).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (512, 512));
        assert!(!clean.windows(8).any(|s| s == b"metadata"));
        assert_eq!(clean, normalize(&clean).unwrap());
    }
    #[test]
    fn degenerate_aspect_ratios_are_cropped_before_upscaling() {
        for (width, height) in [(8192, 1), (1, 8192)] {
            let clean = normalize(&png(width, height)).unwrap();
            let decoded = image::load_from_memory(&clean).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (512, 512));
        }
    }
    #[test]
    fn image_paths_cannot_escape_owned_state() {
        assert!(image_path(Path::new("/state"), "../../secret.png").is_none());
        assert!(
            image_path(Path::new("/state"), &format!("{}.png", "a".repeat(64)))
                .unwrap()
                .starts_with("/state/custom-avatars")
        );
        assert_eq!(initials("<A>\nK\u{202e}!!!😀"), "AK");
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    fn key(account: &str, origin: &str) -> Identity {
        Identity {
            account: account.into(),
            origin: Some(origin.into()),
            backend_account: Some("bridge".into()),
            chat: "same-id".into(),
        }
    }
    #[test]
    fn exact_identities_and_linked_aliases_survive_restart_without_cross_account_leaks() {
        let first = key("one", "source-one");
        let mut linked = first.clone();
        linked.chat = "linked-id".into();
        let style = Appearance::Initials {
            text: "AK".into(),
            colour: [115, 89, 165],
        };
        let mut entries = Vec::new();
        replace(
            &mut entries,
            vec![first.clone(), linked.clone()],
            Some(style.clone()),
        );
        let persisted = serde_json::to_vec(&entries).unwrap();
        let mut restored: Vec<Entry> = serde_json::from_slice(&persisted).unwrap();
        assert_eq!(
            resolve(&restored, std::slice::from_ref(&first)),
            Some(&style)
        );
        assert_eq!(
            resolve(&restored, std::slice::from_ref(&linked)),
            Some(&style)
        );
        assert!(resolve(&restored, &[key("two", "source-one")]).is_none());
        assert!(resolve(&restored, &[key("one", "replaced-source")]).is_none());
        replace(&mut restored, vec![first.clone()], None);
        assert!(resolve(&restored, &[first]).is_none());
        assert_eq!(
            resolve(&restored, std::slice::from_ref(&linked)),
            Some(&style),
            "an unlinked identity's reset leaves the other member alone"
        );
        replace(&mut restored, vec![linked], None);
        assert!(restored.is_empty());
    }
}
