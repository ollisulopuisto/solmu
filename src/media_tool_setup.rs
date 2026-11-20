//! Media-tool checks and native file selection stay off the drawing thread.
use std::{path::PathBuf, sync::mpsc};

#[derive(Clone, Debug)]
pub struct Tool {
    pub path: Option<PathBuf>,
    pub error: Option<String>,
}
#[derive(Default)]
pub struct Setup {
    pub ffmpeg: Option<Tool>,
    pub ffprobe: Option<Tool>,
    pending: Option<mpsc::Receiver<Checked>>,
    pub error: Option<String>,
}
struct Checked {
    ffmpeg: Tool,
    ffprobe: Tool,
    choice: Option<Option<PathBuf>>,
    error: Option<String>,
}
#[derive(Clone, Copy)]
pub enum Request {
    Recheck,
    Choose,
    Automatic,
}
impl Setup {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn start(
        &mut self,
        request: Request,
        waker: crate::backend::Waker,
        locale: crate::i18n::Locale,
    ) {
        if self.busy() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.error = None;
        let selected = crate::media_tools::configured();
        if let Err(error) = std::thread::Builder::new()
            .name("solmu-media-check".into())
            .spawn(move || {
                let result = check(request, selected, locale);
                let _ = tx.send(result);
                waker.wake();
            })
        {
            self.pending = None;
            self.error = Some(error.to_string());
        }
    }
    /// Returns only a checked replacement; failures never overwrite preferences.
    pub fn poll(&mut self) -> Option<Option<PathBuf>> {
        let Some(rx) = &self.pending else { return None };
        match rx.try_recv() {
            Ok(result) => {
                self.pending = None;
                self.ffmpeg = Some(result.ffmpeg);
                self.ffprobe = Some(result.ffprobe);
                self.error = result.error;
                result.choice
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.error = Some("Media-tool check stopped unexpectedly".into());
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}
fn inspect(path: Result<PathBuf, String>, name: &str) -> Tool {
    match path {
        Ok(path) => {
            let error = crate::media_tools::verify(&path, name).err();
            Tool {
                path: Some(path),
                error,
            }
        }
        Err(error) => Tool {
            path: None,
            error: Some(error),
        },
    }
}
fn check(request: Request, selected: Option<PathBuf>, locale: crate::i18n::Locale) -> Checked {
    let mut desired = selected.clone();
    let mut choice = None;
    let mut error = None;
    match request {
        Request::Choose => {
            if let Some(path) = rfd::FileDialog::new()
                .set_title(crate::i18n::gettext(locale, "Choose ffmpeg executable"))
                .pick_file()
            {
                match crate::media_tools::verify(&path, "ffmpeg") {
                    Ok(()) => {
                        desired = Some(path.clone());
                        choice = Some(Some(path));
                    }
                    Err(why) => error = Some(why),
                }
            }
        }
        Request::Automatic => {
            desired = None;
            choice = Some(None);
        }
        Request::Recheck => {}
    }
    let ffmpeg = inspect(
        crate::media_tools::resolve_for("ffmpeg", desired.as_deref()),
        "ffmpeg",
    );
    let ffprobe = inspect(
        crate::media_tools::resolve_for("ffprobe", desired.as_deref()),
        "ffprobe",
    );
    Checked {
        ffmpeg,
        ffprobe,
        choice,
        error,
    }
}

/// Translate application-authored diagnostics; OS technical details stay intact.
pub fn error_text(locale: crate::i18n::Locale, error: &str) -> String {
    use crate::i18n::gettext;
    match error {
        "Media-tool check stopped unexpectedly"=>gettext(locale,"Media-tool check stopped unexpectedly").into_owned(),
        "The selected ffmpeg is no longer executable. Choose another in Settings > Media tools."=>gettext(locale,"The selected ffmpeg is no longer executable. Choose another in Settings > Media tools.").into_owned(),
        "Choose an absolute executable file"=>gettext(locale,"Choose an absolute executable file").into_owned(),
        "Executable check timed out"=>gettext(locale,"Executable check timed out").into_owned(),
        "The selected file did not identify as the requested media tool"=>gettext(locale,"The selected file did not identify as the requested media tool").into_owned(),
        _=>if let Some((tool,dirs))=error.split_once(" was not found. Searched: ") {
            gettext(locale,"{tool} was not found. Searched: {paths}").replace("{tool}",tool).replace("{paths}",dirs)
        }else{error.to_owned()},
    }
}
