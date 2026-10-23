//! The harness plugin's MCP server (docs/api.md, 「The harness plugin」), spoken to in this process over a pipe as a
//! harness speaks to it, against a control socket standing in for the host's: it lists the four fixed tools and the
//! hosted tools of the session's namespaces exactly, leaves a colliding name out, and calls each tool through.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bmo_pocket_client::Pocket;
use bmo_pocket_plugin::PocketServer;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines, ReadHalf, WriteHalf};
use tokio::net::UnixListener;

/// The token the stand-in holds its callers to.
const TOKEN: &str = "session-token";

/// How long a test lets the server take to answer one request before it fails; a guard, never a wait.
const PATIENCE: Duration = Duration::from_secs(10);

// The stand-in's namespaces, each with the tools it holds: `pocket` holds one whose name would be a fixed tool's,
// and `a_b` and `a` two whose names would be one.
fn namespaces() -> Vec<(&'static str, Vec<&'static str>)> {
  vec![
    ("a", vec!["b_c"]),
    ("a_b", vec!["c"]),
    ("garden", vec!["prune", "water"]),
    ("pocket", vec!["call"]),
  ]
}

fn member(namespace: &str, name: &str) -> Value {
  json!({
    "namespace": namespace, "name": name, "title": null, "description": format!("The {name} tool."),
    "input": { "type": "object", "properties": { "text": { "type": "string" } } }, "output": null, "icon": null,
    "capabilities": [format!("{namespace}.use")], "grant": null,
    "hints": { "readOnly": name == "water", "destructive": false, "idempotent": null, "openWorld": false },
    "listing": null, "system": false, "manual": null, "reachedBy": if name == "water" { "call" } else { "act" },
  })
}

fn refused(status: StatusCode, code: &str, error: &str) -> Response {
  (status, Json(json!({ "code": code, "error": error }))).into_response()
}

fn bearer(headers: &HeaderMap) -> bool {
  headers.get("authorization").and_then(|value| value.to_str().ok()) == Some(&format!("Bearer {TOKEN}"))
}

// The control socket standing in, served on a socket of its own: two pages of namespaces, their tools, and calls
// answered with what they were asked, a `prune` refused for its grant.
async fn stand_in(dir: &TempDir) -> PathBuf {
  let socket = dir.path().join("control.sock");
  let listener = UnixListener::bind(&socket).unwrap();
  let app = axum::Router::new()
    .route(
      "/namespaces",
      get(|headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
        if !bearer(&headers) {
          return refused(StatusCode::UNAUTHORIZED, "unauthorized", "the token names no session");
        }

        let every: Vec<Value> = namespaces()
          .into_iter()
          .map(|(name, tools)| json!({ "name": name, "facts": {}, "toolCount": tools.len() }))
          .collect();
        let (items, more) = match query.get("after") {
          None => (every[..2].to_vec(), true),
          Some(after) => {
            (every.into_iter().filter(|one| one["name"].as_str() > Some(after)).collect(), false)
          }
        };

        Json(json!({ "items": items, "more": more })).into_response()
      }),
    )
    .route(
      "/namespaces/{namespace}/tools",
      get(|headers: HeaderMap, Path(namespace): Path<String>| async move {
        if !bearer(&headers) {
          return refused(StatusCode::UNAUTHORIZED, "unauthorized", "the token names no session");
        }

        let Some((_, tools)) = namespaces().into_iter().find(|(name, _)| *name == namespace) else {
          return refused(StatusCode::NOT_FOUND, "not-found", "no such namespace");
        };
        let items: Vec<Value> = tools.iter().map(|tool| member(&namespace, tool)).collect();

        Json(json!({ "items": items, "more": false })).into_response()
      }),
    )
    .route(
      "/namespaces/{namespace}/tools/{tool}",
      get(|headers: HeaderMap, Path((namespace, tool)): Path<(String, String)>| async move {
        if !bearer(&headers) {
          return refused(StatusCode::UNAUTHORIZED, "unauthorized", "the token names no session");
        }

        Json(member(&namespace, &tool)).into_response()
      }),
    )
    .route(
      "/tool-calls",
      post(|headers: HeaderMap, Json(asked): Json<Value>| async move {
        if !bearer(&headers) {
          return refused(StatusCode::UNAUTHORIZED, "unauthorized", "the token names no session");
        }

        if asked["name"] == "prune" {
          return refused(
            StatusCode::FORBIDDEN,
            "forbidden",
            "the session holds no grant for garden.prune",
          );
        }

        let said = format!(
          "{}/{}:{}",
          asked["namespace"].as_str().unwrap(),
          asked["name"].as_str().unwrap(),
          asked["input"]["text"].as_str().unwrap_or("")
        );

        Json(json!({
          "namespace": asked["namespace"], "name": asked["name"], "spentMs": 1,
          "result": { "content": [{ "type": "text", "text": said }], "isError": false },
        }))
        .into_response()
      }),
    );

  tokio::spawn(async move { axum::serve(listener, app).await });
  socket
}

// The plugin's server spoken to as a harness speaks to it, over a pipe in this process.
struct Harness {
  input: WriteHalf<DuplexStream>,
  output: Lines<BufReader<ReadHalf<DuplexStream>>>,
  id: u64,
}

