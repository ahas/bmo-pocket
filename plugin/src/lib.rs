//! The tool host's harness plugin, `ahas-pocket` (docs/api.md, 「The harness plugin」).
//!
//! `bmo-pocket mcp` serves the MCP server here on standard input and output, holding one tool session's token: the
//! four fixed tools, each a call of one route through bmo-pocket-client, and every hosted tool of the session's
//! namespaces, each call of one a `POST /tool-calls`. The skill beside this crate tells a harness when and how to use
//! it.

mod server;
mod tools;

pub use server::{PocketServer, serve_stdio};
pub use tools::{hosted_names, refusal};
