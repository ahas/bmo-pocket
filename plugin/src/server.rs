//! The MCP server: the four fixed tools and the hosted tools of the session's namespaces, every call made through
//! the control socket's client with the session's token.

use std::collections::BTreeMap;

use bmo_pocket_client::harness::harness_tools;
use bmo_pocket_client::{Pocket, Tool as Member, ToolCallRequest};
use rmcp::model::{
  CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, InitializeResult,
  ListToolsResult, PaginatedRequestParams, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::stdio;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::tools::{call_fixed, every_hosted, fixed, hosted_names, refusal};

/// What the server tells a harness of itself when it starts.
const INSTRUCTIONS: &str = "Serves the tools one tool session holds on a tool host. Every tool of the session's \
  namespaces is listed by its own name, <namespace>_<name>; pocket_namespaces and pocket_tools page through the \
  catalogue, pocket_tool reads one tool's schemas, and pocket_call calls any tool by its namespace and name, among \
  them one whose name another tool's would have shared. A refusal comes back as a tool error carrying the host's \
  status, code and sentence.";

/// The server over one tool session, as its token reaches the control socket.
pub struct PocketServer {
  client: Pocket,
  /// The hosted tools by the name a harness calls them by, as the last listing read them: their namespace and name.
  hosted: Mutex<BTreeMap<String, (String, String)>>,
}

impl PocketServer {
  pub fn new(client: Pocket) -> Self {
    Self {
      client,
      hosted: Mutex::new(BTreeMap::new()),
    }
  }

  /// Every tool the server lists: the four fixed ones, then the hosted ones by name.
  pub async fn listed(&self) -> Result<Vec<Tool>, Value> {
    let members = every_hosted(&self.client).await.map_err(|failure| refusal(&failure))?;
    let hosted = hosted_names(&members);
    let mut listed: Vec<Tool> = harness_tools()
      .iter()
      .map(|tool| {
        mcp_tool(json!({
          "name": tool.name,
          "description": tool.description,
          "inputSchema": standing_alone(serde_json::to_value((tool.input)()).unwrap_or_default()),
          "outputSchema": standing_alone(serde_json::to_value((tool.output)()).unwrap_or_default()),
        }))
      })
      .collect();

    listed.extend(hosted.iter().map(|(name, member)| hosted_tool(name, member)));
    *self.hosted.lock().await = hosted
      .into_iter()
      .map(|(name, member)| (name, (member.namespace.clone(), member.name.clone())))
      .collect();

    Ok(listed)
  }

  // The hosted tool a name stands for, read afresh where the last listing did not hold it.
  async fn hosted(&self, name: &str) -> Result<Option<(String, String)>, Value> {
    if let Some(found) = self.hosted.lock().await.get(name).cloned() {
      return Ok(Some(found));
    }

    self.listed().await?;
    Ok(self.hosted.lock().await.get(name).cloned())
  }

  /// One tool called with its arguments: a fixed tool's route answered, or a hosted tool's result as it answered
  /// it; a refusal as a tool error.
  pub async fn call(&self, name: &str, arguments: Map<String, Value>) -> CallToolResult {
    if fixed(name) {
      return match call_fixed(&self.client, name, Value::Object(arguments)).await {
        Ok(answer) => CallToolResult::structured(answer),
        Err(why) => failed(&why),
      };
    }

    let (namespace, tool) = match self.hosted(name).await {
      Ok(Some(found)) => found,
      Ok(None) => {
        return failed(&json!({ "code": "not-found", "error": format!("There is no tool {name}.") }));
      }
      Err(why) => return failed(&why),
    };
    let asked = ToolCallRequest {
      namespace,
      name: tool,
      input: arguments,
    };

    match self.client.call(&asked).await {
      Ok(made) => serde_json::to_value(made.result)
        .ok()
        .and_then(|result| serde_json::from_value::<CallToolResult>(result).ok())
        .unwrap_or_else(|| {
          failed(&json!({ "error": format!("{name} answered what is not a tool result") }))
        }),
      Err(failure) => failed(&refusal(&failure)),
    }
  }
}

/// The server spoken to on standard input and output until the harness closes it.
pub async fn serve_stdio(client: Pocket) -> Result<(), String> {
  let running = PocketServer::new(client).serve(stdio()).await.map_err(|why| why.to_string())?;

  running.waiting().await.map(|_| ()).map_err(|why| why.to_string())
}

// A tool in the protocol's own type.
fn mcp_tool(said: Value) -> Tool {
  serde_json::from_value(said).expect("a tool the plugin builds reads as the protocol's tool")
}

// A schema standing alone as the protocol takes it: an object at its root, its own meta-schema left out.
fn standing_alone(schema: Value) -> Value {
  let mut object = match schema {
    Value::Object(object) => object,
    _ => Map::new(),
  };

  object.remove("$schema");

  if object.contains_key("$ref") || !object.contains_key("type") {
    object.insert("type".into(), "object".into());
  }

  Value::Object(object)
}

// A hosted tool as the protocol lists it, under the name a harness calls it by.
fn hosted_tool(name: &str, member: &Member) -> Tool {
  let mut annotations = json!({
    "readOnlyHint": member.hints.read_only,
    "destructiveHint": member.hints.destructive,
    "openWorldHint": member.hints.open_world,
  });

  if let Some(idempotent) = member.hints.idempotent {
    annotations["idempotentHint"] = idempotent.into();
  }

  let mut said = json!({
    "name": name,
    "description": member.description,
    "inputSchema": standing_alone(member.input.clone()),
    "annotations": annotations,
  });

  if let Some(title) = &member.title {
    said["title"] = title.clone().into();
  }

  if let Some(output) = &member.output {
    said["outputSchema"] = standing_alone(output.clone());
  }

  mcp_tool(said)
}

/// A failed call as a tool error whose text is the refusal. It carries no structured content, which would be read
/// against the tool's output schema.
fn failed(why: &Value) -> CallToolResult {
  CallToolResult::error(vec![ContentBlock::text(why.to_string())])
}

impl ServerHandler for PocketServer {
  fn get_info(&self) -> InitializeResult {
    let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tools().build());
    let mut server = Implementation::default();

    server.name = "bmo-pocket".to_owned();
    server.version = crate::VERSION.to_owned();
    info.server_info = server;
    info.instructions = Some(INSTRUCTIONS.to_owned());
    info
  }

  async fn list_tools(
    &self,
    _: Option<PaginatedRequestParams>,
    _: RequestContext<RoleServer>,
  ) -> Result<ListToolsResult, ErrorData> {
    let listed = self.listed().await.map_err(|why| {
      ErrorData::internal_error(format!("the tool host refused the listing: {why}"), None)
    })?;

    Ok(ListToolsResult::with_all_items(listed))
  }

  async fn call_tool(
    &self,
    request: CallToolRequestParams,
    _: RequestContext<RoleServer>,
  ) -> Result<CallToolResponse, ErrorData> {
    let arguments = request.arguments.unwrap_or_default();

    Ok(CallToolResponse::Complete(self.call(&request.name, arguments).await))
  }
}
