//! The bodies and queries of the control socket's routes (docs/api.md, 「The control socket」), as the host writes
//! and reads them. A field that may have no value is always written, as null.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use bmo_tool::{Granted, Session};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What `GET /health` answers: whether the host is ready, why not where it is not, its version, and how many calls
/// a stop of the host would wait on and which.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Health {
  pub ready: bool,
  pub reason: Option<String>,
  pub version: String,
  pub calls_in_hand: u64,
  /// Each call in hand, oldest first.
  pub in_hand: Vec<CallInHand>,
}

/// One call a stop of the host would wait on: what it asks, whose record it runs under, and when it started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CallInHand {
  /// The entry's full name, or the tool's own name for a resource listing or read.
  pub entry: String,
  /// The `holderName` of the record the call runs under.
  pub holder: String,
  /// When it started, in epoch milliseconds.
  pub started_at: u64,
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

/// Where a page of the tool sessions begins, how much of it is answered, and whose sessions it holds: the last time
/// a page answered, in epoch milliseconds, a limit, and a holder's name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionQuery {
  /// The sessions issued after this time, in epoch milliseconds.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<i64>,
  /// At most this many sessions, with every session issued at the last one's time beside it.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(range(min = 1))]
  pub limit: Option<u32>,
  /// The sessions this holder holds alone.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub holder: Option<String>,
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

/// Where a page of the event stream begins, which kinds it holds and how many events it answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventQuery {
  /// The events numbered after this one.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<i64>,
  /// The kinds answered, a comma-separated list; every kind where absent.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub kinds: Option<String>,
  /// At most this many events; every one held where absent.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(range(min = 1))]
  pub limit: Option<u32>,
}

/// What an event tells of the catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum EventKind {
  /// A tool served that was not, or in another placed version.
  #[serde(rename = "tool.placed")]
  ToolPlaced,
  /// A tool whose pointer or folder is gone.
  #[serde(rename = "tool.removed")]
  ToolRemoved,
}

impl EventKind {
  /// The kind as it stands on the wire, which is also its server-sent event's name.
  pub fn as_str(self) -> &'static str {
    match self {
      Self::ToolPlaced => "tool.placed",
      Self::ToolRemoved => "tool.removed",
    }
  }
}

/// One entry of a tool as the toolbook's list answers it: its full name, which calling tool reaches it, and its
/// description's first sentence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Booked {
  /// Its full name, `<namespace>/<name>`.
  pub tool: String,
  pub reached_by: ReachedBy,
  /// Its description's first sentence.
  pub summary: String,
}

/// The tool an event is of: its name, the namespaces of its entries, and each entry it serves a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolChange {
  /// Its name: its registration's, or its folder's under the tools root.
  pub tool: String,
  /// The namespaces its entries are shown under, in order of name.
  pub namespaces: Vec<String>,
  /// Each entry it serves a session, in order of full name; none for the system alone.
  pub entries: Vec<Booked>,
  /// Whether a placement took the place of another version of the tool the host served.
  pub replaced: bool,
}

/// One event of the stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
  /// Its number, which only grows.
  pub id: i64,
  pub kind: EventKind,
  /// When the host read the change, in epoch milliseconds.
  pub at: u64,
  pub data: ToolChange,
}

/// A page of the events held, in order of number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EventPage {
  pub items: Vec<Event>,
  pub more: bool,
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

/// The screens of the personal tools a session reaches (docs/api.md, 「Personal screens」), and whether the runner its
/// record names answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScreenPage {
  pub items: Vec<PersonalScreen>,
  /// Whether the owner's runner answered; the items are the host's copies and stand either way.
  pub reached: bool,
}

/// One personal tool's screen, as a console attaches it for its owner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersonalScreen {
  /// The personal tool's name.
  pub tool: String,
  /// The version the host's copy of it is placed as.
  pub version: String,
  /// The screen's bundle's name, which is its words' namespace.
  pub name: String,
  /// The path its routes stand under.
  pub screen_path: String,
  /// The key, in its words, of the name it is shown by.
  pub title: String,
  pub icon: String,
  /// The release of bmo-ui its bundle was built on.
  pub layer: String,
  /// Its words, each locale's tree under the locale's code.
  pub words: Map<String, Value>,
  /// The SHA-256 of its index's bytes, which every file's path stands under.
  pub digest: String,
  /// The path of its module on the app face.
  pub module: String,
  /// The path of its stylesheet, where its index holds one.
  pub stylesheet: Option<String>,
}

