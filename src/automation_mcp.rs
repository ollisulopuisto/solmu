//! MCP stdio dispatcher. Modern per-request metadata and explicit legacy initialization.
use serde_json::{Value, json};
pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &str = "2025-11-25";
#[derive(Default)]
pub struct Session {
    legacy: bool,
    ready: bool,
}
fn err(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn tool(name: &str, scope: &str, properties: Value, required: Vec<&str>) -> (String, Value) {
    (
        scope.into(),
        json!({"name":name,"description":if scope=="messages:send"{"Send one text to an explicitly granted destination. Accepted is not delivered. Use a stable request_id; unknown outcomes must not be retried with a different id."}else{"Read through the scoped local API. Upstream read-state side effects are not live-verified."},"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":scope!="messages:send","destructiveHint":false,"idempotentHint":scope!="messages:send","openWorldHint":true}}),
    )
}
fn definitions() -> Vec<(String, Value)> {
    vec![
        tool("accounts", "accounts:read", json!({}), vec![]),
        tool("chats", "chats:read", json!({}), vec![]),
        tool(
            "messages",
            "messages:read",
            json!({"chat":{"type":"string"}}),
            vec!["chat"],
        ),
        tool(
            "search",
            "search:read",
            json!({"query":{"type":"string"}}),
            vec!["query"],
        ),
        tool("people", "people:read", json!({}), vec![]),
        tool(
            "send_text",
            "messages:send",
            json!({"account_id":{"type":"string"},"chat":{"type":"string"},"text":{"type":"string","maxLength":16384},"request_id":{"type":"string","maxLength":80}}),
            vec!["account_id", "chat", "text", "request_id"],
        ),
    ]
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
impl Session {
    pub fn dispatch(
        &mut self,
        input: Value,
        mut call: impl FnMut(&str, &str, Option<&Value>) -> Result<(u16, String), String>,
    ) -> Option<Value> {
        if !input.is_object()
            || input.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || !input.get("method").is_some_and(Value::is_string)
        {
            return Some(err(Value::Null, -32600, "Invalid request"));
        }
        let id = input.get("id").cloned();
        let method = input.get("method").and_then(Value::as_str).unwrap_or("");
        if id.is_none() {
            if method == "notifications/initialized" && self.legacy {
                self.ready = true;
            }
            return None;
        }
        let id = id.unwrap();
        if input.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || !(id.is_string() || id.is_i64() || id.is_u64())
        {
            return Some(err(Value::Null, -32600, "Invalid request"));
        }
        let empty = json!({});
        let params = input.get("params").unwrap_or(&empty);
        if method == "initialize" {
            if self.legacy {
                return Some(err(id, -32600, "Already initialized"));
            }
            if params.get("protocolVersion").and_then(Value::as_str) != Some(LEGACY) {
                return Some(err(
                    id,
                    -32602,
                    "Legacy compatibility supports 2025-11-25; modern clients use 2026-07-28 metadata",
                ));
            }
            self.legacy = true;
            return Some(
                json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":LEGACY,"capabilities":{"tools":{}},"serverInfo":{"name":"solmu-mcp","version":env!("CARGO_PKG_VERSION")}}}),
            );
        }
        let modern = params
            .get("_meta")
            .and_then(|m| m.get("io.modelcontextprotocol/protocolVersion"))
            .and_then(Value::as_str);
        if modern.is_some() || !self.legacy {
            let Some(version) = modern else {
                return Some(err(id, -32602, "Required per-request metadata missing"));
            };
            if version != MODERN {
                return Some(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":[MODERN],"requested":version}}}),
                );
            }
            if !params
                .get("_meta")
                .and_then(|m| m.get("io.modelcontextprotocol/clientCapabilities"))
                .is_some_and(Value::is_object)
            {
                return Some(err(id, -32602, "Client capabilities missing"));
            }
        } else if !self.ready && method != "ping" {
            return Some(err(id, -32600, "Initialization not complete"));
        }
        let modern = modern.is_some();
        let result = match method {
            "server/discover" => {
                json!({"resultType":"complete","supportedVersions":[MODERN,LEGACY],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"solmu-mcp","version":env!("CARGO_PKG_VERSION")}}})
            }
            "ping" => json!({}),
            "tools/list" => {
                let Ok((200, body)) = call("GET", "/v1/capabilities", None) else {
                    return Some(err(id, -32000, "Local API authorization unavailable"));
                };
                let scopes: Vec<String> = serde_json::from_str::<Value>(&body)
                    .ok()
                    .and_then(|v| serde_json::from_value(v["scopes"].clone()).ok())
                    .unwrap_or_default();
                json!({"tools":definitions().into_iter().filter(|(scope,_)|scopes.contains(scope)).map(|(_,tool)|tool).collect::<Vec<_>>()})
            }
            "tools/call" => {
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return Some(err(id, -32602, "Tool name required"));
                };
                let Some(def) = definitions()
                    .into_iter()
                    .find(|(_, d)| d["name"] == name)
                    .map(|(_, d)| d)
                else {
                    return Some(err(id, -32602, "Unknown tool"));
                };
                let args = params.get("arguments").unwrap_or(&empty);
                let Some(object) = args.as_object() else {
                    return Some(err(id, -32602, "Arguments must be an object"));
                };
                let properties = def["inputSchema"]["properties"].as_object().unwrap();
                if object
                    .iter()
                    .any(|(k, v)| !properties.contains_key(k) || !v.is_string())
                    || def["inputSchema"]["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|k| !object.contains_key(k.as_str().unwrap()))
                {
                    return Some(err(id, -32602, "Invalid tool arguments"));
                }
                let get = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
                let path = match name {
                    "accounts" => "/v1/accounts".into(),
                    "chats" => "/v1/chats".into(),
                    "people" => "/v1/people".into(),
                    "messages" | "send_text" => {
                        format!("/v1/chats/{}/messages", encode(get("chat")))
                    }
                    "search" => format!("/v1/messages/search?query={}", encode(get("query"))),
                    _ => unreachable!(),
                };
                let body = json!({"account_id":get("account_id"),"text":get("text"),"request_id":get("request_id")});
                let result = call(
                    if name == "send_text" { "POST" } else { "GET" },
                    &path,
                    (name == "send_text").then_some(&body),
                );
                match result {
                    Ok((status, body)) => {
                        json!({"content":[{"type":"text","text":body}],"isError":status!=200})
                    }
                    Err(_) => {
                        json!({"content":[{"type":"text","text":"Local API unavailable. A write may have unknown outcome; never retry with a new request id."}],"isError":true})
                    }
                }
            }
            _ => return Some(err(id, -32601, "Method not found")),
        };
        let mut result = result;
        if modern && method != "ping" {
            result["resultType"] = json!("complete");
        }
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_metadata_or_unknown_tools_never_call_api() {
        let mut s = Session::default();
        let r = s
            .dispatch(
                json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
                |_, _, _| panic!("must not call"),
            )
            .unwrap();
        assert_eq!(r["error"]["code"], -32602);
        let r=s.dispatch(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"delete","_meta":{"io.modelcontextprotocol/protocolVersion":MODERN,"io.modelcontextprotocol/clientCapabilities":{}}}}),|_,_,_|panic!("must not call")).unwrap();
        assert_eq!(r["error"]["code"], -32602);
    }
}
