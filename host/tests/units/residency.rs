use bmo_tool::Session;
use mcp_host::residency::shows;

fn session(line: &str, role: Option<&str>, shown: &[&str]) -> Session {
  serde_json::from_value(serde_json::json!({
    "line": line,
    "role": role,
    "capabilities": [],
    "shown": shown,
    "checkout": "/nowhere",
    "dataRoot": "/nowhere",
  }))
  .unwrap()
}

#[test]
fn an_entry_is_resident_where_the_sessions_record_shows_it_whatever_its_line_or_role() {
  let shown = ["bmo/say", "worker/graph_get"];

  assert!(shows(&session("chat", Some("chat"), &shown), "bmo/say"));
  assert!(shows(&session("search", None, &shown), "worker/graph_get"));
  assert!(!shows(&session("worker", Some("worker"), &["bmo/say"]), "worker/graph_get"));
}

#[test]
fn a_record_naming_no_shown_entry_lists_none_by_name() {
  let bare: Session = serde_json::from_value(serde_json::json!({
    "line": "worker", "role": "worker", "capabilities": ["graph.read"], "dataRoot": "/nowhere",
  }))
  .unwrap();

  assert!(bare.shown.is_empty());
  assert!(!shows(&bare, "worker/graph_impact"));
  assert!(!shows(&session("worker", Some("worker"), &[]), "worker/graph_get"));
}
