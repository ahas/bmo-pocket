//! The host's faces: the namespaces sessions call on the loopback port, each behind a tool session's bearer token;
//! the control socket, where every route takes the issuer token or a tool session's; and the console face, where
//! the console token reaches the screen's bundle and the control socket's reads (docs/mcp-host.md, 「Tokens」;
//! docs/api.md, 「The control socket」 and 「The console face」).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use bmo_console_face::ConsoleFace;
use bmo_pocket_client::{
  CallInHand, Claim, ClaimRequest, Event, EventPage, EventQuery, Health, NameQuery, Namespace,
  NamespacePage, Schedule, SchedulePage, Server, ServerPage, SessionPage, SessionQuery, StandingSession,
  TimeQuery, Tool, ToolCall, ToolCallRequest, ToolPage, ToolResult, ToolSession, UseQuery,
};
use bmo_tool::{Session, SessionRequest};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;

use crate::async_calls::{self, Asked, AsyncCalls};
use crate::error::HostError;
use crate::events::Events;
use crate::handler::{Claimant, Entry, Handler, Host, Refusal};
use crate::records::{Reach, Records};
use crate::schedule::{Schedules, now_ms};
use crate::toolbook::{TOOLBOOK, Toolbook};

// The token a request carries, as `Authorization: Bearer <token>`.
fn bearer(request: &Request) -> Option<&str> {
  let value = request.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
  let (scheme, token) = value.split_once(' ')?;

  scheme
    .eq_ignore_ascii_case("bearer")
    .then(|| token.trim())
    .filter(|token| !token.is_empty())
}

// Every namespace stands behind this: the token names a record, and the record goes on with the
// request for the handler to serve by, with what its servers are marked reached by.
async fn authorise(State(records): State<Arc<Records>>, mut request: Request, next: Next) -> Response {
  let Some(token) = bearer(&request) else {
    return HostError::NoToken.into_response();
  };
  let Some((id, session)) = records.get(token) else {
    return HostError::UnknownToken.into_response();
  };

  request.extensions_mut().insert::<Arc<Session>>(session);
  request.extensions_mut().insert(Reach {
    records: records.clone(),
    id,
  });
  next.run(request).await
}

async fn unknown_namespace(uri: Uri) -> HostError {
  HostError::UnknownNamespace(uri.path().trim_start_matches("/mcp/").to_owned())
}

// How every server the host serves speaks: stateless, each answer one JSON body.
fn stateless() -> StreamableHttpServerConfig {
  let mut config = StreamableHttpServerConfig::default();

  config.legacy_session_mode = false;
  config.json_response = true;
  config.sse_keep_alive = None;
  config
}

// One request on a namespace's path, served where a placed manifest declares that namespace as the catalogue stands
// at the request and the session's record names it, and answered 404 otherwise.
async fn namespace(
  State(tools): State<Tools>,
  Path(namespace): Path<String>,
  request: Request,
) -> Response {
  let named = request
    .extensions()
    .get::<Arc<Session>>()
    .is_some_and(|session| session.uses(&namespace));

  if !named || !tools.host.namespaces().contains(&namespace) {
    return HostError::UnknownNamespace(namespace).into_response();
  }

  let host = tools.host.clone();
  let service = StreamableHttpService::new(
    move || {
      Ok(Handler {
        namespace: namespace.clone(),
        host: host.clone(),
      })
    },
    Arc::new(NeverSessionManager::default()),
    stateless(),
  );

  service.handle(request).await.map(Body::new)
}

/// What the tools face stands on: the host it asks, and the calls it keeps for its sessions.
#[derive(Clone)]
pub struct Tools {
  host: Arc<Host>,
  calls: Arc<AsyncCalls>,
}

/// The namespaces, each at `/mcp/<namespace>`, the toolbook at `/mcp/toolbook`, and the asynchronous calls at
/// `/use`, served as streamable HTTP with no session kept: every request stands alone and is known by its token.
pub fn tools(records: Arc<Records>, host: Arc<Host>, calls: Arc<AsyncCalls>) -> Router {
  let book = host.clone();
  let toolbook = StreamableHttpService::new(
    move || Ok(Toolbook { host: book.clone() }),
    Arc::new(NeverSessionManager::default()),
    stateless(),
  );

  Router::new()
    .route_service(&format!("/mcp/{TOOLBOOK}"), toolbook)
    .route("/mcp/{namespace}", any(namespace))
    .route("/use", post(start_call).get(list_calls))
    .route("/use/{id}", get(one_call))
    .with_state(Tools { host, calls })
    .fallback(unknown_namespace)
    .layer(middleware::from_fn_with_state(records, authorise))
}

