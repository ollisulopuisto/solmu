//! Finnish hyphenation through libvoikko, detected at runtime.
//!
//! Solmu must start on machines without libvoikko, so the library is
//! opened with dlopen on first use rather than linked: no library (or no
//! Finnish dictionary) simply means no hyphenation, and wrapping falls
//! back to plain balancing. Layout-time only: inserted hyphens never touch
//! the stored or copied text.

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Mutex, OnceLock};

use crate::wrap::Hyphen;

/// Library names tried in order. Homebrew kegs are not on the default
/// dlopen search path, so the usual install locations are named outright.
#[cfg(target_os = "macos")]
const LIBRARIES: &[&str] = &[
    "libvoikko.1.dylib",
    "libvoikko.dylib",
    "/opt/homebrew/lib/libvoikko.1.dylib",
    "/usr/local/lib/libvoikko.1.dylib",
];
#[cfg(target_os = "windows")]
const LIBRARIES: &[&str] = &["libvoikko-1.dll"];
#[cfg(all(unix, not(target_os = "macos")))]
const LIBRARIES: &[&str] = &["libvoikko.so.1", "libvoikko.so"];

struct Voikko {
    handle: *mut c_void,
    hyphenate: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char,
    free_cstr: unsafe extern "C" fn(*mut c_char),
    terminate: unsafe extern "C" fn(*mut c_void),
    /// The handle must not be used from two threads at once, and keeps the
    /// library loaded until the process exits (the library is deliberately
    /// never unloaded: terminating could race a late caller).
    _library: libloading::Library,
}

// The handle is only touched through the Mutex below.
unsafe impl Send for Voikko {}

impl Drop for Voikko {
    fn drop(&mut self) {
        unsafe {
            (self.terminate)(self.handle);
        }
    }
}

fn open() -> Option<Voikko> {
    for name in LIBRARIES {
        let library = match unsafe { libloading::Library::new(name) } {
            Ok(library) => library,
            Err(_) => continue,
        };
        unsafe {
            let init = library
                .get::<unsafe extern "C" fn(
                    *mut *const c_char,
                    *const c_char,
                    *const c_char,
                ) -> *mut c_void>(b"voikkoInit")
                .ok()?;
            let terminate = *library
                .get::<unsafe extern "C" fn(*mut c_void)>(b"voikkoTerminate")
                .ok()?;
            let hyphenate = *library
                .get::<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char>(
                    b"voikkoHyphenateCstr",
                )
                .ok()?;
            let free_cstr = *library
                .get::<unsafe extern "C" fn(*mut c_char)>(b"voikkoFreeCstr")
                .ok()?;
            let mut error: *const c_char = std::ptr::null();
            // The default dictionary path; "fi" picks the Finnish one.
            let handle = init(&mut error, c"fi".as_ptr(), std::ptr::null());
            if handle.is_null() {
                if !error.is_null() {
                    log::warn!(
                        "libvoikko found but Finnish init failed: {}",
                        CStr::from_ptr(error).to_string_lossy()
                    );
                }
                return None;
            }
            log::info!("hyphenation: using {name}");
            return Some(Voikko {
                handle,
                hyphenate,
                free_cstr,
                terminate,
                _library: library,
            });
        }
    }
    None
}

static VOIKKO: OnceLock<Option<Mutex<Voikko>>> = OnceLock::new();

/// Hyphenation points libvoikko offers for `word`, as break-before
/// character indices with joint flags ('=' marks a compound joint).
/// Empty without a working libvoikko. The caller filters by word length
/// and fragment size (see [`crate::wrap`]).
pub fn hyphenate(word: &str) -> Vec<Hyphen> {
    let Some(voikko) = VOIKKO.get_or_init(|| open().map(Mutex::new)).as_ref() else {
        return Vec::new();
    };
    let word_c = match CString::new(word) {
        Ok(word) => word,
        Err(_) => return Vec::new(),
    };
    let voikko = voikko.lock().unwrap_or_else(|p| p.into_inner());
    let pattern = unsafe { (voikko.hyphenate)(voikko.handle, word_c.as_ptr()) };
    if pattern.is_null() {
        return Vec::new();
    }
    // The pattern carries one byte per Unicode character of the word:
    // '-' an ordinary point, '=' a compound joint. Index i describes the
    // gap after character i. Copy the bytes, then free libvoikko's string.
    let bytes = unsafe { CStr::from_ptr(pattern) }.to_bytes().to_vec();
    unsafe { (voikko.free_cstr)(pattern) };
    let word_chars = word.chars().count();
    bytes
        .iter()
        .take(word_chars.saturating_sub(1))
        .enumerate()
        .filter_map(|(i, mark)| match mark {
            b'-' => Some(Hyphen {
                at: i + 1,
                joint: false,
            }),
            b'=' => Some(Hyphen {
                at: i + 1,
                joint: true,
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_library_means_no_points() {
        // The test host may or may not carry libvoikko; both answers are
        // fine, but a present library must hyphenate Finnish.
        let points = super::hyphenate("käyttöliittymäsuunnittelu");
        if !points.is_empty() {
            assert!(points.iter().any(|p| p.at >= 3), "sane points: {points:?}");
        }
    }
}
