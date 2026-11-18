//! Opt-in, pinned model installation. Constructing the app does not fetch anything.
//! All download and verification I/O runs on the worker, never in a paint pass.
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

pub const MODEL_BYTES: u64 = 147_951_465;
pub const MODEL_SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";
pub const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin";

pub fn model_path(state: &Path) -> PathBuf {
    state.join("models/whisper/ggml-base.bin")
}

// An advisory lock prevents startup cleanup from touching an active download
// in another process. Only our own named temporary files are removed.
fn download_lock(state: &Path) -> Result<std::fs::File, String> {
    let parent = model_path(state).parent().unwrap().to_owned();
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(".download.lock"))
        .map_err(|e| e.to_string())?;
    file.try_lock()
        .map_err(|_| "Another local engine download is running".to_owned())?;
    Ok(file)
}

fn clean_download_temps(state: &Path) {
    for (parent, prefix) in [
        (
            model_path(state).parent().unwrap().to_owned(),
            "solmu-whisper-download-",
        ),
        (state.join("runtimes/whisper"), "solmu-whisper-stage-"),
    ] {
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with(prefix) {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                let _ = std::fs::remove_dir_all(entry.path());
            } else {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub fn remove_installation(state: &Path) -> Result<(), String> {
    let _lock = download_lock(state)?;
    for path in [model_path(state), state.join("runtimes/whisper")] {
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(error) = result
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.to_string());
        }
    }
    clean_download_temps(state);
    Ok(())
}

pub fn cleanup_abandoned_downloads(state: &Path) {
    if let Ok(_lock) = download_lock(state) {
        clean_download_temps(state);
    }
}

const DISK_RESERVE: u64 = 16 * 1024 * 1024;

fn enough_space(available: u64, required: u64) -> Result<(), String> {
    if available < required.saturating_add(DISK_RESERVE) {
        Err("Not enough free disk space. Free at least 260 MB and retry.".into())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: path is terminated, stats points to writable storage of the right size.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: successful statvfs initialized every field.
    let stats = unsafe { stats.assume_init() };
    let bytes = u128::from(stats.f_bavail) * u128::from(stats.f_frsize);
    Ok(bytes.min(u128::from(u64::MAX)) as u64)
}

#[cfg(windows)]
fn available_space(path: &Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            path: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: path is terminated, available is writable; optional outputs are null.
    if unsafe {
        GetDiskFreeSpaceExW(
            path.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(available)
}

/// Recheck a model on the transcription worker, never on the UI thread.
pub fn verify_model(path: &Path) -> Result<(), String> {
    verify_file(path, MODEL_BYTES, MODEL_SHA256)
}

fn verify_file(path: &Path, expected_bytes: u64, expected_hash: &str) -> Result<(), String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != expected_bytes {
        return Err("Local model is incomplete. Remove it and download again.".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    if hex_hash(&hash.finalize()) != expected_hash {
        return Err("Local model verification failed. Remove it and download again.".into());
    }
    Ok(())
}

/// Optional runtime lives in app state, never inside Solmu's signed bundle.
/// A platform provider must install and verify it before this path is runnable.
pub fn runtime_path(state: &Path) -> PathBuf {
    crate::whisper_runtime::executable(state)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    Downloading { received: u64, total: u64 },
    Ready(PathBuf),
    Cancelled,
    Failed(String),
}

/// A single explicit download. Dropping the handle cancels unfinished work.
pub struct Download {
    state: Arc<Mutex<DownloadState>>,
    cancelled: Arc<AtomicBool>,
}
impl Download {
    /// Call only from the Download & enable action, never from startup/render.
    pub fn start(app_state: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        let path = model_path(&app_state);
        let total = MODEL_BYTES + crate::whisper_runtime::artifact().map_or(0, |a| a.bytes);
        let state = Arc::new(Mutex::new(DownloadState::Downloading {
            received: 0,
            total,
        }));
        let cancelled = Arc::new(AtomicBool::new(false));
        let download = Self {
            state: state.clone(),
            cancelled: cancelled.clone(),
        };
        std::thread::spawn(move || {
            let progress = |received| {
                *state.lock().unwrap_or_else(|p| p.into_inner()) =
                    DownloadState::Downloading { received, total };
                wake();
            };
            let result = (|| {
                let _lock = download_lock(&app_state)?;
                clean_download_temps(&app_state);
                let parent = path.parent().ok_or("Model path has no directory")?;
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                enough_space(available_space(parent)?, MODEL_BYTES)?;
                if cancelled.load(Ordering::Acquire) {
                    return Err("Download cancelled".into());
                }
                let spec = crate::whisper_runtime::artifact()
                    .ok_or("No verified prebuilt runtime is available for this platform yet")?;
                let executable = runtime_path(&app_state);
                if !executable.exists()
                    || crate::whisper_runtime::verify(&executable, spec).is_err()
                {
                    enough_space(
                        available_space(parent)?,
                        MODEL_BYTES + spec.bytes + 64 * 1024 * 1024,
                    )?;
                    let temporary = tempfile::Builder::new()
                        .prefix("solmu-whisper-download-")
                        .tempdir_in(parent)
                        .map_err(|e| e.to_string())?;
                    let archive = temporary.path().join("runtime.zip");
                    fetch_verified(
                        spec.url,
                        &archive,
                        spec.bytes,
                        spec.sha256,
                        &cancelled,
                        progress,
                    )?;
                    crate::whisper_runtime::install_archive(
                        &archive, &app_state, spec, &cancelled,
                    )?;
                }
                progress(spec.bytes);
                fetch_verified(
                    MODEL_URL,
                    &path,
                    MODEL_BYTES,
                    MODEL_SHA256,
                    &cancelled,
                    |received| progress(spec.bytes + received),
                )
            })();
            let final_state = match result {
                Ok(()) => DownloadState::Ready(path),
                Err(_) if cancelled.load(Ordering::Acquire) => DownloadState::Cancelled,
                Err(e) => DownloadState::Failed(e),
            };
            *state.lock().unwrap_or_else(|p| p.into_inner()) = final_state;
            wake();
        });
        download
    }
    pub fn state(&self) -> DownloadState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn cancellation_requested(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Drop for Download {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn fetch_verified(
    url: &str,
    path: &Path,
    bytes: u64,
    sha256: &str,
    cancelled: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<(), String> {
    if verify_file(path, bytes, sha256).is_ok() {
        progress(bytes);
        return Ok(());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .proxy(crate::proxy::ureq_proxy())
        .timeout_global(Some(Duration::from_secs(30 * 60)))
        .timeout_resolve(Some(Duration::from_secs(10)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(10)))
        .timeout_per_read(Some(Duration::from_secs(5)))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    install(
        &mut response.body_mut().as_reader(),
        path,
        bytes,
        sha256,
        cancelled,
        progress,
    )
}

fn hex_hash(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// A partial file is never accepted as a model. tempfile removes every failed
/// or cancelled attempt; a verified file replaces the destination atomically.
fn install(
    reader: &mut impl Read,
    path: &Path,
    expected_bytes: u64,
    expected_hash: &str,
    cancelled: &AtomicBool,
    progress: impl Fn(u64),
) -> Result<(), String> {
    let parent = path.parent().ok_or("Model path has no directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::Builder::new()
        .prefix("solmu-whisper-download-")
        .tempfile_in(parent)
        .map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err("Download cancelled".into());
        }
        let count = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        received = received
            .checked_add(count as u64)
            .ok_or("Model is too large")?;
        if received > expected_bytes {
            return Err("Model size did not match".into());
        }
        hash.update(&buf[..count]);
        file.write_all(&buf[..count]).map_err(|e| e.to_string())?;
        progress(received);
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("Download cancelled".into());
    }
    if received != expected_bytes || hex_hash(&hash.finalize()) != expected_hash {
        return Err("Model verification failed. Retry the download.".into());
    }
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_respects_live_download_and_remove_clears_runtime() {
        let root = tempfile::tempdir().unwrap();
        let lock = download_lock(root.path()).unwrap();
        let temp = model_path(root.path())
            .parent()
            .unwrap()
            .join("solmu-whisper-download-abandoned");
        std::fs::write(&temp, b"partial").unwrap();
        cleanup_abandoned_downloads(root.path());
        assert!(temp.exists(), "live download lock protects temporary files");
        drop(lock);
        cleanup_abandoned_downloads(root.path());
        assert!(!temp.exists());
        let runtime = root.path().join("runtimes/whisper/old");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(model_path(root.path()), b"model").unwrap();
        remove_installation(root.path()).unwrap();
        assert!(!runtime.exists());
        assert!(!model_path(root.path()).exists());
    }

    #[test]
    fn verified_existing_download_never_uses_the_network() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.bin");
        std::fs::write(&path, b"verified model").unwrap();
        let digest = hex_hash(&Sha256::digest(b"verified model"));
        fetch_verified(
            "not a URL",
            &path,
            14,
            &digest,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"verified model");
    }

    #[test]
    fn disk_space_includes_room_for_the_atomic_install() {
        assert!(enough_space(MODEL_BYTES, MODEL_BYTES).is_err());
        assert!(enough_space(MODEL_BYTES + DISK_RESERVE, MODEL_BYTES).is_ok());
        let root = tempfile::tempdir().unwrap();
        assert!(available_space(root.path()).unwrap() > 0);
    }
    #[test]
    fn verification_and_cancellation_never_replace_a_good_model_with_a_partial() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.bin");
        std::fs::write(&path, b"previous good model").unwrap();
        let bytes = b"new verified model";
        let digest = hex_hash(&Sha256::digest(bytes));
        let cancel = AtomicBool::new(false);
        for input in [&bytes[..4], &bytes[..], b"new corrupted model".as_slice()] {
            let mut reader = input;
            assert!(
                install(
                    &mut reader,
                    &path,
                    bytes.len() as u64,
                    "wrong hash",
                    &cancel,
                    |_| {}
                )
                .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"previous good model");
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        }
        let mut reader = &bytes[..];
        assert!(
            install(
                &mut reader,
                &path,
                bytes.len() as u64,
                &digest,
                &cancel,
                |_| cancel.store(true, Ordering::Release)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"previous good model");
        cancel.store(false, Ordering::Release);
        install(
            &mut &bytes[..],
            &path,
            bytes.len() as u64,
            &digest,
            &cancel,
            |_| {},
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