// A session's call asked to be run by the host: its id and state answered once it is under way.
async fn start_call(
  State(tools): State<Tools>,
  Extension(reach): Extension<Reach>,
  Extension(session): Extension<Arc<Session>>,
  body: Bytes,
) -> Result<Json<Value>, HostError> {
  let asked: Asked =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRequest(why.to_string()))?;

  Ok(Json(tools.calls.start(tools.host.clone(), session, reach.id.0.clone(), asked).await?))
}

// Every call the token's session keeps, oldest first.
async fn list_calls(State(tools): State<Tools>, Extension(reach): Extension<Reach>) -> Json<Value> {
  let calls: Vec<Value> =
    tools.calls.list().into_iter().filter(|call| held_by(call, &reach.id.0)).collect();

  Json(async_calls::page(calls))
}

// One call the token's session keeps, as it stands.
async fn one_call(
  State(tools): State<Tools>,
  Extension(reach): Extension<Reach>,
  Path(id): Path<String>,
) -> Result<Json<Value>, HostError> {
  match tools.calls.one(&id).filter(|call| held_by(call, &reach.id.0)) {
    Some(call) => Ok(Json(call)),
    None => Err(HostError::UnknownCall(id)),
  }
}

// Whether a call was asked under the session the request's token names: the same session alone reads it back.
fn held_by(call: &Value, session: &str) -> bool {
  call["session"].as_str() == Some(session)
}

// What the control socket's routes and the console face's reads stand on: the records, the host it asks for the
// catalogue, and the schedules.
#[derive(Clone)]
struct Control {
  records: Arc<Records>,
  host: Arc<Host>,
  schedules: Arc<Schedules>,
  calls: Arc<AsyncCalls>,
}

// Every asynchronous call the host keeps, for the issuer alone: across every session, oldest first, each as one
// call is answered, narrowed to a state and to the agent a call's wake is for where the query names them.
async fn use_page(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  asked: Result<Query<UseQuery>, QueryRejection>,
) -> Result<Json<Value>, HostError> {
  caller.issuer()?;

  let Query(asked) = asked.map_err(malformed)?;
  let named = |call: &Value, field: &str, wanted: &Option<String>| {
    wanted.as_deref().is_none_or(|wanted| call[field].as_str() == Some(wanted))
  };
  let calls: Vec<Value> = control
    .calls
    .list()
    .into_iter()
    .filter(|call| named(call, "state", &asked.state) && named(call, "agent", &asked.agent))
    .collect();

  Ok(Json(async_calls::page(calls)))
}

/// Who a control request was made by, as its token says: the issuer, or a tool session.
#[derive(Clone)]
pub enum Caller {
  Issuer,
  Session(Arc<Session>),
}

impl Caller {
  // Every entry the token sees: the issuer every entry, those for the system alone among them, and a session the
  // entries its record holds.
  fn entries(&self, host: &Host) -> Vec<Entry> {
    match self {
      Self::Issuer => host.every(),
      Self::Session(session) => host.held(session),
    }
  }

  fn session(&self) -> Result<&Arc<Session>, HostError> {
    match self {
      Self::Session(session) => Ok(session),
      Self::Issuer => Err(HostError::SessionOnly),
    }
  }

  fn issuer(&self) -> Result<(), HostError> {
    match self {
      Self::Issuer => Ok(()),
      Self::Session(_) => Err(HostError::IssuerOnly),
    }
  }
}

// Every control route stands behind this: the token is the issuer's or names a tool session, and who made the
// request goes on with it.
async fn authorise_control(
  State((records, issuer)): State<(Arc<Records>, Arc<str>)>,
  mut request: Request,
  next: Next,
) -> Response {
  let Some(token) = bearer(&request) else {
    return HostError::NoToken.into_response();
  };
  let caller = if token == &*issuer {
    Caller::Issuer
  } else {
    match records.get(token) {
      Some((_, session)) => Caller::Session(session),
      None => return HostError::UnknownToken.into_response(),
    }
  };

  request.extensions_mut().insert(caller);
  next.run(request).await
}

