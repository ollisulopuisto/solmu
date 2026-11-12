//! One linked WhatsApp number inside the process.
//!
//! The window's [`crate::app::App`] owns a list of these. Views draw the
//! active account through `App`'s `Deref`. Each account has its own backend
//! thread, archive, and folders.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::app::{AttachmentDraft, ComposerMention, Conversation};
use crate::backend::{Backend, LinkStatus, Waker};
use crate::model::{
    AccountId, Chat, ChatFilter, ChatId, Contact, Label, Message, PollDraft, StickerPack,
};
use crate::paths::{AccountDirs, AppDirs};
use crate::settings::AccountSettings;

/// One WhatsApp account's runtime state.
pub struct Account {
    /// Pending mutations and outcomes stay with their owning account.
    pub batch_ui: crate::model::BatchState,
    pub id: AccountId,
    pub forward_dialog_hidden: bool,
    pub forward_generation: u64,
    pub forward_items: Vec<crate::composed_forward::Item>,
    pub dirs: AccountDirs,
    pub settings: AccountSettings,
    pub(crate) settings_dirty: bool,
    pub backend: Backend,
    pub link: LinkStatus,
    pub syncing: bool,
    pub sync_percent: Option<u32>,
    pub me: Option<String>,
    pub me_lid: Option<String>,
    pub me_name: Option<String>,
    pub me_about: Option<String>,
    pub chats: Vec<Chat>,
    /// The favorites in bar order as the backend last numbered them. `None`
    /// until it answers: the chat flags stand alone, as in demo mode.
    pub favorites: Option<Vec<ChatId>>,
    pub contacts: HashMap<String, Contact>,
    pub conversations: HashMap<ChatId, Conversation>,
    pub pending_media: HashMap<String, crate::pending_media::Record>,
    pub open_chat: Option<ChatId>,
    /// Recently opened chats, most recent last; `open_chat` is the open
    /// one and is always a member. Feeds the Cmd+K switcher; was the tab
    /// strip's order.
    pub open_tabs: Vec<ChatId>,
    /// The background-history state last sent to this account's worker: the
    /// switch and the chat on screen.
    pub(crate) prefetch_sent: Option<(bool, Option<ChatId>)>,
    pub scroll_chat_into_view: Option<ChatId>,
    pub drafts: HashMap<ChatId, String>,
    /// Whether the open chat had a draft when opened. Keep its list priority
    /// stable while the reader types, clears, or sends that draft.
    pub(crate) open_draft_priority: bool,
    /// This account's media cache as last measured, for Settings. `None`
    /// until a measurement succeeds, so a failed one shows no numbers
    /// rather than zeroes.
    pub storage_stats: Option<crate::model::StorageStats>,
    /// The measurement being waited for; an older answer is dropped.
    pub storage_token: u64,
    /// Whether the media cache is being emptied.
    pub storage_clearing: bool,
    pub(crate) attachment_drafts: HashMap<ChatId, AttachmentDraft>,
    pub(crate) draft_mentions: HashMap<ChatId, Vec<ComposerMention>>,
    pub search: String,
    pub search_selected: Option<ChatId>,
    pub search_hits: Vec<Message>,
    pub typing: HashMap<ChatId, Vec<(String, Instant)>>,
    pub presence: HashMap<String, crate::app::Presence>,
    pub account_receipts_off: bool,
    pub(crate) avatars: HashMap<String, Option<PathBuf>>,
    pub(crate) avatar_requests: HashSet<String>,
    avatar_checks: HashMap<String, std::time::Instant>,
    pub(crate) avatars_full: HashMap<String, Option<PathBuf>>,
    pub(crate) avatar_full_requests: HashSet<String>,
    pub(crate) played_told: HashSet<String>,
    pub stickers: Vec<PathBuf>,
    pub stickers_received: Vec<PathBuf>,
    pub stickers_saved: Vec<PathBuf>,
    pub sticker_packs: Vec<StickerPack>,
    pub stickers_pending: bool,
    pub sticker_import_pending: bool,
    pub sticker_link: String,
    pub poll_draft: PollDraft,
    /// What the location dialog has to read, cleared when it opens.
    pub location_draft: String,
    pub poll_creating: bool,
    pub poll_voting: HashSet<(ChatId, String)>,
    pub contact_edit: Option<(String, String)>,
    pub new_contact_phone: String,
    pub new_contact_name: String,
    pub new_contact_last: String,
    pub new_contact_pending: bool,
    pub(crate) new_contact_sequence: u64,
    pub(crate) new_contact_request: Option<u64>,
    pub(crate) new_contact_dialog_request: Option<u64>,
    /// The group being put together in the New chat dialog, if any.
    pub new_group: Option<crate::new_group::Draft>,
    pub(crate) new_group_sequence: u64,
    /// The `CreateGroup` the window waits for; an earlier one's answer files
    /// its group and opens nothing.
    pub(crate) new_group_request: Option<u64>,
    /// Draft in the per-message transcription language dialog.
    pub transcription_language_draft: String,
    /// Send Later dialog draft: custom date (YYYY-MM-DD) and time (HH:MM),
    /// plus a validation error to show. Prefilled when the dialog opens.
    pub send_later_date: String,
    pub send_later_time: String,
    pub send_later_error: Option<String>,
    /// Queue item the Send Later dialog is replacing: cancelled by the
    /// daemon only once the replacement is queued.
    pub send_later_replaces: Option<String>,
    /// Composer text stashed while editing a queued message, restored if
    /// the dialog is dismissed without scheduling.
    pub send_later_draft_backup: Option<String>,
    /// The scheduler queue as the companion service last reported it;
    /// None while the first fetch is in flight or after a failure.
    pub scheduled_messages: Option<Vec<crate::scheduler::ScheduledMessage>>,
    pub show_archived: bool,
    pub chat_filter: ChatFilter,
    /// The chat list's mode (Recent, Focus or Needs me); Recent by default.
    pub list_mode: crate::model::ListMode,
    pub category_filter: Option<ChatFilter>,
    pub favorites_only: bool,
    /// The chip the Calls page's history shows.
    pub labels: Vec<Label>,
    /// Starred messages, newest star first, and whether the left panel lists
    /// them instead of the chats.
    pub starred: Vec<crate::archive::Starred>,
    pub show_starred: bool,
    /// Whether the left panel lists the locally saved messages instead of
    /// the chats, and the text the panel's own search filters them by.
    pub show_saved: bool,
    pub saved_search: String,
    /// Ids of the starred messages of each chat, for the mark in the
    /// conversation. Filled when a chat opens and on every confirmed star.
    pub stars: HashMap<ChatId, HashSet<String>>,
    /// Ids of each chat's pinned messages, for its menu and its bubble mark.
    pub pins: HashMap<ChatId, HashSet<String>>,
    /// Active pins of each chat, for the line under its header.
    pub chat_pins: HashMap<ChatId, Vec<crate::archive::Pinned>>,
    /// A starred row asked to reply: the quote starts once the message it
    /// points at is loaded, because the composer drops a quote whose message
    /// has not arrived yet.
    pub reply_when_loaded: Option<(ChatId, String)>,
    pub account_privacy: crate::privacy::Snapshot,
    pub interactive_sending: HashSet<(ChatId, String)>,
    pub group_saving: HashSet<ChatId>,
    pub(crate) reported_online: Option<bool>,
    /// Chats this account may pin; WhatsApp Plus raises it once known.
    pub pin_limit: usize,
}

