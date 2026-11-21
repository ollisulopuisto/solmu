//! Newline-delimited stdio only. Diagnostics never use stdout.
use std::{
    io::{BufRead, Read, Write},
    path::PathBuf,
};
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.len().is_multiple_of(2) {
        return Err("usage: solmu-mcp --token-file PATH [--url http://127.0.0.1:23377]".into());
    }
    let mut token = None;
    let mut url = None;
    for pair in args.chunks(2) {
        match pair[0].as_str() {
            "--token-file" if token.is_none() => token = Some(PathBuf::from(&pair[1])),
            "--url" if url.is_none() => url = Some(pair[1].as_str()),
            _ => return Err("unknown or duplicate option".into()),
        }
    }
    let client = solmu::automation_client::Client::new(
        url.unwrap_or("http://127.0.0.1:23377"),
        token.ok_or("--token-file required")?,
    )?;
    let mut session = solmu::automation_mcp::Session::default();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    loop {
        let mut line = Vec::new();
        let n = std::io::Read::by_ref(&mut input)
            .take(24 * 1024 + 1)
            .read_until(b'\n', &mut line)
            .map_err(|_| "stdin unreadable")?;
        if n == 0 {
            break;
        }
        if n > 24 * 1024 {
            return Err("MCP request too large".into());
        }
        let response = match serde_json::from_slice(&line) {
            Ok(value) => {
                session.dispatch(value, |method, path, body| client.call(method, path, body))
            }
            Err(_) => Some(
                serde_json::json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let Some(value) = response {
            serde_json::to_writer(&mut out, &value).map_err(|_| "stdout unavailable")?;
            writeln!(&mut out)
                .and_then(|_| out.flush())
                .map_err(|_| "stdout unavailable")?;
        }
    }
    Ok(())
}