fn malformed(why: QueryRejection) -> HostError {
  HostError::MalformedRequest(why.body_text())
}

/// A page of members keyed by name: those whose names sort after the cursor, in order of name, at most `limit` of
/// them, and whether more stand past the last; refused for a limit of none.
pub fn page_by_name<T>(
  mut every: Vec<T>,
  name: impl Fn(&T) -> String,
  after: Option<&str>,
  limit: Option<u32>,
) -> Result<(Vec<T>, bool), String> {
  if limit == Some(0) {
    return Err("`limit` is a positive integer".to_owned());
  }

  every.sort_by_key(&name);

  if let Some(after) = after {
    every.retain(|one| name(one).as_str() > after);
  }

  let more = limit.is_some_and(|limit| every.len() > limit as usize);

  every.truncate(limit.map_or(usize::MAX, |limit| limit as usize));
  Ok((every, more))
}

/// A page of members keyed by time, already in the order they are answered in: those due after the cursor, at
/// most `limit` of them and every one due at the last one's time beside it, so no run of equal times is split, and
/// whether more stand past the last; refused for a limit of none.
pub fn page_by_time<T>(
  mut every: Vec<T>,
  at: impl Fn(&T) -> i64,
  after: Option<i64>,
  limit: Option<u32>,
) -> Result<(Vec<T>, bool), String> {
  if limit == Some(0) {
    return Err("`limit` is a positive integer".to_owned());
  }

  if let Some(after) = after {
    every.retain(|one| at(one) > after);
  }

  let Some(limit) = limit.map(|limit| limit as usize).filter(|limit| *limit < every.len()) else {
    return Ok((every, false));
  };
  let last = at(&every[limit - 1]);
  let end = every[limit..]
    .iter()
    .position(|one| at(one) != last)
    .map_or(every.len(), |run| limit + run);
  let more = end < every.len();

  every.truncate(end);
  Ok((every, more))
}

// The entries of every namespace the token sees, by namespace.
fn by_namespace(entries: Vec<Entry>) -> BTreeMap<String, Vec<Entry>> {
  let mut grouped: BTreeMap<String, Vec<Entry>> = BTreeMap::new();

  for entry in entries {
    grouped.entry(entry.namespace().to_owned()).or_default().push(entry);
  }

  grouped
}

impl Control {
  // A namespace the token sees, of its entries.
  fn namespace(&self, name: String, held: &[Entry]) -> Namespace {
    Namespace {
      facts: self.host.facts(held),
      tool_count: u32::try_from(held.len()).unwrap_or(u32::MAX),
      name,
    }
  }

  // The entries of one namespace the token sees; refused where it sees none.
  fn held_in(&self, caller: &Caller, namespace: &str) -> Result<Vec<Entry>, HostError> {
    let held: Vec<Entry> = caller
      .entries(&self.host)
      .into_iter()
      .filter(|entry| entry.namespace() == namespace)
      .collect();

    if held.is_empty() {
      return Err(HostError::UnknownNamespace(namespace.to_owned()));
    }

    Ok(held)
  }
}

async fn health(State(state): State<Control>) -> Json<Health> {
  let in_hand: Vec<CallInHand> = state
    .host
    .drain
    .in_hand()
    .into_iter()
    .map(|call| CallInHand {
      entry: call.entry,
      holder: call.holder,
      started_at: call.started_at,
    })
    .collect();

  Json(Health {
    ready: true,
    reason: None,
    version: env!("CARGO_PKG_VERSION").to_owned(),
    calls_in_hand: in_hand.len() as u64,
    in_hand,
  })
}

/// The paragraph on finding and using tools a session's system prompt carries (docs/toolbook.md, 「The rules a
/// prompt carries」), built into the host so the text served is the version's own.
pub const DISCIPLINE: &str = include_str!("../../prompts/tool-use.md");

async fn discipline() -> Response {
  ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], DISCIPLINE).into_response()
}

async fn namespaces(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  asked: Result<Query<NameQuery>, QueryRejection>,
) -> Result<Json<NamespacePage>, HostError> {
  let Query(asked) = asked.map_err(malformed)?;
  let grouped: Vec<(String, Vec<Entry>)> =
    by_namespace(caller.entries(&control.host)).into_iter().collect();
  let (page, more) = page_by_name(grouped, |(name, _)| name.clone(), asked.after.as_deref(), asked.limit)
    .map_err(HostError::MalformedRequest)?;
  let items = page.into_iter().map(|(name, held)| control.namespace(name, &held)).collect();

  Ok(Json(NamespacePage { items, more }))
}

