//! Optional prebuilt runtime download. No runtime source is linked into Solmu.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Artifact {
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub executable_sha256: &'static str,
}

// Populated only from the owner-published, reviewed helper release.
// A missing platform is unavailable, never silently substituted from PATH.
// Every URL, size and hash below was recomputed from the published
// whisper-helper-v1.9.5 assets after anonymous re-download: macOS archives
// from the signed and notarized default-branch run 44301781725, Linux and
// Windows from the unsigned matrix run 44320859629 (hash-pinned only).
pub fn artifact() -> Option<Artifact> {
    let spec = match target() {
        "aarch64-apple-darwin" => Artifact {
            url: "https://github.com/ollisulopuisto/solmu/releases/download/whisper-helper-v1.9.5/solmu-whisper-v1.9.5-arm64.zip",
            bytes: 2_106_898,
            sha256: "2266754f8e10f79ce7627a82e7878872390db2a38d4e1846bc9881b0bd0be215",
            executable_sha256: "3ecb9965b5406d3c7ff72c4103379f02b312b9df5ca16a740437c15a71f2473b",
        },
        "x86_64-apple-darwin" => Artifact {
            url: "https://github.com/ollisulopuisto/solmu/releases/download/whisper-helper-v1.9.5/solmu-whisper-v1.9.5-x86_64.zip",
            bytes: 2_206_873,
            sha256: "a25410c3234683beb0de3045778b42eb149ee8b8b4dea6d184ae28406c632302",
            executable_sha256: "790bdab490e6a533023eec9cc6c2fa603316002d55fecf6541426e772911dee1",
        },
        "x86_64-unknown-linux-gnu" => Artifact {
            url: "https://github.com/ollisulopuisto/solmu/releases/download/whisper-helper-v1.9.5/solmu-whisper-v1.9.5-linux-x86_64.tar.gz",
            bytes: 1_719_171,
            sha256: "42746c9ecdcef5b6ac14e974a020924666d75fd0eddef3e34240677f40ef63fa",
            executable_sha256: "ab7529edbe1cb1d15410f3ac8cc5dc102f57383fe7e08e945e56660da890c7ea",
        },
        "aarch64-unknown-linux-gnu" => Artifact {
            url: "https://github.com/ollisulopuisto/solmu/releases/download/whisper-helper-v1.9.5/solmu-whisper-v1.9.5-linux-aarch64.tar.gz",
            bytes: 1_660_320,
            sha256: "4e3ea6e96c482f6dbab6bc0514f10d434318fabde019714720c5627b71100753",
            executable_sha256: "abf71b67dc6c9d90f3c87ae7371c2a3acd5f30756da8a9ed5c99a32891ab986b",
        },
        "x86_64-pc-windows-msvc" => Artifact {
            url: "https://github.com/ollisulopuisto/solmu/releases/download/whisper-helper-v1.9.5/solmu-whisper-v1.9.5-windows-x86_64.tar.gz",
            bytes: 827_962,
            sha256: "a625992f7abcde6fee4fc32d0df8c0497ccdf30de116ad28390d14b4c9f7c835",
            executable_sha256: "623329cf7e66e9c7fb4fca9dfc3c4f99464571ad1ce30795a6bfb97ae075a968",
        },
        "aarch64-pc-windows-msvc" => Artifact {
            url: "https://github.com/ollisulopuisto/solmu/releases/download/whisper-helper-v1.9.5/solmu-whisper-v1.9.5-windows-arm64.tar.gz",
            bytes: 840_152,
            sha256: "64c60848f07d3d23d452091773ca20355cbdd0a1461afa1a26d70936d83a7c3f",
            executable_sha256: "f7165f6886e97897a1f26fd42bf6b77afa5bfa87def240074902300f217270f1",
        },
        _ => return None,
    };
    Some(spec)
}

pub fn executable(state: &Path) -> PathBuf {
    let revision = artifact().map_or("unavailable", |a| a.sha256);
    state
        .join("runtimes/whisper")
        .join(revision)
        .join(relative_executable())
}

