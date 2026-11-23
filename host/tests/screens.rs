//! Personal screens as the host meets them (docs/mcp-host.md, 「Personal screens」; docs/api.md, 「Personal screens」
//! and 「`[screen]`」): a put of a tool with a screen carrying the screen's words, layer and index alone; the app face
//! listing the screens of the session's owner alone, each file addressed by the digest of its index and relayed from
//! a stand-in runner only where its bytes are the index's, and a call of the screen's own tool's entries alone.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use bmo_pocket_client::transport::request;
use bmo_tool::Session;
use http::{Method, StatusCode};
use mcp_host::async_calls::{AsyncCalls, WakeTries};
use mcp_host::handler::Host;
use mcp_host::placed::Placed;
use mcp_host::records::Records;
use mcp_host::registry::Registry;
use mcp_host::schedule::Schedules;
use mcp_host::screens::sha256_hex;
use mcp_host::server;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

const SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" } } }"#;
const ISSUER: &str = "issuer-token-for-screens";
const MODULE: &str = "export default { routes: [] };\n";
const CHUNK: &str = "export const one = 1;\n";

// A personal tool's manifest with a screen: one entry `my/<entry>`, run by `./notes`.
fn manifest(name: &str, entry: &str) -> String {
  format!(
    "[tool]\nname = \"{name}\"\nprotocol = 1\nrun = [\"./notes\"]\n\n[screen]\nicon = \"i-material-symbols-note\"\n\n[[entry]]\nnamespace = \"my\"\nname = \"{entry}\"\ndescription = \"Keeps notes.\"\ninput = \"echo.json\"\ncapabilities = [\"my.own\"]\n"
  )
}

// The index of the screen's folder: the module and a chunk in a folder of its own.
fn index() -> String {
  json!({ "module.js": sha256_hex(MODULE.as_bytes()), "parts/chunk.js": sha256_hex(CHUNK.as_bytes()) })
    .to_string()
}

// A tool folder as its owner's machine built it: the manifest, its schema, its code, and its screen's build.
fn built(dir: &Path, name: &str, words: &Value) {
  let screen = dir.join("screen");

  fs::create_dir_all(screen.join("parts")).unwrap();
  fs::write(dir.join("tool.toml"), manifest(name, &format!("{name}_keep"))).unwrap();
  fs::write(dir.join("echo.json"), SCHEMA).unwrap();
  fs::write(dir.join("notes"), "#!/bin/sh\necho private code\n").unwrap();
  fs::write(screen.join("module.js"), MODULE).unwrap();
  fs::write(screen.join("parts/chunk.js"), CHUNK).unwrap();
  fs::write(screen.join("layer.json"), r#"{"layer":"0.2.0-nightly.20261123"}"#).unwrap();
  fs::write(screen.join("words.json"), words.to_string()).unwrap();
  fs::write(screen.join("files.json"), index()).unwrap();
}

fn words(name: &str) -> Value {
  json!({ "ko": { name: { "title": "notes" } }, "en": { name: { "title": "Notes" } } })
}

struct Scratch {
  dir: TempDir,
  sockets: TempDir,
}

impl Scratch {
  fn new() -> Self {
    Self {
      dir: tempfile::tempdir().unwrap(),
      sockets: tempfile::Builder::new().prefix("ps").tempdir_in("/tmp").unwrap(),
    }
  }

  fn data(&self) -> PathBuf {
    self.dir.path().join("data")
  }

  fn pocket(&self, words: &[&str]) -> (i32, String, String) {
    let ran = std::process::Command::new(env!("CARGO_BIN_EXE_bmo-pocket"))
      .arg("--data-root")
      .arg(self.data())
      .args(words)
      .output()
      .unwrap();

    (
      ran.status.code().unwrap_or(-1),
      String::from_utf8_lossy(&ran.stdout).into_owned(),
      String::from_utf8_lossy(&ran.stderr).into_owned(),
    )
  }

  // The owner's tool put as its machine built it, under `version`.
  fn put(&self, owner: &str, name: &str, version: &str) {
    let folder = self.dir.path().join("built").join(name);

    built(&folder, name, &words(&format!("my-{}", name.replace('_', "-"))));

    let (code, _, refused) =
      self.pocket(&["put", "--owner", owner, "--version", version, folder.to_str().unwrap()]);

    assert_eq!(code, 0, "{refused}");
  }

  fn host(&self) -> Arc<Host> {
    let relay = vec![env!("CARGO_BIN_EXE_bmo-pocket").to_owned(), "relay".to_owned()];
    let placed = Placed::new(None, self.sockets.path().to_owned())
      .registered(Registry::of(&self.data()))
      .relayed_by(relay);

    placed.reread();

    Arc::new(Host {
      placed: Arc::new(placed),
      pass_env: Vec::new(),
      drain: mcp_host::drain::Drain::default(),
    })
  }

  // The control socket served in this process, and the records its tokens name.
  fn serve(&self) -> (PathBuf, Arc<Records>) {
    let records = Arc::new(Records::load(&self.dir.path().join("sessions")).unwrap());
    let socket = self.sockets.path().join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let calls = Arc::new(AsyncCalls::new(self.dir.path().join("calls"), WakeTries::default()));
    let router = server::control(
      records.clone(),
      self.host(),
      Arc::new(Schedules::default()),
      calls,
      ISSUER.to_owned(),
    );

    tokio::spawn(async move { axum::serve(listener, router).await });
    (socket, records)
  }

  fn runner_socket(&self) -> PathBuf {
    self.sockets.path().join("runner.sock")
  }
}

// A session acting for `owner`, its runner the socket given.
fn session(owner: &str, runner: &Path) -> Session {
  serde_json::from_value(json!({
    "holderName": "screens",
    "namespaces": ["my"],
    "capabilities": [],
    "grants": [],
    "ownerAccount": owner,
    "runner": { "socket": runner },
  }))
  .unwrap()
}

// A stand-in runner on a socket: it answers which versions it keeps, a screen's file — the chunk with bytes other than
// the index names — and a call with the text it was handed; every asking line sent back on the channel.
fn runner(socket: &Path) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
  let listener = UnixListener::bind(socket).unwrap();
  let (asked, asking) = tokio::sync::mpsc::unbounded_channel();

  tokio::spawn(async move {
    while let Ok((stream, _)) = listener.accept().await {
      let asked = asked.clone();

      tokio::spawn(async move {
        let (reading, mut writing) = stream.into_split();
        let mut line = String::new();

        BufReader::new(reading).read_line(&mut line).await.unwrap();

        let first: Value = serde_json::from_str(&line).unwrap();
        let encode = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
        let answered = if first.get("versions").is_some() {
          json!({ "t": "versions", "tools": {} })
        } else if let Some(path) = first["file"]["path"].as_str() {
          match path {
            "module.js" => json!({ "t": "file", "bytes": encode(MODULE) }),
            "parts/chunk.js" => json!({ "t": "file", "bytes": encode("export const one = 2;\n") }),
            _ => json!({ "t": "refused", "why": "no such file" }),
          }
        } else {
          let said = first["call"]["arguments"]["text"].as_str().unwrap_or_default();
          let result =
            json!({ "content": [{ "type": "text", "text": format!("kept:{said}") }], "isError": false });

          json!({ "t": "end", "code": 0, "out": result.to_string() })
        };

        writing.write_all(format!("{answered}\n").as_bytes()).await.unwrap();
        let _ = asked.send(first);
      });
    }
  });

  asking
}