async fn one_namespace(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  Path(namespace): Path<String>,
) -> Result<Json<Namespace>, HostError> {
  let held = control.held_in(&caller, &namespace)?;

  Ok(Json(control.namespace(namespace, &held)))
}

async fn tool_page(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  Path(namespace): Path<String>,
  asked: Result<Query<NameQuery>, QueryRejection>,
) -> Result<Json<ToolPage>, HostError> {
  let Query(asked) = asked.map_err(malformed)?;
  let members: Vec<Tool> = control.held_in(&caller, &namespace)?.iter().map(Entry::member).collect();
  let (items, more) = page_by_name(members, |tool| tool.name.clone(), asked.after.as_deref(), asked.limit)
    .map_err(HostError::MalformedRequest)?;

  Ok(Json(ToolPage { items, more }))
}

async fn one_tool(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<Tool>, HostError> {
  caller
    .entries(&control.host)
    .iter()
    .find(|entry| entry.namespace() == namespace && entry.name() == name)
    .map(|entry| Json(entry.member()))
    .ok_or(HostError::UnknownEntry(format!("{namespace}/{name}")))
}

// A call under the token's record, as a session's call is dispatched, entries for the system alone of its
// namespaces included; each refusal of the host's answered by its status, and the tool's own result as it answered
// it.
async fn tool_call(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  body: Bytes,
) -> Result<Json<ToolCall>, HostError> {
  let session = caller.session()?;
  let called: ToolCallRequest =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRequest(why.to_string()))?;
  let full = format!("{}/{}", called.namespace, called.name);
  let Some(found) = control.host.find_full(session, &full, true) else {
    return Err(HostError::UnknownEntry(full));
  };
  let started = Instant::now();
  let answer = control.host.attempt(&found, session, Value::Object(called.input)).await;
  let spent_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

  let result = match answer {
    Ok(result) => result,
    Err(Refusal::Misfit(why)) => return Err(HostError::ArgumentsMisfit(why)),
    Err(Refusal::Denied(why)) => return Err(HostError::GrantRefused(why)),
    Err(Refusal::Failed(why) | Refusal::Cancelled(why)) => return Err(HostError::ToolFailed(why)),
  };
  let result: ToolResult = serde_json::from_value(result)
    .map_err(|why| HostError::ToolFailed(format!("{full} answered what is not a tool result: {why}")))?;

  Ok(Json(ToolCall {
    namespace: called.namespace,
    name: called.name,
    result,
    spent_ms,
  }))
}

// Whether the token's session may have an address read, and by which entry, as the call-back socket's claim is
// answered.
async fn claim(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  body: Bytes,
) -> Result<Json<Claim>, HostError> {
  let session = caller.session()?;
  let asked: ClaimRequest =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRequest(why.to_string()))?;
  let refused = |reason: String| Claim {
    resource: asked.resource.clone(),
    granted: false,
    reason: Some(reason),
    namespace: None,
    tool_name: None,
    arguments: None,
  };

  Ok(Json(match control.host.claimant(session, &asked.resource).await {
    Claimant::Granted { entry, arguments } => Claim {
      resource: asked.resource.clone(),
      granted: true,
      reason: None,
      namespace: Some(entry.namespace().to_owned()),
      tool_name: Some(entry.name().to_owned()),
      arguments: Some(arguments),
    },
    Claimant::Ungranted { entry, why } => refused(format!("{} claims it, but {why}", entry.full_name())),
    Claimant::None => refused("no tool the session holds claims it".to_owned()),
  }))
}

async fn issue(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  body: Bytes,
) -> Result<(StatusCode, Json<ToolSession>), HostError> {
  caller.issuer()?;

  let asked: SessionRequest = serde_json::from_slice(&body).map_err(|why| {
    eprintln!("mcp-host: a session record was refused: {why}");
    HostError::MalformedRecord(why.to_string())
  })?;
  let session = asked.issued_at(crate::records::now_ms());
  let token = control
    .records
    .issue(session.clone())
    .map_err(|why| HostError::Unwritable(why.to_string()))?;

  Ok((StatusCode::CREATED, Json(ToolSession::issued(token, &session))))
}

