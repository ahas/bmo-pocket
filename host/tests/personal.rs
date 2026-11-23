//! Personal tools as the host meets them (docs/mcp-host.md, 「Personal tools」): each owner's tools held by that
//! owner's sessions alone, a name clashing per owner alone, the rules a personal and a platform manifest are held
//! to, and each call relayed to a stand-in runner over a socket and over a command, through the real relay, its
//! caller going away closing the passage.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bmo_tool::Session;
use mcp_host::handler::Host;
use mcp_host::placed::{Catalogue, Placed};
use mcp_host::registry::Registry;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

const SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" } } }"#;

// A personal tool's manifest: one entry `my/<entry>`, run by `./weekly`, with what `more` adds to the entry.
fn personal_manifest(name: &str, namespace: &str, entry: &str, more: &str) -> String {
  format!(
    "[tool]\nname = \"{name}\"\nprotocol = 1\nrun = [\"./weekly\", \"--quiet\"]\n\n[[entry]]\nnamespace = \"{namespace}\"\nname = \"{entry}\"\ndescription = \"Sums up the week.\"\ninput = \"echo.json\"\ncapabilities = [\"my.own\"]\n{more}"
  )
}

struct Scratch {
  dir: TempDir,
  sockets: TempDir,
}

impl Scratch {
  fn new() -> Self {
    Self {
      dir: tempfile::tempdir().unwrap(),
      sockets: tempfile::Builder::new().prefix("pt").tempdir_in("/tmp").unwrap(),
    }
  }

  fn registry(&self) -> Registry {
    Registry::of(&self.dir.path().join("data"))
  }

  // A personal tool's description placed in its owner's registry at `version`, its pointer naming `owner`.
  fn personal(&self, owner: &str, pointed_owner: Option<&str>, name: &str, version: &str, manifest: &str) {
    let owned = self.registry().owned(owner);
    let dir = owned.version_dir(name, version);

    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("tool.toml"), manifest).unwrap();
    fs::write(dir.join("echo.json"), SCHEMA).unwrap();
    fs::create_dir_all(owned.pointers()).unwrap();
    fs::write(
      owned.pointers().join(format!("{name}.json")),
      json!({
        "name": name,
        "version": version,
        "dir": dir,
        "origin": "path:/elsewhere",
        "owner": pointed_owner,
        "putAt": 1,
      })
      .to_string(),
    )
    .unwrap();
  }

  // A platform tool placed in a tools root, its entry in `namespace`.
  fn platform(&self, name: &str, namespace: &str) -> PathBuf {
    let tools = self.dir.path().join("tools");
    let placed = tools.join(name).join(".placed");

    fs::create_dir_all(&placed).unwrap();
    fs::write(
      placed.join("tool.toml"),
      format!(
        "[tool]\nname = \"{name}\"\nprotocol = 1\nrun = [\"true\"]\n\n[[entry]]\nnamespace = \"{namespace}\"\nname = \"plain\"\ndescription = \"A platform tool.\"\ninput = \"echo.json\"\ncapabilities = [\"plain.read\"]\n"
      ),
    )
    .unwrap();
    fs::write(placed.join("echo.json"), SCHEMA).unwrap();
    tools
  }

  fn host(&self) -> Arc<Host> {
    let relay = vec![env!("CARGO_BIN_EXE_bmo-pocket").to_owned(), "relay".to_owned()];
    let placed = Placed::new(None, self.sockets.path().to_owned())
      .registered(self.registry())
      .relayed_by(relay);

    placed.reread();

    Arc::new(Host {
      placed: Arc::new(placed),
      pass_env: Vec::new(),
      drain: mcp_host::drain::Drain::default(),
    })
  }

  fn runner_socket(&self) -> PathBuf {
    self.sockets.path().join("runner.sock")
  }
}

// A session acting for `owner`, its runner the one given.
fn session(owner: Option<&str>, runner: Value) -> Session {
  let mut record = json!({
    "holderName": "tester",
    "namespaces": ["plain"],
    "capabilities": ["plain.read"],
    "grants": [],
    "variables": { "SECRET_LOOKING": "kept-home" },
  });

  if let Some(owner) = owner {
    record["ownerAccount"] = json!(owner);
  }

  if !runner.is_null() {
    record["runner"] = runner;
  }

  serde_json::from_value(record).unwrap()
}

fn text(answer: &Value) -> String {
  answer["content"][0]["text"].as_str().unwrap_or_default().to_owned()
}

