//! The host's two faces: the namespaces sessions call on the loopback port, each behind the bearer
//! token, and the control socket the launchers issue and revoke tokens on (docs/mcp-host.md).

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Request, State};
use axum::http::{StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, post};
use axum::{Json, Router};
use bmo_tool::Session;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::HostError;
use crate::handler::{Handler, Host, unknown};
use crate::records::Records;
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

// What the control socket stands on: the records, and the host it asks for the catalogue.
#[derive(Clone)]
struct Control {
  records: Arc<Records>,
  host: Arc<Host>,
}

async fn issue(
  State(Control { records, .. }): State<Control>,
  body: Bytes,
) -> Result<Json<serde_json::Value>, HostError> {
  let session: Session = serde_json::from_slice(&body).map_err(|why| {
    eprintln!("mcp-host: a session record was refused: {why}");
    HostError::MalformedRecord(why.to_string())
  })?;
  let token = records.issue(session).map_err(|why| HostError::Unwritable(why.to_string()))?;

  Ok(Json(json!({ "token": token })))
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

// A namespace's listing: each placed entry's line in its order; none where no entry writes one.
async fn listing(
  State(Control { host, .. }): State<Control>,
  Path(namespace): Path<String>,
) -> Result<String, StatusCode> {
  host.listing(&namespace).ok_or(StatusCode::NOT_FOUND)
}

// What a namespace's owners say of it beside its tools; none where nothing is said.
async fn facts(
  State(Control { host, .. }): State<Control>,
  Path(namespace): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
  host.facts(&namespace).map(Json).ok_or(StatusCode::NOT_FOUND)
}

// Every entry the host serves, as a person reads the catalogue.
async fn catalogue(State(Control { host, .. }): State<Control>) -> Json<Value> {
  Json(Value::Array(host.every().iter().map(|entry| entry.catalogued()).collect()))
}

// One entry described, its schemas with it.
async fn entry(
  State(Control { host, .. }): State<Control>,
  Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<Value>, StatusCode> {
  let full = format!("{namespace}/{name}");
  let found = host
    .every()
    .into_iter()
    .find(|entry| entry.full_name() == full)
    .ok_or(StatusCode::NOT_FOUND)?;
  let mut said = found.catalogued();

  said["described"] = described(&found).into();
  said["inputSchema"] = found.input_schema();
  said["outputSchema"] = found.output_schema().unwrap_or(Value::Null);

  Ok(Json(said))
}

#[derive(Deserialize)]
struct Called {
  session: Session,
  tool: String,
  #[serde(default)]
  arguments: Value,
}

// A call under the record it carries, as a session's call is dispatched, entries for the system included.
async fn call(State(Control { host, .. }): State<Control>, body: Bytes) -> Result<Json<Value>, HostError> {
  let called: Called =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRecord(why.to_string()))?;
  let Some(found) = host.find_full(&called.session, &called.tool, true) else {
    return Ok(Json(crate::running::failed(unknown(&called.tool))));
  };
  let answer = host
    .answer(&found, &called.session, called.arguments)
    .await
    .unwrap_or_else(|why| crate::running::failed(why.message));

  called.session.kept.close().await;
  Ok(Json(answer))
}

#[derive(Deserialize)]
struct Claimed {
  session: Session,
  address: String,
}

// Which entry the record it carries holds claims an address, as the call-back socket's claim is answered.
async fn claim(State(Control { host, .. }): State<Control>, body: Bytes) -> Result<Json<Value>, HostError> {
  let claimed: Claimed =
    serde_json::from_slice(&body).map_err(|why| HostError::MalformedRecord(why.to_string()))?;
  let answer = host.claim(&claimed.session, &claimed.address).await;

  claimed.session.kept.close().await;
  Ok(Json(answer))
}

async fn health() -> Response<Body> {
  Response::new(Body::from("ok"))
}

/// The control socket: a record issued and its token answered, a token revoked and what its tools kept
/// ended, a namespace's listing and what its owners say of it, the catalogue, a call made through it and which
/// entry claims an address, and a health check.
pub fn control(records: Arc<Records>, host: Arc<Host>) -> Router {
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
    .with_state(Control { records, host })
}
