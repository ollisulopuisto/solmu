//! Resolve optional local media tools without relying on a login-shell PATH.
use std::path::{Path, PathBuf};

static FFMPEG: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// App-wide explicit choice. All decoders and transcription use this resolver.
pub fn configure(path: Option<PathBuf>) {
    *FFMPEG.write().unwrap() = path;
}
pub fn configured() -> Option<PathBuf> {
    FFMPEG.read().unwrap().clone()
}

pub fn resolve(tool: &str) -> Result<PathBuf, String> {
    resolve_for(tool, configured().as_deref())
}

/// The setup check and playback resolve the same proposed preference.
pub fn resolve_for(tool: &str, selected: Option<&Path>) -> Result<PathBuf, String> {
    resolve_preference(tool, selected, || detect(tool))
}
fn resolve_preference(
    tool: &str,
    selected: Option<&Path>,
    fallback: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    if let Some(path) = selected {
        if tool == "ffmpeg" {
            return executable(path).then(||path.to_path_buf()).ok_or_else(||"The selected ffmpeg is no longer executable. Choose another in Settings > Media tools.".into());
        }
        if tool == "ffprobe"
            && let Some(parent) = path.parent()
            && let Ok(sibling) = resolve_in(tool, &[parent.to_path_buf()])
        {
            return Ok(sibling);
        }
    }
    fallback()
}

/// Automatic lookup never uses the explicit preference.
pub fn detect(tool: &str) -> Result<PathBuf, String> {
    let dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    resolve_in(
        tool,
        &search_directories(dirs, std::env::consts::OS, std::env::current_exe().ok()),
    )
}

/// Shared by actual resolution and platform regression fixtures. Keep the
/// Finder fallback list here rather than duplicating it in tests.
fn search_directories(
    mut dirs: Vec<PathBuf>,
    os: &str,
    executable: Option<PathBuf>,
) -> Vec<PathBuf> {
    if os == "windows" {
        prepend_application_dir(&mut dirs, executable);
    }
    if os == "macos" {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    }
    dirs
}

