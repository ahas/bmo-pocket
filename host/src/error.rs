//! What the host refuses and fails with over HTTP, on either face: a non-2xx status and the common error body of
//! `code` and `error` (docs/api.md, 「Error codes」), each case the host names with its own code.

use axum::Json;
use axum::response::{IntoResponse, Response};
use bmo_pocket_client::{ErrorCode, WireError};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HostError {
  /// No bearer token came with the request.
  #[error("a bearer token is required")]
  NoToken,
  /// The token names no session record: never issued, revoked, or past its run's deadline.
  #[error("the token names no session")]
  UnknownToken,
  /// The path names a namespace no tool is shown under.
  #[error("no entry is declared under the namespace `{0}`")]
  UnknownNamespace(String),
  /// No entry of the namespace declares a listing line.
  #[error("no entry of the namespace `{0}` declares a listing line")]
  NoListing(String),
  /// No entry of that full name, or none the record holds.
  #[error("Tool {0} not found")]
  UnknownEntry(String),
  /// What the launcher handed when asking for a token cannot be a session record.
  #[error("the session record is malformed: {0}")]
  MalformedRecord(String),
  /// A body or a query that does not read as the route asks.
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
      Self::NoToken => ErrorCode::NoToken,
      Self::UnknownToken => ErrorCode::UnknownToken,
      Self::UnknownNamespace(_) => ErrorCode::UnknownNamespace,
      Self::NoListing(_) => ErrorCode::NoListing,
      Self::UnknownEntry(_) => ErrorCode::UnknownEntry,
      Self::MalformedRecord(_) => ErrorCode::MalformedRecord,
      Self::MalformedRequest(_) => ErrorCode::MalformedRequest,
      Self::ArgumentsMisfit(_) => ErrorCode::ArgumentsMisfit,
      Self::GrantRefused(_) => ErrorCode::GrantRefused,
      Self::ToolFailed(_) => ErrorCode::ToolFailed,
      Self::Unwritable(_) => ErrorCode::UnwritableRecord,
      Self::UnknownRoute(_) => ErrorCode::UnknownRoute,
      Self::MethodNotAllowed => ErrorCode::MethodNotAllowed,
    }
  }
}

// Every sentence is said, on either face: the tools face's are fixed, and the control socket's reach the host's own
// user alone.
impl IntoResponse for HostError {
  fn into_response(self) -> Response {
    let code = self.code();

    (code.status(), Json(WireError::new(code, self.to_string()))).into_response()
  }
}