impl Account {
    /// Creates isolated chat, draft and backend state for one linked account.
    pub fn new(
        id: AccountId,
        dirs: AccountDirs,
        settings: AccountSettings,
        backend: Backend,
    ) -> Self {
        let open_chat = settings.last_chat.clone();
        let mut open_tabs = settings.open_tabs.clone();
        if let Some(chat) = &open_chat
            && !open_tabs.contains(chat)
        {
            open_tabs.push(chat.clone());
        }
        let chat_filter = match settings
            .list_filter
            .as_deref()
            .and_then(crate::model::ListFilter::parse)
        {
            Some(crate::model::ListFilter::Chip(chip)) => chip,
            _ => ChatFilter::All,
        };
        // The retired Filters row must not leave an invisible saved scope.
        // Explicit palette filters remain available during the session.
        let category_filter = None;
        let favorites_only = false;
        let chat_filter = settings.chat_queue.unwrap_or(chat_filter);
        let list_mode = settings.list_mode.unwrap_or_else(|| match chat_filter {
            ChatFilter::Unread | ChatFilter::ReplyLater | ChatFilter::ReadLater => {
                crate::model::ListMode::NeedsMe
            }
            _ => crate::model::ListMode::Recent,
        });
        Self {
            batch_ui: Default::default(),
            id,
            forward_dialog_hidden: false,
            forward_generation: 0,
            forward_items: Vec::new(),
            dirs,
            settings,
            settings_dirty: false,
            backend,
            link: LinkStatus::Starting,
            syncing: false,
            sync_percent: None,
            me: None,
            me_lid: None,
            me_name: None,
            me_about: None,
            chats: Vec::new(),
            favorites: None,
            contacts: HashMap::new(),
            conversations: HashMap::new(),
            pending_media: HashMap::new(),
            open_chat,
            open_tabs,
            prefetch_sent: None,
            scroll_chat_into_view: None,
            drafts: HashMap::new(),
            open_draft_priority: false,
            storage_stats: None,
            storage_token: 0,
            storage_clearing: false,
            draft_mentions: HashMap::new(),
            attachment_drafts: HashMap::new(),
            search: String::new(),
            search_selected: None,
            search_hits: Vec::new(),
            typing: HashMap::new(),
            presence: HashMap::new(),
            account_receipts_off: false,
            avatars: HashMap::new(),
            avatar_requests: HashSet::new(),
            avatar_checks: HashMap::new(),
            avatars_full: HashMap::new(),
            avatar_full_requests: HashSet::new(),
            played_told: HashSet::new(),
            stickers: Vec::new(),
            stickers_received: Vec::new(),
            stickers_saved: Vec::new(),
            sticker_packs: Vec::new(),
            stickers_pending: false,
            sticker_import_pending: false,
            sticker_link: String::new(),
            poll_draft: Default::default(),
            location_draft: String::new(),
            poll_creating: false,
            poll_voting: HashSet::new(),
            contact_edit: None,
            new_contact_phone: String::new(),
            new_contact_name: String::new(),
            new_contact_last: String::new(),
            new_contact_pending: false,
            new_contact_sequence: 0,
            new_contact_request: None,
            new_contact_dialog_request: None,
            new_group: None,
            new_group_sequence: 0,
            new_group_request: None,
            transcription_language_draft: String::new(),
            send_later_date: String::new(),
            send_later_time: String::new(),
            send_later_error: None,
            send_later_replaces: None,
            send_later_draft_backup: None,
            scheduled_messages: None,
            show_archived: false,
            chat_filter: ChatFilter::All,
            list_mode,
            category_filter,
            favorites_only,
            labels: Vec::new(),
            starred: Vec::new(),
            show_starred: false,
            show_saved: false,
            saved_search: String::new(),
            stars: HashMap::new(),
            pins: HashMap::new(),
            chat_pins: HashMap::new(),
            reply_when_loaded: None,
            account_privacy: crate::privacy::Snapshot::default(),
            interactive_sending: HashSet::new(),
            group_saving: HashSet::new(),
            reported_online: None,
            pin_limit: crate::backend::PINNED_CHATS,
        }
    }

