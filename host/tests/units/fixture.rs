//! A stand-in tool placed under a tools root, the host serving it in this process, and its control socket served
//! on a socket of the test's own, for the control socket's and the schedules' tests.

use std::path::PathBuf;
use std::sync::Arc;

use bmo_tool::Session;
use bmo_tool::manifest::{MANIFEST, PLACED};
use mcp_host::handler::Host;
use mcp_host::lock::Lock;
use mcp_host::placed::Placed;
use mcp_host::records::Records;
use mcp_host::schedule::Schedules;
use mcp_host::server;
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::UnixListener;

// The stand-in's executable: it keeps what each call was handed in the data root, then breaks for `broken`, answers
// a failed result for the text `fail`, and says the text back otherwise.
const SCRIPT: &str = r#"
import json, os, sys

handed = json.load(sys.stdin)
kept = {
    "entry": handed["entry"],
    "arguments": handed["arguments"],
    "line": handed["session"]["line"],
    "capabilities": handed["session"]["capabilities"],
    "scheduledAt": os.environ.get("BMO_SCHEDULED_AT"),
}

with open(os.path.join(os.environ["BMO_DATA_DIR"], "handed.jsonl"), "a") as kept_file:
    kept_file.write(json.dumps(kept) + "\n")

if handed["entry"] == "notes/broken":
    print("it broke", file=sys.stderr)
    sys.exit(3)

text = handed["arguments"].get("text", "")

if text == "fail":
    print(json.dumps({"content": [{"type": "text", "text": "it failed"}], "isError": True}))
else:
    print(json.dumps({"content": [{"type": "text", "text": "noted:" + text}], "isError": False}))
"#;

const SCHEMA: &str = r#"{ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"], "additionalProperties": false }"#;

/// The stand-in's manifest: a read-only entry with an icon, a listing line and a schedule; an acting one; and one for
/// the system alone that breaks.
pub const MANIFEST_TEXT: &str = r#"
[tool]
name = "almanac"
protocol = 1
run = ["python3", "almanac.py"]
timeout = 20

[[entry]]
namespace = "notes"
name = "read_note"
description = "Says the note back."
input = "note.json"
capabilities = ["notes.read"]
hints = { read_only = true }

[entry.facts]
icon = "i-lucide-notebook"

[entry.listing]
line = "read_note: says a note back"

[[entry.schedule]]
cron = "0 7 * * *"
arguments = { text = "morning" }

[[entry]]
namespace = "notes"
name = "write_note"
description = "Keeps a note."
input = "note.json"
capabilities = ["notes.write"]

[[entry]]
namespace = "notes"
name = "broken"
description = "Breaks."
input = "note.json"
system = true
hints = { read_only = true }
"#;

pub struct Fixture {
  pub dir: TempDir,
  // Sockets are bound under a short path of their own.
  pub sockets: TempDir,
  pub host: Arc<Host>,
}

impl Fixture {
  /// The stand-in placed with this manifest, and a host that read it once.
  pub fn with(manifest: &str) -> Self {
    let dir = tempfile::tempdir().unwrap();
    let sockets = tempfile::Builder::new().prefix("ct").tempdir_in("/tmp").unwrap();
    let placed = dir.path().join("tools/almanac").join(PLACED);

    std::fs::create_dir_all(&placed).unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(placed.join(MANIFEST), manifest).unwrap();
    std::fs::write(placed.join("note.json"), SCHEMA).unwrap();
    std::fs::write(placed.join("almanac.py"), SCRIPT).unwrap();

    let placed = Placed::new(Some(dir.path().join("tools")), sockets.path().to_owned());

    placed.reread();

    let host = Arc::new(Host {
      lock: Lock::default(),
      placed: Arc::new(placed),
      pass_env: Vec::new(),
    });

    Self { dir, sockets, host }
  }

  pub fn new() -> Self {
    Self::with(MANIFEST_TEXT)
  }

  pub fn data(&self) -> PathBuf {
    self.dir.path().join("data")
  }

  /// A record holding these capabilities.
  pub fn session(&self, capabilities: &[&str]) -> Session {
    serde_json::from_value(json!({ "line": "worker", "capabilities": capabilities, "dataRoot": self.data() }))
      .unwrap()
  }

  /// What each call of the stand-in was handed, in order.
  pub fn handed(&self) -> Vec<Value> {
    std::fs::read_to_string(self.data().join("handed.jsonl"))
      .unwrap_or_default()
      .lines()
      .map(|line| serde_json::from_str(line).unwrap())
      .collect()
  }

  /// The control socket served in this process, on a socket of its own; it ends with the test's runtime.
  pub fn serve(&self, schedules: Arc<Schedules>) -> PathBuf {
    let socket = self.sockets.path().join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let records = Arc::new(Records::load(&self.data().join("tool-host/sessions")).unwrap());
    let router = server::control(records, self.host.clone(), schedules);

    tokio::spawn(async move { axum::serve(listener, router).await });
    socket
  }
}
