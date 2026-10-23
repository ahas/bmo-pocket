//! The tool host's control socket as a caller reaches it (docs/api.md, 「The control socket」): the wire types the
//! host answers with, one call per route over the socket, the harness plugin's fixed tools, and the OpenAPI
//! document generated from those types, which the TypeScript client is generated from.

mod client;
mod error;
pub mod harness;
mod openapi;
pub mod transport;
mod wire;

pub use bmo_tool::{Granted, Session, SessionRequest};
pub use client::{Failure, Pocket, default_socket};
pub use error::{ErrorCode, WireError};
pub use openapi::{openapi, openapi_json};
pub use wire::{
  Claim, ClaimRequest, Health, Listing, NameQuery, Namespace, NamespacePage, ReachedBy, Schedule,
  SchedulePage, Server, ServerPage, SessionPage, StandingSession, TimeQuery, Tool, ToolCall,
  ToolCallRequest, ToolHints, ToolPage, ToolResult, ToolSession, Transport,
};
