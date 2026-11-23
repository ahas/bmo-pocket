//! The tool host as a session meets it: the binary started on a scratch data root, tool sessions issued on its
//! control socket with its issuer token, and the protocol spoken to it over HTTP (docs/mcp-host.md, 「Tests」).

use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bmo_pocket_client::transport::{Reply, request};
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpStream;

// A host running on its own data root, with its sockets in a directory short enough to bind.
struct Running {
  child: Child,
  port: u16,
  control: PathBuf,
  issuer: String,
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

  // The deploy tree's `tools/` standing in, which the host reads and watches.
  fn tools(&self) -> PathBuf {
    self.data.path().join("tools")
  }

  // Where the host keeps its issuer token.
  fn issuer_file(&self) -> PathBuf {
    self.data.path().join("tool-host/issuer-token")
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
      .env_remove("RECEIVER_ADDRESS")
      .env_remove("RECEIVER_TOKEN")
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
      .unwrap_or_else(|| panic!("the host did not start: {line}\n{}", self.log()));
    let issuer = fs::read_to_string(self.issuer_file()).unwrap().trim().to_owned();

    Running {
      child,
      port,
      control,
      issuer,
    }
  }
}

// The garden's, the queue's and the desk's tools standing in, placed where the host reads them (docs/mcp-host.md,
// 「Placed tools」), each entry by its own name and capability: a garden entry answers with the plot its session's
// variables name, the desk's entry with the desk its session's labels name.
const READERS_MANIFEST: &str = r#"
[tool]
name = "readers"
protocol = 1
language = "python"
run = ["python3", "readers.py"]
timeout = 20

[[entry]]
namespace = "garden"
name = "plant_search"
description = "Finds the plants a word is about, as id and title."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "plant_get"
description = "One plant."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "plant_list"
description = "Asked with a bed, that bed's plants. Asked with none, the beds."
manual = "manuals/plant_list.md"
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "plant_impact"
description = "What else a change to one plant would reach."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "queue_list"
description = "The queue in its own order."
input = "any.json"
capabilities = ["queue.read"]
hints = { read_only = true }

[[entry]]
namespace = "desk"
name = "desk_owed"
description = "The pieces still owed a row."
input = "any.json"
capabilities = ["desk.read"]
hints = { read_only = true }
"#;

const READERS_SCRIPT: &str = r#"
import json, os, sys

handed = json.load(sys.stdin)

if handed["entry"].startswith("desk/"):
    said = "owed on " + handed["session"]["labels"]["desk"]
else:
    said = "plot/" + os.environ["PLOT"] + "  " + os.environ["PLOT"]

print(json.dumps({"content": [{"type": "text", "text": said}], "isError": False}))
"#;

// The manual the readers' plant_list names, in a folder of the tool's own.
const PLANT_LIST_MANUAL: &str = "# plant_list\n\nPass `bed`, a bed's name, for that bed's plants.\n";

// The stand-in readers placed in the tree a host about to start reads.
fn place_readers(scratch: &Scratch) {
  place(
    &scratch.tools(),
    "readers",
    &[
      ("tool.toml", READERS_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("manuals/plant_list.md", PLANT_LIST_MANUAL),
      ("readers.py", READERS_SCRIPT),
    ],
  );
}

// Every namespace the stand-ins declare, which a record names unless `more` says otherwise.
const NAMESPACES: [&str; 6] = ["desk", "echoes", "garden", "notes", "papers", "web"];

// The entries a holder's sessions are shown by name, as the issuer of their records resolves them: a gardener three
// garden entries, a scribe a filing and a reading entry, and every other holder none.
fn shown_by(holder: &str) -> Value {
  match holder {
    "gardener" => json!(["garden/plant_get", "garden/plant_impact", "garden/plant_search"]),
    "scribe" => json!(["notes/note_add", "notes/status"]),
    _ => json!([]),
  }
}

// A record as its issuer asks for it, a gardener's holding no grant and shown by name what its holder's sessions
// are, unless `more` says otherwise.
fn record(capabilities: &[&str], more: Value) -> Value {
  let mut said = json!({ "holderName": "gardener", "namespaces": NAMESPACES, "capabilities": capabilities, "grants": [] });

  said.as_object_mut().unwrap().extend(more.as_object().unwrap().clone());

  if said.get("shown").is_none() {
    said["shown"] = shown_by(said["holderName"].as_str().unwrap());
  }

  said
}

// One request on the control socket, with the token where one is given.
async fn control(
  host: &Running,
  method: Method,
  path: &str,
  token: Option<&str>,
  body: Option<&Value>,
) -> Reply {
  request(&host.control, method, path, token, body).await.unwrap()
}

// One request on the control socket with the issuer token, its answer read as JSON.
async fn issued_ask(host: &Running, method: Method, path: &str) -> Value {
  let reply = control(host, method, path, Some(&host.issuer), None).await;

  assert_eq!(reply.status, StatusCode::OK, "{}", String::from_utf8_lossy(&reply.body));
  reply.json().unwrap()
}

async fn issue(host: &Running, session: &Value) -> String {
  let reply = control(host, Method::POST, "/tool-sessions", Some(&host.issuer), Some(session)).await;

  assert_eq!(reply.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&reply.body));
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
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

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

  let response = link
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
async fn two_sessions_each_hand_their_own_variables_and_labels_through_one_host() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let one = issue(
    &host,
    &record(
      &["garden.read", "desk.read"],
      json!({ "variables": { "PLOT": "left" }, "labels": { "desk": "left-desk.json" } }),
    ),
  )
  .await;
  let other = issue(
    &host,
    &record(
      &["garden.read", "desk.read"],
      json!({ "variables": { "PLOT": "right" }, "labels": { "desk": "right-desk.json" } }),
    ),
  )
  .await;

  assert_eq!(
    text(&called(&host, &one, "garden", "plant_search", json!({ "text": "plot" })).await),
    "plot/left  left"
  );
  assert_eq!(
    text(&called(&host, &other, "garden", "plant_search", json!({ "text": "plot" })).await),
    "plot/right  right"
  );

  let owed = text(&called(&host, &one, "desk", "desk_owed", json!({})).await);

  assert!(owed.contains("left-desk.json") && !owed.contains("right-desk.json"), "{owed}");
}

// A variables file is read afresh at every call and laid over the record's own variables.
#[tokio::test]
async fn a_records_variables_file_is_read_at_every_call_over_its_variables() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let file = scratch.data.path().join("variables.json");
  let token = issue(
    &host,
    &record(&["garden.read"], json!({ "variables": { "PLOT": "own" }, "variablesFile": file })),
  )
  .await;
  let plot = || async { text(&called(&host, &token, "garden", "plant_get", json!({})).await) };

  assert_eq!(plot().await, "plot/own  own");

  fs::write(&file, json!({ "PLOT": "moved" }).to_string()).unwrap();
  assert_eq!(plot().await, "plot/moved  moved");

  fs::write(&file, json!({ "PLOT": "again" }).to_string()).unwrap();
  assert_eq!(plot().await, "plot/again  again");
}

#[tokio::test]
async fn a_session_is_shown_and_may_call_only_what_its_namespaces_and_capabilities_open() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let token = issue(&host, &record(&["garden.read"], json!({ "variables": { "PLOT": "only" } }))).await;

  // The booked one of the four is the toolbook's to list.
  assert_eq!(listed(&host, &token, "garden").await, ["plant_get", "plant_impact", "plant_search"]);
  assert!(listed(&host, &token, "desk").await.is_empty());

  assert!(!text(&called(&host, &token, "garden", "plant_list", json!({})).await).is_empty());

  let refused = called(&host, &token, "garden", "queue_list", json!({})).await;

  assert_eq!(refused["error"]["code"], -32602);
  assert_eq!(refused["error"]["message"], "Tool queue_list not found");

  // A record not naming a namespace reaches nothing of it, though it holds the capability.
  let elsewhere =
    issue(&host, &record(&["garden.read", "desk.read"], json!({ "namespaces": ["desk"] }))).await;

  assert_eq!(
    rpc(&host, Some(&elsewhere), "garden", "tools/list", json!({})).await.0,
    StatusCode::NOT_FOUND
  );
  assert!(!book(&host, &elsewhere).await.contains("garden/"));
}

// "An entry is resident for a session where its record's `shown` names it" (docs/toolbook.md, 「Which tools are
// resident」): a gardener's record names three garden entries, and a seeker's names none.
#[tokio::test]
async fn a_placed_entry_is_listed_where_its_sessions_record_shows_it_and_booked_where_it_does_not() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let host = scratch.start();
  let on = |holder: &str| {
    record(&["garden.read"], json!({ "holderName": holder, "variables": { "PLOT": "beds" } }))
  };
  let gardener = issue(&host, &on("gardener")).await;
  let seeker = issue(&host, &on("seeker")).await;

  assert_eq!(listed(&host, &gardener, "garden").await, ["plant_get", "plant_impact", "plant_search"]);
  assert!(!book(&host, &gardener).await.contains("garden/plant_search"));
  assert!(listed(&host, &seeker, "garden").await.is_empty());

  let booked = book(&host, &seeker).await;

  for entry in ["plant_get", "plant_impact", "plant_list", "plant_search"] {
    assert!(
      booked.lines().any(|line| line.starts_with(&format!("garden/{entry} · call — "))),
      "{booked}"
    );
  }

  assert_eq!(
    text(&through(&host, &seeker, "call", "garden/plant_search", json!({ "text": "beds" })).await),
    "plot/beds  beds"
  );
  assert_eq!(
    text(&called(&host, &gardener, "garden", "plant_search", json!({ "text": "beds" })).await),
    "plot/beds  beds"
  );
}

