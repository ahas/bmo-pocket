//! The registry as a person and the host meet it (docs/registry.md): tools put from a folder, a git address and a
//! bare name, each refusal, a take and an upgrade, the host picking each up with no restart, a call in flight going
//! on through a take and an upgrade, a running session's prompt front unmoved while its toolbook changes, and each
//! put, upgrade and take told once on the event stream. Each change reaches the host through the catalogue's
//! reread hook, and a call is held where it stands by named pipes, so no test waits on the clock.

use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use bmo_pocket_client::{Booked, EventKind, ReachedBy, ToolChange};
use bmo_tool::Session;
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::async_calls::{AsyncCalls, WakeTries};
use mcp_host::handler::Host;
use mcp_host::install::{self, Put};
use mcp_host::placed::{PLACED, Placed};
use mcp_host::records::Records;
use mcp_host::registry::Registry;
use mcp_host::server;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::{TcpListener, TcpStream};

// The stand-in tool's executable: it says back the text it is handed after the mark its placed folder holds, read
// as it answers, so a call answered from a folder taken away under it would fail; handed two named pipes, it opens
// the first to say it runs and waits on the second before it answers.
const ECHO_SCRIPT: &str = r#"
import json, sys

handed = json.load(sys.stdin)
arguments = handed["arguments"]

if "started" in arguments:
    open(arguments["started"], "w").close()
    open(arguments["release"]).read()

mark = open("mark.txt").read().strip()
print(json.dumps({"content": [{"type": "text", "text": mark + ":" + arguments.get("text", "")}], "isError": False}))
"#;

const ECHO_SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" }, "started": { "type": "string" }, "release": { "type": "string" } } }"#;

// A tool's manifest: one read-only entry of the `fixture` namespace asking for `fixture.read`, with what `more`
// adds to the entry.
fn manifest(name: &str, entry: &str, tool_more: &str, entry_more: &str) -> String {
  format!(
    "[tool]\nname = \"{name}\"\nprotocol = 1\nrun = [\"python3\", \"echo.py\"]\ntimeout = 20\n{tool_more}\n[[entry]]\nnamespace = \"fixture\"\nname = \"{entry}\"\ndescription = \"Says the text back.\"\ninput = \"echo.json\"\ncapabilities = [\"fixture.read\"]\nhints = {{ read_only = true }}\n{entry_more}"
  )
}

struct Scratch {
  dir: TempDir,
  // Sockets are bound under a short path of their own.
  sockets: TempDir,
}

impl Scratch {
  fn new() -> Self {
    Self {
      dir: tempfile::tempdir().unwrap(),
      sockets: tempfile::Builder::new().prefix("rg").tempdir_in("/tmp").unwrap(),
    }
  }

  fn data(&self) -> PathBuf {
    self.dir.path().join("data")
  }

  fn registry(&self) -> Registry {
    Registry::of(&self.data())
  }

  // A tool's source folder, its manifest, schema, script and mark written in it.
  fn source(&self, folder: &str, manifest: &str, mark: &str) -> PathBuf {
    let dir = self.dir.path().join("sources").join(folder);

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("tool.toml"), manifest).unwrap();
    fs::write(dir.join("echo.json"), ECHO_SCHEMA).unwrap();
    fs::write(dir.join("echo.py"), ECHO_SCRIPT).unwrap();
    fs::write(dir.join("mark.txt"), mark).unwrap();
    dir
  }

  // An echo tool's source folder under its own name, answering with `mark`.
  fn echo(&self, name: &str, entry: &str, mark: &str) -> PathBuf {
    self.source(name, &manifest(name, entry, "", ""), mark)
  }

  // An echo tool placed in a tools root's folder, its built folder under `.placed`, answering with `mark`.
  fn placed_in(&self, tools: &Path, folder: &str, manifest: &str, mark: &str) {
    let placed = tools.join(folder).join(PLACED);

    fs::create_dir_all(&placed).unwrap();
    fs::write(placed.join("tool.toml"), manifest).unwrap();
    fs::write(placed.join("echo.json"), ECHO_SCHEMA).unwrap();
    fs::write(placed.join("echo.py"), ECHO_SCRIPT).unwrap();
    fs::write(placed.join("mark.txt"), mark).unwrap();
  }

  // A host serving the registry alone, its catalogue read once.
  fn host(&self) -> Arc<Host> {
    let placed = Placed::new(None, self.sockets.path().to_owned()).registered(self.registry());

    placed.reread();

    Arc::new(Host {
      placed: Arc::new(placed),
      pass_env: Vec::new(),
      drain: mcp_host::drain::Drain::default(),
    })
  }

  fn session(&self, shown: &[&str]) -> Session {
    serde_json::from_value(json!({
      "holderName": "tester",
      "namespaces": ["fixture"],
      "capabilities": ["fixture.read"],
      "grants": [],
      "shown": shown,
    }))
    .unwrap()
  }
}