fn prepend_application_dir(dirs: &mut Vec<PathBuf>, executable: Option<PathBuf>) {
    if let Some(parent) = executable
        .as_deref()
        .and_then(Path::parent)
        .filter(|path| path.is_absolute())
    {
        dirs.insert(0, parent.into());
    }
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn resolve_in(tool: &str, dirs: &[PathBuf]) -> Result<PathBuf, String> {
    let name = if cfg!(windows) {
        format!("{tool}.exe")
    } else {
        tool.to_owned()
    };
    for dir in dirs {
        let candidate = dir.join(&name);
        if candidate.is_absolute() && executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "{tool} was not found. Searched: {}",
        dirs.iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finder_launch_without_homebrew_path_keeps_shell_precedence_and_fallbacks() {
        let shell = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")];
        assert_eq!(
            search_directories(shell.clone(), "macos", None),
            [
                "/usr/bin",
                "/bin",
                "/opt/homebrew/bin",
                "/usr/local/bin",
                "/opt/local/bin"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(search_directories(shell.clone(), "linux", None), shell);
        let empty = search_directories(Vec::new(), "macos", None);
        assert_eq!(
            empty,
            ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from)
        );
    }
    #[test]
    fn resolver_finds_tools_without_a_shell_path_and_keeps_precedence() {
        let root =
            std::env::temp_dir().join(format!("solmu-media-resolver-{}", std::process::id()));
        let shell = root.join("shell");
        let brew = root.join("opt/homebrew/bin");
        std::fs::create_dir_all(&shell).unwrap();
        std::fs::create_dir_all(&brew).unwrap();
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let installed = brew.join(name);
        std::fs::write(&installed, b"fixture").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            resolve_in("ffmpeg", &[shell.clone(), brew.clone()]).unwrap(),
            installed
        );
        std::fs::copy(&installed, shell.join(name)).unwrap();
        assert_eq!(
            resolve_in("ffmpeg", &[shell.clone(), brew.clone()]).unwrap(),
            shell.join(name)
        );
        let error = resolve_in("ffprobe", &[shell.clone(), brew.clone()]).unwrap_err();
        assert!(
            error.contains(&shell.display().to_string())
                && error.contains(&brew.display().to_string())
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn windows_style_application_siblings_precede_path_without_requiring_it() {
        let root =
            std::env::temp_dir().join(format!("solmu-sibling-resolver-{}", std::process::id()));
        let app = root.join("app");
        let path = root.join("path");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&path).unwrap();
        let tool = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        for directory in [&app, &path] {
            std::fs::write(directory.join(tool), b"fixture").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(
                    directory.join(tool),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
        }
        let mut dirs = vec![path];
        prepend_application_dir(&mut dirs, Some(app.join("solmu.exe")));
        assert_eq!(resolve_in("ffmpeg", &dirs).unwrap(), app.join(tool));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn resolver_rejects_relative_directories() {
        assert!(resolve_in("ffmpeg", &[PathBuf::from("relative")]).is_err());
    }
}

/// Run only on a background thread. Kill a hung binary rather than freezing UI.
pub fn verify(path: &Path, tool: &str) -> Result<(), String> {
    if !path.is_absolute() || !executable(path) {
        return Err("Choose an absolute executable file".into());
    }
    let output = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(path)
        .arg("-version")
        .stdin(std::process::Stdio::null())
        .stdout(output.try_clone().map_err(|e| e.to_string())?)
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Executable check timed out".into());
            }
        }
    };
    use std::io::{Read, Seek};
    let mut output = output;
    output.rewind().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    output
        .take(4096)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if status.success() && String::from_utf8_lossy(&bytes).starts_with(&format!("{tool} version "))
    {
        Ok(())
    } else {
        Err("The selected file did not identify as the requested media tool".into())
    }
}

#[cfg(all(test, unix))]
mod executable_checks {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn script(root: &Path, name: &str, body: &str) -> PathBuf {
        let path = root.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    #[test]
    fn selected_executable_and_sibling_reach_the_production_resolver() {
        let root = tempfile::tempdir().unwrap();
        let ffmpeg = script(
            root.path(),
            "custom-ffmpeg",
            "#!/bin/sh\nprintf 'ffmpeg version fixture\\n'\n",
        );
        let ffprobe = script(
            root.path(),
            "ffprobe",
            "#!/bin/sh\nprintf 'ffprobe version fixture\\n'\n",
        );
        assert!(verify(&ffmpeg, "ffmpeg").is_ok());
        assert!(verify(&ffprobe, "ffprobe").is_ok());
        configure(Some(ffmpeg.clone()));
        assert_eq!(resolve("ffmpeg").unwrap(), ffmpeg);
        assert_eq!(resolve("ffprobe").unwrap(), ffprobe);
        std::fs::remove_file(&ffmpeg).unwrap();
        assert!(resolve("ffmpeg").is_err());
        configure(None);
    }
    #[test]
    fn non_executable_sibling_uses_the_same_automatic_fallback_as_playback() {
        let root = tempfile::tempdir().unwrap();
        let custom = root.path().join("custom");
        let auto = root.path().join("auto");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::create_dir_all(&auto).unwrap();
        let ffmpeg = script(
            &custom,
            "ffmpeg",
            "#!/bin/sh\nprintf 'ffmpeg version fixture\n'\n",
        );
        let sibling = custom.join("ffprobe");
        std::fs::write(&sibling, b"not executable").unwrap();
        let fallback = script(
            &auto,
            "ffprobe",
            "#!/bin/sh\nprintf 'ffprobe version fixture\n'\n",
        );
        assert_eq!(
            resolve_preference("ffprobe", Some(&ffmpeg), || resolve_in("ffprobe", &[auto]))
                .unwrap(),
            fallback
        );
    }
    #[test]
    fn rejects_wrong_non_executable_and_hung_files() {
        let root = tempfile::tempdir().unwrap();
        let wrong = script(
            root.path(),
            "wrong",
            "#!/bin/sh\nprintf 'not the right tool\\n'\n",
        );
        assert!(verify(&wrong, "ffmpeg").is_err());
        std::fs::set_permissions(&wrong, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(verify(&wrong, "ffmpeg").is_err());
        let hung = script(root.path(), "hung", "#!/bin/sh\nwhile :; do :; done\n");
        let start = std::time::Instant::now();
        assert!(verify(&hung, "ffmpeg").unwrap_err().contains("timed out"));
        assert!(start.elapsed().as_secs() < 6);
    }
}
