//! Main-thread AppKit application menus. The menu outlives windows, just like
//! the link and tray; reopening replaces only its repaint callback. The
//! traffic lights are placed by fastframe-titlebar. The Messages theme's
//! translucent sidebar is AppKit's own sidebar material, behind the window.

use std::cell::RefCell;
use std::sync::{Arc, LazyLock, Mutex};

use objc2_app_kit::{NSApplication, NSText};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem as Native, Submenu};

use crate::model::{Action, Dialog, Page};

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}
static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Dock/activation diagnostics: one short line per event, in
/// `<profile>/dock-diagnostic.log`, only when `SOLMU_PROFILE_ROOT` is set.
/// Event names and window counts only, never chat, message or token content.
/// The file is cut back once it passes 32 KiB.
fn dock_diag(line: &str) {
    use std::io::Write;
    let Some(root) = ["SOLMU_PROFILE_ROOT", "ZAPFAST_PROFILE_ROOT"]
        .iter()
        .find_map(|var| std::env::var_os(var))
    else {
        return;
    };
    let path = std::path::Path::new(&root).join("dock-diagnostic.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 32 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{} client {line}", crate::util::now());
    }
}

/// The app's windows as counts: all, visible, miniaturized.
fn window_counts(app: &NSApplication) -> String {
    let windows = app.windows();
    let visible = windows.iter().filter(|w| w.isVisible()).count();
    let mini = windows.iter().filter(|w| w.isMiniaturized()).count();
    format!(
        "windows={} visible={visible} mini={mini} active={}",
        windows.len(),
        app.isActive()
    )
}
static EDIT_EVENTS: LazyLock<Arc<Mutex<Vec<egui::Event>>>> =
    LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
static REPAINT: Mutex<Option<egui::Context>> = Mutex::new(None);

/// Menu edits must reach the input before egui processes focus and selection.
struct MenuInput(Arc<Mutex<Vec<egui::Event>>>);

impl egui::plugin::Plugin for MenuInput {
    fn debug_name(&self) -> &'static str {
        "solmu-macos-menu"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        input
            .events
            .extend(self.0.lock().unwrap_or_else(|p| p.into_inner()).drain(..));
    }
}

fn item(id: &str, text: &str, shortcut: Option<&str>) -> MenuItem {
    MenuItem::with_id(
        id,
        text,
        true,
        shortcut.map(|key| key.parse().expect("menu shortcut")),
    )
}

/// The menu item the shortcut registry carries for `id`: same binding the
/// keyboard handler consumes, so the two cannot drift.
fn registered(id: &str) -> MenuItem {
    let entry = crate::ui::keys::shortcut(id)
        .menu
        .as_ref()
        .unwrap_or_else(|| panic!("shortcut {id} has no menu item"));
    item(entry.id, entry.text, Some(entry.accelerator))
}

fn build_menu() -> tray_icon::menu::Result<Menu> {
    let menu = Menu::new();
    let app = Submenu::new("Solmu", true);
    app.append_items(&[
        &item("about", "About Solmu", None),
        &Native::separator(),
        &registered("settings"),
        &Native::separator(),
        &Native::services(None),
        &Native::separator(),
        &Native::hide(Some("Hide Solmu")),
        &Native::hide_others(None),
        &Native::show_all(None),
        &Native::separator(),
        &registered("quit"),
    ])?;
    let file = Submenu::new("File", true);
    file.append_items(&[
        &registered("new-chat"),
        &Native::separator(),
        &registered("close-window"),
    ])?;
    let edit = Submenu::new("Edit", true);
    // Winit's view is not an NSTextView: AppKit's copy:/undo: selectors
    // cannot edit egui text. Send the same events as its keyboard shortcuts.
    edit.append_items(&[
        &registered("undo"),
        &registered("redo"),
        &Native::separator(),
        &registered("cut"),
        &registered("copy"),
        &registered("paste"),
        &registered("select-all"),
        &Native::separator(),
        &registered("find"),
    ])?;
    let view = Submenu::new("View", true);
    view.append_items(&[
        &registered("sidebar"),
        &registered("info-sidebar"),
        &Native::separator(),
        &Native::fullscreen(None),
    ])?;
    let window = Submenu::new("Window", true);
    window.append_items(&[
        &Native::minimize(None),
        &Native::maximize(Some("Zoom")),
        &Native::separator(),
        &item("show-window", "Show Solmu", None),
    ])?;
    let help = Submenu::new("Help", true);
    help.append_items(&[
        &item("whats-new", "What's New", None),
        &registered("shortcuts"),
        &item("help", "Solmu Help", None),
    ])?;
    menu.append_items(&[&app, &file, &edit, &view, &window, &help])?;
    window.set_as_windows_menu_for_nsapp();
    help.set_as_help_menu_for_nsapp();
    Ok(menu)
}

