//! The tool's own tests, its manifest's check: each handed object answered as the executable answers it.

use count_words_rust::{ENTRY, answer};
use serde_json::{Value, json};

// A handed object as the host writes it, under a record holding the entry's capability.
fn handed(entry: &str, arguments: Value) -> Value {
  json!({
    "protocol": 1,
    "kind": "call",
    "entry": entry,
    "arguments": arguments,
    "session": {
      "holderName": "tester",
      "namespaces": ["example"],
      "capabilities": ["example.read"],
      "grants": [],
    },
  })
}

#[test]
fn a_call_is_answered_with_the_count_as_text_and_as_an_object() {
  let answer = answer(&handed(ENTRY, json!({ "text": "one two  three\n" }))).unwrap();
  let counted = json!({ "words": 3, "characters": 15 });

  assert_eq!(answer["isError"], false);
  assert_eq!(answer["structuredContent"], counted);
  assert_eq!(
    serde_json::from_str::<Value>(answer["content"][0]["text"].as_str().unwrap()).unwrap(),
    counted
  );
}

#[test]
fn characters_are_code_points() {
  let answer = answer(&handed(ENTRY, json!({ "text": "caf\u{e9} au lait" }))).unwrap();

  assert_eq!(answer["structuredContent"], json!({ "words": 3, "characters": 12 }));
}

#[test]
fn an_entry_it_does_not_hold_is_answered_as_a_failure() {
  let answer = answer(&handed("example/other", json!({}))).unwrap();

  assert_eq!(answer["isError"], true);
  assert!(answer["content"][0]["text"].as_str().unwrap().starts_with("ERROR: "));
}

#[test]
fn an_object_of_another_protocol_does_not_read() {
  let mut asked = handed(ENTRY, json!({ "text": "one" }));

  asked["protocol"] = json!(2);
  assert!(answer(&asked).unwrap_err().contains("protocol 2"));
}