// A put asked of the folder, allowing what the stand-in tools ask for.
fn put_of(tool: impl Into<String>) -> Put {
  Put {
    tool: tool.into(),
    allow: vec!["fixture.read".to_owned()],
    ..Put::default()
  }
}

// A put answered, with what it said, or its refusal.
fn put(registry: &Registry, asked: &Put) -> Result<Vec<String>, String> {
  let mut said = Vec::new();

  install::put(registry, asked, &mut said).map(|()| said)
}

fn take(registry: &Registry, name: &str, service: Option<&str>) -> Result<Vec<String>, String> {
  let mut said = Vec::new();

  install::take(registry, name, service, &mut said).map(|()| said)
}

fn text(answer: &Value) -> String {
  answer["content"][0]["text"].as_str().unwrap_or_default().to_owned()
}

// What one call of the entry under the session answers.
async fn called(host: &Arc<Host>, session: &Session, entry: &str, arguments: Value) -> String {
  let found = host.find(session, "fixture", entry).expect("the session holds the entry");

  text(&host.answer(&found, session, arguments).await.unwrap())
}

// A git repository of the folder, committed whole.
fn committed(folder: &Path) {
  let git = |args: &[&str]| {
    let ran = Command::new("git")
      .args(["-c", "user.name=test", "-c", "user.email=test", "-c", "init.defaultBranch=main"])
      .args(args)
      .current_dir(folder)
      .output()
      .unwrap();

    assert!(ran.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ran.stderr));
  };

  git(&["init", "--quiet"]);
  git(&["add", "."]);
  git(&["commit", "--quiet", "-m", "The tool"]);
}

fn mkfifo(path: &Path) {
  let named = CString::new(path.as_os_str().as_bytes()).unwrap();

  // SAFETY: a named pipe made at a path this test owns.
  assert_eq!(unsafe { libc::mkfifo(named.as_ptr(), 0o600) }, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_put_from_a_folder_is_placed_as_a_version_pointed_at_and_served_with_no_restart() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let host = scratch.host();
  let session = scratch.session(&[]);

  assert!(host.find(&session, "fixture", "echo_back").is_none());

  let said = put(&registry, &put_of(scratch.echo("echo", "echo_back", "v1").to_string_lossy())).unwrap();
  let pointer = registry.get("echo").unwrap();

  assert_eq!(said[0], "echo asks for the capability fixture.read");
  assert!(said[1].starts_with("put echo ") && said[1].contains(" from path:"), "{said:?}");
  assert!(
    pointer.dir.starts_with(registry.versions().join("echo")) && pointer.dir.join("echo.py").is_file()
  );
  assert_eq!(pointer.service, None);

  // Served from the next reading, the host's process the same.
  host.placed.reread();
  assert_eq!(called(&host, &session, "echo_back", json!({ "text": "hi" })).await, "v1:hi");

  // Booked, since the session's record shows it nothing by name.
  assert!(!host.find(&session, "fixture", "echo_back").unwrap().resident());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tool_put_from_a_git_address_or_by_a_bare_name_is_cloned_and_its_clone_taken_away() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let cloned = scratch.echo("cloned", "cloned_back", "git");

  committed(&cloned);

  let address = format!("file://{}", cloned.display());
  let said = put(&registry, &put_of(&address)).unwrap();

  assert!(said.last().unwrap().ends_with(&format!(" from git:{address}")), "{said:?}");

  // A bare name is made into its repository's name and put from the source template.
  let named = scratch.dir.path().join("repos").join("bmo-named");

  fs::create_dir_all(named.parent().unwrap()).unwrap();
  fs::rename(scratch.echo("named", "named_back", "name"), &named).unwrap();
  committed(&named);

  let template = format!("file://{}/{{repo}}", named.parent().unwrap().display());
  let said = put(
    &registry,
    &Put {
      source: Some(template),
      ..put_of("named")
    },
  )
  .unwrap();

  assert!(said.last().unwrap().ends_with(" from name:named"), "{said:?}");
  assert!(fs::read_dir(registry.build()).unwrap().next().is_none(), "every clone is taken away");

  let host = scratch.host();
  let session = scratch.session(&[]);

  assert_eq!(called(&host, &session, "cloned_back", json!({ "text": "a" })).await, "git:a");
  assert_eq!(called(&host, &session, "named_back", json!({ "text": "b" })).await, "name:b");
}

#[test]
fn a_build_is_run_into_the_new_version_and_the_manifest_and_its_schemas_copied_beside_it() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let built = manifest("built", "built_back", "build = 'cp echo.py mark.txt \"$BMO_TOOL_OUT/\"'\n", "");
  let folder = scratch.source("built", &built, "built");

  fs::write(folder.join("left.txt"), "not placed").unwrap();
  put(&registry, &put_of(folder.to_string_lossy())).unwrap();

  let placed = registry.get("built").unwrap().dir;
  let mut files: Vec<String> = fs::read_dir(&placed)
    .unwrap()
    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
    .collect();

  files.sort();
  assert_eq!(files, ["echo.json", "echo.py", "mark.txt", "tool.toml"]);

  let failing = manifest("failing", "failing_back", "build = 'echo it broke >&2; exit 3'\n", "");
  let refused =
    put(&registry, &put_of(scratch.source("failing", &failing, "x").to_string_lossy())).unwrap_err();

  assert!(refused.contains("its build failed") && refused.contains("it broke"), "{refused}");
  assert!(registry.get("failing").is_none());
}

