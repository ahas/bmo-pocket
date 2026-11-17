use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use mcp_host::held_sessions::{HELD, keep};
use mcp_host::records::Records;
use serde_json::json;

fn declare(dir: &Path, name: &str, record: serde_json::Value) {
  std::fs::write(dir.join(format!("{name}.json")), record.to_string()).unwrap();
}

fn token(dir: &Path, name: &str) -> Option<String> {
  std::fs::read_to_string(dir.join(format!("{name}.token")))
    .ok()
    .map(|token| token.trim().to_owned())
}

fn record(entries: &[&str]) -> serde_json::Value {
  json!({
    "holderName": "console",
    "namespaces": ["notes"],
    "capabilities": [],
    "grants": [],
    "entries": entries,
    "variables": { "BMO_DATA_DIR": "/data" },
  })
}

// A declared session is issued with its token written beside it, readable by the host's user alone, and kept as it
// stands across beats and a restart of the host; a declaration changed is issued afresh and the session before it
// revoked; one taken away takes its session and its token file with it.
#[test]
fn a_declared_session_is_kept_issued_while_its_declaration_stands() {
  let place = tempfile::tempdir().unwrap();
  let sessions = place.path().join("sessions");
  let held = place.path().join(HELD);

  std::fs::create_dir_all(&held).unwrap();
  declare(&held, "console", record(&["notes/read_note"]));

  let records = Records::load(&sessions).unwrap();

  assert!(keep(&held, &records).is_empty());

  let first = token(&held, "console").unwrap();
  let (_, session) = records.get(&first).unwrap();

  assert_eq!(session.holder_name, "console");
  assert_eq!(session.expires_at, None);
  assert_eq!(session.entries, Some(["notes/read_note".to_owned()].into()));
  assert_eq!(session.variables["BMO_DATA_DIR"], "/data");
  assert_eq!(
    std::fs::metadata(held.join("console.token")).unwrap().permissions().mode() & 0o777,
    0o600
  );

  let restarted = Records::load(&sessions).unwrap();

  assert!(keep(&held, &restarted).is_empty());
  assert_eq!(token(&held, "console").unwrap(), first);

  declare(&held, "console", record(&["notes/read_note", "notes/broken"]));

  assert_eq!(keep(&held, &restarted).len(), 1);

  let second = token(&held, "console").unwrap();

  assert_ne!(second, first);
  assert!(restarted.get(&first).is_none());
  assert_eq!(restarted.get(&second).unwrap().1.entries.as_ref().unwrap().len(), 2);

  std::fs::remove_file(held.join("console.json")).unwrap();

  assert_eq!(keep(&held, &restarted).len(), 1);
  assert!(restarted.get(&second).is_none());
  assert!(token(&held, "console").is_none());
}

// A session revoked by hand is issued again at the next beat, under a token written in place of the old one.
#[test]
fn a_held_session_revoked_by_hand_is_issued_again() {
  let place = tempfile::tempdir().unwrap();
  let held = place.path().join(HELD);

  std::fs::create_dir_all(&held).unwrap();
  declare(&held, "console", record(&["notes/read_note"]));

  let records = Records::load(&place.path().join("sessions")).unwrap();

  keep(&held, &records);

  let first = token(&held, "console").unwrap();

  records.revoke(&first).unwrap();
  keep(&held, &records);

  let second = token(&held, "console").unwrap();

  assert_ne!(second, first);
  assert!(records.get(&second).is_some());
}

// A declaration naming `ttlMs`, or one that is no record, issues nothing, and a folder that does not stand declares
// nothing.
#[test]
fn a_declaration_that_does_not_read_or_lapses_issues_nothing() {
  let place = tempfile::tempdir().unwrap();
  let held = place.path().join(HELD);
  let records = Records::load(&place.path().join("sessions")).unwrap();

  assert!(keep(&held, &records).is_empty());

  std::fs::create_dir_all(&held).unwrap();

  let mut lapsing = record(&[]);

  lapsing["ttlMs"] = json!(60_000);
  declare(&held, "lapsing", lapsing);
  std::fs::write(held.join("broken.json"), "{").unwrap();
  keep(&held, &records);

  assert!(token(&held, "lapsing").is_none() && token(&held, "broken").is_none());
  assert!(records.standing().is_empty());
}
