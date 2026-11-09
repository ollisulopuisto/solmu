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
            let result = save_batch(&folder, files);
            Some(save_result(result))
        }
        _ => None,
    }
}

/// Plan every name first: normalization must not merge distinct attachments.
/// Existing files still refuse the batch, and create_new closes overwrite races.
fn save_batch(folder: &Path, files: Vec<(PathBuf, String)>) -> Result<(), String> {
    let names: Vec<String> = files.iter().map(|(_, name)| safe_name(name)).collect();
    let reserved: std::collections::HashSet<String> =
        names.iter().map(|name| name.to_lowercase()).collect();
    let mut used = std::collections::HashSet::new();
    let mut plan = Vec::with_capacity(files.len());
    for ((source, _), name) in files.into_iter().zip(names) {
        let mut chosen = name.clone();
        let mut number = 2;
        if used.contains(&chosen.to_lowercase()) {
            loop {
                let path = Path::new(&name);
                let stem = path
                    .file_stem()
                    .and_then(|part| part.to_str())
                    .unwrap_or(&name);
                let extension = path
                    .extension()
                    .and_then(|part| part.to_str())
                    .map_or(String::new(), |part| format!(".{part}"));
                chosen = format!("{stem} ({number}){extension}");
                number += 1;
                let key = chosen.to_lowercase();
                if !used.contains(&key)
                    && !reserved.contains(&key)
                    && !folder.join(&chosen).exists()
                {
                    break;
                }
            }
        }
        let destination = folder.join(&chosen);
        if destination.exists() {
            return Err(
                "A file with this name already exists; save it individually to choose another name"
                    .into(),
            );
        }
        let source = std::fs::File::open(&source).map_err(|error| error.to_string())?;
        if !source
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("An attachment source is not a file".into());
        }
        used.insert(chosen.to_lowercase());
        plan.push((source, destination));
    }
    for (mut source, destination) in plan {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Exported private conversations must not become readable to other
        // local users through the default 0666 masked by a common 022 umask.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options
            .open(&destination)
            .map_err(|error| error.to_string())?;
        std::io::copy(&mut source, &mut output).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn safe_name(name: &str) -> String {
    // Strip both Unix (/) and Windows (\) path separators to prevent cross-platform directory traversal
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let trimmed = leaf.trim();
    let sanitized = trimmed.trim_end_matches([' ', '.']);
    let stem = sanitized.split('.').next().unwrap_or(sanitized);
    if sanitized.is_empty()
        || sanitized == "."
        || sanitized == ".."
        || sanitized.chars().any(char::is_control)
        || crate::safety::is_reserved_dos_name(stem)
    {
        "attachment".to_owned()
    } else {
        sanitized.to_owned()
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

    #[cfg(unix)]
    #[test]
    fn batch_exports_are_owner_only_even_when_the_source_is_public() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("shared");
        std::fs::create_dir(&folder).unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, b"private attachment").unwrap();
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o644)).unwrap();
        save_batch(&folder, vec![(source, "private.txt".into())]).unwrap();
        let output = folder.join("private.txt");
        assert_eq!(
            std::fs::metadata(&output).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(std::fs::read(&output).unwrap(), b"private attachment");
    }

    #[test]
    fn normalized_batch_names_are_unique_and_keep_each_documents_bytes() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("chosen");
        std::fs::create_dir(&folder).unwrap();
        let files: Vec<_> = [
            "notes.pdf",
            " notes.pdf",
            "notes (2).pdf",
            r"..\notes.pdf",
            "NOTES.pdf",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let source = root.path().join(format!("source{index}"));
            std::fs::write(&source, format!("document{index}")).unwrap();
            (source, name.to_owned())
        })
        .collect();
        save_batch(&folder, files).unwrap();
        for (name, index) in [
            ("notes.pdf", 0),
            ("notes (3).pdf", 1),
            ("notes (2).pdf", 2),
            ("notes (4).pdf", 3),
            ("NOTES (5).pdf", 4),
        ] {
            assert_eq!(
                std::fs::read_to_string(folder.join(name)).unwrap(),
                format!("document{index}")
            );
        }
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 5);
    }

    #[test]
    fn batch_preflight_refuses_existing_files_or_missing_sources_without_partial_saves() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("chosen");
        std::fs::create_dir(&folder).unwrap();
        let source = root.path().join("source");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(folder.join("existing.pdf"), b"keep").unwrap();
        assert!(
            save_batch(
                &folder,
                vec![
                    (source.clone(), "first.pdf".into()),
                    (source.clone(), "existing.pdf".into())
                ]
            )
            .is_err()
        );
        assert!(!folder.join("first.pdf").exists());
        assert_eq!(std::fs::read(folder.join("existing.pdf")).unwrap(), b"keep");
        assert!(
            save_batch(
                &folder,
                vec![
                    (source, "first.pdf".into()),
                    (root.path().join("missing"), "last.pdf".into())
                ]
            )
            .is_err()
        );
        assert!(!folder.join("first.pdf").exists());
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

    #[test]
    fn safe_name_neutralizes_dos_devices_and_trailing_characters() {
        for name in [
            "CON",
            "con.txt",
            "PRN.pdf",
            "aux.png",
            "NUL.docx",
            "com1.jpg",
            "COM9.csv",
            "lpt1.txt",
            "LPT9.log",
            r"folder\CON.txt",
            "../PRN.png",
            "...",
            "   ",
            "photo.png.",
            "photo.png   ",
        ] {
            if name.starts_with("photo.png") {
                assert_eq!(safe_name(name), "photo.png", "{name}");
            } else {
                assert_eq!(safe_name(name), "attachment", "{name}");
            }
        }
        assert_eq!(safe_name("normal_document.pdf"), "normal_document.pdf");
    }
}
