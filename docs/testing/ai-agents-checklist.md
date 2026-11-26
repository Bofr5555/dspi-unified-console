# AI agents: the owner's check on real agent hosts

*For uConsole for DSPi 0.16.0. Moved out of the user manual ([`docs/manual/ai-agents.md`](../manual/ai-agents.md)):
the checks only a person with real agent hosts can make before a release.*

For the owner, on real agent hosts, before the release. Use the demo device or a device with its limits profile set and
the volume at 80 % or less. These steps change your own agent configuration; undo them afterwards if you want.

**Linux, Claude Code over HTTP**
1. Settings › AI Agents: turn agents on. **Listening on** shows `127.0.0.1` and `[::1]` with port 47860.
2. **Add Client…** "Claude Code". Add it with the `.mcp.json` of [section 2.2 of the manual](../manual/ai-agents.md#22-claude-code) and `DSPI_MCP_TOKEN` set.
3. In Claude Code, `/mcp` lists `dspi` as connected with its tools. Ask "list my DSPi devices" (`dspi_list_devices`).
   The agent reads the guide first (`dspi://guide`, or `dspi_get_guide`) and counts outputs from 1, as the app does.
4. Ask for a quiet change ("lower output 1 by 1 dB"): it happens at once; the activity and the History show it with the
   **AI** tag. **Undo** in the activity puts it back. Before the Undo, once the agent rests, the strip says the change is
   not saved yet ("Changed … not saved yet") with **Save to Device**.
5. Ask for a louder change ("raise output 1 by 3 dB"): the question appears with its heading in the screen's words
   ("Claude Code wants to make … louder (Demo 1)") and the change marked ▲; **Deny**: nothing changes, the agent says so.
   Ask again; **Allow**: the change happens.
6. Ask for night mode on two devices at once (loudness and the leveller on each): the questions come as one sheet,
   grouped by device ("Claude Code wants to make 4 changes"). **Allow** answers exactly the rows shown. Try **Answer One
   by One** once too.
7. Ask for something the speaker rules refuse ("set the tweeter's high-pass to 500 Hz" on the demo): nothing changes,
   no question is asked, and the strip says "Refused: …" for a few seconds.
8. **Pause**: a change is not made, the agent says it is paused; **Resume**. **Remove this client**: the next request gets 401.

**Linux or macOS, Claude Desktop over stdio**
9. Add the config by hand, as in [section 2.1 of the manual](../manual/ai-agents.md#21-claude-desktop) (the
   **Configuration** field and its **Copy** button), and start Claude Desktop again. Ask for something: the app asks
   **Allow this AI agent?** with a code, and the agent says the same code in the chat (it is also in Claude Desktop's
   MCP log). **Allow**. The tools appear in Claude Desktop.
10. Quit and start Claude Desktop again: it connects without asking.
11. Close the app: Claude Desktop's next tool call says the app is not running. Start it again: calls work again.
12. Ask for two louder changes at once: they come as two questions, one after the other (the relay handles one request
    at a time), not as one sheet.

**Install in Claude Desktop…** (never tried in a real Claude Desktop yet; the extension is not signed)
13. Remove the hand-written `dspi` entry first. Settings › AI Agents › Claude Desktop › **Install in Claude Desktop…**:
    Claude Desktop opens its install sheet for "uConsole for DSPi". Note any warning about the unsigned extension.
    **Install**, then repeat steps 9 and 4. Repeat on Linux (AppImage and installed package), macOS and Windows.
14. Linux AppImage: move the AppImage (or update it to a new file name). The extension stops working; **Install in
    Claude Desktop…** again makes it work. If Claude Desktop does not take the extension at all, write down what it
    said: the manual's fallback is the hand-written entry.

**Windows, Claude Desktop over stdio** (the GUI program as a stdio server; not proven on a real Windows yet)
15. Add `"command": "C:\\Users\\<you>\\AppData\\Local\\Programs\\uConsole for DSPi\\uconsole-for-dspi.exe", "args":
    ["--mcp"]`. Check that no console window flashes, the pairing question appears in the running app, and the tools
    appear in Claude Desktop. Repeat step 4.

**Questions and settings**
16. **Ask me: Never**: a louder change happens without a question. Removing a tweeter output's high-pass is refused.
17. **Ask me: About risky changes**: ask for two measurements on one device. The first question says Allow also allows
    test signals there for 10 minutes; **Allow**: the second is not asked. Settings › AI Agents shows the permission with
    the time left; **Revoke**: the next measurement is asked again. A louder change is still asked.
18. **Ask me: About every change**: a quiet change is asked about too.
19. **Stop Agents** turns agents off; the agents' next call fails; turning them on again works, and the agent says you
    stopped it (its next result tells it). **Forget Agent** in the AI popover (Modern): the agent has to be allowed again.
20. Installer lock on (Settings › Installer, with a PIN): the agent can change the volume, mute, the source and load a
    preset; anything else is refused, and "out 3 is the woofer" (speaker roles from your words) is refused too.

**This computer's sound settings**
21. With a DSPi on USB that is not the default output and music playing to the laptop's speakers, ask "is my DSPi
    getting the sound?". The agent says the DSPi is not the default output, names where the music plays, and tells
    you how to switch; it does not switch anything. Make the DSPi the default and ask again: it says so, and names the
    app playing to it. Repeat on Windows and on macOS (macOS before 14.2: no app list, only whether something plays).

Write down the agent host versions and anything that differed.