#[test]
fn a_put_is_refused_naming_what_failed_and_nothing_is_placed() {
  let scratch = Scratch::new();
  let registry = scratch.registry();

  // Its source is no folder, git address or tool name, or a bare name with no template.
  assert!(
    put(&registry, &put_of("no/such/folder"))
      .unwrap_err()
      .contains("is no folder, git address or tool name")
  );
  assert!(put(&registry, &put_of("echo")).unwrap_err().contains("none is configured"));
  assert!(
    put(
      &registry,
      &Put {
        source: Some("file:///repos".to_owned()),
        ..put_of("echo")
      }
    )
    .unwrap_err()
    .contains("holds no {repo}")
  );

  // Its manifest does not read, or a schema is no JSON Schema.
  let unread = scratch.source(
    "unread",
    &manifest("unread", "unread_back", "", "").replace("protocol = 1", "protocol = 2"),
    "x",
  );

  assert!(
    put(&registry, &put_of(unread.to_string_lossy()))
      .unwrap_err()
      .contains("its manifest does not read")
  );

  let schemaless = scratch.echo("schemaless", "schemaless_back", "x");

  fs::write(schemaless.join("echo.json"), r#"{ "type": 5 }"#).unwrap();
  assert!(
    put(&registry, &put_of(schemaless.to_string_lossy()))
      .unwrap_err()
      .contains("is not a JSON Schema")
  );

  // It asks for a capability or a grant no --allow names.
  let acting = scratch.source("acting", &manifest("acting", "acting_note", "", "grant = \"notes\"\n"), "x");
  let refused = put(
    &registry,
    &Put {
      allow: Vec::new(),
      ..put_of(acting.to_string_lossy())
    },
  )
  .unwrap_err();

  assert_eq!(refused, "acting asks for what no --allow names: fixture.read, notes");
  assert!(put(&registry, &put_of(acting.to_string_lossy())).unwrap_err().contains("names: notes"));

  // One of its entries is another registered tool's.
  put(&registry, &put_of(scratch.echo("first", "shared_back", "x").to_string_lossy())).unwrap();

  let second = scratch.echo("second", "shared_back", "x");

  assert_eq!(
    put(&registry, &put_of(second.to_string_lossy())).unwrap_err(),
    "fixture/shared_back is the registered tool first's already"
  );

  let (pointers, _) = registry.entries();

  assert_eq!(pointers.iter().map(|pointer| pointer.name.as_str()).collect::<Vec<_>>(), ["first"]);
}

#[test]
fn a_put_and_a_services_registration_never_replace_each_other_and_one_service_never_replaces_another() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let owned = scratch.echo("owned", "owned_back", "service");
  let by_hand = scratch.echo("handmade", "handmade_back", "hand");
  let as_service = |folder: &Path, service: &str| Put {
    service: Some(service.to_owned()),
    placed: true,
    ..put_of(folder.to_string_lossy())
  };

  put(&registry, &as_service(&owned, "alpha")).unwrap();
  put(&registry, &put_of(by_hand.to_string_lossy())).unwrap();

  assert_eq!(
    put(&registry, &put_of(owned.to_string_lossy())).unwrap_err(),
    "owned is registered for the service alpha, which alone replaces or takes it"
  );
  assert!(
    put(&registry, &as_service(&owned, "beta"))
      .unwrap_err()
      .contains("for the service alpha")
  );
  assert_eq!(
    put(&registry, &as_service(&by_hand, "alpha")).unwrap_err(),
    "handmade was put by hand, and a service never replaces it"
  );

  // The service replaces its own, and alone takes it.
  assert!(
    put(&registry, &as_service(&owned, "alpha"))
      .unwrap()
      .last()
      .unwrap()
      .starts_with("upgraded owned")
  );
  assert!(take(&registry, "owned", None).unwrap_err().contains("take it with --service alpha"));
  assert!(take(&registry, "handmade", Some("alpha")).unwrap_err().contains("was put by hand"));
  assert!(take(&registry, "owned", Some("alpha")).unwrap()[0].starts_with("took owned "));
  assert!(
    take(&registry, "owned", Some("alpha"))
      .unwrap_err()
      .contains("no tool owned is registered")
  );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upgrade_moves_the_pointer_onto_a_new_version_and_a_take_takes_the_tool_out_of_the_next_reading()
{
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let folder = scratch.echo("echo", "echo_back", "v1");

  put(&registry, &put_of(folder.to_string_lossy())).unwrap();

  let host = scratch.host();
  let session = scratch.session(&[]);
  let first = registry.get("echo").unwrap();

  fs::write(folder.join("mark.txt"), "v2").unwrap();

  let said = put(&registry, &put_of(folder.to_string_lossy())).unwrap();
  let second = registry.get("echo").unwrap();

  assert_eq!(
    said.last().unwrap(),
    &format!("upgraded echo from {} to {}", first.version, second.version)
  );
  assert_ne!(first.dir, second.dir);

  host.placed.reread();
  assert_eq!(called(&host, &session, "echo_back", json!({ "text": "a" })).await, "v2:a");

  // The old version, which nothing holds, is gone with the reading that retired it.
  assert!(!first.dir.exists() && second.dir.exists());

  take(&registry, "echo", None).unwrap();
  host.placed.reread();

  assert!(host.find(&session, "fixture", "echo_back").is_none());
  assert!(!second.dir.exists());
}

// A registry tool a tools-root folder displaces keeps its version while its pointer names it, and is served again
// once the colliding folder goes (docs/registry.md, 「Versions」).
#[tokio::test(flavor = "multi_thread")]
async fn a_registry_tool_displaced_by_a_name_collision_is_served_again_once_the_tools_root_folder_goes() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let folder = scratch.echo("echo", "echo_back", "registry");

  put(&registry, &put_of(folder.to_string_lossy())).unwrap();

  let root = scratch.dir.path().join("tools");
  fs::create_dir_all(&root).unwrap();

  let placed =
    Placed::new(Some(root.clone()), scratch.sockets.path().to_owned()).registered(registry.clone());
  placed.reread();

  let host = Arc::new(Host {
    placed: Arc::new(placed),
    pass_env: Vec::new(),
    drain: mcp_host::drain::Drain::default(),
  });
  let session = scratch.session(&[]);
  let pointer = registry.get("echo").unwrap();

  assert_eq!(called(&host, &session, "echo_back", json!({ "text": "a" })).await, "registry:a");

  // A folder of the tools root takes the full name, and the registry's tool is left out of the catalogue.
  scratch.placed_in(&root, "echo", &manifest("echo", "echo_back", "", ""), "root");
  host.placed.reread();

  assert_eq!(called(&host, &session, "echo_back", json!({ "text": "a" })).await, "root:a");
  assert!(pointer.dir.exists(), "a displaced version stays while its pointer names it");

  // The colliding folder goes, and the pointer's version is served again.
  fs::remove_dir_all(root.join("echo")).unwrap();
  host.placed.reread();

  assert_eq!(called(&host, &session, "echo_back", json!({ "text": "a" })).await, "registry:a");
  assert!(pointer.dir.exists());
}

