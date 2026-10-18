//! The tool host as a session meets it: the binary started on a scratch data root, tokens issued on
//! its control socket, and the protocol spoken to it over HTTP (docs/mcp-host.md, 「Tests」).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::routing::post;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::unix_http::request;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream};

// A host running on its own data root, with its sockets in a directory short enough to bind.
struct Running {
  child: Child,
  port: u16,
  control: PathBuf,
}

impl Drop for Running {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

struct Scratch {
  data: TempDir,
  sockets: TempDir,
}

impl Scratch {
  fn new() -> Self {
    let scratch = Self {
      data: tempfile::tempdir().unwrap(),
      sockets: tempfile::Builder::new().prefix("mh").tempdir_in("/tmp").unwrap(),
    };

    fs::create_dir_all(scratch.calls()).unwrap();
    scratch
  }

  // Where a stand-in tool keeps what each request handed it.
  fn calls(&self) -> PathBuf {
    self.data.path().join("calls")
  }

  // The file the account service's token is read from.
  fn token_file(&self) -> PathBuf {
    let file = self.data.path().join("accounts.token");

    fs::write(&file, format!("{ACCOUNTS_TOKEN}\n")).unwrap();
    file
  }

  // A host asking the account service standing at `accounts`, with the switch and anything more given.
  fn start_asking(&self, accounts: &str, more: &[&str]) -> Running {
    let token = self.token_file();
    let mut args = vec!["--accounts", accounts, "--accounts-token-file", token.to_str().unwrap()];

    args.extend_from_slice(more);
    self.start_with(&args)
  }

  // The deploy tree's `tools/` standing in, which the host reads and watches.
  fn tools(&self) -> PathBuf {
    self.data.path().join("tools")
  }

  // What the host said on its standard error.
  fn log(&self) -> String {
    fs::read_to_string(self.data.path().join("host.log")).unwrap_or_default()
  }

  fn start(&self) -> Running {
    self.start_with(&[])
  }

