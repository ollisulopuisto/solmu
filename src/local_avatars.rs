//! Local presentation overrides. This module has no backend or network access.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

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

/// Where the square crop window sits on the stored source image. x/y are the
/// window centre as a fraction of the source's width/height; zoom scales the
/// window, 1.0 fits the largest centred square.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Crop {
    /// The centre crop entries had before cropping existed.
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.5,
            zoom: 1.0,
        }
    }
}

pub const MIN_ZOOM: f32 = 1.0;
pub const MAX_ZOOM: f32 = 8.0;
/// A detected face's bounding side is framed with this much room around it.
pub const FACE_HEADROOM: f32 = 2.2;

fn sane_aspect(aspect: f32) -> f32 {
    if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        1.0
    }
}

fn sane_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        zoom.clamp(MIN_ZOOM, MAX_ZOOM)
    } else {
        1.0
    }
}

/// The window's half size as a fraction of the source's width and height.
fn half_window(aspect: f32, zoom: f32) -> (f32, f32) {
    let aspect = sane_aspect(aspect);
    let zoom = sane_zoom(zoom);
    (
        (1.0 / aspect).min(1.0) / (2.0 * zoom),
        aspect.min(1.0) / (2.0 * zoom),
    )
}

/// The window's full size as a fraction of the source's width and height.
pub fn crop_window(aspect: f32, zoom: f32) -> (f32, f32) {
    let (half_w, half_h) = half_window(aspect, zoom);
    (2.0 * half_w, 2.0 * half_h)
}

/// Keep the window square, inside the image and the zoom within bounds.
pub fn clamp_crop(aspect: f32, crop: Crop) -> Crop {
    let zoom = sane_zoom(crop.zoom);
    let (half_w, half_h) = half_window(aspect, zoom);
    let x = if crop.x.is_finite() {
        crop.x.clamp(half_w, 1.0 - half_w)
    } else {
        0.5
    };
    let y = if crop.y.is_finite() {
        crop.y.clamp(half_h, 1.0 - half_h)
    } else {
        0.5
    };
    Crop { x, y, zoom }
}

/// The window as a texture UV rect [left, top, right, bottom] in 0..=1.
/// `aspect` is the source image's width / height.
pub fn crop_uv(aspect: f32, crop: Crop) -> [f32; 4] {
    let crop = clamp_crop(aspect, crop);
    let (half_w, half_h) = half_window(aspect, crop.zoom);
    [
        crop.x - half_w,
        crop.y - half_h,
        crop.x + half_w,
        crop.y + half_h,
    ]
}

