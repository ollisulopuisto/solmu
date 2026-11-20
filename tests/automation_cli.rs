//! Exercise real CLI/stdin framing, and MCP's shared authorization routing.
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn modern(method: &str, params: Value) -> Value {
    let mut p = params;
    p["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":solmu::automation_mcp::MODERN,"io.modelcontextprotocol/clientCapabilities":{}});
    json!({"jsonrpc":"2.0","id":1,"method":method,"params":p})
}
#[test]
fn mcp_lists_only_granted_tools_and_routes_sends_without_retry() {
    let mut session = solmu::automation_mcp::Session::default();
    let response = session
        .dispatch(modern("tools/list", json!({})), |method, path, _| {
            assert_eq!(method, "GET");
            assert_eq!(path, "/v1/capabilities");
            Ok((200, json!({"scopes":["messages:read"]}).to_string()))
        })
        .unwrap();
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "messages");
    let mut calls = 0;
    let result=session.dispatch(modern("tools/call",json!({"name":"send_text","arguments":{"account_id":"a","chat":"!c:x","text":"synthetic","request_id":"stable-1"}})),|method,path,body|{calls+=1;assert_eq!(method,"POST");assert_eq!(path,"/v1/chats/%21c%3Ax/messages");assert_eq!(body.unwrap()["request_id"],"stable-1");Ok((409,json!({"state":"outcome_unknown_do_not_retry"}).to_string()))}).unwrap();
    assert_eq!(calls, 1);
    assert_eq!(result["result"]["isError"], true);
    let result=session.dispatch(modern("tools/call",json!({"name":"send_text","arguments":{"account_id":"a","chat":"c","text":"hi","request_id":"r","files":["secret"]}})),|_,_,_|panic!("unknown arguments never call API")).unwrap();
    assert_eq!(result["error"]["code"], -32602);
}
fn mcp(input: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_solmu-mcp"))
        .args(["--token-file", "unused"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn stdio_discovery_and_legacy_initialization_are_real_processes() {
    let discovery = modern("server/discover", json!({}));
    let output = mcp(format!("{discovery}\n").as_bytes());
    assert!(output.status.success());
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        body["result"]["supportedVersions"][0],
        solmu::automation_mcp::MODERN
    );
    let output=mcp(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\"}}\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(responses[1]["result"], json!({}));
}
#[test]
fn malformed_and_oversized_stdio_never_echo_input() {
    let output = mcp(b"SYNTHETIC-SECRET not json\n");
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("SYNTHETIC-SECRET"));
    let mut large = vec![b'x'; 24 * 1024 + 1];
    large.push(b'\n');
    let output = mcp(&large);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
#[test]
fn cli_unknown_flags_and_non_loopback_never_send() {
    for args in [
        vec!["send", "--unexpected", "x"],
        vec![
            "send",
            "--token-file",
            "none",
            "--account",
            "a",
            "--chat",
            "c",
            "--text-file",
            "none",
            "--request-id",
            "r",
            "--url",
            "http://example.invalid",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_solmu-control"))
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
}
