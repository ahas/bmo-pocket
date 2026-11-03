//! One namespace's tools as the protocol serves them to a session: every placed entry the session holds, whole
//! or in a bundle's narrower form, listed by name where its record's `shown` names it and booked otherwise
//! (docs/mcp-host.md, 「Namespaces and capabilities」, 「Placed tools」; docs/toolbook.md). A call
//! reaches its tool by one dispatch, whether it came by the tool's own name, through the toolbook or on a
//! call-back socket.

use std::collections::BTreeSet;
use std::sync::Arc;

use bmo_pocket_client::{Listing, ReachedBy, Tool, ToolHints};
use bmo_tool::manifest::Entry as Declared;
use bmo_tool::protocol::Asked;
use bmo_tool::{Answer, Held, Hints, Session, narrowed};
use rmcp::model::{
  CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeRequestParams,
  InitializeResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
  ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, ServerCapabilities,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::drain::Drain;
use crate::placed::{Placed, PlacedTool};
use crate::records::Reach;
use crate::residency;
use crate::running::{ask, claim_answer, claimed, failed};

/// What every namespace's handler stands on.
pub struct Host {
  /// The placed tools' catalogue, and where their sockets are bound.
  pub placed: Arc<Placed>,
  /// The variables of the host's own environment a call's clean environment is handed beside its own.
  pub pass_env: Vec<String>,
  /// The host's stop, and what it waits on.
  pub drain: Drain,
}

/// One namespace as the protocol serves it, made for each request on its path.
#[derive(Clone)]
pub struct Handler {
  pub namespace: String,
  pub host: Arc<Host>,
}

/// One entry of a placed tool a session holds, in the form it holds it: whole, or narrowed to a bundle's
/// arguments, which acts under no grant.
#[derive(Clone)]
pub struct Entry {
  pub tool: Arc<PlacedTool>,
  at: usize,
  pub held: Held,
  /// The capability of the bundle it is held in, where it is held in one.
  bundle: Option<String>,
  /// Whether the record of the session holding it lists it by name.
  shown: bool,
}

impl Entry {
  /// Its declaration in its tool's manifest.
  pub fn declared(&self) -> &Declared {
    &self.tool.manifest.entries[self.at]
  }

  pub fn namespace(&self) -> &str {
    &self.declared().namespace
  }

  pub fn name(&self) -> &str {
    &self.declared().name
  }

  /// Its name with its namespace before it, as the toolbook names a tool: `<namespace>/<name>`.
  pub fn full_name(&self) -> String {
    format!("{}/{}", self.namespace(), self.name())
  }

  /// Whether its namespace lists it: where the record of the session holding it lists it; every other entry
  /// is booked.
  pub fn resident(&self) -> bool {
    self.shown
  }

  /// Whether it is for the system alone, never shown to a session.
  pub fn system(&self) -> bool {
    self.declared().system
  }

  pub fn hints(&self) -> Hints {
    self.declared().hints.into()
  }

  pub fn read_only(&self) -> bool {
    self.hints().read_only
  }

  /// Which of the toolbook's two calling tools reaches it: `call` where it only reads.
  pub fn reached_by(&self) -> ReachedBy {
    if self.read_only() { ReachedBy::Call } else { ReachedBy::Act }
  }

  fn title(&self) -> Option<String> {
    self.declared().title.clone()
  }

  /// The protocol's own description of it, in the form held.
  pub fn described(&self) -> Value {
    let hints = self.hints();
    let mut annotations = json!({
      "readOnlyHint": hints.read_only,
      "destructiveHint": hints.destructive,
      "openWorldHint": hints.open_world,
    });

    if let Some(idempotent) = hints.idempotent {
      annotations["idempotentHint"] = idempotent.into();
    }

    let mut said = json!({
      "name": self.name(),
      "description": self.description(),
      "inputSchema": self.input_schema(),
      "annotations": annotations,
    });

    if let Some(title) = self.title() {
      said["title"] = title.into();
    }

    if let Some(output) = self.output_schema() {
      said["outputSchema"] = output;
    }

    said
  }

  pub fn description(&self) -> String {
    self.declared().description.clone()
  }

  /// The text of the manual its manifest names, where it names one.
  pub fn manual(&self) -> Option<String> {
    self.declared().manual_text.clone()
  }

  pub fn input_schema(&self) -> Value {
    let declared = self.declared();

    match self.bundle.as_ref().and_then(|bundle| declared.bundle.get(bundle)) {
      Some(form) => narrowed(declared.input_schema.clone(), &form.shown, form.required.as_deref()),
      None => declared.input_schema.clone(),
    }
  }

  /// The schema of what it answers with, where it declares one.
  pub fn output_schema(&self) -> Option<Value> {
    self.declared().output_schema.clone()
  }

  /// What a session must hold of it: the entry's own capabilities, or the bundle's that shows it.
  pub fn capabilities(&self) -> Vec<String> {
    match self.held {
      Held::Bundle => self.bundle.clone().into_iter().collect(),
      Held::Whole => self.declared().capabilities.clone(),
    }
  }

  /// The resource a session's grants must hold for a call of it to run: a bundle's form acts under none.
  pub fn grant(&self) -> Option<String> {
    match self.held {
      Held::Bundle => None,
      Held::Whole => self.declared().grant.clone(),
    }
  }

  /// The icon its manifest declares for it.
  pub fn icon(&self) -> Option<String> {
    self.declared().facts.as_ref().and_then(|facts| facts.icon.clone())
  }

  /// The tool member the control socket answers for it, in the form held.
  pub fn member(&self) -> Tool {
    let hints = self.hints();

    Tool {
      namespace: self.namespace().to_owned(),
      name: self.name().to_owned(),
      title: self.title(),
      description: self.description(),
      input: self.input_schema(),
      output: self.output_schema(),
      icon: self.icon(),
      capabilities: self.capabilities(),
      grant: self.grant(),
      hints: ToolHints {
        read_only: hints.read_only,
        destructive: hints.destructive,
        idempotent: hints.idempotent,
        open_world: hints.open_world,
      },
      listing: self.declared().listing.as_ref().map(|listing| Listing {
        line: listing.line.clone(),
        order: listing.order,
      }),
      system: self.system(),
      manual: self.manual(),
      reached_by: self.reached_by(),
    }
  }
}

/// Why the host did not hand a call's answer on: the arguments do not fit, the session's grants do not hold the
/// entry's grant, or the tool failed before answering a tool result. A session's model reads each as a failed
/// tool result; the control socket answers each by its status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
  Misfit(String),
  Denied(String),
  Failed(String),
}

