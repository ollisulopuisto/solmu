//! Local text send CLI. Uses the same revocable scoped API as MCP, never the upstream token.
use std::{collections::HashMap, path::PathBuf};
fn main() {
    if let Err(message) = run() {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("send") {
        return Err("usage: solmu-control send --token-file PATH --account ID --chat ID --text-file PATH --request-id ID [--url http://127.0.0.1:23377]".into());
    }
    if !(args.len() - 1).is_multiple_of(2) {
        return Err("each option needs one value".into());
    }
    let mut flags = HashMap::new();
    for pair in args[1..].chunks(2) {
        if ![
            "--token-file",
            "--account",
            "--chat",
            "--text-file",
            "--request-id",
            "--url",
        ]
        .contains(&pair[0].as_str())
            || flags.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err("unknown or duplicate option".into());
        }
    }
    let required = |name| {
        flags
            .get(name)
            .copied()
            .ok_or_else(|| format!("missing {name}"))
    };
    let token = PathBuf::from(required("--token-file")?);
    let text = solmu::automation_client::bounded_file(
        &PathBuf::from(required("--text-file")?),
        solmu::write_grants::MAX_TEXT_BYTES as u64,
    )?;
    let body = serde_json::json!({"account_id":required("--account")?,"text":text,"request_id":required("--request-id")?});
    let request: solmu::write_grants::SendRequest =
        serde_json::from_value(body.clone()).map_err(|_| "invalid request")?;
    if !request.validate() {
        return Err("invalid or oversized send request".into());
    }
    let client = solmu::automation_client::Client::new(
        flags
            .get("--url")
            .copied()
            .unwrap_or("http://127.0.0.1:23377"),
        token,
    )?;
    let id: String = required("--chat")?
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let (status, result) = client.call("POST", &format!("/v1/chats/{id}/messages"), Some(&body))?;
    println!("{result}");
    if status != 200 {
        return Err(format!(
            "request refused or uncertain (HTTP {status}); do not retry with a new request id"
        ));
    }
    Ok(())
}
