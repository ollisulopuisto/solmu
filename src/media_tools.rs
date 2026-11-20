//! Resolve optional local media tools without relying on a login-shell PATH.
use std::path::{Path, PathBuf};

pub fn resolve(tool: &str) -> Result<PathBuf, String> {
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