impl Refusal {
  /// The sentence it is said with.
  pub fn said(&self) -> &str {
    match self {
      Self::Misfit(said) | Self::Denied(said) | Self::Failed(said) => said,
    }
  }
}

// The form a session holds a placed entry in, and the bundle's capability where it is a bundle's: none for an
// entry for the system alone or outside the session's namespaces.
fn placed_held(declared: &Declared, session: &Session) -> Option<(Held, Option<String>)> {
  if declared.system || !session.uses(&declared.namespace) {
    return None;
  }

  if session.holds(&declared.capabilities) {
    return Some((Held::Whole, None));
  }

  declared
    .bundle
    .keys()
    .find(|bundle| session.holds(&[bundle]))
    .map(|bundle| (Held::Bundle, Some(bundle.clone())))
}

/// Who claims an address among the entries a session holds.
pub enum Claimant {
  /// An entry claims it under a grant the session holds, reading it with these arguments.
  Granted { entry: Entry, arguments: Value },
  /// An entry claims it, but under a grant the session lacks, said why.
  Ungranted { entry: Entry, why: String },
  /// No entry the session holds claims it.
  None,
}

/// Why the session may not call an entry: the grant it declares, where the session's grants do not hold it.
pub fn ungranted(entry: &Entry, session: &Session) -> Option<String> {
  let grant = entry.grant()?;

  (!session.granted(&grant)).then(|| format!("the session holds no grant for {grant}"))
}

/// The words a call of a tool the session is not shown is refused with, by its own name or through the
/// toolbook alike.
pub fn unknown(name: &str) -> String {
  format!("Tool {name} not found")
}

// The session the request was authorised as, which the authorisation in front of every namespace put on
// the request before it reached here.
pub(crate) fn session(context: &RequestContext<RoleServer>) -> Result<Arc<Session>, ErrorData> {
  context
    .extensions
    .get::<http::request::Parts>()
    .and_then(|parts| parts.extensions.get::<Arc<Session>>())
    .cloned()
    .ok_or_else(|| ErrorData::internal_error("the request carries no session", None))
}

