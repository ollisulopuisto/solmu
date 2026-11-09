//! Public GIF retrieval is separate from authenticated Beeper traffic.
use super::{Command, Event};
use std::{
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const LIMIT: u64 = 32 * 1024 * 1024;
fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not start GIF download".into())
}
fn media_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid GIF URL")?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !(host == "giphy.com" || host.ends_with(".giphy.com"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("GIF media must come from GIPHY over HTTPS".into());
    }
    Ok(url)
}
fn download(value: &str) -> Result<PathBuf, String> {
    let mut response = client()?
        .get(media_url(value)?)
        .send()
        .map_err(|_| "GIF download failed")?
        .error_for_status()
        .map_err(|_| "GIF download was refused")?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "GIF download failed")?;
    if bytes.len() as u64 > LIMIT {
        return Err("GIF exceeds the attachment size limit".into());
    }
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return Err("GIF response is not an MP4 video".into());
    }
    let path = save_temporary(&bytes, "mp4")?;
    super::beeper::register_temp_upload(&path);
    Ok(path)
}
fn save_temporary(bytes: &[u8], extension: &str) -> Result<PathBuf, String> {
    use std::io::Write;
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "solmu-media-{}-{stamp}-{sequence}.{extension}",
        std::process::id()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|_| "Could not create media file")?;
    file.write_all(bytes)
        .map_err(|_| "Could not save media file")?;
    Ok(path)
}
fn thumbnail(url: &str) -> Option<PathBuf> {
    let response = client()
        .ok()?
        .get(media_url(url).ok()?)
        .timeout(Duration::from_secs(3))
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let mut bytes = Vec::new();
    response.take(512 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 512 * 1024 {
        return None;
    }
    // Never decode remote dimensions here; the image loader applies its own limits.
    if !bytes.starts_with(b"GIF8")
        && !bytes.starts_with(&[0xff, 0xd8, 0xff])
        && !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return None;
    }
    save_temporary(&bytes, "gif").ok()
}
pub(super) fn run(command: Command) -> Option<Event> {
    match command {
        Command::PrepareGif { owner, gif } => Some(Event::PreparedMedia {
            owner,
            kind: super::MediaKind::Gif,
            result: download(&gif.mp4),
        }),
        Command::PrepareSticker {
            owner,
            source,
            crop,
            transparent,
        } => Some(Event::PreparedMedia {
            owner,
            kind: super::MediaKind::Sticker,
            result: make_sticker(&source, crop, transparent),
        }),
        Command::SearchGifs { query, key } => {
            let results = search(&query, &key);
            Some(Event::Gifs { query, results })
        }
        _ => None,
    }
}
fn make_sticker(
    source: &std::path::Path,
    crop: crate::model::StickerCrop,
    transparent: bool,
) -> Result<PathBuf, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(source)
        .map_err(|_| "Could not open sticker picture")?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read sticker picture")?;
    if bytes.len() as u64 > LIMIT {
        return Err("Sticker picture exceeds the size limit".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unknown sticker picture format")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let picture = reader
        .decode()
        .map_err(|_| "Could not decode sticker picture")?
        .to_rgba8();
    let crop = crop.clamped(picture.width(), picture.height());
    let cropped =
        image::imageops::crop_imm(&picture, crop.x, crop.y, crop.side, crop.side).to_image();
    let mut scaled =
        image::imageops::resize(&cropped, 512, 512, image::imageops::FilterType::Lanczos3);
    if !transparent {
        for pixel in scaled.pixels_mut() {
            let alpha = u16::from(pixel[3]);
            for c in &mut pixel.0[..3] {
                *c = ((u16::from(*c) * alpha + 255 * (255 - alpha)) / 255) as u8;
            }
            pixel[3] = 255;
        }
    }
    let mut encoded = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
        .encode(&scaled, 512, 512, image::ExtendedColorType::Rgba8)
        .map_err(|_| "Could not encode sticker")?;
    let path = save_temporary(&encoded, "webp")?;
    super::beeper::register_temp_upload(&path);
    Ok(path)
}
fn search(query: &str, key: &str) -> Result<Vec<crate::model::Gif>, crate::model::GifError> {
    let fail = |message: &str, bad_key| crate::model::GifError {
        message: message.into(),
        bad_key,
    };
    if key.trim().is_empty() {
        return Err(fail("Set a GIPHY API key in Settings", true));
    }
    let url = if query.trim().is_empty() {
        "https://api.giphy.com/v1/gifs/trending"
    } else {
        "https://api.giphy.com/v1/gifs/search"
    };
    let response = client()
        .map_err(|e| fail(&e, false))?
        .get(url)
        .query(&[
            ("api_key", key),
            ("q", query),
            ("limit", "12"),
            ("rating", "pg-13"),
        ])
        .send()
        .map_err(|_| fail("GIF search failed", false))?;
    if !response.status().is_success() {
        return Err(fail(
            "GIPHY refused the search",
            matches!(response.status().as_u16(), 401 | 403),
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("GIF search response failed", false))?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(fail("GIF search response exceeds the size limit", false));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| fail("Invalid GIF search response", false))?;
    Ok(value["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let image = &item["images"]["fixed_height"];
            let mp4 = image["mp4"].as_str()?;
            media_url(mp4).ok()?;
            Some(crate::model::Gif {
                id: item["id"].as_str()?.into(),
                still: item["images"]["fixed_height_still"]["url"]
                    .as_str()
                    .and_then(thumbnail),
                mp4: mp4.into(),
                width: image["width"].as_str()?.parse().ok()?,
                height: image["height"].as_str()?.parse().ok()?,
            })
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_download_never_accepts_local_or_unrelated_urls() {
        for url in [
            "http://media.giphy.com/x.mp4",
            "https://127.0.0.1/x.mp4",
            "https://giphy.com.evil.invalid/x",
            "https://evil.invalid/x",
            "https://name:secret@media.giphy.com/x",
            "https://media.giphy.com:444/x",
        ] {
            assert!(media_url(url).is_err(), "{url}");
        }
        assert!(media_url("https://media.giphy.com/media/id/giphy.mp4").is_ok());
    }
    #[test]
    fn sticker_preparation_keeps_transparency_and_clamps_crop() {
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.png");
        image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 100]))
            .save(&input)
            .unwrap();
        let output = make_sticker(
            &input,
            crate::model::StickerCrop {
                x: 999,
                y: 999,
                side: 999,
            },
            true,
        )
        .unwrap();
        let sticker = image::open(&output).unwrap().to_rgba8();
        assert_eq!(sticker.dimensions(), (512, 512));
        assert_eq!(sticker.get_pixel(1, 1)[3], 100);
        std::fs::remove_file(output).unwrap();
        let output =
            make_sticker(&input, crate::model::StickerCrop::centered(4, 3), false).unwrap();
        assert_eq!(
            image::open(&output).unwrap().to_rgba8().get_pixel(1, 1)[3],
            255
        );
        std::fs::remove_file(output).unwrap();
    }
}
