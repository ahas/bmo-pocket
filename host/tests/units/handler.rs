use std::sync::Arc;

use bmo_tool::Session;
use mcp_host::handler::{Host, as_result, misfits};
use mcp_host::lock::Lock;
use mcp_host::placed::{PLACED, Placed};
use serde_json::json;

#[test]
fn arguments_are_checked_against_the_schema_and_a_null_one_reads_as_left_out() {
  let schema = json!({
    "type": "object",
    "properties": { "query": { "type": "string" }, "count": { "type": "integer" } },
    "required": ["query"],
    "additionalProperties": false,
  });

  assert!(misfits(&schema, &json!({ "query": "x", "count": null })).is_empty());
  assert_eq!(misfits(&schema, &json!({ "query": 3 })).len(), 1);
  assert!(misfits(&schema, &json!({ "query": 3 }))[0].starts_with("/query: "));
  assert_eq!(misfits(&schema, &json!({ "query": null })).len(), 1);
  assert_eq!(misfits(&schema, &json!({ "query": "x", "other": 1 })).len(), 1);
}

#[test]
fn a_placed_answer_that_is_not_a_tool_result_is_a_failed_call() {
  let fine = json!({ "content": [{ "type": "text", "text": "x" }], "isError": false });

  assert_eq!(as_result(fine.clone()), fine);
  assert_eq!(as_result(json!({ "content": 3 }))["isError"], true);
}

// A tool added as one folder, under a namespace and a capability no code names: served to a session holding
// its capability as soon as the folder stands, booked in its toolbook, and listed by name once the session's
// record names it among the entries it is shown.
#[test]
fn a_tool_is_added_as_one_folder_and_listed_by_name_where_a_record_shows_it() {
  let tools = tempfile::tempdir().unwrap();
  let folder = tools.path().join("echo").join(PLACED);

  std::fs::create_dir_all(&folder).unwrap();
  std::fs::write(
    folder.join(bmo_tool::manifest::MANIFEST),
    "[tool]\nname = \"echo\"\nprotocol = 1\nrun = [\"echo\"]\ntimeout = 5\n\n[[entry]]\nnamespace = \"garden\"\nname = \"echo_back\"\ndescription = \"It says it back.\"\ninput = \"in.json\"\ncapabilities = [\"garden.read\"]\n",
  )
  .unwrap();
  std::fs::write(folder.join("in.json"), r#"{ "type": "object" }"#).unwrap();

  let placed = Placed::new(Some(tools.path().to_owned()), tools.path().join("sockets"));

  placed.reread();

  let host = Host {
    lock: Lock::default(),
    placed: Arc::new(placed),
    pass_env: Vec::new(),
  };
  let session = |capabilities: &[&str], shown: &[&str]| -> Session {
    serde_json::from_value(json!({
      "line": "worker",
      "role": "worker",
      "capabilities": capabilities,
      "shown": shown,
      "dataRoot": "/nowhere",
    }))
    .unwrap()
  };
  let held = |session: &Session| -> Vec<(String, bool)> {
    host.held(session).iter().map(|entry| (entry.full_name(), entry.resident())).collect()
  };

  assert!(host.namespaces().contains("garden"));
  assert!(host.held(&session(&["graph.read"], &[])).is_empty());
  assert_eq!(held(&session(&["garden.read"], &[])), [("garden/echo_back".to_owned(), false)]);
  assert_eq!(
    held(&session(&["garden.read"], &["garden/echo_back"])),
    [("garden/echo_back".to_owned(), true)]
  );
}