async fn revoke(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  Path(token): Path<String>,
) -> Result<StatusCode, HostError> {
  caller.issuer()?;

  let (_, gone) = control.records.revoke(&token).map_err(|why| HostError::Unwritable(why.to_string()))?;
  let gone = gone.ok_or(HostError::UnknownSession)?;

  gone.kept.close().await;
  Ok(StatusCode::NO_CONTENT)
}

// A tool session revoked by its record's id, to the issuer alone: the way back for a session whose token is lost.
async fn revoke_record(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  Path(id): Path<String>,
) -> Result<StatusCode, HostError> {
  caller.issuer()?;

  let gone = control.records.revoke_id(&id).map_err(|why| HostError::Unwritable(why.to_string()))?;
  let gone = gone.ok_or(HostError::UnknownSession)?;

  gone.kept.close().await;
  Ok(StatusCode::NO_CONTENT)
}

// The tool sessions standing, to the issuer alone, told by their ids, holders, records and times; a holder's alone
// where one is named.
async fn tool_session_page(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  asked: Result<Query<SessionQuery>, QueryRejection>,
) -> Result<Json<SessionPage>, HostError> {
  caller.issuer()?;

  let Query(asked) = asked.map_err(malformed)?;
  let mut every: Vec<StandingSession> = control
    .records
    .standing()
    .iter()
    .filter(|(_, session)| asked.holder.as_ref().is_none_or(|holder| &session.holder_name == holder))
    .map(|(id, session)| StandingSession::of(&id.0, session, control.records.connected(id)))
    .collect();

  every.sort_by(|left, right| {
    (left.created_at, &left.holder_name).cmp(&(right.created_at, &right.holder_name))
  });

  let issued = |one: &StandingSession| i64::try_from(one.created_at).unwrap_or(i64::MAX);
  let (items, more) =
    page_by_time(every, issued, asked.after, asked.limit).map_err(HostError::MalformedRequest)?;

  Ok(Json(SessionPage { items, more }))
}

// One standing tool session by its token, to the issuer alone, with the servers its clients reached.
async fn one_tool_session(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  Path(token): Path<String>,
) -> Result<Json<StandingSession>, HostError> {
  caller.issuer()?;

  let (id, session) = control.records.get(&token).ok_or(HostError::UnknownSession)?;

  Ok(Json(StandingSession::of(&id.0, &session, control.records.connected(&id))))
}

/// How often a stream with nothing to say says it is open.
const PING: Duration = Duration::from_secs(15);

// One event as its frame of the stream: its number, its kind as the event's name, and the event as JSON.
fn frame(event: &Event) -> Bytes {
  let data = serde_json::to_string(event).unwrap_or_default();

  Bytes::from(format!("id: {}\nevent: {}\ndata: {data}\n\n", event.id, event.kind.as_str()))
}

// Where a stream stands: the last number it went past, what it holds to write, and which kinds it writes.
struct Reading {
  host: Arc<Host>,
  cursor: i64,
  pending: VecDeque<Event>,
  kinds: Option<BTreeSet<String>>,
  latest: tokio::sync::watch::Receiver<i64>,
  stopping: tokio::sync::watch::Receiver<bool>,
}

fn wanted(kinds: Option<&BTreeSet<String>>, event: &Event) -> bool {
  kinds.is_none_or(|kinds| kinds.contains(event.kind.as_str()))
}

impl Reading {
  // The next frame: an event held past the cursor, a ping after a quiet while, or none once the host stops or the
  // reader fell behind the window, which its next connection is told.
  async fn next(mut self) -> Option<(Result<Bytes, std::convert::Infallible>, Self)> {
    loop {
      if let Some(event) = self.pending.pop_front() {
        self.cursor = event.id;

        if wanted(self.kinds.as_ref(), &event) {
          return Some((Ok(frame(&event)), self));
        }

        continue;
      }

      if *self.stopping.borrow() {
        return None;
      }

      self.latest.borrow_and_update();

      match self.host.placed.events.after(Some(self.cursor)) {
        Ok(found) if !found.is_empty() => {
          self.pending = found.into();
          continue;
        }
        Ok(_) => {}
        Err(_) => return None,
      }

      tokio::select! {
        changed = self.latest.changed() => {
          if changed.is_err() {
            return None;
          }
        }
        _ = self.stopping.changed() => {}
        () = tokio::time::sleep(PING) => return Some((Ok(Bytes::from_static(b": ping\n\n")), self)),
      }
    }
  }
}

