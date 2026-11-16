//! Push feed from the Beeper Desktop API (`/v1/ws`, experimental).
//!
//! The feed only wakes the polling loop: a chat or message event sends one
//! unit on the channel and the loop polls at once. Nothing is read from the
//! event payload, so a changed or missing feed costs latency, never data:
//! the 3 s and 15 s polls in `beeper.rs` keep running as the fallback.
//! Only plain `http://` servers are handled (the local Desktop API); an
//! `https://` base URL stays on polling.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

const PING_EVERY: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_STEPS: [u64; 4] = [2, 5, 15, 30];
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const OP_TEXT: u8 = 1;
const OP_CLOSE: u8 = 8;
const OP_PING: u8 = 9;
const OP_PONG: u8 = 10;

/// Stops the feed thread when dropped, so it ends with the backend loop.
pub struct Feed(Arc<AtomicBool>);

impl Drop for Feed {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Starts the feed for `base_url`. Returns None when the URL is not plain
/// http, so the caller simply keeps polling.
pub fn spawn(base_url: &str, token: &str, wake: mpsc::Sender<()>) -> Option<Feed> {
    let authority = base_url.strip_prefix("http://")?.trim_end_matches('/');
    if authority.is_empty() || authority.contains('/') {
        return None;
    }
    let address = if authority.contains(':') {
        authority.to_owned()
    } else {
        format!("{authority}:80")
    };
    let host = authority.to_owned();
    let token = token.to_owned();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    std::thread::Builder::new()
        .name("beeper-ws".into())
        .spawn(move || {
            let mut failures = 0usize;
            while !thread_stop.load(Ordering::Relaxed) {
                let connected = run_once(&address, &host, &token, &wake, &thread_stop);
                if connected {
                    failures = 0;
                }
                let wait = RETRY_STEPS[failures.min(RETRY_STEPS.len() - 1)];
                failures += 1;
                for _ in 0..wait {
                    if thread_stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        })
        .ok()?;
    Some(Feed(stop))
}

/// One connection. Returns whether the handshake succeeded.
fn run_once(
    address: &str,
    host: &str,
    token: &str,
    wake: &mpsc::Sender<()>,
    stop: &AtomicBool,
) -> bool {
    // A name like localhost can resolve to ::1 first while the server
    // listens on IPv4 only, so every resolved address gets a try.
    let Some(stream) = address.to_socket_addrs().ok().and_then(|found| {
        found
            .into_iter()
            .find_map(|socket| TcpStream::connect_timeout(&socket, CONNECT_TIMEOUT).ok())
    }) else {
        return false;
    };
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(PING_EVERY));
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
    let Ok(mut writer) = stream.try_clone() else {
        return false;
    };
    let mut reader = BufReader::new(stream);
    let mut key_bytes = [0u8; 16];
    key_bytes[..8].copy_from_slice(&random_u64().to_le_bytes());
    key_bytes[8..].copy_from_slice(&random_u64().to_le_bytes());
    let key = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(key_bytes)
    };
    let request = format!(
        "GET /v1/ws HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {token}\r\n\r\n"
    );
    if writer.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut status = String::new();
    if reader.read_line(&mut status).is_err() || !status.contains(" 101") {
        return false;
    }
    // Skip the response headers; frames start after the blank line.
    for _ in 0..64 {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(n) if n > 0 && line != "\r\n" => {}
            Ok(n) if n > 0 => break,
            _ => return false,
        }
    }
    let subscribe = br#"{"type":"subscriptions.set","requestID":"solmu","chatIDs":["*"]}"#;
    if writer
        .write_all(&encode_frame(OP_TEXT, subscribe, mask()))
        .is_err()
    {
        return true;
    }
    let mut message: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        // Wait for the first byte of a frame without consuming it, so a quiet
        // link can be probed with a ping. Once a frame has started, any
        // error (a stall included) reconnects instead of resuming mid-frame.
        match reader.fill_buf() {
            Ok([]) => return true,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if writer
                    .write_all(&encode_frame(OP_PING, b"", mask()))
                    .is_err()
                {
                    return true;
                }
                continue;
            }
            Err(_) => return true,
        }
        match read_frame(&mut reader) {
            Ok((opcode, fin, payload)) => match opcode {
                OP_PING => {
                    if writer
                        .write_all(&encode_frame(OP_PONG, &payload, mask()))
                        .is_err()
                    {
                        return true;
                    }
                }
                OP_CLOSE => return true,
                0 | OP_TEXT => {
                    if opcode == OP_TEXT {
                        message.clear();
                    }
                    message.extend_from_slice(&payload);
                    if message.len() > MAX_MESSAGE {
                        return true;
                    }
                    if fin {
                        if is_domain_event(&message) && wake.send(()).is_err() {
                            return true;
                        }
                        message.clear();
                    }
                }
                _ => {}
            },
            Err(_) => return true,
        }
    }
    true
}