#[tokio::test]
async fn a_put_an_upgrade_and_a_take_are_each_told_once_on_the_event_stream() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let folder = scratch.echo("echo", "echo_back", "v1");

  put(&registry, &put_of(folder.to_string_lossy())).unwrap();

  // What the host serves as it starts is told as standing, with no event.
  let host = scratch.host();
  let events = &host.placed.events;

  assert!(events.after(None).unwrap().is_empty());

  put(&registry, &put_of(scratch.echo("other", "other_back", "o").to_string_lossy())).unwrap();
  host.placed.reread();
  host.placed.reread();

  let told = events.after(None).unwrap();
  let placed = ToolChange {
    tool: "other".to_owned(),
    namespaces: vec!["fixture".to_owned()],
    entries: vec![Booked {
      tool: "fixture/other_back".to_owned(),
      reached_by: ReachedBy::Call,
      summary: "Says the text back.".to_owned(),
    }],
    replaced: false,
  };

  assert_eq!(told.len(), 1, "{told:?}");
  assert_eq!((told[0].kind, &told[0].data), (EventKind::ToolPlaced, &placed));

  fs::write(folder.join("mark.txt"), "v2").unwrap();
  put(&registry, &put_of(folder.to_string_lossy())).unwrap();
  take(&registry, "other", None).unwrap();
  host.placed.reread();

  let told = events.after(Some(told[0].id)).unwrap();
  let kinds: Vec<(EventKind, &str, bool)> = told
    .iter()
    .map(|event| (event.kind, event.data.tool.as_str(), event.data.replaced))
    .collect();

  assert_eq!(kinds, [(EventKind::ToolPlaced, "echo", true), (EventKind::ToolRemoved, "other", false)]);
  assert_eq!(told[0].data.entries[0].tool, "fixture/echo_back");
  assert_eq!(told[1].data, placed);
  assert!(told[0].id < told[1].id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_in_flight_goes_on_in_the_version_it_started_in_through_an_upgrade_and_a_take() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let folder = scratch.echo("echo", "echo_back", "v1");

  put(&registry, &put_of(folder.to_string_lossy())).unwrap();

  let host = scratch.host();
  let session = scratch.session(&[]);
  let (started, release) = (scratch.dir.path().join("started"), scratch.dir.path().join("release"));

  mkfifo(&started);
  mkfifo(&release);

  let first = registry.get("echo").unwrap();
  let in_flight = {
    let (host, session) = (host.clone(), session.clone());
    let arguments = json!({ "text": "held", "started": started, "release": release });

    tokio::spawn(async move { called(&host, &session, "echo_back", arguments).await })
  };

  // Opened for reading once the call's process opens it for writing: the call runs in the first version.
  tokio::task::spawn_blocking(move || fs::File::open(started).map(drop))
    .await
    .unwrap()
    .unwrap();

  fs::write(folder.join("mark.txt"), "v2").unwrap();
  put(&registry, &put_of(folder.to_string_lossy())).unwrap();
  host.placed.reread();

  let second = registry.get("echo").unwrap();

  assert!(first.dir.exists(), "a version a call runs in stays");
  assert_eq!(called(&host, &session, "echo_back", json!({ "text": "next" })).await, "v2:next");

  take(&registry, "echo", None).unwrap();
  host.placed.reread();
  assert!(first.dir.exists(), "a version a call runs in stays");

  tokio::task::spawn_blocking(move || fs::OpenOptions::new().write(true).open(release)?.write_all(b"go"))
    .await
    .unwrap()
    .unwrap();

  assert_eq!(in_flight.await.unwrap(), "v1:held");

  // Nothing holds either version now, so the next sweep takes both away.
  host.placed.sweep();
  assert!(!first.dir.exists() && !second.dir.exists());
}