/// A call of one of a screen's own tool's entries, by the entry's name in the personal namespace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScreenCallRequest {
  pub name: String,
  /// The entry's own arguments.
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
  /// The full names of the only entries the session reaches; null where its record does not narrow them.
  pub entries: Option<BTreeSet<String>>,
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
      entries: session.entries.clone(),
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

/// What `GET /uses` narrows the kept asynchronous calls by: one state, and the agent a call's wake is for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UseQuery {
  /// The calls in this state alone: `running`, `done`, `refused`, `failed`, `cancelled` or `lost`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub state: Option<String>,
  /// The calls whose wake is for this agent alone.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
}

/// One asynchronous call the host keeps, as `GET /use/{id}` answers it: the request as it was asked, its state, and
/// once it ended, when, its summary and its wake.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeptCall {
  pub id: String,
  /// The record id of the session the call was asked under, never its token.
  pub session: String,
  /// The tool's full name, `<namespace>/<name>`.
  pub tool: String,
  pub reached_by: String,
  /// The tool's own arguments as they were asked.
  pub arguments: Map<String, Value>,
  pub memory: Option<String>,
  /// The room the wake is for.
  pub room: Option<i64>,
  /// The agent the wake is for.
  pub agent: Option<String>,
  pub via: Option<String>,
  /// When it started, in epoch milliseconds.
  pub started_at: i64,
  /// `running`, `done`, `refused`, `failed`, `cancelled` or `lost`.
  pub state: String,
  /// When it ended, in epoch milliseconds, once it has.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ended_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub summary: Option<String>,
  /// Its wake: the instruction it was queued as, or still owed.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub wake: Option<Value>,
}

/// Every asynchronous call the host keeps that the query names, oldest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsePage {
  pub items: Vec<KeptCall>,
}

/// A page of schedules, in order of when each is next due.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SchedulePage {
  pub items: Vec<Schedule>,
  pub more: bool,
}

/// A tool session that stands, as `GET /tool-sessions` answers it: its holder, its record's reach and its times,
/// never its token or a variable's value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StandingSession {
  /// The record's id: the SHA-256 of its token in hexadecimal, which names its record and never its token.
  pub id: String,
  pub holder_name: String,
  pub namespaces: BTreeSet<String>,
  pub capabilities: BTreeSet<String>,
  pub grants: Vec<Granted>,
  pub shown: BTreeSet<String>,
  /// The full names of the only entries it reaches; null where its record does not narrow them.
  pub entries: Option<BTreeSet<String>>,
  /// The names of the variables handed to its calls, in order of name; their values are never answered.
  pub variables: Vec<String>,
  pub labels: Map<String, Value>,
  /// When the session lapses, in epoch milliseconds; null where it stands until revoked.
  pub expires_at: Option<u64>,
  /// When it was issued, in epoch milliseconds.
  pub created_at: u64,
  /// Each server of the tools face its clients reached, in order of namespace.
  pub connected: Vec<Connection>,
}

/// A server of the tools face a session's clients reached: its namespace, `toolbook` for the toolbook, and when the
/// host first answered its `initialize` or `tools/list` under the session's token, in epoch milliseconds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Connection {
  pub namespace: String,
  pub at: u64,
}

impl StandingSession {
  /// What a record that stands is told by, with the servers its clients reached.
  pub fn of(id: &str, session: &Session, connected: Vec<Connection>) -> Self {
    Self {
      id: id.to_owned(),
      holder_name: session.holder_name.clone(),
      namespaces: session.namespaces.clone(),
      capabilities: session.capabilities.clone(),
      grants: session.grants.clone(),
      shown: session.shown.clone(),
      entries: session.entries.clone(),
      variables: session.variables.keys().cloned().collect(),
      labels: session.labels.clone(),
      expires_at: session.expires_at,
      created_at: session.created_at,
      connected,
    }
  }
}

/// A page of the tool sessions standing, in order of when each was issued.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionPage {
  pub items: Vec<StandingSession>,
  pub more: bool,
}

/// How the host reaches an external server: a command it starts, or an address it connects to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
  Stdio,
  Http,
}

/// One external server the host read a registration of, and whether its tools are listed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Server {
  pub name: String,
  pub namespace: String,
  pub transport: Transport,
  /// Where it is reached: the command's words joined by spaces, or the address.
  pub address: String,
  /// What a record must hold to be shown its tools.
  pub capabilities: Vec<String>,
  /// What reaches a tool whose annotations carry no read-only hint.
  pub reached_by: ReachedBy,
  /// How many tools its server last listed; null before it first answered.
  pub listed: Option<u32>,
  /// Why its last connection or listing failed, where it did.
  pub error: Option<String>,
}

/// A page of the external servers, in order of name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerPage {
  pub items: Vec<Server>,
  pub more: bool,
}
