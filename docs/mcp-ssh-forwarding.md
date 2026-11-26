# Using uConsole for DSPi's AI agents from another computer (SSH)

*Part of the AI agents manual: [docs/manual/ai-agents.md](manual/ai-agents.md), which covers turning agents on, each
agent's configuration, the questions and undo. Written for uConsole for DSPi 0.16.0.*

uConsole for DSPi serves AI agents only on its own computer: it listens on `127.0.0.1` and
`::1` (port 47860 unless you changed it in Settings › AI Agents) and on no network
address, and there is no setting to change that. To use an agent on another computer,
carry the connection over SSH. SSH encrypts it, and you sign in to the DSPi computer as
yourself, so nothing new is opened on your network.

You need an SSH server on the computer that runs uConsole for DSPi (call it `dspi-pc`) and
an SSH client on the computer that runs the agent. macOS and Linux have both. Windows 10
and 11 have the client built in and the server as the optional feature "OpenSSH Server".

uConsole for DSPi must be running on `dspi-pc` with Settings › AI Agents turned on.

## Agents that connect over HTTP (Claude Code, Cursor, VS Code)

1. On `dspi-pc`, in Settings › AI Agents › **Clients**, choose **Add Client…** and copy
   the token it shows. It is shown once.
2. On the agent's computer, open the tunnel and leave it open while you work:

   ```
   ssh -N -L 47860:127.0.0.1:47860 you@dspi-pc
   ```

   If 47860 is taken on the agent's computer, pick another local port, such as
   `-L 50000:127.0.0.1:47860`, and use that port below.
3. Point the agent at `http://127.0.0.1:47860/mcp` with the header
   `Authorization: Bearer <token>`. For Claude Code:

   ```
   claude mcp add --transport http dspi http://127.0.0.1:47860/mcp \
     --header "Authorization: Bearer <token>"
   ```

   That command line, token included, stays in your shell's history. To keep the token out
   of it, use the `.mcp.json` with `${DSPI_MCP_TOKEN}` from the manual
   ([Claude Code](manual/ai-agents.md#22-claude-code)) and set the variable in the
   environment Claude Code starts in.

## Agents that start a program (Claude Desktop)

**Install in Claude Desktop…** in Settings › AI Agents is for a Claude Desktop on the same computer: the extension it
writes starts the app there. For a Claude Desktop on another computer, add the entry below by hand instead.

Run the relay on `dspi-pc` through SSH. This needs the Terminal (`uconsole`)
installed there, and an SSH key, so that SSH does not stop to ask for a password:

```json
{
  "mcpServers": {
    "dspi": { "command": "ssh", "args": ["-T", "you@dspi-pc", "uconsole", "mcp"] }
  }
}
```

The first time, uConsole for DSPi on `dspi-pc` asks **Allow this AI agent?** with a
four-digit pairing code. Check that the agent shows the same code (in the chat when it
asks for a tool while the question waits, or in the agent app's log of the dspi server),
then click **Allow**. The relay keeps its token on `dspi-pc`.

The relay handles one request at a time, so this agent's questions come one by one,
never grouped on one sheet. Agents over the HTTP tunnel above get grouped sheets as on
`dspi-pc` itself.

## Good to know

- Everyone who can sign in to `dspi-pc` as you can reach the agents' endpoint, as on
  that computer itself. The token is what tells agents apart. Remove a token you no longer
  use in Settings › AI Agents › **Clients** (**Remove this client**).
- The endpoint cannot prove which program is listening on the port. If uConsole for DSPi is not
  running on `dspi-pc` and another account there takes port 47860, an agent that connects
  through the tunnel hands that program its token. On a computer other people sign in to,
  pick another port in Settings › AI Agents (and in `-L` above), and if you suspect this
  happened, remove the client there (its token stops at once) and add or pair it again.
- Close the tunnel (Ctrl-C) when you are done.
- Do not forward the port with `-g`, or with a router or firewall rule: anyone on that
  network could then try tokens against it.