pub fn attach(ctx: &egui::Context) {
    ctx.add_plugin(MenuInput(Arc::clone(&EDIT_EVENTS)));
    // Layout tests use headless contexts on test threads, without an NSApp.
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    // The packaged executable is an agent launcher, so its LSUIElement applies
    // to this child too. The GUI alone owns the Dock and Cmd-Tab entry.
    fastframe_dock::set_dock_icon_visible(true);
    *REPAINT.lock().unwrap_or_else(|p| p.into_inner()) = Some(ctx.clone());
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        // The tray's menu shares muda's one handler; its ids are its own.
        if fastframe_tray::claim_menu_event(&event.id.0) {
            return;
        }
        if native_edit(&event.id.0) {
            return;
        }
        if let Some(edit) = edit_event(&event.id.0) {
            EDIT_EVENTS
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(edit);
        } else {
            EVENTS
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(event.id.0);
        }
        if let Some(ctx) = &*REPAINT.lock().unwrap_or_else(|p| p.into_inner()) {
            ctx.request_repaint();
        }
    }));
    MENU.with_borrow_mut(|menu| {
        if menu.is_none() {
            match build_menu() {
                Ok(created) => *menu = Some(created),
                Err(error) => log::warn!("could not create the application menu: {error}"),
            }
        }
        if let Some(menu) = menu {
            menu.init_for_nsapp();
        }
    });
    // Demo windows have no tray to activate the app. This also brings a newly
    // recreated window forward after a menu command while running headless.
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

/// `applicationShouldHandleReopen:hasVisibleWindows:` injected onto the
/// application delegate's class: winit deliberately registers no handler for
/// it (its platform docs assign the decision to the app), and Solmu's
/// window is destroyed when it closes to the tray, so reopening the app
/// from the Dock or Finder with no window left surfaced none. AppKit does
/// not send this message for Cmd-Tab or app-switcher activation; that path
/// is covered by `did_become_active` below. The injected method restores
/// miniaturized windows - the behavior the absent default handler
/// provided - and otherwise asks for a window through the same action the
/// tray's Show uses. The delegate instance stays winit's; only the
/// missing method is added to its class.
unsafe extern "C-unwind" fn reopen(
    _this: &objc2::runtime::AnyObject,
    _sel: objc2::runtime::Sel,
    _sender: &NSApplication,
    has_visible_windows: objc2::runtime::Bool,
) -> objc2::runtime::Bool {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return objc2::runtime::Bool::YES;
    };
    let app = NSApplication::sharedApplication(mtm);
    dock_diag(&format!(
        "reopen has_visible={} {}",
        has_visible_windows.as_bool(),
        window_counts(&app)
    ));
    // Bring the app's windows forward on every Dock reopen, visible or not:
    // the default behavior cannot be relied on when the process was started
    // by a launcher rather than by LaunchServices itself.
    #[allow(deprecated, reason = "its replacement needs macOS 14")]
    app.activateIgnoringOtherApps(true);
    // activateIgnoringOtherApps has no effect on macOS 14+ (activation is
    // cooperative), so also order the app's own windows front directly.
    for window in app.windows().iter() {
        if window.isVisible() {
            window.orderFrontRegardless();
        }
    }
    if has_visible_windows.as_bool() {
        return objc2::runtime::Bool::YES;
    }
    // Miniaturized windows are not visible; restore them like the default
    // handler would. Anything else means the window is gone.
    let mut restored = false;
    for window in app.windows().iter() {
        if window.isMiniaturized() {
            window.deminiaturize(None);
            restored = true;
        }
    }
    if !restored {
        EVENTS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push("show-window".into());
        if let Some(ctx) = &*REPAINT.lock().unwrap_or_else(|p| p.into_inner()) {
            ctx.request_repaint();
        }
    }
    objc2::runtime::Bool::YES
}