// The cursor a reader resumes after: `after`, else `Last-Event-ID`.
fn cursor(asked: &EventQuery, headers: &HeaderMap) -> Result<Option<i64>, HostError> {
  if asked.after.is_some() {
    return Ok(asked.after);
  }

  headers
    .get("last-event-id")
    .map(|value| {
      value
        .to_str()
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
        .ok_or_else(|| HostError::MalformedRequest("`Last-Event-ID` is not an event's number".to_owned()))
    })
    .transpose()
}

// The events held after the cursor, or why the cursor is gone.
fn held_after(events: &Events, after: Option<i64>) -> Result<Vec<Event>, HostError> {
  events.after(after).map_err(|gone| HostError::Gone {
    after: after.unwrap_or(gone.floor),
    floor: gone.floor,
  })
}

// The event stream, to the issuer alone: server-sent events after the cursor and on as each is published, or with
// `Accept: application/json` a page of the events held.
async fn events(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  headers: HeaderMap,
  asked: Result<Query<EventQuery>, QueryRejection>,
) -> Result<Response, HostError> {
  caller.issuer()?;

  let Query(asked) = asked.map_err(malformed)?;

  if asked.limit == Some(0) {
    return Err(HostError::MalformedRequest("`limit` is a positive integer".to_owned()));
  }

  let after = cursor(&asked, &headers)?;
  let kinds: Option<BTreeSet<String>> = asked.kinds.as_deref().map(|kinds| {
    kinds
      .split(',')
      .map(str::trim)
      .filter(|kind| !kind.is_empty())
      .map(str::to_owned)
      .collect()
  });
  let events = &control.host.placed.events;
  let paged = headers
    .get(header::ACCEPT)
    .and_then(|value| value.to_str().ok())
    .is_some_and(|accept| accept.contains("application/json"));

  if paged {
    let mut items: Vec<Event> = held_after(events, after)?
      .into_iter()
      .filter(|event| wanted(kinds.as_ref(), event))
      .collect();
    let more = asked.limit.is_some_and(|limit| items.len() > limit as usize);

    items.truncate(asked.limit.map_or(usize::MAX, |limit| limit as usize));
    return Ok(Json(EventPage { items, more }).into_response());
  }

  let (cursor, pending) = match after {
    Some(after) => (after, held_after(events, Some(after))?.into()),
    None => (events.last(), VecDeque::new()),
  };
  let reading = Reading {
    host: control.host.clone(),
    cursor,
    pending,
    kinds,
    latest: events.watch(),
    stopping: events.stopping(),
  };
  let body = Body::from_stream(futures_util::stream::unfold(reading, Reading::next));
  let mut response = (StatusCode::OK, body).into_response();
  let said = response.headers_mut();

  said.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
  said.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
  Ok(response)
}

// The external servers the host read a registration of, to the issuer alone, listed or not.
async fn server_page(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  asked: Result<Query<NameQuery>, QueryRejection>,
) -> Result<Json<ServerPage>, HostError> {
  caller.issuer()?;

  let Query(asked) = asked.map_err(malformed)?;
  let every: Vec<Server> = control.host.placed.externals.every().iter().map(|link| link.server()).collect();
  let (items, more) =
    page_by_name(every, |server| server.name.clone(), asked.after.as_deref(), asked.limit)
      .map_err(HostError::MalformedRequest)?;

  Ok(Json(ServerPage { items, more }))
}

async fn schedule_page(
  State(control): State<Control>,
  Extension(caller): Extension<Caller>,
  asked: Result<Query<TimeQuery>, QueryRejection>,
) -> Result<Json<SchedulePage>, HostError> {
  let Query(asked) = asked.map_err(malformed)?;
  let now = now_ms();
  let mut every: Vec<(Schedule, usize)> = Vec::new();

  for entry in caller.entries(&control.host) {
    for (at, state) in control.schedules.of(&entry, now).into_iter().enumerate() {
      let Some(next_at) = state.next_at else {
        continue;
      };

      every.push((
        Schedule {
          namespace: entry.namespace().to_owned(),
          tool_name: entry.name().to_owned(),
          cron: state.cron,
          arguments: state.arguments,
          next_at,
          last_at: state.last_at,
          last_error: state.last_error,
          running: state.running,
        },
        at,
      ));
    }
  }

  every.sort_by(|(left, left_at), (right, right_at)| {
    (left.next_at, &left.namespace, &left.tool_name, left_at).cmp(&(
      right.next_at,
      &right.namespace,
      &right.tool_name,
      right_at,
    ))
  });

  let (page, more) = page_by_time(every, |(schedule, _)| schedule.next_at, asked.after, asked.limit)
    .map_err(HostError::MalformedRequest)?;

  Ok(Json(SchedulePage {
    items: page.into_iter().map(|(schedule, _)| schedule).collect(),
    more,
  }))
}

