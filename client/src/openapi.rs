//! The OpenAPI document of the control socket (docs/api.md, 「The OpenAPI document and the clients」): every route,
//! its query, its request body, its answer and the error answers it can give, the body schemas taken from the
//! types this crate sends and reads, which are the types the host answers with.

use std::collections::BTreeMap;

use bmo_tool::SessionRequest;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::{JsonSchema, Schema};
use serde_json::{Map, Value, json};

use crate::error::{ErrorCode, WireError};
use crate::wire::{
  CallRequest, CatalogueEntry, ClaimRequest, Claimed, DescribedEntry, Facts, Issued, ToolResult,
};

/// Where the schemas the routes refer to stand in the document.
const SCHEMAS: &str = "/components/schemas";

const DESCRIPTION: &str = "The tool host's control socket: HTTP/1.1 over a Unix socket that only the host's own \
  user reaches, so it carries no token. Bodies are JSON both ways unless a route answers plain text. Every failure \
  and refusal is a non-2xx status with a body of `code` and `error`. The tools face, MCP over HTTP on the loopback \
  port, is MCP's own protocol and is not stated here.";

/// What a route answers where it succeeds.
enum Answer {
  Json(u16, Schema),
  Text,
  Empty,
}

/// One route of the control socket.
struct Route {
  method: &'static str,
  path: &'static str,
  tag: &'static str,
  summary: &'static str,
  parameters: Vec<Value>,
  request: Option<Schema>,
  answer: Answer,
  refusals: &'static [ErrorCode],
}

fn path_parameter(name: &str, description: &str) -> Value {
  json!({ "name": name, "in": "path", "required": true, "description": description, "schema": { "type": "string" } })
}

fn query_parameter(name: &str, description: &str, schema: Value) -> Value {
  json!({ "name": name, "in": "query", "required": false, "description": description, "schema": schema })
}

fn schema<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
  generator.subschema_for::<T>()
}

fn routes(generator: &mut SchemaGenerator) -> Vec<Route> {
  vec![
    Route {
      method: "post",
      path: "/sessions",
      tag: "sessions",
      summary: "Issue a session record and answer the token it is known by.",
      parameters: Vec::new(),
      request: Some(schema::<SessionRequest>(generator)),
      answer: Answer::Json(200, schema::<Issued>(generator)),
      refusals: &[ErrorCode::MalformedRecord, ErrorCode::UnwritableRecord],
    },
    Route {
      method: "delete",
      path: "/sessions/{token}",
      tag: "sessions",
      summary: "Revoke a record, whether or not the token named one, and end what its tools kept.",
      parameters: vec![path_parameter("token", "The token the record was issued with.")],
      request: None,
      answer: Answer::Empty,
      refusals: &[ErrorCode::UnwritableRecord],
    },
    Route {
      method: "get",
      path: "/listings/{namespace}",
      tag: "namespaces",
      summary: "The namespace's listing as plain text, one line per entry that declares one, in its order.",
      parameters: vec![path_parameter("namespace", "The namespace.")],
      request: None,
      answer: Answer::Text,
      refusals: &[ErrorCode::NoListing],
    },
    Route {
      method: "get",
      path: "/facts/{namespace}",
      tag: "namespaces",
      summary: "What the namespace says of itself: its digest, and each list its entries add an item to.",
      parameters: vec![path_parameter("namespace", "The namespace.")],
      request: None,
      answer: Answer::Json(200, schema::<Facts>(generator)),
      refusals: &[ErrorCode::UnknownNamespace],
    },
    Route {
      method: "get",
      path: "/catalogue",
      tag: "catalogue",
      summary: "A page of every entry, those for the system alone among them, in order of full name.",
      parameters: vec![
        query_parameter(
          "after",
          "An entry's full name: the entries whose full names sort after it. Not with `before`.",
          json!({ "type": "string" }),
        ),
        query_parameter(
          "before",
          "An entry's full name: the entries whose full names sort before it. Not with `after`.",
          json!({ "type": "string" }),
        ),
        query_parameter(
          "limit",
          "At most this many entries, the nearest the cursor; every one where absent.",
          json!({ "type": "integer", "minimum": 1 }),
        ),
      ],
      request: None,
      answer: Answer::Json(200, schema::<Vec<CatalogueEntry>>(generator)),
      refusals: &[ErrorCode::MalformedRequest],
    },
    Route {
      method: "get",
      path: "/catalogue/{namespace}/{name}",
      tag: "catalogue",
      summary: "One entry described, its schemas with it.",
      parameters: vec![
        path_parameter("namespace", "The entry's namespace."),
        path_parameter("name", "The entry's name."),
      ],
      request: None,
      answer: Answer::Json(200, schema::<DescribedEntry>(generator)),
      refusals: &[ErrorCode::UnknownEntry],
    },
    Route {
      method: "post",
      path: "/calls",
      tag: "calls",
      summary: "Call an entry under the record the request carries, as a session's call is dispatched, entries for \
        the system alone among those reached. A tool's own failed result is answered 200.",
      parameters: Vec::new(),
      request: Some(schema::<CallRequest>(generator)),
      answer: Answer::Json(200, schema::<ToolResult>(generator)),
      refusals: &[
        ErrorCode::MalformedRequest,
        ErrorCode::UnknownEntry,
        ErrorCode::ArgumentsMisfit,
        ErrorCode::GrantRefused,
        ErrorCode::ToolFailed,
      ],
    },
    Route {
      method: "post",
      path: "/claims",
      tag: "calls",
      summary: "Which entry the record holds claims an address, and the arguments it reads the page with.",
      parameters: Vec::new(),
      request: Some(schema::<ClaimRequest>(generator)),
      answer: Answer::Json(200, schema::<Claimed>(generator)),
      refusals: &[ErrorCode::MalformedRequest],
    },
    Route {
      method: "get",
      path: "/health",
      tag: "health",
      summary: "`ok`, as plain text, where the host answers.",
      parameters: Vec::new(),
      request: None,
      answer: Answer::Text,
      refusals: &[],
    },
  ]
}

