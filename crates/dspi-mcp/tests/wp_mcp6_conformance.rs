//! WP MCP6: conformance over the real loopback listener, in both spec eras,
//! without Node: what the official conformance suite
//! (`ci/mcp-conformance/run.sh`) found, pinned here so `cargo test` catches a
//! regression, and the shape of every tool, resource and prompt checked in
//! both eras.
#![cfg(feature = "http")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use dspi_core::{CoreSet, DeviceTarget};
use dspi_mcp::http::{HEADER_MISMATCH, HttpConfig, HttpLimits, HttpServer};
use dspi_mcp::protocol::{
    INVALID_PARAMS, LIST_TTL_MS, METHOD_NOT_FOUND, MODERN_VERSIONS, UNSUPPORTED_VERSION,
};
use dspi_mcp::rate::RateLimit;
use dspi_mcp::{AgentPolicy, ConfirmMode, Policy, ReadOnlyPolicy, Server, ServerConfig};
use serde_json::{Value, json};

const MODERN: &str = MODERN_VERSIONS[0];
const LEGACY: &str = "2025-11-25";

struct Http {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Http {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn code(&self) -> Option<i64> {
        self.json()["error"]["code"].as_i64()
    }
}

/// An endpoint over the demo device, and a paired client's token.
struct Endpoint {
    http: HttpServer,
    token: String,
}

fn endpoint(policy: Arc<dyn Policy>) -> Endpoint {
    let set = CoreSet::new();
    let demo = set.open(DeviceTarget::Demo).unwrap();
    // Every tool in a row: the agents' rate limits would answer some calls
    // instead of their tools.
    let unlimited = RateLimit {
        burst: 10_000,
        per_s: 10_000.0,
    };
    let config = ServerConfig {
        policy,
        read_limit: unlimited,
        write_limit: unlimited,
        ..ServerConfig::default()
    };
    let http = HttpServer::start(
        Server::new(set, config),
        HttpConfig {
            port: 0,
            store_dir: None,
            limits: HttpLimits::default(),
        },
        None,
    )
    .unwrap();
    http.with_server(|s| s.set_active_device(Some(demo)));
    let token = http.add_client("wp-mcp6").unwrap().token;
    Endpoint { http, token }
}

impl Endpoint {
    fn post(&self, headers: &[(&str, &str)], body: &Value) -> Http {
        let port = self.http.port();
        let body = body.to_string();
        let mut head = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
             Authorization: Bearer {}\r\nContent-Length: {}\r\n",
            self.token,
            body.len()
        );
        for (n, v) in headers {
            head.push_str(&format!("{n}: {v}\r\n"));
        }
        head.push_str("\r\n");
        head.push_str(&body);
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        s.write_all(head.as_bytes()).unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out);
        let text = String::from_utf8_lossy(&out).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
            .collect();
        Http {
            status,
            headers,
            body: body.to_owned(),
        }
    }

    /// A modern request with the headers its body calls for.
    fn modern(&self, method: &str, mut params: Value) -> Http {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "wp-mcp6", "version": "1"},
        });
        let name = params
            .get("name")
            .or_else(|| params.get("uri"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut h = vec![("MCP-Protocol-Version", MODERN), ("Mcp-Method", method)];
        if let Some(n) = &name {
            h.push(("Mcp-Name", n.as_str()));
        }
        self.post(
            &h,
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
    }

    /// A legacy session: `initialize`, then `notifications/initialized`.
    fn session(&self) -> String {
        let r = self.post(
            &[],
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": LEGACY, "capabilities": {},
                "clientInfo": {"name": "wp-mcp6", "version": "1"}}}),
        );
        assert_eq!(r.status, 200, "{}", r.body);
        let session = r.header("Mcp-Session-Id").unwrap().to_owned();
        let r = self.legacy_raw(
            &session,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        );
        assert_eq!(r.status, 202);
        session
    }

    fn legacy_raw(&self, session: &str, body: &Value) -> Http {
        self.post(
            &[
                ("Mcp-Session-Id", session),
                ("MCP-Protocol-Version", LEGACY),
            ],
            body,
        )
    }

    fn legacy(&self, session: &str, method: &str, params: Value) -> Http {
        self.legacy_raw(
            session,
            &json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params}),
        )
    }
}