#[tokio::test]
async fn a_request_with_no_token_an_unknown_one_the_issuers_or_an_unknown_namespace_is_turned_away() {
  let scratch = Scratch::new();
  let host = scratch.start();
  let token = issue(&host, &record(&["garden.read"], json!({}))).await;

  assert_eq!(rpc(&host, None, "garden", "tools/list", json!({})).await.0, StatusCode::UNAUTHORIZED);
  assert_eq!(
    rpc(&host, Some("nope"), "garden", "tools/list", json!({})).await.0,
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(
    rpc(&host, Some(&host.issuer), "garden", "tools/list", json!({})).await.0,
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(
    rpc(&host, Some(&token), "plants", "tools/list", json!({})).await.0,
    StatusCode::NOT_FOUND
  );
}

// "The issuer token alone issues and revokes" (docs/mcp-host.md, 「Tests」): a record that is not one is refused
// where it is issued, and a session's own token issues nothing.
#[tokio::test]
async fn the_issuer_token_alone_issues_and_a_record_that_is_not_one_is_refused() {
  let scratch = Scratch::new();
  let host = scratch.start();
  let malformed = control(
    &host,
    Method::POST,
    "/tool-sessions",
    Some(&host.issuer),
    Some(&json!({ "holderName": "gardener", "scope": 5 })),
  )
  .await;

  assert_eq!(malformed.status, StatusCode::BAD_REQUEST);
  assert_eq!(malformed.json().unwrap()["code"], "bad-request");

  let asked = record(&["garden.read"], json!({ "ttlMs": 60_000 }));
  let answered = control(&host, Method::POST, "/tool-sessions", Some(&host.issuer), Some(&asked)).await;
  let session = answered.json().unwrap();

  assert_eq!(answered.status, StatusCode::CREATED);
  assert_eq!(session["holderName"], "gardener");
  assert_eq!(session["expiresAt"], session["createdAt"].as_u64().unwrap() + 60_000);

  let own = session["token"].as_str().unwrap();
  let widened = control(&host, Method::POST, "/tool-sessions", Some(own), Some(&asked)).await;

  assert_eq!(widened.status, StatusCode::FORBIDDEN);
  assert_eq!(widened.json().unwrap()["code"], "forbidden");
  assert_eq!(
    control(&host, Method::POST, "/tool-sessions", None, Some(&asked)).await.status,
    StatusCode::UNAUTHORIZED
  );
}

#[tokio::test]
async fn a_revoked_token_and_a_lapsed_session_reach_nothing_and_leave_no_record() {
  let scratch = Scratch::new();
  let host = scratch.start();
  let revoked = issue(&host, &record(&["garden.read"], json!({}))).await;
  let lapsed = issue(&host, &record(&["garden.read"], json!({ "ttlMs": 300 }))).await;
  let sessions = scratch.data.path().join("tool-host/sessions");
  let revoke = |token: String| {
    let host = &host;

    async move {
      control(host, Method::DELETE, &format!("/tool-sessions/{token}"), Some(&host.issuer), None)
        .await
        .status
    }
  };

  assert_eq!(fs::read_dir(&sessions).unwrap().count(), 2);
  assert_eq!(revoke(revoked.clone()).await, StatusCode::NO_CONTENT);
  assert_eq!(revoke(revoked.clone()).await, StatusCode::NOT_FOUND);
  assert_eq!(
    rpc(&host, Some(&revoked), "garden", "tools/list", json!({})).await.0,
    StatusCode::UNAUTHORIZED
  );

  tokio::time::sleep(Duration::from_millis(400)).await;

  assert_eq!(
    rpc(&host, Some(&lapsed), "garden", "tools/list", json!({})).await.0,
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
  let token = issue(&host, &record(&["garden.read"], json!({ "variables": { "PLOT": "told" } }))).await;
  let search = json!({ "name": "plant_search", "arguments": { "text": "told" } });

  for method in ["tools/list", "resources/list"] {
    let (_, listed) = rpc_on("2026-07-28", &host, Some(&token), "garden", method, json!({})).await;

    assert_eq!(listed["result"]["resultType"], "complete", "{method}: {listed}");
    assert_eq!(
      (&listed["result"]["ttlMs"], &listed["result"]["cacheScope"]),
      (&json!(0), &json!("private")),
      "{method}: {listed}"
    );
  }

  let (_, called) = rpc_on("2026-07-28", &host, Some(&token), "garden", "tools/call", search.clone()).await;

  assert_eq!(called["result"]["resultType"], "complete", "{called}");

  let (_, earlier) = rpc(&host, Some(&token), "garden", "tools/list", json!({})).await;

  assert!(earlier["result"].get("resultType").is_none(), "{earlier}");
}

#[tokio::test]
async fn records_and_the_issuer_token_survive_a_restart_and_the_same_tokens_work_after_it() {
  let scratch = Scratch::new();

  place_readers(&scratch);

  let (token, issuer) = {
    let host = scratch.start();

    (
      issue(&host, &record(&["garden.read"], json!({ "variables": { "PLOT": "kept" } }))).await,
      host.issuer.clone(),
    )
  };
  let host = scratch.start();
  let file = fs::read_dir(scratch.data.path().join("tool-host/sessions"))
    .unwrap()
    .next()
    .unwrap()
    .unwrap();

  assert_eq!(host.issuer, issuer);
  assert_eq!(
    text(&called(&host, &token, "garden", "plant_search", json!({ "text": "kept" })).await),
    "plot/kept  kept"
  );
  assert!(!file.file_name().to_string_lossy().contains(&token));
  assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
  assert_eq!(fs::metadata(scratch.issuer_file()).unwrap().permissions().mode() & 0o777, 0o600);
  assert_eq!(issued_ask(&host, Method::GET, "/health").await["ready"], true);
}

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
namespace = "papers"
capabilities = ["papers.read"]
"#;

const PAPERS_SCRIPT: &str = r#"
import json, sys, time

asked = json.load(sys.stdin)
open(f"{CALLS}/{time.time_ns()}.in", "w").write(json.dumps(asked))

if asked["kind"] == "resources_list":
    print(json.dumps({"resources": [{"uri": "papers://procedure/carried", "name": "carried", "mimeType": "text/markdown"}]}))
elif asked["uri"] == "papers://procedure/carried":
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
// input, and a namespace serving resources alone holds no tool the control socket answers.
#[tokio::test]
async fn resources_are_listed_and_read_by_their_placed_tool_handed_the_record_and_the_request_on_standard_input()
 {
  let scratch = Scratch::new();

  place_papers(&scratch);

  let host = scratch.start();
  let reader = issue(&host, &record(&["papers.read"], json!({ "holderName": "seeker" }))).await;
  let stranger = issue(&host, &record(&["garden.read"], json!({}))).await;
  let (_, resources) = rpc(&host, Some(&reader), "papers", "resources/list", json!({})).await;

  assert_eq!(resources["result"]["resources"][0]["uri"], "papers://procedure/carried");

  let (_, read) = rpc(
    &host,
    Some(&reader),
    "papers",
    "resources/read",
    json!({ "uri": "papers://procedure/carried" }),
  )
  .await;

  assert_eq!(read["result"]["contents"][0]["text"], CARRIED);

  let (_, refused) = rpc(
    &host,
    Some(&reader),
    "papers",
    "resources/read",
    json!({ "uri": "papers://procedure/missing" }),
  )
  .await;

  assert_eq!(refused["error"]["message"], "no procedure named papers://procedure/missing");

  let calls = handed(&scratch);

  assert_eq!(calls.len(), 3);
  assert_eq!(calls[0]["kind"], "resources_list");
  assert_eq!(calls[0]["session"]["holderName"], "seeker");
  assert!(!calls[0].to_string().contains(&reader));
  assert_eq!(
    (&calls[1]["kind"], &calls[1]["uri"]),
    (&json!("resources_read"), &json!("papers://procedure/carried"))
  );
  assert!(
    rpc(&host, Some(&stranger), "papers", "resources/list", json!({})).await.1["result"]["resources"]
      .as_array()
      .unwrap()
      .is_empty()
  );
  assert_eq!(handed(&scratch).len(), 3);
  assert_eq!(
    control(&host, Method::GET, "/namespaces/papers", Some(&host.issuer), None).await.status,
    StatusCode::NOT_FOUND
  );
}

// A placed sending entry, declaring the verb it needs and its icon beside it.
const STAMPER_MANIFEST: &str = r#"
[tool]
name = "stamper"
protocol = 1
language = "python"
run = ["python3", "stamper.py"]
timeout = 20

[[entry]]
namespace = "notes"
name = "stamp"
description = "Sends a stamp."
input = "any.json"
capabilities = ["notes.use"]
hints = { open_world = true }

[entry.facts]
verb = "stamp"
icon = "i-lucide-stamp"
"#;

// A placed filing entry, under a name a scribe's record shows: it carries a grant, a bundle's narrower form and the
// record its call leaves, and its executable answers every call alike.
const FILER_MANIFEST: &str = r#"
[tool]
name = "filer"
protocol = 1
language = "python"
run = ["python3", "filer.py"]
timeout = 20

[[entry]]
namespace = "notes"
name = "note_add"
description = "Files a row."
input = "note_add.json"
capabilities = ["notes.use"]
grant = "notes.file"

[entry.bundle."notes.brief"]
shown = ["title", "text"]
required = ["title", "text"]

[entry.facts]
record = "note-add"
"#;

const FILING_SCHEMA: &str = r#"{ "type": "object", "properties": { "title": { "type": "string" }, "text": { "type": "string" }, "scope": { "type": "integer" } }, "required": ["title"], "additionalProperties": false }"#;

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
      ("note_add.json", FILING_SCHEMA),
      ("filer.py", FILER_SCRIPT),
    ],
  );
}

// Placed readings: `status`, which a scribe's record shows, and `history`, which no record shows, each read-only and
// answering with the reading's object; its executable answers every call alike.
const READINGS_MANIFEST: &str = r#"
[tool]
name = "readings"
protocol = 1
language = "python"
run = ["python3", "readings.py"]
timeout = 20

[[entry]]
namespace = "notes"
name = "status"
description = "The live work, as it stands."
input = "status.json"
output = "reading.json"
capabilities = ["notes.use"]
hints = { read_only = true }