// "`put --owner` of a tool with a screen carries the screen's words, layer and index and none of its module, and
// refuses words that stand beside the screen's own name or name no title"
#[test]
fn an_owner_put_carries_the_screen_s_words_layer_and_index_alone() {
  let scratch = Scratch::new();

  scratch.put("a1", "notes", "100-abc");

  let placed = Registry::of(&scratch.data()).owned("a1").version_dir("notes", "100-abc");
  let mut carried: Vec<String> = walk(&placed);

  carried.sort();
  assert_eq!(
    carried,
    ["echo.json", "screen/files.json", "screen/layer.json", "screen/words.json", "tool.toml"],
    "{carried:?}"
  );

  for (written, why) in [
    (
      json!({ "ko": { "my-notes": { "title": "n" }, "other": { "title": "o" } } }),
      "beside `my-notes`",
    ),
    (json!({ "ko": { "my-notes": { "heading": "n" } } }), "names no `my-notes.title`"),
  ] {
    let folder = scratch.dir.path().join("refused").join("notes");

    built(&folder, "notes", &written);

    let (code, _, refused) = scratch.pocket(&["put", "--owner", "a1", folder.to_str().unwrap()]);

    assert_ne!(code, 0);
    assert!(refused.contains(why), "{refused}");
  }
}

fn walk(dir: &Path) -> Vec<String> {
  let mut found = Vec::new();

  for entry in fs::read_dir(dir).unwrap().flatten() {
    let path = entry.path();

    if path.is_dir() {
      let name = entry.file_name().to_string_lossy().into_owned();

      found.extend(walk(&path).into_iter().map(|below| format!("{name}/{below}")));
    } else {
      found.push(entry.file_name().to_string_lossy().into_owned());
    }
  }

  found
}

