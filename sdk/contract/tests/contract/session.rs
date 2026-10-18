use bmo_tool::Session;
use serde_json::json;

#[test]
fn a_record_reads_as_the_launcher_writes_it() {
  let record: Session = serde_json::from_value(json!({
    "line": "search",
    "capabilities": ["web.read", "web.images"],
    "dataRoot": "/data",
    "archive": "/data/bmo.db",
    "runVariables": { "BMO_RUN_NAME": "search-7" },
    "research": { "closesAt": 1_700_000_000_000_u64, "branches": 3, "rulebooks": ["goods"] },
    "deadline": 1_700_000_100_000_u64,
  }))
  .unwrap();

  assert!(record.holds(&["web.read", "web.images"]));
  assert!(!record.holds(&["web.read", "web.branches"]));
  assert_eq!(record.run_variables["BMO_RUN_NAME"], "search-7");
  assert_eq!(record.research.as_ref().unwrap().branches, Some(3));
  assert_eq!(
    serde_json::from_value::<Session>(serde_json::to_value(&record).unwrap()).unwrap(),
    record
  );

  // The entries it is shown by name are the issuer's, and a record names no archive where it has none.
  let shown: Session = serde_json::from_value(json!({
    "line": "worker", "capabilities": ["graph.read"], "dataRoot": "/data", "shown": ["worker/graph_get"],
  }))
  .unwrap();

  assert!(shown.shown.contains("worker/graph_get") && shown.archive.is_none());
  assert!(record.shown.is_empty() && record.archive.is_some());
}

#[test]
fn a_capability_is_held_by_its_name_and_the_turn_is_the_place_s_run_over_the_record_s_own() {
  let place = tempfile::tempdir().unwrap();

  std::fs::write(
    place.path().join("run.json"),
    json!({ "env": { "BMO_TURN_CHAT": "-5", "HOME": "/elsewhere" } }).to_string(),
  )
  .unwrap();

  let record: Session = serde_json::from_value(json!({
    "line": "chat", "role": "chat", "capabilities": ["room.tools"], "dataRoot": "/data", "archive": "/data/bmo.db",
    "place": place.path(), "runVariables": { "BMO_TURN_CHAT": "1", "BMO_RUN_NAME": "r" },
  }))
  .unwrap();
  let turn = record.turn_environment();

  assert!(record.holds(&["room.tools"]) && !record.holds(&["room.tools", "web.read"]));
  assert_eq!(record.role.as_deref(), Some("chat"));
  assert_eq!(turn["BMO_TURN_CHAT"], "-5");
  assert_eq!(turn["BMO_RUN_NAME"], "r");
  assert!(!turn.contains_key("HOME"));
}

#[test]
fn a_field_the_record_does_not_know_is_refused() {
  let read = serde_json::from_value::<Session>(json!({
    "line": "worker", "capabilities": [], "dataRoot": "/data", "archive": "/data/bmo.db", "room": 5,
  }));

  assert!(read.unwrap_err().to_string().contains("room"));
}
