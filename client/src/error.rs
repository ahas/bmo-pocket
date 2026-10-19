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
  /// A request on the tools face carries no bearer token.
  NoToken,
  /// The token names no record: never issued, revoked, or past its deadline.
  UnknownToken,
  /// No placed entry is declared under the namespace.
  UnknownNamespace,
  /// No entry of the namespace declares a listing line.
  NoListing,
  /// No entry of that full name, or none the record holds.
  UnknownEntry,
  /// What was handed does not read as a session record.
  MalformedRecord,
  /// A body or a query that does not read as the route asks.
  MalformedRequest,
  /// The arguments do not fit the entry's schema in the form held.
  ArgumentsMisfit,
  /// The grant lock refused the call.
  GrantRefused,
  /// The tool exited uncleanly, answered what is not a tool result, or ran past its timeout.
  ToolFailed,
  /// A record could not be kept or taken back on disk.
  UnwritableRecord,
  /// A path the control socket does not serve.
  UnknownRoute,
  /// A method a served path does not take.
  MethodNotAllowed,
}

impl ErrorCode {
  /// The status it is answered with.
  pub fn status(self) -> StatusCode {
    match self {
      Self::NoToken | Self::UnknownToken => StatusCode::UNAUTHORIZED,
      Self::UnknownNamespace | Self::NoListing | Self::UnknownEntry | Self::UnknownRoute => StatusCode::NOT_FOUND,
      Self::MalformedRecord | Self::MalformedRequest => StatusCode::BAD_REQUEST,
      Self::ArgumentsMisfit => StatusCode::UNPROCESSABLE_ENTITY,
      Self::GrantRefused => StatusCode::FORBIDDEN,
      Self::ToolFailed => StatusCode::BAD_GATEWAY,
      Self::UnwritableRecord => StatusCode::INTERNAL_SERVER_ERROR,
      Self::MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
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
