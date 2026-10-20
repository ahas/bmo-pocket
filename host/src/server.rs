//! The host's two faces: the namespaces sessions call on the loopback port, each behind the bearer
//! token, and the control socket the launchers issue and revoke tokens on (docs/mcp-host.md).

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, Request, State};
use axum::http::{StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use bmo_pocket_client::{
  CallRequest, CatalogueEntry, CatalogueQuery, ClaimRequest, Claimed, DescribedEntry, Issued,
};
use bmo_tool::{Session, SessionRequest};
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::Value;

use crate::error::HostError;
use crate::handler::{Entry, Handler, Host, Refusal};
use crate::records::Records;
use crate::schedule::{Schedules, now_ms};
use crate::toolbook::{TOOLBOOK, Toolbook, described};

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

// One request on a namespace's path, served where a placed manifest declares that
// namespace as the catalogue stands at the request, and answered 404 where none does.
async fn namespace(
  State(host): State<Arc<Host>>,
  Path(namespace): Path<String>,
  request: Request,
) -> Response {
  if !host.namespaces().contains(&namespace) {
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

// What the control socket stands on: the records, the host it asks for the catalogue, and the schedules each
// catalogue entry carries.
#[derive(Clone)]
struct Control {
  records: Arc<Records>,
  host: Arc<Host>,
  schedules: Arc<Schedules>,
}

async fn issue(
  State(Control { records, .. }): State<Control>,
  body: Bytes,
) -> Result<Json<Issued>, HostError> {
  let asked: SessionRequest = serde_json::from_slice(&body).map_err(|why| {
    eprintln!("mcp-host: a session record was refused: {why}");
    HostError::MalformedRecord(why.to_string())
  })?;
  let session = asked.issued_at(crate::records::now_ms());
  let token = records.issue(session).map_err(|why| HostError::Unwritable(why.to_string()))?;

  Ok(Json(Issued { token }))
}

async fn revoke(
  State(Control { records, .. }): State<Control>,
  Path(token): Path<String>,
) -> Result<StatusCode, HostError> {
  let (_, gone) = records.revoke(&token).map_err(|why| HostError::Unwritable(why.to_string()))?;

  if let Some(gone) = gone {
    gone.kept.close().await;
  }

  Ok(StatusCode::NO_CONTENT)
}

// A namespace's listing: each placed entry's line in its order; refused where no entry writes one.
async fn listing(
  State(Control { host, .. }): State<Control>,
  Path(namespace): Path<String>,
) -> Result<String, HostError> {
  host.listing(&namespace).ok_or(HostError::NoListing(namespace))
}

// What a namespace's owners say of it beside its tools; refused where no entry is declared under it.
async fn facts(
  State(Control { host, .. }): State<Control>,
  Path(namespace): Path<String>,
) -> Result<Json<Value>, HostError> {
  host.facts(&namespace).map(Json).ok_or(HostError::UnknownNamespace(namespace))
}

impl Control {
  // An entry as the catalogue answers it: its schedules with where each stands.
  fn catalogued(&self, entry: &Entry, now: i64) -> CatalogueEntry {
    let mut said = entry.catalogued();

    said.schedules = self.schedules.of(entry, now);
    said
  }
}

/// The page of entries a cursor and a limit name, out of every entry in order of full name: those after the
/// cursor from the nearest on, or the nearest before it in that same order; refused for two cursors at once or a
/// limit of none.
pub fn page<T>(
  mut every: Vec<T>,
  name: impl Fn(&T) -> String,
  asked: &CatalogueQuery,
) -> Result<Vec<T>, String> {
  if asked.after.is_some() && asked.before.is_some() {
    return Err("a page takes one cursor, `after` or `before`, not both".to_owned());
  }

  if asked.limit == Some(0) {
    return Err("`limit` is a positive integer".to_owned());
  }

  let limit = asked.limit.map_or(usize::MAX, |limit| limit as usize);

  every.sort_by_key(&name);

  if let Some(before) = &asked.before {
    every.retain(|one| name(one) < *before);

    let from = every.len().saturating_sub(limit);

    return Ok(every.split_off(from));
  }

  if let Some(after) = &asked.after {
    every.retain(|one| name(one) > *after);
  }

  every.truncate(limit);
  Ok(every)
}

// A page of every entry the host serves, as a person reads the catalogue.
async fn catalogue(
  State(control): State<Control>,
  asked: Result<Query<CatalogueQuery>, QueryRejection>,
) -> Result<Json<Vec<CatalogueEntry>>, HostError> {
  let Query(asked) = asked.map_err(|why| HostError::MalformedRequest(why.body_text()))?;
  let now = now_ms();
  let entries =
    page(control.host.every(), Entry::full_name, &asked).map_err(HostError::MalformedRequest)?;

  Ok(Json(entries.iter().map(|entry| control.catalogued(entry, now)).collect()))
}

// One entry described, its schemas with it.
async fn entry(
  State(control): State<Control>,
  Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<DescribedEntry>, HostError> {
  let full = format!("{namespace}/{name}");
  let found = control
    .host
    .every()
    .into_iter()
    .find(|entry| entry.full_name() == full)
    .ok_or(HostError::UnknownEntry(full))?;

  Ok(Json(DescribedEntry {
    entry: control.catalogued(&found, now_ms()),
    described: described(&found),
    input_schema: found.input_schema(),
    output_schema: found.output_schema(),
  }))
}

// A call under the record it carries, as a session's call is dispatched, entries for the system included; each
// refusal of the host's answered by its status, and the tool's own result as it answered it.
async fn call(State(Control { host, .. }): State<Control>, body: Bytes) -> Result<Json<Value>, HostError> {
  let called: CallRequest =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRequest(why.to_string()))?;
  let Some(found) = host.find_full(&called.session, &called.tool, true) else {
    return Err(HostError::UnknownEntry(called.tool));
  };
  let arguments = called.arguments.map_or(Value::Null, Value::Object);
  let answer = host.attempt(&found, &called.session, arguments).await;

  called.session.kept.close().await;

  match answer {
    Ok(result) => Ok(Json(result)),
    Err(Refusal::Misfit(why)) => Err(HostError::ArgumentsMisfit(why)),
    Err(Refusal::Denied(why)) => Err(HostError::GrantRefused(why)),
    Err(Refusal::Failed(why)) => Err(HostError::ToolFailed(why)),
  }
}

// Which entry the record it carries holds claims an address, as the call-back socket's claim is answered.
async fn claim(
  State(Control { host, .. }): State<Control>,
  body: Bytes,
) -> Result<Json<Claimed>, HostError> {
  let claimed: ClaimRequest =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRequest(why.to_string()))?;
  let answer = host.claim(&claimed.session, &claimed.address).await;

  claimed.session.kept.close().await;
  Ok(Json(Claimed {
    entry: answer["entry"].as_str().map(str::to_owned),
    arguments: answer.get("arguments").cloned().filter(|arguments| !arguments.is_null()),
  }))
}

async fn health() -> Response<Body> {
  Response::new(Body::from("ok"))
}

async fn unknown_route(uri: Uri) -> HostError {
  HostError::UnknownRoute(uri.path().to_owned())
}

async fn method_not_allowed() -> HostError {
  HostError::MethodNotAllowed
}

/// The control socket: a record issued and its token answered, a token revoked and what its tools kept
/// ended, a namespace's listing and what its owners say of it, a page of the catalogue and one entry of it, a
/// call made through it and which entry claims an address, and a health check; any other path or method
/// refused in the common error shape.
pub fn control(records: Arc<Records>, host: Arc<Host>, schedules: Arc<Schedules>) -> Router {
  Router::new()
    .route("/sessions", post(issue))
    .route("/sessions/{token}", delete(revoke))
    .route("/listings/{namespace}", get(listing))
    .route("/facts/{namespace}", get(facts))
    .route("/catalogue", get(catalogue))
    .route("/catalogue/{namespace}/{name}", get(entry))
    .route("/calls", post(call))
    .route("/claims", post(claim))
    .route("/health", get(health))
    .fallback(unknown_route)
    .method_not_allowed_fallback(method_not_allowed)
    .with_state(Control {
      records,
      host,
      schedules,
    })
}