fn agent() -> Arc<dyn Policy> {
    Arc::new(AgentPolicy::new(ConfirmMode::AskRisky))
}

/// The suite's sep-2575-request-meta-invalid-*: a request that says
/// 2026-07-28 in its header but lacks the `_meta` (or its version) is invalid
/// params, not an unsupported version.
#[test]
fn wp_mcp6_a_modern_request_without_its_meta_is_invalid_params() {
    let e = endpoint(agent());
    let h = [
        ("MCP-Protocol-Version", MODERN),
        ("Mcp-Method", "server/discover"),
    ];
    for params in [
        json!({}),
        json!({"_meta": {"io.modelcontextprotocol/clientCapabilities": {}}}),
        json!({"_meta": {"io.modelcontextprotocol/protocolVersion": MODERN}}),
    ] {
        let r = e.post(
            &h,
            &json!({"jsonrpc": "2.0", "id": 101, "method": "server/discover", "params": params}),
        );
        assert_eq!(r.status, 400, "{params}: {}", r.body);
        assert_eq!(r.code(), Some(INVALID_PARAMS), "{params}: {}", r.body);
        assert_eq!(r.json()["id"], 101, "the error carries the request's id");
    }
}

/// The suite's sep-2575-http-server-header-mismatch-400: a header naming
/// another version than the body is a mismatch first; an unsupported version
/// in both is -32022.
#[test]
fn wp_mcp6_a_header_that_contradicts_the_body_is_a_mismatch_before_the_version() {
    let e = endpoint(agent());
    let body = |v: &str| {
        json!({"jsonrpc": "2.0", "id": 302, "method": "server/discover", "params": {"_meta": {
            "io.modelcontextprotocol/protocolVersion": v,
            "io.modelcontextprotocol/clientCapabilities": {}}}})
    };
    let r = e.post(
        &[
            ("MCP-Protocol-Version", MODERN),
            ("Mcp-Method", "server/discover"),
        ],
        &body("v999.0.0"),
    );
    assert_eq!(
        (r.status, r.code()),
        (400, Some(HEADER_MISMATCH)),
        "{}",
        r.body
    );
    let r = e.post(
        &[
            ("MCP-Protocol-Version", "v999.0.0"),
            ("Mcp-Method", "server/discover"),
        ],
        &body("v999.0.0"),
    );
    assert_eq!(
        (r.status, r.code()),
        (400, Some(UNSUPPORTED_VERSION)),
        "{}",
        r.body
    );
    assert_eq!(r.json()["error"]["data"]["requested"], "v999.0.0");
    // No header at all: the body's version is judged.
    let r = e.post(&[], &body("v999.0.0"));
    assert_eq!(r.code(), Some(UNSUPPORTED_VERSION), "{}", r.body);
}

/// The suite's sep-2575-http-server-method-not-found-404-*: 2026-07-28 removed
/// `initialize` and `ping`; the legacy era keeps both.
#[test]
fn wp_mcp6_initialize_and_ping_are_not_modern_methods() {
    let e = endpoint(agent());
    for method in [
        "initialize",
        "ping",
        "logging/setLevel",
        "resources/subscribe",
    ] {
        let r = e.modern(method, json!({}));
        assert_eq!(
            (r.status, r.code()),
            (404, Some(METHOD_NOT_FOUND)),
            "{method}: {}",
            r.body
        );
        assert!(r.header("Mcp-Session-Id").is_none(), "{method}");
    }
    let session = e.session();
    let r = e.legacy(&session, "ping", json!({}));
    assert_eq!((r.status, r.json()["result"].clone()), (200, json!({})));
}

