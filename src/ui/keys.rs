//! Keyboard shortcuts.

use egui::{Key, Modifiers};

use crate::app::App;
use crate::model::{Action, Chat, Dialog, InfoView, Page, Scroll};

/// Whether Home, End and Ctrl+End scroll the open chat rather than move the
/// cursor of the focused text field (#456).
fn jump_keys_reach_chat(editing_text: bool, composer_focused: bool, composer_empty: bool) -> bool {
    !editing_text || (composer_focused && composer_empty)
}

/// Routes keyboard commands to actions, respecting overlays and text focus.
pub fn handle(app: &mut App, ctx: &egui::Context) {
    if app.flat_group_ui.assigning.is_some() {
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            app.flat_group_ui.assigning = None;
        }
        return;
    }
    if app.picture_edit.is_some() {
        editor_keys(app, ctx);
        return;
    }
    if app.image_preview.is_some() {
        video_keys(app, ctx);
        preview_keys(app, ctx);
        return;
    }
    if app.sticker_view.is_some() {
        sticker_view_keys(app, ctx);
        return;
    }
    if app.page == Page::Chats
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && app.image_preview.is_none()
        && !egui::Popup::is_any_open(ctx)
    {
        let networks = app.beeper_browse_networks();
        if !networks.is_empty() {
            let shortcut = ctx.input_mut(|input| {
                let keys = [
                    Key::Num1,
                    Key::Num2,
                    Key::Num3,
                    Key::Num4,
                    Key::Num5,
                    Key::Num6,
                    Key::Num7,
                    Key::Num8,
                    Key::Num9,
                    Key::Num0,
                ];
                let mut found = None;
                input.events.retain(|event| {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    {
                        let mode = if modifiers.matches_exact(Modifiers::COMMAND) {
                            Some(0)
                        } else if (cfg!(target_os = "macos")
                            && modifiers.command
                            && modifiers.ctrl
                            && !modifiers.alt
                            && !modifiers.shift)
                            || (!cfg!(target_os = "macos")
                                && modifiers.matches_exact(Modifiers::COMMAND | Modifiers::SHIFT))
                        {
                            Some(2)
                        } else {
                            None
                        };
                        if let Some(mode) = mode
                            && let Some(index) = keys.iter().position(|candidate| candidate == key)
                        {
                            // Ctrl+Shift+1 is the documented Chats destination
                            // off macOS, not the hidden visible-row shortcut.
                            if !cfg!(target_os = "macos") && mode == 2 && index == 0 {
                                return true;
                            }
                            found = Some((mode, index));
                            return false;
                        }
                    }
                    true
                });
                found
            });
            if let Some((mode, index)) = shortcut {
                match mode {
                    // Command+digit belongs to the favorites bar, as it does
                    // without networks; the bar order is the shared helper's.
                    0 => {
                        if let Some(chat) = super::chats::favorite_bar_chats(app).get(index) {
                            app.actions.push(Action::OpenChat(chat.id.clone()));
                        }
                    }
                    2 => {
                        let ids: Vec<_> = app
                            .visible_chats()
                            .into_iter()
                            .map(|chat| chat.id.clone())
                            .collect();
                        if let Some(id) = ids.get(index) {
                            app.actions.push(Action::OpenChat(id.clone()));
                        }
                    }
                    _ => {}
                }
                return;
            }
        }
    }
    let editing_text = ctx.text_edit_focused();
    let find = find_action(app);
    let archive = archive_action(app);
    let read_later = read_later_action(app);
    let reply_later = reply_later_action(app);
    let next_account = account_switch_action(app, true);
    let prev_account = account_switch_action(app, false);
    let composer_focused = ctx.memory(|memory| memory.has_focus(egui::Id::new("composer-text")));
    // Home/End (and Ctrl+End) move the text cursor in a field with text in
    // it, as in any editor; they scroll the chat only from an empty composer
    // or with no field focused.
    let jump_allowed =
        jump_keys_reach_chat(editing_text, composer_focused, app.composer.is_empty());
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        // The message list's key counts only with its modifier alone:
        // `consume_key` would also take Alt+Shift+2, and on macOS
        // Command+Control+2, which is Command+2's favorite.
        #[cfg(target_os = "macos")]
        let messages_key_free = input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Num2,
                    pressed: true,
                    modifiers,
                    ..
                } if messages_modifier_alone(*modifiers)
            )
        });
        let mut key = |modifiers: Modifiers, key: Key, action: Action| {
            if input.consume_key(modifiers, key) {
                actions.push(action);
            }
        };
        // Checked first: a plain Ctrl+F binding also matches Ctrl+Shift+F.
        let (modifiers, key_code) = SEARCH_CHATS.keys[0];
        key(modifiers, key_code, Action::FocusSearch);
        let (modifiers, key_code) = FIND.keys[0];
        key(modifiers, key_code, find);
        // App lock lives on the Option variant, when a PIN is set.
        if app.settings.app_lock_hash.is_some() {
            key(
                Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
                Key::L,
                Action::LockApp,
            );
        }
        let (modifiers, key_code) = JUMP_TO_CHAT.keys[0];
        key(modifiers, key_code, Action::ShowDialog(Dialog::JumpToChat));
        let (modifiers, key_code) = PALETTE.keys[0];
        key(modifiers, key_code, Action::ShowDialog(Dialog::Palette));
        if let Some(read_later) = read_later {
            let (modifiers, key_code) = READ_LATER.keys[0];
            key(modifiers, key_code, read_later);
        }
        // Cmd+Alt+I is consumed before plain Cmd+I: consume_key also matches
        // when the event carries extra modifiers, so Reply later would
        // otherwise swallow the info-sidebar chord.
        let (modifiers, key_code) = INFO_SIDEBAR.keys[0];
        key(modifiers, key_code, Action::ToggleInfoSidebar);
        if let Some(reply_later) = reply_later {
            let (modifiers, key_code) = REPLY_LATER.keys[0];
            key(modifiers, key_code, reply_later);
        }
        if let Some(next_account) = next_account {
            let (modifiers, key_code) = NEXT_ACCOUNT.keys[0];
            key(modifiers, key_code, next_account);
        }
        if let Some(prev_account) = prev_account {
            let (modifiers, key_code) = NEXT_ACCOUNT.keys[1];
            key(modifiers, key_code, prev_account);
        }
        if app.is_linked() {
            let (modifiers, key_code) = NEW_CHAT.keys[0];
            key(modifiers, key_code, Action::ShowDialog(Dialog::NewChat));
        }
        if !editing_text {
            let (modifiers, key_code) = SHORTCUTS_DIALOG.keys[0];
            key(modifiers, key_code, Action::ShowDialog(Dialog::Shortcuts));
        }
        if let Some(archive) = archive {
            let (modifiers, key_code) = ARCHIVE.keys[0];
            key(modifiers, key_code, archive);
        }
        let (modifiers, key_code) = SIDEBAR.keys[0];
        key(modifiers, key_code, Action::ToggleSidebar);
        let (modifiers, key_code) = FLAT_GROUPS.keys[0];
        key(modifiers, key_code, Action::ToggleFlatGroups);
        let (modifiers, key_code) = SETTINGS.keys[0];
        key(modifiers, key_code, Action::ToggleSettings);
        let (modifiers, key_code) = QUIT.keys[0];
        key(modifiers, key_code, Action::Quit);
        let (modifiers, key_code) = CLOSE_WINDOW.keys[0];
        key(modifiers, key_code, Action::CloseWindow);
        // Cmd+W owns CloseWindow everywhere; plain Ctrl+W closes the active
        // chat only where it is not the same physical shortcut.
        if cfg!(target_os = "macos") {
            let (modifiers, key_code) = CLOSE_CHAT.keys[0];
            key(modifiers, key_code, Action::CloseChat);
        }
        let (modifiers, key_code) = SHORTCUTS_DIALOG.keys[1];
        key(modifiers, key_code, Action::ShowDialog(Dialog::Shortcuts));
        if jump_allowed {
            let (modifiers, key_code) = SCROLL_BOTTOM.keys[0];
            key(modifiers, key_code, Action::ScrollToBottom);
        }
        // The message list keeps Control+2 on macOS, where Command+1 to 9
        // open favorites and Option with a digit types characters some
        // keyboard layouts need, such as # and €. Elsewhere Alt+2 now
        // picks the second network, so the list is reached by Tab.
        #[cfg(target_os = "macos")]
        if app.dialog.is_none()
            && !app.show_update
            && app.picker.is_none()
            && app.reaction_target.is_none()
            && app.recording.is_none()
            // The call screen covers the chats.
                && messages_key_free
        {
            #[cfg(target_os = "macos")]
            {
                let (modifiers, key_code) = FOCUS_MESSAGES.keys[0];
                key(modifiers, key_code, Action::FocusMessages);
            }
        }
        // The rail's Chats, by the digit's place on the keyboard: with
        // Shift held, layouts report the digit as its symbol.
        // Ctrl+1..0 without Shift open favorites by their place in the bar.
        if app.is_linked()
            && let Some(1) = take_destination(input)
        {
            actions.extend(super::rail::Destination::Chats.actions(app));
        }
    });
    // Exact modifiers keep the linked search variant and app-lock binding
    // separate. Linked Cmd-R is handled before its composer in merged::show.
    if message_shortcuts_allowed(app, ctx) {
        let ordinary_reply = latest_incoming_action(app);
        if let Some(ordinary_reply) = ordinary_reply
            && take_exact(ctx, Modifiers::COMMAND, Key::R)
        {
            if app.selection.is_some() {
                actions.push(Action::CancelSelection);
            }
            actions.push(ordinary_reply);
        }
        if take_exact(ctx, Modifiers::COMMAND, Key::L)
            && let Some(forward) = forward_action(app)
        {
            actions.push(forward);
        }
    }
    drop_shortcut_text(ctx);
    // Escape cancels the topmost state. Menus handle Escape themselves.
    let menu_open = egui::Popup::is_any_open(ctx);
    let search_focused = ctx.memory(|memory| memory.has_focus(egui::Id::new("chat-search")));
    if app.chat_view()
        && app.selection.as_ref().is_some_and(|(chat, selected)| {
            app.open_chat.as_ref() == Some(chat) && !selected.is_empty()
        })
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && !app.chat_search_calendar
        && !menu_open
        // The selection bar replaces the composer. Its previous frame's
        // focus must not swallow Copy before egui drops that hidden field.
        && (!editing_text || composer_focused)
        && ctx.input(|input| input.events.iter().any(is_copy_event))
        && let Some(text) = super::conversation::selected_text(app)
    {
        // Native clipboard commands arrive as Event::Copy, including Copy
        // from the macOS menu. Consume only when we have text to export;
        // otherwise leave Copy available to egui's text-selection hook.
        ctx.input_mut(|input| {
            input.events.retain(|event| !is_copy_event(event));
        });
        actions.push(Action::CopyText(text));
    }
    // The focused message list takes the arrows, Home, End and the page keys
    // to move its current message, ahead of the scrolling below.
    if app.chat_view()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && !menu_open
        && let Some(chat) = app.open_chat.clone()
        && ctx.memory(|memory| memory.has_focus(crate::ui::conversation::messages_list_id(&chat)))
        && let Some(step) =
            crate::ui::list_nav::take_step(ctx, || app.message_page_rows(ctx, &chat))
    {
        actions.push(Action::MessageCursor(step));
    }
    // PgUp/PgDn/Home/End scroll the open chat. PgUp/PgDn also work while the
    // composer has focus, since egui's TextEdit does not handle them itself;
    // Home/End keep moving the text cursor in a non-empty field, as ↑ keeps
    // its normal meaning outside an empty composer.
    if app.chat_view()
        && app.open_chat.is_some()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && !menu_open
    {
        ctx.input_mut(|input| {
            if take_plain(input, Key::PageUp) {
                actions.push(Action::ScrollPage(Scroll::PageUp));
            }
            if take_plain(input, Key::PageDown) {
                actions.push(Action::ScrollPage(Scroll::PageDown));
            }
            if jump_allowed {
                if take_plain(input, Key::Home) {
                    actions.push(Action::ScrollPage(Scroll::Top));
                }
                if take_plain(input, Key::End) {
                    actions.push(Action::ScrollPage(Scroll::Bottom));
                }
            }
        });
    }
    let escape = (!menu_open || app.reaction_target.is_some())
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape));
    if escape {
        if app.show_update {
            actions.push(Action::CloseUpdate);
        } else if app.dialog.is_some() {
            actions.push(Action::CloseDialog);
        } else if app.recording.is_some() {
            actions.push(Action::CancelRecording);
        } else if app.picker.is_some() || app.reaction_target.is_some() {
            actions.push(Action::ClosePicker);
        } else if app.selection.is_some() {
            // Taken here, the key never reaches the selection bar, and
            // would otherwise fall through to closing the chat.
            actions.push(Action::CancelSelection);
        } else if app.show_starred {
            actions.push(Action::ToggleStarred);
        } else if app.chat_search_visible() && app.chat_search_calendar {
            // The day filter first, then the pane it hangs from.
            app.chat_search_calendar = false;
        } else if app.chat_search_visible() && !composer_focused {
            actions.push(Action::CloseChatSearch);
        } else if app.emoji_start.is_some() {
            actions.push(Action::CloseEmojiSuggestions);
        } else if app.mention_start.is_some() {
            actions.push(Action::CloseMentions);
        } else if !app.pending.is_empty() {
            actions.push(Action::ClearPending);
        } else if app.editing.is_some() {
            actions.push(Action::CancelEdit);
        } else if app.reply_to.is_some() {
            actions.push(Action::CancelReply);
        } else if app.info_visible() && app.group_name_edit.is_some() {
            // Cancels the group rename and keeps the panel open.
            actions.push(Action::CloseGroupName);
        } else if app.info_visible() && app.contact_edit.is_some() {
            actions.push(Action::CloseContactEdit);
        } else if app.info_visible() {
            // After what the composer was in the middle of: the media view
            // goes back to the overview, and the overview closes the panel.
            let media = matches!(
                app.info.as_ref().map(|info| info.view),
                Some(InfoView::Media(_))
            );
            actions.push(if media {
                Action::ShowInfo(InfoView::Overview)
            } else {
                Action::CloseInfo
            });
        } else if app.page == Page::Wallpaper {
            actions.push(Action::Open(Page::Settings));
        } else if app.page == Page::Settings && !app.settings_search.is_empty() {
            actions.push(Action::SearchSettings(String::new()));
        } else if app.page == Page::Settings {
            actions.push(Action::Open(Page::Chats));
        } else if search_focused || !app.search.is_empty() {
            if !app.search.is_empty() {
                actions.push(Action::Search(String::new()));
            }
            if app.open_chat.is_some() {
                actions.push(Action::FocusComposer);
            }
        } else if app.chat_search_visible() {
            actions.push(Action::CloseChatSearch);
        } else if app.open_chat.is_some() {
            actions.push(Action::CloseChat);
        } else if app.locked_folder {
            actions.push(Action::CloseLockedFolder);
        }
    }
    if escape {
        keep_focus_on_escape(app, ctx);
    }
    // Enter sends a recording because the text field is hidden.
    if app.recording.is_some()
        && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter))
    {
        actions.push(Action::SendRecording);
    }
    // Alt+Up/Down switches chats without leaving the composer, and so do
    // Ctrl+Shift+[ and Ctrl+Shift+], WhatsApp's own keys, and Control+Tab
    // with Shift for back, the tab cycling of Safari and Terminal. With
    // Shift held, US-style layouts report the brackets as braces, so both
    // spellings count; a layout with another character on the shifted key
    // has Alt.
    let step = ctx.input_mut(|input| {
        let brackets = Modifiers::COMMAND | Modifiers::SHIFT;
        let tab = take_control_tab(input);
        let mut pressed = |modifiers: Modifiers, keys: &[Key]| {
            keys.iter().any(|key| input.consume_key(modifiers, *key))
        };
        if tab == Some(1)
            || pressed(Modifiers::ALT, &[Key::ArrowDown])
            || pressed(brackets, &[Key::CloseBracket, Key::CloseCurlyBracket])
        {
            1
        } else if tab == Some(-1)
            || pressed(Modifiers::ALT, &[Key::ArrowUp])
            || pressed(brackets, &[Key::OpenBracket, Key::OpenCurlyBracket])
        {
            -1
        } else {
            0
        }
    });
    // Numbered shortcuts: Command+1..0 opens the favorites bar's nth
    // chat, Command+Option+1..9 picks the sidebar's nth filter (9 toggles
    // Unreplied), and Option+1..0 picks a network. Command and
    // Command+Option digits are free even mid-typing; Option+digit types
    // characters some layouts need, so it yields to a focused text field.
    let (number, network_number, filter_number) = if app.page == Page::Chats
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && !menu_open
    {
        let network = (!editing_text)
            .then(|| ctx.input_mut(take_network_number))
            .flatten();
        (
            ctx.input_mut(take_chat_number),
            network,
            ctx.input_mut(take_filter_number),
        )
    } else {
        (None, None, None)
    };
    if let Some(index) = network_number {
        let networks = app.beeper_browse_networks();
        if let Some(network) = networks.get(index) {
            app.select_beeper_scope(Some(network.clone()), None);
        }
    }
    match filter_number {
        // The ninth slot is the layered Unreplied toggle, as on the rail.
        Some(8) => {
            app.beeper_unreplied = !app.beeper_unreplied;
        }
        Some(index) => {
            if let Some(filter) = crate::model::ChatFilter::EVERY.get(index) {
                actions.push(Action::SetChatFilter(*filter));
            }
        }
        None => {}
    }
    if step != 0 || number.is_some() {
        // Switching chats from the keyboard is keyboard navigation, as Tab
        // is: a chat without a message field then hands its focus to the
        // message list rather than to nothing.
        ctx.data_mut(|data| data.insert_temp(crate::theme::keyboard_focus_id(), true));
        if let Some(index) = number
            && let Some(next) = super::chats::favorite_bar_chats(app)
                .get(index)
                .map(|chat| chat.id.clone())
        {
            app.search_selected = None;
            // Opening a chat must not shift the Unread list underfoot.
            if app.search.trim().is_empty() && !app.show_archived {
                actions.push(Action::KeepUnread(next.clone()));
            }
            app.scroll_chat_into_view = Some(next.clone());
            actions.push(Action::OpenChat(next));
        }
        if step != 0 {
            let visible = app.visible_chats();
            if !visible.is_empty() {
                let current = app
                    .open_chat
                    .as_ref()
                    .and_then(|open| visible.iter().position(|chat| chat.id == *open));
                let next = match current {
                    Some(index) => (index as i64 + step).rem_euclid(visible.len() as i64) as usize,
                    None => 0,
                };
                if let Some(next) = visible.get(next).map(|chat| chat.id.clone()) {
                    // Stepping through the Unread list must not shift it underfoot.
                    if app.search.trim().is_empty() && !app.show_archived {
                        actions.push(Action::KeepUnread(next.clone()));
                    }
                    app.scroll_chat_into_view = Some(next.clone());
                    actions.push(Action::OpenChat(next));
                }
            }
        }
    }
    // Arrow Up in an empty, focused composer edits the user's most recent
    // message, as WhatsApp does. The key keeps its normal meaning everywhere
    // else: it navigates open overlays and moves the cursor in a non-empty
    // field.
    let edit_previous = composer_focused
        && app.chat_view()
        && app.open_chat.is_some()
        && app
            .open_chat
            .as_deref()
            .and_then(|id| app.chat(id))
            .is_some_and(Chat::can_send)
        && app.composer.is_empty()
        && app.pending.is_empty()
        && app.editing.is_none()
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.dialog.is_none()
        && !app.show_update
        && app.recording.is_none()
        && !menu_open
        && ctx.input_mut(|input| take_plain(input, Key::ArrowUp));
    if let Some(id) = edit_previous.then(|| app.previous_own_editable()).flatten() {
        actions.push(Action::Edit(id));
    }
    app.actions.extend(actions);
}

