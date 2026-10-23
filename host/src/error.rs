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
  #[error("the issuer token alone issues, revokes and lists tool sessions and lists the external servers")]
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
  /// The token names no tool session.
  #[error("no tool session stands under that token")]
  UnknownSession,
  /// A token on the console face other than the console token.
  #[error("the console token alone reaches the console face")]
  NotTheConsole,
  /// A route the screen uses asked with no `Owner-Role` allowing the screen's role.
  #[error("the screen is the administrator's: `Owner-Role` must allow it")]
  ScreenRole,
  /// The host holds no console bundle, so it serves no screen.
  #[error("the host holds no console bundle")]
  NoBundle,
  /// No file of the console bundle goes by that name.
  #[error("the console bundle has no file `{0}`")]
  NoBundleFile(String),
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
  /// The tool failed before it answered a tool result; the sentence names how.
  #[error("{0}")]
  ToolFailed(String),
  /// The host could not keep or take back a record on disk.
  #[error("the session record could not be written: {0}")]
  Unwritable(String),
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
      Self::IssuerOnly | Self::SessionOnly | Self::GrantRefused(_) | Self::ScreenRole => {
        ErrorCode::Forbidden
      }
      Self::UnknownNamespace(_)
      | Self::UnknownEntry(_)
      | Self::UnknownSession
      | Self::NoBundle
      | Self::NoBundleFile(_) => ErrorCode::NotFound,
      Self::MalformedRecord(_) | Self::MalformedRequest(_) => ErrorCode::BadRequest,
      Self::ArgumentsMisfit(_) => ErrorCode::ArgumentsMisfit,
      Self::ToolFailed(_) => ErrorCode::ToolFailed,
      Self::Unwritable(_) => ErrorCode::Failed,
      Self::UnknownRoute(_) | Self::MethodNotAllowed => ErrorCode::NoRoute,
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
