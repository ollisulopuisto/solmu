//! Optional prebuilt runtime download. No runtime source is linked into Solmu.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub struct Artifact {
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub executable_sha256: &'static str,
}

// Populated only from the successfully signed/notarized helper artifact.
// A missing platform is unavailable, never silently substituted from PATH.
pub fn artifact() -> Option<Artifact> {
    None
}

pub fn executable(state: &Path) -> PathBuf {
    let revision = artifact().map_or("unavailable", |a| a.sha256);
    state
        .join("runtimes/whisper")
        .join(revision)
        .join("SolmuWhisper.app/Contents/MacOS/whisper-cli")
}

#[cfg(target_os = "macos")]
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
            Ok(Some(_)) => return Err("Runtime verification or installation failed".into()),
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

pub fn verify(executable: &Path, spec: Artifact) -> Result<(), String> {
    hash_file(executable, spec.executable_sha256)?;
    #[cfg(target_os = "macos")]
    {
        let app = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .ok_or("Invalid runtime bundle path")?;
        let requirement = "anchor apple generic and identifier \"fi.sulopuisto.solmu.whisper\" and certificate leaf[subject.OU] = \"LWSR8T5392\"";
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
    #[cfg(not(target_os = "macos"))]
    return Err("No verified runtime installer for this platform yet".into());
    #[cfg(target_os = "macos")]
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
    #[cfg(not(target_os = "macos"))]
    let _ = state;
    #[cfg(target_os = "macos")]
    let parent = state.join("runtimes/whisper");
    #[cfg(target_os = "macos")]
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let staging = tempfile::tempdir_in(&parent).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    bounded_command(
        std::process::Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(staging.path()),
    )?;
    #[cfg(not(target_os = "macos"))]
    return Err("No verified runtime installer for this platform yet".into());
    #[cfg(target_os = "macos")]
    {
        let relative = Path::new("SolmuWhisper.app/Contents/MacOS/whisper-cli");
        verify(&staging.path().join(relative), spec)?;
        if cancelled.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        let destination = parent.join(spec.sha256);
        if destination.exists() {
            // An earlier successful installation is reused only if still valid.
            verify(&destination.join(relative), spec)?;
        } else {
            std::fs::rename(staging.path(), &destination).map_err(|e| e.to_string())?;
        }
        Ok(destination.join(relative))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
