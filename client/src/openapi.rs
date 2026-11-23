//! The OpenAPI document of the control socket (docs/api.md, 「The OpenAPI document and the clients」): every route,
//! its query, its request body, its answer and the error codes it can give, the body schemas taken from the types
//! this crate sends and reads, which are the types the host answers with; and the harness plugin, under
//! `x-harness` and `x-tools`.

use std::collections::BTreeMap;

use bmo_tool::SessionRequest;
use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::{JsonSchema, Schema};
use serde_json::{Map, Value, json};

use crate::error::{ErrorCode, WireError};
use crate::harness::{ENTRY, MCP_COMMAND, PLUGIN, harness_tools};
use crate::wire::{
  Claim, ClaimRequest, Event, EventPage, Health, Namespace, NamespacePage, SchedulePage, ScreenCallRequest,
  ScreenPage, ServerPage, SessionPage, StandingSession, Tool, ToolCall, ToolCallRequest, ToolPage,
  ToolSession, UsePage,
};

/// Where the schemas the routes refer to stand in the document.
const SCHEMAS: &str = "/components/schemas";

const DESCRIPTION: &str = "The tool host's control socket: HTTP/1.1 over a Unix socket. Every route takes a bearer \
  token: the issuer token the host keeps, which alone issues and revokes tool sessions and sees every namespace, \
  tool and schedule, the tool sessions standing, the external servers and the event stream, or a tool session's \
  token, which sees the namespaces its record names and the tools it holds there, and calls and claims under that \
  record. Bodies are JSON both ways, but for the discipline, which answers text, and the event stream, which \
  answers server-sent events unless a page is asked for as JSON. Every failure and refusal is a non-2xx status \
  with a body of `code` and `error`. The tools face, MCP over HTTP on the loopback port, is MCP's own protocol and is not stated here; the console face answers this \
  document's reads in its shapes behind the console token, and is not stated here either.";

/// Which token a route takes.
#[derive(Clone, Copy)]
enum Token {
  Either,
  Issuer,
  Session,
}

impl Token {
  fn said(self) -> &'static str {
    match self {
      Self::Either => "either",
      Self::Issuer => "issuer",
      Self::Session => "session",
    }
  }
}

/// What a route answers with where it succeeds.
enum Answer {
  /// A JSON body of this schema.
  Json(Schema),
  /// A body of plain UTF-8 text.
  Text,
  /// Server-sent events, each event's data of the second schema, or with `Accept: application/json` a page of the
  /// first.
  Events(Schema, Schema),
  /// No body.
  Empty,
  /// A file's bytes, of the media type its name's extension names.
  Bytes,
}

/// One route of the control socket.
struct Route {
  method: &'static str,
  path: &'static str,
  operation: &'static str,
  tag: &'static str,
  summary: &'static str,
  token: Token,
  parameters: Vec<Value>,
  request: Option<Schema>,
  /// The status it succeeds with, and the body it answers with.
  answer: (u16, Answer),
  codes: &'static [ErrorCode],
}

fn path_parameter(name: &str, description: &str) -> Value {
  json!({ "name": name, "in": "path", "required": true, "description": description, "schema": { "type": "string" } })
}

fn query_parameter(name: &str, description: &str, schema: Value) -> Value {
  json!({ "name": name, "in": "query", "required": false, "description": description, "schema": schema })
}

fn limit() -> Value {
  query_parameter(
    "limit",
    "At most this many members; every one where absent.",
    json!({ "type": "integer", "minimum": 1 }),
  )
}

fn after_name(of: &str) -> Value {
  query_parameter(
    "after",
    &format!("The last {of} name a page answered: the members whose names sort after it."),
    json!({ "type": "string" }),
  )
}

fn after_time(what: &str) -> Value {
  query_parameter(
    "after",
    &format!("A time in epoch milliseconds: the {what} after it."),
    json!({ "type": "integer", "format": "int64" }),
  )
}

fn header_parameter(name: &str, description: &str) -> Value {
  json!({ "name": name, "in": "header", "required": false, "description": description, "schema": { "type": "string" } })
}

fn schema<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
  generator.subschema_for::<T>()
}