pub fn target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => "unsupported",
    }
}

pub fn relative_executable() -> &'static str {
    if cfg!(target_os = "macos") {
        "SolmuWhisper.app/Contents/MacOS/whisper-cli"
    } else if cfg!(target_os = "windows") {
        "bin/whisper-cli.exe"
    } else {
        "bin/whisper-cli"
    }
}

fn bounded_command(command: &mut std::process::Command) -> Result<(), String> {
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "Runtime command {:?} failed: {status}",
                    command.get_program()
                ));
            }
            Ok(None) if started.elapsed() < Duration::from_secs(60) => {
                std::thread::sleep(Duration::from_millis(25));
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(error) => error.to_string(),
                    _ => "Runtime verification or installation timed out".into(),
                });
            }
        }
    }
}

fn hash_file(path: &Path, expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    use std::fmt::Write;
    let mut actual = String::new();
    for byte in hash.finalize() {
        let _ = write!(actual, "{byte:02x}");
    }
    if actual != expected {
        return Err("Whisper runtime hash did not match".into());
    }
    Ok(())
}

pub fn verify_hash(executable: &Path, spec: Artifact) -> Result<(), String> {
    hash_file(executable, spec.executable_sha256)
}

/// Fingerprint all regular files in the verified runtime tree, not just the
/// CLI. Rechecked per job to catch changed libraries or bundle metadata without
/// launching the OS signature tools on every short voice note.
pub fn fingerprint(executable: &Path) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut root = executable;
    for _ in Path::new(relative_executable()).components() {
        root = root.parent().ok_or("Invalid runtime path")?;
    }
    if std::fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Runtime directory is linked".into());
    }
    let mut pending = vec![root.to_owned()];
    let mut visited = 0usize;
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            } else {
                return Err("Runtime contains a linked or special file".into());
            }
            visited += 1;
            if visited > 1024 {
                return Err("Runtime has too many files".into());
            }
        }
    }
    files.sort();
    let mut hash = Sha256::new();
    let mut total = 0usize;
    for path in files {
        let name = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        hash.update(length.to_le_bytes());
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count);
            if total > 64 << 20 {
                return Err("Runtime exceeds its size limit".into());
            }
            hash.update(&buffer[..count]);
        }
    }
    Ok(hash.finalize().into())
}

pub fn verify(executable: &Path, spec: Artifact) -> Result<(), String> {
    verify_hash(executable, spec)?;
    #[cfg(target_os = "macos")]
    {
        let app = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or("Invalid runtime bundle path")?;
        let requirement = "=anchor apple generic and identifier \"fi.sulopuisto.solmu.whisper\" and certificate leaf[subject.OU] = \"LWSR8T5392\"";
        bounded_command(
            std::process::Command::new("/usr/bin/codesign")
                .args(["--verify", "--strict", "--deep", "-R", requirement])
                .arg(app),
        )?;
        bounded_command(
            std::process::Command::new("/usr/sbin/spctl")
                .args(["--assess", "--type", "execute"])
                .arg(app),
        )?;
    }
    Ok(())
}

