//! Credential-free, bounded public-web metadata retrieval. Never use the
//! authenticated Beeper client here. UI and cache policy live above this layer.
use reqwest::{
    Url,
    blocking::{Client, Response},
};
use scraper::{Html, Selector};
use std::{
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const HTML_LIMIT: u64 = 512 * 1024;
const IMAGE_LIMIT: u64 = 2 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Preview {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<Vec<u8>>,
    pub no_store: bool,
}

/// Only globally reachable addresses. Deliberately conservative: previewing
/// special-use or transition networks is not worth reaching local services.
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0
                || a == 10
                || a == 127
                || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || b == 0 || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] <= 0x01ff || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] <= 0x0fff)
        }
    }
}

fn validate(value: &str) -> Result<Url, &'static str> {
    if value.len() > 8192 || value.chars().any(char::is_control) || value.contains('\\') {
        return Err("Invalid preview URL");
    }
    let url = Url::parse(value).map_err(|_| "Invalid preview URL")?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return Err("Preview requires a public HTTPS link");
    }
    let host = url
        .host_str()
        .ok_or("Missing preview host")?
        .trim_matches(['[', ']']);
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !public_ip(ip) {
            return Err("Private preview address blocked");
        }
    } else if !host.contains('.')
        || [
            ".localhost",
            ".local",
            ".internal",
            ".home",
            ".test",
            ".invalid",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix))
    {
        return Err("Private preview host blocked");
    }
    Ok(url)
}

/// Filtering at the resolver used by the socket prevents a second unchecked
/// DNS lookup from turning a validated public hostname into a private address.
// OS resolution may stall outside Rust's request timeout. Keep it outside the
// client's runtime in a process-wide pool of exactly two threads, with a bounded
// queue. A stalled resolver cannot retain a fetch worker or create more threads.
type DnsReply = tokio::sync::oneshot::Sender<std::io::Result<Vec<std::net::SocketAddr>>>;
static DNS: std::sync::LazyLock<std::sync::mpsc::SyncSender<(String, DnsReply)>> =
    std::sync::LazyLock::new(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<(String, DnsReply)>(4);
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for index in 0..2 {
            let rx = rx.clone();
            let _ = std::thread::Builder::new()
                .name(format!("preview-dns-{index}"))
                .spawn(move || {
                    loop {
                        let Ok((host, reply)) = rx.lock().unwrap_or_else(|e| e.into_inner()).recv()
                        else {
                            return;
                        };
                        if reply.is_closed() {
                            continue;
                        }
                        let result = (host.as_str(), 0)
                            .to_socket_addrs()
                            .map(|ips| ips.collect());
                        let _ = reply.send(result);
                    }
                });
        }
        tx
    });
fn public_answers(ips: &[std::net::SocketAddr]) -> bool {
    !ips.is_empty() && ips.iter().all(|address| public_ip(address.ip()))
}
#[derive(Debug)]
struct PublicResolver;
impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            DNS.try_send((host, tx)).map_err(|_| "Preview DNS busy")?;
            let ips = tokio::time::timeout(Duration::from_secs(3), rx).await???;
            if !public_answers(&ips) {
                return Err("Private preview DNS answer blocked".into());
            }
            let addresses: reqwest::dns::Addrs = Box::new(ips.into_iter());
            Ok(addresses)
        })
    }
}

