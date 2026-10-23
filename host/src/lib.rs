//! The tool host: one process serving every session's MCP tools, each session known by its bearer
//! token (docs/mcp-host.md).

pub mod command;
pub mod error;
pub mod external;
pub mod handler;
pub mod install;
pub mod placed;
pub mod records;
pub mod registry;
pub mod residency;
pub mod running;
pub mod schedule;
pub mod server;
pub mod toolbook;
