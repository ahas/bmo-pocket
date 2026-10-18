//! What the host answers over HTTP before a request reaches a tool: RFC 7807 problem details with a
//! machine-readable code (docs/api.md, 「A refusal over HTTP」), each case the host names with its own.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
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
  #[error("no namespace `{0}`")]
  UnknownNamespace(String),
  /// What the launcher handed when asking for a token cannot be a session record.
  #[error("the session record is malformed: {0}")]
  MalformedRecord(String),
  /// The host could not keep or take back a record on disk.
  #[error("the session record could not be written: {0}")]
  Unwritable(String),
}

impl HostError {
  pub fn status_code(&self) -> StatusCode {
    match self {
      Self::NoToken | Self::UnknownToken => StatusCode::UNAUTHORIZED,
      Self::UnknownNamespace(_) => StatusCode::NOT_FOUND,
      Self::MalformedRecord(_) => StatusCode::BAD_REQUEST,
      Self::Unwritable(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
  }

  pub fn error_code(&self) -> &'static str {
    match self {
      Self::NoToken => "mcp-host.no-token",
      Self::UnknownToken => "mcp-host.unknown-token",
      Self::UnknownNamespace(_) => "mcp-host.unknown-namespace",
      Self::MalformedRecord(_) => "mcp-host.malformed-record",
      Self::Unwritable(_) => "mcp-host.unwritable-record",
    }
  }
}

// The problem's detail, which may carry what only the host's own user should read, is said in a debug build
// alone; the code is what a caller decides on.
impl IntoResponse for HostError {
  fn into_response(self) -> Response {
    let status = self.status_code();
    let code = self.error_code();
    let mut body = json!({
      "type": format!("urn:sumida:error:{code}"),
      "title": status.canonical_reason().unwrap_or("Error"),
      "status": status.as_u16(),
      "code": code,
    });

    if cfg!(debug_assertions) {
      body["detail"] = json!(self.to_string());
    }

    let mut response = (status, Json(body)).into_response();

    response
      .headers_mut()
      .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/problem+json"));
    response
  }
}