fn routes(generator: &mut SchemaGenerator) -> Vec<Route> {
  use ErrorCode::{
    ArgumentsMisfit, BadRequest, Failed, Forbidden, Gone, NotFound, RunnerUnreached, ToolFailed,
    Unauthorized,
  };

  vec![
    Route {
      method: "get",
      path: "/health",
      operation: "listHealth",
      tag: "health",
      summary: "Readiness and version.",
      token: Token::Either,
      parameters: Vec::new(),
      request: None,
      answer: (200, Answer::Json(schema::<Health>(generator))),
      codes: &[Unauthorized],
    },
    Route {
      method: "get",
      path: "/discipline",
      operation: "readDiscipline",
      tag: "discipline",
      summary: "The paragraph on finding and using tools that a session's system prompt carries, as text.",
      token: Token::Either,
      parameters: Vec::new(),
      request: None,
      answer: (200, Answer::Text),
      codes: &[Unauthorized],
    },
    Route {
      method: "get",
      path: "/events",
      operation: "listEvents",
      tag: "events",
      summary: "The host's event stream: each tool a reading of the catalogue placed or removed, as server-sent \
        events after the cursor and then as each is published, a comment line every fifteen seconds; with \
        `Accept: application/json`, a page of the events held. A cursor older than the window held is gone.",
      token: Token::Issuer,
      parameters: vec![
        query_parameter(
          "after",
          "An event's number: the events numbered after it.",
          json!({ "type": "integer", "format": "int64" }),
        ),
        query_parameter(
          "kinds",
          "The kinds answered, a comma-separated list; every kind where absent.",
          json!({ "type": "string" }),
        ),
        query_parameter(
          "limit",
          "At most this many events, in a page; every one held where absent.",
          json!({ "type": "integer", "minimum": 1 }),
        ),
        header_parameter(
          "Last-Event-ID",
          "The last event's number a stream answered, read where `after` is absent.",
        ),
        header_parameter(
          "Accept",
          "`application/json` for a page of the events held in place of the stream.",
        ),
      ],
      request: None,
      answer: (200, Answer::Events(schema::<EventPage>(generator), schema::<Event>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden, Gone],
    },
    Route {
      method: "get",
      path: "/namespaces",
      operation: "listNamespaces",
      tag: "namespaces",
      summary: "A page of the namespaces the token sees, in order of name.",
      token: Token::Either,
      parameters: vec![after_name("namespace's"), limit()],
      request: None,
      answer: (200, Answer::Json(schema::<NamespacePage>(generator))),
      codes: &[Unauthorized, BadRequest],
    },
    Route {
      method: "get",
      path: "/namespaces/{namespace}",
      operation: "readNamespace",
      tag: "namespaces",
      summary: "One namespace and its facts.",
      token: Token::Either,
      parameters: vec![path_parameter("namespace", "The namespace's name.")],
      request: None,
      answer: (200, Answer::Json(schema::<Namespace>(generator))),
      codes: &[Unauthorized, NotFound],
    },
    Route {
      method: "get",
      path: "/namespaces/{namespace}/tools",
      operation: "listNamespaceTools",
      tag: "namespaces",
      summary: "A page of one namespace's tools the token sees, in order of name, with their icons.",
      token: Token::Either,
      parameters: vec![path_parameter("namespace", "The namespace's name."), after_name("tool's"), limit()],
      request: None,
      answer: (200, Answer::Json(schema::<ToolPage>(generator))),
      codes: &[Unauthorized, BadRequest, NotFound],
    },
    Route {
      method: "get",
      path: "/namespaces/{namespace}/tools/{toolName}",
      operation: "readNamespaceTool",
      tag: "namespaces",
      summary: "One tool, its schemas with it.",
      token: Token::Either,
      parameters: vec![
        path_parameter("namespace", "The namespace's name."),
        path_parameter("toolName", "The tool's name in the namespace."),
      ],
      request: None,
      answer: (200, Answer::Json(schema::<Tool>(generator))),
      codes: &[Unauthorized, NotFound],
    },
    Route {
      method: "post",
      path: "/tool-calls",
      operation: "createToolCall",
      tag: "tool-calls",
      summary: "Call one tool under the token's tool session; answered with its result. A tool's own failed \
        result is answered 200.",
      token: Token::Session,
      parameters: Vec::new(),
      request: Some(schema::<ToolCallRequest>(generator)),
      answer: (200, Answer::Json(schema::<ToolCall>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden, NotFound, ArgumentsMisfit, ToolFailed],
    },
    Route {
      method: "post",
      path: "/claims",
      operation: "createClaim",
      tag: "claims",
      summary: "Whether the token's tool session may have an address read, by the entries and grants it holds.",
      token: Token::Session,
      parameters: Vec::new(),
      request: Some(schema::<ClaimRequest>(generator)),
      answer: (200, Answer::Json(schema::<Claim>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden],
    },
    Route {
      method: "get",
      path: "/tool-sessions",
      operation: "listToolSessions",
      tag: "tool-sessions",
      summary: "A page of the tool sessions standing, in order of when each was issued: each one's holder, \
        record and times, never its token or a variable's value; a page never ends inside a run of equal times.",
      token: Token::Issuer,
      parameters: vec![
        after_time("sessions issued"),
        limit(),
        query_parameter("holder", "A holder's name: its sessions alone.", json!({ "type": "string" })),
      ],
      request: None,
      answer: (200, Answer::Json(schema::<SessionPage>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden],
    },
    Route {
      method: "post",
      path: "/tool-sessions",
      operation: "createToolSession",
      tag: "tool-sessions",
      summary: "Issue a tool session for a holder, carrying the namespaces, capabilities and grants it allows.",
      token: Token::Issuer,
      parameters: Vec::new(),
      request: Some(schema::<SessionRequest>(generator)),
      answer: (201, Answer::Json(schema::<ToolSession>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden, Failed],
    },
    Route {
      method: "get",
      path: "/tool-sessions/{token}",
      operation: "readToolSession",
      tag: "tool-sessions",
      summary: "One standing tool session, as the page answers it, with each server of the tools face its clients \
        reached.",
      token: Token::Issuer,
      parameters: vec![path_parameter("token", "The token the session was issued with.")],
      request: None,
      answer: (200, Answer::Json(schema::<StandingSession>(generator))),
      codes: &[Unauthorized, Forbidden, NotFound],
    },
    Route {
      method: "delete",
      path: "/tool-sessions/{token}",
      operation: "deleteToolSession",
      tag: "tool-sessions",
      summary: "Revoke a tool session and end what its tools kept.",
      token: Token::Issuer,
      parameters: vec![path_parameter("token", "The token the session was issued with.")],
      request: None,
      answer: (204, Answer::Empty),
      codes: &[Unauthorized, Forbidden, NotFound, Failed],
    },
    Route {
      method: "delete",
      path: "/tool-session-records/{id}",
      operation: "deleteToolSessionRecord",
      tag: "tool-sessions",
      summary: "Revoke the tool session a record id names, as by its token, and end what its tools kept: the way \
        back for a session whose token is lost.",
      token: Token::Issuer,
      parameters: vec![path_parameter("id", "The record's id, the SHA-256 of its token in hexadecimal.")],
      request: None,
      answer: (204, Answer::Empty),
      codes: &[Unauthorized, Forbidden, NotFound, Failed],
    },
    Route {
      method: "get",
      path: "/schedules",
      operation: "listSchedules",
      tag: "schedules",
      summary: "A page of the tool calls the host makes at the times each tool's own manifest declares, in order \
        of when each is next due; a page never ends inside a run of equal times.",
      token: Token::Either,
      parameters: vec![after_time("schedules next due"), limit()],
      request: None,
      answer: (200, Answer::Json(schema::<SchedulePage>(generator))),
      codes: &[Unauthorized, BadRequest],
    },
    Route {
      method: "get",
      path: "/uses",
      operation: "listKeptCalls",
      tag: "uses",
      summary: "Every asynchronous call the host keeps, across every session, oldest first, each as one call is \
        answered: a watch, a reminder or a call still running, narrowed to a state and to the agent its wake is for.",
      token: Token::Issuer,
      parameters: vec![
        query_parameter("state", "One state: the calls in it alone.", json!({ "type": "string" })),
        query_parameter(
          "agent",
          "An agent's name: the calls whose wake is for it alone.",
          json!({ "type": "string" }),
        ),
      ],
      request: None,
      answer: (200, Answer::Json(schema::<UsePage>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden],
    },
    Route {
      method: "get",
      path: "/screens",
      operation: "listScreens",
      tag: "screens",
      summary: "The screens of the personal tools the token's session reaches, each with its words, its layer and \
        the paths of its module and its stylesheet under the digest of its index, and whether the runner the \
        session's record names answered.",
      token: Token::Session,
      parameters: vec![],
      request: None,
      answer: (200, Answer::Json(schema::<ScreenPage>(generator))),
      codes: &[Unauthorized, Forbidden],
    },
    Route {
      method: "get",
      path: "/screens/{tool}/{digest}/{path}",
      operation: "readScreenFile",
      tag: "screens",
      summary: "One file of a personal tool's screen, read on its owner's machine and answered where its bytes are \
        the ones the screen's index names; kept for good, since the digest names the build.",
      token: Token::Session,
      parameters: vec![
        path_parameter("tool", "The personal tool's name."),
        path_parameter("digest", "The SHA-256 of the screen's index, in lowercase hexadecimal."),
        path_parameter("path", "The file's path in the screen's folder, its folders among it."),
      ],
      request: None,
      answer: (200, Answer::Bytes),
      codes: &[Unauthorized, Forbidden, NotFound, RunnerUnreached],
    },
    Route {
      method: "post",
      path: "/screens/{tool}/calls",
      operation: "callScreenTool",
      tag: "screens",
      summary: "A call of one of a screen's own tool's entries, by its name in the personal namespace, dispatched \
        as `POST /tool-calls` dispatches it.",
      token: Token::Session,
      parameters: vec![path_parameter("tool", "The personal tool's name.")],
      request: Some(schema::<ScreenCallRequest>(generator)),
      answer: (200, Answer::Json(schema::<ToolCall>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden, NotFound, ArgumentsMisfit, ToolFailed],
    },
    Route {
      method: "get",
      path: "/servers",
      operation: "listServers",
      tag: "servers",
      summary: "A page of the external servers registered, in order of name: where each is reached and whether \
        its tools are listed, never its headers, token file or environment.",
      token: Token::Issuer,
      parameters: vec![after_name("server's"), limit()],
      request: None,
      answer: (200, Answer::Json(schema::<ServerPage>(generator))),
      codes: &[Unauthorized, BadRequest, Forbidden],
    },
  ]
}

impl Route {
  fn operation(&self, error: &Schema) -> Value {
    let mut responses = Map::new();
    let (status, answer) = &self.answer;

    responses.insert(
      status.to_string(),
      match answer {
        Answer::Json(schema) => {
          json!({ "description": "The route's answer.", "content": { "application/json": { "schema": schema } } })
        }
        Answer::Text => json!({
          "description": "The route's answer, as text.",
          "content": { "text/plain": { "schema": { "type": "string" } } },
        }),
        Answer::Events(page, event) => json!({
          "description": "The stream, or a page of the events held where `Accept: application/json` asks for one.",
          "content": {
            "text/event-stream": {
              "schema": { "type": "string" },
              "x-event": event,
            },
            "application/json": { "schema": page },
          },
        }),
        Answer::Empty => json!({ "description": "Done; no body." }),
        Answer::Bytes => json!({
          "description": "The file's bytes, of the media type its extension names.",
          "content": { "application/octet-stream": { "schema": { "type": "string", "format": "binary" } } },
        }),
      },
    );

    let mut by_status: BTreeMap<u16, Vec<&'static str>> = BTreeMap::new();

    for code in self.codes {
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
      "operationId": self.operation,
      "tags": [self.tag],
      "summary": self.summary,
      "x-token": self.token.said(),
      "x-codes": self.codes.iter().map(|code| code.as_str()).collect::<Vec<_>>(),
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

  let tools: Vec<Value> = harness_tools()
    .iter()
    .map(|tool| {
      json!({
        "name": tool.name,
        "description": tool.description,
        "input": (tool.input_in)(&mut generator),
        "output": (tool.output_in)(&mut generator),
        "route": tool.route,
      })
    })
    .collect();

  json!({
    "openapi": "3.1.0",
    "info": {
      "title": "bmo-pocket",
      "version": env!("CARGO_PKG_VERSION"),
      "description": DESCRIPTION,
    },
    "servers": [{ "url": "/", "description": "the control socket" }],
    "security": [{ "bearer": [] }],
    "tags": [
      { "name": "health", "description": "Whether the host is ready." },
      { "name": "events", "description": "The tools each reading of the catalogue placed and removed." },
      { "name": "discipline", "description": "The prompt text a session handed the tool host's tools carries." },
      { "name": "namespaces", "description": "Every namespace with its tools: the catalogue." },
      { "name": "tool-calls", "description": "Calls made under a tool session." },
      { "name": "claims", "description": "Which entry a tool session may have an address read by." },
      { "name": "tool-sessions", "description": "The tool sessions a harness is handed tools through." },
      { "name": "schedules", "description": "The calls the host makes at the times each tool declares." },
      { "name": "uses", "description": "The asynchronous calls the host keeps for every session." },
      { "name": "servers", "description": "The external MCP servers registered, and whether each is listed." },
    ],
    "paths": paths,
    "components": {
      "securitySchemes": { "bearer": { "type": "http", "scheme": "bearer" } },
      "schemas": generator.take_definitions(true),
    },
    "x-harness": { "plugin": PLUGIN, "mcp": MCP_COMMAND, "entry": ENTRY },
    "x-tools": tools,
  })
}

/// The document as the repository keeps it in `docs/openapi.json`.
pub fn openapi_json() -> String {
  let mut text = serde_json::to_string_pretty(&openapi()).unwrap_or_default();

  text.push('\n');
  text
}
