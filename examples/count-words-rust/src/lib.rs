//! The example tool in Rust (the tool host's docs/authoring.md): its one entry, `example/count_words_rust`, answers
//! how many words and characters a text holds. The host hands the call as one JSON object on standard input and
//! reads one tool result on standard output (docs/api.md, 「The call protocol」); this library answers the object
//! and the executable carries it in and out.

use std::fmt::Display;

use serde_json::{Value, json};

/// The full name of the one entry this executable answers.
pub const ENTRY: &str = "example/count_words_rust";

/// The call protocol this executable speaks.
const PROTOCOL: u64 = 1;

/// The answer to one handed object: a call of the entry answered with its count, any other call or kind with a
/// failure the caller reads. An object that is no call of this protocol does not read, and the executable says why
/// on standard error and ends unclean, which the host takes as a failed call.
pub fn answer(handed: &Value) -> Result<Value, String> {
  if handed["protocol"].as_u64() != Some(PROTOCOL) {
    return Err(format!("protocol {} is not {PROTOCOL}", handed["protocol"]));
  }

  if !handed["session"].is_object() {
    return Err("it holds no session".to_owned());
  }

  if handed["kind"] != "call" {
    return Ok(failure("this executable answers calls alone"));
  }

  if handed["entry"] != ENTRY {
    return Ok(failure(format!("this executable holds no entry {}", handed["entry"])));
  }

  // The host checked the arguments against the input schema before the call, so the text stands and is a string.
  let text = handed["arguments"]["text"].as_str().unwrap_or_default();

  Ok(structured(json!({
    "words": text.split_whitespace().count(),
    "characters": text.chars().count(),
  })))
}

/// An object as an entry with an output schema answers it: the object, and the same object as its text.
fn structured(object: Value) -> Value {
  json!({
    "content": [{ "type": "text", "text": object.to_string() }],
    "isError": false,
    "structuredContent": object,
  })
}

/// A call the tool could not answer: its reason after `ERROR: `, marked as an error the caller reads.
fn failure(reason: impl Display) -> Value {
  json!({
    "content": [{ "type": "text", "text": format!("ERROR: {reason}") }],
    "isError": true,
  })
}
