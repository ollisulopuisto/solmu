//! Solmu internals exposed for diagnostics and tests.

pub mod a11y;
pub mod account;
pub mod animation;
pub mod announcer;
pub mod app;
pub mod app_lock;
#[cfg(target_os = "linux")]
pub mod appearance;
pub mod archive;
pub mod audio;
pub mod autostart;
pub mod backdrop;
pub mod backend;
pub mod beeper_signin;
pub mod bidi;
pub mod biometrics;
pub mod categories;
pub mod chat_search_names;
pub mod composed_forward;
#[cfg(any(test, feature = "demo"))]
pub mod demo;
pub mod emoji;
pub mod energy;
pub mod face_detect;
pub mod flat_groups;
pub mod geo;
pub mod ghost_groups;
pub mod group_admin;
pub mod hidden_messages;
mod hyphenate;
pub mod i18n;
pub mod image_cache;
pub mod image_preview;
pub mod link_policy;
pub mod link_preview;
mod linked_read;
pub mod local_avatars;
pub mod local_whisper;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod markup;
pub mod media_pause;
pub mod mentions;
pub mod model;
pub mod model_presence;
pub mod motion;
pub mod nav;
pub mod new_group;
pub mod notify;
pub mod opener;
pub mod paths;
pub mod pending_media;
pub mod preview_worker;
pub mod privacy;
pub mod proxy;
pub mod read_api;
pub mod read_http;
pub mod recurrence;
pub mod run_widths;
pub mod safety;
pub mod saved;
pub mod scheduler;
pub mod scheduler_api;
pub mod scheduler_client;
pub mod settings;
pub mod single_instance;
pub mod sticker_meta;
pub mod sticker_search;
pub mod target;
pub mod text_replace;
pub mod theme;
pub mod timestretch;
pub mod transcribe_worker;
pub mod transcriber;
pub mod transcriber_engine;
pub mod transcript;
pub mod ui;
pub mod updates;
pub mod util;
pub mod version;
pub mod video;
pub mod voice;
pub mod wallpaper;
pub mod whatsnew;
mod wrap;

pub mod merged;
pub mod suggest;

pub mod whisper_runtime;

pub mod reply_graph;

pub mod audience_queue;
pub mod reply_router;

pub mod rich_edit;
pub mod write_grants;

pub mod write_journal;

pub mod write_api;

pub mod automation_client;

pub mod automation_mcp;

pub mod literal_hints;