/// Recognizes native Copy and plain Command/Ctrl+C presses.
fn is_copy_event(event: &egui::Event) -> bool {
    matches!(event, egui::Event::Copy)
        || matches!(event, egui::Event::Key {
            key: Key::C, pressed: true, modifiers, ..
        } if (modifiers.command || modifiers.ctrl)
            && !modifiers.alt && !modifiers.shift)
}

/// Takes an exact Command/Ctrl+1..0 press, leaving modified number keys alone.
fn take_chat_number(input: &mut egui::InputState) -> Option<usize> {
    let keys = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
        Key::Num0,
    ];
    let mut number = None;
    input.events.retain(|event| {
        if number.is_none()
            && let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            && modifiers.matches_exact(Modifiers::COMMAND)
            && let Some(index) = keys.iter().position(|candidate| candidate == key)
        {
            number = Some(index);
            return false;
        }
        true
    });
    number
}

/// Takes an exact Command+Option+1..9 press, the sidebar filter's place,
/// leaving modified number keys alone.
fn take_filter_number(input: &mut egui::InputState) -> Option<usize> {
    let keys = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    let mut number = None;
    input.events.retain(|event| {
        if number.is_none()
            && let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            && modifiers.matches_exact(Modifiers::COMMAND | Modifiers::ALT)
            && let Some(index) = keys.iter().position(|candidate| candidate == key)
        {
            number = Some(index);
            return false;
        }
        true
    });
    number
}

/// Takes an exact Alt/Option+1..0 press, the network's place in the rail,
/// leaving modified number keys alone.
fn take_network_number(input: &mut egui::InputState) -> Option<usize> {
    let keys = [
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
        Key::Num0,
    ];
    let mut number = None;
    input.events.retain(|event| {
        if number.is_none()
            && let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            && modifiers.matches_exact(Modifiers::ALT)
            && let Some(index) = keys.iter().position(|candidate| candidate == key)
        {
            number = Some(index);
            return false;
        }
        true
    });
    number
}

/// Takes Ctrl+Shift+1 (Cmd on macOS), the rail's Chats. It goes by the
/// key's place on the keyboard: with Shift held, each layout reports the
/// digit as a symbol of its own.
fn take_destination(input: &mut egui::InputState) -> Option<u8> {
    let (wanted_modifiers, wanted_key) = RAIL_CHATS.keys[0];
    let mut destination = None;
    input.events.retain(|event| {
        if destination.is_none()
            && let egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                modifiers,
                ..
            } = event
            && modifiers.matches_exact(wanted_modifiers)
        {
            destination = match physical_key.unwrap_or(*key) {
                found if found == wanted_key => Some(1),
                _ => None,
            };
            return destination.is_none();
        }
        true
    });
    destination
}

/// Removes this frame's first plain (unmodified) press of `key`, if any, and
/// reports whether one was found. `consume_key` is unsuitable here: it also
/// matches the key with Shift or Alt held, and a plain binding must leave
/// those combinations, such as Shift+Home for text selection, alone.
pub(crate) fn take_plain(input: &mut egui::InputState, key: Key) -> bool {
    let mut taken = false;
    input.events.retain(|event| {
        if taken {
            return true;
        }
        let matches = matches!(
            event,
            egui::Event::Key {
                key: found,
                pressed: true,
                modifiers,
                ..
            } if *found == key && *modifiers == Modifiers::NONE
        );
        taken |= matches;
        !matches
    });
    taken
}

/// Removes this frame's first Control+Tab press and reports the step it
/// asks for: forward, or back with Shift. This is the physical Control key
/// on every platform, as in Safari's and Terminal's tab cycling: macOS
/// keeps Command+Tab for its own switcher, and `consume_key` would read
/// Command as Control, so the match is by hand.
fn take_control_tab(input: &mut egui::InputState) -> Option<i32> {
    let mut step = None;
    input.events.retain(|event| {
        if step.is_some() {
            return true;
        }
        if let egui::Event::Key {
            key: Key::Tab,
            pressed: true,
            modifiers,
            ..
        } = event
            && modifiers.ctrl
            && !modifiers.command
            && !modifiers.mac_cmd
            && !modifiers.alt
        {
            step = Some(if modifiers.shift { -1 } else { 1 });
            return false;
        }
        true
    });
    step
}

/// Handles keys while the image preview is open. No chat shortcut runs, and
/// typing and clipboard input are swallowed; Left/Right browse photos, while
/// Tab, Enter, Space and Up/Down stay for the preview's own controls.
fn preview_keys(app: &mut App, ctx: &egui::Context) {
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::Escape) {
            actions.push(Action::CloseImagePreview);
        }
        let copy_shortcut = input.consume_key(Modifiers::COMMAND, Key::C)
            || input.consume_key(Modifiers::CTRL, Key::C);
        // The header leaves the copy action out while a clip is on screen, so
        // the shortcut does the same: copying a clip would decode a video as an
        // image and fail.
        if copy_shortcut
            && let Some(preview) = &app.image_preview
            && let Some(path) = preview.path()
            && !preview.shows_video(&app.viewer_media)
        {
            actions.push(Action::CopyImage(path.to_owned()));
        }
        let mut event_actions = Vec::new();
        for event in &input.events {
            if let egui::Event::Key {
                key,
                modifiers,
                pressed: true,
                ..
            } = event
            {
                event_actions.extend(crate::image_preview::preview_action(*key, *modifiers));
            }
        }
        // A clip has no zoom, as its header has no zoom controls.
        let clip = app
            .image_preview
            .as_ref()
            .is_some_and(|preview| preview.shows_video(&app.viewer_media));
        if clip {
            event_actions.retain(|action| {
                !matches!(
                    action,
                    Action::ZoomImageIn
                        | Action::ZoomImageOut
                        | Action::FitImage
                        | Action::ImageActualSize
                )
            });
        }
        actions.extend(event_actions);
        input
            .events
            .retain(|event| !crate::image_preview::consumes_key(event));
    });
    app.actions.extend(actions);
}