[[entry]]
namespace = "notes"
name = "history"
description = "The history, newest first."
input = "history.json"
output = "reading.json"
capabilities = ["notes.use"]
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
        "status.json",
        r#"{ "type": "object", "properties": { "full": { "type": "boolean" } }, "additionalProperties": false }"#,
      ),
      (
        "history.json",
        r#"{ "type": "object", "properties": { "scope": { "type": "integer" }, "limit": { "type": "integer" }, "full": { "type": "boolean" } }, "additionalProperties": false }"#,
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
    "stamper",
    &[
      ("tool.toml", STAMPER_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("stamper.py", ""),
    ],
  );
  place_filer(&scratch);

  let host = scratch.start();
  let notes = issued_ask(&host, Method::GET, "/namespaces/notes").await;
  let facts = &notes["facts"];
  let named = |field: &str, value: &str| -> Vec<(String, String)> {
    facts[field]
      .as_array()
      .unwrap()
      .iter()
      .map(|one| (one["name"].as_str().unwrap().to_owned(), one[value].as_str().unwrap().to_owned()))
      .collect()
  };

  assert_eq!((notes["name"].as_str(), notes["toolCount"].as_u64()), (Some("notes"), Some(2)));
  assert_eq!(facts["digest"].as_str().unwrap().len(), 64);
  assert_eq!(named("sending", "verb"), [("stamp".to_owned(), "stamp".to_owned())]);
  assert_eq!(named("records", "record"), [("note_add".to_owned(), "note-add".to_owned())]);
  assert!(named("icons", "icon").contains(&("stamp".to_owned(), "i-lucide-stamp".to_owned())));
  assert_eq!(
    control(&host, Method::GET, "/namespaces/garden", Some(&host.issuer), None).await.status,
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

// A session whose `expiresAt` has passed with nothing calling it: the beat drops its record, and what the record kept
// ends with it, while a session standing until revoked stands.
#[tokio::test]
async fn a_lapsed_session_is_dropped_on_the_next_beat_though_nothing_calls_it() {
  let scratch = Scratch::new();
  let host = scratch.start_with(&["--sweep-every-ms", "100"]);
  let sessions = scratch.data.path().join("tool-host/sessions");

  issue(&host, &record(&["web.read"], json!({}))).await;
  let standing_id = appeared(&scratch, &[]);
  issue(&host, &record(&["web.read"], json!({ "ttlMs": 1 }))).await;
  let lapsed_id = appeared(&scratch, std::slice::from_ref(&standing_id));
  let bound = std::time::Instant::now() + Duration::from_secs(30);

  while sessions.join(format!("{lapsed_id}.json")).exists() {
    assert!(std::time::Instant::now() < bound, "the lapsed session's record was never dropped");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }

  assert!(sessions.join(format!("{standing_id}.json")).exists());
}

// "A call of an entry declaring a grant runs where the record's grants hold it" (docs/mcp-host.md, 「Grants」), on the
// tools face and on the control socket alike; an entry declaring none runs for any session holding it.
#[tokio::test]
async fn an_entry_declaring_a_grant_runs_only_where_the_records_grants_hold_it() {
  let scratch = Scratch::new();

  place_filer(&scratch);
  place_readings(&scratch);

  let host = scratch.start();
  let scribe = |grants: Value| record(&["notes.use"], json!({ "holderName": "scribe", "grants": grants }));
  let ungranted = issue(&host, &scribe(json!([]))).await;
  let elsewhere = issue(&host, &scribe(json!([{ "resource": "notes" }]))).await;
  let granted = issue(&host, &scribe(json!([{ "resource": "notes.file" }]))).await;
  let refused = called(&host, &ungranted, "notes", "note_add", json!({ "title": "x" })).await;

  assert_eq!(refused["result"]["isError"], true);
  assert!(text(&refused).contains("the session holds no grant for notes.file"), "{refused}");
  assert_eq!(
    called(&host, &elsewhere, "notes", "note_add", json!({ "title": "x" })).await["result"]["isError"],
    true
  );
  assert_eq!(
    text(&called(&host, &granted, "notes", "note_add", json!({ "title": "x" })).await),
    FILED
  );
  assert_eq!(text(&called(&host, &ungranted, "notes", "status", json!({})).await), CARRIED);

  let call = json!({ "namespace": "notes", "name": "note_add", "input": { "title": "x" } });
  let denied = control(&host, Method::POST, "/tool-calls", Some(&ungranted), Some(&call)).await;

  assert_eq!(denied.status, StatusCode::FORBIDDEN);
  assert_eq!(denied.json().unwrap()["code"], "forbidden");

  let made = control(&host, Method::POST, "/tool-calls", Some(&granted), Some(&call)).await;
  let made = made.json().unwrap();

  assert_eq!((&made["namespace"], &made["name"]), (&json!("notes"), &json!("note_add")));
  assert_eq!(made["result"]["content"][0]["text"], FILED);
  assert!(made["spentMs"].is_u64());
}

// The tools a session is shown, each by the schema of the form it holds.
async fn shown(host: &Running, token: &str) -> Vec<(String, Value)> {
  rpc(host, Some(token), "notes", "tools/list", json!({})).await.1["result"]["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|tool| (tool["name"].as_str().unwrap().to_owned(), tool["inputSchema"].clone()))
    .collect()
}

// A record holding bundles alone is shown and served their entries alone, each schema narrowed to what its entry
// shows and each call held to it; one holding the whole catalogue as well is shown each entry once and whole.
#[tokio::test]
async fn a_record_holding_a_bundle_alone_is_shown_its_tools_narrowed_and_one_holding_both_each_whole() {
  let scratch = Scratch::new();

  place_filer(&scratch);
  place_readings(&scratch);

  let host = scratch.start();
  let both = issue(
    &host,
    &record(&["notes.use", "notes.brief", "notes.send"], json!({ "holderName": "scribe" })),
  )
  .await;
  let bundles =
    issue(&host, &record(&["notes.brief", "notes.send"], json!({ "holderName": "scribe" }))).await;
  let whole = shown(&host, &both).await;
  let narrowed = shown(&host, &bundles).await;
  let names = |tools: &[(String, Value)]| tools.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>();

  assert_eq!(names(&whole).iter().filter(|name| *name == "note_add").count(), 1);
  assert!(names(&whole).contains(&"status".to_owned()));
  assert!(
    whole
      .iter()
      .any(|(name, schema)| name == "note_add" && schema["properties"]["scope"].is_object())
  );
  assert_eq!(names(&narrowed), ["note_add"]);

  let filing = &narrowed[0].1;
  let mut offered: Vec<&String> = filing["properties"].as_object().unwrap().keys().collect();

  offered.sort();
  assert_eq!(offered, ["text", "title"]);
  assert_eq!(filing["required"], json!(["title", "text"]));

  // A bundle's form is held to its own arguments, and acts under no grant.
  let refused =
    called(&host, &bundles, "notes", "note_add", json!({ "title": "x", "text": "y", "scope": 5 })).await;

  assert_eq!(refused["result"]["isError"], true);
  assert!(
    text(&refused).contains("was not called: its arguments do not fit its schema"),
    "{refused}"
  );
  assert_eq!(
    text(&called(&host, &bundles, "notes", "note_add", json!({ "title": "x", "text": "y" })).await),
    FILED
  );
  assert_eq!(called(&host, &bundles, "notes", "status", json!({})).await["error"]["code"], -32602);
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
  let seeker = issue(
    &host,
    &record(
      &["notes.use", "garden.read"],
      json!({ "holderName": "seeker", "variables": { "PLOT": "book" }, "grants": [{ "resource": "notes.file" }] }),
    ),
  )
  .await;

  // A seeker's record shows none of the `notes` namespace's entries, so every one it holds is booked.
  assert!(listed(&host, &seeker, "notes").await.is_empty());
  assert_eq!(listed(&host, &seeker, "toolbook").await, ["list", "describe", "call", "act"]);

  let listing = called(&host, &seeker, "toolbook", "list", json!({})).await;
  let book = text(&listing);

  for line in [
    "notes/history · call — The history, newest first.",
    "notes/status · call — The live work, as it stands.",
    "notes/note_add · act — Files a row.",
    "garden/plant_list · call — Asked with a bed, that bed's plants.",
  ] {
    assert!(book.lines().any(|listed| listed == line), "{line}\n{book}");
  }

  // Beside its lines, the list answers each tool as an object a program reads.
  let items = listing["result"]["structuredContent"]["items"].as_array().unwrap();

  assert_eq!(items.len(), book.lines().count());
  assert!(
    items.contains(&json!({ "tool": "notes/note_add", "reachedBy": "act", "summary": "Files a row." })),
    "{listing}"
  );

  let narrowed = called(&host, &seeker, "toolbook", "list", json!({ "query": "NEWEST" })).await;

  assert_eq!(text(&narrowed), "notes/history · call — The history, newest first.");
  assert_eq!(
    narrowed["result"]["structuredContent"],
    json!({
      "items": [{ "tool": "notes/history", "reachedBy": "call", "summary": "The history, newest first." }]
    })
  );

  let described =
    text(&called(&host, &seeker, "toolbook", "describe", json!({ "tool": "notes/history" })).await);

  assert!(
    described.starts_with(
      "notes/history — reached with call\nThe history, newest first.\nRead-only: yes\nInput schema: "
    ),
    "{described}"
  );
  assert!(described.contains("\nAnswers with: {"), "{described}");
  assert!(!described.contains("Manual:"), "{described}");

  // A tool whose manifest names a manual is described with its full text after the description.
  let planted =
    called(&host, &seeker, "toolbook", "describe", json!({ "tool": "garden/plant_list" })).await;
  let manual = text(&planted);

  assert!(
    manual.starts_with(
      "garden/plant_list — reached with call\nAsked with a bed, that bed's plants. Asked with none, the beds.\n\
       Manual:\n# plant_list\n\nPass `bed`, a bed's name, for that bed's plants.\nRead-only: yes\n"
    ),
    "{manual}"
  );

  // The description is also the tool in the control socket's shape, as the session's token reads it there.
  let shaped = &planted["result"]["structuredContent"];
  let reply = control(&host, Method::GET, "/namespaces/garden/tools/plant_list", Some(&seeker), None).await;

  assert_eq!(reply.status, StatusCode::OK, "{}", String::from_utf8_lossy(&reply.body));
  assert_eq!(*shaped, reply.json().unwrap());
  assert_eq!(shaped["reachedBy"], "call");
  assert_eq!(shaped["manual"], PLANT_LIST_MANUAL);
  let acting = called(&host, &seeker, "toolbook", "describe", json!({ "tool": "notes/note_add" })).await;

  assert_eq!(acting["result"]["structuredContent"]["reachedBy"], "act");

  // A booked tool answers through the toolbook and by its own name alike.
  assert_eq!(
    text(&through(&host, &seeker, "call", "notes/history", json!({ "limit": 3 })).await),
    CARRIED
  );
  assert_eq!(text(&called(&host, &seeker, "notes", "history", json!({ "limit": 3 })).await), CARRIED);
  assert!(text(&through(&host, &seeker, "call", "garden/plant_list", json!({})).await).contains("book"));

  // Arguments that do not fit are refused before anything runs, naming each and carrying the schema.
  let misfit = through(&host, &seeker, "call", "notes/history", json!({ "limit": "three" })).await;

  assert_eq!(misfit["result"]["isError"], true);
  assert!(
    text(&misfit)
      .starts_with("ERROR: notes/history was not called: its arguments do not fit its schema.\n- /limit: "),
    "{misfit}"
  );
  assert!(text(&misfit).contains("Its schema: {"), "{misfit}");

  // A tool that acts is reached with act alone.
  let refused = through(&host, &seeker, "call", "notes/note_add", json!({ "title": "x" })).await;

  assert_eq!(
    text(&refused),
    "ERROR: notes/note_add acts, so call does not reach it: call it with act."
  );
  assert_eq!(
    text(&through(&host, &seeker, "act", "notes/note_add", json!({ "title": "x" })).await),
    FILED
  );

  // A tool the session does not hold is refused by the words a direct call is.
  assert_eq!(
    text(&through(&host, &seeker, "call", "web/nope", json!({})).await),
    "ERROR: Tool web/nope not found"
  );
  assert_eq!(
    text(&called(&host, &seeker, "toolbook", "describe", json!({ "tool": "nope" })).await),
    "ERROR: Tool nope not found"
  );

  // A reader holds web.read alone: none of the `notes` namespace's entries is in a toolbook of its.
  let reader = issue(&host, &record(&["web.read"], json!({ "holderName": "reader" }))).await;

  assert_eq!(
    text(&called(&host, &reader, "toolbook", "list", json!({})).await),
    "No tool is reached through the toolbook in this session."
  );
  assert_eq!(
    text(&through(&host, &reader, "call", "notes/history", json!({})).await),
    "ERROR: Tool notes/history not found"
  );
}

// A placed tool's folder (docs/mcp-host.md, 「Placed tools」): written beside where it is placed, then put in
// place in one rename, as the deployment places one.
fn place(tools: &Path, folder: &str, files: &[(&str, &str)]) {
  let staged = tools.join(folder).join(".placing");

  fs::create_dir_all(&staged).unwrap();

  for (name, text) in files {
    let file = staged.join(name);

    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
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
namespace = "garden"
name = "echo_back"
description = "Says the text back. Then more."
input = "echo.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "notes"
name = "echo_note"
description = "Notes the text."
input = "note.json"
capabilities = ["notes.use"]
grant = "notes.echo"

[entry.facts]
icon = "i-lucide-notebook"

[[entry]]
namespace = "garden"
name = "echo_hidden"
description = "Shown to a web reader alone."
input = "echo.json"
capabilities = ["web.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "echo_quiet"
description = "Says the text back to the system alone."
input = "echo.json"
system = true
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "echo_slow"
description = "Answers after the timeout it still carries."
input = "echo.json"
capabilities = ["garden.read"]
timeout = 1
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "echo_broken"
description = "Ends badly."
input = "echo.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "web"
name = "echo_page"
description = "Reads a page of example.org."
input = "echo.json"
capabilities = ["garden.read"]
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
capabilities = ["garden.read"]
hints = { read_only = true }
"#;

const ECHO_SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" }, "via": { "type": "string" }, "with": { "type": "object" }, "claim": { "type": "string" }, "hold": { "type": "string" } }, "required": ["text"], "additionalProperties": false }"#;

const NOTE_SCHEMA: &str =
  r#"{ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }"#;

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
    time.sleep(2)

if "hold" in arguments:
    import subprocess
    held = subprocess.Popen(["sleep", "60"])
    with open(arguments["hold"], "w") as written:
        written.write(str(held.pid))
    time.sleep(30)

if entry.endswith("echo_broken"):
    print("it broke", file=sys.stderr)
    sys.exit(3)

said = handed["session"]["holderName"] + ":" + arguments["text"]

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

  let mut host = scratch.start();
  let pid = host.child.id();
  let seeker = |grants: Value| {
    record(&["garden.read", "notes.use"], json!({ "holderName": "seeker", "grants": grants }))
  };
  let token = issue(&host, &seeker(json!([]))).await;
  let keeper = issue(&host, &seeker(json!([{ "resource": "notes.echo" }]))).await;

  assert!(!book(&host, &token).await.contains("echo_back"));
  assert_eq!(
    rpc(&host, Some(&token), "echoes", "tools/list", json!({})).await.0,
    StatusCode::NOT_FOUND
  );

  let standing = issued_ask(&host, Method::GET, "/namespaces/notes").await;

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
    move || async move { book(at, held).await.contains("garden/echo_back") },
  )
  .await;

  let listed = book(&host, &token).await;
  // Calls open and read the tool's files, which is no change to what is placed.
  let readings = || scratch.log().matches("mcp-host: read ").count();
  let read_before_calls = readings();

  assert!(
    listed.lines().any(|line| line == "garden/echo_back · call — Says the text back."),
    "{listed}"
  );
  assert!(listed.lines().any(|line| line == "notes/echo_note · act — Notes the text."), "{listed}");
  assert!(!listed.contains("echo_hidden") && !listed.contains("echo_quiet"), "{listed}");
  assert!(
    text(&called(&host, &token, "toolbook", "describe", json!({ "tool": "garden/echo_back" })).await)
      .starts_with("garden/echo_back — reached with call\nSays the text back. Then more.\nRead-only: yes\nInput schema: {")
  );
  assert_eq!(rpc(&host, Some(&token), "echoes", "tools/list", json!({})).await.0, StatusCode::OK);

  // Called through the toolbook and by its own name alike, its arguments held to the schema in its folder.
  assert_eq!(
    text(&through(&host, &token, "call", "garden/echo_back", json!({ "text": "hi" })).await),
    "seeker:hi"
  );
  assert_eq!(
    text(&called(&host, &token, "garden", "echo_back", json!({ "text": "x" })).await),
    "seeker:x"
  );
  assert!(
    text(&through(&host, &token, "call", "garden/echo_back", json!({ "text": 3 })).await)
      .contains("was not called: its arguments do not fit its schema")
  );

  // An entry that acts is reached with act alone, and runs only for a session holding its grant.
  assert!(
    text(&through(&host, &token, "call", "notes/echo_note", json!({ "text": "n" })).await)
      .contains("call it with act")
  );
  assert!(
    text(&through(&host, &token, "act", "notes/echo_note", json!({ "text": "n" })).await)
      .contains("the session holds no grant for notes.echo")
  );
  assert_eq!(
    text(&through(&host, &keeper, "act", "notes/echo_note", json!({ "text": "n" })).await),
    "seeker:n"
  );

  // Its call-back socket reaches what the session could, and the entries for the system alone.
  let via = |to: &str, with: Value| json!({ "text": "a", "via": to, "with": with });

  assert_eq!(
    text(
      &through(
        &host,
        &token,
        "call",
        "garden/echo_back",
        via("garden/echo_hidden", json!({ "text": "h" }))
      )
      .await
    ),
    "seeker:a via ERROR: Tool garden/echo_hidden not found"
  );
  assert_eq!(
    text(
      &through(
        &host,
        &token,
        "call",
        "garden/echo_back",
        via("garden/echo_quiet", json!({ "text": "q" }))
      )
      .await
    ),
    "seeker:a via seeker:q"
  );
  assert!(
    text(
      &through(
        &host,
        &token,
        "call",
        "garden/echo_back",
        via("notes/echo_note", json!({ "text": "b" }))
      )
      .await
    )
    .contains("the session holds no grant for notes.echo")
  );
  assert_eq!(
    text(
      &through(
        &host,
        &token,
        "call",
        "garden/echo_back",
        json!({ "text": "c", "claim": "https://www.example.org/p" })
      )
      .await
    ),
    r#"seeker:c claimed {"arguments": {"page": "https://www.example.org/p"}, "entry": "web/echo_page"}"#
  );

  // A call waits for its answer past a timeout its entry still carries, and one that ends badly fails saying so.
  assert_eq!(
    text(&through(&host, &token, "call", "garden/echo_slow", json!({ "text": "s" })).await),
    "seeker:s"
  );
  assert_eq!(
    text(&through(&host, &token, "call", "garden/echo_broken", json!({ "text": "b" })).await),
    "ERROR: garden/echo_broken failed: it ended with code 3: it broke"
  );
  assert_eq!(readings(), read_before_calls, "{}", scratch.log());

  // Its namespace's tools and facts carry what its manifest says.
  let page = issued_ask(&host, Method::GET, "/namespaces/web/tools/echo_page").await;

  assert_eq!(page["listing"]["line"], "echo_page: a page of example.org");

  let notes = issued_ask(&host, Method::GET, "/namespaces/notes").await;

  assert!(
    notes["facts"]["icons"]
      .as_array()
      .unwrap()
      .contains(&json!({ "name": "echo_note", "icon": "i-lucide-notebook" }))
  );
  // The digest is reckoned over every entry of the namespace the token sees, so an entry placed beside those
  // standing moves it, and a namespace none of whose entries says anything still has its digest once it holds an entry.
  assert_ne!(notes["facts"]["digest"], standing["facts"]["digest"]);
  assert_eq!(
    issued_ask(&host, Method::GET, "/namespaces/garden").await["facts"]["digest"]
      .as_str()
      .unwrap()
      .len(),
    64
  );

  // An item a placed entry adds to a list of its namespace's facts stands headed with its name.
  let web = issued_ask(&host, Method::GET, "/namespaces/web").await;

  assert_eq!(
    web["facts"]["sites"].as_array().unwrap().last().unwrap(),
    &json!({ "name": "echo_page", "site": "example.org", "reads": ["example.org"] })
  );

  let tools = issued_ask(&host, Method::GET, "/namespaces/garden/tools").await;

  assert!(
    tools["items"]
      .as_array()
      .unwrap()
      .iter()
      .any(|tool| tool["name"] == "echo_quiet" && tool["system"] == true)
  );

  // An entry for the system alone of the record's namespaces is called through the control socket.
  let quiet = json!({ "namespace": "garden", "name": "echo_quiet", "input": { "text": "s" } });
  let system = control(&host, Method::POST, "/tool-calls", Some(&token), Some(&quiet))
    .await
    .json()
    .unwrap();

  assert_eq!(system["result"]["content"][0]["text"], "seeker:s");

  // A call whose caller hangs up before it answers has every process it started ended.
  let held = scratch.data.path().join("held.pid");
  let asked = json!({ "namespace": "garden", "name": "echo_quiet", "input": { "text": "h", "hold": held.to_string_lossy() } });
  let (socket, bearer) = (host.control.clone(), token.clone());
  let hanging = tokio::spawn(async move {
    request(&socket, Method::POST, "/tool-calls", Some(&bearer), Some(&asked)).await
  });

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
  let claimed = |address: &str| {
    let asked = json!({ "resource": address });
    let (host, token) = (&host, token.clone());

    async move { control(host, Method::POST, "/claims", Some(&token), Some(&asked)).await.json().unwrap() }
  };

  assert_eq!(
    claimed("https://www.example.org/p").await,
    json!({
      "resource": "https://www.example.org/p", "granted": true, "reason": null, "namespace": "web",
      "toolName": "echo_page", "arguments": { "page": "https://www.example.org/p" },
    })
  );
  assert_eq!(
    claimed("https://example.com/p").await,
    json!({
      "resource": "https://example.com/p", "granted": false, "reason": "no tool the session holds claims it",
      "namespace": null, "toolName": null, "arguments": null,
    })
  );

  // A folder that does not read is said in the log and leaves the rest served.
  place(&scratch.tools(), "broken", &[("tool.toml", "[tool]\nname = \"broken\"\n")]);
  let watched = &scratch;

  until("the refusal in the log", move || async move {
    watched.log().contains("left out tools/broken: ")
  })
  .await;
  assert_eq!(
    text(&through(&host, &token, "call", "garden/echo_back", json!({ "text": "on" })).await),
    "seeker:on"
  );

  take_away(&scratch.tools(), "echo");
  let (at, held) = (&host, token.as_str());

  until("the tool gone", move || async move { !book(at, held).await.contains("echo_back") }).await;

  assert_eq!(
    text(&through(&host, &token, "call", "garden/echo_back", json!({ "text": "x" })).await),
    "ERROR: Tool garden/echo_back not found"
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
open(os.path.join(os.environ["PIDS"], "helper-" + record["holderName"] + ".pid"), "w").write(str(os.getpid()))
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

const TALLY_MANIFEST: &str = "[tool]\nname = \"tally\"\nprotocol = 1\nrun = [\"python3\", \"tally.py\"]\ntimeout = 10\nshared = [\"counter\"]\n\n[[entry]]\nnamespace = \"garden\"\nname = \"tally\"\ndescription = \"Counts this record's calls.\"\ninput = \"any.json\"\ncapabilities = [\"garden.read\"]\nhints = { read_only = true }\n";

const TALLY_SCRIPT: &str = r#"
import json, socket, sys

handed = json.load(sys.stdin)

with socket.socket(socket.AF_UNIX) as line:
    line.connect(handed["shared"]["counter"])
    count = line.makefile().readline().strip()

print(json.dumps({"content": [{"type": "text", "text": count}]}))
"#;

const CHATTY_MANIFEST: &str = "[tool]\nname = \"chatty\"\nprotocol = 1\nrun = [\"python3\", \"chatty.py\"]\nmode = \"server\"\ntimeout = 10\n\n[[entry]]\nnamespace = \"garden\"\nname = \"chatty\"\ndescription = \"Says which server answered.\"\ninput = \"any.json\"\ncapabilities = [\"garden.read\"]\nhints = { read_only = true }\n";

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
        result = {"content": [{"type": "text", "text": f"{start['session']['holderName']}:{os.getpid()}:{calls}"}]}
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
  let pids = scratch.data.path().to_string_lossy().into_owned();
  let one = issue(
    &host,
    &record(&["garden.read"], json!({ "holderName": "one", "variables": { "PIDS": pids } })),
  )
  .await;
  let two = issue(
    &host,
    &record(&["garden.read"], json!({ "holderName": "two", "variables": { "PIDS": pids } })),
  )
  .await;

  // A shared helper is registered like any tool, through the registry's command.
  let helper = scratch.data.path().join("counter-source");

  fs::create_dir_all(&helper).unwrap();
  fs::write(helper.join("tool.toml"), COUNTER_MANIFEST).unwrap();
  fs::write(helper.join("counter.py"), COUNTER_SCRIPT).unwrap();

  let registered = Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
    .arg("--data-root")
    .arg(scratch.data.path())
    .arg("put")
    .arg(&helper)
    .output()
    .unwrap();

  assert!(registered.status.success(), "{}", String::from_utf8_lossy(&registered.stderr));
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
    move || async move { book(at, held).await.contains("garden/chatty") },
  )
  .await;

  assert_eq!(text(&through(&host, &one, "call", "garden/tally", json!({})).await), "1");
  assert_eq!(text(&through(&host, &one, "call", "garden/tally", json!({})).await), "2");
  assert_eq!(text(&through(&host, &two, "call", "garden/tally", json!({})).await), "1");

  let first = text(&through(&host, &one, "call", "garden/chatty", json!({})).await);
  let (served, count) = first.rsplit_once(':').unwrap();

  assert!(served.starts_with("one:") && count == "1", "{first}");
  assert_eq!(
    text(&through(&host, &one, "call", "garden/chatty", json!({})).await),
    format!("{served}:2")
  );
  assert!(text(&through(&host, &two, "call", "garden/chatty", json!({})).await).starts_with("two:"));

  // Revoked, the record's helper ends; the other record's goes on.
  let helper_one = fs::read_to_string(scratch.data.path().join("helper-one.pid")).unwrap();
  let helper_two = fs::read_to_string(scratch.data.path().join("helper-two.pid")).unwrap();
  let revoked =
    control(&host, Method::DELETE, &format!("/tool-sessions/{one}"), Some(&host.issuer), None).await;

  assert_eq!(revoked.status, StatusCode::NO_CONTENT);
  let ended = helper_one.as_str();

  until("the helper's end", move || async move { !alive(ended) }).await;
  assert!(alive(&helper_two));
  assert_eq!(text(&through(&host, &two, "call", "garden/tally", json!({})).await), "2");
}

// A tool whose every call says it started, then waits for its release; its second entry declares it does not hold the
// host's stop, and its calls share a helper.
const HOLDER_MANIFEST: &str = r#"
[tool]
name = "holder"
protocol = 1
language = "python"
run = ["python3", "holder.py"]
shared = ["keeper"]

[[entry]]
namespace = "garden"
name = "hold"
description = "Holds until it is released."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "hold_past_stop"
description = "Holds until it is released, and does not hold the host's stop."
input = "any.json"
capabilities = ["garden.read"]
ends_at_stop = true
hints = { read_only = true }
"#;

const HOLDER_SCRIPT: &str = r#"
import json, os, sys, time

given = json.load(sys.stdin)["arguments"]
open(given["started"], "w").write(str(os.getpid()))

while not os.path.exists(given["released"]):
    time.sleep(0.01)

print(json.dumps({"content": [{"type": "text", "text": given["say"]}], "isError": False}))
"#;

const KEEPER_MANIFEST: &str = "[tool]\nname = \"keeper\"\nprotocol = 1\nrun = [\"python3\", \"keeper.py\"]\n\n[helper]\ndescription = \"Says when it is asked to end.\"\n";

const KEEPER_SCRIPT: &str = r#"
import json, os, signal, socket, sys

handed = json.load(sys.stdin)

def ended(*_):
    open(os.environ["KEEPER_ASKED"], "w").write("asked")
    sys.exit(0)

signal.signal(signal.SIGTERM, ended)
listener = socket.socket(socket.AF_UNIX)
listener.bind(handed["socket"])
listener.listen()
print("listening", flush=True)

while True:
    listener.accept()[0].close()
"#;

// An external stdio MCP server whose one tool holds as the holder does.
const OUTER_SERVER: &str = r#"
import json, os, sys, time

def say(message):
    print(json.dumps(dict(message, jsonrpc="2.0")), flush=True)

for line in sys.stdin:
    asked = json.loads(line)
    if "id" not in asked:
        continue
    method, params = asked.get("method"), asked.get("params") or {}
    if method == "initialize":
        say({"id": asked["id"], "result": {"protocolVersion": params["protocolVersion"],
             "capabilities": {"tools": {}}, "serverInfo": {"name": "outer", "version": "1"}}})
    elif method == "tools/list":
        say({"id": asked["id"], "result": {"tools": [{"name": "hold", "description": "Holds until it is released.",
             "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": True}}]}})
    elif method == "tools/call":
        given = params["arguments"]
        open(given["started"], "w").write(str(os.getpid()))
        while not os.path.exists(given["released"]):
            time.sleep(0.01)
        say({"id": asked["id"], "result": {"content": [{"type": "text", "text": given["say"]}], "isError": False}})
"#;

// One JSON request on the tools face, on a connection of its own, with what it answered.
async fn on_tools_face(port: u16, token: String, path: &str, body: Value) -> (StatusCode, Value) {
  let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let asked = Request::post(path)
    .header("host", "127.0.0.1")
    .header("content-type", "application/json")
    .header("accept", "application/json, text/event-stream")
    .header("mcp-protocol-version", "2025-06-18")
    .header("authorization", format!("Bearer {token}"))
    .body(Full::new(Bytes::from(body.to_string())))
    .unwrap();
  let response = link.send_request(asked).await.unwrap();
  let status = response.status();
  let bytes = response.into_body().collect().await.unwrap().to_bytes();

  (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

// `bmo-pocket quiet` against this host: its exit and what it said.
fn quiet(scratch: &Scratch, host: &Running) -> (Option<i32>, String) {
  let said = Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
    .arg("--data-root")
    .arg(scratch.data.path())
    .arg("quiet")
    .env("BMO_MCP_HOST_SOCKET", &host.control)
    .output()
    .unwrap();

  (said.status.code(), String::from_utf8_lossy(&said.stdout).into_owned())
}

// "Asked to stop ... the host takes no new call and ends none it holds" (docs/mcp-host.md, 「The host's stop」): calls
// by every road run to their end through the stop, a new connection is refused on each face, a call of an
// `ends_at_stop` entry is ended at once, and the process exits after the last, its helper asked to end first.
#[tokio::test]
async fn a_stop_takes_no_new_call_lets_each_one_in_hand_end_and_exits_after_the_last() {
  let scratch = Scratch::new();
  let root = scratch.data.path().to_path_buf();
  let keeper = root.join("keeper-source");
  let outer = root.join("outer-source");
  let any = r#"{ "type": "object" }"#;

  fs::create_dir_all(&keeper).unwrap();
  fs::write(keeper.join("tool.toml"), KEEPER_MANIFEST).unwrap();
  fs::write(keeper.join("keeper.py"), KEEPER_SCRIPT).unwrap();
  fs::create_dir_all(&outer).unwrap();
  fs::write(outer.join("outer.py"), OUTER_SERVER).unwrap();
  fs::write(
    outer.join("server.toml"),
    format!(
      "name = \"outer\"\nnamespace = \"outer\"\ncapabilities = [\"outer.use\"]\n\n[stdio]\ncommand = [\"python3\", \"{}\"]\n",
      outer.join("outer.py").display()
    ),
  )
  .unwrap();

  for put in [
    vec![keeper.to_str().unwrap()],
    vec!["--mcp", outer.join("server.toml").to_str().unwrap(), "--allow", "outer.use"],
  ] {
    let registered = Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .arg("--data-root")
      .arg(&root)
      .arg("put")
      .args(&put)
      .output()
      .unwrap();

    assert!(registered.status.success(), "{}", String::from_utf8_lossy(&registered.stderr));
  }

  place(
    &scratch.tools(),
    "holder",
    &[("tool.toml", HOLDER_MANIFEST), ("any.json", any), ("holder.py", HOLDER_SCRIPT)],
  );

  let mut host = scratch.start();
  let keeper_asked = root.join("keeper.asked");
  let token = issue(
    &host,
    &record(
      &["garden.read", "outer.use"],
      json!({
        "namespaces": ["garden", "outer"],
        "variables": { "KEEPER_ASKED": keeper_asked.to_string_lossy() },
      }),
    ),
  )
  .await;

  until("the external server's listing", || async {
    let path = "/namespaces/outer/tools/hold";

    control(&host, Method::GET, path, Some(&host.issuer), None).await.status == StatusCode::OK
  })
  .await;

  assert_eq!(quiet(&scratch, &host), (Some(0), "no tool call in hand\n".to_owned()));

  let started = |call: &str| root.join(format!("{call}.started"));
  let released = |call: &str| root.join(format!("{call}.released"));
  let holding = |call: &str| {
    json!({
      "started": started(call).to_string_lossy(),
      "released": released(call).to_string_lossy(),
      "say": format!("{call} ended"),
    })
  };
  let by_control = |namespace: &str, name: &str, input: Value| {
    let asked = json!({ "namespace": namespace, "name": name, "input": input });
    let (socket, token) = (host.control.clone(), token.clone());

    tokio::spawn(async move {
      request(&socket, Method::POST, "/tool-calls", Some(&token), Some(&asked)).await.unwrap()
    })
  };

  // One call in hand by each road: the control socket, the tools face, an asynchronous call and an external server;
  // and one that does not hold the stop.
  let controlled = by_control("garden", "hold", holding("controlled"));
  let external = by_control("outer", "hold", holding("external"));
  let past_stop = by_control("garden", "hold_past_stop", holding("past-stop"));
  let on_face = tokio::spawn(on_tools_face(
    host.port,
    token.clone(),
    "/mcp/garden",
    json!({
      "jsonrpc": "2.0", "id": 1, "method": "tools/call",
      "params": { "name": "hold", "arguments": holding("on-face") },
    }),
  ));
  let (status, kept) = on_tools_face(
    host.port,
    token.clone(),
    "/use",
    json!({ "tool": "garden/hold", "arguments": holding("kept") }),
  )
  .await;

  assert_eq!(status, StatusCode::OK, "{kept}");

  let kept_end = root.join("use").join(kept["id"].as_str().unwrap());

  until("every call's start", || {
    let all = ["controlled", "external", "past-stop", "on-face", "kept"].map(started);

    async move { all.iter().all(|file| fs::read_to_string(file).is_ok_and(|pid| !pid.is_empty())) }
  })
  .await;

  assert_eq!(issued_ask(&host, Method::GET, "/health").await["callsInHand"], 4);

  let (code, said) = quiet(&scratch, &host);

  assert_eq!(code, Some(1), "{said}");
  assert!(said.starts_with("4 tool calls in hand: "), "{said}");
  assert_eq!(said.matches("garden/hold (gardener, ").count(), 3, "{said}");
  assert!(said.contains("outer/hold (gardener, ") && !said.contains("hold_past_stop"), "{said}");

  // SAFETY: a signal to the host this test started, which has not been waited for, so its pid is its own.
  assert_eq!(unsafe { libc::kill(host.child.id() as i32, libc::SIGTERM) }, 0);

  // The call that does not hold the stop is ended at once and says why, and every face stops listening.
  let cut = past_stop.await.unwrap();
  let said = String::from_utf8_lossy(&cut.body).into_owned();

  assert!(said.contains("the host stopped"), "{said}");
  assert!(!cut.status.is_success(), "{said}");

  let cut_pid = fs::read_to_string(started("past-stop")).unwrap();

  until("the ended call's process to go", || {
    let cut_pid = cut_pid.clone();

    async move { !alive(&cut_pid) }
  })
  .await;
  until("the faces to stop listening", || async {
    let tools = TcpStream::connect(("127.0.0.1", host.port)).await;
    let socket = tokio::net::UnixStream::connect(&host.control).await;

    let refused = |error: &std::io::Error| error.kind() == std::io::ErrorKind::ConnectionRefused;

    tools.as_ref().err().is_some_and(refused) && socket.as_ref().err().is_some_and(refused)
  })
  .await;

  let (code, said) = quiet(&scratch, &host);

  assert_eq!(code, Some(0));
  assert!(said.contains("could not be read"), "{said}");

  // Each call in hand ends only as it is released, answered to its own caller, the host running through it all.
  assert!(!controlled.is_finished() && !external.is_finished() && !on_face.is_finished());
  assert!(host.child.try_wait().unwrap().is_none());

  fs::write(released("controlled"), "").unwrap();

  let answer: Value = controlled.await.unwrap().json().unwrap();

  assert_eq!(answer["result"]["content"][0]["text"], "controlled ended");
  assert!(host.child.try_wait().unwrap().is_none());

  fs::write(released("external"), "").unwrap();

  let answer: Value = external.await.unwrap().json().unwrap();

  assert_eq!(answer["result"]["content"][0]["text"], "external ended");
  assert!(host.child.try_wait().unwrap().is_none());

  fs::write(released("on-face"), "").unwrap();

  let (status, answer) = on_face.await.unwrap();

  assert_eq!(status, StatusCode::OK);
  assert_eq!(text(&answer), "on-face ended");
  assert!(host.child.try_wait().unwrap().is_none());
  assert!(!keeper_asked.exists(), "a helper was ended while a call could still use it");

  fs::write(released("kept"), "").unwrap();

  let mut exited = None;

  until("the host's exit after its last call", || {
    exited = host.child.try_wait().unwrap();

    let done = exited.is_some();

    async move { done }
  })
  .await;

  assert!(exited.unwrap().success());

  let end: Value = serde_json::from_slice(&fs::read(kept_end.join("end.json")).unwrap()).unwrap();

  assert_eq!(end["state"], "done");
  assert!(keeper_asked.exists(), "the helper was not asked to end");
  assert!(!host.control.exists());
}

// A tool whose `linger` never answers of itself, declaring itself unbounded, and whose `relay` calls `linger` through
// its call-back socket; each call says it started.
const LINGERER_MANIFEST: &str = r#"
[tool]
name = "lingerer"
protocol = 1
language = "python"
run = ["python3", "lingerer.py"]

[[entry]]
namespace = "garden"
name = "linger"
description = "Never answers of itself."
input = "any.json"
capabilities = ["garden.read"]
unbounded = true
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "relay"
description = "Calls linger through its call-back socket."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }
"#;

const LINGERER_SCRIPT: &str = r#"
import json, os, socket, sys, time

handed = json.load(sys.stdin)
given = handed["arguments"]
open(given["started"], "w").write(str(os.getpid()))

if handed["entry"] == "garden/relay":
    with socket.socket(socket.AF_UNIX) as line:
        line.connect(handed["callback"])
        asked = {"kind": "call", "entry": "garden/linger", "arguments": given["inner"]}
        line.sendall((json.dumps(asked) + "\n").encode())
        print(line.makefile().readline())
    sys.exit()

time.sleep(3600)
"#;

// One JSON request on the tools face whose connection stands as long as the task asking it: the task aborted, the
// connection closes with no answer read.
fn held_on_tools_face(
  port: u16,
  token: String,
  path: &'static str,
  body: Value,
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();
    let asked = Request::post(path)
      .header("host", "127.0.0.1")
      .header("content-type", "application/json")
      .header("accept", "application/json, text/event-stream")
      .header("mcp-protocol-version", "2025-06-18")
      .header("authorization", format!("Bearer {token}"))
      .body(Full::new(Bytes::from(body.to_string())))
      .unwrap();

    tokio::select! {
      _ = connection => {}
      _ = link.send_request(asked) => {}
    }
  })
}

// The calls `GET /health` names in hand, by entry, each said to be the gardener's.
async fn named_in_hand(host: &Running) -> Vec<String> {
  let health = issued_ask(host, Method::GET, "/health").await;
  let named = health["inHand"].as_array().unwrap();

  assert_eq!(health["callsInHand"], named.len());

  named
    .iter()
    .map(|call| {
      assert_eq!(call["holder"], "gardener", "{call}");
      assert!(call["startedAt"].as_u64().is_some_and(|at| at > 0), "{call}");
      call["entry"].as_str().unwrap().to_owned()
    })
    .collect()
}

// "A call whose caller is gone holds nothing" (docs/mcp-host.md, 「The host's stop」): a call on the tools face whose
// connection closes, the call it made through its call-back socket with it, and an asynchronous call and a control
// request whose record is revoked, each of an unbounded entry but the first, have their processes ended and drop out
// of what `quiet` waits on, which names each while it stands.
#[tokio::test]
async fn a_call_whose_caller_is_gone_is_ended_and_no_longer_held_in_hand() {
  let scratch = Scratch::new();
  let root = scratch.data.path().to_path_buf();
  let started = |call: &str| root.join(format!("{call}.started"));
  let pid_of = |call: &str| fs::read_to_string(started(call)).unwrap_or_default();

  place(
    &scratch.tools(),
    "lingerer",
    &[
      ("tool.toml", LINGERER_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("lingerer.py", LINGERER_SCRIPT),
    ],
  );

  let host = scratch.start();
  let token = issue(&host, &record(&["garden.read"], json!({ "namespaces": ["garden"] }))).await;

  // The tools face: the relay's connection closed, the relay and the call it made through its socket end.
  let on_face = held_on_tools_face(
    host.port,
    token.clone(),
    "/mcp/garden",
    json!({
      "jsonrpc": "2.0", "id": 1, "method": "tools/call",
      "params": {
        "name": "relay",
        "arguments": {
          "started": started("relay").to_string_lossy(),
          "inner": { "started": started("relayed").to_string_lossy() },
        },
      },
    }),
  );

  until("the relay and the call it made to start", || {
    let both = [pid_of("relay"), pid_of("relayed")];

    async move { both.iter().all(|pid| !pid.is_empty()) }
  })
  .await;

  assert_eq!(named_in_hand(&host).await, ["garden/relay", "garden/linger"]);

  let (code, said) = quiet(&scratch, &host);

  assert_eq!(code, Some(1), "{said}");
  assert!(said.starts_with("2 tool calls in hand: garden/relay (gardener, "), "{said}");
  assert!(said.contains(", garden/linger (gardener, ") && said.ends_with(")\n"), "{said}");

  on_face.abort();

  for call in ["relay", "relayed"] {
    let pid = pid_of(call);

    until(&format!("the {call} call's process to end"), move || {
      let pid = pid.clone();

      async move { !alive(&pid) }
    })
    .await;
  }

  until("the face's calls to leave the hand", || async { named_in_hand(&host).await.is_empty() }).await;

  // The record revoked: an asynchronous call and a control request under it end, the request answered as cancelled.
  let (status, kept) = on_tools_face(
    host.port,
    token.clone(),
    "/use",
    json!({ "tool": "garden/linger", "arguments": { "started": started("kept").to_string_lossy() } }),
  )
  .await;

  assert_eq!(status, StatusCode::OK, "{kept}");

  let controlled = {
    let asked =
      json!({ "namespace": "garden", "name": "linger", "input": { "started": started("controlled") } });
    let (socket, token) = (host.control.clone(), token.clone());

    tokio::spawn(async move {
      request(&socket, Method::POST, "/tool-calls", Some(&token), Some(&asked)).await.unwrap()
    })
  };

  until("the kept and the controlled calls to start", || {
    let both = [pid_of("kept"), pid_of("controlled")];

    async move { both.iter().all(|pid| !pid.is_empty()) }
  })
  .await;

  assert_eq!(named_in_hand(&host).await.len(), 2);

  let revoked =
    control(&host, Method::DELETE, &format!("/tool-sessions/{token}"), Some(&host.issuer), None).await;

  assert_eq!(revoked.status, StatusCode::NO_CONTENT);

  let cut = tokio::time::timeout(Duration::from_secs(10), controlled)
    .await
    .expect("the controlled call's answer did not come within ten seconds")
    .unwrap();
  let said = String::from_utf8_lossy(&cut.body).into_owned();

  assert_eq!(cut.status, StatusCode::BAD_GATEWAY, "{said}");
  assert!(said.contains("cancelled: its caller is gone"), "{said}");

  for call in ["kept", "controlled"] {
    let pid = pid_of(call);

    until(&format!("the {call} call's process to end"), move || {
      let pid = pid.clone();

      async move { !alive(&pid) }
    })
    .await;
  }

  let id = kept["id"].as_str().unwrap().to_owned();

  until("the kept call's end", || {
    let ended = end_of(&scratch, &id);

    async move { ended.is_some() }
  })
  .await;

  let end = end_of(&scratch, &id).unwrap();

  assert_eq!(end["state"], "cancelled", "{end}");
  assert!(end["summary"].as_str().unwrap().contains("its caller is gone"), "{end}");
  assert!(named_in_hand(&host).await.is_empty());
  assert_eq!(quiet(&scratch, &host), (Some(0), "no tool call in hand\n".to_owned()));
}

// A tool that waits until it is released, then says so, and holds nothing else.
const WAITER_MANIFEST: &str = r#"
[tool]
name = "waiter"
protocol = 1
language = "python"
run = ["python3", "holder.py"]

[[entry]]
namespace = "garden"
name = "wait"
description = "Waits until it is released."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }
"#;

// How the stand-in receiver meets a wake: cuts each connection as it opens, holds it open and never answers, reads
// the wake and keeps it but cuts the connection before answering, or queues it and answers its id.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum Meets {
  Cut,
  Silent,
  Swallowed,
  Queued,
}

// The receiver standing in on a loopback port, meeting each wake as it is set to; every wake it read is kept, in
// order, and a queued one is answered with its place in that order.
struct Receiver {
  port: u16,
  meets: Arc<AtomicU8>,
  connections: Arc<AtomicUsize>,
  queued: Arc<Mutex<Vec<Value>>>,
  serving: tokio::task::JoinHandle<()>,
}

impl Drop for Receiver {
  fn drop(&mut self) {
    self.serving.abort();
  }
}

impl Receiver {
  async fn closed() -> Self {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let meets = Arc::new(AtomicU8::new(Meets::Cut as u8));
    let connections = Arc::new(AtomicUsize::new(0));
    let queued = Arc::new(Mutex::new(Vec::new()));
    let (meeting, counted, kept) = (meets.clone(), connections.clone(), queued.clone());
    let serving = tokio::spawn(async move {
      // The connections a silent receiver holds, which go with this task.
      let mut held = Vec::new();

      while let Ok((stream, _)) = listener.accept().await {
        counted.fetch_add(1, Ordering::SeqCst);

        match meeting.load(Ordering::SeqCst) {
          now if now == Meets::Cut as u8 => continue,
          now if now == Meets::Silent as u8 => {
            held.push(stream);
            continue;
          }
          _ => {}
        }

        let (kept, meeting) = (kept.clone(), meeting.clone());
        let answer = service_fn(move |asked: Request<Incoming>| {
          let (kept, meeting) = (kept.clone(), meeting.clone());

          async move {
            let body = asked.into_body().collect().await.map_err(std::io::Error::other)?.to_bytes();
            let id = {
              let mut kept = kept.lock().unwrap();

              kept.push(serde_json::from_slice(&body).unwrap_or(Value::Null));
              kept.len()
            };

            if meeting.load(Ordering::SeqCst) == Meets::Swallowed as u8 {
              return Err(std::io::Error::other("the stand-in cut the connection before answering"));
            }

            let said = Full::new(Bytes::from(json!({ "id": id }).to_string()));

            Ok(http::Response::builder().status(StatusCode::CREATED).body(said).unwrap())
          }
        });

        tokio::spawn(http1::Builder::new().serve_connection(TokioIo::new(stream), answer));
      }
    });

    Self {
      port,
      meets,
      connections,
      queued,
      serving,
    }
  }

  fn meet(&self, meets: Meets) {
    self.meets.store(meets as u8, Ordering::SeqCst);
  }

  fn open(&self) {
    self.meet(Meets::Queued);
  }

  fn queued(&self) -> Vec<Value> {
    self.queued.lock().unwrap().clone()
  }

  // Where the host reads the receiver from: the env file under its data root.
  fn publish(&self, scratch: &Scratch) {
    let env = scratch.data.path().join("env");

    fs::create_dir_all(&env).unwrap();
    fs::write(
      env.join("receiver.env"),
      format!("RECEIVER_ADDRESS=http://127.0.0.1:{}\nRECEIVER_TOKEN=receiver-token\n", self.port),
    )
    .unwrap();
  }
}

// The waiter placed before the host reads its tools, a session issued that holds it, and the waits an owed wake
// takes and the span of one try kept short.
async fn waiting_host(scratch: &Scratch) -> (Running, String) {
  place(
    &scratch.tools(),
    "waiter",
    &[
      ("tool.toml", WAITER_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("holder.py", HOLDER_SCRIPT),
    ],
  );

  let host = scratch.start_with(&["--wake-retry-ms", "20", "--wake-try-ms", "200"]);
  let token = issue(&host, &record(&["garden.read"], json!({ "namespaces": ["garden"] }))).await;

  (host, token)
}

// An asynchronous call of the waiter, answered with the call's id; released at once where `release` says so.
async fn wait_call(scratch: &Scratch, host: &Running, token: &str, call: &str, release: bool) -> String {
  let root = scratch.data.path();
  let released = root.join(format!("{call}.released"));

  if release {
    fs::write(&released, "").unwrap();
  }

  let arguments = json!({
    "started": root.join(format!("{call}.started")).to_string_lossy(),
    "released": released.to_string_lossy(),
    "say": format!("{call} ended"),
  });
  let (status, kept) = on_tools_face(
    host.port,
    token.to_owned(),
    "/use",
    json!({ "tool": "garden/wait", "arguments": arguments }),
  )
  .await;

  assert_eq!(status, StatusCode::OK, "{kept}");
  kept["id"].as_str().unwrap().to_owned()
}

// The end kept for the call `id`, where one stands.
fn end_of(scratch: &Scratch, id: &str) -> Option<Value> {
  serde_json::from_slice(&fs::read(scratch.data.path().join("use").join(id).join("end.json")).ok()?).ok()
}

// The wake the receiver queued for the call `id`, where it queued one.
fn fact_for(receiver: &Receiver, id: &str) -> Option<Value> {
  receiver.queued().into_iter().find(|asked| asked["said"]["fact"]["use"] == id)
}

// "A wake that could not be queued is retried until queued" (docs/api.md, 「Asynchronous calls」): the receiver cuts
// every connection, the end says the wake is owed and why after more than one try, and once the receiver opens the
// same host queues it, once.
#[tokio::test]
async fn a_wake_the_receiver_cuts_is_owed_and_asked_again_until_it_is_queued() {
  let scratch = Scratch::new();
  let receiver = Receiver::closed().await;

  receiver.publish(&scratch);

  let (host, token) = waiting_host(&scratch).await;
  let id = wait_call(&scratch, &host, &token, "quick", true).await;

  until("the wake owed after two tries", || {
    let end = end_of(&scratch, &id);

    async move {
      end.is_some_and(|end| end["wake"]["pending"] == true && end["wake"]["tries"].as_u64() >= Some(2))
    }
  })
  .await;

  let owed = end_of(&scratch, &id).unwrap();

  assert_eq!(owed["state"], "done");
  assert!(owed["wake"]["error"].as_str().unwrap().contains("out of reach"), "{owed}");
  assert!(receiver.queued().is_empty());

  receiver.open();

  until("the wake queued", || {
    let end = end_of(&scratch, &id);

    async move { end.is_some_and(|end| end["wake"].get("id").is_some()) }
  })
  .await;

  let queued = receiver.queued();
  let fact = &queued[0]["said"]["fact"];

  assert_eq!(queued.len(), 1);
  assert_eq!(end_of(&scratch, &id).unwrap()["wake"], json!({ "id": 1 }));
  assert_eq!((fact["use"].as_str(), fact["state"].as_str()), (Some(id.as_str()), Some("done")));
  assert!(receiver.connections.load(Ordering::SeqCst) > 2);
  assert!(scratch.log().contains("failed its try 2"), "{}", scratch.log());
}

// "At its start the host settles every record it keeps" (docs/api.md, 「Asynchronous calls」): a host killed with one
// call's wake owed and another call running leaves both told by the host started after it, the second ended `lost`.
#[tokio::test]
async fn the_host_started_after_one_killed_mid_work_queues_its_owed_wake_and_tells_its_lost_call() {
  let scratch = Scratch::new();
  let receiver = Receiver::closed().await;

  receiver.publish(&scratch);

  let (mut first, token) = waiting_host(&scratch).await;
  let owed = wait_call(&scratch, &first, &token, "owed", true).await;
  let running = wait_call(&scratch, &first, &token, "running", false).await;
  let started = scratch.data.path().join("running.started");

  until("the first wake owed and the second call under way", || {
    let end = end_of(&scratch, &owed);
    let started = fs::read_to_string(&started).unwrap_or_default();

    async move { end.is_some_and(|end| end["wake"]["pending"] == true) && !started.is_empty() }
  })
  .await;

  first.child.kill().unwrap();
  first.child.wait().unwrap();

  // The waiter outlives its host; it is taken down here, as nothing else would.
  let pid: i32 = fs::read_to_string(&started).unwrap().trim().parse().unwrap();

  // SAFETY: the pid the waiter this test started wrote of itself, whose host is gone and never waited on it.
  unsafe { libc::kill(pid, libc::SIGKILL) };
  assert!(end_of(&scratch, &running).is_none());
  assert!(receiver.queued().is_empty());

  receiver.open();

  let _second = scratch.start_with(&["--wake-retry-ms", "20"]);

  until("both wakes queued", || {
    let ends = [end_of(&scratch, &owed), end_of(&scratch, &running)];

    async move { ends.iter().all(|end| end.as_ref().is_some_and(|end| end["wake"].get("id").is_some())) }
  })
  .await;

  let lost = end_of(&scratch, &running).unwrap();
  let told_lost = fact_for(&receiver, &running).unwrap();
  let told_owed = fact_for(&receiver, &owed).unwrap();

  assert_eq!(receiver.queued().len(), 2);
  assert_eq!(lost["state"], "lost");
  assert!(lost["summary"].as_str().unwrap().contains("was lost"), "{lost}");
  assert_eq!(told_lost["said"]["fact"]["state"], "lost");
  assert_eq!(told_lost["said"]["fact"]["summary"], lost["summary"]);
  assert_eq!(told_owed["said"]["fact"]["state"], "done");
  assert_eq!(told_owed["said"]["fact"]["summary"], "garden/wait answered: owed ended");
}

// "A connection cut and a try left unanswered ... all alike" and "the receiver may queue it twice; it is never left
// untold" (docs/api.md, 「Asynchronous calls」): a receiver that holds the connection and never answers leaves the
// wake owed once the try's span has passed; one that reads the wake and cuts the connection before answering is
// asked again, so it reads the same wake more than once; and once it answers, the end names the last of them.
#[tokio::test]
async fn a_wake_never_answered_or_cut_once_read_is_asked_again_until_it_is_answered() {
  let scratch = Scratch::new();
  let receiver = Receiver::closed().await;

  receiver.meet(Meets::Silent);
  receiver.publish(&scratch);

  let (host, token) = waiting_host(&scratch).await;
  let id = wait_call(&scratch, &host, &token, "quick", true).await;
  let failed_by = |words: &'static str| {
    let end = end_of(&scratch, &id);

    async move { end.is_some_and(|end| end["wake"]["error"].as_str().is_some_and(|why| why.contains(words))) }
  };

  until("a try left unanswered", || failed_by("did not answer within 200 ms")).await;
  assert!(receiver.queued().is_empty());

  receiver.meet(Meets::Swallowed);

  until("a try read and cut", || failed_by("out of reach")).await;
  assert!(!receiver.queued().is_empty());

  receiver.open();

  until("the wake queued", || {
    let end = end_of(&scratch, &id);

    async move { end.is_some_and(|end| end["wake"].get("id").is_some()) }
  })
  .await;

  let read = receiver.queued();

  assert!(read.len() >= 2, "{read:?}");
  assert!(read.iter().all(|asked| asked["said"]["fact"]["use"] == id.as_str()), "{read:?}");
  assert_eq!(end_of(&scratch, &id).unwrap()["wake"], json!({ "id": read.len() }));
}

// "A stop leaves an owed wake on disk for the host started next" (docs/api.md, 「Asynchronous calls」): a host asked by
// `SIGTERM` to stop while a wake is owed exits without waiting on it, and the host started after it queues the wake.
#[tokio::test]
async fn a_host_stopped_with_a_wake_owed_exits_and_the_host_started_next_queues_it() {
  let scratch = Scratch::new();
  let receiver = Receiver::closed().await;

  receiver.publish(&scratch);

  let (mut first, token) = waiting_host(&scratch).await;
  let id = wait_call(&scratch, &first, &token, "owed", true).await;

  until("the wake owed", || {
    let end = end_of(&scratch, &id);

    async move {
      end.is_some_and(|end| end["wake"]["pending"] == true && end["wake"]["tries"].as_u64() >= Some(1))
    }
  })
  .await;

  // SAFETY: a signal to the host this test started, which has not been waited for, so its pid is its own.
  assert_eq!(unsafe { libc::kill(first.child.id() as i32, libc::SIGTERM) }, 0);

  let mut exited = None;

  until("the stopped host's exit", || {
    exited = first.child.try_wait().unwrap();

    let done = exited.is_some();

    async move { done }
  })
  .await;

  assert!(exited.unwrap().success());
  assert_eq!(end_of(&scratch, &id).unwrap()["wake"]["pending"], true);
  assert!(receiver.queued().is_empty());

  receiver.open();

  let _second = scratch.start_with(&["--wake-retry-ms", "20"]);

  until("the wake queued", || {
    let end = end_of(&scratch, &id);

    async move { end.is_some_and(|end| end["wake"].get("id").is_some()) }
  })
  .await;

  let queued = receiver.queued();

  assert_eq!(queued.len(), 1);
  assert_eq!(queued[0]["said"]["fact"]["use"], id.as_str());
  assert_eq!(queued[0]["said"]["fact"]["state"], "done");
  assert_eq!(end_of(&scratch, &id).unwrap()["wake"], json!({ "id": 1 }));
}

// The almanac app's tool: two garden entries of one app, answered by one script.
const ALMANAC_MANIFEST: &str = r#"
[tool]
name = "almanac"
protocol = 1
run = ["python3", "almanac.py"]

[app]
key = "almanac"
name = "Almanac"
description = "Says when to sow."

[[entry]]
namespace = "garden"
name = "sow_when"
description = "Says when a plant is sown."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "sow_where"
description = "Says where a plant is sown."
input = "any.json"
capabilities = ["garden.read"]
hints = { read_only = true }
"#;

const ALMANAC_SCRIPT: &str = r#"
import json, sys

handed = json.load(sys.stdin)
print(json.dumps({"content": [{"type": "text", "text": "almanac:" + handed["entry"]}], "isError": False}))
"#;

// "Apps" (docs/mcp-host.md, 「Tests」): a record whose withheld-apps file names an app is not shown its booked
// entries and is refused its description and its calls on every road, saying it is turned off by the
// organisation's settings; a tool naming no app is served as before; a resident entry of the app stays listed and
// is refused; the file rewritten between two calls decides the second; and a file that does not read withholds
// every app.
#[tokio::test]
async fn an_app_a_record_withholds_is_out_of_its_listings_and_refused_on_every_road() {
  let scratch = Scratch::new();

  place_readers(&scratch);
  place(
    &scratch.tools(),
    "almanac",
    &[
      ("tool.toml", ALMANAC_MANIFEST),
      ("any.json", r#"{ "type": "object" }"#),
      ("almanac.py", ALMANAC_SCRIPT),
    ],
  );

  let host = scratch.start();
  let file = scratch.data.path().join("withheld.json");
  let withhold = |apps: Value| fs::write(&file, json!({ "apps": apps }).to_string()).unwrap();

  withhold(json!(["almanac"]));

  let token = issue(
    &host,
    &record(
      &["garden.read"],
      json!({
        "holderName": "planter",
        "shown": ["garden/sow_when", "garden/plant_get"],
        "variables": { "PLOT": "north" },
        "withheldAppsFile": file,
      }),
    ),
  )
  .await;
  let off = "is turned off for this session by the organisation's settings";

  // Off: the booked entry is out of the toolbook, its description and every call refused; the resident one stays
  // in its namespace's list and is refused when called; the system's tools are untouched.
  let listed_book = book(&host, &token).await;

  assert!(
    !listed_book.contains("sow_where") && listed_book.contains("garden/plant_search"),
    "{listed_book}"
  );
  assert!(
    text(&called(&host, &token, "toolbook", "describe", json!({ "tool": "garden/sow_where" })).await)
      .contains(off)
  );
  assert!(text(&through(&host, &token, "call", "garden/sow_where", json!({})).await).contains(off));
  assert_eq!(listed(&host, &token, "garden").await, ["plant_get", "sow_when"]);
  assert!(text(&called(&host, &token, "garden", "sow_when", json!({})).await).contains(off));
  assert!(text(&called(&host, &token, "garden", "plant_get", json!({})).await).starts_with("plot/"));

  let tools = control(&host, Method::GET, "/namespaces/garden/tools", Some(&token), None).await;
  let tools: Value = tools.json().unwrap();
  let names: Vec<&str> = tools["items"]
    .as_array()
    .unwrap()
    .iter()
    .map(|tool| tool["name"].as_str().unwrap())
    .collect();

  assert!(
    !names.contains(&"sow_when") && !names.contains(&"sow_where") && names.contains(&"plant_get"),
    "{names:?}"
  );

  let refused = control(
    &host,
    Method::POST,
    "/tool-calls",
    Some(&token),
    Some(&json!({ "namespace": "garden", "name": "sow_where", "input": {} })),
  )
  .await;

  assert_eq!(refused.status, StatusCode::FORBIDDEN);
  assert!(String::from_utf8_lossy(&refused.body).contains(off));

  // On again: the file is read at the next request, with no new session.
  withhold(json!([]));

  assert!(book(&host, &token).await.contains("garden/sow_where"));
  assert_eq!(
    text(&through(&host, &token, "call", "garden/sow_where", json!({})).await),
    "almanac:garden/sow_where"
  );
  assert_eq!(
    text(&called(&host, &token, "garden", "sow_when", json!({})).await),
    "almanac:garden/sow_when"
  );

  // A file that does not read withholds every app, and the system's tools still answer.
  fs::write(&file, "not json").unwrap();

  assert!(!book(&host, &token).await.contains("sow_where"));
  assert!(text(&called(&host, &token, "garden", "sow_when", json!({})).await).contains(off));
  assert!(text(&called(&host, &token, "garden", "plant_get", json!({})).await).starts_with("plot/"));
}
