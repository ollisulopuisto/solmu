//! Local attachment actions never need a Beeper connection. Decode/copy work
//! and native dialogs run off the UI thread; every failure returns an event.
use std::path::{Path, PathBuf};

use super::{Command, Event};

pub(super) fn handles(command: &Command) -> bool {
    matches!(
        command,
        Command::PrepareClipboardImage(_)
            | Command::SaveAttachmentAs { .. }
            | Command::SaveAttachments { .. }
    )
}

pub(super) fn run(command: Command) -> Option<Event> {
    run_with_dialogs(
        command,
        |name| {
            rfd::FileDialog::new()
                .set_title("Save attachment")
                .set_file_name(name)
                .save_file()
        },
        || {
            rfd::FileDialog::new()
                .set_title("Save attachments")
                .pick_folder()
        },
    )
}

fn run_with_dialogs(
    command: Command,
    save_file: impl FnOnce(&str) -> Option<PathBuf>,
    pick_folder: impl FnOnce() -> Option<PathBuf>,
) -> Option<Event> {
    match command {
        Command::PrepareClipboardImage(path) => Some(Event::ClipboardImage(decode(&path))),
        Command::SaveAttachmentAs { source, name } => {
            let name = safe_name(&name);
            let destination = save_file(&name)?;
            Some(save_result(copy(&source, &destination)))
        }
        Command::SaveAttachments { files } => {
            let folder = pick_folder()?;
            let result = files.into_iter().try_for_each(|(source, name)| {
                let destination = folder.join(safe_name(&name));
                if destination.exists() {
                    return Err("A file with this name already exists; save it individually to choose another name".into());
                }
                copy(&source, &destination)
            });
            Some(save_result(result))
        }
        _ => None,
    }
}

fn safe_name(name: &str) -> String {
    // Strip both Unix (/) and Windows (\) path separators to prevent cross-platform directory traversal
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let trimmed = leaf.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.chars().any(char::is_control)
    {
        "attachment".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn decode(path: &Path) -> Result<crate::model::DecodedImage, String> {
    // Guess from the bytes: cached attachments need not have an extension.
    let image = image::ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?
        .to_rgba8();
    Ok(crate::model::DecodedImage {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: image.into_raw(),
    })
}

fn copy(source: &Path, destination: &Path) -> Result<(), String> {
    // Selecting the original file is a successful no-op, not a truncation.
    if source
        .canonicalize()
        .ok()
        .zip(destination.canonicalize().ok())
        .is_some_and(|(source, destination)| source == destination)
    {
        return Ok(());
    }
    std::fs::copy(source, destination)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn save_result(result: Result<(), String>) -> Event {
    match result {
        Ok(()) => Event::Info("Attachment saved".into()),
        Err(error) => Event::Error(format!("Could not save attachment: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(command: Command, destination: Option<PathBuf>) -> Option<Event> {
        run_with_dialogs(command, |_| destination.clone(), || destination.clone())
    }

    #[test]
    fn copy_decodes_extensionless_cached_image_and_reports_missing_or_corrupt_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cached-media");
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 128];
        image::save_buffer_with_format(
            &path,
            &pixels,
            2,
            1,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .unwrap();
        let command = Command::PrepareClipboardImage(path.clone());
        assert!(handles(&command));
        match run(command, None) {
            Some(Event::ClipboardImage(Ok(image))) => {
                assert_eq!((image.width, image.height), (2, 1));
                assert_eq!(image.bytes, pixels);
            }
            _ => panic!("copy must return decoded pixels to the UI clipboard writer"),
        }
        std::fs::write(&path, "not an image").unwrap();
        assert!(matches!(
            run(Command::PrepareClipboardImage(path.clone()), None),
            Some(Event::ClipboardImage(Err(_)))
        ));
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(
            run(Command::PrepareClipboardImage(path), None),
            Some(Event::ClipboardImage(Err(_)))
        ));
    }

    #[test]
    fn save_copies_exact_bytes_cancel_is_quiet_and_errors_are_visible() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("saved.png");
        std::fs::write(&source, b"original image bytes").unwrap();
        let command = || Command::SaveAttachmentAs {
            source: source.clone(),
            name: "image.png".into(),
        };
        assert!(handles(&command()));
        assert!(run(command(), None).is_none());
        assert!(!destination.exists());
        assert!(matches!(
            run(command(), Some(destination.clone())),
            Some(Event::Info(_))
        ));
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"original image bytes"
        );
        assert!(matches!(
            run(command(), Some(source.clone())),
            Some(Event::Info(_))
        ));
        assert_eq!(std::fs::read(&source).unwrap(), b"original image bytes");
        assert!(matches!(
            run(command(), Some(root.path().join("absent/fail"))),
            Some(Event::Error(_))
        ));
        std::fs::remove_file(&source).unwrap();
        assert!(matches!(
            run(command(), Some(destination)),
            Some(Event::Error(_))
        ));
    }

    #[test]
    fn batch_save_stays_inside_the_chosen_folder_and_reports_failures() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let folder = root.path().join("chosen");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(&source, b"bytes").unwrap();
        let command = || Command::SaveAttachments {
            files: vec![
                (source.clone(), "../photo.png".into()),
                (source.clone(), r"..\..\traversal.png".into()),
                (source.clone(), r"subfolder\picture.png".into()),
                (source.clone(), "   ".into()),
            ],
        };
        assert!(handles(&command()));
        assert!(matches!(
            run(command(), Some(folder.clone())),
            Some(Event::Info(_))
        ));
        assert_eq!(std::fs::read(folder.join("photo.png")).unwrap(), b"bytes");
        assert_eq!(
            std::fs::read(folder.join("traversal.png")).unwrap(),
            b"bytes"
        );
        assert_eq!(std::fs::read(folder.join("picture.png")).unwrap(), b"bytes");
        assert_eq!(std::fs::read(folder.join("attachment")).unwrap(), b"bytes");
        assert!(!root.path().join("photo.png").exists());
        assert!(!root.path().join("traversal.png").exists());
        std::fs::remove_file(&source).unwrap();
        assert!(matches!(
            run(command(), Some(folder)),
            Some(Event::Error(_))
        ));
        assert!(!handles(&Command::Shutdown));
    }
}
