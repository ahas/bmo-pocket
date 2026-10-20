//! The harness plugin as the contract states it (docs/api.md, 「The harness plugin」): the plugin's name, the command
//! that starts its MCP server, what a harness does first with it, and its four fixed tools, each with its input and
//! output and the route it calls. The OpenAPI document states them under `x-harness` and `x-tools`, and the plugin
//! serves them.

use schemars::generate::SchemaGenerator;
use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize};

use crate::wire::{NamespacePage, Tool, ToolCall, ToolCallRequest, ToolPage};

/// The plugin's name.
pub const PLUGIN: &str = "ahas-pocket";

/// The command that starts its MCP server.
pub const MCP_COMMAND: &str = "bmo-pocket mcp";

/// What a harness does first with it.
pub const ENTRY: &str = "A harness starts the MCP server with a tool session token and gets every hosted tool of \
  the namespaces the session allows, plus pocket_namespaces, pocket_tools, pocket_tool and pocket_call.";

/// `pocket_namespaces`: a page of the namespaces.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamespacesInput {
  /// The last namespace's name a page answered: the namespaces after it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<String>,
  /// At most this many namespaces.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(range(min = 1))]
  pub limit: Option<u32>,
}

/// `pocket_tools`: a page of one namespace's tools.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolsInput {
  pub namespace: String,
  /// The last tool's name a page answered: the tools after it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<String>,
  /// At most this many tools.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(range(min = 1))]
  pub limit: Option<u32>,
}

/// `pocket_tool`: one tool.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolInput {
  pub namespace: String,
  pub tool_name: String,
}

/// One fixed tool of the plugin.
pub struct HarnessTool {
  pub name: &'static str,
  pub description: &'static str,
  /// The route it calls, as `<METHOD> <path>`.
  pub route: &'static str,
  /// Its input schema, standing alone.
  pub input: fn() -> Schema,
  /// Its output schema, standing alone.
  pub output: fn() -> Schema,
  /// Its input schema, among a document's schemas.
  pub input_in: fn(&mut SchemaGenerator) -> Schema,
  /// Its output schema, among a document's schemas.
  pub output_in: fn(&mut SchemaGenerator) -> Schema,
}

/// The plugin's four fixed tools.
pub fn harness_tools() -> [HarnessTool; 4] {
  [
    HarnessTool {
      name: "pocket_namespaces",
      description: "Every namespace with its tools: the catalogue.",
      route: "GET /namespaces",
      input: || schema_for!(NamespacesInput),
      output: || schema_for!(NamespacePage),
      input_in: |generator| generator.subschema_for::<NamespacesInput>(),
      output_in: |generator| generator.subschema_for::<NamespacePage>(),
    },
    HarnessTool {
      name: "pocket_tools",
      description: "The tools of one namespace, with their icons.",
      route: "GET /namespaces/{namespace}/tools",
      input: || schema_for!(ToolsInput),
      output: || schema_for!(ToolPage),
      input_in: |generator| generator.subschema_for::<ToolsInput>(),
      output_in: |generator| generator.subschema_for::<ToolPage>(),
    },
    HarnessTool {
      name: "pocket_tool",
      description: "One tool's entry and schemas.",
      route: "GET /namespaces/{namespace}/tools/{toolName}",
      input: || schema_for!(ToolInput),
      output: || schema_for!(Tool),
      input_in: |generator| generator.subschema_for::<ToolInput>(),
      output_in: |generator| generator.subschema_for::<Tool>(),
    },
    HarnessTool {
      name: "pocket_call",
      description: "Call one tool under a tool session; answered with its result.",
      route: "POST /tool-calls",
      input: || schema_for!(ToolCallRequest),
      output: || schema_for!(ToolCall),
      input_in: |generator| generator.subschema_for::<ToolCallRequest>(),
      output_in: |generator| generator.subschema_for::<ToolCall>(),
    },
  ]
}
