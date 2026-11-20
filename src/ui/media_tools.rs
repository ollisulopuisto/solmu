//! Shared Preferences and first-sign-in media-tool check.
use crate::{app::App, i18n::gettext, media_tool_setup::Request};

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    if app.media_tool_setup.ffmpeg.is_none() && !app.media_tool_setup.busy() {
        app.check_media_tools(Request::Recheck);
    }
    ui.heading(gettext(locale, "Local media tools"));
    ui.label(gettext(locale,"ffmpeg enables voice and video transcription and some media playback. No installation or PATH change is made."));
    for (name, tool) in [
        ("ffmpeg", &app.media_tool_setup.ffmpeg),
        ("ffprobe", &app.media_tool_setup.ffprobe),
    ] {
        ui.strong(name);
        match tool {
            Some(tool) => {
                if tool.error.is_none() {
                    ui.label(gettext(locale, "Available · Executable check passed"));
                } else {
                    ui.label(gettext(
                        locale,
                        "Unavailable · Check the path or choose another file",
                    ));
                }
                if let Some(path) = &tool.path {
                    ui.add(egui::Label::new(path.display().to_string()).wrap());
                }
                if let Some(error) = &tool.error {
                    ui.push_id(name, |ui| {
                        ui.collapsing(gettext(locale, "Technical details"), |ui| {
                            ui.add(egui::Label::new(error).wrap());
                        });
                    });
                }
            }
            None => {
                ui.label(gettext(locale, "Not checked yet"));
            }
        }
    }
    if app.settings.ffmpeg_path.is_some() {
        ui.label(gettext(locale, "Using your selected ffmpeg"));
    } else {
        ui.label(gettext(locale, "Detected automatically"));
    }
    if let Some(error) = &app.media_tool_setup.error {
        ui.add(egui::Label::new(error).wrap());
    }
    let busy = app.media_tool_setup.busy();
    ui.add_enabled_ui(!busy, |ui| {
        ui.horizontal_wrapped(|ui| {
            if ui.button(gettext(locale, "Choose file…")).clicked() {
                app.check_media_tools(Request::Choose);
            }
            if ui.button(gettext(locale, "Recheck")).clicked() {
                app.check_media_tools(Request::Recheck);
            }
            if app.settings.ffmpeg_path.is_some()
                && ui
                    .button(gettext(locale, "Use automatic detection"))
                    .clicked()
            {
                app.check_media_tools(Request::Automatic);
            }
        });
    });
    if busy {
        ui.spinner();
        ui.label(gettext(locale, "Checking media tools…"));
    }
}
