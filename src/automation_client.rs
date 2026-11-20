//! Shared loopback-only client for CLI and stdio MCP. Reads the private token file per call.
use std::{
    io::Read,
    path::{Path, PathBuf},
};
pub struct Client {
    base: String,
    token_file: PathBuf,
    http: reqwest::blocking::Client,
}
pub fn bounded_file(path: &Path, max: u64) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|_| "input file unavailable")?;
    let mut body = String::new();
    file.take(max + 1)
        .read_to_string(&mut body)
        .map_err(|_| "input is not UTF-8")?;
    if body.len() as u64 > max {
        return Err("input file too large".into());
    }
    Ok(body)
}
impl Client {
    pub fn new(base: &str, token_file: PathBuf) -> Result<Self, String> {
        let url = reqwest::Url::parse(base).map_err(|_| "invalid local API URL")?;
        if url.scheme() != "http"
            || url
                .host_str()
                .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
                .is_none_or(|ip| !ip.is_loopback())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err(
                "API URL must be a loopback HTTP origin with no credentials or path".into(),
            );
        }
        let http = reqwest::blocking::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .timeout(std::time::Duration::from_secs(35))
            .build()
            .map_err(|_| "client unavailable")?;
        Ok(Self {
            base: base.trim_end_matches('/').into(),
            token_file,
            http,
        })
    }
    fn token(&self) -> Result<String, String> {
        let meta = std::fs::metadata(&self.token_file).map_err(|_| "token file unavailable")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err("token file must be private (0600)".into());
            }
        }
        if !meta.is_file() {
            return Err("token file is not a file".into());
        }
        let token = bounded_file(&self.token_file, 512)?;
        let token = token.trim();
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err("invalid token file".into());
        }
        Ok(token.into())
    }
    pub fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(u16, String), String> {
        let parsed = if method == "GET" && path == "/v1/capabilities" {
            (crate::read_api::Route::Accounts, path.into())
        } else {
            crate::read_api::route("GET", path).map_err(|_| "route unavailable")?
        };
        if method != "GET"
            && (method != "POST"
                || !matches!(parsed.0, crate::read_api::Route::Messages(_))
                || path.contains('?'))
        {
            return Err("route unavailable".into());
        }
        let request = self
            .http
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| "invalid method")?,
                format!(
                    "{}{}",
                    self.base,
                    if method == "POST" { path } else { &parsed.1 }
                ),
            )
            .bearer_auth(self.token()?);
        let request = if let Some(body) = body {
            request.json(body)
        } else {
            request
        };
        let response=request.send().map_err(|_|"local API unreachable; a write may have unknown outcome, never retry with a new request id")?;
        let status = response.status().as_u16();
        let mut text = String::new();
        response
            .take(1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|_| "response unreadable; do not retry writes with a new request id")?;
        if text.len() > 1024 * 1024 {
            return Err("response too large".into());
        }
        if serde_json::from_str::<serde_json::Value>(&text).is_err() {
            return Err("response is not JSON".into());
        }
        Ok((status, text))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_or_credential_urls_refused() {
        for url in [
            "http://example.com",
            "http://localhost:1",
            "https://127.0.0.1",
            "http://127.0.0.1/path",
            "http://u:p@127.0.0.1",
            "http://127.0.0.1?token=x",
        ] {
            assert!(Client::new(url, "unused".into()).is_err(), "{url}");
        }
        assert!(Client::new("http://127.0.0.1:23377", "unused".into()).is_ok());
    }
}
