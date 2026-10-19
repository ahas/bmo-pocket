//! The bodies and queries of the control socket's routes (docs/api.md, 「The control socket」), as the host writes
//! and reads them. A field that may have no value is always written, as null.

use std::collections::BTreeMap;

use bmo_tool::Session;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What `POST /sessions` answers: the token the record was issued with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Issued {
  pub token: String,
}

/// The toolbook tool an entry is called through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReachedBy {
  Call,
  Act,
}

/// One schedule an entry declares, and where it stands in the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleState {
  /// The cron pattern its manifest declares.
  pub cron: String,
  /// The arguments each scheduled call is handed.
  pub arguments: Map<String, Value>,
  /// When it is next due, in epoch milliseconds; none where the pattern names no time to come.
  pub next_at: Option<i64>,
  /// When its last call was made since the host started, in epoch milliseconds.
  pub last_at: Option<i64>,
  /// Why its last call failed, where it did.
  pub last_error: Option<String>,
  /// Whether a call of it is running now.
  pub running: bool,
}

/// One entry of the catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueEntry {
  /// Its full name, `<namespace>/<name>`.
  pub name: String,
  pub read_only: bool,
  pub reached_by: ReachedBy,
  /// What a record must hold to be shown it whole.
  pub capabilities: Vec<String>,
  /// The room relation the sender of the message it acts on must hold.
  pub grant: Option<String>,
  /// Whether it is for the system alone.
  pub system: bool,
  /// Whether its namespace lists it by name.
  pub resident: bool,
  /// Its tool's placed folder name.
  pub placed: String,
  pub description: String,
  /// The icon its manifest declares for it.
  pub icon: Option<String>,
  pub schedules: Vec<ScheduleState>,
}

/// One entry described, its schemas with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DescribedEntry {
  #[serde(flatten)]
  pub entry: CatalogueEntry,
  /// The entry as the toolbook's `describe` says it.
  pub described: String,
  pub input_schema: Value,
  pub output_schema: Option<Value>,
}

/// Where a page of the catalogue begins and how much of it is answered: one cursor, an entry's full name, and a
/// limit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogueQuery {
  /// The entries whose full names sort after this one.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<String>,
  /// The entries whose full names sort before this one.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub before: Option<String>,
  /// At most this many entries, the nearest the cursor.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub limit: Option<u32>,
}

/// What a namespace says of itself: the digest of what it shows, and each list its entries add an item to, by the
/// list's name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Facts {
  pub digest: String,
  #[serde(flatten)]
  pub lists: BTreeMap<String, Vec<Map<String, Value>>>,
}

/// A call made through the control socket, under the record it carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallRequest {
  pub session: Session,
  /// The entry's full name.
  pub tool: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub arguments: Option<Map<String, Value>>,
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

/// Which entry claims an address, asked under the record it carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimRequest {
  pub session: Session,
  pub address: String,
}

/// The entry that claims an address and the arguments it reads the page with; both none where no entry claims
/// it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Claimed {
  pub entry: Option<String>,
  pub arguments: Option<Value>,
}
