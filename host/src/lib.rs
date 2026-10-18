//! The tool host: one process serving every session's MCP tools, each session known by its bearer
//! token (docs/mcp-host.md).

pub mod error;
pub mod handler;
pub mod lock;
pub mod placed;
pub mod records;
pub mod residency;
pub mod running;
pub mod server;
pub mod toolbook;
pub mod unix_http;
