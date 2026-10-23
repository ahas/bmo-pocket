//! The tool host as a session meets it: the binary started on a scratch data root, tool sessions issued on its
//! control socket with its issuer token, and the protocol spoken to it over HTTP (docs/mcp-host.md, 「Tests」).

use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bmo_pocket_client::transport::{Reply, request};
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
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

  let book = text(&called(&host, &seeker, "toolbook", "list", json!({})).await);

  for line in [
    "notes/history · call — The history, newest first.",
    "notes/status · call — The live work, as it stands.",
    "notes/note_add · act — Files a row.",
    "garden/plant_list · call — Asked with a bed, that bed's plants.",
  ] {
    assert!(book.lines().any(|listed| listed == line), "{line}\n{book}");
  }

  assert_eq!(
    text(&called(&host, &seeker, "toolbook", "list", json!({ "query": "NEWEST" })).await),
    "notes/history · call — The history, newest first."
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
  let manual =
    text(&called(&host, &seeker, "toolbook", "describe", json!({ "tool": "garden/plant_list" })).await);

  assert!(
    manual.starts_with(
      "garden/plant_list — reached with call\nAsked with a bed, that bed's plants. Asked with none, the beds.\n\
       Manual:\n# plant_list\n\nPass `bed`, a bed's name, for that bed's plants.\nRead-only: yes\n"
    ),
    "{manual}"
  );

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
description = "Never answers in time."
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

  // A call past its timeout, and one that ends badly, fail saying so.
  let started = std::time::Instant::now();

  assert!(
    text(&through(&host, &token, "call", "garden/echo_slow", json!({ "text": "s" })).await)
      .contains("did not answer within 1 seconds")
  );
  assert!(started.elapsed() < Duration::from_secs(10));
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
