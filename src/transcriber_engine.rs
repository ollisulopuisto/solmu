//! Local speech engines behind the transcriber's Engine trait. Every
//! backend runs on the user's own machines; there is no cloud path.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::transcriber::Engine;

/// whisper.cpp's CLI (`whisper-cli`, formerly `main`): one process per
/// voice note, text on stdout. Voice notes are seconds long, so process
/// startup dominates; a bounded wall-clock cap keeps a wedged model from
/// stalling the queue.
pub struct WhisperCpp {
    /// Path to the whisper.cpp binary.
    pub binary: PathBuf,
    /// Path to the ggml/gguf model file.
    pub model: PathBuf,
    /// Wall-clock cap per voice note.
    pub timeout: Duration,
}

impl WhisperCpp {
    pub fn new(binary: PathBuf, model: PathBuf) -> Self {
        Self {
            binary,
            model,
            timeout: Duration::from_secs(120),
        }
    }
}

impl Engine for WhisperCpp {
    fn transcribe(&mut self, audio: &Path, language: &str) -> Result<String, String> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(audio)
            .arg("--no-timestamps")
            .arg("--output-txt")
            .arg("-of")
            .arg("/dev/stdout")
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Auto-detect when no language is pinned; whisper.cpp wants no
        // -l flag at all for that.
        if !language.is_empty() {
            cmd.arg("-l").arg(language);
        }
        let started = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("whisper.cpp not runnable at {}: {e}", self.binary.display()))?;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    let mut out = String::new();
                    use std::io::Read;
                    child.stdout.take().map(|mut s| s.read_to_string(&mut out));
                    let text = out.trim().to_string();
                    if text.is_empty() {
                        return Err("whisper.cpp produced no transcript".into());
                    }
                    return Ok(text);
                }
                Ok(Some(status)) => return Err(format!("whisper.cpp exited {status}")),
                Ok(None) => {
                    if started.elapsed() > self.timeout {
                        let _ = child.kill();
                        return Err("whisper.cpp timed out".into());
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(format!("whisper.cpp wait failed: {e}")),
            }
        }
    }
}

/// PCM the Wyoming service accepts: 16 kHz mono signed 16-bit LE.
pub const WYOMING_RATE: u32 = 16_000;

/// Converts a downloaded voice note into Wyoming PCM. The app wires its
/// existing rodio/symphonia decode here; tests inject fixtures. Keeping
/// it a trait object means the engine module never links the audio stack.
pub type PcmDecoder = Box<dyn FnMut(&Path) -> Result<Vec<u8>, String> + Send>;

/// The household Whisper behind the Wyoming protocol: TCP JSON events,
/// audio streamed as raw 16 kHz mono chunks. Wyoming is the user's
/// measured accuracy winner and already runs on the Studio, so a voice
/// note reuses it without loading a second model. The address is explicit
/// config; an unset or unreachable service is a definitive job error,
/// never a silent fallback to another engine.
pub struct Wyoming {
    /// host:port of the Wyoming Whisper service.
    pub addr: String,
    pub timeout: Duration,
    /// Turns an encoded voice note (ogg/opus, m4a, mp3) into Wyoming PCM.
    pub decode: Option<PcmDecoder>,
}

impl Wyoming {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            timeout: Duration::from_secs(120),
            decode: None,
        }
    }

    fn write_event(
        stream: &mut std::net::TcpStream,
        event: &serde_json::Value,
        payload: Option<&[u8]>,
    ) -> Result<(), String> {
        use std::io::Write;
        let mut line = serde_json::to_string(event).map_err(|e| e.to_string())?;
        if let Some(payload) = payload {
            // Insert payload_length into the event header.
            line.pop(); // closing brace
            line.push_str(&format!(",\"payload_length\":{}}}", payload.len()));
        }
        stream
            .write_all(line.as_bytes())
            .and_then(|()| stream.write_all(b"\n"))
            .map_err(|e| format!("wyoming write: {e}"))?;
        if let Some(payload) = payload {
            stream
                .write_all(payload)
                .map_err(|e| format!("wyoming audio write: {e}"))?;
        }
        Ok(())
    }
}

