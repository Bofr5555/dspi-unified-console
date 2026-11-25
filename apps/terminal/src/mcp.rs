//! `uconsole mcp`: the stdio relay for AI agents (WP MCP3, MPLAN 3).
//!
//! An agent host that starts local MCP servers as programs (Claude Desktop,
//! most IDE agents) runs `uconsole mcp`. It relays the agent's messages to
//! uConsole for DSPi's loopback endpoint and the answers back (`dspi_mcp::relay`):
//! the agent works on what the app has open, and the user sees every change
//! live. Nothing here opens a device.
//!
//! - **Pairing.** The first run, or one after the app forgot this agent,
//!   asks uConsole for DSPi: it shows the agent's name and a code, which this
//!   prints on standard error; the user clicks Allow. The token is kept in
//!   `<config dir>/uconsole/mcp-token`, a file only this user can read (as
//!   the DSPi Link tokens are).
//! - `--port <n>` when the app's Settings, AI agents names another port than
//!   47860.
//! - Standard output carries the MCP messages only; the relay's words go to
//!   standard error. It ends when standard input closes.

use std::path::PathBuf;

use dspi_cmd::COMMAND;
use dspi_mcp::http::DEFAULT_PORT;
use dspi_mcp::relay::{self, FileTokenKeeper, RelayConfig};

use crate::exit;

/// Where the relay keeps its token.
fn token_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(COMMAND).join("mcp-token"))
}

/// `uconsole mcp [--port <n>]`.
pub fn run(flags: &[&str]) -> u8 {
    if flags.iter().any(|f| matches!(*f, "--help" | "-h")) {
        usage();
        return exit::OK;
    }
    let port = match crate::flag_value(flags, "--port") {
        None => DEFAULT_PORT,
        Some(p) => match p.parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                eprintln!("{COMMAND} mcp: --port takes a port number, 1 to 65535");
                return exit::USAGE;
            }
        },
    };
    let Some(path) = token_path() else {
        eprintln!("{COMMAND} mcp: this system has no configuration folder to keep the token in");
        return exit::USAGE;
    };
    let mut keeper = FileTokenKeeper::new(path);
    let cfg = RelayConfig {
        port,
        fallback_name: format!("{COMMAND} mcp"),
        ..RelayConfig::default()
    };
    // Not the TM15 macros: the relay writes whole MCP messages itself, and a
    // closed standard output ends it (the agent has gone).
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout().lock();
    let mut log = std::io::stderr().lock();
    match relay::run(cfg, &mut keeper, &mut input, &mut output, &mut log) {
        Ok(()) => exit::OK,
        Err(_) => exit::TRANSPORT,
    }
}

fn usage() {
    println!(
        "\
{COMMAND} mcp - let an AI agent use uConsole for DSPi

USAGE:
    {COMMAND} mcp [--port <n>]

An agent host starts this as a local MCP server (stdio). It relays to
uConsole for DSPi, which must be running with Settings, AI agents turned on.
The first time, uConsole for DSPi asks you to allow the agent: check that it
shows the code printed here. --port: the port set in uConsole for DSPi
(default {DEFAULT_PORT}).

Claude Desktop, in claude_desktop_config.json:
    {{\"mcpServers\": {{\"dspi\": {{\"command\": \"{COMMAND}\", \"args\": [\"mcp\"]}}}}}}"
    );
}

#[cfg(test)]
mod tests_wp_mcp3 {
    use super::*;

    #[test]
    fn wp_mcp3_a_bad_port_is_a_usage_error() {
        assert_eq!(run(&["mcp", "--port", "x"]), exit::USAGE);
        assert_eq!(run(&["mcp", "--port", "0"]), exit::USAGE);
        assert_eq!(run(&["mcp", "--help"]), exit::OK);
    }

    #[test]
    fn wp_mcp3_the_token_lives_beside_the_link_tokens() {
        if let Some(p) = token_path() {
            assert!(p.ends_with(format!("{COMMAND}/mcp-token")));
        }
    }
}