impl Route {
  fn operation(&self, error: &Schema) -> Value {
    let mut responses = Map::new();

    match &self.answer {
      Answer::Json(status, schema) => {
        responses.insert(
          status.to_string(),
          json!({ "description": "The route's answer.", "content": { "application/json": { "schema": schema } } }),
        );
      }
      Answer::Text => {
        responses.insert(
          "200".to_owned(),
          json!({ "description": "The route's answer.", "content": { "text/plain": { "schema": { "type": "string" } } } }),
        );
      }
      Answer::Empty => {
        responses.insert("204".to_owned(), json!({ "description": "Done; no body." }));
      }
    }

    let mut by_status: BTreeMap<u16, Vec<&'static str>> = BTreeMap::new();

    for code in self.refusals {
      by_status.entry(code.status().as_u16()).or_default().push(code.as_str());
    }

    for (status, codes) in by_status {
      responses.insert(
        status.to_string(),
        json!({
          "description": format!("Refused or failed: {}.", codes.join(", ")),
          "content": { "application/json": { "schema": {
            "allOf": [error, { "properties": { "code": { "enum": codes } } }],
          } } },
        }),
      );
    }

    let mut operation = json!({
      "tags": [self.tag],
      "summary": self.summary,
      "responses": responses,
    });

    if !self.parameters.is_empty() {
      operation["parameters"] = Value::Array(self.parameters.clone());
    }

    if let Some(request) = &self.request {
      operation["requestBody"] =
        json!({ "required": true, "content": { "application/json": { "schema": request } } });
    }

    operation
  }
}

/// The control socket's OpenAPI document.
pub fn openapi() -> Value {
  let mut generator = SchemaSettings::draft2020_12()
    .for_serialize()
    .with(|settings| {
      settings.definitions_path = SCHEMAS.into();
      settings.meta_schema = None;
    })
    .into_generator();
  let error = generator.subschema_for::<WireError>();
  let mut paths = Map::new();

  for route in routes(&mut generator) {
    if let Some(item) = paths.entry(route.path).or_insert_with(|| json!({})).as_object_mut() {
      item.insert(route.method.to_owned(), route.operation(&error));
    }
  }

  json!({
    "openapi": "3.1.0",
    "info": {
      "title": "The tool host's control socket",
      "version": env!("CARGO_PKG_VERSION"),
      "description": DESCRIPTION,
    },
    "tags": [
      { "name": "sessions", "description": "The session records the host serves each session's tools by." },
      { "name": "namespaces", "description": "What a namespace says of itself beside its tools." },
      { "name": "catalogue", "description": "Every entry the host serves, its icon and its schedules." },
      { "name": "calls", "description": "Calls and claims made under the record a request carries." },
      { "name": "health", "description": "Whether the host answers." },
    ],
    "paths": paths,
    "components": { "schemas": generator.take_definitions(true) },
  })
}

/// The document as the repository keeps it in `docs/openapi.json`.
pub fn openapi_json() -> String {
  let mut text = serde_json::to_string_pretty(&openapi()).unwrap_or_default();

  text.push('\n');
  text
}