    /// Opens a live account from disk, migrating older global fields onto account 1.
    pub fn spawn(
        app_dirs: &AppDirs,
        id: AccountId,
        legacy: &crate::settings::Settings,
        waker: &Waker,
    ) -> std::io::Result<Self> {
        let dirs = app_dirs.account(&id);
        dirs.ensure()?;
        let path = dirs.settings_file();
        let mut settings = if path.exists() {
            AccountSettings::load(&path)
        } else if id.as_str() == "1" {
            // Only `App::new` spawns account 1 (an added account is numbered
            // 2 or higher), before the window's thread is marked, so this
            // write may wait for the disk and still fail the start.
            let migrated = AccountSettings::from_legacy(legacy);
            migrated.save(&path)?;
            migrated
        } else {
            AccountSettings::default()
        };
        resolve_wallpaper_path(&dirs, &mut settings);
        let backend = Backend::spawn_beeper(
            dirs.clone(),
            waker.clone(),
            crate::backend::beeper::BeeperConfig::from_env()
                .expect("Beeper session or environment config"),
        );
        backend.send(crate::backend::Command::BeeperResumeForwards(
            legacy.beeper.forwarded_messages.clone(),
        ));
        Ok(Self::new(id, dirs, settings, backend))
    }

    pub fn detached(
        app_dirs: &AppDirs,
        id: AccountId,
        settings: AccountSettings,
    ) -> std::io::Result<(Self, std::sync::mpsc::Sender<crate::backend::Event>)> {
        let dirs = app_dirs.account(&id);
        dirs.ensure()?;
        let (backend, events) = Backend::detached();
        Ok((Self::new(id, dirs, settings, backend), events))
    }

