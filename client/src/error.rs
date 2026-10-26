//! The one shape every failure and refusal of the host is answered in (docs/api.md, 「Error codes」): a non-2xx
//! status and a body of `code` and `error`.

use http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::{IntoStaticStr, VariantArray};

/// A stable code a caller decides on, each answered with one status.
#[derive(
  Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, VariantArray, IntoStaticStr,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum ErrorCode {
  /// The body or the query is not the shape the route takes.
  BadRequest,
  /// The token is missing, or names neither the issuer nor a tool session.
  Unauthorized,
  /// The token is right but may not do this.
  Forbidden,
  /// The route is right, but nothing the token sees stands at the key it names.
  NotFound,
  /// No route answers the method and path.
  NoRoute,
  /// An event after the cursor is no longer held: older than the window the host holds, or of an earlier run.
  Gone,
  /// The input does not fit the tool's schema in the form held.
  ArgumentsMisfit,
  /// The host failed.
  Failed,
  /// The tool exited uncleanly, answered what is not a tool result, or ran past its timeout.
  ToolFailed,
}

impl ErrorCode {
  /// The status it is answered with.
  pub fn status(self) -> StatusCode {
    match self {
      Self::BadRequest => StatusCode::BAD_REQUEST,
      Self::Unauthorized => StatusCode::UNAUTHORIZED,
      Self::Forbidden => StatusCode::FORBIDDEN,
      Self::NotFound | Self::NoRoute => StatusCode::NOT_FOUND,
      Self::Gone => StatusCode::GONE,
      Self::ArgumentsMisfit => StatusCode::UNPROCESSABLE_ENTITY,
      Self::Failed => StatusCode::INTERNAL_SERVER_ERROR,
      Self::ToolFailed => StatusCode::BAD_GATEWAY,
    }
  }

  /// The code as it stands on the wire.
  pub fn as_str(self) -> &'static str {
    self.into()
  }
}

/// The body of every failure and refusal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WireError {
  pub code: ErrorCode,
  /// Why, as an English sentence.
  pub error: String,
}

impl WireError {
  pub fn new(code: ErrorCode, error: impl Into<String>) -> Self {
    Self {
      code,
      error: error.into(),
    }
  }
}
