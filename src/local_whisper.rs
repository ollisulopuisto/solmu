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
    pub fn start(path: PathBuf, wake: impl Fn() + Send + 'static) -> Self {
        let state = Arc::new(Mutex::new(DownloadState::Downloading {
            received: 0,
            total: MODEL_BYTES,
        }));
        let cancelled = Arc::new(AtomicBool::new(false));
        let download = Self {
            state: state.clone(),
            cancelled: cancelled.clone(),
        };
        std::thread::spawn(move || {
            let progress = |received| {
                *state.lock().unwrap_or_else(|p| p.into_inner()) = DownloadState::Downloading {
                    received,
                    total: MODEL_BYTES,
                };
                wake();
            };
            let result = (|| {
                let mut builder = reqwest::blocking::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .timeout(Duration::from_secs(30 * 60))
                    .https_only(true);
                if let Some(proxy) = crate::proxy::reqwest_proxy() {
                    builder = builder.proxy(proxy);
                }
                let mut response = builder
                    .build()
                    .map_err(|e| e.to_string())?
                    .get(MODEL_URL)
                    .send()
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| e.to_string())?;
                install(
                    &mut response,
                    &path,
                    MODEL_BYTES,
                    MODEL_SHA256,
                    &cancelled,
                    progress,
                )
            })();
            let final_state = if cancelled.load(Ordering::Acquire) {
                DownloadState::Cancelled
            } else {
                match result {
                    Ok(()) => DownloadState::Ready(path),
                    Err(e) => DownloadState::Failed(e),
                }
            };
            *state.lock().unwrap_or_else(|p| p.into_inner()) = final_state;
            wake();
        });
        download
    }
    pub fn state(&self) -> DownloadState {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).clone()
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
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
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