// The server of this namespace, or the toolbook, marked reached under the request's record once its `initialize`
// or `tools/list` is answered (docs/api.md, 「A standing session」).
pub(crate) fn reached(context: &RequestContext<RoleServer>, namespace: &str) {
  if let Some(reach) = context
    .extensions
    .get::<http::request::Parts>()
    .and_then(|parts| parts.extensions.get::<Reach>())
  {
    reach.mark(namespace);
  }
}

// The `initialize` answer of a server as the protocol negotiates it, the server marked reached once it is.
pub(crate) fn initialized<S: ServerHandler>(
  server: &S,
  request: &InitializeRequestParams,
  context: &RequestContext<RoleServer>,
  namespace: &str,
) -> Result<InitializeResult, ErrorData> {
  context.peer.set_peer_info(request.clone());

  let answer = server.negotiate_initialize(request)?;

  reached(context, namespace);
  Ok(answer)
}

// A listing or a read as the session it was answered for may keep it: never fresh past its answer,
// since a deployment can change it at any call, and its own alone, since what a session is shown
// is its record's. A client on revision 2026-07-28 refuses a listing that says neither.
pub(crate) fn private(mut value: Value) -> Value {
  if let Some(result) = value.as_object_mut() {
    result.entry("ttlMs").or_insert_with(|| 0.into());
    result.entry("cacheScope").or_insert_with(|| "private".into());
  }

  value
}

// A result in the protocol's own type. Every result the host builds is a complete one, which a
// client on revision 2026-07-28 is told by `resultType` and refuses the result without; rmcp takes
// the field off again for a client on an earlier revision.
pub(crate) fn read<T: serde::de::DeserializeOwned>(mut value: Value) -> Result<T, ErrorData> {
  if let Some(result) = value.as_object_mut() {
    result.entry("resultType").or_insert_with(|| "complete".into());
  }

  serde_json::from_value(value)
    .map_err(|why| ErrorData::internal_error(format!("a result the protocol cannot carry: {why}"), None))
}

pub(crate) fn answered(answer: &Answer) -> Result<CallToolResponse, ErrorData> {
  response(serde_json::to_value(answer).map_err(|why| ErrorData::internal_error(why.to_string(), None))?)
}

// A tool result in the protocol's shape, as a call answers it.
fn response(value: Value) -> Result<CallToolResponse, ErrorData> {
  Ok(CallToolResponse::Complete(read::<CallToolResult>(value)?))
}

/// What does not fit in a call's arguments, by the tool's input schema: each argument that failed and
/// why. An argument given as null is read as one left out, as every tool's own reading reads it. A
/// schema the validator cannot read leaves the call to the tool's own check.
pub fn misfits(schema: &Value, arguments: &Value) -> Vec<String> {
  let Ok(validator) = jsonschema::validator_for(schema) else {
    return Vec::new();
  };
  let given = match arguments {
    Value::Object(fields) => Value::Object(
      fields
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect(),
    ),
    other => other.clone(),
  };

  validator
    .iter_errors(&given)
    .map(|error| {
      let at = error.instance_path().to_string();

      if at.is_empty() { error.to_string() } else { format!("{at}: {error}") }
    })
    .collect()
}

/// A placed tool's answer to a call, held to the protocol's shape: one that does not read as a tool result
/// is a failed call, said why.
pub fn checked_result(value: Value) -> Result<Value, String> {
  let mut checked = value.clone();

  if let Some(result) = checked.as_object_mut() {
    result.entry("resultType").or_insert_with(|| "complete".into());
  }

  match serde_json::from_value::<CallToolResult>(checked) {
    Ok(_) => Ok(value),
    Err(why) => Err(format!("its answer is not a tool result: {why}")),
  }
}

/// A placed tool's answer to a call as a session reads it: one that does not read as a tool result is a failed
/// call.
pub fn as_result(value: Value) -> Value {
  checked_result(value).unwrap_or_else(failed)
}

impl Host {
  /// Every entry the host serves, each whole, those for the system alone among them.
  pub fn every(&self) -> Vec<Entry> {
    let catalogue = self.placed.current();
    let mut every = Vec::new();

    for tool in &catalogue.tools {
      for at in 0..tool.manifest.entries.len() {
        every.push(Entry {
          tool: tool.clone(),
          at,
          held: Held::Whole,
          bundle: None,
          shown: false,
        });
      }
    }

    every.sort_by_key(|entry| (entry.namespace().to_owned(), entry.name().to_owned()));
    every
  }

