//! Harness stub: the choice enums model.rs names, with their real variants
//! and serde shapes, without the Settings store around them.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    Dark,
    Light,
    #[default]
    System,
    Messages,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontChoice {
    #[default]
    System,
    Inter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperColor {
    #[default]
    Theme,
    Beige,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoLock {
    OneMinute,
    #[default]
    FifteenMinutes,
    OneHour,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSound {
    #[default]
    #[serde(alias = "chime")]
    Receive,
    #[serde(alias = "ripple")]
    Alert,
    System,
    None,
    Custom(std::path::PathBuf),
}

/// Which chats show one notification, then stay quiet for ten minutes.
/// Variants verbatim from solmu src/settings.rs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLimit {
    #[default]
    Off,
    Direct,
    Group,
    Both,
}

/// Per-account settings, field list verbatim from solmu src/settings.rs;
/// load/save simplified for the harness (plain JSON, no file writer queue).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountSettings {
    pub send_read_receipts: bool,
    #[serde(default = "default_true")]
    pub send_typing: bool,
    #[serde(alias = "auto_download_images", default = "default_true")]
    pub auto_download: bool,
    pub last_chat: Option<String>,
    /// Chats open as tabs, in bar order; the active one is `last_chat`.
    pub open_tabs: Vec<String>,
    #[serde(default = "default_true")]
    pub notifications: bool,
    pub limit_notifications: NotificationLimit,
    pub save_contacts_to_phone: bool,
    pub wallpaper_image: Option<std::path::PathBuf>,
    pub history_prefetch: bool,
    pub list_filter: Option<String>,
    pub chat_category: Option<crate::model::ChatFilter>,
    pub favorites_only: bool,
    pub chat_queue: Option<crate::model::ChatFilter>,
    pub list_mode: Option<crate::model::ListMode>,
    pub call_filter: Option<String>,
}

fn default_true() -> bool {
    true
}

/// The app-wide legacy settings account.rs migrates from; only the fields
/// AccountSettings::from_legacy reads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub beeper: BeeperPreferences,
    pub send_read_receipts: bool,
    pub send_typing: bool,
    pub auto_download: bool,
    pub last_chat: Option<String>,
    pub notifications: bool,
}

impl AccountSettings {
    pub fn load(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn from_legacy(settings: &Settings) -> Self {
        Self {
            send_read_receipts: settings.send_read_receipts,
            send_typing: settings.send_typing,
            auto_download: settings.auto_download,
            last_chat: settings.last_chat.clone(),
            notifications: settings.notifications,
            ..Self::default()
        }
    }

    pub fn save_to(
        &self,
        _files: &fastframe_file::Writer,
        path: &std::path::Path,
    ) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

impl AccountSettings {
    /// Direct save used by the legacy migration path.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BeeperPreferences {
    pub forwarded_messages: Vec<crate::composed_forward::ForwardedMessage>,
}
