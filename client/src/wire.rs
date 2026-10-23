//! The bodies and queries of the control socket's routes (docs/api.md, 「The control socket」), as the host writes
//! and reads them. A field that may have no value is always written, as null.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use bmo_tool::{Granted, Session};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What `GET /health` answers: whether the host is ready, why not where it is not, and its version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Health {
  pub ready: bool,
  pub reason: Option<String>,
  pub version: String,
}

/// Where a page of a collection keyed by name begins and how much of it is answered: the last name a page
/// answered, and a limit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameQuery {
  /// The members whose names sort after this one.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<String>,
  /// At most this many members; every one where absent.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(range(min = 1))]
  pub limit: Option<u32>,
}

/// Where a page of a collection keyed by time begins and how much of it is answered: the last time a page
/// answered, in epoch milliseconds, and a limit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimeQuery {
  /// The members due after this time, in epoch milliseconds.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<i64>,
  /// At most this many members, with every member due at the last one's time beside it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(range(min = 1))]
  pub limit: Option<u32>,
}

/// One namespace and what it says of itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Namespace {
  pub name: String,
  /// Its digest, and the verbs, records, icons and lists its entries add, as `docs/mcp-host.md` states them.
  pub facts: Map<String, Value>,
  /// How many of its entries the token sees.
  pub tool_count: u32,
}

/// A page of namespaces, in order of name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NamespacePage {
  pub items: Vec<Namespace>,
  /// Whether anything stands past the last item.
  pub more: bool,
}

/// What a tool says of how it behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolHints {
  pub read_only: bool,
  pub destructive: bool,
  /// Whether calling it twice with the same arguments does what calling it once does; null where it says nothing.
  pub idempotent: Option<bool>,
  pub open_world: bool,
}

/// The line a tool adds to its namespace's listing, and its place among the namespace's lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Listing {
  pub line: String,
  pub order: i64,
}

/// One tool of a namespace, in the form the token holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
  pub namespace: String,
  pub name: String,
  pub title: Option<String>,
  pub description: String,
  /// Its input schema, in the form the token holds it.
  pub input: Value,
  /// The schema of what it answers with, where it declares one.
  pub output: Option<Value>,
  /// The icon its manifest declares for it, a name its readers know.
  pub icon: Option<String>,
  /// What a session must hold to be shown it whole.
  pub capabilities: Vec<String>,
  /// The resource a session's grants must hold for a call of it to run.
  pub grant: Option<String>,
  pub hints: ToolHints,
  pub listing: Option<Listing>,
  /// Whether it is for the system alone, shown to no session.
  pub system: bool,
  /// The full text of the manual its manifest names, where it names one.
  pub manual: Option<String>,
  /// Which of the toolbook's calling tools reaches it.
  pub reached_by: ReachedBy,
}

/// Which of the toolbook's two calling tools reaches a tool: `call` one that only reads, `act` any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReachedBy {
  Call,
  Act,
}

impl ReachedBy {
  /// The calling tool's name.
  pub fn name(self) -> &'static str {
    match self {
      Self::Call => "call",
      Self::Act => "act",
    }
  }
}

/// A page of a namespace's tools, in order of name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolPage {
  pub items: Vec<Tool>,
  pub more: bool,
}

/// A call of one tool under the token's session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolCallRequest {
  pub namespace: String,
  pub name: String,
  /// The tool's own arguments.
  pub input: Map<String, Value>,
}

/// A call made, and the tool's result as the tool answered it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
  pub namespace: String,
  pub name: String,
  pub result: ToolResult,
  /// How long the call took, in milliseconds.
  pub spent_ms: u64,
}

/// An MCP tool result, as the tool answered it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
  pub content: Vec<Value>,
  /// Whether the tool says it failed.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub is_error: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub structured_content: Option<Value>,
  /// Whatever else the tool wrote, kept as it wrote it.
  #[serde(flatten)]
  pub rest: Map<String, Value>,
}

/// Whether the token's session may have an address read, and by which entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
  /// The address.
  pub resource: String,
}

/// The answer to a claim: the entry that claims the address and the arguments it reads it with, where the
/// session holds the entry and its grant; why not, where it does not.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
  pub resource: String,
  pub granted: bool,
  /// Why the claim is not granted; null where it is.
  pub reason: Option<String>,
  pub namespace: Option<String>,
  pub tool_name: Option<String>,
  pub arguments: Option<Value>,
}

/// A tool session as `POST /tool-sessions` answers it: its token and the record it was issued with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolSession {
  pub token: String,
  pub holder_name: String,
  pub namespaces: BTreeSet<String>,
  pub capabilities: BTreeSet<String>,
  pub grants: Vec<Granted>,
  pub shown: BTreeSet<String>,
  pub variables: BTreeMap<String, String>,
  pub variables_file: Option<PathBuf>,
  pub labels: Map<String, Value>,
  /// When the session lapses, in epoch milliseconds; null where it stands until revoked.
  pub expires_at: Option<u64>,
  /// When it was issued, in epoch milliseconds.
  pub created_at: u64,
}

impl ToolSession {
  /// The tool session a record was issued as, under its token.
  pub fn issued(token: String, session: &Session) -> Self {
    Self {
      token,
      holder_name: session.holder_name.clone(),
      namespaces: session.namespaces.clone(),
      capabilities: session.capabilities.clone(),
      grants: session.grants.clone(),
      shown: session.shown.clone(),
      variables: session.variables.clone(),
      variables_file: session.variables_file.clone(),
      labels: session.labels.clone(),
      expires_at: session.expires_at,
      created_at: session.created_at,
    }
  }
}

/// One schedule an entry declares, and where it stands in the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
  pub namespace: String,
  pub tool_name: String,
  /// The cron pattern its manifest declares.
  pub cron: String,
  /// The arguments each scheduled call is handed.
  pub arguments: Map<String, Value>,
  /// When it is next due, in epoch milliseconds.
  pub next_at: i64,
  /// When its last call was made since the host started, in epoch milliseconds.
  pub last_at: Option<i64>,
  /// Why its last call failed, where it did.
  pub last_error: Option<String>,
  /// Whether a call of it is running now.
  pub running: bool,
}

/// A page of schedules, in order of when each is next due.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchedulePage {
  pub items: Vec<Schedule>,
  pub more: bool,
}