// The personal entries a session holds, each with the owner of the tool it is held of.
fn personal_held(host: &Host, session: &Session) -> Vec<(String, String)> {
  host
    .held(session)
    .into_iter()
    .filter_map(|entry| Some((entry.full_name(), entry.tool.owner.clone()?)))
    .collect()
}

// "two owners each holding a personal tool of the same full name each list, describe and call their own; a session
// naming one owner neither lists nor calls the other's, and one naming no owner holds neither"
#[test]
fn each_owner_holds_its_own_personal_tools_and_no_other_owners() {
  let scratch = Scratch::new();

  scratch.personal(
    "a1",
    Some("a1"),
    "weekly",
    "v1",
    &personal_manifest("weekly", "my", "weekly_summary", ""),
  );
  scratch.personal(
    "b2",
    Some("b2"),
    "weekly",
    "v7",
    &personal_manifest("weekly", "my", "weekly_summary", ""),
  );

  let host = scratch.host();

  assert_eq!(
    personal_held(&host, &session(Some("a1"), Value::Null)),
    [("my/weekly_summary".to_owned(), "a1".to_owned())]
  );
  assert_eq!(
    personal_held(&host, &session(Some("b2"), Value::Null)),
    [("my/weekly_summary".to_owned(), "b2".to_owned())]
  );
  assert!(personal_held(&host, &session(Some("c3"), Value::Null)).is_empty());
  assert!(personal_held(&host, &session(None, Value::Null)).is_empty());

  // Each owner's is found as its own, the other's version never.
  let found = host.find_full(&session(Some("b2"), Value::Null), "my/weekly_summary", false).unwrap();

  assert_eq!(found.tool.version(), "v7");
  assert!(host.find_full(&session(Some("c3"), Value::Null), "my/weekly_summary", true).is_none());
  // Booked, and left out of the issuer's view of the catalogue.
  assert!(!found.resident());
  assert!(host.every().iter().all(|entry| entry.tool.owner.is_none()));
}

// "a platform tool in the namespace `my` and a personal tool outside its rules are left out"
#[test]
fn a_platform_tool_in_my_and_a_personal_tool_outside_its_rules_are_left_out() {
  let scratch = Scratch::new();
  let tools = scratch.platform("squatter", "my");

  scratch.personal(
    "a1",
    Some("a1"),
    "elsewhere",
    "v1",
    &personal_manifest("elsewhere", "notes", "jot", ""),
  );
  scratch.personal(
    "a1",
    Some("a1"),
    "networked",
    "v1",
    &format!(
      "{}\n[permissions]\nnetwork = [\"example.com\"]\n",
      personal_manifest("networked", "my", "fetch", "")
    ),
  );
  scratch.personal(
    "a1",
    Some("a1"),
    "timed",
    "v1",
    &personal_manifest("timed", "my", "tick", "[[entry.schedule]]\ncron = \"0 9 * * 1\"\n"),
  );
  scratch.personal("a1", Some("b2"), "misplaced", "v1", &personal_manifest("misplaced", "my", "lost", ""));
  scratch.personal("a1", Some("a1"), "kept", "v1", &personal_manifest("kept", "my", "kept", ""));

  let registry = scratch.registry();
  let (catalogue, faults) = Catalogue::read_all(Some(&tools), Some(&registry), None);
  let said = faults.join("\n");

  assert!(catalogue.tools.is_empty(), "{said}");
  assert!(
    said.contains("tools/squatter: my/plain: the namespace `my` is the personal tools' own"),
    "{said}"
  );
  assert!(
    said.contains("registry/elsewhere: notes/jot: a personal tool's entries stand in the namespace `my`")
  );
  assert!(said.contains("registry/networked: it asks for the network or a secret"), "{said}");
  assert!(said.contains("registry/timed: my/tick: a personal tool's entry"), "{said}");
  assert!(
    said.contains("registry/misplaced: it names no owner, or another than its folder's"),
    "{said}"
  );
  assert_eq!(
    catalogue.personal["a1"].iter().map(|tool| tool.folder.as_str()).collect::<Vec<_>>(),
    ["kept"]
  );
}

