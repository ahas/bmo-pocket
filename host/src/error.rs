//! What the host refuses and fails with over HTTP, on either face: a non-2xx status and the common error body of
//! `code` and `error` (docs/api.md, 「Error codes」), each case the host names answered with its code.

use axum::Json;
use axum::response::{IntoResponse, Response};
use bmo_pocket_client::{ErrorCode, WireError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
  /// No bearer token came with the request.
  #[error("a bearer token is required")]
  NoToken,
  /// The token is neither the issuer's nor a tool session's: never issued, revoked, or past its `expiresAt`.
  #[error("the token names no session")]
  UnknownToken,
  /// A tool session's token on a route the issuer token alone takes.
  #[error(
    "the issuer token alone issues, revokes and lists tool sessions, lists the external servers and reads the \
     event stream"
  )]
  IssuerOnly,
  /// The issuer token on a route a tool session's token alone takes.
  #[error("a call or a claim is made under a tool session's token")]
  SessionOnly,
  /// The token sees no entry under the namespace.
  #[error("no entry the token sees is declared under the namespace `{0}`")]
  UnknownNamespace(String),
  /// No tool of that full name the token sees.
  #[error("Tool {0} not found")]
  UnknownEntry(String),
  /// No screen, digest or file of a personal tool's screen the token's session reaches stands at the path.
  #[error("{0}")]
  UnknownScreen(String),
  /// The owner's runner could not be reached, did not answer a screen's file, or answered other bytes.
  #[error("{0}")]
  RunnerUnreached(String),
  /// No asynchronous call of that id was asked under the token's session.
  #[error("call {0} not found")]
  UnknownCall(String),
  /// The token names no tool session.
  #[error("no tool session stands under that token")]
  UnknownSession,
  /// A token on the console face other than the console token.
  #[error("the console token alone reaches the console face")]
  NotTheConsole,
  /// What was handed to be issued cannot be a session record.
  #[error("the session record is malformed: {0}")]
  MalformedRecord(String),
  /// A body or a query that does not read as the route takes it.
  #[error("the request is malformed: {0}")]
  MalformedRequest(String),
  /// The arguments do not fit the entry's schema; the sentence is the dispatch's own.
  #[error("{0}")]
  ArgumentsMisfit(String),
  /// The session's grants do not hold the entry's grant; the sentence names it.
  #[error("{0}")]
  GrantRefused(String),
  /// The entry's app is one the session's record withholds; the sentence says so.
  #[error("{0}")]
  Withheld(String),
  /// The tool failed before it answered a tool result; the sentence names how.
  #[error("{0}")]
  ToolFailed(String),
  /// The host could not keep or take back a record on disk.
  #[error("the session record could not be written: {0}")]
  Unwritable(String),
  /// A cursor of the event stream older than the window held, or one of an earlier run.
  #[error(
    "the event {after} is no longer held: the events held follow {floor}; read the namespaces again and subscribe \
     with no cursor"
  )]
  Gone { after: i64, floor: i64 },
  /// A path the control socket does not serve.
  #[error("no route `{0}`")]
  UnknownRoute(String),
  /// A method a served path does not take.
  #[error("the path does not take this method")]
  MethodNotAllowed,
}

impl HostError {
  pub fn code(&self) -> ErrorCode {
    match self {
      Self::NoToken | Self::UnknownToken | Self::NotTheConsole => ErrorCode::Unauthorized,
      Self::IssuerOnly | Self::SessionOnly | Self::GrantRefused(_) | Self::Withheld(_) => {
        ErrorCode::Forbidden
      }
      Self::UnknownNamespace(_)
      | Self::UnknownEntry(_)
      | Self::UnknownCall(_)
      | Self::UnknownSession
      | Self::UnknownScreen(_) => ErrorCode::NotFound,
      Self::RunnerUnreached(_) => ErrorCode::RunnerUnreached,
      Self::MalformedRecord(_) | Self::MalformedRequest(_) => ErrorCode::BadRequest,
      Self::ArgumentsMisfit(_) => ErrorCode::ArgumentsMisfit,
      Self::ToolFailed(_) => ErrorCode::ToolFailed,
      Self::Unwritable(_) => ErrorCode::Failed,
      Self::UnknownRoute(_) | Self::MethodNotAllowed => ErrorCode::NoRoute,
      Self::Gone { .. } => ErrorCode::Gone,
    }
  }
}

// Every sentence is said, on either face: the tools face's are fixed, and the control socket's reach a token holder
// alone.
impl IntoResponse for HostError {
  fn into_response(self) -> Response {
    let code = self.code();

    (code.status(), Json(WireError::new(code, self.to_string()))).into_response()
  }
}
