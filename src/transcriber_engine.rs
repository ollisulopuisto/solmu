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
        // A regular private file cannot fill a pipe and deadlock the child.
        // Limit both the live output and the final read; no transcripts in logs.
        const MAX_OUTPUT: u64 = 1 << 20;
        let mut output = tempfile::tempfile().map_err(|e| e.to_string())?;
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(audio)
            .arg("--no-timestamps")
            .arg("--no-prints")
            .arg("-l")
            .arg(if language.trim().is_empty() {
                "auto"
            } else {
                language
            })
            .stdin(Stdio::null())
            .stdout(Stdio::from(output.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::null());
        let started = Instant::now();
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("whisper.cpp not runnable at {}: {e}", self.binary.display()))?;
        let result = loop {
            if started.elapsed() > self.timeout {
                break Err("whisper.cpp timed out".to_string());
            }
            match output.metadata() {
                Ok(meta) if meta.len() > MAX_OUTPUT => {
                    break Err("whisper.cpp output exceeded its size limit".to_string());
                }
                Err(e) => break Err(format!("whisper.cpp output check failed: {e}")),
                _ => {}
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => break Ok(()),
                Ok(Some(status)) => break Err(format!("whisper.cpp exited {status}")),
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(e) => break Err(format!("whisper.cpp wait failed: {e}")),
            }
        };
        // Reap on every path, including timeouts and unexpected wait errors.
        if result.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        result?;
        use std::io::{Read, Seek};
        output.rewind().map_err(|e| e.to_string())?;
        let mut text = String::new();
        output
            .take(MAX_OUTPUT + 1)
            .read_to_string(&mut text)
            .map_err(|e| e.to_string())?;
        if text.len() as u64 > MAX_OUTPUT {
            return Err("whisper.cpp output exceeded its size limit".into());
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("whisper.cpp produced no transcript".into());
        }
        Ok(text)
    }
}

/// Decode once with the app's audio stack, then use a private standard WAV.
/// The CLI never needs to know about Opus/M4A/video codecs or remote services.
pub struct DecodedWhisper {
    pub engine: WhisperCpp,
    pub decode: PcmDecoder,
    pub model_verified: bool,
}

