//! A stand-in tool placed under a tools root, the host serving it in this process, and its control socket served on a
//! socket of the test's own, and its console face on a port of the test's own, for the control socket's, the
//! console face's and the schedules' tests.

use std::path::PathBuf;
use std::sync::Arc;

use bmo_tool::manifest::{MANIFEST, PLACED};
use bmo_tool::{Granted, Session};
use mcp_host::console::Bundle;
use mcp_host::handler::Host;
use mcp_host::placed::Placed;
use mcp_host::records::Records;
use mcp_host::schedule::Schedules;
use mcp_host::server::{self, Screen};
use serde_json::Value;
use tempfile::TempDir;
use tokio::net::UnixListener;

// The stand-in's executable: it keeps what each call was handed beside itself, then breaks for `broken`, answers a
// failed result for the text `fail`, and says the text back otherwise.
const SCRIPT: &str = r#"
import json, os, sys

handed = json.load(sys.stdin)
kept = {
    "entry": handed["entry"],
    "arguments": handed["arguments"],
    "holderName": handed["session"]["holderName"],
    "capabilities": handed["session"]["capabilities"],
    "grants": handed["session"]["grants"],
    "scheduledAt": os.environ.get("BMO_SCHEDULED_AT"),
}

with open("handed.jsonl", "a") as kept_file:
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

/// The stand-in's manifest: a read-only entry with an icon, a listing line and a schedule; an acting one under a
/// grant; and one for the system alone that breaks.
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
grant = "notes.keep"

[[entry]]
namespace = "notes"
name = "broken"
description = "Breaks."
input = "note.json"
system = true
hints = { read_only = true }
"#;

pub struct Fixture {
  dir: TempDir,
  // Sockets are bound under a short path of their own, which stands as long as the fixture.
  sockets: TempDir,
  pub host: Arc<Host>,
}

impl Fixture {
  /// The stand-in placed with this manifest, and a host that read it once.
  pub fn with(manifest: &str) -> Self {
    let dir = tempfile::tempdir().unwrap();
    let sockets = tempfile::Builder::new().prefix("ct").tempdir_in("/tmp").unwrap();
    let placed = dir.path().join("tools/almanac").join(PLACED);

    std::fs::create_dir_all(&placed).unwrap();
    std::fs::write(placed.join(MANIFEST), manifest).unwrap();
    std::fs::write(placed.join("note.json"), SCHEMA).unwrap();
    std::fs::write(placed.join("almanac.py"), SCRIPT).unwrap();

    let placed = Placed::new(Some(dir.path().join("tools")), sockets.path().to_owned());

    placed.reread();

    let host = Arc::new(Host {
      placed: Arc::new(placed),
      pass_env: Vec::new(),
    });

    Self { dir, sockets, host }
  }

  pub fn new() -> Self {
    Self::with(MANIFEST_TEXT)
  }

  /// A record of the namespace `notes` holding these capabilities and grants.
  pub fn session(&self, capabilities: &[&str], grants: &[&str]) -> Session {
    Session {
      holder_name: "tester".to_owned(),
      namespaces: ["notes".to_owned()].into(),
      capabilities: capabilities.iter().map(|one| (*one).to_owned()).collect(),
      grants: grants
        .iter()
        .map(|one| Granted {
          resource: (*one).to_owned(),
        })
        .collect(),
      ..Session::default()
    }
  }

  /// What each call of the stand-in was handed, in order.
  pub fn handed(&self) -> Vec<Value> {
    std::fs::read_to_string(self.dir.path().join("tools/almanac").join(PLACED).join("handed.jsonl"))
      .unwrap_or_default()
      .lines()
      .map(|line| serde_json::from_str(line).unwrap())
      .collect()
  }

  /// The control socket served in this process on a socket of its own, behind `ISSUER`; it ends with the test's
  /// runtime.
  pub fn serve(&self, schedules: Arc<Schedules>) -> PathBuf {
    let records = Arc::new(Records::load(&self.dir.path().join("sessions")).unwrap());

    self.serve_control(records, schedules)
  }

  fn serve_control(&self, records: Arc<Records>, schedules: Arc<Schedules>) -> PathBuf {
    let socket = self.sockets.path().join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let router = server::control(records, self.host.clone(), schedules, ISSUER.to_owned());

    tokio::spawn(async move { axum::serve(listener, router).await });
    socket
  }

  /// The control socket, as `serve` serves it, and the console face over the same records on a loopback port of
  /// its own, behind `CONSOLE` and with this bundle; both end with the test's runtime.
  pub fn serve_with_console(&self, bundle: Option<Bundle>) -> (PathBuf, u16) {
    let records = Arc::new(Records::load(&self.dir.path().join("sessions")).unwrap());
    let schedules = Arc::new(Schedules::default());
    let socket = self.serve_control(records.clone(), schedules.clone());
    let bound = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = bound.local_addr().unwrap().port();

    bound.set_nonblocking(true).unwrap();

    let listener = tokio::net::TcpListener::from_std(bound).unwrap();
    let screen = Screen {
      token: CONSOLE.to_owned(),
      bundle,
    };
    let router = server::console(records, self.host.clone(), schedules, screen);

    tokio::spawn(async move { axum::serve(listener, router).await });
    (socket, port)
  }
}

/// The issuer token the fixture's control socket is served behind.
pub const ISSUER: &str = "issuer-token-of-the-fixture";

/// The console token the fixture's console face is served behind.
pub const CONSOLE: &str = "console-token-of-the-fixture";
