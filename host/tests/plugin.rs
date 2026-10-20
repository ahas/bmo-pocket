//! `bmo-pocket mcp` as a harness starts it (docs/mcp-host.md, 「Tests」): the real host serving a placed tool on a
//! scratch data root, a tool session issued with its issuer token, and the plugin's MCP server spoken to on its
//! standard input and output. Every process each binary starts goes down with the test.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use bmo_pocket_client::{Pocket, SessionRequest};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// How long the test lets a server take to answer one request before it fails; a guard, never a wait.
const PATIENCE: Duration = Duration::from_secs(20);

// A process started in a group of its own, the whole group ended when it is dropped.
struct Group(Child);

impl Drop for Group {
  fn drop(&mut self) {
    if let Ok(group) = i32::try_from(self.0.id()) {
      // SAFETY: `killpg` only sends a signal; a group already gone is answered with an error, ignored here.
      unsafe {
        libc::killpg(group, libc::SIGKILL);
      }
    }

    let _ = self.0.wait();
  }
}

const MANIFEST: &str = r#"
[tool]
name = "garden"
protocol = 1
run = ["python3", "garden.py"]
timeout = 20

[[entry]]
namespace = "garden"
name = "water"
description = "Waters a bed."
input = "any.json"
capabilities = ["garden.use"]
hints = { read_only = true }

[[entry]]
namespace = "garden"
name = "prune"
description = "Prunes a bed."
input = "any.json"
capabilities = ["garden.use"]
"#;

const SCRIPT: &str = r#"
import json, sys

handed = json.load(sys.stdin)
said = handed["entry"] + ":" + handed["arguments"].get("bed", "")
print(json.dumps({"content": [{"type": "text", "text": said}], "isError": False}))
"#;

fn place(tools: &Path) {
  let placed = tools.join("garden/.placed");

  fs::create_dir_all(&placed).unwrap();
  fs::write(placed.join("tool.toml"), MANIFEST).unwrap();
  fs::write(
    placed.join("any.json"),
    r#"{ "type": "object", "properties": { "bed": { "type": "string" } } }"#,
  )
  .unwrap();
  fs::write(placed.join("garden.py"), SCRIPT).unwrap();
}

#[tokio::test]
async fn the_plugin_started_with_a_session_token_lists_and_calls_the_hosts_tools() {
  let data = tempfile::tempdir().unwrap();
  let sockets = tempfile::Builder::new().prefix("pl").tempdir_in("/tmp").unwrap();
  let control = sockets.path().join("control.sock");

  place(&data.path().join("tools"));

  let mut host = Command::new(env!("CARGO_BIN_EXE_mcp-host"));

  host
    .args(["serve", "--port", "0", "--settle-ms", "50"])
    .arg("--data-root")
    .arg(data.path())
    .arg("--control")
    .arg(&control)
    .arg("--tools")
    .arg(data.path().join("tools"))
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .process_group(0);

  let mut host = Group(host.spawn().unwrap());
  let mut started = String::new();

  std::io::BufRead::read_line(&mut std::io::BufReader::new(host.0.stdout.take().unwrap()), &mut started)
    .unwrap();
  assert!(started.contains("listening"), "{started}");

  let issuer = fs::read_to_string(data.path().join("tool-host/issuer-token"))
    .unwrap()
    .trim()
    .to_owned();
  let issued = Pocket::at(&control, issuer)
    .issue(&SessionRequest {
      holder_name: "tester".to_owned(),
      namespaces: ["garden".to_owned()].into(),
      capabilities: ["garden.use".to_owned()].into(),
      ..SessionRequest::default()
    })
    .await
    .unwrap();

  let mut plugin = tokio::process::Command::new(env!("CARGO_BIN_EXE_bmo-pocket"));

  plugin
    .arg("mcp")
    .arg("--socket")
    .arg(&control)
    .env("BMO_POCKET_TOKEN", &issued.token)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .process_group(0)
    .kill_on_drop(true);

  let mut plugin = plugin.spawn().unwrap();
  let mut input = plugin.stdin.take().unwrap();
  let mut output = BufReader::new(plugin.stdout.take().unwrap()).lines();
  let mut ask = async |id: u64, method: &str, params: Value| -> Value {
    let asked = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });

    input.write_all(format!("{asked}\n").as_bytes()).await.unwrap();
    input.flush().await.unwrap();

    loop {
      let line = tokio::time::timeout(PATIENCE, output.next_line()).await.unwrap().unwrap().unwrap();
      let answer: Value = serde_json::from_str(&line).unwrap();

      if answer["id"] == id {
        return answer["result"].clone();
      }
    }
  };

  let info = ask(
    1,
    "initialize",
    json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }),
  )
  .await;

  assert_eq!(info["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));

  let listed = ask(2, "tools/list", json!({})).await;
  let names: Vec<&str> = listed["tools"]
    .as_array()
    .unwrap()
    .iter()
    .map(|tool| tool["name"].as_str().unwrap())
    .collect();

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

  let called =
    ask(3, "tools/call", json!({ "name": "garden_water", "arguments": { "bed": "east" } })).await;

  assert_eq!(called["content"][0]["text"], "garden/water:east", "{called}");

  // The harness closing its end ends the plugin; the host's group goes with the test.
  drop(input);
  assert!(tokio::time::timeout(PATIENCE, plugin.wait()).await.unwrap().unwrap().success());
  drop(host);
}
