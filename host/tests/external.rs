//! External servers as a person and a session meet them (docs/mcp-host.md, 「External servers」): a stand-in stdio MCP
//! server registered by configuration, its tools listed, described and called through the toolbook for a record
//! holding the registration's capability, started again after it dies, and gone once taken; a streamable HTTP
//! registration sent its token file's text; each refusal of a registration; and each registration read on
//! `GET /servers`, listed or not.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use bmo_pocket_client::{NameQuery, Pocket, ReachedBy, Transport};
use bmo_tool::Session;
use http::Request;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::handler::Host;
use mcp_host::install::{self, Put};
use mcp_host::placed::Placed;
use mcp_host::records::Records;
use mcp_host::registry::Registry;
use mcp_host::schedule::Schedules;
use mcp_host::server;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream, UnixListener};

// The stand-in MCP server, speaking the protocol on standard input and output a line a message: it marks each start
// in the file `STARTS` names, lists three tools, says the text back after `MARK`, and ends itself for `fall_over`.
const STAND_IN: &str = r#"
import json, os, sys

with open(os.environ["STARTS"], "a") as starts:
    starts.write("start\n")

TOOLS = [
    {"name": "echoText", "description": "Says the text back. It reads nothing else.",
     "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]},
     "annotations": {"readOnlyHint": True}},
    {"name": "write-note", "description": "Writes a note.",
     "inputSchema": {"type": "object", "properties": {"note": {"type": "string"}}}},
    {"name": "fall_over", "description": "Ends the server.", "inputSchema": {"type": "object"},
     "annotations": {"readOnlyHint": True}},
]

def say(message):
    print(json.dumps(dict(message, jsonrpc="2.0")), flush=True)

for line in sys.stdin:
    asked = json.loads(line)
    if "id" not in asked:
        continue
    method, params = asked.get("method"), asked.get("params") or {}
    if method == "initialize":
        say({"id": asked["id"], "result": {"protocolVersion": params["protocolVersion"],
             "capabilities": {"tools": {}}, "serverInfo": {"name": "stand-in", "version": "1"}}})
    elif method == "tools/list":
        say({"id": asked["id"], "result": {"tools": TOOLS}})
    elif method == "tools/call" and params["name"] == "fall_over":
        sys.exit(0)
    elif method == "tools/call" and params["name"] in ("echoText", "write-note"):
        given = params.get("arguments") or {}
        text = given.get("text") or "noted " + given.get("note", "")
        say({"id": asked["id"], "result": {"content": [{"type": "text", "text": os.environ["MARK"] + text}],
             "isError": False}})
    else:
        say({"id": asked["id"], "error": {"code": -32602, "message": "no such tool"}})
"#;

const MANUAL: &str = "# Echo\n\nHand it `text`; it answers the same text.\n";

struct Scratch {
  dir: TempDir,
  sockets: TempDir,
}

impl Scratch {
  fn new() -> Self {
    Self {
      dir: tempfile::tempdir().unwrap(),
      sockets: tempfile::Builder::new().prefix("ex").tempdir_in("/tmp").unwrap(),
    }
  }

  fn registry(&self) -> Registry {
    Registry::of(&self.dir.path().join("data"))
  }

  fn starts(&self) -> PathBuf {
    self.dir.path().join("starts.txt")
  }

  fn started(&self) -> usize {
    fs::read_to_string(self.starts()).unwrap_or_default().lines().count()
  }

  // A registration file written in a folder of its own beside the stand-in and its manual, `more` added to it.
  fn registration(&self, name: &str, more: &str) -> PathBuf {
    let folder = self.dir.path().join("sources").join(name);
    let script = folder.join("stand_in.py");

    fs::create_dir_all(folder.join("manuals")).unwrap();
    fs::write(&script, STAND_IN).unwrap();
    fs::write(folder.join("manuals/echo.md"), MANUAL).unwrap();
    fs::write(
      folder.join("server.toml"),
      format!(
        "name = \"{name}\"\nnamespace = \"outer\"\ncapabilities = [\"outer.use\"]\ntimeout = 20\n{more}\n\n[stdio]\ncommand = [\"python3\", \"{}\"]\nenv = {{ STARTS = \"{}\", MARK = \"m:\" }}\n\n[tools.echoText]\nmanual = \"manuals/echo.md\"\n",
        script.display(),
        self.starts().display()
      ),
    )
    .unwrap();
    folder.join("server.toml")
  }