/// The suite's caching scenario: 2026-07-28's resources/read carries
/// `ttlMs` and `cacheScope`. The fixed resources may be kept a minute, a
/// device's state not at all; the legacy era has neither field.
#[test]
fn wp_mcp6_resources_read_says_how_long_it_may_be_kept() {
    let e = endpoint(agent());
    let state = e.modern("resources/list", json!({})).json()["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["uri"].as_str())
        .find(|u| u.starts_with("dspi://device/"))
        .unwrap()
        .to_owned();
    for (uri, ttl) in [
        ("dspi://guide", LIST_TTL_MS),
        ("dspi://filter-types", LIST_TTL_MS),
        (state.as_str(), 0),
    ] {
        let r = e.modern("resources/read", json!({"uri": uri}));
        let result = &r.json()["result"];
        assert_eq!(result["ttlMs"], json!(ttl), "{uri}: {}", r.body);
        assert_eq!(result["cacheScope"], "private", "{uri}");
        assert_eq!(result["resultType"], "complete", "{uri}");
    }
    let session = e.session();
    let r = e.legacy(&session, "resources/read", json!({"uri": "dspi://guide"}));
    let result = &r.json()["result"];
    assert!(result.get("ttlMs").is_none() && result.get("cacheScope").is_none());
}

/// SEP-2164: "not found" is -32602 and names the URI (cleaned) in `data`.
#[test]
fn wp_mcp6_a_missing_resource_names_its_uri() {
    let e = endpoint(agent());
    let uri = "dspi://device/nobody/state";
    let r = e.modern("resources/read", json!({"uri": uri}));
    assert_eq!(r.code(), Some(INVALID_PARAMS), "{}", r.body);
    assert_eq!(r.json()["error"]["data"]["uri"], uri);
    assert_eq!(r.json()["id"], 1);
    let session = e.session();
    let r = e.legacy(&session, "resources/read", json!({"uri": "x\u{202e}y"}));
    assert_eq!(r.json()["error"]["data"]["uri"], "xy", "{}", r.body);
}

fn listed(r: &Http, key: &str) -> Vec<Value> {
    r.json()["result"][key]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Every tool, resource and prompt is listed alike in both eras, and has the
/// shape the specification asks for: tool names of 1–128 of
/// `[A-Za-z0-9_.-]`, an object input schema whose `required` names its
/// properties, an object output schema, a description and annotations.
#[test]
fn wp_mcp6_both_eras_list_the_same_well_formed_tools_resources_and_prompts() {
    let e = endpoint(agent());
    let session = e.session();
    for (method, key) in [
        ("tools/list", "tools"),
        ("resources/list", "resources"),
        ("resources/templates/list", "resourceTemplates"),
        ("prompts/list", "prompts"),
    ] {
        let modern = e.modern(method, json!({}));
        let legacy = e.legacy(&session, method, json!({}));
        assert_eq!(modern.status, 200, "{method}: {}", modern.body);
        assert!(!listed(&modern, key).is_empty(), "{method}");
        assert_eq!(listed(&modern, key), listed(&legacy, key), "{method}");
        assert_eq!(modern.json()["result"]["ttlMs"], json!(LIST_TTL_MS));
        assert!(legacy.json()["result"].get("resultType").is_none());
    }
    let tools = listed(&e.legacy(&session, "tools/list", json!({})), "tools");
    let mut names = Vec::new();
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        names.push(name);
        assert!(
            !name.is_empty()
                && name.len() <= 128
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
            "{name}"
        );
        assert!(
            !t["description"].as_str().unwrap_or("").is_empty(),
            "{name}"
        );
        let input = &t["inputSchema"];
        assert_eq!(input["type"], "object", "{name}");
        let props = input["properties"].as_object();
        assert!(props.is_some(), "{name}: properties");
        for r in input["required"].as_array().into_iter().flatten() {
            let r = r.as_str().unwrap();
            assert!(props.unwrap().contains_key(r), "{name}: required {r}");
        }
        if let Some(out) = t.get("outputSchema") {
            assert_eq!(out["type"], "object", "{name}: outputSchema");
        }
        assert!(
            t["annotations"]["readOnlyHint"].is_boolean(),
            "{name}: readOnlyHint"
        );
    }
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "tool names are unique");
    // The link tools are in (feature `link`, as the app builds it).
    #[cfg(feature = "link")]
    assert!(names.contains(&"dspi_installation_view"));
}