/// Centre the window on a detected face with some headroom. The face is in
/// image coordinates: centre as a fraction of width/height, side as a
/// fraction of the image's short side.
pub fn face_crop(aspect: f32, face: crate::face_detect::Face) -> Crop {
    let zoom = if face.side.is_finite() && face.side > 0.0 {
        1.0 / (face.side * FACE_HEADROOM)
    } else {
        1.0
    };
    clamp_crop(
        aspect,
        Crop {
            x: face.x,
            y: face.y,
            zoom,
        },
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Appearance {
    Initials {
        text: String,
        colour: [u8; 3],
    },
    Image {
        file: String,
        /// Square crop window; the centre crop when absent (entries saved
        /// before cropping existed).
        #[serde(default)]
        crop: Crop,
        /// Source image width / height; 1.0 for entries saved when only the
        /// cropped square was stored.
        #[serde(default = "default_source_aspect")]
        source_aspect: f32,
    },
}

fn default_source_aspect() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

/// Up to three visible letters/numbers or emoji, never split a joined cluster.
pub fn initials(text: &str) -> String {
    text.graphemes(true)
        .filter(|cluster| {
            // Keep combining marks with their base; reject controls and bidi
            // formatting. ZWJ/variation selectors remain within valid emoji.
            cluster.chars().next().is_some_and(char::is_alphanumeric)
                || crate::emoji::only_emoji(cluster) == Some(1)
        })
        .take(3)
        .collect()
}

pub fn image_path(state: &Path, file: &str) -> Option<PathBuf> {
    let hash = file.strip_suffix(".png")?;
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| state.join("custom-avatars").join(file))
}

/// Largest side of the stored source image. Cropping happens at display
/// time, so the source keeps its aspect ratio.
pub const SOURCE_MAX_DIMENSION: u32 = 1024;

/// Read only a bounded local file. The picker never accepts URLs.
pub fn import(path: &Path) -> Result<(String, Vec<u8>, u32, u32), String> {
    let metadata = std::fs::metadata(path).map_err(|_| "Could not open image")?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err("Choose an image file up to 10 MB".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "Could not open image")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read image")?;
    let (png, width, height) = normalize_source(&bytes)?;
    let hash: String = Sha256::digest(&png)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok((format!("{hash}.png"), png, width, height))
}

/// Decode, orient and bound a picked image, keeping its aspect ratio for the
/// editor's crop window. Never upscales to fill a square and never crops
/// here, so the thin-image blowup of scaling a sliver to fill a square is
/// impossible: total pixels stay bounded by the decode limits above.
pub fn normalize_source(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
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
    // Bound the stored source without changing its shape; clamp the target
    // sides so a thin image (8192x1) shrinks to 1024x1, never to zero.
    let scaled = if decoded.width().max(decoded.height()) > SOURCE_MAX_DIMENSION {
        let scale =
            f64::from(SOURCE_MAX_DIMENSION) / f64::from(decoded.width().max(decoded.height()));
        let target_w =
            ((f64::from(decoded.width()) * scale).round() as u32).clamp(1, SOURCE_MAX_DIMENSION);
        let target_h =
            ((f64::from(decoded.height()) * scale).round() as u32).clamp(1, SOURCE_MAX_DIMENSION);
        decoded.resize_exact(target_w, target_h, image::imageops::FilterType::Lanczos3)
    } else {
        decoded
    };
    let mut png = Cursor::new(Vec::new());
    scaled
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|_| "Could not prepare avatar")?;
    Ok((png.into_inner(), scaled.width(), scaled.height()))
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
        assert!(normalize_source(b"not an image").is_err());
        assert!(normalize_source(&vec![0; MAX_BYTES as usize + 1]).is_err());
        assert!(normalize_source(&png(MAX_DIMENSION + 1, 1)).is_err());
        let mut source = png(900, 600);
        source.extend_from_slice(b"private trailing metadata must not survive");
        let (clean, width, height) = normalize_source(&source).unwrap();
        assert_eq!((width, height), (900, 600));
        let decoded = image::load_from_memory(&clean).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (900, 600));
        assert!(!clean.windows(8).any(|s| s == b"metadata"));
        assert_eq!(clean, normalize_source(&clean).unwrap().0);
    }
    #[test]
    fn oversized_sources_shrink_without_changing_shape() {
        let (clean, width, height) = normalize_source(&png(4096, 2048)).unwrap();
        assert_eq!(
            (width, height),
            (SOURCE_MAX_DIMENSION, SOURCE_MAX_DIMENSION / 2)
        );
        let decoded = image::load_from_memory(&clean).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (width, height));
    }
    #[test]
    fn degenerate_aspect_ratios_never_upscale_or_blow_up() {
        for (width, height) in [(8192, 1), (1, 8192)] {
            let (clean, width, height) = normalize_source(&png(width, height)).unwrap();
            assert!(width.max(height) <= SOURCE_MAX_DIMENSION);
            assert!(width.min(height) >= 1);
            let decoded = image::load_from_memory(&clean).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (width, height));
            // A one-pixel-wide window is still a valid, bounded UV rect.
            let uv = crop_uv(width as f32 / height as f32, Crop::default());
            assert!(uv[0] >= 0.0 && uv[1] >= 0.0 && uv[2] <= 1.0 && uv[3] <= 1.0);
            assert!(uv[0] < uv[2] && uv[1] < uv[3]);
        }
    }
    #[test]
    fn crop_window_stays_square_inside_any_source() {
        // Square source, default crop: the whole texture.
        assert_eq!(crop_uv(1.0, Crop::default()), [0.0, 0.0, 1.0, 1.0]);
        // Wide source: the window takes the full height, centred.
        let uv = crop_uv(2.0, Crop::default());
        assert_eq!([uv[1], uv[3]], [0.0, 1.0]);
        assert!(((uv[2] - uv[0]) - 0.5).abs() < 1e-6);
        assert!((uv[0] - 0.25).abs() < 1e-6);
        // Tall source: the window takes the full width, centred.
        let uv = crop_uv(0.5, Crop::default());
        assert_eq!([uv[0], uv[2]], [0.0, 1.0]);
        assert!(((uv[3] - uv[1]) - 0.5).abs() < 1e-6);
        assert!((uv[1] - 0.25).abs() < 1e-6);
        // Zooming halves the window; clamping keeps it inside at the edges.
        assert_eq!(
            clamp_crop(
                1.0,
                Crop {
                    x: 0.0,
                    y: 1.0,
                    zoom: 2.0
                }
            ),
            Crop {
                x: 0.25,
                y: 0.75,
                zoom: 2.0
            }
        );
        assert_eq!(crop_window(1.0, 2.0), (0.5, 0.5));
        // Zoom is bounded; nonsense numbers fall back to the centre crop.
        assert_eq!(
            clamp_crop(
                1.0,
                Crop {
                    x: 0.5,
                    y: 0.5,
                    zoom: 99.0
                }
            )
            .zoom,
            MAX_ZOOM
        );
        assert_eq!(
            clamp_crop(
                1.0,
                Crop {
                    x: 0.5,
                    y: 0.5,
                    zoom: 0.0
                }
            )
            .zoom,
            MIN_ZOOM
        );
        assert_eq!(
            clamp_crop(
                0.0,
                Crop {
                    x: f32::NAN,
                    y: f32::INFINITY,
                    zoom: f32::NAN
                }
            ),
            Crop::default()
        );
    }
    #[test]
    fn face_crop_centres_on_the_face_with_headroom() {
        let face = crate::face_detect::Face {
            x: 0.5,
            y: 0.4,
            side: 0.25,
        };
        let crop = face_crop(1.0, face);
        assert_eq!(crop.x, 0.5);
        assert_eq!(crop.y, 0.4);
        assert!((crop.zoom - 1.0 / (0.25 * FACE_HEADROOM)).abs() < 1e-6);
        // A huge face never zooms out past the full frame; a zero or NaN
        // side falls back to the default zoom.
        let huge = face_crop(1.0, crate::face_detect::Face { side: 4.0, ..face });
        assert_eq!(huge.zoom, MIN_ZOOM);
        let broken = face_crop(
            1.0,
            crate::face_detect::Face {
                side: f32::NAN,
                ..face
            },
        );
        assert_eq!(broken.zoom, 1.0);
        // A face near an edge keeps the window inside the image.
        let edge = face_crop(
            1.0,
            crate::face_detect::Face {
                x: 0.05,
                y: 0.05,
                ..face
            },
        );
        let uv = crop_uv(1.0, edge);
        assert!(uv[0] >= 0.0 && uv[1] >= 0.0 && uv[2] <= 1.0 && uv[3] <= 1.0);
    }
    #[test]
    fn entries_saved_before_cropping_default_to_the_centre_crop() {
        let old = r#"[{"identities":[{"account":"a","origin":null,"backend_account":null,"chat":"c"}],"appearance":{"Image":{"file":"abc.png"}}}]"#;
        let mut entries: Vec<Entry> = serde_json::from_str(old).unwrap();
        let Appearance::Image {
            file,
            crop,
            source_aspect,
        } = &entries[0].appearance
        else {
            panic!("image entry");
        };
        assert_eq!(file, "abc.png");
        assert_eq!(*crop, Crop::default());
        assert_eq!(*source_aspect, 1.0);
        // Saving now writes the crop fields, and the entry round-trips.
        entries[0].appearance = Appearance::Image {
            file: "abc.png".into(),
            crop: Crop {
                x: 0.4,
                y: 0.3,
                zoom: 1.5,
            },
            source_aspect: 1.5,
        };
        let persisted = serde_json::to_string(&entries).unwrap();
        assert!(persisted.contains("\"crop\":{"), "{persisted}");
        assert!(persisted.contains("\"source_aspect\":1.5"), "{persisted}");
        let restored: Vec<Entry> = serde_json::from_str(&persisted).unwrap();
        assert_eq!(restored, entries);
    }
    #[test]
    fn avatar_initials_preserve_emoji_and_grapheme_boundaries() {
        for text in ["👩🏽‍💻", "👨‍👩‍👧‍👦", "🇫🇮", "❤️", "1️⃣", "e\u{301}"]
        {
            assert_eq!(initials(text), text);
        }
        assert_eq!(initials("👩🏽‍💻🇫🇮❤️Z"), "👩🏽‍💻🇫🇮❤️");
        assert_eq!(initials("A😀B!C"), "A😀B");
        assert_eq!(initials("\n\u{202e}<>!"), "");
        let style = Appearance::Initials {
            text: "👩🏽‍💻🇫🇮❤️".into(),
            colour: [1, 2, 3],
        };
        let json = serde_json::to_string(&style).unwrap();
        assert_eq!(serde_json::from_str::<Appearance>(&json).unwrap(), style);
    }

    #[test]
    fn image_paths_cannot_escape_owned_state() {
        assert!(image_path(Path::new("/state"), "../../secret.png").is_none());
        assert!(
            image_path(Path::new("/state"), &format!("{}.png", "a".repeat(64)))
                .unwrap()
                .starts_with("/state/custom-avatars")
        );
        assert_eq!(initials("<A>\nK\u{202e}!!!😀"), "AK😀");
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