/// Keeps keyboard focus where it was on an Escape in the window itself,
/// anywhere in the app, after keyboard navigation or with a screen reader
/// attached; a dialog or the update notice keep their own handling of it.
/// egui lets go of focus on a plain Escape at the start of the pass, before
/// anything is drawn, unless the focused widget claims the key, and a screen
/// reader then hears focus fall to the window. Given back in this pass,
/// which draws the widget again, focus stays on it while Escape still does
/// what `handle` gives it to do: it closes or clears the topmost thing, up
/// to the open chat itself. When that takes the focused widget away, as
/// closing the chat does, focus goes with it once the widget is no longer
/// drawn, as before; with nothing to close, focus stays where it was.
fn keep_focus_on_escape(app: &App, ctx: &egui::Context) {
    if app.dialog.is_some() || app.show_update || ctx.memory(|memory| memory.focused().is_some()) {
        return;
    }
    // Only for the keyboard: a pointer user who clicked into a field sees
    // it let go on Escape, as egui does.
    if escape_keeps_focus(ctx)
        && let Some(id) = focus_before_escape(ctx)
    {
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
}

/// The widget that had focus when this pass began, among those the last
/// pass drew.
fn focus_before_escape(ctx: &egui::Context) -> Option<egui::Id> {
    let drawn: Vec<egui::Id> = ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter().map(|widget| widget.id))
            .collect()
    });
    ctx.memory(|memory| {
        drawn
            .into_iter()
            .find(|id| memory.had_focus_last_frame(*id))
    })
}

/// Whether Escape keeps focus where it was: after keyboard navigation, or
/// with a screen reader attached, which always hears where focus goes.
fn escape_keeps_focus(ctx: &egui::Context) -> bool {
    let keyboard = ctx.data(|data| {
        data.get_temp::<bool>(crate::theme::keyboard_focus_id())
            .unwrap_or(false)
    });
    // A node builder exists only while AccessKit is on.
    let screen_reader = ctx
        .accesskit_node_builder(egui::accesskit_root_id(), |_| ())
        .is_some();
    keyboard || screen_reader
}

/// Drops the text of a digit shortcut taken this frame: a digit with Alt,
/// or with Control on macOS, such as Alt+2. Windows delivers Alt+2 as the
/// key and as the text "2": with the key taken by its shortcut, the text
/// would still reach a focused field and be typed there. Control with a
/// digit types nothing.
fn drop_shortcut_text(ctx: &egui::Context) {
    let held = |modifiers: &Modifiers| {
        if cfg!(target_os = "macos") {
            modifiers.ctrl
        } else {
            modifiers.alt
        }
    };
    ctx.input_mut(|input| {
        // Taken: in this frame's input, but no longer among its events.
        let taken: Vec<&str> = input
            .raw
            .events
            .iter()
            .filter(|event| !input.events.contains(event))
            .filter_map(|event| match event {
                egui::Event::Key {
                    key:
                        key @ (Key::Num0
                        | Key::Num1
                        | Key::Num2
                        | Key::Num3
                        | Key::Num4
                        | Key::Num5
                        | Key::Num6
                        | Key::Num7
                        | Key::Num8
                        | Key::Num9),
                    pressed: true,
                    modifiers,
                    ..
                } if held(modifiers) => Some(key.name()),
                _ => None,
            })
            .collect();
        for digit in taken {
            if let Some(index) = input
                .events
                .iter()
                .position(|event| matches!(event, egui::Event::Text(text) if text == digit))
            {
                input.events.remove(index);
            }
        }
    });
}

/// Handles keys while the cropper covers the window: Escape leaves the
/// staged picture alone, Enter keeps the crop.
fn editor_keys(app: &mut App, ctx: &egui::Context) {
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::Escape) {
            actions.push(Action::CancelPictureEdit);
        }
        if input.consume_key(Modifiers::NONE, Key::Enter) {
            actions.push(Action::ApplyPictureEdit);
        }
    });
    app.actions.extend(actions);
}

/// Handles a playing clip's keys while the viewer shows it: Space plays or
/// pauses, M mutes, S changes the speed, and the arrows jump five seconds, as
/// they step through the album only while a picture is on the stage. Escape
/// closes the viewer, which puts a video back in its message.
fn video_keys(app: &mut App, ctx: &egui::Context) {
    let Some((message, path)) = app
        .video
        .message()
        .map(str::to_owned)
        .zip(app.video.path().map(std::path::Path::to_owned))
    else {
        return;
    };
    // Only the clip on the viewer: anything else was stopped as it opened.
    let on_screen = app.image_preview.as_ref().is_some_and(|preview| {
        preview.message() == message
            && preview.shows_video(&app.viewer_media)
            && preview.clip_path() == Some(path.as_path())
    });
    if !on_screen {
        return;
    }
    let status = app.video.status(&message);
    let jump = |seconds: f32| {
        let status = status.as_ref()?;
        let total = status.total.as_secs_f32();
        (total > 0.0).then(|| Action::SeekVideo {
            message: message.clone(),
            fraction: ((status.position.as_secs_f32() + seconds) / total).clamp(0.0, 1.0),
        })
    };
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        if input.consume_key(Modifiers::NONE, Key::Space) {
            actions.push(Action::PlayVideo {
                message: message.clone(),
                path: path.clone(),
            });
        }
        if input.consume_key(Modifiers::NONE, Key::M) {
            actions.push(Action::ToggleVideoSound);
        }
        if input.consume_key(Modifiers::NONE, Key::S) {
            actions.push(Action::CycleVideoSpeed);
        }
        if input.consume_key(Modifiers::NONE, Key::ArrowLeft) {
            actions.extend(jump(-5.0));
        }
        if input.consume_key(Modifiers::NONE, Key::ArrowRight) {
            actions.extend(jump(5.0));
        }
    });
    app.actions.extend(actions);
}

/// Handles keys while a sticker is shown large: Escape closes it, and no
/// chat shortcut or typing reaches the chat behind it.
fn sticker_view_keys(app: &mut App, ctx: &egui::Context) {
    let close = ctx.input_mut(|input| {
        let close = input.consume_key(Modifiers::NONE, Key::Escape);
        input
            .events
            .retain(|event| !crate::image_preview::consumes_key(event));
        close
    });
    if close {
        app.actions.push(Action::CloseStickerPreview);
    }
}

/// Consume one exact modifier chord without swallowing its shifted variants.
fn take_exact(ctx: &egui::Context, modifiers: Modifiers, key: Key) -> bool {
    ctx.input_mut(|input| {
        let mut pressed = false;
        input.events.retain(|event| {
            if matches!(event, egui::Event::Key { key: found, pressed: true, modifiers: actual, .. }
                if *found == key && actual.matches_exact(modifiers))
            {
                pressed = true;
                false
            } else {
                true
            }
        });
        pressed
    })
}

fn message_shortcuts_allowed(app: &App, ctx: &egui::Context) -> bool {
    app.chat_view()
        && app.current_chat().is_some()
        && app.dialog.is_none()
        && !app.show_update
        && app.picker.is_none()
        && app.reaction_target.is_none()
        && app.recording.is_none()
        && !egui::Popup::is_any_open(ctx)
}

fn linked_person<'a>(app: &'a App, chat: &Chat) -> Option<&'a crate::merged::Person> {
    super::merged::person_for_view(app, chat)
}

fn latest_incoming_action(app: &App) -> Option<Action> {
    let chat = app.current_chat()?;
    // Preserve the linked view's exact-source reply state and routing.
    if linked_person(app, chat).is_some() {
        return None;
    }
    app.conversations
        .get(&chat.id)?
        .messages
        .iter()
        .filter(|message| {
            message.chat == chat.id
                && !message.from_me
                && !matches!(
                    message.content,
                    crate::model::Content::Revoked | crate::model::Content::Call { .. }
                )
        })
        .max_by_key(|message| (message.timestamp, message.history_order, &message.id))
        .map(|message| Action::Reply(message.id.clone()))
}

fn forward_action(app: &App) -> Option<Action> {
    let chat = app.current_chat()?;
    if let Some((selected_chat, selected)) = &app.selection
        && selected_chat == &chat.id
    {
        return (!selected.is_empty()).then(|| {
            Action::ShowDialog(Dialog::Forward {
                chat: chat.id.clone(),
                messages: selected.clone(),
            })
        });
    }
    let candidates =
        linked_person(app, chat).map(|person| super::merged::valid_members(app, person, chat));
    let latest = app
        .conversations
        .iter()
        .filter(|(id, _)| {
            candidates.as_ref().map_or(*id == &chat.id, |members| {
                members.iter().any(|member| &member.chat == *id)
            })
        })
        .flat_map(|(id, history)| {
            history
                .messages
                .iter()
                .filter(move |message| &message.chat == id)
        })
        .filter(|message| crate::app::can_select(&message.content))
        .max_by_key(|message| {
            (
                message.timestamp,
                &message.chat,
                message.history_order,
                &message.id,
            )
        })?;
    Some(Action::ShowDialog(Dialog::Forward {
        chat: latest.chat.clone(),
        messages: vec![latest.id.clone()],
    }))
}

/// Ctrl+F has one scope: the open chat, never a different search scope.
fn find_action(_app: &App) -> Action {
    Action::OpenChatSearch
}

/// Ctrl+E archives the open chat, or unarchives an archived one, as in
/// WhatsApp. Archiving closes the chat, as the chat menus do.
fn archive_action(app: &App) -> Option<Action> {
    if !app.chat_view()
        || app.dialog.is_some()
        || app.show_update
        || app.picker.is_some()
        || app.reaction_target.is_some()
        || app.recording.is_some()
    {
        return None;
    }
    let chat = app.chat(app.open_chat.as_deref()?)?;
    Some(Action::SetArchived(chat.id.clone(), !chat.archived))
}

/// Cmd+U queues the open chat for reading later, as the chat menus do.
fn read_later_action(app: &App) -> Option<Action> {
    Some(Action::ToggleReadLater(later_queue_target(app)?))
}

/// Cmd+I queues the open chat for replying later, as the chat menus do.
fn reply_later_action(app: &App) -> Option<Action> {
    Some(Action::ToggleReplyLater(later_queue_target(app)?))
}

/// The open chat a later-queue key would mark, when no overlay owns keys.
fn later_queue_target(app: &App) -> Option<crate::model::ChatId> {
    if !app.chat_view()
        || app.dialog.is_some()
        || app.show_update
        || app.picker.is_some()
        || app.reaction_target.is_some()
        || app.recording.is_some()
    {
        return None;
    }
    let chat = app.chat(app.open_chat.as_deref()?)?;
    Some(chat.id.clone())
}

/// Ctrl+Alt+Right shows the next linked number, Ctrl+Alt+Left the
/// previous, in the switcher's order and wrapping around. An account
/// still being added is left through its own Cancel, not by these keys.
fn account_switch_action(app: &App, forward: bool) -> Option<Action> {
    if !app.has_several_accounts()
        || app.adding_account
        || app.dialog.is_some()
        || app.show_update
        || app.picker.is_some()
        || app.reaction_target.is_some()
        || app.recording.is_some()
    {
        return None;
    }
    let next = if forward {
        (app.active + 1) % app.accounts.len()
    } else {
        (app.active + app.accounts.len() - 1) % app.accounts.len()
    };
    Some(Action::SwitchAccount(app.accounts[next].id.clone()))
}

/// Control held with no other modifier: on macOS, Command+Control is
/// Command's, which `consume_key` would also match as Control.
#[cfg(any(target_os = "macos", test))]
fn control_alone(modifiers: Modifiers) -> bool {
    modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd && !modifiers.alt && !modifiers.shift
}

/// The message list's modifier held alone: Control on macOS, Alt elsewhere.
#[cfg(target_os = "macos")]
fn messages_modifier_alone(modifiers: Modifiers) -> bool {
    control_alone(modifiers)
}

/// The message list's shortcut on macOS: Control+2, where Command with a
/// digit opens a favorite. Elsewhere Alt+2 picks the second network.
#[cfg(target_os = "macos")]
const MESSAGES_KEYS: &str = "Control+2";

/// Command (Ctrl elsewhere) with Shift held, as a constant for the table:
/// `Modifiers::COMMAND | Modifiers::SHIFT` is not a const expression.
const COMMAND_SHIFT: Modifiers = Modifiers {
    shift: true,
    command: true,
    ..Modifiers::NONE
};

/// Command (Ctrl elsewhere) with Alt held, for the table.
const COMMAND_ALT: Modifiers = Modifiers {
    alt: true,
    command: true,
    ..Modifiers::NONE
};

/// One keyboard shortcut, defined once: the handler's binding, the help
/// dialog's row and the macOS menu's accelerator all come from this entry,
/// so the three surfaces cannot drift.
pub struct Shortcut {
    /// A stable name for looking the entry up from the handler and menus.
    pub id: &'static str,
    /// Every physical spelling that triggers it, in matching order.
    pub keys: &'static [(Modifiers, Key)],
    /// The help dialog's keys column, before platform relabeling. None when
    /// the binding has no row of its own (documented with a sibling).
    pub label: Option<&'static str>,
    /// The help dialog's description column.
    pub description: Option<&'static str>,
    /// The macOS menu item mirroring it: id, text and accelerator string.
    pub menu: Option<MenuItem>,
}

/// A macOS menu entry carrying a keyboard accelerator.
pub struct MenuItem {
    pub id: &'static str,
    pub text: &'static str,
    pub accelerator: &'static str,
}