fn pcm_wav(pcm: &[u8]) -> Result<tempfile::NamedTempFile, String> {
    use std::io::Write;
    if pcm.is_empty() || !pcm.len().is_multiple_of(2) {
        return Err("Audio contains no valid samples".into());
    }
    let len = u32::try_from(pcm.len())
        .ok()
        .filter(|len| *len <= u32::MAX - 36)
        .ok_or("Audio is too large")?;
    let mut file = tempfile::Builder::new()
        .prefix("solmu-transcribe-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| e.to_string())?;
    let writer = &mut file;
    writer
        .write_all(b"RIFF")
        .and_then(|()| writer.write_all(&(36 + len).to_le_bytes()))
        .and_then(|()| writer.write_all(b"WAVEfmt "))
        .and_then(|()| writer.write_all(&16u32.to_le_bytes()))
        .and_then(|()| writer.write_all(&1u16.to_le_bytes()))
        .and_then(|()| writer.write_all(&1u16.to_le_bytes()))
        .and_then(|()| writer.write_all(&16_000u32.to_le_bytes()))
        .and_then(|()| writer.write_all(&32_000u32.to_le_bytes()))
        .and_then(|()| writer.write_all(&2u16.to_le_bytes()))
        .and_then(|()| writer.write_all(&16u16.to_le_bytes()))
        .and_then(|()| writer.write_all(b"data"))
        .and_then(|()| writer.write_all(&len.to_le_bytes()))
        .and_then(|()| writer.write_all(pcm))
        .and_then(|()| writer.flush())
        .map_err(|e| e.to_string())?;
    Ok(file)
}

impl Engine for DecodedWhisper {
    fn transcribe(&mut self, audio: &Path, language: &str) -> Result<String, String> {
        let spec = crate::whisper_runtime::artifact()
            .ok_or("Whisper runtime is not configured for this platform")?;
        crate::whisper_runtime::verify(&self.engine.binary, spec)?;
        if !self.model_verified {
            crate::local_whisper::verify_model(&self.engine.model)?;
            self.model_verified = true;
        }
        let pcm = (self.decode)(audio)?;
        let wav = pcm_wav(&pcm)?;
        self.engine.transcribe(wav.path(), language)
    }
}

/// PCM the Wyoming service accepts: 16 kHz mono signed 16-bit LE.
pub const WYOMING_RATE: u32 = 16_000;

/// Largest event data or payload a Wyoming server may ask us to buffer.
/// Transcript JSON is a few KB and ASR replies carry no binary payload;
/// 1 MiB is generous, and anything larger is a buggy or hostile peer,
/// not a transcript.
const MAX_WYOMING_EVENT_BYTES: u64 = 1024 * 1024;

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
        // transcript from a real server reads as empty. Lengths come off
        // the wire, so they are capped before allocating: a transcript's
        // JSON data is a few KB and ASR replies carry no binary payload.
        // The socket read timeout set above bounds stalled reads; map it
        // to the same timeout error the between-line deadline returns.
        fn read_err(context: &str) -> impl Fn(std::io::Error) -> String + '_ {
            move |e| {
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock
                {
                    "wyoming timed out waiting for the transcript".to_string()
                } else {
                    format!("wyoming {context} read: {e}")
                }
            }
        }
        let mut reader = std::io::BufReader::new(stream);
        let deadline = Instant::now() + timeout;
        loop {
            if Instant::now() > deadline {
                return Err("wyoming timed out waiting for the transcript".into());
            }
            let mut line = String::new();
            let n = reader.read_line(&mut line).map_err(read_err("header"))?;
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
                if len > MAX_WYOMING_EVENT_BYTES {
                    return Err(format!("wyoming event data too large: {len} bytes"));
                }
                let mut buf = vec![0u8; len as usize];
                reader.read_exact(&mut buf).map_err(read_err("data"))?;
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
                if len > MAX_WYOMING_EVENT_BYTES {
                    return Err(format!("wyoming event payload too large: {len} bytes"));
                }
                let mut buf = vec![0u8; len as usize];
                reader.read_exact(&mut buf).map_err(read_err("payload"))?;
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

    #[test]
    fn whisper_cpp_requests_auto_language_and_portable_stdout() {
        let mut engine = WhisperCpp::new(fake_whisper("args.sh"), PathBuf::from("/tmp/model.bin"));
        let text = engine.transcribe(Path::new("/tmp/a.wav"), "").unwrap();
        assert!(text.ends_with("-l\nauto"));
        assert!(text.contains("--no-prints"));
        assert!(!text.contains("/dev/stdout"));
        assert!(!text.contains("--output-txt"));
    }

    #[test]
    fn whisper_cpp_rejects_excess_output_without_pipe_deadlock() {
        let mut engine = WhisperCpp::new(fake_whisper("flood.sh"), PathBuf::from("/tmp/model.bin"));
        engine.timeout = Duration::from_secs(3);
        let started = Instant::now();
        assert!(
            engine
                .transcribe(Path::new("/tmp/a.wav"), "")
                .unwrap_err()
                .contains("size limit")
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn wav_is_standard_mono_16khz_pcm_and_is_removed_on_drop() {
        let pcm = [0, 0, 255, 127, 0, 128];
        let file = pcm_wav(&pcm).unwrap();
        let path = file.path().to_owned();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(&bytes[24..28], &16_000u32.to_le_bytes());
        assert_eq!(&bytes[44..], &pcm);
        drop(file);
        assert!(!path.exists());
        assert!(pcm_wav(&[]).is_err());
        assert!(pcm_wav(&[0]).is_err());
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

    /// Lengths come off the wire; a huge one must error before any
    /// allocation, not abort on OOM or stall on a slow stream.
    #[test]
    fn wyoming_rejects_absurd_data_length() {
        let (addr, _rx) = wyoming_stub_raw(
            b"{\"type\":\"transcript\",\"data_length\":18446744073709551615}\n".to_vec(),
        );
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("data too large"), "got: {err}");
    }

    #[test]
    fn wyoming_rejects_absurd_payload_length() {
        let (addr, _rx) = wyoming_stub_raw(
            b"{\"type\":\"transcript\",\"payload_length\":18446744073709551615}\n".to_vec(),
        );
        let mut engine = Wyoming::new(addr);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("payload too large"), "got: {err}");
    }

    /// A stalled server must surface the socket read timeout as the same
    /// timeout error the between-line deadline returns, not a raw io
    /// error string.
    #[test]
    fn wyoming_stalled_server_times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            // Accept and hold: read whatever arrives, never reply.
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let mut sink = String::new();
            while reader.read_line(&mut sink).unwrap_or(0) > 0 {
                sink.clear();
            }
        });
        let mut engine = Wyoming::new(addr);
        engine.timeout = Duration::from_millis(300);
        engine.decode = Some(Box::new(|_| Ok(pcm_fixture(4))));
        let started = Instant::now();
        let err = engine.transcribe(Path::new("/a.ogg"), "fi").unwrap_err();
        assert!(err.contains("timed out"), "got: {err}");
        assert!(started.elapsed() < Duration::from_secs(5));
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

