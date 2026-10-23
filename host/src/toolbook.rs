//! The toolbook (docs/toolbook.md): the server every line is handed beside its namespaces, through which a
//! session reaches the tools it holds that their namespaces do not list. It lists them, describes one,
//! and calls one by the same dispatch a direct call takes. What it answers reaches the session only as a
//! tool result, so no session's prefix moves for it.

use std::sync::Arc;

use bmo_tool::{Answer, Session};
use rmcp::model::{
  CallToolRequestParams, CallToolResponse, Implementation, InitializeResult, ListToolsResult,
  PaginatedRequestParams, ServerCapabilities,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::handler::{Entry, Host, answered, private, read, session, unknown};

/// The path segment the toolbook is served at, and the server name the CLI calls its tools under.
pub const TOOLBOOK: &str = "toolbook";

#[derive(Clone)]
pub struct Toolbook {
  pub host: Arc<Host>,
}

#[derive(Deserialize, JsonSchema)]
struct ListInput {
  /// Words the tool's full name or description holds, to narrow the list.
  query: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct DescribeInput {
  /// The tool's full name, namespace/tool, as list gives it.
  tool: String,
}

#[derive(Deserialize, JsonSchema)]
struct CallInput {
  /// The tool's full name, namespace/tool, as list gives it.
  tool: String,
  /// The tool's own arguments, as one object.
  arguments: Map<String, Value>,
}

// The schema of an input type, as a tool's own input schema is written.
fn schema<T: JsonSchema>() -> Value {
  let mut schema = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);

  if let Value::Object(fields) = &mut schema {
    fields.remove("$schema");
    fields.remove("title");
  }

  schema
}

/// The toolbook's own four tools, as the protocol describes them. Their descriptions stand in every
/// session's prefix; `list`'s is what tells a session how a tool its instructions name is reached.
pub fn tools() -> Vec<Value> {
  let reading = json!({ "readOnlyHint": true, "destructiveHint": false, "openWorldHint": false });

  vec![
    json!({
      "name": "list",
      "description": "Lists the tools you reach through the toolbook rather than by their own names: a tool your instructions name that is not among your tools is one of them. One line each: its full name (namespace/tool), whether call or act reaches it, and what it answers. Pass query to narrow the list.",
      "inputSchema": schema::<ListInput>(),
      "annotations": reading,
    }),
    json!({
      "name": "describe",
      "description": "One toolbook tool by its full name: what it does, its manual on how to call it where it has one, its input schema, whether it only reads, and whether call or act reaches it.",
      "inputSchema": schema::<DescribeInput>(),
      "annotations": reading,
    }),
    json!({
      "name": "call",
      "description": "Calls a tool that only reads, by its full name with its own arguments, and answers with that tool's answer. A tool that acts is reached with act.",
      "inputSchema": schema::<CallInput>(),
      "annotations": { "readOnlyHint": true, "destructiveHint": false, "openWorldHint": true },
    }),
    json!({
      "name": "act",
      "description": "Calls a tool that acts, by its full name with its own arguments, and answers with that tool's answer.",
      "inputSchema": schema::<CallInput>(),
      "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true },
    }),
  ]
}

// Which of the toolbook's two calling tools reaches an entry.
fn reached_by(entry: &Entry) -> &'static str {
  if entry.read_only() { "call" } else { "act" }
}

/// A description's first sentence, which a line of the list carries.
pub fn first_sentence(text: &str) -> &str {
  let text = text.trim();

  text.find(". ").map_or(text, |end| &text[..=end])
}

/// The list's lines: the booked tools the session holds, by full name, each narrowed by `query` where one
/// is given.
pub fn listed(held: &[Entry], query: Option<&str>) -> Vec<String> {
  let query = query.map(str::trim).filter(|query| !query.is_empty()).map(str::to_lowercase);
  let mut lines: Vec<(String, String)> = held
    .iter()
    .filter(|entry| !entry.resident())
    .filter(|entry| {
      query.as_ref().is_none_or(|query| {
        entry.full_name().to_lowercase().contains(query)
          || entry.description().to_lowercase().contains(query)
      })
    })
    .map(|entry| {
      let full = entry.full_name();
      let line = format!("{full} · {} — {}", reached_by(entry), first_sentence(&entry.description()));

      (full, line)
    })
    .collect();

  lines.sort();
  lines.into_iter().map(|(_, line)| line).collect()
}