/// A row of the registry: either a binding or a help-only gesture such as
/// arrows, Enter and digit ranges, which have no single binding to consume.
pub enum Row {
    Binding(&'static Shortcut),
    Help(&'static str, &'static str),
}

const JUMP_TO_CHAT: Shortcut = Shortcut {
    id: "jump-to-chat",
    keys: &[(Modifiers::COMMAND, Key::K)],
    label: Some("Ctrl+K"),
    description: Some("Jump between recent chats"),
    menu: None,
};
const SEARCH_CHATS: Shortcut = Shortcut {
    id: "search-chats",
    keys: &[(COMMAND_SHIFT, Key::F)],
    label: Some("Ctrl+Shift+F"),
    description: Some("Search all messages and chats"),
    menu: Some(MenuItem {
        id: "search-all",
        text: "Search All Messages and Chats\u{2026}",
        accelerator: "Super+Shift+KeyF",
    }),
};
const PALETTE: Shortcut = Shortcut {
    id: "palette",
    keys: &[(Modifiers::COMMAND, Key::P)],
    label: Some("Ctrl+P"),
    description: Some("Command palette: chats, filters, actions"),
    menu: None,
};
const FIND: Shortcut = Shortcut {
    id: "find",
    keys: &[(Modifiers::COMMAND, Key::F)],
    label: Some("Ctrl+F"),
    description: Some("Search within this chat"),
    menu: Some(MenuItem {
        id: "search",
        text: "Search Within This Chat\u{2026}",
        accelerator: "Super+KeyF",
    }),
};
const INCOMING_BRIDGE: Shortcut = Shortcut {
    id: "incoming-bridge",
    keys: &[(Modifiers::COMMAND, Key::R)],
    label: Some("Ctrl+R"),
    description: Some("Quote-reply to the latest incoming message (on its source in linked chats)"),
    menu: None,
};
const FORWARD: Shortcut = Shortcut {
    id: "forward-message",
    keys: &[(Modifiers::COMMAND, Key::L)],
    label: Some("Ctrl+L"),
    description: Some("Forward selected messages, or the latest message"),
    menu: None,
};
const SCROLL_BOTTOM: Shortcut = Shortcut {
    id: "scroll-bottom",
    keys: &[(Modifiers::COMMAND, Key::End)],
    label: Some("Ctrl+End"),
    description: Some("Jump to the newest message"),
    menu: None,
};
const RAIL_CHATS: Shortcut = Shortcut {
    id: "rail-chats",
    keys: &[(COMMAND_SHIFT, Key::Num1)],
    label: Some("Ctrl+Shift+1"),
    description: Some("Chats"),
    menu: None,
};
const NEW_CHAT: Shortcut = Shortcut {
    id: "new-chat",
    keys: &[(Modifiers::COMMAND, Key::N)],
    label: Some("Ctrl+N"),
    description: Some("New chat or message yourself"),
    menu: Some(MenuItem {
        id: "new",
        text: "New Chat\u{2026}",
        accelerator: "Super+KeyN",
    }),
};
const ARCHIVE: Shortcut = Shortcut {
    id: "archive",
    keys: &[(Modifiers::COMMAND, Key::E)],
    label: Some("Ctrl+E"),
    description: Some("Archive or unarchive the open chat"),
    menu: None,
};
const NEXT_ACCOUNT: Shortcut = Shortcut {
    id: "next-account",
    keys: &[
        (COMMAND_ALT, Key::ArrowRight),
        (COMMAND_ALT, Key::ArrowLeft),
    ],
    label: Some("Ctrl+Alt+\u{2192} / Ctrl+Alt+\u{2190}"),
    description: Some("Switch to the next / previous linked number"),
    menu: None,
};
const SIDEBAR: Shortcut = Shortcut {
    id: "sidebar",
    keys: &[(Modifiers::COMMAND, Key::B)],
    label: Some("Ctrl+B"),
    description: Some("Collapse or expand the chat list"),
    menu: Some(MenuItem {
        id: "sidebar",
        text: "Toggle Chat List",
        accelerator: "Super+KeyB",
    }),
};
// Approved group toggle; the info sidebar moves to Cmd-Option-I in #281.
const FLAT_GROUPS: Shortcut = Shortcut {
    id: "flat-groups",
    keys: &[(Modifiers::COMMAND, Key::D)],
    label: Some("Ctrl+D"),
    description: Some("Switch Latest and Groups"),
    menu: Some(MenuItem {
        id: "flat-groups",
        text: "Switch Latest and Groups",
        accelerator: "Super+KeyD",
    }),
};
const INFO_SIDEBAR: Shortcut = Shortcut {
    id: "info-sidebar",
    // Shared Postilaatikko/Solmu keymap: the info sidebar lives on
    // Cmd+Alt+I; Cmd+D is reserved for the groups toggle.
    keys: &[(COMMAND_ALT, Key::I)],
    label: Some("Ctrl+Alt+I"),
    description: Some("Show or hide the info sidebar"),
    menu: Some(MenuItem {
        id: "info-sidebar",
        text: "Toggle Info Sidebar",
        accelerator: "Super+Alt+KeyI",
    }),
};
const SETTINGS: Shortcut = Shortcut {
    id: "settings",
    keys: &[(Modifiers::COMMAND, Key::Comma)],
    label: Some("Ctrl+,"),
    description: Some("Settings"),
    menu: Some(MenuItem {
        id: "settings",
        text: "Settings\u{2026}",
        accelerator: "Super+Comma",
    }),
};
const SHORTCUTS_DIALOG: Shortcut = Shortcut {
    id: "shortcuts",
    keys: &[
        (Modifiers::NONE, Key::Questionmark),
        (Modifiers::COMMAND, Key::Slash),
    ],
    label: Some("? / Ctrl+/"),
    description: Some("Keyboard shortcuts (? when not typing)"),
    menu: Some(MenuItem {
        id: "shortcuts",
        text: "Keyboard Shortcuts",
        accelerator: "Super+Slash",
    }),
};
const CLOSE_WINDOW: Shortcut = Shortcut {
    id: "close-window",
    keys: &[(Modifiers::COMMAND, Key::W)],
    label: Some("Ctrl+W"),
    description: Some("Close the window (Solmu remains in the tray)"),
    menu: Some(MenuItem {
        id: "close",
        text: "Close Window",
        accelerator: "Super+KeyW",
    }),
};
const QUIT: Shortcut = Shortcut {
    id: "quit",
    keys: &[(Modifiers::COMMAND, Key::Q)],
    label: Some("Ctrl+Q"),
    description: Some("Quit"),
    menu: Some(MenuItem {
        id: "quit",
        text: "Quit Solmu",
        accelerator: "Super+KeyQ",
    }),
};
#[cfg(target_os = "macos")]
const FOCUS_MESSAGES: Shortcut = Shortcut {
    id: "focus-messages",
    keys: &[(Modifiers::CTRL, Key::Num2)],
    label: Some(MESSAGES_KEYS),
    description: Some("Go to the message list"),
    menu: None,
};
/// Bindings without a help row of their own.
const CLOSE_CHAT: Shortcut = Shortcut {
    id: "close-chat",
    keys: &[(Modifiers::CTRL, Key::W)],
    label: None,
    description: None,
    menu: None,
};
const READ_LATER: Shortcut = Shortcut {
    id: "read-later",
    keys: &[(Modifiers::COMMAND, Key::U)],
    label: Some("Ctrl+U"),
    description: Some("Mark the open chat to read later"),
    menu: None,
};
const REPLY_LATER: Shortcut = Shortcut {
    id: "reply-later",
    keys: &[(Modifiers::COMMAND, Key::I)],
    label: Some("Ctrl+I"),
    description: Some("Mark the open chat to reply later"),
    menu: None,
};
/// Menu accelerators that edit text natively rather than through `handle`:
/// winit's view is no NSTextView, so the menu sends the events itself.
const EDIT_UNDO: Shortcut = Shortcut {
    id: "undo",
    keys: &[],
    label: None,
    description: None,
    menu: Some(MenuItem {
        id: "undo",
        text: "Undo",
        accelerator: "Super+KeyZ",
    }),
};
const EDIT_REDO: Shortcut = Shortcut {
    id: "redo",
    keys: &[],
    label: None,
    description: None,
    menu: Some(MenuItem {
        id: "redo",
        text: "Redo",
        accelerator: "Super+Shift+KeyZ",
    }),
};
const EDIT_CUT: Shortcut = Shortcut {
    id: "cut",
    keys: &[],
    label: None,
    description: None,
    menu: Some(MenuItem {
        id: "cut",
        text: "Cut",
        accelerator: "Super+KeyX",
    }),
};
const EDIT_COPY: Shortcut = Shortcut {
    id: "copy",
    keys: &[],
    label: None,
    description: None,
    menu: Some(MenuItem {
        id: "copy",
        text: "Copy",
        accelerator: "Super+KeyC",
    }),
};
const EDIT_PASTE: Shortcut = Shortcut {
    id: "paste",
    keys: &[],
    label: None,
    description: None,
    menu: Some(MenuItem {
        id: "paste",
        text: "Paste",
        accelerator: "Super+KeyV",
    }),
};
const EDIT_SELECT_ALL: Shortcut = Shortcut {
    id: "select-all",
    keys: &[],
    label: None,
    description: None,
    menu: Some(MenuItem {
        id: "select-all",
        text: "Select All",
        accelerator: "Super+KeyA",
    }),
};

/// Control+Tab's help label: the physical Control key on every platform,
/// named "Control" so macOS relabels it as its own symbol, not Command's.
#[cfg(target_os = "macos")]
const TAB_STEP_KEYS: &str = "Control+Tab / Control+Shift+Tab";
#[cfg(not(target_os = "macos"))]
const TAB_STEP_KEYS: &str = "Ctrl+Tab / Ctrl+Shift+Tab";

/// Every shortcut, in the order the help dialog shows them; entries without
/// a help row follow at the end.
pub static REGISTRY: &[Row] = &[
    Row::Binding(&JUMP_TO_CHAT),
    Row::Binding(&SEARCH_CHATS),
    Row::Binding(&PALETTE),
    Row::Binding(&FIND),
    Row::Binding(&INCOMING_BRIDGE),
    Row::Binding(&FORWARD),
    Row::Help(
        "\u{2191} / \u{2193}, Enter",
        "Walk the open chat's search results and jump to one",
    ),
    Row::Help(
        "PageUp / PageDown",
        "Scroll the open chat a screen at a time",
    ),
    Row::Help(
        "Home / End",
        "Jump to the oldest loaded / newest message (with the input empty)",
    ),
    Row::Binding(&SCROLL_BOTTOM),
    Row::Help("Alt+\u{2191} / Alt+\u{2193}", "Previous / next chat"),
    Row::Help("Ctrl+Shift+[ / ]", "Previous / next chat, as in WhatsApp"),
    Row::Help(TAB_STEP_KEYS, "Next / previous chat, as in Safari's tabs"),
    Row::Help("Ctrl+1..0", "Open a favorite by its place in the sidebar"),
    Row::Help(
        "Alt+1..0",
        "Filter to a network by its place in the sidebar (outside text fields)",
    ),
    Row::Help(
        "Ctrl+Alt+1..9",
        "All, Unread, Reply later, Read later, Favorites, Private, Groups, Channels; 9 toggles Unreplied",
    ),
    Row::Help(
        "Ctrl+Alt+Shift+R",
        "Reply to the latest matching message in linked-person search",
    ),
    Row::Binding(&RAIL_CHATS),
    Row::Help(
        "\u{2191}",
        "Edit the previous message (when the input is empty)",
    ),
    Row::Help("Enter", "Send (Shift+Enter for a new line)"),
    Row::Help(
        "Escape",
        "Dismiss the current action, return from search or Starred, or close the chat",
    ),
    Row::Binding(&NEW_CHAT),
    Row::Binding(&ARCHIVE),
    Row::Binding(&READ_LATER),
    Row::Binding(&REPLY_LATER),
    Row::Binding(&NEXT_ACCOUNT),
    Row::Help(
        "Ctrl+V",
        "Paste text, or stage a picture from the clipboard",
    ),
    Row::Help("Ctrl+C", "Copy the selected messages"),
    Row::Binding(&SIDEBAR),
    Row::Binding(&INFO_SIDEBAR),
    Row::Binding(&FLAT_GROUPS),
    Row::Help(
        "Ctrl+End",
        "Jump to the newest message (when the input is empty)",
    ),
    #[cfg(target_os = "macos")]
    Row::Binding(&FOCUS_MESSAGES),
    Row::Help(
        "\u{2191} / \u{2193}, Home / End, PgUp / PgDn",
        "Move through messages (in the message list)",
    ),
    Row::Help("PgUp / PgDn", "Scroll the open chat by a page"),
    Row::Help(
        "Home / End",
        "Top / bottom of the open chat (when the input is empty)",
    ),
    Row::Binding(&SETTINGS),
    Row::Help(
        "\u{2190} / \u{2192}",
        "Previous / next item in the media viewer",
    ),
    Row::Binding(&SHORTCUTS_DIALOG),
    Row::Help("Ctrl+Alt+Shift+L", "Lock the app, when a PIN is set"),
    Row::Binding(&CLOSE_WINDOW),
    Row::Binding(&QUIT),
    Row::Binding(&CLOSE_CHAT),
    Row::Binding(&EDIT_UNDO),
    Row::Binding(&EDIT_REDO),
    Row::Binding(&EDIT_CUT),
    Row::Binding(&EDIT_COPY),
    Row::Binding(&EDIT_PASTE),
    Row::Binding(&EDIT_SELECT_ALL),
];

/// Looks a registry entry up by its stable id.
pub fn shortcut(id: &str) -> &'static Shortcut {
    REGISTRY
        .iter()
        .find_map(|row| match row {
            Row::Binding(shortcut) if shortcut.id == id => Some(*shortcut),
            _ => None,
        })
        .unwrap_or_else(|| panic!("unknown shortcut: {id}"))
}

