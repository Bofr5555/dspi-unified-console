//! The DSPi console's MCP server (MPLAN section 3, MCPDES).
//!
//! Sans-IO: [`Server::handle`] takes one JSON-RPC message from a client and
//! the time, and gives the reply. The transports (loopback HTTP and the stdio
//! relay, WP MCP3) and the app's UI (MCP4) sit around it; tests call it
//! directly. No async runtime and no MCP library: the protocol is written
//! here for both spec eras (owner decision 2026-11-21).
//!
//! | Module | What | WP |
//! |---|---|---|
//! | [`protocol`] | JSON-RPC 2.0, the two eras, error codes | MCP1 |
//! | [`server`] | clients, dispatch, the rate limits, the activity log | MCP1 |
//! | [`registry`] | the tools, the policy seam, the one scoped call wrapper | MCP1 |
//! | [`tools`] | the tools by area | MCP1 (read), MCP2a–c, MCP5 |
//! | [`resources`] | `dspi://` resources | MCP1 |
//! | [`prompts`] | user-invoked recipes | MCP1 |
//! | [`untrusted`] | the sanitiser for text nobody vetted | MCP1 |
//! | [`activity`] | what the agents did | MCP1 |
//! | [`rate`] | token buckets on the caller's clock | MCP1 |
//! | `http` | the loopback listener (feature `http`) | MCP3 |
//! | `auth` | per-client tokens and pairing (feature `http`) | MCP3 |
//! | `relay` | the stdio relay to the listener (feature `http`) | MCP3 |
//!
//! Safety rests on the core, not on this crate's good behaviour: every tool
//! call runs inside a `dspi_core::actor::ActorScope` on the device it
//! addresses ([`registry::run`]), so the core attributes, gates and hooks
//! the agent's writes; the [`registry::Policy`] decides what an agent may
//! write. Until MCP2a the policy is [`registry::ReadOnlyPolicy`].
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod activity;
pub mod prompts;
pub mod protocol;
pub mod rate;
pub mod registry;
pub mod resources;
pub mod server;
pub mod tools;
pub mod untrusted;
// -- WP MCP3 -- the transports
#[cfg(feature = "http")]
pub mod auth;
#[cfg(feature = "http")]
pub mod http;
#[cfg(feature = "http")]
pub mod relay;

pub use registry::{Policy, ReadOnlyPolicy};
pub use server::{Server, ServerConfig};

/// One connected client of a [`Server`] (a transport's connection or
/// paired token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u64);

#[cfg(test)]
mod tests_wp_mcp1;
// -- WP MCP3 --
#[cfg(test)]
#[cfg(feature = "http")]
mod tests_wp_mcp3;