/// A random value from std's per-instance hasher seed. The handshake key and
/// frame masks only have to vary between frames, not resist an attacker, on a
/// loopback link that already carries the bearer token in the clear.
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}

fn mask() -> [u8; 4] {
    (random_u64() as u32).to_le_bytes()
}

/// A client frame: FIN set, masked as RFC 6455 requires of clients.
fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        len if len < 126 => frame.push(0x80 | len as u8),
        len if len <= u16::MAX as usize => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    frame
}

/// Reads one frame: (opcode, fin, unmasked payload).
fn read_frame(reader: &mut impl Read) -> io::Result<(u8, bool, Vec<u8>)> {
    let mut head = [0u8; 2];
    reader.read_exact(&mut head)?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7f {
        126 => {
            let mut bytes = [0u8; 2];
            reader.read_exact(&mut bytes)?;
            u16::from_be_bytes(bytes) as u64
        }
        127 => {
            let mut bytes = [0u8; 8];
            reader.read_exact(&mut bytes)?;
            u64::from_be_bytes(bytes)
        }
        len => len as u64,
    };
    if len > MAX_MESSAGE as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask)?;
    }
    let mut payload = vec![0u8; len as usize];
    reader.read_exact(&mut payload)?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok((opcode, fin, payload))
}

/// Chat and message events wake the poller; control messages do not.
fn is_domain_event(text: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("type")?.as_str().map(str::to_owned))
        .is_some_and(|kind| kind.starts_with("chat.") || kind.starts_with("message."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_frame_is_masked_and_reads_back_to_its_payload() {
        for size in [5usize, 125, 126, 70_000] {
            let payload = vec![b'x'; size];
            let frame = encode_frame(OP_TEXT, &payload, [1, 2, 3, 4]);
            assert_eq!(frame[1] & 0x80, 0x80, "client frames are masked");
            let (opcode, fin, read) = read_frame(&mut frame.as_slice()).unwrap();
            assert_eq!((opcode, fin), (OP_TEXT, true));
            assert_eq!(read, payload, "size {size}");
        }
    }

    #[test]
    fn an_unmasked_server_frame_is_read_as_is() {
        let mut frame = vec![0x81, 2];
        frame.extend_from_slice(b"hi");
        let (opcode, fin, payload) = read_frame(&mut frame.as_slice()).unwrap();
        assert_eq!(
            (opcode, fin, payload.as_slice()),
            (OP_TEXT, true, &b"hi"[..])
        );
    }

    #[test]
    fn an_oversized_frame_is_refused_before_it_is_allocated() {
        let mut frame = vec![0x81, 127];
        frame.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(read_frame(&mut frame.as_slice()).is_err());
    }

    #[test]
    fn only_chat_and_message_events_count_as_wakes() {
        assert!(is_domain_event(
            br#"{"type":"message.upserted","chatID":"a"}"#
        ));
        assert!(is_domain_event(br#"{"type":"chat.deleted","chatID":"a"}"#));
        assert!(!is_domain_event(br#"{"type":"ready","version":1}"#));
        assert!(!is_domain_event(br#"{"type":"error","code":"X"}"#));
        assert!(!is_domain_event(b"not json"));
    }

    #[test]
    fn the_feed_subscribes_and_wakes_on_an_event_that_arrives_in_pieces() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            assert!(request.starts_with("GET /v1/ws HTTP/1.1"));
            assert!(request.contains("Authorization: Bearer tok"));
            stream
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n")
                .unwrap();
            let (_, _, subscribe) = read_frame(&mut reader).unwrap();
            assert!(
                String::from_utf8(subscribe)
                    .unwrap()
                    .contains("subscriptions.set")
            );
            let event = br#"{"type":"message.upserted","chatID":"a"}"#;
            let mut frame = vec![0x81, event.len() as u8];
            frame.extend_from_slice(event);
            stream.write_all(&frame[..1]).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(150));
            stream.write_all(&frame[1..]).unwrap();
            stream.write_all(&[0x88, 0]).unwrap();
        });
        let (wake, woken) = mpsc::channel();
        let stop = AtomicBool::new(false);
        // localhost may resolve to ::1 first: the IPv4 address must still be tried.
        let connected = run_once(
            &format!("localhost:{port}"),
            "localhost",
            "tok",
            &wake,
            &stop,
        );
        server.join().unwrap();
        assert!(connected);
        assert!(woken.try_recv().is_ok());
    }

    #[test]
    fn a_base_url_that_is_not_plain_http_stays_on_polling() {
        let (wake, _) = mpsc::channel();
        assert!(spawn("https://localhost:23373", "t", wake.clone()).is_none());
        assert!(spawn("http://localhost:23373/extra/path", "t", wake).is_none());
    }
}