  // A host serving the registry alone, its catalogue read once.
  fn host(&self) -> Arc<Host> {
    let placed = Placed::new(None, self.sockets.path().to_owned()).registered(self.registry());

    placed.reread();

    Arc::new(Host {
      placed: Arc::new(placed),
      pass_env: Vec::new(),
    })
  }
}

fn session(namespaces: &[&str], capabilities: &[&str], shown: &[&str]) -> Session {
  serde_json::from_value(json!({
    "holderName": "tester",
    "namespaces": namespaces,
    "capabilities": capabilities,
    "grants": [],
    "shown": shown,
  }))
  .unwrap()
}

fn register(registry: &Registry, file: &Path, allow: &[&str]) -> Result<Vec<String>, String> {
  let mut said = Vec::new();
  let asked = Put {
    tool: file.display().to_string(),
    allow: allow.iter().map(|one| (*one).to_owned()).collect(),
    ..Put::default()
  };

  install::register_server(registry, &asked, &mut said).map(|()| said)
}

fn text(answer: &Value) -> String {
  answer["content"][0]["text"].as_str().unwrap_or_default().to_owned()
}

// The tools face served in this process for the records, on a port of its own.
async fn serve(records: Arc<Records>, host: Arc<Host>) -> u16 {
  let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let port = listener.local_addr().unwrap().port();
  let app = server::tools(records, host, std::env::temp_dir().join("mcp-host-calls"));

  tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
  port
}