    /// Whether the device has linked data, including while offline.
    pub fn is_linked(&self) -> bool {
        matches!(
            self.link,
            LinkStatus::Connected | LinkStatus::Connecting | LinkStatus::Disconnected { .. }
        ) || (!self.chats.is_empty() && !matches!(self.link, LinkStatus::LoggedOut))
    }

    /// The name the switcher shows: the profile name, else the number,
    /// else "Account" and its number here.
    pub fn display_label(&self, locale: crate::i18n::Locale) -> String {
        if let Some(name) = self
            .me_name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            return name.trim().to_owned();
        }
        if let Some(phone) = self.phone() {
            return phone;
        }
        crate::i18n::gettext(locale, "Account {id}").replace("{id}", self.id.as_str())
    }

    /// Returns or requests a cached profile picture from this account.
    pub fn avatar(&mut self, id: &str) -> Option<PathBuf> {
        let beeper = self.chats.iter().any(|chat| chat.backend_account.is_some());
        if beeper {
            let expired = self
                .avatar_checks
                .get(id)
                .is_some_and(|at| at.elapsed() > std::time::Duration::from_secs(900));
            let missing = self
                .avatars
                .get(id)
                .and_then(|p| p.as_ref())
                .is_some_and(|p| !p.is_file());
            if expired || missing {
                self.avatars.remove(id);
                self.avatar_checks.remove(id);
            }
            if self.avatar_checks.len() >= 2048
                && !self.avatar_checks.contains_key(id)
                && let Some(key) = self.avatar_checks.keys().next().cloned()
            {
                self.avatar_checks.remove(&key);
                self.avatars.remove(&key);
            }
        }
        if let Some(known) = self.avatars.get(id) {
            return known.clone();
        }
        if self.avatar_requests.insert(id.to_owned()) {
            if beeper {
                self.avatar_checks
                    .insert(id.to_owned(), std::time::Instant::now());
            }
            self.backend.send(crate::backend::Command::FetchAvatar {
                id: id.to_owned(),
                full: false,
            });
        }
        None
    }

    /// Our phone number, formatted, once WhatsApp has told us.
    pub fn phone(&self) -> Option<String> {
        let me = self.me.as_deref()?;
        crate::model::phone_of(me).map(crate::util::phone)
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    /// Hands the account's settings to the app's file writer.
    pub fn save_settings(&mut self, files: &fastframe_file::Writer) {
        self.settings_dirty = false;
        if let Err(error) = self.settings.save_to(files, &self.dirs.settings_file()) {
            log::warn!("could not save account settings: {error}");
        }
    }
}

fn resolve_wallpaper_path(dirs: &AccountDirs, settings: &mut AccountSettings) {
    if settings
        .wallpaper_image
        .as_ref()
        .is_some_and(|path| path.exists())
    {
        return;
    }
    for extension in ["jpg", "png", "webp", "gif"] {
        let path = dirs.wallpaper_file(extension);
        if path.exists() {
            settings.wallpaper_image = Some(path);
            return;
        }
    }
    if settings
        .wallpaper_image
        .as_ref()
        .is_some_and(|path| !path.exists())
    {
        settings.wallpaper_image = None;
    }
}