  /// Every tool the session holds, in every namespace, by namespace and name: each whole where the session
  /// holds its every capability, and otherwise in its bundle's form where it holds that bundle.
  pub fn held(&self, session: &Session) -> Vec<Entry> {
    let catalogue = self.placed.current();
    let mut held = Vec::new();

    for tool in &catalogue.tools {
      for (at, declared) in tool.manifest.entries.iter().enumerate() {
        if let Some((form, bundle)) = placed_held(declared, session) {
          held.push(Entry {
            tool: tool.clone(),
            at,
            held: form,
            bundle,
            shown: residency::shows(session, &declared.full_name()),
          });
        }
      }
    }

    held.sort_by_key(|entry| (entry.namespace().to_owned(), entry.name().to_owned()));
    held
  }

  /// The tool of this namespace and name the session holds, resident or booked.
  pub fn find(&self, session: &Session, namespace: &str, name: &str) -> Option<Entry> {
    self
      .held(session)
      .into_iter()
      .find(|entry| entry.namespace() == namespace && entry.name() == name)
  }

  /// The entry a full name names among those the session holds, or, where `system` says so, among the
  /// entries for the system alone of the session's namespaces too, which the system and other tools reach.
  pub fn find_full(&self, session: &Session, full: &str, system: bool) -> Option<Entry> {
    let (namespace, name) = full.split_once('/')?;

    self.find(session, namespace, name).or_else(|| {
      (system && session.uses(namespace))
        .then(|| self.every().into_iter().find(|entry| entry.system() && entry.full_name() == full))
        .flatten()
    })
  }

  /// Every namespace a placed tool declares an entry or its resources in.
  pub fn namespaces(&self) -> BTreeSet<String> {
    self.placed.current().namespaces()
  }

  /// One call of a tool the session holds: its arguments checked against its schema in the form held, its grant
  /// against the session's grants, then the tool, whose tool result goes back unchanged; or why the host did not
  /// hand one on.
  pub async fn attempt(
    self: &Arc<Self>,
    entry: &Entry,
    session: &Session,
    arguments: Value,
  ) -> Result<Value, Refusal> {
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    let schema = entry.input_schema();
    let wrong = misfits(&schema, &arguments);

    if !wrong.is_empty() {
      return Err(Refusal::Misfit(format!(
        "{} was not called: its arguments do not fit its schema.\n{}\nIts schema: {schema}",
        entry.full_name(),
        wrong.iter().map(|one| format!("- {one}")).collect::<Vec<_>>().join("\n"),
      )));
    }

    if let Some(why) = ungranted(entry, session) {
      return Err(Refusal::Denied(format!("{} was not called: {why}", entry.full_name())));
    }

    let asked = Asked::Call {
      entry: entry.full_name(),
      arguments,
    };

    match ask(self, &entry.tool, session, asked).await {
      Ok(value) => checked_result(value).map_err(Refusal::Failed),
      Err(why) => Err(Refusal::Failed(format!("{} failed: {why}", entry.full_name()))),
    }
  }

  /// One call of a tool the session holds, answered as a tool result, the host's refusal among them as a failed
  /// one.
  pub async fn answer(
    self: &Arc<Self>,
    entry: &Entry,
    session: &Session,
    arguments: Value,
  ) -> Result<Value, ErrorData> {
    Ok(
      self
        .attempt(entry, session, arguments)
        .await
        .unwrap_or_else(|refusal| failed(refusal.said())),
    )
  }

  /// One call of a tool the session holds, as the protocol answers it.
  pub async fn dispatch(
    self: &Arc<Self>,
    entry: &Entry,
    session: &Session,
    arguments: Value,
  ) -> Result<CallToolResponse, ErrorData> {
    response(self.answer(entry, session, arguments).await?)
  }

  /// Which entry the session holds claims an address, and the arguments it reads that page with: each entry
  /// claiming the address's host whose grant the session holds asked, one after another, the first that claims
  /// it answering.
  pub async fn claim(self: &Arc<Self>, session: &Session, address: &str) -> Value {
    match self.claimant(session, address).await {
      Claimant::Granted { entry, arguments } => claim_answer(Some((entry.full_name(), arguments))),
      Claimant::Ungranted { .. } | Claimant::None => claim_answer(None),
    }
  }