// A stand-in runner on a socket answering each passage with what `answer` makes of its asking line, every asking
// line sent back on the channel.
fn socket_runner(
  socket: &Path,
  answer: fn(&Value) -> Vec<Value>,
) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
  let listener = UnixListener::bind(socket).unwrap();
  let (asked, asking) = tokio::sync::mpsc::unbounded_channel();

  tokio::spawn(async move {
    while let Ok((stream, _)) = listener.accept().await {
      let asked = asked.clone();

      tokio::spawn(async move {
        let (reading, mut writing) = stream.into_split();
        let mut line = String::new();
        let mut reading = BufReader::new(reading);

        reading.read_line(&mut line).await.unwrap();

        let first: Value = serde_json::from_str(&line).unwrap();

        for answered in answer(&first) {
          writing.write_all(format!("{answered}\n").as_bytes()).await.unwrap();
        }

        let _ = asked.send(first);
      });
    }
  });

  asking
}

fn answered_with_text(asked: &Value) -> Vec<Value> {
  let said = asked["call"]["arguments"]["text"].as_str().unwrap_or_default();
  let result = json!({ "content": [{ "type": "text", "text": format!("ran:{said}") }], "isError": false });

  vec![
    json!({ "t": "said", "line": "warming up" }),
    json!({ "t": "end", "code": 0, "out": result.to_string() }),
  ]
}

// "a call is relayed to a stand-in runner over a socket and over a command, the tool's run words, version and call
// reaching it and its answer, its errors and its code coming back as the tool's"
#[tokio::test]
async fn a_call_is_relayed_over_a_socket_and_over_a_command_as_the_tool_answered() {
  let scratch = Scratch::new();

  scratch.personal(
    "a1",
    Some("a1"),
    "weekly",
    "v1",
    &personal_manifest("weekly", "my", "weekly_summary", ""),
  );

  let host = scratch.host();
  let socket = scratch.runner_socket();
  let mut asking = socket_runner(&socket, answered_with_text);
  let record = session(Some("a1"), json!({ "socket": socket }));
  let entry = host.find_full(&record, "my/weekly_summary", false).unwrap();
  let answer = host.answer(&entry, &record, json!({ "text": "hello" })).await.unwrap();

  assert_eq!(text(&answer), "ran:hello", "{answer}");

  let asked = asking.recv().await.unwrap();

  assert_eq!(asked["owner"], "a1");
  assert_eq!(asked["tool"], "weekly");
  assert_eq!(asked["version"], "v1");
  assert_eq!(asked["run"], json!(["./weekly", "--quiet"]));
  assert_eq!(asked["call"]["kind"], "call");
  assert_eq!(asked["call"]["entry"], "my/weekly_summary");
  // The record reaches the PC without its variables, its runner or a call-back socket.
  assert_eq!(asked["call"]["session"]["ownerAccount"], "a1");
  assert!(asked["call"]["session"].get("variables").is_none(), "{asked}");
  assert!(asked["call"]["session"].get("runner").is_none(), "{asked}");
  assert!(asked["call"].get("callback").is_none(), "{asked}");

  // Over a command: a runner started for the call, answering a failure the tool's own code carries.
  let script = r#"import json, sys
asked = json.loads(sys.stdin.readline())
print(json.dumps({"t": "said", "line": "no week " + asked["version"]}), flush=True)
print(json.dumps({"t": "end", "code": 3, "out": ""}), flush=True)
"#;
  let commanded = session(Some("a1"), json!({ "command": ["python3", "-c", script] }));
  let answer = host.answer(&entry, &commanded, json!({ "text": "x" })).await.unwrap();
  let said = text(&answer);

  assert_eq!(answer["isError"], true);
  assert!(said.contains("it ended with code 3") && said.contains("no week v1"), "{said}");
}

// "a record naming no runner fails the call saying so; a refusal or a runner that cannot be reached fails it saying
// why"
#[tokio::test]
async fn a_call_with_no_runner_a_refusal_or_no_runner_standing_fails_saying_why() {
  let scratch = Scratch::new();

  scratch.personal(
    "a1",
    Some("a1"),
    "weekly",
    "v1",
    &personal_manifest("weekly", "my", "weekly_summary", ""),
  );

  let host = scratch.host();
  let fails = async |record: &Session| {
    let entry = host.find_full(record, "my/weekly_summary", false).unwrap();
    let answer = host.answer(&entry, record, json!({})).await.unwrap();

    assert_eq!(answer["isError"], true, "{answer}");
    text(&answer)
  };

  let none = fails(&session(Some("a1"), Value::Null)).await;

  assert!(none.contains("names no runner"), "{none}");

  let socket = scratch.runner_socket();
  let _asking = socket_runner(&socket, |_| vec![json!({ "t": "refused", "why": "bubblewrap is missing" })]);
  let refused = fails(&session(Some("a1"), json!({ "socket": socket }))).await;

  assert!(refused.contains("the runner did not run it: bubblewrap is missing"), "{refused}");

  let gone = fails(&session(Some("a1"), json!({ "socket": "/nonexistent/runner.sock" }))).await;

  assert!(gone.contains("could not be reached"), "{gone}");
}