// One JSON-RPC request to the in-process host's namespace, answered as the bytes it sent back.
async fn rpc(port: u16, token: &str, namespace: &str, method: &str, params: Value) -> (StatusCode, Bytes) {
  let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  let (mut link, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

  tokio::spawn(connection);

  let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
  let asked = Request::post(format!("/mcp/{namespace}"))
    .header("host", "127.0.0.1")
    .header("content-type", "application/json")
    .header("accept", "application/json, text/event-stream")
    .header("mcp-protocol-version", "2025-06-18")
    .header("authorization", format!("Bearer {token}"))
    .body(Full::new(Bytes::from(body.to_string())))
    .unwrap();
  let response = link.send_request(asked).await.unwrap();
  let status = response.status();

  (status, response.into_body().collect().await.unwrap().to_bytes())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_sessions_initialize_and_tools_list_stay_byte_for_byte_while_its_toolbook_changes() {
  let scratch = Scratch::new();
  let registry = scratch.registry();

  put(&registry, &put_of(scratch.echo("resident", "echo_back", "r").to_string_lossy())).unwrap();

  let host = scratch.host();
  let records = Arc::new(Records::load(&scratch.dir.path().join("sessions")).unwrap());
  let token = records.issue(scratch.session(&["fixture/echo_back"])).unwrap();
  let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let port = listener.local_addr().unwrap().port();
  let app = server::tools(
    records,
    host.clone(),
    Arc::new(AsyncCalls::new(std::env::temp_dir().join("mcp-host-calls"), WakeTries::default())),
  );

  tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

  let opening = json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "probe", "version": "1" } });
  let front = || async {
    let (status, initialize) = rpc(port, &token, "fixture", "initialize", opening.clone()).await;
    let (_, listed) = rpc(port, &token, "fixture", "tools/list", json!({})).await;
    let (_, toolbook) = rpc(port, &token, "toolbook", "initialize", opening.clone()).await;

    assert_eq!(status, StatusCode::OK);
    (initialize, listed, toolbook)
  };
  let booked = || async {
    let (_, answer) =
      rpc(port, &token, "toolbook", "tools/call", json!({ "name": "list", "arguments": {} })).await;

    text(&serde_json::from_slice::<Value>(&answer).unwrap()["result"])
  };
  let before = front().await;
  let book_before = booked().await;

  assert!(String::from_utf8_lossy(&before.1).contains("echo_back"));
  assert!(!book_before.contains("fixture/other_back"), "{book_before}");

  // A tool put beside it, an upgrade of the resident one, and a take: each moves the toolbook alone.
  put(&registry, &put_of(scratch.echo("other", "other_back", "o").to_string_lossy())).unwrap();
  host.placed.reread();

  assert_eq!(front().await, before);
  assert!(booked().await.contains("fixture/other_back · call — Says the text back."));

  put(&registry, &put_of(scratch.echo("resident", "echo_back", "r2").to_string_lossy())).unwrap();
  take(&registry, "other", None).unwrap();
  host.placed.reread();

  assert_eq!(front().await, before);
  assert_eq!(booked().await, book_before);
}

