//! The control socket's OpenAPI document (docs/api.md, 「The OpenAPI document and the clients」): the committed file
//! is what the client's types generate, and the document holds together.

use std::path::Path;

use bmo_pocket_client::{openapi, openapi_json};
use serde_json::Value;

fn committed() -> String {
  let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/openapi.json");

  std::fs::read_to_string(&file).unwrap_or_else(|why| panic!("{}: {why}", file.display()))
}

#[test]
fn the_committed_document_is_what_the_types_generate() {
  assert!(
    committed() == openapi_json(),
    "docs/openapi.json differs from what the client generates: \
     cargo run -p bmo-pocket-client --example openapi > docs/openapi.json"
  );
}

// Every `$ref` in a value, by its pointer.
fn refs(value: &Value, found: &mut Vec<String>) {
  match value {
    Value::Object(fields) => {
      if let Some(Value::String(pointer)) = fields.get("$ref") {
        found.push(pointer.clone());
      }

      fields.values().for_each(|inner| refs(inner, found));
    }
    Value::Array(items) => items.iter().for_each(|inner| refs(inner, found)),
    _ => {}
  }
}

#[test]
fn every_route_takes_the_bearer_token_and_every_reference_resolves() {
  let document = openapi();
  let paths = document["paths"].as_object().unwrap();
  let routes: Vec<String> = paths
    .iter()
    .flat_map(|(path, item)| item.as_object().unwrap().keys().map(move |method| format!("{method} {path}")))
    .collect();

  assert_eq!(routes.len(), 17, "{routes:?}");
  assert_eq!(document["security"], serde_json::json!([{ "bearer": [] }]));
  assert_eq!(document["info"]["version"], env!("CARGO_PKG_VERSION"));

  for item in paths.values() {
    for operation in item.as_object().unwrap().values() {
      assert!(operation["operationId"].is_string(), "{operation}");
      assert!(operation["responses"]["401"].is_object(), "{operation}");
    }
  }

  let mut found = Vec::new();

  refs(&document, &mut found);
  assert!(!found.is_empty());

  for pointer in found {
    assert!(
      document.pointer(pointer.trim_start_matches('#')).is_some(),
      "{pointer} does not resolve"
    );
  }
}

#[test]
fn the_harness_tools_each_name_a_route_the_document_states() {
  let document = openapi();
  let tools = document["x-tools"].as_array().unwrap();
  let names: Vec<&str> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();

  assert_eq!(names, ["pocket_namespaces", "pocket_tools", "pocket_tool", "pocket_call"]);
  assert_eq!(document["x-harness"]["plugin"], "bmo-pocket");
  assert_eq!(document["x-harness"]["mcp"], "bmo-pocket mcp");

  for tool in tools {
    let (method, path) = tool["route"].as_str().unwrap().split_once(' ').unwrap();

    assert!(
      document["paths"][path][method.to_lowercase()].is_object(),
      "{} names {method} {path}, which the document does not state",
      tool["name"]
    );
  }
}
