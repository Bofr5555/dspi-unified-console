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
//! | [`policy`] | what an agent may write, what is asked, what is refused | MCP2a |
//! | [`confirm`] | calls held for the user's word | MCP2a |
//! | [`tools`] | the tools by area | MCP1 (read), MCP2a–c, MCP5 |
//! | [`resources`] | `dspi://` resources | MCP1 |
//! | [`prompts`] | user-invoked recipes | MCP1 |
//! | [`untrusted`] | the sanitiser for text nobody vetted | MCP1 |
//! | [`activity`] | what the agents did | MCP1 |
//! | [`host_audio`] | this computer's audio, read only | FEAT3 |
//! | [`rate`] | token buckets on the caller's clock | MCP1 |
//! | `http` | the loopback listener (feature `http`) | MCP3 |
//! | `auth` | per-client tokens and pairing (feature `http`) | MCP3 |
//! | `relay` | the stdio relay to the listener (feature `http`) | MCP3 |
//!
//! Safety rests on the core, not on this crate's good behaviour: every tool
//! call runs inside a `dspi_core::actor::ActorScope` on the device it
//! addresses ([`registry::run`]), so the core attributes, gates and hooks
//! the agent's writes; the [`registry::Policy`] decides what an agent may
//! write. The default is [`registry::ReadOnlyPolicy`]; the app runs agents
//! under [`policy::AgentPolicy`] once the user has turned them on (MCP2a).
#![warn(
    missing_docs,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

pub mod activity;
pub mod confirm; // -- WP MCP2a --
pub mod host_audio; // -- WP FEAT3 --
pub mod policy; // -- WP MCP2a --
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
// -- WP MCP2a --
pub use policy::{AgentPolicy, ConfirmMode};
pub use server::Handled;

/// One connected client of a [`Server`] (a transport's connection or
/// paired token).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u64);

#[cfg(test)]
mod tests_wp_mcp1;
// -- WP MCP2a --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_mcp2a;
// -- WP MCP2b --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_mcp2b;
// -- WP MCP3 --
#[cfg(test)]
#[cfg(feature = "http")]
mod tests_wp_mcp3;
// -- WP MCP2c --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_mcp2c;
// -- WP MCP4 --
#[cfg(test)]
#[cfg(feature = "http")]
#[allow(clippy::arithmetic_side_effects)]
mod tests_wp_mcp4;
// -- WP FEAT2 --
#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_feat2;
// -- WP MCP5 --
#[cfg(test)]
#[allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation
)]
mod tests_wp_mcp5;
// -- WP FEAT1 --
#[cfg(test)]
#[cfg(feature = "link")]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests_wp_feat1;
// -- WP FEAT3 --
#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests_wp_feat3;