#[test]
fn the_command_says_what_it_did_and_ends_0_1_or_2_as_it_did_refused_or_could_not_read_its_words() {
  let scratch = Scratch::new();
  let folder = scratch.echo("echo", "echo_back", "v1");
  let pocket = |args: &[&str]| {
    Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .arg("--data-root")
      .arg(scratch.data())
      .args(args)
      .env_remove("BMO_POCKET_SOURCE")
      .output()
      .unwrap()
  };
  let folder = folder.to_str().unwrap();

  let refused = pocket(&["put", folder]);

  assert_eq!(refused.status.code(), Some(1));
  assert!(String::from_utf8_lossy(&refused.stdout).contains("echo asks for the capability fixture.read"));
  assert_eq!(
    String::from_utf8_lossy(&refused.stderr),
    "bmo-pocket: echo asks for what no --allow names: fixture.read\n"
  );

  let placed = pocket(&["put", folder, "--allow", "fixture.read"]);

  assert_eq!(placed.status.code(), Some(0), "{}", String::from_utf8_lossy(&placed.stderr));
  assert!(String::from_utf8_lossy(&placed.stdout).contains("put echo "));

  let listed = String::from_utf8_lossy(&pocket(&["list"]).stdout).into_owned();

  assert!(listed.starts_with("echo ") && listed.contains(": fixture/echo_back"), "{listed}");

  let pointers: Value = serde_json::from_slice(&pocket(&["list", "--json"]).stdout).unwrap();

  assert_eq!(pointers[0]["name"], "echo");
  assert!(pointers[0]["putAt"].is_u64() && pointers[0].get("put_at").is_none(), "{pointers}");
  assert_eq!(pocket(&["take", "echo"]).status.code(), Some(0));
  assert_eq!(String::from_utf8_lossy(&pocket(&["list"]).stdout), "No tool is registered.\n");
  assert_eq!(pocket(&["frobnicate"]).status.code(), Some(2));
}

// The account service standing in on a loopback port of its own: it keeps each declaration's path and body, and
// answers each 200, or 409 where `refusing` says so.
struct AccountService {
  port: u16,
  kept: Arc<std::sync::Mutex<Vec<(String, String, Value)>>>,
}