/// Cmd-Tab activates without sending a Dock/Finder reopen event. Always
/// forward activation to the app: AppKit's window list also contains the
/// status item's window, which can outlive the closed chat window.
/// The shell knows whether its resident window exists. Native miniaturized
/// windows are restored here by default, with an explicit opt-out available.
unsafe extern "C-unwind" fn did_become_active(
    _this: &objc2::runtime::AnyObject,
    _sel: objc2::runtime::Sel,
    _notification: &objc2::runtime::AnyObject,
) {
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    dock_diag(&format!("did_become_active {}", window_counts(&app)));
    // Native restoration is on after the owner's minimized Cmd-Tab acceptance.
    if direct_restore_enabled(
        std::env::var("SOLMU_EXPERIMENTAL_RESTORE_MINIMIZED")
            .ok()
            .as_deref(),
    ) {
        for window in app.windows().iter() {
            if window.isMiniaturized() {
                window.deminiaturize(None);
                window.orderFrontRegardless();
            }
        }
    }
    EVENTS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push("activate-window".into());
    if let Some(ctx) = &*REPAINT.lock().unwrap_or_else(|p| p.into_inner()) {
        ctx.request_repaint();
    }
}

/// Restore by default; keep the existing variable as an explicit opt-out.
fn direct_restore_enabled(value: Option<&str>) -> bool {
    value != Some("0")
}

/// Adds `reopen` to the application delegate's class once winit has set its
/// delegate, unless a handler already exists. Called from `drain`, which
/// runs while the event loop pumps frames.
fn install_reopen_handler() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return;
    };
    let class = AsRef::<objc2::runtime::AnyObject>::as_ref(&*delegate).class();
    if install_handlers_on_class(class) {
        // winit assigned this delegate before we added the optional method.
        // A method added later is not automatically an observer of AppKit's
        // already-configured delegate notifications. Enroll it explicitly.
        register_activation_handler(
            &objc2_foundation::NSNotificationCenter::defaultCenter(),
            AsRef::<objc2::runtime::AnyObject>::as_ref(&*delegate),
            AsRef::<objc2::runtime::AnyObject>::as_ref(&app),
        );
        dock_diag("activation notification observer registered");
        INSTALLED.store(true, Ordering::Relaxed);
    }
}

/// Subscribe the installed delegate to real activation delivery, not merely
/// to the presence of a selector. Re-registration is idempotent for this
/// observer and app, without touching unrelated notifications or delegates.
fn register_activation_handler(
    center: &objc2_foundation::NSNotificationCenter,
    observer: &objc2::runtime::AnyObject,
    app: &objc2::runtime::AnyObject,
) {
    // SAFETY: install_handlers_on_class has installed the notification
    // selector with its protocol's signature. winit owns the delegate for
    // the lifetime of its event loop, including headless AppKit pumping.
    unsafe {
        let name = objc2_app_kit::NSApplicationDidBecomeActiveNotification;
        center.removeObserver_name_object(observer, Some(name), Some(app));
        center.addObserver_selector_name_object(
            observer,
            objc2::sel!(applicationDidBecomeActive:),
            Some(name),
            Some(app),
        );
    }
}

