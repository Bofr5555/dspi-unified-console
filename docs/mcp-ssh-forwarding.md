# Using DSPi Console's AI agents from another computer (SSH)

DSPi Console serves AI agents only on its own computer: it listens on `127.0.0.1` and
`::1` (port 47860 unless you changed it in Settings › AI agents) and on no network
address, and there is no setting to change that. To use an agent on another computer,
carry the connection over SSH. SSH encrypts it, and you sign in to the DSPi computer as
yourself, so nothing new is opened on your network.

You need an SSH server on the computer that runs DSPi Console (call it `dspi-pc`) and
an SSH client on the computer that runs the agent. macOS and Linux have both. Windows 10
and 11 have the client built in and the server as the optional feature "OpenSSH Server".

DSPi Console must be running on `dspi-pc` with Settings › AI agents turned on.

## Agents that connect over HTTP (Claude Code, Cursor, VS Code)

1. On `dspi-pc`, in Settings › AI agents, choose **Add client…** and copy the token it
   shows. It is shown once.
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

## Agents that start a program (Claude Desktop)

Run the relay on `dspi-pc` through SSH. This needs the DSPi Terminal (`uconsole`)
installed there, and an SSH key, so that SSH does not stop to ask for a password:

```json
{
  "mcpServers": {
    "dspi": { "command": "ssh", "args": ["-T", "you@dspi-pc", "uconsole", "mcp"] }
  }
}
```

The first time, DSPi Console on `dspi-pc` asks you to allow the agent. Check that it
shows the same code as the agent's log, then click **Allow**. The relay keeps its token on
`dspi-pc`.

## Good to know

- Everyone who can sign in to `dspi-pc` as you can reach the agents' endpoint, as on
  that computer itself. The token is what tells agents apart. Remove a token you no longer
  use in Settings › AI agents.
- Close the tunnel (Ctrl-C) when you are done.
- Do not forward the port with `-g`, or with a router or firewall rule: anyone on that
  network could then try tokens against it.
