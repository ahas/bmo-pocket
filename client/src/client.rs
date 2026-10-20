//! One call per route of the control socket (docs/api.md, 「The control socket」), each made with the client's
//! bearer token and answering the route's body, or the host's refusal as it answered it.

use std::io;
use std::path::{Path, PathBuf};

use bmo_tool::SessionRequest;
use http::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::error::WireError;
use crate::transport::{Reply, request};
use crate::wire::{
  Claim, ClaimRequest, Health, NameQuery, Namespace, NamespacePage, SchedulePage, TimeQuery, Tool,
  ToolCall, ToolCallRequest, ToolPage, ToolSession,
};

/// Why a call answered no body of its route.
#[derive(Debug, Error)]
pub enum Failure {
  /// The socket could not be reached at all: the host is not running, or not yet.
  #[error("the control socket could not be reached: {0}")]
  Unreachable(io::Error),
  /// The host refused or failed, as its error answer says.
  #[error("the host answered {status}: {} ({})", .error.error, .error.code.as_str())]
  Refused { status: StatusCode, error: WireError },
  /// The host answered what this client does not read.
  #[error("the host answered {status} with what does not read: {why}")]
  Unreadable { status: StatusCode, why: String },
}

/// The control socket's path where nothing names another: `BMO_MCP_HOST_SOCKET`, or `bmo/mcp-host.sock` under
/// the user's runtime directory.
pub fn default_socket() -> PathBuf {
  std::env::var_os("BMO_MCP_HOST_SOCKET").map_or_else(
    || {
      std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("bmo/mcp-host.sock")
    },
    PathBuf::from,
  )
}

/// The tool host's control socket, asked with one bearer token: the issuer's, or a tool session's.
#[derive(Clone, Debug)]
pub struct Pocket {
  socket: PathBuf,
  token: String,
}

// A route's path and query, its segments encoded.
fn route(segments: &[&str], query: &[(&str, String)]) -> String {
  let mut url = Url::parse("http://localhost/").expect("a fixed base address reads");

  if let Ok(mut path) = url.path_segments_mut() {
    path.clear().extend(segments);
  }

  if !query.is_empty() {
    url.query_pairs_mut().extend_pairs(query);
  }

  url[url::Position::BeforePath..].to_owned()
}

// The query a page is asked with: its cursor and its limit, where given.
fn paged(after: Option<String>, limit: Option<u32>) -> Vec<(&'static str, String)> {
  [("after", after), ("limit", limit.map(|limit| limit.to_string()))]
    .into_iter()
    .filter_map(|(name, value)| Some((name, value?)))
    .collect()
}

// What a reply reads as where its status is the one the route succeeds with.
fn body<T: DeserializeOwned>(reply: Reply, success: StatusCode) -> Result<T, Failure> {
  let status = reply.status;

  if status != success {
    return Err(refusal(&reply));
  }

  serde_json::from_slice(&reply.body).map_err(|why| Failure::Unreadable {
    status,
    why: why.to_string(),
  })
}

fn refusal(reply: &Reply) -> Failure {
  match serde_json::from_slice::<WireError>(&reply.body) {
    Ok(error) => Failure::Refused {
      status: reply.status,
      error,
    },
    Err(why) => Failure::Unreadable {
      status: reply.status,
      why: format!("no error answer: {why}"),
    },
  }
}

fn json<T: serde::Serialize>(value: &T) -> Result<Value, Failure> {
  serde_json::to_value(value).map_err(|why| Failure::Unreadable {
    status: StatusCode::OK,
    why: format!("the request does not write: {why}"),
  })
}

impl Pocket {
  /// The control socket at this path, asked with this token.
  pub fn at(socket: impl Into<PathBuf>, token: impl Into<String>) -> Self {
    Self {
      socket: socket.into(),
      token: token.into(),
    }
  }