  fn start_with(&self, more: &[&str]) -> Running {
    let control = self.sockets.path().join("control.sock");
    let mut child = Command::new(env!("CARGO_BIN_EXE_mcp-host"))
      .args(["serve", "--port", "0"])
      .args(more)
      .arg("--data-root")
      .arg(self.data.path())
      .arg("--control")
      .arg(&control)
      .arg("--tools")
      .arg(self.tools())
      .args(["--settle-ms", "50"])
      .stdout(Stdio::piped())
      .stderr(fs::File::create(self.data.path().join("host.log")).unwrap())
      .spawn()
      .unwrap();
    let mut line = String::new();

    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();

    let port = line
      .split("127.0.0.1:")
      .nth(1)
      .and_then(|rest| rest.split(',').next())
      .and_then(|port| port.parse().ok())
      .unwrap();

    Running {
      child,
      port,
      control,
    }
  }
}

fn now_ms() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

// A checkout whose graph holds one node of its own name.
fn checkout(name: &str) -> TempDir {
  let dir = tempfile::tempdir().unwrap();

  fs::create_dir_all(dir.path().join("graph")).unwrap();
  fs::write(
    dir.path().join("graph/area.json"),
    json!([{ "id": format!("area/{name}"), "title": name, "what": "-", "files": [], "symbols": [], "aliases": [], "needs": [], "base": "-" }]).to_string(),
  )
  .unwrap();

  dir
}

// The graph's, the queue's and the desk's tools standing in, placed where the host reads them
// (docs/mcp-host.md, 「Placed tools」), each entry by its own name and capability: a graph verb answers
// with the one node of its session's checkout, the desk's entry with its session's desk.
const READERS_MANIFEST: &str = r#"
[tool]
name = "readers"
protocol = 1
language = "python"
run = ["python3", "readers.py"]
timeout = 20

[[entry]]
namespace = "worker"
name = "graph_search"
description = "Finds the features a word is about, as id and title."
input = "any.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "graph_get"
description = "One feature."
input = "any.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "graph_list"
description = "Asked with an area, that area's features. Asked with none, the areas."
input = "any.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "graph_impact"
description = "What else a change to one feature would reach."
input = "any.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "todo_list"
description = "The work queue in its own order."
input = "any.json"
capabilities = ["queue.read"]
hints = { read_only = true }

[[entry]]
namespace = "fragments"
name = "fragment_owed"
description = "The pieces still owed a row."
input = "any.json"
capabilities = ["fragments.desk"]
hints = { read_only = true }
"#;

const READERS_SCRIPT: &str = r#"
import json, os, sys

handed = json.load(sys.stdin)
session = handed["session"]

if handed["entry"].startswith("fragments/"):
    said = "owed on " + session["fragmentDesk"]
else:
    with open(os.path.join(session["checkout"], "graph", "area.json")) as graph:
        node = json.load(graph)[0]
    said = node["id"] + "  " + node["title"]

print(json.dumps({"content": [{"type": "text", "text": said}], "isError": False}))
"#;

// The stand-in readers placed in the tree a host about to start reads.
fn place_readers(scratch: &Scratch) {
  place(
    &scratch.tools(),
    "readers",
    &[
      ("tool.toml", READERS_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("readers.py", READERS_SCRIPT),
    ],
  );
}

// The entries a line's sessions are shown by name, as the issuer of their records resolves them: the worker
// line three graph verbs, the chat line a filing and a reading entry, and every other line none.
fn shown_on(line: &str) -> Value {
  match line {
    "worker" => json!(["worker/graph_get", "worker/graph_impact", "worker/graph_search"]),
    "chat" => json!(["bmo/running", "bmo/todo_add"]),
    _ => json!([]),
  }
}

// A record as its issuer writes it, shown by name what its line's sessions are unless `more` says otherwise.
fn record(data: &Path, capabilities: &[&str], more: Value) -> Value {
  let mut said = json!({ "line": "worker", "capabilities": capabilities, "dataRoot": data });

  said.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());

  if said.get("shown").is_none() {
    said["shown"] = shown_on(said["line"].as_str().unwrap());
  }

  said
}

// A namespace's listing as the control socket answers it.
async fn listing(host: &Running, namespace: &str) -> (StatusCode, String) {
  let reply = request(&host.control, Method::GET, &format!("/listings/{namespace}"), None)
    .await
    .unwrap();

  (reply.status, String::from_utf8_lossy(&reply.body).into_owned())
}

async fn issue(host: &Running, session: &Value) -> String {
  let reply = request(&host.control, Method::POST, "/sessions", Some(session)).await.unwrap();

  assert_eq!(reply.status, StatusCode::OK, "{}", String::from_utf8_lossy(&reply.body));
  reply.json().unwrap()["token"].as_str().unwrap().to_owned()
}

// One JSON-RPC request to a namespace, with the token where one is given.
async fn rpc(
  host: &Running,
  token: Option<&str>,
  namespace: &str,
  method: &str,
  params: Value,
) -> (StatusCode, Value) {
  rpc_on("2025-06-18", host, token, namespace, method, params).await
}

// The same request from a client on another revision of the protocol.
async fn rpc_on(
  revision: &str,
  host: &Running,
  token: Option<&str>,
  namespace: &str,
  method: &str,
  params: Value,
) -> (StatusCode, Value) {
  let stream = TcpStream::connect(("127.0.0.1", host.port)).await.unwrap();
  let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let mut params = params;

  // From the 2026 revision a request stands alone and says who asks in its own `_meta`.
  if revision.starts_with("2026") {
    params["_meta"] = json!({ "io.modelcontextprotocol/protocolVersion": revision, "io.modelcontextprotocol/clientCapabilities": {} });
  }

  let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
  let mut asked = Request::post(format!("/mcp/{namespace}"))
    .header("host", "127.0.0.1")
    .header("content-type", "application/json")
    .header("accept", "application/json, text/event-stream")
    .header("mcp-protocol-version", revision)
    .header("mcp-method", method);

  if let Some(name) = params["name"].as_str() {
    asked = asked.header("mcp-name", name);
  }

  if let Some(token) = token {
    asked = asked.header("authorization", format!("Bearer {token}"));
  }

  let response = sender
    .send_request(asked.body(Full::new(Bytes::from(body.to_string()))).unwrap())
    .await
    .unwrap();
  let status = response.status();
  let bytes = response.into_body().collect().await.unwrap().to_bytes();

  (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn called(host: &Running, token: &str, namespace: &str, tool: &str, arguments: Value) -> Value {
  rpc(
    host,
    Some(token),
    namespace,
    "tools/call",
    json!({ "name": tool, "arguments": arguments }),
  )
  .await
  .1
}

fn text(answer: &Value) -> String {
  answer["result"]["content"][0]["text"].as_str().unwrap_or_default().to_owned()
}

async fn listed(host: &Running, token: &str, namespace: &str) -> Vec<String> {
  let (_, answer) = rpc(host, Some(token), namespace, "tools/list", json!({})).await;

  answer["result"]["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|tool| tool["name"].as_str().unwrap().to_owned())
    .collect()
}

#[tokio::test]
async fn two_sessions_each_read_their_own_checkout_and_desk_through_one_host() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let (left, right) = (checkout("left"), checkout("right"));
  let desks = tempfile::tempdir().unwrap();
  let (left_desk, right_desk) = (desks.path().join("left-desk.json"), desks.path().join("right-desk.json"));
  let one = issue(
    &host,
    &record(
      scratch.data.path(),
      &["graph.read", "fragments.desk"],
      json!({ "checkout": left.path(), "fragmentDesk": left_desk }),
    ),
  )
  .await;
  let other = issue(
    &host,
    &record(
      scratch.data.path(),
      &["graph.read", "fragments.desk"],
      json!({ "checkout": right.path(), "fragmentDesk": right_desk }),
    ),
  )
  .await;

  assert_eq!(
    text(&called(&host, &one, "worker", "graph_search", json!({ "text": "area" })).await),
    "area/left  left"
  );
  assert_eq!(
    text(&called(&host, &other, "worker", "graph_search", json!({ "text": "area" })).await),
    "area/right  right"
  );

  let owed = text(&called(&host, &one, "fragments", "fragment_owed", json!({})).await);

  assert!(owed.contains("left-desk.json") && !owed.contains("right-desk.json"), "{owed}");
}

#[tokio::test]
async fn a_session_is_shown_and_may_call_only_what_its_capabilities_open() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let graph = checkout("only");
  let token = issue(
    &host,
    &record(scratch.data.path(), &["graph.read"], json!({ "checkout": graph.path() })),
  )
  .await;

  // The booked one of the four is the toolbook's to list.
  assert_eq!(listed(&host, &token, "worker").await, ["graph_get", "graph_impact", "graph_search"]);
  assert!(listed(&host, &token, "fragments").await.is_empty());

  assert!(!text(&called(&host, &token, "worker", "graph_list", json!({})).await).is_empty());

  let refused = called(&host, &token, "worker", "todo_list", json!({})).await;

  assert_eq!(refused["error"]["code"], -32602);
  assert_eq!(refused["error"]["message"], "Tool todo_list not found");
}

// "An entry is resident for a session where its record's `shown` names it" (docs/toolbook.md, 「Which tools are
// resident」): a worker's record names three graph verbs, and a searcher's names none.
#[tokio::test]
async fn a_placed_entry_is_listed_where_its_sessions_record_shows_it_and_booked_where_it_does_not() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let graph = checkout("lines");
  let on = |line: &str| {
    record(scratch.data.path(), &["graph.read"], json!({ "line": line, "checkout": graph.path() }))
  };
  let worker = issue(&host, &on("worker")).await;
  let searcher = issue(&host, &on("search")).await;

  assert_eq!(listed(&host, &worker, "worker").await, ["graph_get", "graph_impact", "graph_search"]);
  assert!(!book(&host, &worker).await.contains("worker/graph_search"));
  assert!(listed(&host, &searcher, "worker").await.is_empty());

  let booked = book(&host, &searcher).await;

  for entry in ["graph_get", "graph_impact", "graph_list", "graph_search"] {
    assert!(
      booked.lines().any(|line| line.starts_with(&format!("worker/{entry} · call — "))),
      "{booked}"
    );
  }

  assert_eq!(
    text(&through(&host, &searcher, "call", "worker/graph_search", json!({ "text": "lines" })).await),
    "area/lines  lines"
  );
  assert_eq!(
    text(&called(&host, &worker, "worker", "graph_search", json!({ "text": "lines" })).await),
    "area/lines  lines"
  );
}

#[tokio::test]
async fn a_request_with_no_token_an_unknown_one_or_an_unknown_namespace_is_turned_away() {
  let scratch = Scratch::new();
  let host = scratch.start();
  let token = issue(&host, &record(scratch.data.path(), &["graph.read"], json!({}))).await;

  assert_eq!(rpc(&host, None, "worker", "tools/list", json!({})).await.0, StatusCode::UNAUTHORIZED);
  assert_eq!(
    rpc(&host, Some("nope"), "worker", "tools/list", json!({})).await.0,
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(
    rpc(&host, Some(&token), "graph", "tools/list", json!({})).await.0,
    StatusCode::NOT_FOUND
  );
}

#[tokio::test]
async fn a_record_that_is_not_one_is_refused_where_it_is_issued() {
  let scratch = Scratch::new();
  let host = scratch.start();
  let reply =
    request(&host.control, Method::POST, "/sessions", Some(&json!({ "line": "worker", "room": 5 })))
      .await
      .unwrap();

  assert_eq!(reply.status, StatusCode::BAD_REQUEST);
  assert_eq!(reply.json().unwrap()["code"], "mcp-host.malformed-record");
}

#[tokio::test]
async fn a_revoked_token_and_a_lapsed_run_reach_nothing_and_leave_no_record() {
  let scratch = Scratch::new();
  let host = scratch.start();
  let revoked = issue(&host, &record(scratch.data.path(), &["graph.read"], json!({}))).await;
  let lapsed = issue(
    &host,
    &record(scratch.data.path(), &["graph.read"], json!({ "deadline": now_ms() + 300 })),
  )
  .await;
  let sessions = scratch.data.path().join("tool-host/sessions");

  assert_eq!(fs::read_dir(&sessions).unwrap().count(), 2);
  assert_eq!(
    request(&host.control, Method::DELETE, &format!("/sessions/{revoked}"), None)
      .await
      .unwrap()
      .status,
    StatusCode::NO_CONTENT
  );
  assert_eq!(
    rpc(&host, Some(&revoked), "worker", "tools/list", json!({})).await.0,
    StatusCode::UNAUTHORIZED
  );

  tokio::time::sleep(Duration::from_millis(400)).await;

  assert_eq!(
    rpc(&host, Some(&lapsed), "worker", "tools/list", json!({})).await.0,
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(fs::read_dir(&sessions).unwrap().count(), 0);
}

// The revision the installed Claude Code negotiates refuses a result that does not say it is
// complete, and loads no tool from it (measured with Claude Code 2.1.289).
#[tokio::test]
async fn a_client_on_the_2026_revision_is_told_each_result_is_complete_and_an_earlier_one_is_not() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let graph = checkout("told");
  let token = issue(
    &host,
    &record(scratch.data.path(), &["graph.read"], json!({ "checkout": graph.path() })),
  )
  .await;
  let search = json!({ "name": "graph_search", "arguments": { "text": "told" } });

  for method in ["tools/list", "resources/list"] {
    let (_, listed) = rpc_on("2026-07-28", &host, Some(&token), "worker", method, json!({})).await;

    assert_eq!(listed["result"]["resultType"], "complete", "{method}: {listed}");
    assert_eq!(
      (&listed["result"]["ttlMs"], &listed["result"]["cacheScope"]),
      (&json!(0), &json!("private")),
      "{method}: {listed}"
    );
  }

  let (_, called) = rpc_on("2026-07-28", &host, Some(&token), "worker", "tools/call", search.clone()).await;

  assert_eq!(called["result"]["resultType"], "complete", "{called}");

  let (_, earlier) = rpc(&host, Some(&token), "worker", "tools/list", json!({})).await;

  assert!(earlier["result"].get("resultType").is_none(), "{earlier}");
}

#[tokio::test]
async fn records_survive_a_restart_and_the_same_token_works_after_it() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let graph = checkout("kept");
  let token = {
    let host = scratch.start();

    issue(
      &host,
      &record(scratch.data.path(), &["graph.read"], json!({ "checkout": graph.path() })),
    )
    .await
  };
  let host = scratch.start();
  let file = fs::read_dir(scratch.data.path().join("tool-host/sessions"))
    .unwrap()
    .next()
    .unwrap()
    .unwrap();

  assert_eq!(
    text(&called(&host, &token, "worker", "graph_search", json!({ "text": "kept" })).await),
    "area/kept  kept"
  );
  assert!(!file.file_name().to_string_lossy().contains(&token));
  assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
}

use std::os::unix::fs::PermissionsExt;

// What the stand-in readings answer every call with, and the one procedure the stand-in procedures serve.
const CARRIED: &str = "carried";

// A tool serving a namespace's resources and no entry, whose executable keeps what each request handed it under
// the scratch's calls, lists one procedure, reads it, and refuses a read of any other with why.
const PAPERS_MANIFEST: &str = r#"
[tool]
name = "papers"
protocol = 1
language = "python"
run = ["python3", "papers.py"]
timeout = 20

[resources]
namespace = "bmo"
capabilities = ["room.tools"]
"#;

const PAPERS_SCRIPT: &str = r#"
import json, sys, time

asked = json.load(sys.stdin)
open(f"{CALLS}/{time.time_ns()}.in", "w").write(json.dumps(asked))

if asked["kind"] == "resources_list":
    print(json.dumps({"resources": [{"uri": "bmo://procedure/carried", "name": "carried", "mimeType": "text/markdown"}]}))
elif asked["uri"] == "bmo://procedure/carried":
    print(json.dumps({"contents": [{"uri": asked["uri"], "mimeType": "text/markdown", "text": "carried"}]}))
else:
    print(json.dumps({"error": "no procedure named " + asked["uri"]}))
"#;

fn place_papers(scratch: &Scratch) {
  let script = format!("CALLS = {:?}\n{PAPERS_SCRIPT}", scratch.calls().display().to_string());

  place(&scratch.tools(), "papers", &[("tool.toml", PAPERS_MANIFEST), ("papers.py", &script)]);
}

// What the stand-in procedures were handed, one request each, oldest first.
fn handed(scratch: &Scratch) -> Vec<Value> {
  let mut calls: Vec<PathBuf> = fs::read_dir(scratch.calls())
    .unwrap()
    .map(|entry| entry.unwrap().path())
    .filter(|path| path.extension().is_some_and(|extension| extension == "in"))
    .collect();

  calls.sort();
  calls
    .into_iter()
    .map(|path| serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap())
    .collect()
}

// A placed tool is started with nothing of the call on its command line and is handed the call on standard
// input, and a namespace serving resources alone lists nothing for a line's instructions.
#[tokio::test]
async fn resources_are_listed_and_read_by_their_placed_tool_handed_the_record_and_the_request_on_standard_input()
 {
  let scratch = Scratch::new();

  place_papers(&scratch);

  let host = scratch.start();
  let reader =
    issue(&host, &record(scratch.data.path(), &["room.tools"], json!({ "line": "search" }))).await;
  let stranger = issue(&host, &record(scratch.data.path(), &["graph.read"], json!({}))).await;
  let (_, resources) = rpc(&host, Some(&reader), "bmo", "resources/list", json!({})).await;

  assert_eq!(resources["result"]["resources"][0]["uri"], "bmo://procedure/carried");

  let (_, read) = rpc(
    &host,
    Some(&reader),
    "bmo",
    "resources/read",
    json!({ "uri": "bmo://procedure/carried" }),
  )
  .await;

  assert_eq!(read["result"]["contents"][0]["text"], CARRIED);

  let (_, refused) = rpc(
    &host,
    Some(&reader),
    "bmo",
    "resources/read",
    json!({ "uri": "bmo://procedure/missing" }),
  )
  .await;

  assert_eq!(refused["error"]["message"], "no procedure named bmo://procedure/missing");

  let calls = handed(&scratch);

  assert_eq!(calls.len(), 3);
  assert_eq!(calls[0]["kind"], "resources_list");
  assert_eq!(calls[0]["session"]["line"], "search");
  assert!(!calls[0].to_string().contains(&reader));
  assert_eq!(
    (&calls[1]["kind"], &calls[1]["uri"]),
    (&json!("resources_read"), &json!("bmo://procedure/carried"))
  );
  assert!(
    rpc(&host, Some(&stranger), "bmo", "resources/list", json!({})).await.1["result"]["resources"]
      .as_array()
      .unwrap()
      .is_empty()
  );
  assert_eq!(handed(&scratch).len(), 3);
  assert_eq!(listing(&host, "bmo").await.0, StatusCode::NOT_FOUND);
}

// A placed sending entry, declaring the chat verb it needs and its icon beside it.
const SENDER_MANIFEST: &str = r#"
[tool]
name = "sender"
protocol = 1
language = "python"
run = ["python3", "sender.py"]
timeout = 20

[[entry]]
namespace = "bmo"
name = "sticker"
description = "Sends a sticker."
input = "any.json"
capabilities = ["room.tools"]
hints = { open_world = true }

[entry.facts]
verb = "sticker"
icon = "i-lucide-sticker"
"#;

// A placed filing entry, under a name a chat line's record shows: it carries a grant, a bundle's narrower form and
// the record its call leaves, and its executable answers every call alike.
const FILER_MANIFEST: &str = r#"
[tool]
name = "filer"
protocol = 1
language = "python"
run = ["python3", "filer.py"]
timeout = 20

[[entry]]
namespace = "bmo"
name = "todo_add"
description = "Files a row."
input = "todo_add.json"
capabilities = ["room.tools"]
grant = "file_todo"

[entry.bundle."room.chat"]
shown = ["title", "text"]
required = ["title", "text"]

[entry.facts]
record = "todo-add"
"#;

const FILING_SCHEMA: &str = r#"{ "type": "object", "properties": { "title": { "type": "string" }, "text": { "type": "string" }, "chat": { "type": "integer" }, "message": { "type": "integer" } }, "required": ["title"], "additionalProperties": false }"#;

const FILED: &str = "filed";

const FILER_SCRIPT: &str = r#"
import json, sys

json.load(sys.stdin)
print(json.dumps({"content": [{"type": "text", "text": "filed"}], "isError": False}))
"#;

fn place_filer(scratch: &Scratch) {
  place(
    &scratch.tools(),
    "filer",
    &[
      ("tool.toml", FILER_MANIFEST),
      ("todo_add.json", FILING_SCHEMA),
      ("filer.py", FILER_SCRIPT),
    ],
  );
}

// Placed readings: `running`, which a chat line's record shows, and `jobs`, which no record shows, each read-only
// and answering with the reading's object; its executable answers every call alike.
const READINGS_MANIFEST: &str = r#"
[tool]
name = "readings"
protocol = 1
language = "python"
run = ["python3", "readings.py"]
timeout = 20

[[entry]]
namespace = "bmo"
name = "running"
description = "The live work, as it stands."
input = "running.json"
output = "reading.json"
capabilities = ["room.tools"]
hints = { read_only = true }

[[entry]]
namespace = "bmo"
name = "jobs"
description = "A room's job runs, newest first."
input = "jobs.json"
output = "reading.json"
capabilities = ["room.tools"]
hints = { read_only = true }
"#;

const READINGS_SCRIPT: &str = r#"
import json, sys

json.load(sys.stdin)
print(json.dumps({"content": [{"type": "text", "text": "carried"}], "isError": False}))
"#;

fn place_readings(scratch: &Scratch) {
  place(
    &scratch.tools(),
    "readings",
    &[
      ("tool.toml", READINGS_MANIFEST),
      (
        "running.json",
        r#"{ "type": "object", "properties": { "full": { "type": "boolean" } }, "additionalProperties": false }"#,
      ),
      (
        "jobs.json",
        r#"{ "type": "object", "properties": { "chat": { "type": "integer" }, "limit": { "type": "integer" }, "full": { "type": "boolean" } }, "additionalProperties": false }"#,
      ),
      (
        "reading.json",
        r#"{ "type": "object", "properties": { "lines": { "type": "array", "items": { "type": "string" } } }, "required": ["lines"] }"#,
      ),
      ("readings.py", READINGS_SCRIPT),
    ],
  );
}

// A namespace says which record a call leaves, by what the call was.
#[tokio::test]
async fn a_namespace_says_its_digest_its_sending_verbs_its_records_and_its_icons() {
  let scratch = Scratch::new();

  place(
    &scratch.tools(),
    "sender",
    &[
      ("tool.toml", SENDER_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("sender.py", ""),
    ],
  );
  place_filer(&scratch);

  let host = scratch.start();
  let reply = request(&host.control, Method::GET, "/facts/bmo", None).await.unwrap();
  let facts: Value = serde_json::from_slice(&reply.body).unwrap();
  let named = |field: &str, value: &str| -> Vec<(String, String)> {
    facts[field]
      .as_array()
      .unwrap()
      .iter()
      .map(|one| (one["name"].as_str().unwrap().to_owned(), one[value].as_str().unwrap().to_owned()))
      .collect()
  };

  assert_eq!(reply.status, StatusCode::OK);
  assert_eq!(facts["digest"].as_str().unwrap().len(), 64);
  assert_eq!(named("sending", "verb"), [("sticker".to_owned(), "sticker".to_owned())]);
  assert_eq!(named("records", "record"), [("todo_add".to_owned(), "todo-add".to_owned())]);
  assert!(named("icons", "icon").contains(&("sticker".to_owned(), "i-lucide-sticker".to_owned())));
  assert_eq!(
    request(&host.control, Method::GET, "/facts/worker", None).await.unwrap().status,
    StatusCode::NOT_FOUND
  );
}

// A record's id, read off the file it is kept in: the one that appeared since `before`.
fn appeared(scratch: &Scratch, before: &[String]) -> String {
  fs::read_dir(scratch.data.path().join("tool-host/sessions"))
    .unwrap()
    .map(|entry| entry.unwrap().file_name().to_string_lossy().trim_end_matches(".json").to_owned())
    .find(|id| !before.contains(id))
    .unwrap()
}

// A run whose deadline has passed with nothing calling it: the beat drops its record, and what the record
// kept ends with it, while a resident program's record stands.
#[tokio::test]
async fn a_lapsed_run_is_dropped_on_the_next_beat_though_nothing_calls_it() {
  let scratch = Scratch::new();
  let host = scratch.start_with(&["--sweep-every-ms", "100"]);
  let sessions = scratch.data.path().join("tool-host/sessions");

  issue(&host, &record(scratch.data.path(), &["web.read"], json!({}))).await;
  let resident_id = appeared(&scratch, &[]);
  issue(&host, &record(scratch.data.path(), &["web.read"], json!({ "deadline": now_ms() }))).await;
  let lapsed_id = appeared(&scratch, std::slice::from_ref(&resident_id));
  let bound = std::time::Instant::now() + Duration::from_secs(30);

  while sessions.join(format!("{lapsed_id}.json")).exists() {
    assert!(std::time::Instant::now() < bound, "the lapsed run's record was never dropped");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }

  assert!(sessions.join(format!("{resident_id}.json")).exists());
}

// The bearer token the stand-in account service holds its callers to.
const ACCOUNTS_TOKEN: &str = "accounts-token";

// The account service standing in, on a loopback port of its own: its model in force where `in_force` says so;
// in room -5 the sender of message 1 holds what is asked and the sender of message 2 does not; user 9 holds no
// tool capability; and a caller without its token is refused as the service refuses one. Its base address.
async fn stand_in_accounts(in_force: bool) -> String {
  use axum::http::HeaderMap;
  use axum::response::IntoResponse;
  use axum::routing::get;

  fn bearer(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|value| value.to_str().ok())
      == Some(&format!("Bearer {ACCOUNTS_TOKEN}"))
  }

  fn refused() -> axum::response::Response {
    (
      StatusCode::UNAUTHORIZED,
      Json(json!({ "error": "The token is not the server's.", "code": "unauthorized" })),
    )
      .into_response()
  }

  let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let base = format!("http://{}", listener.local_addr().unwrap());
  let app = axum::Router::new()
    .route(
      "/accounts/model",
      get(move |headers: HeaderMap| async move {
        if !bearer(&headers) {
          return refused();
        }

        Json(json!({ "in_force": in_force })).into_response()
      }),
    )
    .route(
      "/checks/message",
      post(|headers: HeaderMap, Json(asked): Json<Value>| async move {
        if !bearer(&headers) {
          return refused();
        }

        let grant = format!("bmo_room:{}#{}", asked["room"], asked["relation"].as_str().unwrap_or(""));

        Json(json!({ "allowed": asked["room"] == -5 && asked["message"] == 1, "grant": grant }))
          .into_response()
      }),
    )
    .route(
      "/checks/object",
      post(|headers: HeaderMap, Json(asked): Json<Value>| async move {
        if !bearer(&headers) {
          return refused();
        }

        let grant = format!("bmo_tools:{}#use", asked["object"].as_str().unwrap_or(""));

        Json(json!({ "allowed": asked["user"] != 9, "grant": grant })).into_response()
      }),
    );

  tokio::spawn(async move {
    axum::serve(listener, app).await.unwrap();
  });

  base
}

// A tool that acts carries the grant it needs and takes the message it acts on; a bundle's form carries none.
#[tokio::test]
async fn a_tool_that_acts_runs_only_on_a_message_whose_sender_holds_its_grant_in_the_turns_room() {
  let scratch = Scratch::new();

  place_filer(&scratch);
  place_readings(&scratch);

  let accounts = stand_in_accounts(true).await;
  let host = scratch.start_asking(&accounts, &[]);
  let session = record(
    scratch.data.path(),
    &["room.tools"],
    json!({ "line": "chat", "runVariables": { "BMO_TURN_CHAT": "-5" } }),
  );
  let token = issue(&host, &session).await;
  let unnamed = called(&host, &token, "bmo", "todo_add", json!({ "title": "x" })).await;

  assert_eq!(unnamed["result"]["isError"], true);
  assert!(text(&unnamed).contains("names the message it acts on"), "{unnamed}");

  let refused = called(&host, &token, "bmo", "todo_add", json!({ "title": "x", "message": 2 })).await;

  assert_eq!(refused["result"]["isError"], true);
  assert!(text(&refused).contains("does not hold bmo_room:-5#file_todo"), "{refused}");
  assert_eq!(
    text(&called(&host, &token, "bmo", "todo_add", json!({ "title": "x", "message": 1 })).await),
    FILED
  );
  assert_eq!(text(&called(&host, &token, "bmo", "running", json!({})).await), CARRIED);

  // A turn whose every new message is the owner's acts for nobody else, and names no message.
  let privileged = issue(
    &host,
    &record(
      scratch.data.path(),
      &["room.tools"],
      json!({ "line": "chat", "runVariables": { "BMO_TURN_CHAT": "-5", "BMO_TURN_PRIVILEGED": "1" } }),
    ),
  )
  .await;

  assert_eq!(
    text(&called(&host, &privileged, "bmo", "todo_add", json!({ "title": "x" })).await),
    FILED
  );

  // A job run for another user's item uses a tool only where that user holds its capability.
  let for_user = issue(
    &host,
    &record(
      scratch.data.path(),
      &["room.tools"],
      json!({ "line": "chat", "runVariables": { "BMO_TURN_CHAT": "-5", "BMO_TURN_USER": "9" } }),
    ),
  )
  .await;
  let narrowed = called(&host, &for_user, "bmo", "running", json!({})).await;

  assert_eq!(narrowed["result"]["isError"], true);
  assert!(text(&narrowed).contains("does not hold bmo_tools:room.tools#use"), "{narrowed}");
}

// The tools a session is shown, each by the schema of the form it holds.
async fn shown(host: &Running, token: &str) -> Vec<(String, Value)> {
  rpc(host, Some(token), "bmo", "tools/list", json!({})).await.1["result"]["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|tool| (tool["name"].as_str().unwrap().to_owned(), tool["inputSchema"].clone()))
    .collect()
}

// A record holding bundles alone is shown and served their entries alone, each schema narrowed to what its entry
// shows and each call held to it; one holding the whole catalogue as well is shown each entry once and whole.
#[tokio::test]
async fn a_record_holding_a_role_bundle_alone_is_shown_its_tools_narrowed_and_one_holding_both_each_whole()
{
  let scratch = Scratch::new();

  place_filer(&scratch);
  place_readings(&scratch);

  let host = scratch.start();
  let both = issue(
    &host,
    &record(
      scratch.data.path(),
      &["room.tools", "room.chat", "room.send"],
      json!({ "line": "chat" }),
    ),
  )
  .await;
  let bundles = issue(
    &host,
    &record(scratch.data.path(), &["room.chat", "room.send"], json!({ "line": "chat" })),
  )
  .await;
  let whole = shown(&host, &both).await;
  let narrowed = shown(&host, &bundles).await;
  let names = |tools: &[(String, Value)]| tools.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();

  assert_eq!(names(&whole).iter().filter(|name| *name == "todo_add").count(), 1);
  assert!(names(&whole).contains(&"running".to_owned()));
  assert!(
    whole
      .iter()
      .any(|(name, schema)| name == "todo_add" && schema["properties"]["chat"].is_object())
  );
  assert_eq!(names(&narrowed), ["todo_add"]);

  let filing = &narrowed[0].1;
  let mut offered: Vec<&String> = filing["properties"].as_object().unwrap().keys().collect();

  offered.sort();
  assert_eq!(offered, ["text", "title"]);
  assert_eq!(filing["required"], json!(["title", "text"]));

  // A bundle's form is held to its own arguments, and acts under no grant.
  let refused =
    called(&host, &bundles, "bmo", "todo_add", json!({ "title": "x", "text": "y", "chat": 5 })).await;

  assert_eq!(refused["result"]["isError"], true);
  assert!(
    text(&refused).contains("was not called: its arguments do not fit its schema"),
    "{refused}"
  );
  assert_eq!(
    text(&called(&host, &bundles, "bmo", "todo_add", json!({ "title": "x", "text": "y" })).await),
    FILED
  );
  assert_eq!(called(&host, &bundles, "bmo", "running", json!({})).await["error"]["code"], -32602);
}

// The lock stands only where an account service is named, the switch leaves it on and the service's model is
// in force: a call naming no message is filed under the switch off, with no service named, and with the
// model not in force, and refused once all three hold.
#[tokio::test]
async fn the_switch_turns_the_lock_off_and_with_no_account_service_or_no_model_in_force_it_never_stands() {
  let scratch = Scratch::new();

  place_filer(&scratch);

  let in_force = stand_in_accounts(true).await;
  let not_in_force = stand_in_accounts(false).await;
  let session = record(
    scratch.data.path(),
    &["room.tools"],
    json!({ "line": "chat", "runVariables": { "BMO_TURN_CHAT": "-5" } }),
  );
  let filed = |host: Running| {
    let session = session.clone();

    async move {
      let token = issue(&host, &session).await;

      text(&called(&host, &token, "bmo", "todo_add", json!({ "title": "x" })).await)
    }
  };

  assert_eq!(filed(scratch.start_asking(&in_force, &["--grant-lock", "off"])).await, FILED);
  assert_eq!(filed(scratch.start()).await, FILED);
  assert_eq!(filed(scratch.start_asking(&not_in_force, &[])).await, FILED);
  assert!(
    filed(scratch.start_asking(&in_force, &[]))
      .await
      .contains("names the message it acts on")
  );
}

// The toolbook (docs/toolbook.md): what a namespace does not list, listed, described and called through
// it by the dispatch a direct call takes.
#[tokio::test]
async fn the_toolbook_lists_describes_and_calls_what_the_namespaces_do_not_list() {
  let scratch = Scratch::new();

  place_readers(&scratch);
  place_filer(&scratch);
  place_readings(&scratch);

  let host = scratch.start();
  let graph = checkout("book");
  let searcher = issue(
    &host,
    &record(
      scratch.data.path(),
      &["room.tools", "graph.read"],
      json!({ "line": "search", "checkout": graph.path() }),
    ),
  )
  .await;

  // A searcher's record shows none of the `bmo` namespace's entries, so every one it holds is booked.
  assert!(listed(&host, &searcher, "bmo").await.is_empty());
  assert_eq!(listed(&host, &searcher, "toolbook").await, ["list", "describe", "call", "act"]);

  let book = text(&called(&host, &searcher, "toolbook", "list", json!({})).await);

  for line in [
    "bmo/jobs · call — A room's job runs, newest first.",
    "bmo/running · call — The live work, as it stands.",
    "bmo/todo_add · act — Files a row.",
    "worker/graph_list · call — Asked with an area, that area's features.",
  ] {
    assert!(book.lines().any(|listed| listed == line), "{line}\n{book}");
  }

  assert_eq!(
    text(&called(&host, &searcher, "toolbook", "list", json!({ "query": "JOB RUNS" })).await),
    "bmo/jobs · call — A room's job runs, newest first."
  );

  let described =
    text(&called(&host, &searcher, "toolbook", "describe", json!({ "tool": "bmo/jobs" })).await);

  assert!(
    described.starts_with(
      "bmo/jobs — reached with call\nA room's job runs, newest first.\nRead-only: yes\nInput schema: "
    ),
    "{described}"
  );
  assert!(described.contains("\nAnswers with: {"), "{described}");

  // A booked tool answers through the toolbook and by its own name alike.
  assert_eq!(
    text(
      &called(
        &host,
        &searcher,
        "toolbook",
        "call",
        json!({ "tool": "bmo/jobs", "arguments": { "limit": 3 } })
      )
      .await
    ),
    CARRIED
  );
  assert_eq!(text(&called(&host, &searcher, "bmo", "jobs", json!({ "limit": 3 })).await), CARRIED);
  assert!(
    text(
      &called(
        &host,
        &searcher,
        "toolbook",
        "call",
        json!({ "tool": "worker/graph_list", "arguments": {} })
      )
      .await
    )
    .contains("area")
  );

  // Arguments that do not fit are refused before anything runs, naming each and carrying the schema.
  let misfit = called(
    &host,
    &searcher,
    "toolbook",
    "call",
    json!({ "tool": "bmo/jobs", "arguments": { "limit": "three" } }),
  )
  .await;

  assert_eq!(misfit["result"]["isError"], true);
  assert!(
    text(&misfit)
      .starts_with("ERROR: bmo/jobs was not called: its arguments do not fit its schema.\n- /limit: "),
    "{misfit}"
  );
  assert!(text(&misfit).contains("Its schema: {"), "{misfit}");

  // A tool that acts is reached with act alone.
  let refused = called(
    &host,
    &searcher,
    "toolbook",
    "call",
    json!({ "tool": "bmo/todo_add", "arguments": { "title": "x" } }),
  )
  .await;

  assert_eq!(text(&refused), "ERROR: bmo/todo_add acts, so call does not reach it: call it with act.");
  assert_eq!(
    text(
      &called(
        &host,
        &searcher,
        "toolbook",
        "act",
        json!({ "tool": "bmo/todo_add", "arguments": { "title": "x" } })
      )
      .await
    ),
    FILED
  );

  // A tool the session does not hold is refused by the words a direct call is.
  assert_eq!(
    text(
      &called(&host, &searcher, "toolbook", "call", json!({ "tool": "web/nope", "arguments": {} })).await
    ),
    "ERROR: Tool web/nope not found"
  );
  assert_eq!(
    text(&called(&host, &searcher, "toolbook", "describe", json!({ "tool": "nope" })).await),
    "ERROR: Tool nope not found"
  );

  // A worker holds web.read alone: none of the `bmo` namespace's entries is in a toolbook of its.
  let worker = issue(&host, &record(scratch.data.path(), &["web.read"], json!({}))).await;

  assert_eq!(
    text(&called(&host, &worker, "toolbook", "list", json!({})).await),
    "No tool is reached through the toolbook in this session."
  );
  assert_eq!(
    text(&called(&host, &worker, "toolbook", "call", json!({ "tool": "bmo/jobs", "arguments": {} })).await),
    "ERROR: Tool bmo/jobs not found"
  );
}

// A placed tool's folder (docs/mcp-host.md, 「Placed tools」): written beside where it is placed, then put in
// place in one rename, as the deployment places one.
fn place(tools: &Path, folder: &str, files: &[(&str, &str)]) {
  let staged = tools.join(folder).join(".placing");

  fs::create_dir_all(&staged).unwrap();

  for (name, text) in files {
    fs::write(staged.join(name), text).unwrap();
  }

  fs::rename(&staged, tools.join(folder).join(".placed")).unwrap();
}

// A placed tool taken away, as the deployment takes one out.
fn take_away(tools: &Path, folder: &str) {
  let gone = tools.join(folder).join(".taken");

  fs::rename(tools.join(folder).join(".placed"), &gone).unwrap();
  fs::remove_dir_all(tools.join(folder)).unwrap();
}

// Waits, ten seconds at most, for a condition the host's reading or a process's end makes true.
async fn until<F: std::future::Future<Output = bool>>(what: &str, mut check: impl FnMut() -> F) {
  for _ in 0..200 {
    if check().await {
      return;
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
  }

  panic!("{what} did not come within ten seconds");
}

async fn book(host: &Running, token: &str) -> String {
  text(&called(host, token, "toolbook", "list", json!({})).await)
}

async fn through(host: &Running, token: &str, proxy: &str, tool: &str, arguments: Value) -> Value {
  called(host, token, "toolbook", proxy, json!({ "tool": tool, "arguments": arguments })).await
}

// The echo tool: every entry it declares answered by one script reading the call on standard input.
const ECHO_MANIFEST: &str = r#"
[tool]
name = "echo"
protocol = 1
language = "python"
run = ["python3", "echo.py"]
timeout = 20

[[entry]]
namespace = "worker"
name = "echo_back"
description = "Says the text back. Then more."
input = "echo.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[[entry]]
namespace = "bmo"
name = "echo_note"
description = "Notes the text."
input = "note.json"
capabilities = ["room.tools"]
grant = "notes"

[entry.facts]
icon = "i-lucide-notebook"

[[entry]]
namespace = "worker"
name = "echo_hidden"
description = "Shown to a web reader alone."
input = "echo.json"
capabilities = ["web.read"]
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "echo_quiet"
description = "Says the text back to the system alone."
input = "echo.json"
system = true
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "echo_slow"
description = "Never answers in time."
input = "echo.json"
capabilities = ["graph.read"]
timeout = 1
hints = { read_only = true }

[[entry]]
namespace = "worker"
name = "echo_broken"
description = "Ends badly."
input = "echo.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[[entry]]
namespace = "web"
name = "echo_page"
description = "Reads a page of example.org."
input = "echo.json"
capabilities = ["graph.read"]
hints = { read_only = true }

[entry.claims]
hosts = ["example.org"]

[entry.listing]
line = "echo_page: a page of example.org"

[entry.facts.lists.sites]
site = "example.org"
reads = ["example.org"]

[[entry]]
namespace = "echoes"
name = "echo_far"
description = "Stands in a namespace of its own."
input = "echo.json"
capabilities = ["graph.read"]
hints = { read_only = true }
"#;

const ECHO_SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" }, "via": { "type": "string" }, "with": { "type": "object" }, "claim": { "type": "string" }, "hold": { "type": "string" } }, "required": ["text"], "additionalProperties": false }"#;

const NOTE_SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" }, "message": { "type": "integer" } }, "required": ["text"] }"#;

const ECHO_SCRIPT: &str = r#"
import json, socket, sys, time

handed = json.load(sys.stdin)

def back(asked):
    with socket.socket(socket.AF_UNIX) as line:
        line.connect(handed["callback"])
        line.sendall((json.dumps(asked) + "\n").encode())
        return json.loads(line.makefile().readline())

if handed["kind"] == "claim":
    page = {"page": handed["address"]} if "example.org" in handed["address"] else None
    print(json.dumps({"arguments": page}))
    sys.exit()

entry, arguments = handed["entry"], handed["arguments"]

if entry.endswith("echo_slow"):
    time.sleep(30)

if "hold" in arguments:
    import subprocess
    held = subprocess.Popen(["sleep", "60"])
    with open(arguments["hold"], "w") as written:
        written.write(str(held.pid))
    time.sleep(30)

if entry.endswith("echo_broken"):
    print("it broke", file=sys.stderr)
    sys.exit(3)

said = handed["session"]["line"] + ":" + arguments["text"]

if "via" in arguments:
    said += " via " + back({"kind": "call", "entry": arguments["via"], "arguments": arguments.get("with", {})})["content"][0]["text"]

if "claim" in arguments:
    said += " claimed " + json.dumps(back({"kind": "claim", "address": arguments["claim"]}), sort_keys=True)

print(json.dumps({"content": [{"type": "text", "text": said}], "isError": False}))
"#;

// "Placed tools" (docs/mcp-host.md, 「Tests」): a tool folder placed into a running host's tree is listed,
// described and called through `call` or `act` under the capability and grant checks, then taken away and gone,
// the host's process the same one throughout.
#[tokio::test]
async fn a_tool_folder_placed_into_a_running_hosts_tree_is_served_through_the_toolbook_and_gone_once_taken_away()
 {
  let scratch = Scratch::new();

  place_readings(&scratch);

  let accounts = stand_in_accounts(true).await;
  let mut host = scratch.start_asking(&accounts, &[]);
  let pid = host.child.id();
  let token = issue(
    &host,
    &record(
      scratch.data.path(),
      &["graph.read", "room.tools"],
      json!({ "line": "search", "runVariables": { "BMO_TURN_CHAT": "-5" } }),
    ),
  )
  .await;

  assert!(!book(&host, &token).await.contains("echo_back"));
  assert_eq!(
    rpc(&host, Some(&token), "echoes", "tools/list", json!({})).await.0,
    StatusCode::NOT_FOUND
  );

  let standing: Value =
    request(&host.control, Method::GET, "/facts/bmo", None).await.unwrap().json().unwrap();

  place(
    &scratch.tools(),
    "echo",
    &[
      ("tool.toml", ECHO_MANIFEST),
      ("echo.json", ECHO_SCHEMA),
      ("note.json", NOTE_SCHEMA),
      ("echo.py", ECHO_SCRIPT),
    ],
  );
  let (at, held) = (&host, token.as_str());

  until(
    "the placed tool",
    move || async move { book(at, held).await.contains("worker/echo_back") },
  )
  .await;

  let listed = book(&host, &token).await;
  // Calls open and read the tool's files, which is no change to what is placed.
  let readings = || scratch.log().matches("mcp-host: read ").count();
  let read_before_calls = readings();

  assert!(
    listed.lines().any(|line| line == "worker/echo_back · call — Says the text back."),
    "{listed}"
  );
  assert!(listed.lines().any(|line| line == "bmo/echo_note · act — Notes the text."), "{listed}");
  assert!(!listed.contains("echo_hidden") && !listed.contains("echo_quiet"), "{listed}");
  assert!(
    text(&called(&host, &token, "toolbook", "describe", json!({ "tool": "worker/echo_back" })).await)
      .starts_with("worker/echo_back — reached with call\nSays the text back. Then more.\nRead-only: yes\nInput schema: {")
  );
  assert_eq!(rpc(&host, Some(&token), "echoes", "tools/list", json!({})).await.0, StatusCode::OK);

  // Called through the toolbook and by its own name alike, its arguments held to the schema in its folder.
  assert_eq!(
    text(&through(&host, &token, "call", "worker/echo_back", json!({ "text": "hi" })).await),
    "search:hi"
  );
  assert_eq!(
    text(&called(&host, &token, "worker", "echo_back", json!({ "text": "x" })).await),
    "search:x"
  );
  assert!(
    text(&through(&host, &token, "call", "worker/echo_back", json!({ "text": 3 })).await)
      .contains("was not called: its arguments do not fit its schema")
  );

  // An entry that acts is reached with act alone, and runs only for a sender holding its grant.
  assert!(
    text(&through(&host, &token, "call", "bmo/echo_note", json!({ "text": "n" })).await)
      .contains("call it with act")
  );
  assert!(
    text(&through(&host, &token, "act", "bmo/echo_note", json!({ "text": "n", "message": 2 })).await)
      .contains("does not hold bmo_room:-5#notes")
  );
  assert_eq!(
    text(&through(&host, &token, "act", "bmo/echo_note", json!({ "text": "n", "message": 1 })).await),
    "search:n"
  );

  // Its call-back socket reaches what the session could, and the entries for the system alone.
  let via = |to: &str, with: Value| json!({ "text": "a", "via": to, "with": with });

  assert_eq!(
    text(
      &through(
        &host,
        &token,
        "call",
        "worker/echo_back",
        via("worker/echo_hidden", json!({ "text": "h" }))
      )
      .await
    ),
    "search:a via ERROR: Tool worker/echo_hidden not found"
  );
  assert_eq!(
    text(
      &through(
        &host,
        &token,
        "call",
        "worker/echo_back",
        via("worker/echo_quiet", json!({ "text": "q" }))
      )
      .await
    ),
    "search:a via search:q"
  );
  assert!(
    text(
      &through(
        &host,
        &token,
        "call",
        "worker/echo_back",
        via("bmo/echo_note", json!({ "text": "b", "message": 2 }))
      )
      .await
    )
    .contains("does not hold bmo_room:-5#notes")
  );
  assert_eq!(
    text(
      &through(
        &host,
        &token,
        "call",
        "worker/echo_back",
        json!({ "text": "c", "claim": "https://www.example.org/p" })
      )
      .await
    ),
    r#"search:c claimed {"arguments": {"page": "https://www.example.org/p"}, "entry": "web/echo_page"}"#
  );

  // A call past its timeout, and one that ends badly, fail saying so.
  let started = std::time::Instant::now();

  assert!(
    text(&through(&host, &token, "call", "worker/echo_slow", json!({ "text": "s" })).await)
      .contains("did not answer within 1 seconds")
  );
  assert!(started.elapsed() < Duration::from_secs(10));
  assert_eq!(
    text(&through(&host, &token, "call", "worker/echo_broken", json!({ "text": "b" })).await),
    "ERROR: worker/echo_broken failed: it ended with code 3: it broke"
  );
  assert_eq!(readings(), read_before_calls, "{}", scratch.log());

  // Its namespace's listing and facts carry what its manifest says, and the catalogue names its folder.
  assert!(listing(&host, "web").await.1.ends_with("echo_page: a page of example.org"));

  let facts = request(&host.control, Method::GET, "/facts/bmo", None).await.unwrap().json().unwrap();

  assert!(
    facts["icons"]
      .as_array()
      .unwrap()
      .contains(&json!({ "name": "echo_note", "icon": "i-lucide-notebook" }))
  );
  // The digest is reckoned over every entry of the namespace, so an entry placed beside those standing moves it,
  // and a namespace no owner says anything of still has its digest once it holds an entry.
  assert_ne!(facts["digest"], standing["digest"]);

  let worker: Value = request(&host.control, Method::GET, "/facts/worker", None)
    .await
    .unwrap()
    .json()
    .unwrap();

  assert_eq!(worker["digest"].as_str().unwrap().len(), 64);

  // An item a placed entry adds to a list of its namespace's facts stands headed with its name.
  let web: Value = request(&host.control, Method::GET, "/facts/web", None).await.unwrap().json().unwrap();

  assert_eq!(
    web["sites"].as_array().unwrap().last().unwrap(),
    &json!({ "name": "echo_page", "site": "example.org", "reads": ["example.org"] })
  );

  let catalogue = request(&host.control, Method::GET, "/catalogue", None).await.unwrap().json().unwrap();

  assert!(
    catalogue
      .as_array()
      .unwrap()
      .iter()
      .any(|entry| entry["name"] == "worker/echo_back" && entry["placed"] == "echo")
  );

  let system = request(
    &host.control,
    Method::POST,
    "/calls",
    Some(&json!({ "session": record(scratch.data.path(), &[], json!({})), "tool": "worker/echo_quiet", "arguments": { "text": "s" } })),
  )
  .await
  .unwrap()
  .json()
  .unwrap();

  assert_eq!(system["content"][0]["text"], "worker:s");

  // A system call whose caller hangs up before it answers has every process it started ended.
  let held = scratch.data.path().join("held.pid");
  let asked = json!({ "session": record(scratch.data.path(), &[], json!({})), "tool": "worker/echo_quiet", "arguments": { "text": "h", "hold": held.to_string_lossy() } });
  let control = host.control.clone();
  let hanging = tokio::spawn(async move { request(&control, Method::POST, "/calls", Some(&asked)).await });

  until("the held process", || {
    let held = held.clone();

    async move { fs::read_to_string(&held).is_ok_and(|pid| !pid.trim().is_empty()) }
  })
  .await;

  let started = fs::read_to_string(&held).unwrap();

  assert!(alive(&started));

  hanging.abort();
  until("the held process's end", || {
    let started = started.clone();

    async move { !alive(&started) }
  })
  .await;

  // The control socket names the entry that claims an address, asked as the call-back socket asks, and none for
  // one no entry claims.
  let control = &host.control;
  let claimed = |address: &str| {
    let asked =
      json!({ "session": record(scratch.data.path(), &["graph.read"], json!({})), "address": address });

    async move { request(control, Method::POST, "/claims", Some(&asked)).await.unwrap().json().unwrap() }
  };

  assert_eq!(
    claimed("https://www.example.org/p").await,
    json!({ "entry": "web/echo_page", "arguments": { "page": "https://www.example.org/p" } })
  );
  assert_eq!(claimed("https://example.com/p").await, json!({ "entry": null }));

  // A folder that does not read is said in the log and leaves the rest served.
  place(&scratch.tools(), "broken", &[("tool.toml", "[tool]\nname = \"broken\"\n")]);
  let watched = &scratch;

  until("the refusal in the log", move || async move {
    watched.log().contains("left out tools/broken: ")
  })
  .await;
  assert_eq!(
    text(&through(&host, &token, "call", "worker/echo_back", json!({ "text": "on" })).await),
    "search:on"
  );

  take_away(&scratch.tools(), "echo");
  let (at, held) = (&host, token.as_str());

  until("the tool gone", move || async move { !book(at, held).await.contains("echo_back") }).await;

  assert_eq!(
    text(&through(&host, &token, "call", "worker/echo_back", json!({ "text": "x" })).await),
    "ERROR: Tool worker/echo_back not found"
  );
  assert_eq!(
    rpc(&host, Some(&token), "echoes", "tools/list", json!({})).await.0,
    StatusCode::NOT_FOUND
  );
  assert!(host.child.try_wait().unwrap().is_none());
  assert_eq!(host.child.id(), pid);
}

const COUNTER_MANIFEST: &str = "[tool]\nname = \"counter\"\nprotocol = 1\nrun = [\"python3\", \"counter.py\"]\ntimeout = 10\n\n[helper]\ndescription = \"A count one record's calls share.\"\n";

const COUNTER_SCRIPT: &str = r#"
import json, os, socket, sys

handed = json.load(sys.stdin)
record = handed["session"]
open(os.path.join(record["dataRoot"], "helper-" + record["line"] + ".pid"), "w").write(str(os.getpid()))
listener = socket.socket(socket.AF_UNIX)
listener.bind(handed["socket"])
listener.listen()
print("listening", flush=True)
count = 0

while True:
    asked, _ = listener.accept()
    count += 1
    asked.sendall(f"{count}\n".encode())
    asked.close()
"#;

const TALLY_MANIFEST: &str = "[tool]\nname = \"tally\"\nprotocol = 1\nrun = [\"python3\", \"tally.py\"]\ntimeout = 10\nshared = [\"counter\"]\n\n[[entry]]\nnamespace = \"worker\"\nname = \"tally\"\ndescription = \"Counts this record's calls.\"\ninput = \"any.json\"\ncapabilities = [\"graph.read\"]\nhints = { read_only = true }\n";

const TALLY_SCRIPT: &str = r#"
import json, socket, sys

handed = json.load(sys.stdin)

with socket.socket(socket.AF_UNIX) as line:
    line.connect(handed["shared"]["counter"])
    count = line.makefile().readline().strip()

print(json.dumps({"content": [{"type": "text", "text": count}]}))
"#;

const CHATTY_MANIFEST: &str = "[tool]\nname = \"chatty\"\nprotocol = 1\nrun = [\"python3\", \"chatty.py\"]\nmode = \"server\"\ntimeout = 10\n\n[[entry]]\nnamespace = \"worker\"\nname = \"chatty\"\ndescription = \"Says which server answered.\"\ninput = \"any.json\"\ncapabilities = [\"graph.read\"]\nhints = { read_only = true }\n";

// A server speaking MCP over its standard input and output, as little of it as a call needs.
const CHATTY_SCRIPT: &str = r#"
import json, os, sys

start = json.loads(os.environ["BMO_TOOL_START"])
calls = 0

for line in sys.stdin:
    message = json.loads(line)

    if "id" not in message or "method" not in message:
        continue

    if message["method"] == "initialize":
        result = {"protocolVersion": message["params"]["protocolVersion"], "capabilities": {"tools": {}}, "serverInfo": {"name": "chatty", "version": "1"}}
    elif message["method"] == "tools/call":
        calls += 1
        result = {"content": [{"type": "text", "text": f"{start['session']['line']}:{os.getpid()}:{calls}"}]}
    else:
        result = {}

    print(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}), flush=True)
"#;

// Whether a process of this id still runs.
fn alive(pid: &str) -> bool {
  Path::new("/proc").join(pid.trim()).exists()
}

#[tokio::test]
async fn a_shared_helper_serves_one_records_calls_and_ends_with_it_and_a_server_tool_keeps_one_process_per_record()
 {
  let scratch = Scratch::new();
  let host = scratch.start();
  let any = r#"{ "type": "object" }"#;
  let one = issue(&host, &record(scratch.data.path(), &["graph.read"], json!({ "line": "one" }))).await;
  let two = issue(&host, &record(scratch.data.path(), &["graph.read"], json!({ "line": "two" }))).await;

  place(
    &scratch.tools(),
    "_shared/counter",
    &[("tool.toml", COUNTER_MANIFEST), ("counter.py", COUNTER_SCRIPT)],
  );
  place(
    &scratch.tools(),
    "tally",
    &[("tool.toml", TALLY_MANIFEST), ("any.json", any), ("tally.py", TALLY_SCRIPT)],
  );
  place(
    &scratch.tools(),
    "chatty",
    &[("tool.toml", CHATTY_MANIFEST), ("any.json", any), ("chatty.py", CHATTY_SCRIPT)],
  );
  let (at, held) = (&host, one.as_str());

  until(
    "the placed tools",
    move || async move { book(at, held).await.contains("worker/chatty") },
  )
  .await;

  assert_eq!(text(&through(&host, &one, "call", "worker/tally", json!({})).await), "1");
  assert_eq!(text(&through(&host, &one, "call", "worker/tally", json!({})).await), "2");
  assert_eq!(text(&through(&host, &two, "call", "worker/tally", json!({})).await), "1");

  let first = text(&through(&host, &one, "call", "worker/chatty", json!({})).await);
  let (served, count) = first.rsplit_once(':').unwrap();

  assert!(served.starts_with("one:") && count == "1", "{first}");
  assert_eq!(
    text(&through(&host, &one, "call", "worker/chatty", json!({})).await),
    format!("{served}:2")
  );
  assert!(text(&through(&host, &two, "call", "worker/chatty", json!({})).await).starts_with("two:"));

  // Revoked, the record's helper ends; the other record's goes on.
  let helper_one = fs::read_to_string(scratch.data.path().join("helper-one.pid")).unwrap();
  let helper_two = fs::read_to_string(scratch.data.path().join("helper-two.pid")).unwrap();
  let revoked = request(&host.control, Method::DELETE, &format!("/sessions/{one}"), None).await.unwrap();

  assert_eq!(revoked.status, StatusCode::NO_CONTENT);
  let ended = helper_one.as_str();

  until("the helper's end", move || async move { !alive(ended) }).await;
  assert!(alive(&helper_two));
  assert_eq!(text(&through(&host, &two, "call", "worker/tally", json!({})).await), "2");
}
