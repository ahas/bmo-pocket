use mcp_host::toolbook::{first_sentence, tools};
use serde_json::json;

#[test]
fn a_line_of_the_list_carries_the_first_sentence_and_nothing_after_it() {
  assert_eq!(first_sentence("Reads a price. Then more."), "Reads a price.");
  assert_eq!(first_sentence("  One sentence alone "), "One sentence alone");
  assert_eq!(first_sentence("Reads v1.2 of it. More."), "Reads v1.2 of it.");
}

#[test]
fn the_four_tools_say_how_each_is_reached_and_only_call_and_list_and_describe_only_read() {
  let tools = tools();
  let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();

  assert_eq!(names, ["list", "describe", "call", "act"]);
  assert_eq!(tools[2]["annotations"]["readOnlyHint"], true);
  assert_eq!(tools[3]["annotations"]["readOnlyHint"], false);
  assert_eq!(tools[2]["inputSchema"]["required"], json!(["tool", "arguments"]));
}