/// Independently add the two optional handlers, preserving any existing ones.
fn install_handlers_on_class(class: &objc2::runtime::AnyClass) -> bool {
    install_handlers_with_protocol(
        class,
        objc2::runtime::AnyProtocol::get(c"NSApplicationDelegate"),
    )
}

/// The optional protocol is explicit so the missing-metadata path is testable.
fn install_handlers_with_protocol(
    class: &objc2::runtime::AnyClass,
    protocol: Option<&objc2::runtime::AnyProtocol>,
) -> bool {
    use objc2::Encode;
    use objc2::runtime::{Bool, Imp, Sel};
    use std::ffi::CString;

    let sel = objc2::sel!(applicationShouldHandleReopen:hasVisibleWindows:);
    // Never override a handler the delegate already implements.
    if class.instance_method(sel).is_none() {
        let protocol_types = protocol.and_then(|p| {
            let desc = unsafe {
                objc2::ffi::protocol_getMethodDescription(
                    std::ptr::from_ref(p),
                    sel,
                    Bool::new(false),
                    Bool::new(true),
                )
            };
            if desc.types.is_null() {
                None
            } else {
                Some(desc.types)
            }
        });
        let fallback = CString::new(format!("{}@:@{}", Bool::ENCODING, Bool::ENCODING)).ok();
        let types = protocol_types.unwrap_or_else(|| {
            fallback
                .as_ref()
                .map(|s| s.as_ptr())
                .unwrap_or(c"c@:@c".as_ptr())
        });
        let added = unsafe {
            objc2::ffi::class_addMethod(
                std::ptr::from_ref(class).cast_mut(),
                sel,
                std::mem::transmute::<
                    unsafe extern "C-unwind" fn(
                        &objc2::runtime::AnyObject,
                        Sel,
                        &NSApplication,
                        Bool,
                    ) -> Bool,
                    Imp,
                >(reopen),
                types,
            )
        };
        if !added.as_bool() && class.instance_method(sel).is_none() {
            return false;
        }
    }
    // Install independently: fastframe-tray can already own the Dock reopen
    // selector, but that says nothing about Cmd-Tab activation. Never let its
    // existing handler prevent this separate selector from being installed.
    // Same injection for Cmd-Tab / app-switcher activation.
    let active_sel = objc2::sel!(applicationDidBecomeActive:);
    if class.instance_method(active_sel).is_some() {
        return true;
    }
    let protocol_active_types = protocol.and_then(|p| {
        let desc = unsafe {
            objc2::ffi::protocol_getMethodDescription(
                std::ptr::from_ref(p),
                active_sel,
                Bool::new(false),
                Bool::new(true),
            )
        };
        if desc.types.is_null() {
            None
        } else {
            Some(desc.types)
        }
    });
    let active_types = protocol_active_types.unwrap_or(c"v@:@".as_ptr());
    let active_added = unsafe {
        objc2::ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            active_sel,
            std::mem::transmute::<
                unsafe extern "C-unwind" fn(
                    &objc2::runtime::AnyObject,
                    Sel,
                    &objc2::runtime::AnyObject,
                ),
                Imp,
            >(did_become_active),
            active_types,
        )
    };
    active_added.as_bool() || class.instance_method(active_sel).is_some()
}

/// A native file dialog runs a modal loop, so its text fields must receive
/// editing commands immediately instead of leaving them queued for egui.
fn native_edit(id: &str) -> bool {
    use objc2::sel;
    let selector = match id {
        "copy" => sel!(copy:),
        "cut" => sel!(cut:),
        "paste" => sel!(paste:),
        "undo" => sel!(undo:),
        "redo" => sel!(redo:),
        "select-all" => sel!(selectAll:),
        _ => return false,
    };
    let Some(main) = objc2::MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(main);
    let Some(responder) = app.keyWindow().and_then(|window| window.firstResponder()) else {
        return false;
    };
    // Only native text editors (including a file dialog's field editor) can
    // handle these selectors. GL views and AccessKit's dynamically subclassed
    // WinitView must use egui, regardless of their Objective-C class name.
    if responder.downcast_ref::<NSText>().is_none() {
        return false;
    }
    // SAFETY: standard AppKit editing selectors; nil target walks the native
    // responder chain, and these actions accept a nil sender.
    unsafe { app.sendAction_to_from(selector, None, None) }
}