  /// Who claims an address among the entries the session holds: each entry claiming the address's host asked, one
  /// after another, the first that claims it under a grant the session holds answering; or the first that claims
  /// it under a grant the session lacks, where none does; or none.
  pub async fn claimant(self: &Arc<Self>, session: &Session, address: &str) -> Claimant {
    let mut lacking = Claimant::None;

    for entry in self.held(session) {
      let declared = entry.declared();

      if !declared.claims(address) {
        continue;
      }

      let asked = Asked::Claim {
        entry: entry.full_name(),
        address: address.to_owned(),
      };

      match ask(self, &entry.tool, session, asked).await {
        Ok(answer) => {
          let Some(arguments) = claimed(&answer) else {
            continue;
          };

          match ungranted(&entry, session) {
            None => return Claimant::Granted { entry, arguments },
            Some(why) if matches!(lacking, Claimant::None) => lacking = Claimant::Ungranted { entry, why },
            Some(_) => {}
          }
        }
        Err(why) => eprintln!("mcp-host: {} could not be asked for {address}: {why}", entry.full_name()),
      }
    }

    lacking
  }

  // The placed tools serving resources under a namespace to this session.
  fn serving(&self, session: &Session, namespace: &str) -> Vec<Arc<PlacedTool>> {
    self
      .placed
      .current()
      .tools
      .iter()
      .filter(|tool| {
        tool
          .manifest
          .resources
          .as_ref()
          .is_some_and(|served| served.namespace == namespace && session.holds(&served.capabilities))
      })
      .cloned()
      .collect()
  }

  /// The resources a namespace lists this session: each placed tool's serving it.
  pub async fn list_resources(self: &Arc<Self>, session: &Session, namespace: &str) -> Vec<Value> {
    let mut listed = Vec::new();

    for tool in self.serving(session, namespace) {
      match ask(self, &tool, session, Asked::ResourcesList).await {
        Ok(answer) => listed.extend(answer["resources"].as_array().cloned().unwrap_or_default()),
        Err(why) => eprintln!("mcp-host: tools/{} could not list its resources: {why}", tool.folder),
      }
    }

    listed
  }

  /// One resource read for this session by the first placed tool that holds it; refused with the reason the
  /// last one asked gave, or as not found where none gave one.
  pub async fn read_resource(
    self: &Arc<Self>,
    session: &Session,
    namespace: &str,
    uri: &str,
  ) -> Result<Value, String> {
    let mut why = format!("Resource {uri} not found");

    for tool in self.serving(session, namespace) {
      let asked = Asked::ResourcesRead {
        uri: uri.to_owned(),
      };

      match ask(self, &tool, session, asked).await {
        Ok(answer) if answer.get("contents").is_some_and(Value::is_array) => return Ok(answer),
        Ok(answer) => {
          if let Some(said) = answer.get("error").and_then(Value::as_str) {
            said.clone_into(&mut why);
          }
        }
        Err(failed) => eprintln!("mcp-host: tools/{} could not read {uri}: {failed}", tool.folder),
      }
    }

    Err(why)
  }

  /// What a namespace says of itself, of the entries of it a token sees: each entry's verb, record and icon in the
  /// sending verbs, records and icons, each item it adds to a list of the facts, headed with its name, and the
  /// digest of every one of them.
  pub fn facts(&self, held: &[Entry]) -> serde_json::Map<String, Value> {
    let placed: Vec<(String, bmo_tool::manifest::FactsPart)> = held
      .iter()
      .filter_map(|entry| Some((entry.name().to_owned(), entry.declared().facts.clone()?)))
      .collect();
    let mut facts = json!({ "digest": digest(held) });

    for (name, said) in placed {
      for (list, item) in said.lists {
        let mut added = serde_json::Map::from_iter([("name".to_owned(), json!(name))]);

        added.extend(item);

        if let Some(items) = facts
          .as_object_mut()
          .and_then(|object| object.entry(list).or_insert_with(|| json!([])).as_array_mut())
        {
          items.push(Value::Object(added));
        }
      }

      for (list, field, value) in [
        ("sending", "verb", said.verb),
        ("records", "record", said.record),
        ("icons", "icon", said.icon),
      ] {
        if let (Some(value), Some(object)) = (value, facts.as_object_mut()) {
          let added = json!({ "name": name, field: value });

          match object.entry(list).or_insert_with(|| json!([])).as_array_mut() {
            Some(items) => items.push(added),
            None => continue,
          }
        }
      }
    }

    match facts {
      Value::Object(facts) => facts,
      _ => serde_json::Map::new(),
    }
  }
}

