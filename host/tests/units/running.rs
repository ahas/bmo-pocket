use mcp_host::running::claimed;
use serde_json::json;

#[test]
fn a_claims_answer_carries_its_arguments_or_none() {
  assert_eq!(claimed(&json!({ "arguments": { "id": 1 } })), Some(json!({ "id": 1 })));
  assert_eq!(claimed(&json!({ "arguments": null })), None);
}
