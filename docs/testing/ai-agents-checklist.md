# AI agents: the owner's check on real agent hosts

*Moved out of the user manual ([`docs/manual/ai-agents.md`](../manual/ai-agents.md)) for 0.15.0; written for wave 6
(MCP6), with the questions of ODEC and the sound settings of FEAT3.*

For the owner, on real agent hosts, before the release. Use the demo device or a device with its limits profile set and
the volume at 80 % or less. These steps change your own agent configuration; undo them afterwards if you want.

**Linux, Claude Code over HTTP**
1. Settings › AI Agents: turn agents on. **Listening on** shows `127.0.0.1` and `[::1]` with port 47860.
2. **Add Client…** "Claude Code". Add it with the `.mcp.json` of [section 2.2 of the manual](../manual/ai-agents.md#22-claude-code) and `DSPI_MCP_TOKEN` set.
3. In Claude Code, `/mcp` lists `dspi` as connected with its tools. Ask "list my DSPi devices" (`dspi_list_devices`).
4. Ask for a quiet change ("lower output 1 by 1 dB"): it happens at once; the activity and the History show it with the
   **AI** tag. **Undo** in the activity puts it back.
5. Ask for a louder change ("raise output 1 by 3 dB"): the question appears; **Deny**: nothing changes, the agent says so.
   Ask again; **Allow**: the change happens.
6. **Pause**: a change is not made, the agent says it is paused; **Resume**. **Remove this client**: the next request gets 401.

**Linux or macOS, Claude Desktop over stdio**
7. Add the config of [section 2.1 of the manual](../manual/ai-agents.md#21-claude-desktop) and start Claude Desktop again. The app asks **Allow this AI agent?** with the code
   Claude Desktop's MCP log shows. **Allow**. The tools appear in Claude Desktop.
8. Quit and start Claude Desktop again: it connects without asking.
9. Close the app: Claude Desktop's next tool call says the app is not running. Start it again: calls work again.

**Windows, Claude Desktop over stdio** (the GUI program as a stdio server; not proven on a real Windows yet)
10. Add `"command": "C:\\Users\\<you>\\AppData\\Local\\Programs\\uConsole for DSPi\\uconsole-for-dspi.exe", "args":
    ["--mcp"]`. Check that no console window flashes, the pairing question appears in the running console, and the tools
    appear in Claude Desktop. Repeat step 4.

**Questions and settings**
11. **Ask me: Never**: a louder change happens without a question. Removing a tweeter output's high-pass is refused.
11a. **Ask me: About risky changes** (WP ODEC): ask for two measurements on one device. The first question says Allow also
    allows test signals there for 10 minutes; **Allow**: the second is not asked. The AI page shows the permission with the
    time left; **Revoke**: the next measurement is asked again. A louder change is still asked.
12. **Ask me: About every change**: a quiet change is asked about too.
13. **Stop** turns agents off; the agents' next call fails; turning them on again works, and the agent says you stopped
    it (its next result tells it).

**This computer's sound settings** (WP FEAT3)
14. With a DSPi on USB that is not the default output and music playing to the laptop's speakers, ask "is my DSPi
    getting the sound?". The agent says the DSPi is not the default output, names where the music plays, and tells
    you how to switch; it does not switch anything. Make the DSPi the default and ask again: it says so, and names the
    app playing to it. Repeat on Windows and on macOS (macOS before 14.2: no app list, only whether something plays).

Write down the agent host versions and anything that differed.