/// Every read tool, called with no arguments in each era, gives a result
/// (perhaps a tool error that says what is missing), never a protocol error:
/// text content, `isError`, a structured object, and `resultType` in the
/// modern era.
#[test]
fn wp_mcp6_every_read_tool_answers_in_both_eras() {
    let e = endpoint(agent());
    let session = e.session();
    let tools = listed(&e.legacy(&session, "tools/list", json!({})), "tools");
    let reads: Vec<&str> = tools
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == true)
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(reads.len() >= 30, "{}", reads.len());
    for name in reads {
        let call = json!({"name": name, "arguments": {}});
        for (era, r) in [
            ("modern", e.modern("tools/call", call.clone())),
            ("legacy", e.legacy(&session, "tools/call", call.clone())),
        ] {
            assert_eq!(r.status, 200, "{era} {name}: {}", r.body);
            let result = &r.json()["result"];
            assert!(result["isError"].is_boolean(), "{era} {name}: {}", r.body);
            let content = result["content"].as_array().unwrap();
            assert!(
                !content.is_empty()
                    && content
                        .iter()
                        .all(|c| c["type"] == "text" && c["text"].is_string()),
                "{era} {name}"
            );
            if let Some(s) = result.get("structuredContent") {
                assert!(s.is_object(), "{era} {name}: structuredContent");
            }
            assert_eq!(
                result.get("resultType").is_some(),
                era == "modern",
                "{era} {name}"
            );
        }
    }
}

/// Arguments of the wrong type are the tool's error (the model can correct
/// itself: 2025-11-25 "Error Handling"), for every tool; an unknown tool is
/// a protocol error. Under the read-only policy nothing is written whatever a
/// tool makes of them.
#[test]
fn wp_mcp6_wrong_arguments_are_a_tool_error_for_every_tool() {
    let e = endpoint(Arc::new(ReadOnlyPolicy));
    let tools = listed(&e.modern("tools/list", json!({})), "tools");
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        let r = e.modern(
            "tools/call",
            json!({"name": name, "arguments": {"device": 5, "output": "x"}}),
        );
        assert_eq!(r.status, 200, "{name}: {}", r.body);
        assert_eq!(r.json()["result"]["isError"], true, "{name}: {}", r.body);
    }
    let r = e.modern(
        "tools/call",
        json!({"name": "dspi_no_such_tool", "arguments": {}}),
    );
    assert_eq!(r.code(), Some(INVALID_PARAMS), "{}", r.body);
    let history = e.modern(
        "tools/call",
        json!({"name": "dspi_get_history", "arguments": {}}),
    );
    let entries = &history.json()["result"]["structuredContent"]["entries"];
    assert!(
        entries.as_array().is_some_and(Vec::is_empty),
        "nothing was written: {entries}"
    );
}

/// Every prompt can be got in both eras with its required arguments, and a
/// missing one is -32602.
#[test]
fn wp_mcp6_every_prompt_is_got_in_both_eras() {
    let e = endpoint(agent());
    let session = e.session();
    let prompts = listed(&e.modern("prompts/list", json!({})), "prompts");
    for p in &prompts {
        let name = p["name"].as_str().unwrap();
        let required: Vec<&str> = p["arguments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| a["required"] == true)
            .filter_map(|a| a["name"].as_str())
            .collect();
        let args: serde_json::Map<String, Value> = required
            .iter()
            .map(|a| ((*a).to_owned(), json!("0")))
            .collect();
        for (era, r) in [
            (
                "modern",
                e.modern("prompts/get", json!({"name": name, "arguments": args})),
            ),
            (
                "legacy",
                e.legacy(
                    &session,
                    "prompts/get",
                    json!({"name": name, "arguments": args}),
                ),
            ),
        ] {
            let messages = r.json()["result"]["messages"].as_array().cloned();
            assert!(
                messages.is_some_and(|m| !m.is_empty()),
                "{era} {name}: {}",
                r.body
            );
        }
        if let Some(first) = required.first() {
            let mut fewer = args.clone();
            fewer.remove(*first);
            let r = e.modern("prompts/get", json!({"name": name, "arguments": fewer}));
            assert_eq!(r.code(), Some(INVALID_PARAMS), "{name}: {}", r.body);
        }
    }
}