fn edit_event(id: &str) -> Option<egui::Event> {
    match id {
        "copy" => return Some(egui::Event::Copy),
        "cut" => return Some(egui::Event::Cut),
        "paste" => {
            return Some(egui::Event::Paste(
                arboard::Clipboard::new()
                    .ok()
                    .and_then(|mut clipboard| clipboard.get_text().ok())
                    .unwrap_or_default(),
            ));
        }
        _ => {}
    }
    let (key, shift) = match id {
        "undo" => (egui::Key::Z, false),
        "redo" => (egui::Key::Z, true),
        "select-all" => (egui::Key::A, false),
        _ => return None,
    };
    Some(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            command: true,
            mac_cmd: true,
            shift,
            ..Default::default()
        },
    })
}

fn action(id: &str) -> Option<Action> {
    Some(match id {
        "about" => Action::ShowDialog(Dialog::About),
        "settings" => Action::Open(Page::Settings),
        "new" => Action::ShowDialog(Dialog::NewChat),
        "close" => Action::CloseWindow,
        "quit" => Action::Quit,
        "search" => Action::FocusSearch,
        "sidebar" => Action::ToggleSidebar,
        "info-sidebar" => Action::ToggleInfoSidebar,
        "whats-new" => Action::ShowDialog(Dialog::WhatsNew),
        "shortcuts" => Action::ShowDialog(Dialog::Shortcuts),
        "help" => Action::OpenUrl("https://zapfast.rocks/using-zapfast/".into()),
        "show-window" => Action::ShowWindow,
        "activate-window" => Action::ActivateWindow,
        _ => return None,
    })
}