  /// The control socket at its default path, asked with this token.
  pub fn standing(token: impl Into<String>) -> Self {
    Self::at(default_socket(), token)
  }

  pub fn socket(&self) -> &Path {
    &self.socket
  }

  async fn ask(&self, method: Method, path: &str, sent: Option<&Value>) -> Result<Reply, Failure> {
    request(&self.socket, method, path, Some(&self.token), sent)
      .await
      .map_err(Failure::Unreachable)
  }

  /// `GET /health`: whether the host is ready, and its version.
  pub async fn health(&self) -> Result<Health, Failure> {
    body(self.ask(Method::GET, "/health", None).await?, StatusCode::OK)
  }

  /// `GET /namespaces`: a page of the namespaces the token sees, in order of name.
  pub async fn namespaces(&self, page: &NameQuery) -> Result<NamespacePage, Failure> {
    let query = paged(page.after.clone(), page.limit);

    body(self.ask(Method::GET, &route(&["namespaces"], &query), None).await?, StatusCode::OK)
  }

  /// `GET /namespaces/{namespace}`: one namespace and its facts.
  pub async fn namespace(&self, namespace: &str) -> Result<Namespace, Failure> {
    body(
      self.ask(Method::GET, &route(&["namespaces", namespace], &[]), None).await?,
      StatusCode::OK,
    )
  }

  /// `GET /namespaces/{namespace}/tools`: a page of one namespace's tools the token sees, in order of name.
  pub async fn tools(&self, namespace: &str, page: &NameQuery) -> Result<ToolPage, Failure> {
    let query = paged(page.after.clone(), page.limit);
    let path = route(&["namespaces", namespace, "tools"], &query);

    body(self.ask(Method::GET, &path, None).await?, StatusCode::OK)
  }

  /// `GET /namespaces/{namespace}/tools/{toolName}`: one tool.
  pub async fn tool(&self, namespace: &str, tool_name: &str) -> Result<Tool, Failure> {
    let path = route(&["namespaces", namespace, "tools", tool_name], &[]);

    body(self.ask(Method::GET, &path, None).await?, StatusCode::OK)
  }

  /// `POST /tool-calls`: one tool called under the token's session.
  pub async fn call(&self, call: &ToolCallRequest) -> Result<ToolCall, Failure> {
    body(self.ask(Method::POST, "/tool-calls", Some(&json(call)?)).await?, StatusCode::OK)
  }

  /// `POST /claims`: whether the token's session may have an address read, and by which entry.
  pub async fn claim(&self, resource: &str) -> Result<Claim, Failure> {
    let asked = json(&ClaimRequest {
      resource: resource.to_owned(),
    })?;

    body(self.ask(Method::POST, "/claims", Some(&asked)).await?, StatusCode::OK)
  }

  /// `POST /tool-sessions`: a tool session issued, with the issuer's token.
  pub async fn issue(&self, session: &SessionRequest) -> Result<ToolSession, Failure> {
    body(
      self.ask(Method::POST, "/tool-sessions", Some(&json(session)?)).await?,
      StatusCode::CREATED,
    )
  }

  /// `DELETE /tool-sessions/{token}`: a tool session revoked, with the issuer's token, and what its tools kept
  /// ended.
  pub async fn revoke(&self, token: &str) -> Result<(), Failure> {
    let reply = self.ask(Method::DELETE, &route(&["tool-sessions", token], &[]), None).await?;

    match reply.status {
      StatusCode::NO_CONTENT => Ok(()),
      _ => Err(refusal(&reply)),
    }
  }

  /// `GET /schedules`: a page of the schedules the token sees, in order of when each is next due.
  pub async fn schedules(&self, page: &TimeQuery) -> Result<SchedulePage, Failure> {
    let query = paged(page.after.map(|after| after.to_string()), page.limit);

    body(self.ask(Method::GET, &route(&["schedules"], &query), None).await?, StatusCode::OK)
  }
}
