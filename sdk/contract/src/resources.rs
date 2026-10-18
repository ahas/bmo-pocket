//! The documents a placed executable serves as resources. Its manifest declares the namespace they stand
//! under and the capabilities a session must hold to be shown them (docs/mcp-host.md, 「A call」); the
//! executable answers only what they are.

use serde_json::Value;

use crate::session::Session;
use crate::tool::Pending;

/// The documents an executable serves, each listed in the protocol's own shape and read for the session that
/// asks.
pub trait Resources: Send + Sync {
  fn list<'a>(&'a self, session: &'a Session) -> Pending<'a, Vec<Value>>;
  /// One document's contents in the protocol's shape, or why it cannot be read.
  fn read<'a>(&'a self, session: &'a Session, uri: &'a str) -> Pending<'a, Result<Value, String>>;
}