/// The help dialog's rows, in registry order.
pub fn help_rows() -> impl Iterator<Item = (&'static str, &'static str)> {
    REGISTRY.iter().filter_map(|row| match row {
        Row::Binding(shortcut) => shortcut.label.zip(shortcut.description),
        Row::Help(label, description) => Some((*label, *description)),
    })
}

/// The muda accelerator string for a binding, for the menu keys the
/// registry carries: letters name their `Key` code, digits their `Digit`.
pub fn menu_accelerator(modifiers: Modifiers, key: Key) -> Option<String> {
    let mut parts = Vec::new();
    if modifiers.command {
        parts.push("Super");
    }
    if modifiers.ctrl {
        parts.push("Control");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    let code = match key {
        Key::A => "KeyA",
        Key::B => "KeyB",
        Key::C => "KeyC",
        Key::D => "KeyD",
        Key::F => "KeyF",
        Key::I => "KeyI",
        Key::N => "KeyN",
        Key::Q => "KeyQ",
        Key::V => "KeyV",
        Key::W => "KeyW",
        Key::X => "KeyX",
        Key::Z => "KeyZ",
        Key::Comma => "Comma",
        Key::Equals => "Equal",
        Key::Minus => "Minus",
        Key::Slash => "Slash",
        Key::Num0 => "Digit0",
        _ => return None,
    };
    parts.push(code);
    Some(parts.join("+"))
}

/// A modifier word in a key label: "Ctrl" is the platform's Command key
/// (Command on macOS, Control elsewhere), "Control" the literal key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Control,
    Option,
    Shift,
    Command,
}

impl Modifier {
    fn symbol(self) -> char {
        match self {
            // The Mac convention's glyphs.
            Self::Control => '⌃',
            Self::Option => '⌥',
            Self::Shift => '⇧',
            Self::Command => '⌘',
        }
    }
}

/// Reads a modifier word and its separator ("Ctrl+", "Alt + ") at the
/// start of `text`, returning the modifier and where the rest begins.
fn take_modifier(text: &str) -> Option<(Modifier, &str)> {
    let (word, modifier) = [
        ("Control", Modifier::Control),
        ("Ctrl", Modifier::Command),
        ("Strg", Modifier::Command),
        ("Alt", Modifier::Option),
        ("Shift", Modifier::Shift),
    ]
    .into_iter()
    .find(|(word, _)| text.starts_with(word))?;
    let rest = text[word.len()..].trim_start();
    let rest = rest.strip_prefix('+')?.trim_start();
    Some((modifier, rest))
}

/// Relabels a modifier chain at the start of `text` the Mac way: glyphs in
/// Control, Option, Shift, Command order with no separators, so "Ctrl+Shift+F"
/// and "Shift+Ctrl+Enter" alike come out with Shift before Command. Returns
/// the glyphs and the unconsumed tail, or `None` when no chain starts here.
/// A mouse gesture ("Ctrl + click") keeps a space: "⌘ click" reads as a
/// click, not a key.
fn relabel_chain(text: &str) -> Option<(String, &str)> {
    const ORDER: [Modifier; 4] = [
        Modifier::Control,
        Modifier::Option,
        Modifier::Shift,
        Modifier::Command,
    ];
    let mut found = Vec::new();
    let mut rest = text;
    while let Some((modifier, after)) = take_modifier(rest) {
        if !found.contains(&modifier) {
            found.push(modifier);
        }
        rest = after;
    }
    if found.is_empty() {
        return None;
    }
    let mut out: String = ORDER
        .into_iter()
        .filter(|modifier| found.contains(modifier))
        .map(Modifier::symbol)
        .collect();
    if rest.starts_with("click") {
        out.push(' ');
    }
    Some((out, rest))
}

/// Uses the Mac modifier convention on macOS ("⌘K", "⇧⌘F", literal Control
/// as "⌃"), for chains anywhere in the string, so tooltips and hints that
/// embed a shortcut in a sentence relabel too. Other platforms keep the
/// label as written.
pub fn label(keys: &str) -> String {
    label_for(keys, cfg!(target_os = "macos"))
}