/// One entry described: what it does, its manual where it has one, how it is reached, whether it only reads,
/// and its schemas.
pub fn described(entry: &Entry) -> String {
  let mut said = vec![
    format!("{} — reached with {}", entry.full_name(), reached_by(entry)),
    entry.description(),
  ];

  if let Some(manual) = entry.manual() {
    said.push(format!("Manual:\n{}", manual.trim()));
  }

  said.push(format!("Read-only: {}", if entry.read_only() { "yes" } else { "no" }));
  said.push(format!("Input schema: {}", entry.input_schema()));

  if let Some(output) = entry.output_schema() {
    said.push(format!("Answers with: {output}"));
  }

  said.join("\n")
}

/// The entry a full name names among those the session holds, or why there is none, in the words a
/// direct call of an unknown tool is refused with.
pub fn named(held: Vec<Entry>, full: &str) -> Result<Entry, String> {
  let found = full.split_once('/').and_then(|(namespace, name)| {
    held.into_iter().find(|entry| entry.namespace() == namespace && entry.name() == name)
  });

  found.ok_or_else(|| unknown(full))
}

// The input a toolbook tool was handed, or the protocol's refusal of what does not fit it.
fn input<T: serde::de::DeserializeOwned>(tool: &str, arguments: Value) -> Result<T, ErrorData> {
  let arguments = if arguments.is_null() { json!({}) } else { arguments };

  serde_json::from_value(arguments).map_err(|why| ErrorData::invalid_params(format!("{tool}: {why}"), None))
}

impl Toolbook {
  async fn list(&self, session: &Session, asked: ListInput) -> Result<CallToolResponse, ErrorData> {
    let held = self.host.held(session);
    let lines = listed(&held, asked.query.as_deref());

    if lines.is_empty() {
      return answered(&Answer::text(match asked.query {
        Some(query) => format!("No toolbook tool matches '{query}'."),
        None => "No tool is reached through the toolbook in this session.".to_owned(),
      }));
    }

    answered(&Answer::lines(&lines, "narrow it with query"))
  }

  async fn describe(&self, session: &Session, asked: DescribeInput) -> Result<CallToolResponse, ErrorData> {
    match named(self.host.held(session), &asked.tool) {
      Ok(entry) => answered(&Answer::text(described(&entry))),
      Err(why) => answered(&Answer::failure(why)),
    }
  }

  // A call of the tool a full name names: refused where the session holds no such tool, and by `call`
  // where the tool acts; otherwise dispatched as a direct call is.
  async fn forward(
    &self,
    context: &RequestContext<RoleServer>,
    proxy: &str,
    asked: CallInput,
  ) -> Result<CallToolResponse, ErrorData> {
    let session = session(context)?;
    let entry = match named(self.host.held(&session), &asked.tool) {
      Ok(entry) => entry,
      Err(why) => return answered(&Answer::failure(why)),
    };

    if proxy == "call" && !entry.read_only() {
      return answered(&Answer::failure(format!(
        "{} acts, so call does not reach it: call it with act.",
        entry.full_name()
      )));
    }

    self.host.dispatch(&entry, &session, Value::Object(asked.arguments)).await
  }
}

impl ServerHandler for Toolbook {
  fn get_info(&self) -> InitializeResult {
    let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build());
    let mut server = Implementation::default();

    server.name = TOOLBOOK.to_owned();
    server.version = env!("CARGO_PKG_VERSION").to_owned();
    info.server_info = server;
    info
  }

  async fn list_tools(
    &self,
    _: Option<PaginatedRequestParams>,
    context: RequestContext<RoleServer>,
  ) -> Result<ListToolsResult, ErrorData> {
    session(&context)?;

    read(private(json!({ "tools": tools() })))
  }

  async fn call_tool(
    &self,
    request: CallToolRequestParams,
    context: RequestContext<RoleServer>,
  ) -> Result<CallToolResponse, ErrorData> {
    let session = session(&context)?;
    let arguments = request.arguments.map_or(Value::Null, Value::Object);

    match request.name.as_ref() {
      "list" => self.list(&session, input("list", arguments)?).await,
      "describe" => self.describe(&session, input("describe", arguments)?).await,
      proxy @ ("call" | "act") => self.forward(&context, proxy, input(proxy, arguments)?).await,
      other => Err(ErrorData::invalid_params(unknown(other), None)),
    }
  }
}