pub fn drain(hidden: bool) -> Vec<Action> {
    install_reopen_handler();
    if hidden {
        EDIT_EVENTS
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clear();
    }
    let events = std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|p| p.into_inner()));
    let mut actions = Vec::new();
    for id in events {
        if let Some(action) = action(&id) {
            if hidden
                && matches!(
                    action,
                    Action::ShowDialog(_) | Action::Open(_) | Action::FocusSearch
                )
            {
                actions.push(Action::ShowWindow);
            }
            actions.push(action);
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_minimized_restoration_is_on_unless_explicitly_disabled() {
        for value in [None, Some(""), Some("1"), Some("false"), Some("true")] {
            assert!(direct_restore_enabled(value));
        }
        assert!(!direct_restore_enabled(Some("0")));
    }

    #[test]
    fn missing_delegate_protocol_still_installs_both_handlers() {
        use objc2::ClassType;
        use objc2::runtime::ClassBuilder;
        let builder = ClassBuilder::new(
            c"SolmuMissingDelegateProtocolTest",
            objc2_foundation::NSObject::class(),
        )
        .expect("unique test class");
        let class = builder.register();
        let reopen = objc2::sel!(applicationShouldHandleReopen:hasVisibleWindows:);
        let active = objc2::sel!(applicationDidBecomeActive:);
        assert!(class.instance_method(reopen).is_none());
        assert!(class.instance_method(active).is_none());
        assert!(install_handlers_with_protocol(class, None));
        assert!(class.instance_method(reopen).is_some());
        assert!(class.instance_method(active).is_some());
        let reopen_imp = class.instance_method(reopen).unwrap().implementation() as *const ();
        let active_imp = class.instance_method(active).unwrap().implementation() as *const ();
        assert!(
            install_handlers_with_protocol(class, None),
            "repeat is safe"
        );
        assert_eq!(
            class.instance_method(reopen).unwrap().implementation() as *const (),
            reopen_imp
        );
        assert_eq!(
            class.instance_method(active).unwrap().implementation() as *const (),
            active_imp
        );
    }

    #[test]
    fn activation_registration_delivers_once_after_late_method_installation() {
        use objc2::ClassType;
        use objc2::rc::Retained;
        use objc2::runtime::{AnyObject, ClassBuilder, Sel};
        use objc2_foundation::{NSNotificationCenter, NSObject};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static DELIVERED: AtomicUsize = AtomicUsize::new(0);
        extern "C-unwind" fn active(_this: &AnyObject, _sel: Sel, _note: &AnyObject) {
            DELIVERED.fetch_add(1, Ordering::Relaxed);
        }
        let selector = objc2::sel!(applicationDidBecomeActive:);
        let mut builder =
            ClassBuilder::new(c"SolmuActivationDeliveryTest", NSObject::class()).unwrap();
        // SAFETY: the method accepts the one notification argument.
        unsafe {
            builder.add_method(selector, active as extern "C-unwind" fn(_, _, _));
        }
        let class = builder.register();
        // SAFETY: NSObject's initializer on our freshly registered subclass.
        let observer: Retained<AnyObject> = unsafe { objc2::msg_send![class, new] };
        let app = NSObject::new();
        let other_app = NSObject::new();
        let center = NSNotificationCenter::new();
        register_activation_handler(&center, &observer, &app);
        register_activation_handler(&center, &observer, &app);
        // SAFETY: both senders are live NSObject instances. The callback
        // does not need NSApplication or a main-thread UI in this test.
        unsafe {
            center.postNotificationName_object(
                objc2_app_kit::NSApplicationDidBecomeActiveNotification,
                Some(&other_app),
            );
            assert_eq!(DELIVERED.load(Ordering::Relaxed), 0);
            center.postNotificationName_object(
                objc2_app_kit::NSApplicationDidBecomeActiveNotification,
                Some(&app),
            );
            assert_eq!(DELIVERED.load(Ordering::Relaxed), 1);
            center.removeObserver(&observer);
            center.postNotificationName_object(
                objc2_app_kit::NSApplicationDidBecomeActiveNotification,
                Some(&app),
            );
            assert_eq!(DELIVERED.load(Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn an_existing_dock_handler_does_not_block_cmd_tab() {
        use objc2::ClassType;
        use objc2::runtime::{AnyObject, Bool, ClassBuilder, Sel};
        extern "C-unwind" fn dock(
            _delegate: &AnyObject,
            _selector: Sel,
            _app: *mut NSApplication,
            _visible: Bool,
        ) -> Bool {
            Bool::YES
        }
        let reopen = objc2::sel!(applicationShouldHandleReopen:hasVisibleWindows:);
        let active = objc2::sel!(applicationDidBecomeActive:);
        let mut builder = ClassBuilder::new(
            c"SolmuExistingDockHandlerTest",
            objc2_foundation::NSObject::class(),
        )
        .expect("unique test class");
        // SAFETY: dock matches the selector's Objective-C signature.
        unsafe {
            builder.add_method(reopen, dock as extern "C-unwind" fn(_, _, _, _) -> _);
        }
        let class = builder.register();
        assert!(class.instance_method(active).is_none());
        assert!(install_handlers_on_class(class));
        assert!(class.instance_method(active).is_some());
        assert_eq!(
            class.instance_method(reopen).unwrap().implementation() as *const (),
            dock as *const ()
        );
        assert!(
            install_handlers_on_class(class),
            "later windows keep both handlers"
        );
    }

    #[test]
    fn menu_edits_reach_the_focused_text_field_before_the_pass() {
        let ctx = egui::Context::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        ctx.add_plugin(MenuInput(Arc::clone(&events)));
        let id = egui::Id::new("menu-edit-fixture");
        let mut text = String::from("draft");
        let mut frame = |event: Option<egui::Event>| {
            events.lock().unwrap().extend(event);
            ctx.memory_mut(|memory| memory.request_focus(id));
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(id));
            });
            output.textures_delta.clear();
            output
        };
        let _ = frame(None);
        let _ = frame(edit_event("select-all"));
        let output = frame(edit_event("cut"));
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(value) if value == "draft")
        }));
        let _ = frame(Some(egui::Event::Paste("replacement".into())));
        let _ = frame(edit_event("select-all"));
        let output = frame(edit_event("copy"));
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(value) if value == "replacement")
        }));
        assert_eq!(text, "replacement");
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn menu_uses_the_apps_close_quit_and_edit_paths() {
        assert!(matches!(action("close"), Some(Action::CloseWindow)));
        assert!(matches!(action("quit"), Some(Action::Quit)));
        assert!(matches!(action("show-window"), Some(Action::ShowWindow)));
        assert!(matches!(
            action("activate-window"),
            Some(Action::ActivateWindow)
        ));
        assert!(matches!(edit_event("copy"), Some(egui::Event::Copy)));
        assert!(
            matches!(edit_event("redo"), Some(egui::Event::Key { key: egui::Key::Z, modifiers, .. }) if modifiers.command && modifiers.shift)
        );
        for shortcut in [
            "Super+KeyN",
            "Super+Comma",
            "Super+Equal",
            "Super+Digit0",
            "Super+Shift+KeyZ",
        ] {
            shortcut
                .parse::<tray_icon::menu::accelerator::Accelerator>()
                .unwrap();
        }
    }
}

