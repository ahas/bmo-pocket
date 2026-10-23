//! The host's two faces: the namespaces sessions call on the loopback port, each behind a tool session's bearer
//! token, and the control socket, where every route takes the issuer token or a tool session's (docs/mcp-host.md,
//! 「Tokens」; docs/api.md, 「The control socket」).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Extension, Path, Query, Request, State};
use axum::http::{StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use bmo_pocket_client::{
  Claim, ClaimRequest, Health, NameQuery, Namespace, NamespacePage, Schedule, SchedulePage, TimeQuery,
  Tool, ToolCall, ToolCallRequest, ToolPage, ToolResult, ToolSession,
};
use bmo_tool::{Session, SessionRequest};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;

use crate::error::HostError;
use crate::handler::{Claimant, Entry, Handler, Host, Refusal};
use crate::records::Records;
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
// request for the handler to serve by.
async fn authorise(State(records): State<Arc<Records>>, mut request: Request, next: Next) -> Response {
  let Some(token) = bearer(&request) else {
    return HostError::NoToken.into_response();
  };
  let Some((_, session)) = records.get(token) else {
    return HostError::UnknownToken.into_response();
  };

  request.extensions_mut().insert::<Arc<Session>>(session);
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
  State(host): State<Arc<Host>>,
  Path(namespace): Path<String>,
  request: Request,
) -> Response {
  let named = request
    .extensions()
    .get::<Arc<Session>>()
    .is_some_and(|session| session.uses(&namespace));

  if !named || !host.namespaces().contains(&namespace) {
    return HostError::UnknownNamespace(namespace).into_response();
  }

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

/// The namespaces, each at `/mcp/<namespace>`, and the toolbook at `/mcp/toolbook`, served as streamable
/// HTTP with no session kept: every request stands alone and is known by its token.
pub fn tools(records: Arc<Records>, host: Arc<Host>) -> Router {
  let book = host.clone();
  let toolbook = StreamableHttpService::new(
    move || Ok(Toolbook { host: book.clone() }),
    Arc::new(NeverSessionManager::default()),
    stateless(),
  );

  Router::new()
    .route_service(&format!("/mcp/{TOOLBOOK}"), toolbook)
    .route("/mcp/{namespace}", any(namespace))
    .with_state(host)
    .fallback(unknown_namespace)
    .layer(middleware::from_fn_with_state(records, authorise))
}

// What the control socket stands on: the records, the host it asks for the catalogue, the schedules, and the issuer
// token.
#[derive(Clone)]
struct Control {
  records: Arc<Records>,
  host: Arc<Host>,
  schedules: Arc<Schedules>,
  issuer: Arc<str>,
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
async fn authorise_control(State(control): State<Control>, mut request: Request, next: Next) -> Response {
  let Some(token) = bearer(&request) else {
    return HostError::NoToken.into_response();
  };
  let caller = if token == &*control.issuer {
    Caller::Issuer
  } else {
    match control.records.get(token) {
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

async fn health() -> Json<Health> {
  Json(Health {
    ready: true,
    reason: None,
    version: env!("CARGO_PKG_VERSION").to_owned(),
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
    Err(Refusal::Failed(why)) => return Err(HostError::ToolFailed(why)),
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

/// The control socket, each route behind the issuer token or a tool session's: health; the discipline; the
/// namespaces, one of them, a namespace's tools and one tool; a call and a claim under a tool session; a tool
/// session issued and revoked; and the schedules. Any other path or method is refused in the common error shape.
pub fn control(
  records: Arc<Records>,
  host: Arc<Host>,
  schedules: Arc<Schedules>,
  issuer: String,
) -> Router {
  let state = Control {
    records,
    host,
    schedules,
    issuer: issuer.into(),
  };

  Router::new()
    .route("/health", get(health))
    .route("/discipline", get(discipline))
    .route("/namespaces", get(namespaces))
    .route("/namespaces/{namespace}", get(one_namespace))
    .route("/namespaces/{namespace}/tools", get(tool_page))
    .route("/namespaces/{namespace}/tools/{toolName}", get(one_tool))
    .route("/tool-calls", post(tool_call))
    .route("/claims", post(claim))
    .route("/tool-sessions", post(issue))
    .route("/tool-sessions/{token}", delete(revoke))
    .route("/schedules", get(schedule_page))
    .fallback(unknown_route)
    .method_not_allowed_fallback(method_not_allowed)
    .layer(middleware::from_fn_with_state(state.clone(), authorise_control))
    .with_state(state)
}
