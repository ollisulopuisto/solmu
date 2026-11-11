//! Test harness crate: solmu sources (staged from current src/ by check-source.sh) + minimal stubs for
//! leaf modules outside the Beeper adapter's reach. See Cargo.toml.
pub mod account;
pub mod app;
pub mod app_lock;
pub mod archive;
pub mod backend;
pub mod group_admin;
pub mod i18n;
pub mod markup;
pub mod mentions;
pub mod model;
pub mod new_group;
pub mod paths;
pub mod pending_media;
pub mod privacy;
pub mod safety;
pub mod scheduler;
pub mod scheduler_client;
pub mod settings;
pub mod ui;
pub mod updates;
pub mod util;

pub mod merged;

// Credential access is outside this portable adapter harness; source sign-in harness tests it separately.
pub mod beeper_signin { pub fn stored(_: &str) -> Result<String, String> { Ok(String::new()) } }

// Opus encoding needs libopus, which this portable adapter harness does not build.
// The full tree tests the real encoder; here a voice send reports it is unavailable.
pub mod voice {
    pub const RATE: u32 = 48_000;
    pub fn normalize(_: &mut [f32]) {}
    pub fn encode(_: &[f32]) -> Result<Vec<u8>, String> { Err("Opus encoder is not part of the adapter harness".into()) }
}