fn client() -> Result<Client, &'static str> {
    // Never silently bypass a configured privacy proxy. A future supported
    // proxy path must enforce the same public-address checks at its egress.
    if ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.is_empty()))
    {
        return Err("Configured proxies are not supported for preview fetching");
    }
    Client::builder()
        .https_only(true)
        .no_proxy()
        .referer(false)
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(PublicResolver))
        .connect_timeout(Duration::from_secs(3))
        .timeout(DEADLINE)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .user_agent("Solmu-LinkPreview/1")
        .build()
        .map_err(|_| "Could not start preview fetch")
}
fn remaining(start: Instant, cancelled: &AtomicBool) -> Result<Duration, &'static str> {
    if cancelled.load(Ordering::Relaxed) {
        return Err("Preview cancelled");
    }
    DEADLINE
        .checked_sub(start.elapsed())
        .filter(|time| !time.is_zero())
        .ok_or("Preview timed out")
}
fn get(
    client: &Client,
    url: Url,
    start: Instant,
    cancelled: &AtomicBool,
) -> Result<Response, &'static str> {
    get_with(url, start, cancelled, |url, timeout| {
        client
            .get(url)
            .timeout(timeout)
            .header("Accept", "text/html,image/png,image/jpeg,image/webp")
            .send()
            .map_err(|_| "Preview request failed")
    })
}
fn get_with(
    mut url: Url,
    start: Instant,
    cancelled: &AtomicBool,
    mut send: impl FnMut(Url, Duration) -> Result<Response, &'static str>,
) -> Result<Response, &'static str> {
    for hop in 0..=3 {
        validate(url.as_str())?;
        url.set_fragment(None);
        let response = send(url.clone(), remaining(start, cancelled)?)?;
        if response.status().is_redirection() {
            if hop == 3 {
                return Err("Too many preview redirects");
            }
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|s| s.to_str().ok())
                .ok_or("Invalid preview redirect")?;
            url = url.join(target).map_err(|_| "Invalid preview redirect")?;
        } else {
            return response
                .error_for_status()
                .map_err(|_| "Preview unavailable");
        }
    }
    Err("Preview unavailable")
}
fn bytes(mut response: Response, limit: u64) -> Result<Vec<u8>, &'static str> {
    if response
        .content_length()
        .is_some_and(|length| length > limit)
        || response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|length| length > limit)
        || response
            .headers()
            .contains_key(reqwest::header::CONTENT_ENCODING)
    {
        return Err("Preview response too large or encoded");
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read preview")?;
    if bytes.len() as u64 > limit {
        return Err("Preview response too large");
    }
    Ok(bytes)
}
fn text(value: Option<String>, limit: usize) -> Option<String> {
    let value = value?.split_whitespace().collect::<Vec<_>>().join(" ");
    let value: String = value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(limit)
        .collect();
    (!value.is_empty()).then_some(value)
}
fn metadata(html: &str) -> (Option<String>, Option<String>, Option<String>) {
    let document = Html::parse_document(html);
    let meta = Selector::parse("meta").expect("constant selector");
    let find = |property: &str| {
        document
            .select(&meta)
            .find(|node| {
                node.value()
                    .attr("property")
                    .or_else(|| node.value().attr("name"))
                    .is_some_and(|key| key.eq_ignore_ascii_case(property))
            })
            .and_then(|node| node.value().attr("content"))
            .map(str::to_owned)
    };
    let title = find("og:title").or_else(|| {
        document
            .select(&Selector::parse("title").expect("constant selector"))
            .next()
            .map(|node| node.text().collect::<String>())
    });
    (
        text(title, 240),
        text(find("og:description").or_else(|| find("description")), 600),
        find("og:image:secure_url").or_else(|| find("og:image")),
    )
}

