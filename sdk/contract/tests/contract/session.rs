use bmo_tool::protocol::{Asked, Handed};
use bmo_tool::{Session, SessionRequest, Withheld};
use serde_json::json;

#[test]
fn a_record_reads_as_the_issuer_writes_it_and_holds_its_namespaces_capabilities_and_grants() {
  let record: Session = serde_json::from_value(json!({
    "holderName": "reader",
    "namespaces": ["web"],
    "capabilities": ["web.read", "web.images"],
    "grants": [{ "resource": "web.fetch" }],
    "variables": { "READER_PLACE": "/data" },
    "labels": { "closesAt": 1_700_000_000_000_u64, "branches": 3 },
    "expiresAt": 1_700_000_100_000_u64,
    "createdAt": 1_700_000_000_000_u64,
  }))
  .unwrap();

  assert!(record.holds(&["web.read", "web.images"]));
  assert!(!record.holds(&["web.read", "web.branches"]));
  assert!(record.uses("web") && !record.uses("garden"));
  assert!(record.granted("web.fetch") && !record.granted("web"));
  assert_eq!(record.variables["READER_PLACE"], "/data");
  assert_eq!(record.labels["branches"], 3);
  assert!(record.lapsed(1_700_000_100_000) && !record.lapsed(1_700_000_099_999));
  assert_eq!(
    serde_json::from_value::<Session>(serde_json::to_value(&record).unwrap()).unwrap(),
    record
  );

  // The entries it is shown by name are the issuer's, and a record with no `expiresAt` stands until revoked.
  let shown: Session = serde_json::from_value(json!({
    "holderName": "reader", "namespaces": ["garden"], "capabilities": ["garden.read"], "grants": [],
    "shown": ["garden/look"],
  }))
  .unwrap();

  assert!(shown.shown.contains("garden/look") && !shown.lapsed(u64::MAX));
  assert!(record.shown.is_empty());
}

#[test]
fn a_request_is_issued_with_the_times_the_host_answers() {
  let asked: SessionRequest = serde_json::from_value(json!({
    "holderName": "reader", "namespaces": ["web"], "capabilities": [], "grants": [], "ttlMs": 60_000,
  }))
  .unwrap();
  let issued = asked.clone().issued_at(1_000);

  assert_eq!(issued.created_at, 1_000);
  assert_eq!(issued.expires_at, Some(61_000));
  assert_eq!(
    SessionRequest {
      ttl_ms: None,
      ..asked
    }
    .issued_at(1_000)
    .expires_at,
    None
  );
}

#[test]
fn the_variables_file_is_read_afresh_and_one_that_does_not_read_is_said() {
  let place = tempfile::tempdir().unwrap();
  let file = place.path().join("variables.json");
  let record = Session {
    variables_file: Some(file.clone()),
    ..Session::default()
  };

  assert!(record.file_variables().unwrap_err().contains("variables.json"));

  std::fs::write(&file, json!({ "STEP": "2" }).to_string()).unwrap();
  assert_eq!(record.file_variables().unwrap()["STEP"], "2");

  std::fs::write(&file, json!({ "STEP": "3" }).to_string()).unwrap();
  assert_eq!(record.file_variables().unwrap()["STEP"], "3");

  std::fs::write(&file, json!({ "STEP": 4 }).to_string()).unwrap();
  assert!(record.file_variables().is_err());
  assert!(Session::default().file_variables().unwrap().is_empty());
}

#[test]
fn a_field_the_record_does_not_know_is_refused() {
  let read = serde_json::from_value::<Session>(json!({
    "holderName": "reader", "namespaces": [], "capabilities": [], "grants": [], "dataRoot": "/data",
  }));

  assert!(read.unwrap_err().to_string().contains("dataRoot"));
}

#[test]
fn the_withheld_apps_are_read_afresh_and_a_file_that_does_not_read_is_said() {
  let place = tempfile::tempdir().unwrap();
  let file = place.path().join("withheld.json");
  let record = Session {
    withheld_apps_file: Some(file.clone()),
    ..Session::default()
  };

  assert!(record.withheld_apps().unwrap_err().contains("withheld.json"));

  std::fs::write(&file, json!({ "apps": ["garden-plan"] }).to_string()).unwrap();
  assert!(record.withheld_apps().unwrap().holds("garden-plan"));
  assert!(!record.withheld_apps().unwrap().holds("almanac"));

  std::fs::write(&file, json!({ "apps": [] }).to_string()).unwrap();
  assert!(!record.withheld_apps().unwrap().holds("garden-plan"));

  std::fs::write(&file, json!(["garden-plan"]).to_string()).unwrap();
  assert!(record.withheld_apps().is_err());
  assert!(Withheld::Every.holds("anything"));
  assert_eq!(Session::default().withheld_apps().unwrap(), Withheld::default());

  let handed = Handed::new(Asked::Start, record);

  assert!(handed.session.withheld_apps_file.is_none(), "a tool is never handed the file");
}