impl AccountService {
  fn start(refusing: bool) -> Self {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let kept: Arc<std::sync::Mutex<Vec<(String, String, Value)>>> = Arc::default();
    let keeping = kept.clone();

    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

      runtime.block_on(async move {
        let listener = TcpListener::from_std(listener).unwrap();

        loop {
          let (stream, _) = listener.accept().await.unwrap();
          let keeping = keeping.clone();
          let answer = hyper::service::service_fn(move |asked: Request<hyper::body::Incoming>| {
            let keeping = keeping.clone();

            async move {
              let path = asked.uri().path().to_owned();
              let bearer = asked
                .headers()
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
              let body = asked.into_body().collect().await.map_err(std::io::Error::other)?.to_bytes();

              keeping.lock().unwrap().push((
                path,
                bearer,
                serde_json::from_slice(&body).unwrap_or(Value::Null),
              ));

              let (status, said) = if refusing {
                (
                  StatusCode::CONFLICT,
                  json!({ "code": "kind-declared-elsewhere", "error": "declared by another unit" }),
                )
              } else {
                (StatusCode::OK, json!({}))
              };

              Ok::<_, std::io::Error>(
                http::Response::builder()
                  .status(status)
                  .body(Full::new(Bytes::from(said.to_string())))
                  .unwrap(),
              )
            }
          });

          tokio::spawn(
            hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), answer),
          );
        }
      });
    });

    Self { port, kept }
  }

  fn chain(&self) -> mcp_host::chain::Chain {
    mcp_host::chain::Chain {
      address: format!("http://127.0.0.1:{}", self.port),
      token: "chain-token".to_owned(),
    }
  }

  fn kept(&self) -> Vec<(String, String, Value)> {
    self.kept.lock().unwrap().clone()
  }
}

// "An app on the delegation chain" (docs/api.md): a put of a tool naming an app declares its two settings, offered
// narrowing at the platform and the units and on nearest down to the person, each with the manifest's default, and
// places the tool; a put of a tool naming none declares nothing; and one whose account service refuses, or names
// none, places nothing.
#[test]
fn a_put_of_an_apps_tool_declares_its_two_settings_on_the_delegation_chain_first() {
  let scratch = Scratch::new();
  let registry = scratch.registry();
  let service = AccountService::start(false);
  let app =
    "[app]\nkey = \"almanac\"\nname = \"Almanac\"\ndescription = \"Says when to sow.\"\noffered = false\n";
  let folder = scratch.source("almanac", &manifest("almanac", "sow_when", app, ""), "a");
  let said = put(
    &registry,
    &Put {
      chain: Some(service.chain()),
      ..put_of(folder.to_string_lossy())
    },
  )
  .unwrap();

  assert!(
    said
      .iter()
      .any(|line| line == "declared the setting app.almanac.offered on the delegation chain")
  );
  assert!(registry.get("almanac").is_some());

  let kept = service.kept();
  let (offered, on) = (&kept[0], &kept[1]);

  assert_eq!(kept.len(), 2);
  assert_eq!(offered.0, "/chain/kinds/app.almanac.offered");
  assert_eq!(offered.1, "Bearer chain-token");
  assert_eq!(offered.2["declaredBy"], "pocket");
  assert_eq!(offered.2["mode"], "narrowing");
  assert_eq!(offered.2["default"], false);
  assert_eq!(offered.2["levels"], json!(["platform", "unit"]));
  assert!(offered.2["description"].as_str().unwrap().contains("Almanac: Says when to sow."));
  assert_eq!(on.0, "/chain/kinds/app.almanac.on");
  assert_eq!(on.2["mode"], "nearest");
  assert_eq!(on.2["default"], true);
  assert_eq!(on.2["levels"], json!(["platform", "unit", "person"]));
  assert_eq!(on.2["personWriter"], "both");

  // A tool naming no app declares nothing.
  put(
    &registry,
    &Put {
      chain: Some(service.chain()),
      ..put_of(scratch.echo("plain", "plain_echo", "p").to_string_lossy())
    },
  )
  .unwrap();
  assert_eq!(service.kept().len(), 2);

  // Refused by the account service, or with none named, nothing is placed.
  let other = scratch.source("sower", &manifest("sower", "sow_where", app, ""), "s");
  let refusing = AccountService::start(true);
  let refused = put(
    &registry,
    &Put {
      chain: Some(refusing.chain()),
      ..put_of(other.to_string_lossy())
    },
  )
  .unwrap_err();

  assert!(
    refused.contains("did not take the setting app.almanac.offered") && refused.contains("409"),
    "{refused}"
  );
  assert!(registry.get("sower").is_none());
  assert!(
    put(&registry, &put_of(other.to_string_lossy()))
      .unwrap_err()
      .contains("no account service is named")
  );
  assert!(registry.get("sower").is_none());
}

