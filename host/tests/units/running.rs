use std::time::Duration;

use mcp_host::running::{Bound, claimed};
use serde_json::json;

#[test]
fn a_claims_answer_carries_its_arguments_or_none() {
  assert_eq!(claimed(&json!({ "arguments": { "id": 1 } })), Some(json!({ "id": 1 })));
  assert_eq!(claimed(&json!({ "arguments": null })), None);
}

#[test]
fn a_call_is_bounded_by_its_entry_else_its_tool_and_not_at_all_where_unbounded() {
  let entry = |part: &str| {
    let text = format!(
      "[tool]\nname = \"launch\"\nprotocol = 1\nrun = [\"launch\"]\ntimeout = 180\n\n[[entry]]\nnamespace = \"room\"\nname = \"search\"\ndescription = \"A search.\"\ninput = \"in.json\"\ncapabilities = [\"room.tools\"]\n{part}"
    );

    bmo_tool::manifest::Manifest::parse(&text, "launch").unwrap().entries.remove(0)
  };

  assert_eq!(Bound::of(Some(&entry("")), 180).0, Some(Duration::from_secs(180)));
  assert_eq!(Bound::of(Some(&entry("timeout = 5\n")), 180).0, Some(Duration::from_secs(5)));
  assert_eq!(Bound::of(Some(&entry("unbounded = true\n")), 180).0, None);
  assert_eq!(Bound::of(None, 180).0, Some(Duration::from_secs(180)));
}
