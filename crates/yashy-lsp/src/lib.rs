//! The Yashy language server.
//!
//! Run it with `yashy-lsp`; an editor launches it and speaks LSP over stdio.
//! [`serve`] is the whole server, so the tests drive it through an in-memory
//! pipe instead of spawning a process.

pub mod server;
pub mod text;

// One implementation of JSON framing, shared with the driver's MCP server.
pub use yashy_common::json_rpc as json;

pub use server::serve;
pub use text::{path_to_uri, uri_to_path};