thread_local! {
    /// The vibrancy behind the current window, while the theme asks for it,
    /// and whether it is drawn dark.
    static VIBRANCY: RefCell<Option<(objc2::rc::Retained<objc2_app_kit::NSVisualEffectView>, bool)>> =
        const { RefCell::new(None) };
}

/// Puts AppKit's sidebar material behind the window's content while `on`,
/// light or dark as `palette` is, or takes it away. The window is
/// transparent; whatever egui leaves translucent, the sidebar in the
/// Messages theme, shows the blurred desktop as Messages does. Cheap to call
/// every frame: it changes the view only when the palette does or the
/// window was made anew.
pub fn vibrancy(frame: &eframe::Frame, palette: &crate::theme::Palette, on: bool) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: eframe supplies a live NSView, and this runs on its main thread.
    let content = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
    // SAFETY: as above; the content view's parent is the window's frame view.
    let Some(frame_view) = (unsafe { content.superview() }) else {
        return;
    };
    VIBRANCY.with_borrow_mut(|slot| {
        // A closed window took its view with it.
        let current = slot.as_ref().is_some_and(|(view, _)| {
            // SAFETY: the view is ours and alive; read on the main thread.
            unsafe { view.superview() }.is_some_and(|parent| parent == frame_view)
        });
        if !current {
            *slot = None;
        }
        match (on, slot.is_some()) {
            (true, false) => {
                let view = NSVisualEffectView::initWithFrame(mtm.alloc(), frame_view.bounds());
                view.setMaterial(NSVisualEffectMaterial::Sidebar);
                view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
                view.setState(NSVisualEffectState::FollowsWindowActiveState);
                view.setAutoresizingMask(
                    NSAutoresizingMaskOptions::ViewWidthSizable
                        | NSAutoresizingMaskOptions::ViewHeightSizable,
                );
                frame_view.addSubview_positioned_relativeTo(
                    &view,
                    NSWindowOrderingMode::Below,
                    Some(content),
                );
                // Flipped below to the palette's appearance.
                *slot = Some((view, !palette.dark));
            }
            (false, true) => {
                if let Some((view, _)) = slot.take() {
                    view.removeFromSuperview();
                }
            }
            _ => {}
        }
        // The material follows the palette rather than the system, which a
        // cached choice can lag behind for a frame.
        if let Some((view, dark)) = slot.as_mut()
            && *dark != palette.dark
        {
            // SAFETY: AppKit's constants, read on the main thread.
            let name = unsafe {
                if palette.dark {
                    NSAppearanceNameDarkAqua
                } else {
                    NSAppearanceNameAqua
                }
            };
            view.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
            *dark = palette.dark;
        }
    });
}