// "a call whose caller goes away mid-call ends the relay, whose passage the stand-in runner sees close"
#[tokio::test]
async fn a_call_whose_caller_goes_away_closes_the_runner_s_passage() {
  let scratch = Scratch::new();

  scratch.personal(
    "a1",
    Some("a1"),
    "weekly",
    "v1",
    &personal_manifest("weekly", "my", "weekly_summary", ""),
  );

  let host = scratch.host();
  let socket = scratch.runner_socket();
  let listener = UnixListener::bind(&socket).unwrap();
  let (closed, closing) = tokio::sync::oneshot::channel::<()>();
  let (asked, asking) = tokio::sync::oneshot::channel::<()>();

  // The runner takes the asking and answers nothing, until the passage closes under it.
  tokio::spawn(async move {
    let (stream, _) = listener.accept().await.unwrap();
    let mut reading = BufReader::new(stream);
    let mut line = String::new();

    reading.read_line(&mut line).await.unwrap();
    let _ = asked.send(());

    let mut rest = Vec::new();
    let _ = reading.read_to_end(&mut rest).await;
    let _ = closed.send(());
  });

  let record = session(Some("a1"), json!({ "socket": socket }));
  let calling = {
    let host = host.clone();

    tokio::spawn(async move {
      let entry = host.find_full(&record, "my/weekly_summary", false).unwrap();

      host.answer(&entry, &record, json!({})).await
    })
  };

  tokio::time::timeout(Duration::from_secs(20), asking)
    .await
    .expect("the runner was asked")
    .unwrap();
  // The caller goes away: its call dropped, as a closed connection drops it.
  calling.abort();
  tokio::time::timeout(Duration::from_secs(20), closing)
    .await
    .expect("the passage closed once its caller went")
    .unwrap();
}

// `bmo-pocket` run on the scratch data root, answering its exit code, what it said and why it refused.
fn pocket(scratch: &Scratch, words: &[&str]) -> (i32, String, String) {
  let ran = std::process::Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
    .arg("--data-root")
    .arg(scratch.dir.path().join("data"))
    .args(words)
    .output()
    .unwrap();

  (
    ran.status.code().unwrap_or(-1),
    String::from_utf8_lossy(&ran.stdout).into_owned(),
    String::from_utf8_lossy(&ran.stderr).into_owned(),
  )
}

// A built personal tool as its owner's machine holds it: the manifest, its schema, and code and data beside them.
fn built(scratch: &Scratch, folder: &str, manifest: &str) -> PathBuf {
  let dir = scratch.dir.path().join("built").join(folder);

  fs::create_dir_all(dir.join("lib")).unwrap();
  fs::write(dir.join("tool.toml"), manifest).unwrap();
  fs::write(dir.join("echo.json"), SCHEMA).unwrap();
  fs::write(dir.join("weekly"), "#!/bin/sh\necho private code\n").unwrap();
  fs::write(dir.join("lib").join("notes.txt"), "private data").unwrap();
  dir
}