async fn unknown_route(uri: Uri) -> HostError {
  HostError::UnknownRoute(uri.path().to_owned())
}

async fn method_not_allowed() -> HostError {
  HostError::MethodNotAllowed
}

/// The control socket, each route behind the issuer token or a tool session's: health; the discipline; the event
/// stream; the namespaces, one of them, a namespace's tools and one tool; a call and a claim under a tool session;
/// the tool sessions standing, one read, one issued and one revoked; the external servers; and the schedules. Any
/// other path or method is refused in the common error shape.
pub fn control(
  records: Arc<Records>,
  host: Arc<Host>,
  schedules: Arc<Schedules>,
  calls: Arc<AsyncCalls>,
  issuer: String,
) -> Router {
  let gate: (Arc<Records>, Arc<str>) = (records.clone(), issuer.into());
  let state = Control {
    records,
    host,
    schedules,
    calls,
  };

  Router::new()
    .route("/health", get(health))
    .route("/discipline", get(discipline))
    .route("/events", get(events))
    .route("/namespaces", get(namespaces))
    .route("/namespaces/{namespace}", get(one_namespace))
    .route("/namespaces/{namespace}/tools", get(tool_page))
    .route("/namespaces/{namespace}/tools/{toolName}", get(one_tool))
    .route("/tool-calls", post(tool_call))
    .route("/claims", post(claim))
    .route("/tool-sessions", get(tool_session_page).post(issue))
    .route("/tool-sessions/{token}", get(one_tool_session).delete(revoke))
    .route("/tool-session-records/{id}", delete(revoke_record))
    .route("/servers", get(server_page))
    .route("/schedules", get(schedule_page))
    .route("/uses", get(use_page))
    .fallback(unknown_route)
    .method_not_allowed_fallback(method_not_allowed)
    .layer(middleware::from_fn_with_state(gate, authorise_control))
    .with_state(state)
}

/// What the console face stands on beside the control socket's reads: the console token, and the face bmo-chat's
/// crate built from the host's declaration and its bundle.
pub struct Screen {
  pub token: String,
  pub face: ConsoleFace,
}

// The host's own guard of the reads it lays under the console face: the console token, and no other, reaches them,
// and they are answered as the issuer's.
async fn authorise_console(State(token): State<Arc<str>>, mut request: Request, next: Next) -> Response {
  match bearer(&request) {
    None => return HostError::NoToken.into_response(),
    Some(given) if given != &*token => return HostError::NotTheConsole.into_response(),
    Some(_) => {}
  }

  request.extensions_mut().insert(Caller::Issuer);
  next.run(request).await
}

/// The console face: the manifest and the bundle's files as the crate answers them, and the control socket's reads,
/// as the issuer is answered them, under the crate's screens' guard and the console token. Any other path or method
/// is refused in the common error shape.
pub fn console(
  records: Arc<Records>,
  host: Arc<Host>,
  schedules: Arc<Schedules>,
  calls: Arc<AsyncCalls>,
  screen: Screen,
) -> Router {
  let state = Control {
    records,
    host,
    schedules,
    calls,
  };
  let reads = Router::new()
    .route("/namespaces", get(namespaces))
    .route("/namespaces/{namespace}", get(one_namespace))
    .route("/namespaces/{namespace}/tools", get(tool_page))
    .route("/namespaces/{namespace}/tools/{toolName}", get(one_tool))
    .route("/schedules", get(schedule_page))
    .route("/tool-sessions", get(tool_session_page))
    .route("/servers", get(server_page))
    .route("/uses", get(use_page));

  screen
    .face
    .guard_screens(reads)
    .route_layer(middleware::from_fn_with_state(Arc::<str>::from(screen.token), authorise_console))
    .merge(screen.face.router())
    .fallback(unknown_route)
    .method_not_allowed_fallback(method_not_allowed)
    .with_state(state)
}