impl Engine for Wyoming {
    fn transcribe(&mut self, audio: &Path, language: &str) -> Result<String, String> {
        use std::io::{BufRead, Read};
        let decode = self
            .decode
            .as_mut()
            .ok_or_else(|| "wyoming: no PCM decoder wired".to_string())?;
        let pcm = decode(audio)?;
        let addr = self.addr.clone();
        let timeout = self.timeout;
        let mut stream = std::net::TcpStream::connect(
            addr.parse::<std::net::SocketAddr>()
                .map_err(|e| format!("wyoming address {addr:?} invalid: {e}"))?,
        )
        .map_err(|e| format!("wyoming service at {addr} unreachable: {e}"))?;
        stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
            .map_err(|e| format!("wyoming timeout config: {e}"))?;

        let transcribe = if language.is_empty() {
            serde_json::json!({"type": "transcribe"})
        } else {
            serde_json::json!({"type": "transcribe", "data": {"language": language}})
        };
        Self::write_event(&mut stream, &transcribe, None)?;
        Self::write_event(
            &mut stream,
            &serde_json::json!({
                "type": "audio-start",
                "data": {"format": "raw", "rate": WYOMING_RATE, "width": 2, "channels": 1}
            }),
            None,
        )?;
        for chunk in pcm.chunks(8192) {
            Self::write_event(
                &mut stream,
                &serde_json::json!({
                    "type": "audio-chunk",
                    "data": {"rate": WYOMING_RATE, "width": 2, "channels": 1}
                }),
                Some(chunk),
            )?;
        }
        Self::write_event(
            &mut stream,
            &serde_json::json!({"type": "audio-stop"}),
            None,
        )?;

        // Events are header JSON lines. The reference Wyoming library
        // (rhasspy/wyoming, which wyoming-faster-whisper and the Parakeet
        // MLX servers build on) never inlines event data: it moves `data`
        // out of the header into `data_length` raw JSON bytes written right
        // after the header line, with no newline of their own. A
        // `payload_length` header likewise means that many binary bytes
        // follow. Both byte runs must be consumed before the next header
        // line, and the data bytes merged back into `data`, or every
        // transcript from a real server reads as empty.
        let mut reader = std::io::BufReader::new(stream);
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() > deadline {
                return Err("wyoming timed out waiting for the transcript".into());
            }
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .map_err(|e| format!("wyoming read: {e}"))?;
            if n == 0 {
                return Err("wyoming closed without a transcript".into());
            }
            let mut event: serde_json::Value =
                serde_json::from_str(line.trim()).map_err(|e| format!("wyoming bad event: {e}"))?;
            if let Some(len) = event
                .get("data_length")
                .and_then(|v| v.as_u64())
                .filter(|l| *l > 0)
            {
                let mut buf = vec![0u8; len as usize];
                reader
                    .read_exact(&mut buf)
                    .map_err(|e| format!("wyoming data read: {e}"))?;
                let extra: serde_json::Value = serde_json::from_slice(&buf)
                    .map_err(|e| format!("wyoming bad event data: {e}"))?;
                if let Some(extra) = extra.as_object() {
                    let data = event
                        .as_object_mut()
                        .ok_or_else(|| "wyoming event header is not an object".to_string())?
                        .entry("data")
                        .or_insert_with(|| serde_json::json!({}));
                    if let Some(data) = data.as_object_mut() {
                        for (k, v) in extra {
                            data.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            if let Some(len) = event
                .get("payload_length")
                .and_then(|v| v.as_u64())
                .filter(|l| *l > 0)
            {
                let mut buf = vec![0u8; len as usize];
                reader
                    .read_exact(&mut buf)
                    .map_err(|e| format!("wyoming payload read: {e}"))?;
            }
            match event.get("type").and_then(|t| t.as_str()) {
                Some("transcript") => {
                    let text = event
                        .get("data")
                        .and_then(|d| d.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if text.is_empty() {
                        return Err("wyoming returned an empty transcript".into());
                    }
                    return Ok(text);
                }
                Some("error") => {
                    let msg = event
                        .get("data")
                        .and_then(|d| d.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown");
                    return Err(format!("wyoming service error: {msg}"));
                }
                _ => continue,
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Immutable checkout fixtures avoid executing a file just written on
    /// an overlay filesystem, which can transiently return ETXTBSY in CI.
    fn fake_whisper(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/whisper")
            .join(name)
    }

    #[test]
    fn whisper_cpp_returns_stdout_text() {
        let bin = fake_whisper("stdout.sh");
        let mut engine = WhisperCpp::new(bin.clone(), PathBuf::from("/tmp/model.bin"));
        let text = engine.transcribe(Path::new("/tmp/note.ogg"), "fi").unwrap();
        assert_eq!(text, "hei maailma");
    }

    #[test]
    fn whisper_cpp_maps_missing_binary() {
        let mut engine = WhisperCpp::new(PathBuf::from("/no/such/binary"), PathBuf::from("/m"));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("not runnable"), "got: {err}");
    }

    #[test]
    fn whisper_cpp_maps_failure_exit() {
        let bin = fake_whisper("failure.sh");
        let mut engine = WhisperCpp::new(bin.clone(), PathBuf::from("/m"));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("exited"), "got: {err}");
    }

    #[test]
    fn whisper_cpp_maps_empty_output() {
        let bin = fake_whisper("empty.sh");
        let mut engine = WhisperCpp::new(bin.clone(), PathBuf::from("/m"));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("no transcript"), "got: {err}");
    }

    #[test]
    fn whisper_cpp_timeout_kills() {
        let bin = fake_whisper("timeout.sh");
        let mut engine = WhisperCpp::new(bin.clone(), PathBuf::from("/m"));
        engine.timeout = Duration::from_millis(300);
        let started = Instant::now();
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("timed out"), "got: {err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    use std::io::{BufRead, Read, Write};
    use std::net::TcpListener;

    /// A Wyoming stub that writes `reply` verbatim once audio stops.
    /// Used both for inline-data replies and for exact reference-library
    /// framing, where event data travels outside the header line.
    fn wyoming_stub_raw(reply: Vec<u8>) -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let mut log = String::new();
            let mut audio_bytes = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                log.push_str(&line);
                let event: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
                if let Some(len) = event.get("payload_length").and_then(|v| v.as_u64()) {
                    let mut buf = vec![0u8; len as usize];
                    reader.read_exact(&mut buf).unwrap();
                    audio_bytes += len as usize;
                }
                if event.get("type").and_then(|t| t.as_str()) == Some("audio-stop") {
                    reader.get_mut().write_all(&reply).unwrap();
                    break;
                }
            }
            tx.send(format!("{log}|audio_bytes={audio_bytes}")).unwrap();
        });
        (addr, rx)
    }

    /// A Wyoming stub: speaks the event protocol, records what it heard,
    /// replies with a fixed transcript. Proves framing, payload handling
    /// and language forwarding without the real service.
    fn wyoming_stub(reply_text: &'static str) -> (String, std::sync::mpsc::Receiver<String>) {
        let reply = format!(
            "{{\"type\":\"transcript\",\"data\":{{\"text\":\"{}\"}}}}\n",
            reply_text
        );
        wyoming_stub_raw(reply.into_bytes())
    }

    /// The exact wire format of the reference Wyoming library
    /// (rhasspy/wyoming `async_write_event`, used by wyoming-faster-whisper
    /// and the Parakeet MLX servers): the header line names the type and
    /// carries `data_length` and a version, the JSON data bytes follow the
    /// header newline with no newline of their own, then the next event
    /// header starts immediately.
    fn rhasspy_event(event_type: &str, data_json: &str) -> Vec<u8> {
        format!(
            "{{\"type\":\"{}\",\"data_length\":{},\"version\":\"1.7.0\"}}\n{}",
            event_type,
            data_json.len(),
            data_json
        )
        .into_bytes()
    }

    fn pcm_fixture(len: usize) -> Vec<u8> {
        vec![0x5Au8; len]
    }

    #[test]
    fn wyoming_streams_pcm_and_returns_transcript() {
        let (addr, rx) = wyoming_stub("hei maailma");
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(9000))));
        let text = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap();
        assert_eq!(text, "hei maailma");
        let log = rx.recv().unwrap();
        assert!(log.contains("\"type\":\"transcribe\""));
        assert!(log.contains("\"language\":\"fi\""));
        assert!(log.contains("\"type\":\"audio-start\""));
        assert!(log.contains("\"rate\":16000"));
        // 9000 bytes = 8192 + 808 in two chunks.
        assert!(log.ends_with("audio_bytes=9000"), "log: {log}");
    }

    /// The bug behind "wyoming returned an empty transcript" on a working
    /// server: a reference-library server frames the transcript with
    /// `data_length` instead of inline `data`, so a reader that only looks
    /// at the header line sees no text. Reproduced against a stub speaking
    /// the exact framing; before the fix this test failed with the empty
    /// transcript error.
    #[test]
    fn wyoming_reads_rhasspy_framed_transcript() {
        let (addr, _rx) = wyoming_stub_raw(rhasspy_event(
            "transcript",
            "{\"text\":\"Testaan yht\u{00e4}\u{00e4} juttua.\"}",
        ));
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(9000))));
        let text = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap();
        assert_eq!(text, "Testaan yht\u{00e4}\u{00e4} juttua.");
    }

    /// Servers may emit framed non-transcript events (info, timing) before
    /// the transcript; their data bytes must be consumed or the next header
    /// line parses as garbage.
    #[test]
    fn wyoming_skips_framed_non_transcript_events() {
        let mut reply = rhasspy_event("not-started", "{\"reason\":\"warming up\"}");
        reply.extend(rhasspy_event("transcript", "{\"text\":\"hei maailma\"}"));
        let (addr, _rx) = wyoming_stub_raw(reply);
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let text = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap();
        assert_eq!(text, "hei maailma");
    }

    /// A genuinely silent clip still errors as empty, never a silent
    /// fallback; only the framing changed, not the empty-text rule.
    #[test]
    fn wyoming_framed_empty_transcript_is_an_error() {
        let (addr, _rx) = wyoming_stub_raw(rhasspy_event("transcript", "{\"text\":\"  \"}"));
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("empty transcript"), "got: {err}");
    }

    #[test]
    fn wyoming_empty_language_omits_language_field() {
        let (addr, rx) = wyoming_stub("ok");
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        engine.transcribe(Path::new("/a.ogg"), "").unwrap();
        let log = rx.recv().unwrap();
        let line = log.lines().next().unwrap();
        assert!(line.contains("\"type\":\"transcribe\""));
        assert!(!line.contains("language"), "line: {line}");
    }

    #[test]
    fn wyoming_unreachable_is_definitive_error() {
        let mut engine = Wyoming::new("127.0.0.1:1");
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("unreachable"), "got: {err}");
    }

    #[test]
    fn wyoming_requires_decoder() {
        let mut engine = Wyoming::new("127.0.0.1:1");
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("no PCM decoder"), "got: {err}");
    }

    #[test]
    fn wyoming_bad_address_is_definitive_error() {
        let mut engine = Wyoming::new("not-an-addr");
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("invalid"), "got: {err}");
    }
}