/// The digest of what a namespace shows a session holding it whole: every entry's name, description, schemas
/// and hints, in name order. A line's configuration names the namespace and not its tools, so this is what
/// moves its prefix when an entry is added or a schema changes.
fn digest(entries: &[Entry]) -> String {
  let mut shown: Vec<(String, Value)> = entries
    .iter()
    .map(|entry| {
      let hints = entry.hints();

      (
        entry.name().to_owned(),
        json!({
          "name": entry.name(),
          "description": entry.description(),
          "input": entry.input_schema(),
          "output": entry.output_schema(),
          "hints": [hints.read_only, hints.destructive, hints.open_world],
        }),
      )
    })
    .collect();

  shown.sort_by(|left, right| left.0.cmp(&right.0));

  let shown = Value::Array(shown.into_iter().map(|(_, entry)| entry).collect());

  format!("{:x}", Sha256::digest(shown.to_string().as_bytes()))
}

impl Handler {
  // The tools the namespace lists: the resident ones the session holds in it. A booked one is listed by
  // the toolbook instead.
  fn tools(&self, session: &Session) -> Vec<Value> {
    self
      .host
      .held(session)
      .iter()
      .filter(|entry| entry.namespace() == self.namespace && entry.resident())
      .map(Entry::described)
      .collect()
  }
}

impl ServerHandler for Handler {
  fn get_info(&self) -> InitializeResult {
    let mut info =
      InitializeResult::new(ServerCapabilities::builder().enable_tools().enable_resources().build());
    let mut server = Implementation::default();

    server.name = self.namespace.clone();
    server.version = env!("CARGO_PKG_VERSION").to_owned();
    info.server_info = server;
    info
  }

  async fn initialize(
    &self,
    request: InitializeRequestParams,
    context: RequestContext<RoleServer>,
  ) -> Result<InitializeResult, ErrorData> {
    initialized(self, &request, &context, &self.namespace)
  }

  async fn list_tools(
    &self,
    _: Option<PaginatedRequestParams>,
    context: RequestContext<RoleServer>,
  ) -> Result<ListToolsResult, ErrorData> {
    let session = session(&context)?;
    let listed = read(private(json!({ "tools": self.tools(&session) })))?;

    reached(&context, &self.namespace);
    Ok(listed)
  }

  // A call by the tool's own name. A booked tool the session holds is answered too, so a session whose
  // program listed its tools before the tool was booked goes on calling it by that name.
  async fn call_tool(
    &self,
    request: CallToolRequestParams,
    context: RequestContext<RoleServer>,
  ) -> Result<CallToolResponse, ErrorData> {
    let session = session(&context)?;
    let arguments = request.arguments.map_or(Value::Null, Value::Object);
    let Some(entry) = self.host.find(&session, &self.namespace, &request.name) else {
      return Err(ErrorData::invalid_params(unknown(&request.name), None));
    };

    self.host.dispatch(&entry, &session, arguments).await
  }

  // The documents the namespace serves the session, where it serves any and the session holds what they ask.
  async fn list_resources(
    &self,
    _: Option<PaginatedRequestParams>,
    context: RequestContext<RoleServer>,
  ) -> Result<ListResourcesResult, ErrorData> {
    let session = session(&context)?;
    let listed = self.host.list_resources(&session, &self.namespace).await;

    read(private(json!({ "resources": listed })))
  }

  async fn read_resource(
    &self,
    request: ReadResourceRequestParams,
    context: RequestContext<RoleServer>,
  ) -> Result<ReadResourceResponse, ErrorData> {
    let session = session(&context)?;

    match self.host.read_resource(&session, &self.namespace, &request.uri).await {
      Ok(contents) => Ok(ReadResourceResponse::Complete(read::<ReadResourceResult>(private(contents))?)),
      Err(why) => Err(ErrorData::resource_not_found(why, None)),
    }
  }
}