// "An app on the delegation chain" (docs/api.md): `app` declares a screen-only app's two settings from a file holding
// the `[app]` table alone, as a put declares a tool's, and places nothing; it ends 1 where the file holds anything
// else, the account service refuses or none is named, and 2 where its words do not read.
#[test]
fn the_app_verb_declares_a_screen_only_apps_two_settings_and_places_nothing() {
  let scratch = Scratch::new();
  let service = AccountService::start(false);
  let file = scratch.data().join("app.toml");
  let pocket = |args: &[&str], address: Option<String>| {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bmo-pocket"));

    command.arg("--data-root").arg(scratch.data()).args(args).env_remove("WEB_TOKEN_FILE");

    match address {
      Some(address) => command.env("WEB_ADDRESS", address).env("WEB_TOKEN", "chain-token"),
      None => command.env_remove("WEB_ADDRESS").env_remove("WEB_TOKEN"),
    };

    command.output().unwrap()
  };
  let address = || Some(format!("http://127.0.0.1:{}", service.port));

  fs::create_dir_all(scratch.data()).unwrap();
  fs::write(
    &file,
    "[app]\nkey = \"ledger\"\nname = \"Ledger\"\ndescription = \"Keeps the accounts.\"\non = false\n",
  )
  .unwrap();

  let declared = pocket(&["app", file.to_str().unwrap()], address());

  assert_eq!(declared.status.code(), Some(0), "{}", String::from_utf8_lossy(&declared.stderr));
  assert_eq!(
    String::from_utf8_lossy(&declared.stdout),
    "declared the setting app.ledger.offered on the delegation chain\n\
     declared the setting app.ledger.on on the delegation chain\n"
  );

  let kept = service.kept();

  assert_eq!(kept.len(), 2);
  assert_eq!(kept[0].0, "/chain/kinds/app.ledger.offered");
  assert_eq!(kept[0].1, "Bearer chain-token");
  assert_eq!(kept[0].2["declaredBy"], "pocket");
  assert_eq!(kept[0].2["default"], true);
  assert!(kept[0].2["description"].as_str().unwrap().contains("Ledger: Keeps the accounts."));
  assert_eq!(kept[1].0, "/chain/kinds/app.ledger.on");
  assert_eq!(kept[1].2["mode"], "nearest");
  assert_eq!(kept[1].2["default"], false);
  assert!(scratch.registry().entries().0.is_empty());

  // The same declarations a put of a tool naming the same app makes.
  let tool =
    "[app]\nkey = \"ledger\"\nname = \"Ledger\"\ndescription = \"Keeps the accounts.\"\non = false\n";
  let folder = scratch.source("ledger", &manifest("ledger", "tally", tool, ""), "l");

  put(
    &scratch.registry(),
    &Put {
      chain: Some(service.chain()),
      ..put_of(folder.to_string_lossy())
    },
  )
  .unwrap();
  assert_eq!(service.kept()[2..], kept[..]);

  // A file holding more than the app, an account service refusing or none named: refused, nothing declared.
  fs::write(
    &file,
    "[app]\nkey = \"ledger\"\nname = \"Ledger\"\ndescription = \"x\"\n[tool]\nname = \"x\"\n",
  )
  .unwrap();

  let more = pocket(&["app", file.to_str().unwrap()], address());

  assert_eq!(more.status.code(), Some(1));
  assert!(String::from_utf8_lossy(&more.stderr).contains("tool"));
  assert_eq!(service.kept().len(), 4);

  fs::write(&file, "[app]\nkey = \"ledger\"\nname = \"Ledger\"\ndescription = \"x\"\n").unwrap();

  let refusing = AccountService::start(true);
  let refused =
    pocket(&["app", file.to_str().unwrap()], Some(format!("http://127.0.0.1:{}", refusing.port)));

  assert_eq!(refused.status.code(), Some(1));
  assert!(String::from_utf8_lossy(&refused.stderr).contains("did not take the setting app.ledger.offered"));

  let unnamed = pocket(&["app", file.to_str().unwrap()], None);

  assert_eq!(unnamed.status.code(), Some(1));
  assert!(String::from_utf8_lossy(&unnamed.stderr).contains("no account service is named"));
  assert_eq!(pocket(&["app"], address()).status.code(), Some(2));
  assert_eq!(service.kept().len(), 4);
}
