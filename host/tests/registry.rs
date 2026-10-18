//! The registry as a person and the host meet it (docs/registry.md): tools put from a folder, a git address and a
//! bare name, each refusal, a take and an upgrade, the host picking each up with no restart, a call in flight going
//! on through a take and an upgrade, and a running session's prompt front unmoved while its toolbook changes. Each
//! change reaches the host through the catalogue's reread hook, and a call is held where it stands by named pipes,
//! so no test waits on the clock.

use std::ffi::CString;
use std::fs;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use bmo_tool::Session;
use http::{Request, StatusCode};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use mcp_host::handler::Host;
use mcp_host::install::{self, Put};
use mcp_host::lock::Lock;
use mcp_host::placed::Placed;
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

  // A host serving the registry alone, its catalogue read once.
  fn host(&self) -> Arc<Host> {
    let placed = Placed::new(None, self.sockets.path().to_owned()).registered(self.registry());

    placed.reread();

    Arc::new(Host {
      lock: Lock::default(),
      placed: Arc::new(placed),
      pass_env: Vec::new(),
    })
  }

  fn session(&self, shown: &[&str]) -> Session {
    serde_json::from_value(json!({
      "line": "worker",
      "capabilities": ["fixture.read"],
      "shown": shown,
      "dataRoot": self.data(),
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
      .args([
        "-c",
        "user.name=test",
        "-c",
        "user.email=test",
        "-c",
        "init.defaultBranch=main",
      ])
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
  let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.unwrap();

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
  let response = sender.send_request(asked).await.unwrap();
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
  let app = server::tools(records, host.clone());

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
  assert_eq!(pocket(&["take", "echo"]).status.code(), Some(0));
  assert_eq!(String::from_utf8_lossy(&pocket(&["list"]).stdout), "No tool is registered.\n");
  assert_eq!(pocket(&["frobnicate"]).status.code(), Some(2));
}