impl Harness {
  async fn start(token: &str, socket: PathBuf) -> Self {
    let (ours, theirs) = tokio::io::duplex(1 << 20);
    let server = PocketServer::new(Pocket::at(socket, token));

    tokio::spawn(async move {
      if let Ok(running) = server.serve(tokio::io::split(theirs)).await {
        let _ = running.waiting().await;
      }
    });

    let (reading, input) = tokio::io::split(ours);
    let mut harness = Self {
      input,
      output: BufReader::new(reading).lines(),
      id: 0,
    };
    let info = harness
      .ask("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }))
      .await;

    assert_eq!(info["serverInfo"]["name"], "bmo-pocket");
    assert_eq!(info["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    harness.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    harness
  }

  async fn send(&mut self, message: Value) {
    self.input.write_all(format!("{message}\n").as_bytes()).await.unwrap();
    self.input.flush().await.unwrap();
  }

  async fn ask(&mut self, method: &str, params: Value) -> Value {
    self.id += 1;
    self
      .send(json!({ "jsonrpc": "2.0", "id": self.id, "method": method, "params": params }))
      .await;

    loop {
      let line = tokio::time::timeout(PATIENCE, self.output.next_line())
        .await
        .expect("the server answers in time")
        .unwrap()
        .expect("the server answers before it closes");
      let answer: Value = serde_json::from_str(&line).unwrap();

      if answer["id"] == self.id {
        return answer.get("result").cloned().unwrap_or(answer);
      }
    }
  }

  async fn listed(&mut self) -> Vec<Value> {
    self.ask("tools/list", json!({})).await["tools"].as_array().unwrap().clone()
  }

  async fn call(&mut self, tool: &str, arguments: Value) -> Value {
    self.ask("tools/call", json!({ "name": tool, "arguments": arguments })).await
  }
}

fn text(result: &Value) -> String {
  result["content"][0]["text"].as_str().unwrap_or_default().to_owned()
}

#[tokio::test]
async fn it_lists_the_fixed_tools_and_the_hosted_ones_and_leaves_a_colliding_name_out() {
  let dir = tempfile::tempdir().unwrap();
  let socket = stand_in(&dir).await;
  let mut harness = Harness::start(TOKEN, socket).await;
  let listed = harness.listed().await;
  let names: Vec<&str> = listed.iter().map(|tool| tool["name"].as_str().unwrap()).collect();

  assert_eq!(
    names,
    [
      "pocket_namespaces",
      "pocket_tools",
      "pocket_tool",
      "pocket_call",
      "garden_prune",
      "garden_water"
    ]
  );

  for tool in &listed {
    assert!(!tool["description"].as_str().unwrap().is_empty(), "{tool}");
    assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
    assert!(tool["inputSchema"].get("$schema").is_none(), "{tool}");
  }

  let water = listed.iter().find(|tool| tool["name"] == "garden_water").unwrap();

  assert_eq!(water["annotations"]["readOnlyHint"], true);
  assert_eq!(listed[3]["inputSchema"]["required"], json!(["namespace", "name", "input"]));
}

#[tokio::test]
async fn each_tool_calls_its_route_through_the_client_and_a_refusal_is_a_tool_error() {
  let dir = tempfile::tempdir().unwrap();
  let socket = stand_in(&dir).await;
  let mut harness = Harness::start(TOKEN, socket).await;

  let namespaces = harness.call("pocket_namespaces", json!({})).await;

  assert_eq!(namespaces["structuredContent"]["items"][0]["name"], "a", "{namespaces}");
  assert_eq!(namespaces["structuredContent"]["more"], true);

  let tools = harness.call("pocket_tools", json!({ "namespace": "garden" })).await;

  assert_eq!(tools["structuredContent"]["items"][1]["name"], "water", "{tools}");

  let one = harness.call("pocket_tool", json!({ "namespace": "garden", "toolName": "water" })).await;

  assert_eq!(one["structuredContent"]["description"], "The water tool.", "{one}");

  let called = harness
    .call("pocket_call", json!({ "namespace": "a", "name": "b_c", "input": { "text": "x" } }))
    .await;

  assert_eq!(called["structuredContent"]["result"]["content"][0]["text"], "a/b_c:x", "{called}");

  // A hosted tool is one call of its own, answered with the tool's result as it answered it.
  let hosted = harness.call("garden_water", json!({ "text": "rain" })).await;

  assert_eq!(text(&hosted), "garden/water:rain", "{hosted}");
  assert_eq!(hosted["isError"], false);

  let refused = harness.call("garden_prune", json!({ "text": "x" })).await;
  let said: Value = serde_json::from_str(&text(&refused)).unwrap();

  assert_eq!(refused["isError"], true);
  assert_eq!(
    (said["status"].as_u64(), said["code"].as_str()),
    (Some(403), Some("forbidden")),
    "{said}"
  );

  let misread = harness.call("pocket_tool", json!({ "namespace": "garden" })).await;

  assert_eq!(misread["isError"], true);
  assert!(text(&misread).contains("bad-request"), "{misread}");
  assert_eq!(harness.call("nope_tool", json!({})).await["isError"], true);
}

#[tokio::test]
async fn a_token_the_host_refuses_reaches_no_tool() {
  let dir = tempfile::tempdir().unwrap();
  let socket = stand_in(&dir).await;
  let mut harness = Harness::start("another-token", socket).await;
  let refused = harness.call("pocket_namespaces", json!({})).await;
  let said: Value = serde_json::from_str(&text(&refused)).unwrap();

  assert_eq!(refused["isError"], true);
  assert_eq!(said["code"], "unauthorized");
  assert!(
    harness.ask("tools/list", json!({})).await["error"]["message"]
      .as_str()
      .unwrap()
      .contains("unauthorized")
  );
}