/// Explicit manual runtime validation. The normal suite never downloads a
/// model or runs a downloaded helper. Inputs come from a selected CI artifact.
#[cfg(all(test, target_os = "macos"))]
mod real_runtime_smoke {
    use super::*;
    #[test]
    #[ignore = "explicit manual helper/model smoke test only"]
    fn local_whisper_real_recording_installs_repairs_and_transcribes_opus() {
        let directory =
            PathBuf::from(std::env::var("SOLMU_HELPER_FIXTURE").expect("helper fixture"));
        let metadata: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join("solmu-whisper-v1.9.5-arm64.json")).unwrap(),
        )
        .unwrap();
        let archive = directory.join(metadata["filename"].as_str().unwrap());
        assert_eq!(
            metadata["upstream_revision"],
            "d1be6fde11ac6e0407606b4e42fe72d34add8037"
        );
        let executable_hash = metadata["executable_sha256"].as_str().unwrap_or_else(|| {
            // The first signed arm64 trial predates the metadata field. Both
            // hashes were independently recorded before this validation test.
            assert_eq!(
                metadata["sha256"],
                "a36cf9a93cff6bc5f08c65f0de0dd42f66b996880b6098abaf1fdc66b84a367c"
            );
            "a2b9571b58e06eb808891e729b4561d5b212a942d37670c329c0aa2dbf33db42"
        });
        let spec = crate::whisper_runtime::Artifact {
            url: "https://invalid.invalid/not-used-in-offline-install-smoke",
            bytes: metadata["size"].as_u64().unwrap(),
            sha256: Box::leak(
                metadata["sha256"]
                    .as_str()
                    .unwrap()
                    .to_owned()
                    .into_boxed_str(),
            ),
            executable_sha256: Box::leak(executable_hash.to_owned().into_boxed_str()),
        };
        let state = tempfile::tempdir().unwrap();
        // Set only by the explicit public-download validation dispatch after
        // the reviewed runtime URL and hashes have been pinned.
        if std::env::var_os("SOLMU_VALIDATE_PUBLIC_DOWNLOAD").is_some() {
            let download = crate::local_whisper::Download::start(state.path().to_owned(), || {});
            let started = Instant::now();
            loop {
                match download.state() {
                    crate::local_whisper::DownloadState::Ready(model) => {
                        crate::local_whisper::verify_model(&model).unwrap();
                        let spec =
                            crate::whisper_runtime::artifact().expect("pinned public runtime");
                        crate::whisper_runtime::verify(
                            &crate::whisper_runtime::executable(state.path()),
                            spec,
                        )
                        .expect("public downloader installed verified runtime");
                        break;
                    }
                    crate::local_whisper::DownloadState::Downloading { .. }
                        if started.elapsed() < Duration::from_secs(20 * 60) =>
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    status => panic!("public downloader did not become ready: {status:?}"),
                }
            }
        }
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let binary = crate::whisper_runtime::install_archive(&archive, state.path(), spec, &cancel)
            .expect("verified install");
        crate::whisper_runtime::verify(&binary, spec).expect("post-install Gatekeeper/hash");
        std::fs::write(&binary, b"bad install").unwrap();
        assert!(crate::whisper_runtime::verify(&binary, spec).is_err());
        let binary = crate::whisper_runtime::install_archive(&archive, state.path(), spec, &cancel)
            .expect("repair damaged runtime");
        let model = PathBuf::from(std::env::var("SOLMU_MODEL_FIXTURE").expect("model fixture"));
        crate::local_whisper::verify_model(&model).expect("pinned model");
        let recording =
            PathBuf::from(std::env::var("SOLMU_RECORDING_FIXTURE").expect("recording fixture"));
        let samples =
            crate::audio::decode_for_transcription(&recording).expect("decode real speech");
        let opus = crate::voice::encode(&samples).expect("encode voice-note Opus");
        let note = state.path().join("recording.ogg");
        std::fs::write(&note, opus).unwrap();
        let samples = crate::audio::decode_for_transcription(&note).expect("decode voice note");
        let pcm = crate::transcriber::wyoming_pcm_from_mono(&samples, crate::voice::RATE);
        let wav = pcm_wav(&pcm).unwrap();
        crate::whisper_runtime::verify(&binary, spec).unwrap();
        let mut engine = WhisperCpp::new(binary, model);
        let transcript = engine.transcribe(wav.path(), "en").expect("real inference");
        assert!(
            transcript.to_lowercase().contains("ask not"),
            "expected speech recognized"
        );
        // Never print transcripts or fixture contents to CI logs.
    }
}