fn label_for(keys: &str, macos: bool) -> String {
    if !macos {
        return keys.to_owned();
    }
    let mut out = String::with_capacity(keys.len());
    let mut rest = keys;
    while !rest.is_empty() {
        if let Some((glyphs, tail)) = relabel_chain(rest) {
            out.push_str(&glyphs);
            rest = tail;
            continue;
        }
        // The next chain can start after any non-alphanumeric boundary.
        let next = rest
            .char_indices()
            .skip(1)
            .find(|(index, _)| {
                !rest[..*index].ends_with(|c: char| c.is_alphanumeric())
                    && relabel_chain(&rest[*index..]).is_some()
            })
            .map(|(index, _)| index);
        match next {
            Some(index) => {
                out.push_str(&rest[..index]);
                rest = &rest[index..];
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_bindings_have_no_same_platform_collision() {
        for macos in [false, true] {
            let mut seen = Vec::new();
            for row in REGISTRY {
                let Row::Binding(binding) = row else { continue };
                if !macos && binding.id == "close-chat" {
                    continue;
                }
                for (modifiers, key) in binding.keys {
                    let physical = (
                        if macos {
                            modifiers.ctrl
                        } else {
                            modifiers.ctrl || modifiers.command
                        },
                        macos && (modifiers.command || modifiers.mac_cmd),
                        modifiers.alt,
                        modifiers.shift,
                        *key,
                    );
                    assert!(
                        !seen.iter().any(|(found, _)| *found == physical),
                        "duplicate binding {} on macOS={macos}",
                        binding.id
                    );
                    seen.push((physical, binding.id));
                }
            }
        }
    }

    #[test]
    fn linked_reply_help_uses_the_actual_shortcut() {
        let shortcut = super::super::merged::latest_reply_shortcut();
        assert_eq!(
            shortcut,
            egui::KeyboardShortcut::new(
                Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
                Key::R
            )
        );
        assert!(help_rows().any(|(keys, _)| keys == "Ctrl+Alt+Shift+R"));
    }

    #[test]
    fn the_help_rows_stay_exactly_as_written() {
        let expected: Vec<(&str, &str)> = vec![
            ("Ctrl+K", "Jump between recent chats"),
            ("Ctrl+Shift+F", "Search all messages and chats"),
            ("Ctrl+P", "Command palette: chats, filters, actions"),
            ("Ctrl+F", "Search within this chat"),
            (
                "Ctrl+R",
                "Quote-reply to the latest incoming message (on its source in linked chats)",
            ),
            ("Ctrl+L", "Forward selected messages, or the latest message"),
            (
                "\u{2191} / \u{2193}, Enter",
                "Walk the open chat's search results and jump to one",
            ),
            (
                "PageUp / PageDown",
                "Scroll the open chat a screen at a time",
            ),
            (
                "Home / End",
                "Jump to the oldest loaded / newest message (with the input empty)",
            ),
            ("Ctrl+End", "Jump to the newest message"),
            ("Alt+\u{2191} / Alt+\u{2193}", "Previous / next chat"),
            ("Ctrl+Shift+[ / ]", "Previous / next chat, as in WhatsApp"),
            (TAB_STEP_KEYS, "Next / previous chat, as in Safari's tabs"),
            ("Ctrl+1..0", "Open a favorite by its place in the sidebar"),
            (
                "Alt+1..0",
                "Filter to a network by its place in the sidebar (outside text fields)",
            ),
            (
                "Ctrl+Alt+1..9",
                "All, Unread, Reply later, Read later, Favorites, Private, Groups, Channels; 9 toggles Unreplied",
            ),
            (
                "Ctrl+Alt+Shift+R",
                "Reply to the latest matching message in linked-person search",
            ),
            ("Ctrl+Shift+1", "Chats"),
            (
                "\u{2191}",
                "Edit the previous message (when the input is empty)",
            ),
            ("Enter", "Send (Shift+Enter for a new line)"),
            (
                "Escape",
                "Dismiss the current action, return from search or Starred, or close the chat",
            ),
            ("Ctrl+N", "New chat or message yourself"),
            ("Ctrl+E", "Archive or unarchive the open chat"),
            ("Ctrl+U", "Mark the open chat to read later"),
            ("Ctrl+I", "Mark the open chat to reply later"),
            (
                "Ctrl+Alt+\u{2192} / Ctrl+Alt+\u{2190}",
                "Switch to the next / previous linked number",
            ),
            (
                "Ctrl+V",
                "Paste text, or stage a picture from the clipboard",
            ),
            ("Ctrl+C", "Copy the selected messages"),
            ("Ctrl+B", "Collapse or expand the chat list"),
            ("Ctrl+Alt+I", "Show or hide the info sidebar"),
            (
                "Ctrl+End",
                "Jump to the newest message (when the input is empty)",
            ),
            #[cfg(target_os = "macos")]
            (MESSAGES_KEYS, "Go to the message list"),
            (
                "\u{2191} / \u{2193}, Home / End, PgUp / PgDn",
                "Move through messages (in the message list)",
            ),
            ("PgUp / PgDn", "Scroll the open chat by a page"),
            (
                "Home / End",
                "Top / bottom of the open chat (when the input is empty)",
            ),
            ("Ctrl+,", "Settings"),
            (
                "\u{2190} / \u{2192}",
                "Previous / next item in the media viewer",
            ),
            ("? / Ctrl+/", "Keyboard shortcuts (? when not typing)"),
            ("Ctrl+Alt+Shift+L", "Lock the app, when a PIN is set"),
            ("Ctrl+W", "Close the window (Solmu remains in the tray)"),
            ("Ctrl+Q", "Quit"),
        ];
        assert_eq!(help_rows().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn mac_labels_follow_the_modifier_convention() {
        for (written, mac) in [
            ("Ctrl+K", "⌘K"),
            ("Ctrl+Shift+F", "⇧⌘F"),
            ("Shift+Ctrl+Enter", "⇧⌘Enter"),
            ("Ctrl+Alt+Shift+1", "⌥⇧⌘1"),
            ("Control+2", "⌃2"),
            ("Alt+\u{2191} / Alt+\u{2193}", "⌥↑ / ⌥↓"),
            ("Ctrl+1..9", "⌘1..9"),
            ("Ctrl++ / Ctrl+-", "⌘+ / ⌘-"),
            ("Ctrl+Tab / Control+Shift+Tab", "⌘Tab / ⌃⇧Tab"),
            ("Enter", "Enter"),
            ("PgUp / PgDn", "PgUp / PgDn"),
            ("? / Ctrl+/", "? / ⌘/"),
        ] {
            assert_eq!(label_for(written, true), mac, "{written}");
            assert_eq!(label_for(written, false), written, "{written}");
        }
    }

    #[test]
    fn mac_labels_relabel_shortcuts_inside_sentences() {
        assert_eq!(
            label_for("Show the chat list (Ctrl+B)", true),
            "Show the chat list (⌘B)"
        );
        assert_eq!(
            label_for(
                "Ctrl+K for recent chats · Ctrl+Shift+F to search · ? for keyboard shortcuts",
                true
            ),
            "⌘K for recent chats · ⇧⌘F to search · ? for keyboard shortcuts"
        );
        // A mouse gesture keeps its space: "⌘ click" is a click, not a key.
        assert_eq!(
            label_for("Ctrl + click · Select a message", true),
            "⌘ click · Select a message"
        );
        // Words that merely contain a modifier's letters stay untouched.
        assert_eq!(
            label_for("Alternative layouts", true),
            "Alternative layouts"
        );
        // German Control follows Command, as before.
        assert_eq!(label_for("Strg+F", true), "⌘F");
    }

    #[test]
    fn control_tab_steps_chats_like_alt_arrows() {
        let (_root, mut app, ids) = app_with_chats(3);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        // Physical Control: forward, and back with Shift.
        assert!(!press(&mut app, &ctx, Key::Tab, Modifiers::CTRL));
        assert!(matches!(
            app.actions.as_slice(),
            [.., Action::OpenChat(chat)] if *chat == ids[1]
        ));
        app.open_chat = Some(ids[1].clone());
        app.actions.clear();
        assert!(!press(
            &mut app,
            &ctx,
            Key::Tab,
            Modifiers::CTRL | Modifiers::SHIFT
        ));
        assert!(matches!(
            app.actions.as_slice(),
            [.., Action::OpenChat(chat)] if *chat == ids[0]
        ));
        // Command+Tab is the platform's own switcher, never this binding,
        // and plain Tab stays with focus traversal.
        for modifiers in [Modifiers::COMMAND, Modifiers::NONE] {
            app.actions.clear();
            press(&mut app, &ctx, Key::Tab, modifiers);
            assert!(
                !app.actions
                    .iter()
                    .any(|action| matches!(action, Action::OpenChat(_))),
                "{modifiers:?} moved chats"
            );
        }
    }

    #[test]
    fn every_menu_accelerator_matches_a_binding() {
        for row in REGISTRY {
            let Row::Binding(entry) = row else { continue };
            let Some(menu) = &entry.menu else { continue };
            if entry.keys.is_empty() {
                // Native edit items reach egui as clipboard events instead.
                continue;
            }
            assert!(
                entry.keys.iter().any(|(modifiers, key)| {
                    menu_accelerator(*modifiers, *key).as_deref() == Some(menu.accelerator)
                }),
                "{}: the menu accelerator matches no binding",
                entry.id
            );
        }
    }

    #[test]
    fn shortcut_ids_are_unique() {
        let mut ids: Vec<&str> = REGISTRY
            .iter()
            .filter_map(|row| match row {
                Row::Binding(entry) => Some(entry.id),
                Row::Help(_, _) => None,
            })
            .collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate shortcut ids");
    }

    #[test]
    fn search_shortcuts_have_distinct_keys_and_menu_scopes() {
        assert_eq!(FIND.keys, &[(Modifiers::COMMAND, Key::F)]);
        assert_eq!(SEARCH_CHATS.keys, &[(COMMAND_SHIFT, Key::F)]);
        assert_eq!(JUMP_TO_CHAT.keys, &[(Modifiers::COMMAND, Key::K)]);
        assert_eq!(FIND.menu.as_ref().unwrap().accelerator, "Super+KeyF");
        assert_eq!(
            SEARCH_CHATS.menu.as_ref().unwrap().accelerator,
            "Super+Shift+KeyF"
        );
        assert_ne!(
            FIND.menu.as_ref().unwrap().id,
            SEARCH_CHATS.menu.as_ref().unwrap().id
        );
    }

    #[test]
    fn ctrl_f_never_changes_search_scope_with_the_page() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        assert!(matches!(find_action(&app), Action::OpenChatSearch));
        app.open_chat = Some("1@s.whatsapp.net".into());
        app.page = Page::Chats;
        assert!(matches!(find_action(&app), Action::OpenChatSearch));
        app.page = Page::Settings;
        assert!(matches!(find_action(&app), Action::OpenChatSearch));
        app.page = Page::Wallpaper;
        assert!(matches!(find_action(&app), Action::OpenChatSearch));
    }

    #[test]
    fn escape_clears_the_settings_search_before_leaving_settings() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Settings;
        app.settings_search = "sound".into();
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::SearchSettings(text)] if text.is_empty())
        );
        app.actions.clear();
        app.settings_search.clear();
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::Open(Page::Chats)]
        ));
    }

    /// Copy is for a picture. The header leaves the action out while a clip is
    /// on screen, and the shortcut follows the same rule rather than decoding a
    /// video as an image and failing.
    #[test]
    fn ctrl_c_does_not_copy_a_clip_from_the_viewer() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ctx = egui::Context::default();
        let copy = |app: &mut App, ctx: &egui::Context| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: Key::C,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::CTRL,
                    }],
                    ..Default::default()
                },
                |ui| handle(app, ui.ctx()),
            );
            output.textures_delta.clear();
        };
        let showing = |path: &str, message: &str| {
            Some(crate::image_preview::PreviewState::new(
                Some(std::path::PathBuf::from(path)),
                "fixture".into(),
                message.into(),
            ))
        };
        app.image_preview = showing("/cache/photo.png", "m1");
        copy(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CopyImage(_)]));
        app.actions.clear();
        app.image_preview = showing("/cache/clip.mp4", "m2");
        copy(&mut app, &ctx);
        assert!(app.actions.is_empty(), "a clip is played, not copied");
    }

    fn escape(app: &mut App, ctx: &egui::Context) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| handle(app, ui.ctx()),
        );
        output.textures_delta.clear();
    }

    #[test]
    fn escape_folds_the_day_filter_then_the_search_pane_then_the_chat() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.chat_search_open = true;
        app.chat_search_calendar = true;
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(!app.chat_search_calendar);
        assert!(app.actions.is_empty(), "the pane stays for now");
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseChatSearch]));
        app.actions.clear();
        // Settings hide the pane, so Escape leaves them instead.
        app.page = Page::Settings;
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::Open(Page::Chats)]
        ));
    }

    #[test]
    fn escape_returns_from_the_media_view_then_closes_the_info_panel() {
        use crate::model::{InfoPanel, InfoView, MediaTab};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.info = Some(InfoPanel {
            chat: "fixture".into(),
            view: InfoView::Media(MediaTab::Links),
        });
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::ShowInfo(InfoView::Overview)]
        ));
        app.actions.clear();
        app.info.as_mut().unwrap().view = InfoView::Overview;
        // A group rename in the panel is cancelled before the panel closes.
        app.group_name_edit = Some("Rust".into());
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseGroupName]));
        app.actions.clear();
        app.group_name_edit = None;
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseInfo]));
    }

    /// What the composer is in the middle of goes first: a reply, an edit,
    /// staged files and suggestions are cancelled before the panel closes,
    /// and a focused composer with nothing to cancel still closes the panel
    /// rather than the chat.
    #[test]
    fn escape_cancels_composer_state_before_closing_the_info_panel() {
        use crate::model::{InfoPanel, InfoView};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.info = Some(InfoPanel {
            chat: "fixture".into(),
            view: InfoView::Overview,
        });
        let ctx = egui::Context::default();
        app.reply_to = Some("reply".into());
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CancelReply]));
        assert!(app.info.is_some(), "the panel stays");
        app.actions.clear();
        app.reply_to = None;
        app.emoji_start = Some(0);
        escape(&mut app, &ctx);
        assert!(matches!(
            app.actions.as_slice(),
            [Action::CloseEmojiSuggestions]
        ));
        app.actions.clear();
        app.emoji_start = None;
        app.stage(crate::app::Pending::File("unsent.png".into()));
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::ClearPending]));
        app.actions.clear();
        app.pending.clear();
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("composer-text")));
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::CloseInfo]),
            "the panel, not the chat: {:?}",
            app.actions
        );
    }

    #[test]
    fn escape_cancels_a_contact_rename_and_keeps_the_info_panel() {
        use crate::model::{InfoPanel, InfoView};
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        app.info = Some(InfoPanel {
            chat: "fixture".into(),
            view: InfoView::Overview,
        });
        app.contact_edit = Some(("Ada".into(), String::new()));
        let ctx = egui::Context::default();
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseContactEdit]));
    }

    #[test]
    fn escape_returns_from_search_and_reply_before_closing_the_chat() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.page = Page::Chats;
        app.open_chat = Some("fixture".into());
        let ctx = egui::Context::default();
        app.reply_to = Some("reply".into());
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CancelReply]));
        app.actions.clear();
        app.reply_to = None;
        app.search = "Ada".into();
        escape(&mut app, &ctx);
        assert!(
            matches!(app.actions.as_slice(), [Action::Search(text), Action::FocusComposer] if text.is_empty())
        );
        app.actions.clear();
        app.search.clear();
        escape(&mut app, &ctx);
        assert!(matches!(app.actions.as_slice(), [Action::CloseChat]));
    }

    #[test]
    fn escape_closes_the_update_before_touching_an_unfinished_message() {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        app.show_update = true;
        app.page = Page::Settings;
        app.reply_to = Some("reply-fixture".into());
        app.stage(crate::app::Pending::File("unsent.png".into()));
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| handle(&mut app, ui.ctx()),
        );
        output.textures_delta.clear();
        assert!(matches!(app.actions.as_slice(), [Action::CloseUpdate]));
    }

    /// Marks every chat a favorites-bar favorite in creation order.
    fn favorite_all(app: &mut App) {
        for (index, chat) in app.chats.iter_mut().enumerate() {
            chat.favorite = true;
            chat.favorite_position = index as u32;
            chat.backend_account = Some("wa-personal".into());
        }
    }

    fn app_with_chats(count: usize) -> (tempfile::TempDir, App, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let mut app = App::headless(
            crate::paths::AppDirs::under(root.path()),
            crate::settings::Settings::default(),
        )
        .0;
        let ids: Vec<String> = (0..count)
            .map(|index| format!("49170000{index:04}@s.whatsapp.net"))
            .collect();
        for (index, id) in ids.iter().enumerate() {
            let mut chat = Chat::new(id.clone(), format!("Chat {index:02}"));
            chat.last_activity = 100 - index as i64;
            app.chats.push(chat);
        }
        app.page = Page::Chats;
        (root, app, ids)
    }

    /// Runs `handle` on one key press and reports whether the press was left
    /// for the views, as every press that is not a shortcut must be.
    fn press(app: &mut App, ctx: &egui::Context, key: Key, modifiers: Modifiers) -> bool {
        let mut survived = false;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ui| {
                handle(app, ui.ctx());
                survived = ui.ctx().input(|input| {
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key {
                                key: found,
                                pressed: true,
                                ..
                            } if *found == key
                        )
                    })
                });
            },
        );
        output.textures_delta.clear();
        survived
    }

    fn shortcut_message(
        chat: &str,
        id: &str,
        incoming: bool,
        timestamp: i64,
    ) -> crate::model::Message {
        crate::model::Message {
            id: id.into(),
            chat: chat.into(),
            sender: "synthetic-sender".into(),
            sender_name: None,
            from_me: !incoming,
            timestamp,
            history_order: None,
            content: crate::model::Content::text(id),
            status: crate::model::Delivery::Sent,
            delivered_at: None,
            read_at: None,
            quoted: None,
            reactions: vec![],
            edited: false,
            mentions: vec![],
            forwarded: false,
            thumbnail: None,
        }
    }

    #[test]
    fn command_r_quotes_latest_incoming_in_ordinary_chat_despite_later_outgoing() {
        let (_root, mut app, ids) = app_with_chats(1);
        let chat = ids[0].clone();
        app.open_chat = Some(chat.clone());
        app.composer = "preserve draft".into();
        let mut revoked = shortcut_message(&chat, "revoked", true, 40);
        revoked.content = crate::model::Content::Revoked;
        let mut call = shortcut_message(&chat, "call-entry", true, 50);
        call.content = crate::model::Content::Call {
            video: false,
            status: crate::model::CallStatus::Missed,
            duration: 0,
            group: false,
        };
        app.conversations.insert(
            chat.clone(),
            crate::app::Conversation {
                messages: vec![
                    shortcut_message(&chat, "older", true, 10),
                    shortcut_message(&chat, "latest-incoming", true, 20),
                    shortcut_message(&chat, "outgoing-quote", false, 30),
                    revoked,
                    call,
                ],
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        assert!(!press(&mut app, &ctx, Key::R, Modifiers::COMMAND));
        assert_eq!(app.actions, [Action::Reply("latest-incoming".into())]);
        app.actions.clear();
        assert_eq!(app.composer, "preserve draft");
        app.selection = Some((chat.clone(), vec!["older".into()]));
        assert!(!press(&mut app, &ctx, Key::R, Modifiers::COMMAND));
        assert_eq!(
            app.actions,
            [
                Action::CancelSelection,
                Action::Reply("latest-incoming".into())
            ]
        );
        app.actions.clear();
        assert!(press(
            &mut app,
            &ctx,
            Key::R,
            Modifiers::COMMAND | Modifiers::ALT
        ));
        assert!(app.actions.is_empty());
    }

    #[test]
    fn ordinary_reply_remains_owned_when_saved_link_cannot_render() {
        let (_root, mut app, ids) = app_with_chats(1);
        let chat = ids[0].clone();
        app.chats[0].backend_origin = Some("origin".into());
        app.chats[0].backend_account = Some("account".into());
        let member = crate::merged::Member::of(&app.chats[0]).unwrap();
        app.settings.beeper.people.push(crate::merged::Person {
            name: "Person".into(),
            members: vec![member],
            primary: None,
        });
        app.open_chat = Some(chat.clone());
        app.conversations.insert(
            chat.clone(),
            crate::app::Conversation {
                messages: vec![shortcut_message(&chat, "incoming", true, 10)],
                ..Default::default()
            },
        );
        assert!(
            latest_incoming_action(&app).is_none(),
            "linked view owns its unavailable state"
        );
        let duplicate = app.chats[0].clone();
        app.chats.push(duplicate);
        assert_eq!(
            latest_incoming_action(&app),
            Some(Action::Reply("incoming".into()))
        );
        app.chats.pop();
        app.chats[0].kind = crate::model::ChatKind::Group;
        assert_eq!(
            latest_incoming_action(&app),
            Some(Action::Reply("incoming".into()))
        );
    }

    #[test]
    fn command_l_opens_existing_forward_dialog_for_selection_or_latest_without_sending() {
        let (_root, mut app, ids) = app_with_chats(1);
        let chat = ids[0].clone();
        app.open_chat = Some(chat.clone());
        app.conversations.insert(
            chat.clone(),
            crate::app::Conversation {
                messages: vec![
                    shortcut_message(&chat, "first", true, 10),
                    shortcut_message(&chat, "last", false, 20),
                ],
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        assert!(!press(&mut app, &ctx, Key::L, Modifiers::COMMAND));
        assert_eq!(
            app.actions,
            [Action::ShowDialog(Dialog::Forward {
                chat: chat.clone(),
                messages: vec!["last".into()]
            })]
        );
        app.actions.clear();
        app.selection = Some((chat.clone(), vec!["first".into(), "last".into()]));
        assert!(!press(&mut app, &ctx, Key::L, Modifiers::COMMAND));
        assert_eq!(
            app.actions,
            [Action::ShowDialog(Dialog::Forward {
                chat: chat.clone(),
                messages: vec!["first".into(), "last".into()]
            })]
        );
        app.actions.clear();
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(&mut app, &ctx, Key::L, Modifiers::COMMAND));
        assert!(app.actions.is_empty());
    }

    /// The modifiers a real Ctrl+Shift press carries on Linux and Windows,
    /// and a real Cmd+Shift press on macOS.
    fn ctrl_shift() -> [Modifiers; 3] {
        [
            COMMAND_SHIFT,
            Modifiers {
                ctrl: true,
                command: true,
                shift: true,
                ..Modifiers::NONE
            },
            Modifiers {
                mac_cmd: true,
                command: true,
                shift: true,
                ..Modifiers::NONE
            },
        ]
    }

    #[test]
    fn command_alt_i_opens_info_and_command_d_toggles_groups() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        // The shared keymap moves the info sidebar to Cmd+Alt+I and reserves
        // Cmd+D for the groups toggle, so Cmd+D no longer opens info.
        assert!(!press(&mut app, &ctx, Key::D, Modifiers::COMMAND));
        assert_eq!(app.actions, [Action::ToggleFlatGroups]);
        app.actions.clear();
        // The extra Alt must not leak the press into plain Cmd+I (Reply
        // later): the info binding is consumed before that one.
        assert!(!press(
            &mut app,
            &ctx,
            Key::I,
            Modifiers::COMMAND | Modifiers::ALT
        ));
        assert!(matches!(
            app.actions.as_slice(),
            [Action::ToggleInfoSidebar]
        ));
        assert!(
            app.sidebar_visible,
            "Cmd+Alt+I does not toggle the left list"
        );
        assert_eq!(
            INFO_SIDEBAR.keys,
            &[(Modifiers::COMMAND | Modifiers::ALT, Key::I)]
        );
        assert_eq!(REPLY_LATER.keys, &[(Modifiers::COMMAND, Key::I)]);
        assert_eq!(SIDEBAR.keys, &[(Modifiers::COMMAND, Key::B)]);
        assert!(REGISTRY.iter().any(|row| matches!(
            row,
            Row::Binding(entry)
                if entry.id == "flat-groups" && entry.keys.contains(&(Modifiers::COMMAND, Key::D))
        )));
        assert_eq!(
            menu_accelerator(Modifiers::COMMAND, Key::D).as_deref(),
            Some("Super+KeyD")
        );
        assert_eq!(
            menu_accelerator(Modifiers::COMMAND | Modifiers::ALT, Key::I).as_deref(),
            Some("Super+Alt+KeyI")
        );
    }

    #[test]
    fn bracket_shortcuts_step_to_the_previous_and_next_chat() {
        let (_root, mut app, ids) = app_with_chats(3);
        app.open_chat = Some(ids[1].clone());
        let ctx = egui::Context::default();
        // With Shift held, US-style layouts report the brackets as braces.
        for modifiers in ctrl_shift() {
            for (key, expected) in [
                (Key::CloseBracket, &ids[2]),
                (Key::CloseCurlyBracket, &ids[2]),
                (Key::OpenBracket, &ids[0]),
                (Key::OpenCurlyBracket, &ids[0]),
            ] {
                app.actions.clear();
                app.scroll_chat_into_view = None;
                let survived = press(&mut app, &ctx, key, modifiers);
                assert!(!survived, "{key:?} with {modifiers:?} reached the views");
                assert!(
                    app.actions.contains(&Action::OpenChat(expected.clone())),
                    "{key:?} with {modifiers:?} opens {expected}: {:?}",
                    app.actions
                );
                assert_eq!(app.scroll_chat_into_view.as_ref(), Some(expected));
            }
        }
    }

    #[test]
    fn only_control_alone_counts_as_the_macos_messages_key() {
        assert!(control_alone(Modifiers::CTRL));
        // Command+Control+2 stays Command+2's chat.
        assert!(!control_alone(Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        }));
        assert!(!control_alone(Modifiers {
            ctrl: true,
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        }));
        assert!(!control_alone(Modifiers::CTRL | Modifiers::SHIFT));
        assert!(!control_alone(Modifiers::NONE));
    }

    #[test]
    fn numbered_shortcuts_open_each_position_with_platform_command_modifiers() {
        let (_root, mut app, ids) = app_with_chats(9);
        favorite_all(&mut app);
        let ctx = egui::Context::default();
        for modifiers in ctrl_shift().map(|modifiers| Modifiers {
            shift: false,
            ..modifiers
        }) {
            for (key, expected) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .into_iter()
            .zip(&ids)
            {
                app.actions.clear();
                app.scroll_chat_into_view = None;
                assert!(!press(&mut app, &ctx, key, modifiers));
                assert!(app.actions.contains(&Action::OpenChat(expected.clone())));
                assert_eq!(app.scroll_chat_into_view.as_ref(), Some(expected));
            }
        }
    }

    #[test]
    fn numbered_shortcuts_follow_the_favorites_bar_not_the_list() {
        let (_root, mut app, ids) = app_with_chats(4);
        favorite_all(&mut app);
        let ctx = egui::Context::default();
        // Archived and locked favorites leave the bar; a pinned one stays,
        // a favorite among favorites, and keeps its digit.
        app.chats[3].pinned = true;
        app.chats[2].archived = true;
        app.chats[1].locked = true;
        assert!(!press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[0].clone())));
        app.actions.clear();
        assert!(!press(&mut app, &ctx, Key::Num2, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[3].clone())));

        app.chat_filter = crate::model::ChatFilter::Unread;
        app.chats[0].unread = 1;
        app.actions.clear();
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert!(app.actions.contains(&Action::OpenChat(ids[0].clone())));
        assert!(app.actions.contains(&Action::KeepUnread(ids[0].clone())));

        app.search = "Chat 00".into();
        app.search_selected = Some(ids[0].clone());
        app.actions.clear();
        press(&mut app, &ctx, Key::Num9, Modifiers::COMMAND);
        assert!(app.actions.is_empty());
        assert_eq!(app.search_selected.as_ref(), Some(&ids[0]));
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert_eq!(app.actions, [Action::OpenChat(ids[0].clone())]);
        assert!(app.search_selected.is_none());

        app.search.clear();
        app.show_archived = true;
        app.actions.clear();
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert_eq!(app.actions, [Action::OpenChat(ids[0].clone())]);
    }

    #[test]
    fn numbered_shortcuts_do_not_wrap_missing_positions() {
        let (_root, mut app, ids) = app_with_chats(2);
        favorite_all(&mut app);
        let ctx = egui::Context::default();
        app.open_chat = Some(ids[0].clone());
        for key in [Key::Num3, Key::Num9] {
            assert!(!press(&mut app, &ctx, key, Modifiers::COMMAND));
            assert!(app.actions.is_empty());
            assert!(app.scroll_chat_into_view.is_none());
        }
        app.chats.clear();
        press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND);
        assert!(app.actions.is_empty());
        // Command+0 is the favorites bar's tenth slot, empty with two.
        press(&mut app, &ctx, Key::Num0, Modifiers::COMMAND);
        assert!(app.actions.is_empty());
    }

    #[test]
    fn numbered_shortcuts_leave_plain_and_modified_numbers_alone() {
        let (_root, mut app, _ids) = app_with_chats(2);
        let ctx = egui::Context::default();
        for modifiers in [
            Modifiers::NONE,
            Modifiers::SHIFT,
            Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
        ] {
            assert!(press(&mut app, &ctx, Key::Num1, modifiers));
            assert!(app.actions.is_empty());
        }
    }

    /// Command+Option+1..8 picks the sidebar's nth filter in the chips'
    /// order, 9 toggles Unreplied over the list, and 0 stays free.
    #[test]
    fn filter_digits_follow_the_visible_queue_before_facets() {
        assert_eq!(
            crate::model::ChatFilter::EVERY,
            [
                crate::model::ChatFilter::All,
                crate::model::ChatFilter::Unread,
                crate::model::ChatFilter::ReplyLater,
                crate::model::ChatFilter::ReadLater,
                crate::model::ChatFilter::Favorites,
                crate::model::ChatFilter::Private,
                crate::model::ChatFilter::Groups,
                crate::model::ChatFilter::Channels,
            ]
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn chats_destination_is_not_shadowed_by_numbered_network_navigation() {
        let (_root, mut app, _ids) = app_with_chats(2);
        app.link = crate::backend::LinkStatus::Connected;
        app.show_archived = true;
        for chat in &mut app.chats {
            chat.backend_network = Some("whatsapp".into());
        }
        let ctx = egui::Context::default();
        assert!(!press(&mut app, &ctx, Key::Num1, COMMAND_SHIFT));
        assert!(app.actions.contains(&Action::ShowArchived(false)));
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::OpenChat(_)))
        );
    }

    #[test]
    fn command_option_numbered_shortcuts_pick_sidebar_filters() {
        let (_root, mut app, _ids) = app_with_chats(2);
        let ctx = egui::Context::default();
        use crate::model::ChatFilter;
        let modifiers = Modifiers::COMMAND | Modifiers::ALT;
        assert!(!press(&mut app, &ctx, Key::Num1, modifiers));
        assert!(
            app.actions
                .contains(&Action::SetChatFilter(ChatFilter::All))
        );
        app.actions.clear();
        assert!(!press(&mut app, &ctx, Key::Num3, modifiers));
        assert!(
            app.actions
                .contains(&Action::SetChatFilter(ChatFilter::ReplyLater))
        );
        app.actions.clear();
        assert!(!press(&mut app, &ctx, Key::Num5, modifiers));
        assert_eq!(
            app.actions,
            vec![Action::SetChatFilter(ChatFilter::Favorites)]
        );
        app.actions.clear();
        // 9 is the layered Unreplied toggle, as the rail's icon does it.
        assert!(!press(&mut app, &ctx, Key::Num9, modifiers));
        assert!(app.beeper_unreplied);
        assert!(app.actions.is_empty(), "Unreplied layers over the queue");
        app.actions.clear();
        app.beeper_unreplied = false;
        // 0 is free: nothing takes it.
        assert!(press(&mut app, &ctx, Key::Num0, modifiers));
        assert!(app.actions.is_empty());
    }

    /// Option+digit picks a network, and a focused text field keeps it:
    /// layouts type characters with it, such as @ or brackets, that must
    /// reach the field.
    #[test]
    fn option_numbered_shortcuts_pick_networks_outside_text_fields() {
        let (_root, mut app, _ids) = app_with_chats(2);
        for chat in &mut app.chats {
            chat.backend_network = Some("whatsapp".into());
        }
        let ctx = egui::Context::default();
        assert!(!press(&mut app, &ctx, Key::Num1, Modifiers::ALT));
        assert_eq!(app.beeper_network_filter.as_deref(), Some("whatsapp"));
        app.beeper_network_filter = None;
        let field = egui::Id::new("composer-text");
        egui::widgets::text_edit::TextEditState::default().store(&ctx, field);
        ctx.memory_mut(|memory| memory.request_focus(field));
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::ALT));
        assert!(app.beeper_network_filter.is_none());
    }

    /// With Beeper networks present, Command+digit still opens the
    /// favorites bar; networks live on Option+digit, so the bar's promise
    /// holds in every account setup.
    #[test]
    fn beeper_shortcuts_keep_command_digit_on_the_favorites_bar() {
        let (_root, mut app, ids) = app_with_chats(3);
        favorite_all(&mut app);
        for chat in &mut app.chats {
            chat.backend_network = Some("whatsapp".into());
        }
        let ctx = egui::Context::default();
        // Command+1 opens the bar's first favorite, not the first network.
        assert!(!press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[0].clone())));
        assert!(app.beeper_network_filter.is_none());
        app.actions.clear();
        // Option+1 picks the first network.
        assert!(!press(&mut app, &ctx, Key::Num1, Modifiers::ALT));
        assert_eq!(app.beeper_network_filter.as_deref(), Some("whatsapp"));
        app.beeper_network_filter = None;
        // Command+Alt+Shift+digit no longer opens pins: they are favorites
        // now, on plain Command+digit.
        app.chats[1].pinned = true;
        press(
            &mut app,
            &ctx,
            Key::Num1,
            Modifiers::COMMAND | Modifiers::ALT | Modifiers::SHIFT,
        );
        assert!(!app.actions.contains(&Action::OpenChat(ids[1].clone())));
    }

    /// Ctrl+Shift+1 (Cmd on macOS) leads to the rail's Chats by where the
    /// digit sits, whatever symbol the layout puts on it with Shift held;
    /// Ctrl+1 still opens chats.
    #[test]
    fn shifted_digits_lead_to_the_chats() {
        let (_root, mut app, ids) = app_with_chats(2);
        app.link = crate::backend::LinkStatus::Connected;
        // Chats is not a toggle: from a Chats page the digit changes
        // nothing, so the check begins away from it.
        app.page = Page::Settings;
        let ctx = egui::Context::default();
        let shifted = |app: &mut App, key: Key, physical: Key| {
            app.actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: Some(physical),
                        pressed: true,
                        repeat: false,
                        modifiers: COMMAND_SHIFT,
                    }],
                    ..Default::default()
                },
                |ui| handle(app, ui.ctx()),
            );
            output.textures_delta.clear();
            std::mem::take(&mut app.actions)
        };
        // US layouts report Shift+1 as "!".
        assert_eq!(
            shifted(&mut app, Key::Exclamationmark, Key::Num1),
            [Action::Open(Page::Chats)]
        );
        app.page = Page::Chats;
        app.show_starred = true;
        assert_eq!(
            shifted(&mut app, Key::Exclamationmark, Key::Num1),
            [Action::ToggleStarred]
        );
        app.show_starred = false;
        // Command+digit jumps to the favorites bar's nth chat, not the
        // list's; with all chats favorited in list order, Command+2 keeps
        // opening the second one.
        favorite_all(&mut app);
        assert!(!press(&mut app, &ctx, Key::Num2, Modifiers::COMMAND));
        assert!(app.actions.contains(&Action::OpenChat(ids[1].clone())));
    }

    #[test]
    fn numbered_shortcuts_leave_settings_and_overlays_alone() {
        let (_root, mut app, _ids) = app_with_chats(2);
        let ctx = egui::Context::default();
        app.page = Page::Settings;
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        app.page = Page::Chats;
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        app.dialog = None;
        app.show_update = true;
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        app.show_update = false;
        app.reaction_target = Some(("chat-fixture".into(), "message-fixture".into()));
        assert!(press(&mut app, &ctx, Key::Num1, Modifiers::COMMAND));
        assert!(app.actions.is_empty());
    }

    /// One number on screen and a second linked beside it.
    fn app_with_two_numbers() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        let dirs = crate::paths::AppDirs::under(root.path());
        let mut app = App::headless(dirs.clone(), crate::settings::Settings::default()).0;
        let second = crate::account::Account::detached(
            &dirs,
            crate::model::AccountId::parse("2").unwrap(),
            crate::settings::AccountSettings::default(),
        )
        .unwrap()
        .0;
        app.accounts.push(second);
        app.active = 0;
        app.page = Page::Chats;
        (root, app)
    }

    #[test]
    fn command_option_arrows_step_through_linked_numbers_and_wrap() {
        let (_root, mut app) = app_with_two_numbers();
        let ids: Vec<_> = app
            .accounts
            .iter()
            .map(|account| account.id.clone())
            .collect();
        let ctx = egui::Context::default();
        let modifiers = Modifiers::COMMAND | Modifiers::ALT;
        // With two numbers both arrows wrap to the other one.
        for (active, expected) in [(0, &ids[1]), (1, &ids[0])] {
            for key in [Key::ArrowRight, Key::ArrowLeft] {
                app.active = active;
                app.actions.clear();
                assert!(!press(&mut app, &ctx, key, modifiers));
                assert_eq!(app.actions, [Action::SwitchAccount(expected.clone())]);
            }
        }
    }

    #[test]
    fn command_option_arrows_need_a_second_number_that_is_not_being_added() {
        let (_root, mut single, _ids) = app_with_chats(1);
        let ctx = egui::Context::default();
        let modifiers = Modifiers::COMMAND | Modifiers::ALT;
        assert!(press(&mut single, &ctx, Key::ArrowRight, modifiers));
        assert!(
            single.actions.is_empty(),
            "one number has nothing to switch to"
        );
        let (_root, mut app) = app_with_two_numbers();
        app.adding_account = true;
        assert!(press(&mut app, &ctx, Key::ArrowRight, modifiers));
        assert!(
            app.actions.is_empty(),
            "a number being added keeps its Cancel"
        );
    }

    #[test]
    fn command_option_arrows_leave_dialogs_and_other_combinations_alone() {
        let (_root, mut app) = app_with_two_numbers();
        let ctx = egui::Context::default();
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(
            &mut app,
            &ctx,
            Key::ArrowRight,
            Modifiers::COMMAND | Modifiers::ALT
        ));
        app.dialog = None;
        for modifiers in [Modifiers::NONE, Modifiers::SHIFT, Modifiers::COMMAND] {
            assert!(press(&mut app, &ctx, Key::ArrowRight, modifiers));
        }
        assert!(
            !app.actions
                .iter()
                .any(|action| matches!(action, Action::SwitchAccount(_))),
            "{:?}",
            app.actions
        );
    }

    #[test]
    fn ctrl_e_archives_and_unarchives_the_open_chat() {
        let (_root, mut app, ids) = app_with_chats(2);
        app.open_chat = Some(ids[1].clone());
        let ctx = egui::Context::default();
        for modifiers in ctrl_shift().map(|modifiers| Modifiers {
            shift: false,
            ..modifiers
        }) {
            app.chats[1].archived = false;
            app.actions.clear();
            assert!(!press(&mut app, &ctx, Key::E, modifiers));
            assert_eq!(app.actions, [Action::SetArchived(ids[1].clone(), true)]);
            app.chats[1].archived = true;
            app.actions.clear();
            assert!(!press(&mut app, &ctx, Key::E, modifiers));
            assert_eq!(app.actions, [Action::SetArchived(ids[1].clone(), false)]);
        }
    }

    #[test]
    fn ctrl_e_needs_an_open_chat_and_no_overlay() {
        let (_root, mut app, ids) = app_with_chats(1);
        let ctx = egui::Context::default();
        assert!(press(&mut app, &ctx, Key::E, Modifiers::COMMAND));
        assert!(app.actions.is_empty(), "no chat is open");
        app.open_chat = Some(ids[0].clone());
        app.dialog = Some(Dialog::Shortcuts);
        assert!(press(&mut app, &ctx, Key::E, Modifiers::COMMAND));
        app.dialog = None;
        app.page = Page::Settings;
        assert!(press(&mut app, &ctx, Key::E, Modifiers::COMMAND));
        app.page = Page::Chats;
        assert!(press(&mut app, &ctx, Key::E, Modifiers::NONE));
        assert!(app.actions.is_empty());
    }

    #[test]
    fn a_bracket_without_ctrl_and_shift_together_stays_typed() {
        let (_root, mut app, ids) = app_with_chats(3);
        app.open_chat = Some(ids[1].clone());
        let ctx = egui::Context::default();
        let typed = [
            Modifiers::NONE,
            Modifiers::SHIFT,
            Modifiers::COMMAND,
            Modifiers {
                ctrl: true,
                command: true,
                ..Modifiers::NONE
            },
            Modifiers {
                mac_cmd: true,
                command: true,
                ..Modifiers::NONE
            },
            Modifiers::ALT,
            Modifiers::ALT | Modifiers::SHIFT,
        ];
        for modifiers in typed {
            for key in [
                Key::OpenBracket,
                Key::OpenCurlyBracket,
                Key::CloseBracket,
                Key::CloseCurlyBracket,
            ] {
                app.actions.clear();
                let survived = press(&mut app, &ctx, key, modifiers);
                assert!(survived, "{key:?} with {modifiers:?} was consumed");
                assert!(
                    !app.actions
                        .iter()
                        .any(|action| matches!(action, Action::OpenChat(_))),
                    "{key:?} with {modifiers:?} switched chats"
                );
            }
        }
    }

    #[test]
    fn page_keys_scroll_the_open_chat_and_are_consumed() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        for (key, expected) in [
            (Key::PageUp, Action::ScrollPage(Scroll::PageUp)),
            (Key::PageDown, Action::ScrollPage(Scroll::PageDown)),
            (Key::Home, Action::ScrollPage(Scroll::Top)),
            (Key::End, Action::ScrollPage(Scroll::Bottom)),
        ] {
            app.actions.clear();
            let survived = press(&mut app, &ctx, key, Modifiers::NONE);
            assert!(!survived, "{key:?} reached the views");
            assert_eq!(
                app.actions,
                [expected],
                "{key:?} did not push the expected action"
            );
        }
    }

    /// The focused message list takes the arrows, Home, End and the page
    /// keys for its cursor; unfocused, the page keys still scroll.
    #[test]
    fn a_focused_message_list_takes_the_navigation_keys() {
        use crate::ui::list_nav::Step;
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let list = crate::ui::conversation::messages_list_id(&ids[0]);
        for (key, expected) in [
            (Key::ArrowUp, Step::Previous),
            (Key::ArrowDown, Step::Next),
            (Key::Home, Step::First),
            (Key::End, Step::Last),
            (Key::PageUp, Step::PageUp(8)),
            (Key::PageDown, Step::PageDown(8)),
        ] {
            app.actions.clear();
            ctx.memory_mut(|memory| memory.request_focus(list));
            let survived = press(&mut app, &ctx, key, Modifiers::NONE);
            assert!(!survived, "{key:?} reached the views");
            assert_eq!(app.actions, [Action::MessageCursor(expected)], "{key:?}");
        }
        // Shift+Home and the like are not list keys.
        app.actions.clear();
        ctx.memory_mut(|memory| memory.request_focus(list));
        assert!(press(&mut app, &ctx, Key::Home, Modifiers::SHIFT));
        assert!(app.actions.is_empty(), "{:?}", app.actions);
        // Unfocused, PgUp scrolls as before and the arrows are left alone.
        ctx.memory_mut(|memory| memory.surrender_focus(list));
        app.actions.clear();
        assert!(!press(&mut app, &ctx, Key::PageUp, Modifiers::NONE));
        assert_eq!(app.actions, [Action::ScrollPage(Scroll::PageUp)]);
        app.actions.clear();
        assert!(press(&mut app, &ctx, Key::ArrowDown, Modifiers::NONE));
        assert!(app.actions.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_messages_shortcut_focuses_the_list() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let modifiers = if cfg!(target_os = "macos") {
            Modifiers::CTRL
        } else {
            Modifiers::ALT
        };
        assert!(!press(&mut app, &ctx, Key::Num2, modifiers));
        assert_eq!(app.actions, [Action::FocusMessages]);
    }

    /// Ctrl+Alt+2 picks the second sidebar filter, and keeps working with
    /// a field focused: the combo types no characters to steal.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn the_filter_digits_pick_filters_even_while_typing() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let modifiers = Modifiers::COMMAND | Modifiers::ALT;
        assert!(!press(&mut app, &ctx, Key::Num2, modifiers));
        assert_eq!(
            app.actions,
            [Action::SetChatFilter(crate::model::ChatFilter::Unread)]
        );
        app.actions.clear();
        let field = egui::Id::new("composer-text");
        egui::widgets::text_edit::TextEditState::default().store(&ctx, field);
        ctx.memory_mut(|memory| memory.request_focus(field));
        assert!(!press(&mut app, &ctx, Key::Num2, modifiers));
        assert_eq!(
            app.actions,
            [Action::SetChatFilter(crate::model::ChatFilter::Unread)]
        );
    }

    /// The list's shortcut and the rail's Ctrl+Shift+1 (Cmd on macOS) never
    /// take each other's keys, and Shift does not turn one into the other.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_messages_shortcut_and_the_rail_keys_stay_apart() {
        let (_root, mut app, ids) = app_with_chats(2);
        app.link = crate::backend::LinkStatus::Connected;
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        let messages = if cfg!(target_os = "macos") {
            Modifiers::CTRL
        } else {
            Modifiers::ALT
        };
        press(&mut app, &ctx, Key::Num2, messages);
        assert_eq!(app.actions, [Action::FocusMessages]);
        app.actions.clear();
        press(&mut app, &ctx, Key::Num2, messages | Modifiers::SHIFT);
        assert!(
            !app.actions.contains(&Action::FocusMessages),
            "{:?}",
            app.actions
        );
    }

    #[test]
    fn page_keys_leave_shift_and_ctrl_combinations_alone() {
        let (_root, mut app, ids) = app_with_chats(1);
        app.open_chat = Some(ids[0].clone());
        let ctx = egui::Context::default();
        for (key, modifiers) in [
            (Key::Home, Modifiers::SHIFT),
            (Key::PageUp, Modifiers::SHIFT),
            (Key::Home, Modifiers::COMMAND),
        ] {
            app.actions.clear();
            let survived = press(&mut app, &ctx, key, modifiers);
            assert!(survived, "{key:?} with {modifiers:?} was consumed");
            assert!(app.actions.is_empty(), "{key:?} with {modifiers:?} acted");
        }
        // Ctrl+End (Cmd+End on macOS) still jumps to the newest message.
        app.actions.clear();
        let survived = press(&mut app, &ctx, Key::End, Modifiers::COMMAND);
        assert!(!survived);
        assert_eq!(app.actions, [Action::ScrollToBottom]);
    }

    #[test]
    fn ctrl_end_and_home_end_leave_a_field_with_text_alone() {
        // No field focused, or an empty composer: the keys scroll the chat.
        assert!(jump_keys_reach_chat(false, false, false));
        assert!(jump_keys_reach_chat(true, true, true));
        // A composer with text, or any other field (search, dialogs): the
        // keys move the text cursor (#456).
        assert!(!jump_keys_reach_chat(true, true, false));
        assert!(!jump_keys_reach_chat(true, false, true));
        assert!(!jump_keys_reach_chat(true, false, false));
    }

    #[test]
    fn page_keys_do_nothing_without_an_open_chat_a_dialog_or_off_the_chats_page() {
        let (_root, mut app, ids) = app_with_chats(1);
        let ctx = egui::Context::default();
        let keys = [Key::PageUp, Key::PageDown, Key::Home, Key::End];
        // No chat open.
        for key in keys {
            app.actions.clear();
            assert!(
                press(&mut app, &ctx, key, Modifiers::NONE),
                "{key:?} survived with no chat open"
            );
            assert!(app.actions.is_empty());
        }
        app.open_chat = Some(ids[0].clone());
        // A dialog is open.
        app.dialog = Some(Dialog::Shortcuts);
        for key in keys {
            app.actions.clear();
            assert!(press(&mut app, &ctx, key, Modifiers::NONE));
            assert!(app.actions.is_empty());
        }
        app.dialog = None;
        // Off the Chats page.
        app.page = Page::Settings;
        for key in keys {
            app.actions.clear();
            assert!(press(&mut app, &ctx, key, Modifiers::NONE));
            assert!(app.actions.is_empty());
        }
    }
}
