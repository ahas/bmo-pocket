use bmo_tool::Session;
use mcp_host::residency::shows;

fn session(holder: &str, shown: &[&str]) -> Session {
  serde_json::from_value(serde_json::json!({
    "holderName": holder,
    "namespaces": ["garden", "notes"],
    "capabilities": [],
    "grants": [],
    "shown": shown,
  }))
  .unwrap()
}

#[test]
fn an_entry_is_resident_where_the_sessions_record_shows_it_whoever_holds_it() {
  let shown = ["notes/say", "garden/look"];

  assert!(shows(&session("first", &shown), "notes/say"));
  assert!(shows(&session("second", &shown), "garden/look"));
  assert!(!shows(&session("third", &["notes/say"]), "garden/look"));
}

#[test]
fn a_record_naming_no_shown_entry_lists_none_by_name() {
  let bare: Session = serde_json::from_value(serde_json::json!({
    "holderName": "first", "namespaces": ["garden"], "capabilities": ["garden.read"], "grants": [],
  }))
  .unwrap();

  assert!(bare.shown.is_empty());
  assert!(!shows(&bare, "garden/water"));
  assert!(!shows(&session("first", &[]), "garden/look"));
}