// "the app face lists the owner's screens alone, with the paths of their files under the digest of the index, and
// whether the runner answered; a file is relayed from the runner only where its bytes are the index's, under the
// digest standing, and a call reaches the screen's own tool's entries alone"
#[tokio::test]
async fn the_app_face_lists_relays_files_by_digest_and_calls_the_screen_s_own_tool() {
  let scratch = Scratch::new();

  scratch.put("a1", "notes", "100-abc");
  scratch.put("a1", "diary", "150-abc");
  scratch.put("a2", "other_notes", "200-def");

  let runner_socket = scratch.runner_socket();
  let mut asking = runner(&runner_socket);
  let (socket, records) = scratch.serve();
  let token = records.issue(session("a1", &runner_socket)).unwrap();
  let digest = sha256_hex(index().as_bytes());

  // The listing: the owner's screens, by name, and one's names, its words and its files' paths under the digest.
  let listed = request(&socket, Method::GET, "/screens", Some(&token), None).await.unwrap();

  assert_eq!(listed.status, StatusCode::OK);

  let page = listed.json().unwrap();
  let tools: Vec<&str> = page["items"]
    .as_array()
    .unwrap()
    .iter()
    .map(|item| item["tool"].as_str().unwrap())
    .collect();
  let item = &page["items"][1];

  assert_eq!(page["reached"], true, "{page}");
  assert_eq!(tools, ["diary", "notes"], "{page}");
  assert_eq!(item["tool"], "notes");
  assert_eq!(item["version"], "100-abc");
  assert_eq!(item["name"], "my-notes");
  assert_eq!(item["screenPath"], "/my/notes");
  assert_eq!(item["title"], "my-notes.title");
  assert_eq!(item["icon"], "i-material-symbols-note");
  assert_eq!(item["layer"], "0.2.0-nightly.20261123");
  assert_eq!(item["words"]["en"]["my-notes"]["title"], "Notes");
  assert_eq!(item["digest"], digest.as_str());
  assert_eq!(item["module"], format!("/screens/notes/{digest}/module.js"));
  assert_eq!(item["stylesheet"], Value::Null);
  assert!(asking.recv().await.unwrap().get("versions").is_some());

  // The module, read on the owner's machine for the version at home, kept for good.
  let module =
    request(&socket, Method::GET, &format!("/screens/notes/{digest}/module.js"), Some(&token), None)
      .await
      .unwrap();

  assert_eq!(module.status, StatusCode::OK);
  assert_eq!(module.body.as_ref(), MODULE.as_bytes());
  assert_eq!(module.headers["content-type"], "text/javascript; charset=utf-8");
  assert_eq!(module.headers["cache-control"], "private, max-age=31536000, immutable");
  assert_eq!(
    asking.recv().await.unwrap()["file"],
    json!({ "tool": "notes", "version": "100-abc", "path": "module.js" })
  );

  // Bytes other than the index names, another digest and a path the index does not name are refused.
  let chunk = request(
    &socket,
    Method::GET,
    &format!("/screens/notes/{digest}/parts/chunk.js"),
    Some(&token),
    None,
  )
  .await
  .unwrap();

  assert_eq!(chunk.status, StatusCode::BAD_GATEWAY);
  assert_eq!(chunk.json().unwrap()["code"], "runner-unreached");

  for path in [
    format!("/screens/notes/{}/module.js", "0".repeat(64)),
    format!("/screens/notes/{digest}/secret.js"),
  ] {
    let refused = request(&socket, Method::GET, &path, Some(&token), None).await.unwrap();

    assert_eq!(refused.status, StatusCode::NOT_FOUND, "{path}");
  }

  // Another owner's screen is none of this session's.
  let other = request(
    &socket,
    Method::GET,
    &format!("/screens/other_notes/{digest}/module.js"),
    Some(&token),
    None,
  )
  .await
  .unwrap();

  assert_eq!(other.status, StatusCode::NOT_FOUND);

  // A call of the screen's own entry is relayed as any call of the tool; another tool's entry is no screen's.
  let call = json!({ "name": "notes_keep", "input": { "text": "hello" } });
  let called = request(&socket, Method::POST, "/screens/notes/calls", Some(&token), Some(&call))
    .await
    .unwrap();

  assert_eq!(called.status, StatusCode::OK, "{:?}", called.json());
  assert_eq!(called.json().unwrap()["result"]["content"][0]["text"], "kept:hello");

  let foreign = json!({ "name": "diary_keep", "input": {} });
  let refused = request(&socket, Method::POST, "/screens/notes/calls", Some(&token), Some(&foreign))
    .await
    .unwrap();

  assert_eq!(refused.status, StatusCode::NOT_FOUND);

  // The issuer reaches no screen: the app face is a session's alone.
  let issuer = request(&socket, Method::GET, "/screens", Some(ISSUER), None).await.unwrap();

  assert_eq!(issuer.status, StatusCode::FORBIDDEN);

  // An owner whose runner cannot be reached reads the screens it holds, saying so, and none of their files.
  let gone = records.issue(session("a2", &scratch.sockets.path().join("gone.sock"))).unwrap();
  let page = request(&socket, Method::GET, "/screens", Some(&gone), None)
    .await
    .unwrap()
    .json()
    .unwrap();

  assert_eq!(page["reached"], false);
  assert_eq!(page["items"][0]["name"], "my-other-notes");

  let unreached = request(
    &socket,
    Method::GET,
    &format!("/screens/other_notes/{digest}/module.js"),
    Some(&gone),
    None,
  )
  .await
  .unwrap();

  assert_eq!(unreached.status, StatusCode::BAD_GATEWAY);
}