// One toolbook tool called on the tools face, answered as its result.
async fn toolbook(port: u16, token: &str, tool: &str, arguments: Value) -> Value {
  let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
    "params": { "name": tool, "arguments": arguments } });
  let asked = Request::post("/mcp/toolbook")
    .header("host", "127.0.0.1")
    .header("content-type", "application/json")
    .header("accept", "application/json, text/event-stream")
    .header("mcp-protocol-version", "2025-06-18")
    .header("authorization", format!("Bearer {token}"))
    .body(Full::new(Bytes::from(body.to_string())))
    .unwrap();
  let answer = link.send_request(asked).await.unwrap().into_body().collect().await.unwrap().to_bytes();

  serde_json::from_slice::<Value>(&answer).unwrap()["result"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_registered_stdio_server_is_listed_described_and_called_through_the_toolbook_and_gone_once_taken()
{
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let said = register(&registry, &scratch.registration("stand-in", ""), &["outer.use"]).unwrap();

  assert_eq!(said[0], "stand-in asks for the capability outer.use");
  assert!(said[1].starts_with("registered the external server stand-in "), "{said:?}");

  let pointer = registry.get("stand-in").unwrap();

  assert!(pointer.dir.join("server.toml").is_file() && pointer.dir.join("manuals/echo.md").is_file());
  assert!(pointer.origin.starts_with("path:") && pointer.origin.ends_with("server.toml"));

  // Its tools join the catalogue once the server has listed them.
  let host = scratch.host();
  let holding = session(&["outer"], &["outer.use"], &[]);

  assert!(host.find(&holding, "outer", "echo_text").is_none());
  host.placed.refresh().await;

  let echo = host.find(&holding, "outer", "echo_text").expect("its tool is an entry");

  assert_eq!(echo.reached_by(), ReachedBy::Call);
  assert_eq!(host.find(&holding, "outer", "write_note").unwrap().reached_by(), ReachedBy::Act);
  assert!(host.find(&session(&["outer"], &["other.use"], &[]), "outer", "echo_text").is_none());
  assert_eq!(scratch.started(), 1, "one connection lists and serves");

  // Listed, described and called through the toolbook as any booked entry.
  let records = Arc::new(Records::load(&scratch.dir.path().join("sessions")).unwrap());
  let token = records.issue(holding.clone()).unwrap();
  let port = serve(records, host.clone()).await;
  let listed = toolbook(port, &token, "list", json!({})).await;
  let lines = text(&listed);

  assert!(lines.contains("outer/echo_text · call — Says the text back."), "{lines}");
  assert!(lines.contains("outer/write_note · act — Writes a note."), "{lines}");

  let described = toolbook(port, &token, "describe", json!({ "tool": "outer/echo_text" })).await;

  assert_eq!(described["structuredContent"]["manual"], MANUAL);
  assert!(text(&described).contains("Hand it `text`"));

  let called = toolbook(
    port,
    &token,
    "call",
    json!({ "tool": "outer/echo_text", "arguments": { "text": "hi" } }),
  )
  .await;

  assert_eq!(text(&called), "m:hi");

  let refused =
    toolbook(port, &token, "call", json!({ "tool": "outer/write_note", "arguments": {} })).await;

  assert_eq!(refused["isError"], true);
  assert!(text(&refused).contains("act"), "{refused}");

  let acted =
    toolbook(port, &token, "act", json!({ "tool": "outer/write_note", "arguments": { "note": "x" } }))
      .await;

  assert_eq!(text(&acted), "m:noted x");

  let misfit = toolbook(port, &token, "call", json!({ "tool": "outer/echo_text", "arguments": {} })).await;

  assert_eq!(misfit["isError"], true);
  assert_eq!(scratch.started(), 1, "every call went over the one connection");

  // A server that dies fails its call, and the next call starts it again.
  let fell = host.find(&holding, "outer", "fall_over").unwrap();
  let failed = host.attempt(&fell, &holding, json!({})).await;

  assert!(failed.is_err(), "{failed:?}");
  assert_eq!(
    host
      .answer(&echo, &holding, json!({ "text": "again" }))
      .await
      .map(|answer| text(&answer))
      .unwrap(),
    "m:again"
  );
  assert_eq!(scratch.started(), 2);

  // Taken, its tools are gone from the next reading.
  let mut said = Vec::new();

  install::take(&registry, "stand-in", None, &mut said).unwrap();
  host.placed.reread();
  assert!(host.find(&holding, "outer", "echo_text").is_none());
  assert!(!text(&toolbook(port, &token, "list", json!({})).await).contains("outer/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_streamable_http_registration_is_sent_its_token_files_text_as_its_bearer_token() {
  // The far side: a host serving the stand-in's tools under `outer`, shown to a session whose token the near
  // side's token file holds.
  let far = Scratch::new();

  register(&far.registry(), &far.registration("stand-in", ""), &["outer.use"]).unwrap();

  let far_host = far.host();

  far_host.placed.refresh().await;

  let records = Arc::new(Records::load(&far.dir.path().join("sessions")).unwrap());
  let shown = ["outer/echo_text", "outer/write_note", "outer/fall_over"];
  let far_token = records.issue(session(&["outer"], &["outer.use"], &shown)).unwrap();
  let port = serve(records, far_host).await;

  // The near side registers the far side's namespace as an external server under `relay`.
  let near = Scratch::new();
  let folder = near.dir.path().join("sources/relay");
  let token_file = near.dir.path().join("far-token");

  fs::create_dir_all(&folder).unwrap();
  fs::write(&token_file, "not-the-token\n").unwrap();
  fs::write(
    folder.join("server.toml"),
    format!(
      "name = \"relay\"\nnamespace = \"relay\"\ncapabilities = [\"relay.use\"]\ntimeout = 20\n\n[http]\nurl = \"http://127.0.0.1:{port}/mcp/outer\"\ntoken_file = \"{}\"\n",
      token_file.display()
    ),
  )
  .unwrap();
  register(&near.registry(), &folder.join("server.toml"), &["relay.use"]).unwrap();

  let host = near.host();
  let holding = session(&["relay"], &["relay.use"], &[]);

  // A token the far side does not know lists nothing, and says why.
  host.placed.refresh().await;
  assert!(host.find(&holding, "relay", "echo_text").is_none());
  assert!(host.placed.externals.wanting());

  // The token file is read afresh at the next connection.
  fs::write(&token_file, format!("{far_token}\n")).unwrap();
  host.placed.refresh().await;

  let echo = host.find(&holding, "relay", "echo_text").expect("the far side's tool is an entry");

  assert_eq!(echo.reached_by(), ReachedBy::Call);
  assert_eq!(host.find(&holding, "relay", "write_note").unwrap().reached_by(), ReachedBy::Act);
  assert_eq!(text(&host.answer(&echo, &holding, json!({ "text": "via" })).await.unwrap()), "m:via");
}

#[test]
fn a_registration_is_refused_naming_what_failed_and_nothing_is_placed() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let file = scratch.registration("stand-in", "");

  assert_eq!(
    register(&registry, &file, &[]).unwrap_err(),
    "stand-in asks for what no --allow names: outer.use"
  );

  let both = scratch.registration("both", "");

  fs::write(
    &both,
    format!("{}\n[http]\nurl = \"http://127.0.0.1:1/mcp\"\n", fs::read_to_string(&both).unwrap()),
  )
  .unwrap();
  assert!(
    register(&registry, &both, &["outer.use"])
      .unwrap_err()
      .contains("both a [stdio] and an [http]")
  );

  let unmanned = scratch.registration("unmanned", "");

  fs::remove_file(unmanned.parent().unwrap().join("manuals/echo.md")).unwrap();
  assert!(register(&registry, &unmanned, &["outer.use"]).unwrap_err().contains("its manual"));

  assert!(
    register(&registry, &scratch.dir.path().join("none.toml"), &["outer.use"])
      .unwrap_err()
      .contains("could not be read")
  );
  assert!(registry.entries().0.is_empty());
}

#[test]
fn the_command_registers_a_server_by_mcp_and_lists_it() {
  let scratch = Scratch::new();
  let file = scratch.registration("stand-in", "reached_by = \"call\"");
  let pocket = |args: &[&str]| {
    Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .arg("--data-root")
      .arg(scratch.dir.path().join("data"))
      .args(args)
      .output()
      .unwrap()
  };
  let file = file.to_str().unwrap();
  let put = pocket(&["put", "--mcp", file, "--allow", "outer.use"]);

  assert_eq!(put.status.code(), Some(0), "{}", String::from_utf8_lossy(&put.stderr));

  let listed = String::from_utf8_lossy(&pocket(&["list"]).stdout).into_owned();

  assert!(
    listed.starts_with("stand-in ") && listed.contains(": the external server stdio `python3 "),
    "{listed}"
  );
  assert!(listed.trim_end().ends_with("under outer/"), "{listed}");
  assert_eq!(pocket(&["put", "--mcp", file, "somewhere"]).status.code(), Some(2));
  assert_eq!(pocket(&["take", "stand-in"]).status.code(), Some(0));
}

// Each registration the host read is answered on `GET /servers` (docs/api.md, 「External servers」): before its
// server first answered with no listing, and after it with how many tools it listed; where it is reached, never its
// environment.
#[tokio::test(flavor = "multi_thread")]
async fn each_registration_read_is_answered_on_the_servers_route_listed_or_not() {
  let scratch = Scratch::new();
  let registry = scratch.registry();

  register(&registry, &scratch.registration("stand-in", "reached_by = \"call\""), &["outer.use"]).unwrap();

  let host = scratch.host();
  let socket = scratch.sockets.path().join("control.sock");
  let listener = UnixListener::bind(&socket).unwrap();
  let records = Arc::new(Records::load(&scratch.dir.path().join("sessions")).unwrap());
  let control = server::control(records, host.clone(), Arc::new(Schedules::default()), "issuer".to_owned());

  tokio::spawn(async move { axum::serve(listener, control).await });

  let issuer = Pocket::at(&socket, "issuer");
  let before = issuer.servers(&NameQuery::default()).await.unwrap();

  assert_eq!(before.items.len(), 1);

  let server = &before.items[0];

  assert_eq!((server.name.as_str(), server.namespace.as_str()), ("stand-in", "outer"));
  assert_eq!(
    (server.transport, server.reached_by, server.timeout),
    (Transport::Stdio, ReachedBy::Call, 20)
  );
  assert!(
    server.address.starts_with("python3 ") && server.address.ends_with("stand_in.py"),
    "{}",
    server.address
  );
  assert_eq!(server.capabilities, ["outer.use"]);
  assert_eq!((server.listed, server.error.as_deref()), (None, None));

  host.placed.refresh().await;

  let after = issuer.servers(&NameQuery::default()).await.unwrap();

  assert_eq!(after.items[0].listed, Some(3));

  let said = serde_json::to_string(&after).unwrap();

  assert!(!said.contains("STARTS") && !said.contains("m:"), "{said}");

  let mut taken = Vec::new();

  install::take(&registry, "stand-in", None, &mut taken).unwrap();
  host.placed.reread();
  assert!(issuer.servers(&NameQuery::default()).await.unwrap().items.is_empty());
}