// "`put --owner` places the description copy alone under the version named, refuses a manifest outside a personal
// tool's rules and a full name another of the owner's tools holds while another owner's stands beside it, points at
// a version already standing as it stands, and is taken and listed per owner"
#[test]
fn an_owner_put_places_the_description_copy_alone_under_the_version_its_machine_named() {
  let scratch = Scratch::new();
  let weekly = built(&scratch, "weekly", &personal_manifest("weekly", "my", "weekly_summary", ""));
  let (code, said, refused) =
    pocket(&scratch, &["put", "--owner", "a1", "--version", "100-abc", weekly.to_str().unwrap()]);

  assert_eq!(code, 0, "{refused}");
  assert!(said.contains("put a1's weekly 100-abc"), "{said}");
  assert!(said.contains("weekly serves my/weekly_summary"), "{said}");

  let owned = scratch.registry().owned("a1");
  let placed = owned.version_dir("weekly", "100-abc");
  let mut files: Vec<String> = fs::read_dir(&placed)
    .unwrap()
    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
    .collect();

  files.sort();
  assert_eq!(files, ["echo.json", "tool.toml"], "no code or data reaches the host");

  let pointer = owned.get("weekly").unwrap();

  assert_eq!(pointer.owner.as_deref(), Some("a1"));
  assert_eq!(pointer.origin, "machine:a1");
  assert_eq!(pointer.version, "100-abc");

  let host = scratch.host();
  let found = host.find_full(&session(Some("a1"), Value::Null), "my/weekly_summary", false).unwrap();

  assert_eq!(found.tool.version(), "100-abc");

  // Outside a personal tool's rules: a namespace not `my`, the network asked for, a bad version name.
  let elsewhere = built(&scratch, "elsewhere", &personal_manifest("elsewhere", "notes", "jot", ""));
  let (code, _, refused) = pocket(&scratch, &["put", "--owner", "a1", elsewhere.to_str().unwrap()]);

  assert_eq!(code, 1);
  assert!(refused.contains("stand in the namespace `my`"), "{refused}");

  let reaching = built(
    &scratch,
    "reaching",
    &format!(
      "{}\n[permissions]\nnetwork = [\"example.com\"]\n",
      personal_manifest("reaching", "my", "reach", "")
    ),
  );
  let (code, _, refused) = pocket(&scratch, &["put", "--owner", "a1", reaching.to_str().unwrap()]);

  assert_eq!(code, 1);
  assert!(refused.contains("the network or a secret"), "{refused}");

  let (code, _, refused) =
    pocket(&scratch, &["put", "--owner", "a1", "--version", "../up", weekly.to_str().unwrap()]);

  assert_eq!(code, 1);
  assert!(refused.contains("no version's name"), "{refused}");

  // Another of a1's tools may not take the same entry; b2's may.
  let twin = built(&scratch, "twin", &personal_manifest("twin", "my", "weekly_summary", ""));
  let (code, _, refused) = pocket(&scratch, &["put", "--owner", "a1", twin.to_str().unwrap()]);

  assert_eq!(code, 1);
  assert!(
    refused.contains("my/weekly_summary is the registered tool weekly's already"),
    "{refused}"
  );

  let (code, _, refused) = pocket(&scratch, &["put", "--owner", "b2", twin.to_str().unwrap()]);

  assert_eq!(code, 0, "{refused}");

  // A new version moves the pointer; going back to one still standing points at it as it stands.
  let (code, said, _) =
    pocket(&scratch, &["put", "--owner", "a1", "--version", "200-def", weekly.to_str().unwrap()]);

  assert_eq!(code, 0);
  assert!(said.contains("moved a1's weekly from 100-abc to 200-def"), "{said}");
  fs::write(placed.join("mark"), "kept").unwrap();

  let (code, said, _) =
    pocket(&scratch, &["put", "--owner", "a1", "--version", "100-abc", weekly.to_str().unwrap()]);

  assert_eq!(code, 0);
  assert!(said.contains("moved a1's weekly from 200-def to 100-abc"), "{said}");
  assert_eq!(
    fs::read_to_string(placed.join("mark")).unwrap(),
    "kept",
    "the standing version is not written"
  );

  // Listed per owner, with its entries and permissions; taken from the owner's registry alone.
  let (code, said, _) = pocket(&scratch, &["list", "--owner", "a1", "--json"]);
  let listed: Value = serde_json::from_str(&said).unwrap();

  assert_eq!(code, 0);
  assert_eq!(listed[0]["name"], "weekly");
  assert_eq!(listed[0]["version"], "100-abc");
  assert_eq!(listed[0]["owner"], "a1");
  assert_eq!(listed[0]["entries"][0]["name"], "my/weekly_summary");
  assert_eq!(listed[0]["entries"][0]["description"], "Sums up the week.");
  assert_eq!(listed[0]["permissions"], json!({ "network": [], "secrets": [] }));

  let (code, said, _) = pocket(&scratch, &["take", "--owner", "a1", "weekly"]);

  assert_eq!(code, 0);
  assert!(said.contains("took a1's weekly 100-abc"), "{said}");
  assert!(scratch.registry().owned("a1").get("weekly").is_none());
  assert!(scratch.registry().owned("b2").get("twin").is_some());

  let (code, _, refused) = pocket(&scratch, &["take", "--owner", "a1", "weekly"]);

  assert_eq!(code, 1);
  assert!(refused.contains("a1 holds no personal tool weekly"), "{refused}");

  let (code, _, refused) = pocket(&scratch, &["take", "--owner", "../x", "weekly"]);

  assert_eq!(code, 1);
  assert!(refused.contains("no account's name"), "{refused}");
}