pub fn fetch(value: &str, cancelled: &AtomicBool) -> Result<Preview, &'static str> {
    let start = Instant::now();
    let url = validate(value)?;
    remaining(start, cancelled)?;
    let client = client()?;
    let response = get(&client, url, start, cancelled)?;
    let html_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|s| s.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if !html_type.eq_ignore_ascii_case("text/html") {
        return Err("Preview is not HTML");
    }
    let mut no_store = response
        .headers()
        .get(reqwest::header::CACHE_CONTROL)
        .and_then(|s| s.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"))
        });
    let base = response.url().clone();
    let html = bytes(response, HTML_LIMIT)?;
    remaining(start, cancelled)?;
    let (title, description, image_url) = metadata(&String::from_utf8_lossy(&html));
    if title.is_none() && description.is_none() {
        return Err("No preview metadata found");
    }
    remaining(start, cancelled)?;
    let image = image_url
        .and_then(|url| base.join(&url).ok())
        .and_then(|url| get(&client, url, start, cancelled).ok())
        .and_then(|response| {
            remaining(start, cancelled).ok()?;
            no_store |= response
                .headers()
                .get(reqwest::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(',')
                        .any(|d| d.trim().eq_ignore_ascii_case("no-store"))
                });
            let kind = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)?
                .to_str()
                .ok()?
                .split(';')
                .next()?
                .trim();
            if !["image/jpeg", "image/png", "image/webp"].contains(&kind) {
                return None;
            }
            let bytes = bytes(response, IMAGE_LIMIT).ok()?;
            remaining(start, cancelled).ok()?;
            let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .ok()?;
            if !matches!(
                reader.format(),
                Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP)
            ) {
                return None;
            }
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(16 * 1024 * 1024);
            reader.limits(limits);
            let decoded = reader.decode().ok()?;
            if u64::from(decoded.width()) * u64::from(decoded.height()) > 4_000_000 {
                return None;
            }
            remaining(start, cancelled).ok()?;
            let thumbnail = decoded.thumbnail(320, 320);
            let mut encoded = std::io::Cursor::new(Vec::new());
            thumbnail
                .write_to(&mut encoded, image::ImageFormat::Png)
                .ok()?;
            Some(encoded.into_inner())
        });
    remaining(start, cancelled)?;
    Ok(Preview {
        title,
        description,
        image,
        no_store,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirects_revalidate_targets_and_never_expose_private_request() {
        let cancelled = AtomicBool::new(false);
        for target in [
            "https://127.0.0.1/secret",
            "http://example.com/plain",
            "https://10.0.0.1/",
            "https://[::1]/",
            "https://user:secret@example.com/",
            "https://example.com:8443/",
        ] {
            let mut sent = Vec::new();
            let result = get_with(
                validate("https://example.com/start?q=keep#fragment").unwrap(),
                Instant::now(),
                &cancelled,
                |url, _| {
                    sent.push(url.to_string());
                    Ok(http::Response::builder()
                        .status(302)
                        .header("Location", target)
                        .body(Vec::<u8>::new())
                        .unwrap()
                        .into())
                },
            );
            assert!(result.is_err(), "{target}");
            assert_eq!(sent, ["https://example.com/start?q=keep"]);
        }
        let mut sent = Vec::new();
        assert!(
            get_with(
                validate("https://example.com/start").unwrap(),
                Instant::now(),
                &cancelled,
                |url, _| {
                    sent.push(url.to_string());
                    Ok(http::Response::builder()
                        .status(302)
                        .header("Location", "/again#drop")
                        .body(Vec::<u8>::new())
                        .unwrap()
                        .into())
                }
            )
            .is_err()
        );
        assert_eq!(sent.len(), 4, "at most three redirects");
        assert!(sent.iter().all(|url| !url.contains('#')));
    }
    #[test]
    fn cancellation_deadline_dns_and_stream_limits_fail_closed() {
        let mut sends = 0;
        let cancelled = AtomicBool::new(true);
        assert!(
            get_with(
                validate("https://example.com/").unwrap(),
                Instant::now(),
                &cancelled,
                |_, _| {
                    sends += 1;
                    Err("unexpected")
                }
            )
            .is_err()
        );
        cancelled.store(false, Ordering::SeqCst);
        assert!(
            get_with(
                validate("https://example.com/").unwrap(),
                Instant::now() - DEADLINE,
                &cancelled,
                |_, _| {
                    sends += 1;
                    Err("unexpected")
                }
            )
            .is_err()
        );
        assert_eq!(sends, 0);
        let public = "93.184.216.34:443".parse().unwrap();
        assert!(public_answers(&[public]));
        assert!(!public_answers(&[]));
        assert!(!public_answers(&[public, "127.0.0.1:443".parse().unwrap()]));
        assert!(!public_answers(&["[::ffff:127.0.0.1]:443"
            .parse()
            .unwrap()]));
        let response =
            |content: Vec<u8>| Response::from(http::Response::builder().body(content).unwrap());
        assert_eq!(bytes(response(vec![1; 4]), 4).unwrap(), vec![1; 4]);
        assert!(
            bytes(response(vec![1; 5]), 4).is_err(),
            "stream cap without content length"
        );
        let encoded = Response::from(
            http::Response::builder()
                .header("Content-Encoding", "gzip")
                .body(vec![1; 3])
                .unwrap(),
        );
        assert!(bytes(encoded, 4).is_err());
        let too_long = Response::from(
            http::Response::builder()
                .header("Content-Length", "99999")
                .body(vec![1])
                .unwrap(),
        );
        assert!(bytes(too_long, 4).is_err());
    }
    #[test]
    fn rejects_private_addresses_encodings_and_non_https() {
        for value in [
            "https://127.0.0.1/",
            "https://2130706433/",
            "https://0x7f000001/",
            "https://[::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://10.0.0.1/",
            "https://100.100.1.1/",
            "https://169.254.169.254/",
            "https://[fc00::1]/",
            "https://localhost/",
            "https://printer.local/",
            "http://example.com/",
            "https://example.com:8443/",
            "https://name:password@example.com/",
        ] {
            assert!(validate(value).is_err(), "{value}");
        }
        assert!(validate("https://example.com/story?q=exact#part").is_ok());
    }
    #[test]
    fn parses_real_html_without_scripts_or_canonical_retargeting() {
        let (title, description, image) = metadata(
            r#"<!doctype html><title>Fallback</title><meta content='A &amp; B' property='og:title'><meta name='description' content='A short description'><meta property='og:image' content='/image.png'><meta property='og:url' content='https://attacker.invalid'><script>fetch('/private')</script>"#,
        );
        assert_eq!(title.as_deref(), Some("A & B"));
        assert_eq!(description.as_deref(), Some("A short description"));
        assert_eq!(image.as_deref(), Some("/image.png"));
    }
}