/// Archive bytes must already match the pinned size and SHA256. The app bundle
/// is verified in a private staging directory before an atomic rename. Nothing
/// in Solmu.app is changed; no quarantine attributes are removed.
pub fn install_archive(
    archive: &Path,
    state: &Path,
    spec: Artifact,
    cancelled: &AtomicBool,
) -> Result<PathBuf, String> {
    if std::fs::metadata(archive).map_err(|e| e.to_string())?.len() != spec.bytes {
        return Err("Whisper runtime size did not match".into());
    }
    hash_file(archive, spec.sha256)?;
    if cancelled.load(Ordering::Acquire) {
        return Err("Download cancelled".into());
    }
    let parent = state.join("runtimes/whisper");
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let staging = tempfile::Builder::new()
        .prefix("solmu-whisper-stage-")
        .tempdir_in(&parent)
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    bounded_command(
        std::process::Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(staging.path()),
    )?;
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        let tar = PathBuf::from(
            std::env::var_os("SystemRoot").ok_or("Windows system directory unavailable")?,
        )
        .join("System32/tar.exe");
        #[cfg(not(target_os = "windows"))]
        let tar = PathBuf::from(option_env!("SOLMU_TAR").unwrap_or("/usr/bin/tar"));
        bounded_command(
            std::process::Command::new(tar)
                .arg("-xf")
                .arg(archive)
                .arg("-C")
                .arg(staging.path()),
        )?;
    }
    let relative = Path::new(relative_executable());
    let binary = staging.path().join(relative);
    // Reject symlinked parent directories and binaries before executing anything.
    let mut part = staging.path().to_owned();
    for component in relative.components() {
        part.push(component);
        if std::fs::symlink_metadata(&part)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Runtime archive contains a linked executable path".into());
        }
    }
    verify(&binary, spec)?;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("Download cancelled".into());
    }
    let destination = parent.join(spec.sha256);
    if destination.exists() {
        if verify(&destination.join(relative), spec).is_ok() {
            return Ok(destination.join(relative));
        }
        // A corrupt install is never executed. The staged replacement has
        // already passed every check before removing the damaged directory.
        std::fs::remove_dir_all(&destination).map_err(|e| e.to_string())?;
    }
    std::fs::rename(staging.path(), &destination).map_err(|e| e.to_string())?;
    Ok(destination.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_fingerprint_covers_metadata_and_libraries() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join(relative_executable());
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"runtime").unwrap();
        let metadata = root.path().join("metadata");
        std::fs::write(&metadata, b"first").unwrap();
        let before = fingerprint(&binary).unwrap();
        std::fs::write(&metadata, b"changed").unwrap();
        assert_ne!(before, fingerprint(&binary).unwrap());
        std::fs::write(&metadata, b"first").unwrap();
        assert_eq!(before, fingerprint(&binary).unwrap());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn native_archive_install_repairs_and_rejects_bad_hash() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let binary = source.join(relative_executable());
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"synthetic runtime").unwrap();
        let archive = root.path().join("runtime.tar.gz");
        assert!(
            std::process::Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&source)
                .arg("bin")
                .status()
                .unwrap()
                .success()
        );
        let spec = Artifact {
            url: "https://invalid.invalid/not-used",
            bytes: std::fs::metadata(&archive).unwrap().len(),
            sha256: Box::leak(
                Sha256::digest(std::fs::read(&archive).unwrap())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
                    .into_boxed_str(),
            ),
            executable_sha256: Box::leak(
                Sha256::digest(b"synthetic runtime")
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
                    .into_boxed_str(),
            ),
        };
        let state = root.path().join("state");
        let cancel = AtomicBool::new(false);
        let installed = install_archive(&archive, &state, spec, &cancel).unwrap();
        std::fs::write(&installed, b"damage").unwrap();
        assert!(verify(&installed, spec).is_err());
        install_archive(&archive, &state, spec, &cancel).unwrap();
        verify(&installed, spec).unwrap();
        let bad = Artifact {
            sha256: "wrong",
            ..spec
        };
        assert!(install_archive(&archive, &state, bad, &cancel).is_err());
        assert!(
            state
                .join("runtimes/whisper")
                .read_dir()
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("solmu-whisper-stage-"))
        );
    }

    #[test]
    fn runtime_hash_must_match_before_any_extraction() {
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("runtime.zip");
        std::fs::write(&archive, b"not an archive").unwrap();
        let spec = Artifact {
            url: "https://invalid.invalid",
            bytes: 14,
            sha256: "wrong",
            executable_sha256: "wrong",
        };
        assert!(install_archive(&archive, root.path(), spec, &AtomicBool::new(false)).is_err());
        assert!(!root.path().join("runtimes").exists());
    }
}
