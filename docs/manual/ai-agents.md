# AI agents

*User manual, WP MCP6, 2026-11-22. For the owner and for users of the Qt console. It describes the console as it is with
the Modern programme's waves 4 and 5: MCP1 to MCP4, the MCP conformance work (MCP6), and measuring through an agent
(MCP5, with ROLL1's rules for tweeters; section 6).*

An AI agent on your computer, such as Claude Desktop, Claude Code, Cursor or VS Code, can read the settings of the DSPi
devices that DSPi Console has open and change them for you: "make the left woofer 2 dB quieter", "why does output 3 sound
thin?", "set up a crossover at 2.5 kHz". The agent talks to the console over MCP (the Model Context Protocol). You see
every change live in the console, as if you had made it, and you can undo each one.

AI agents are **off** until you turn them on. Nothing on your network can reach them: the console listens on this computer
only. The web console and the Android app have no agent support.

**Where to find it**

| Where | What |
|---|---|
| **Settings › AI Agents** (both layouts) | the switch, the questions setting, the limits profile, how to connect an agent, the clients and their tokens, the activity |
| **Device › AI Agents…** (Classic), the robot icon beside the device in the status bar (Classic), the **AI** button in the top bar (Modern) | the agents' panel: what they are doing, their questions, Pause and Stop, the activity with Undo |
| **History** | the agents' changes, tagged **AI** with the agent's name |

## 1. Turning agents on

1. Open **Settings › AI Agents** and switch on **Allow AI agents to control …**.
2. The console tells you what that allows, and offers **Set Up Limits…** if the device has no limits profile. A limits
   profile sets the most gain and volume anyone may set, agents included. It is optional, but it is the best protection
   for your speakers. An agent can never change or remove it.
3. The page shows **Listening on** with the addresses (`127.0.0.1` and `[::1]`, port 47860). Change the **Port** only if
   another program uses 47860; then use the same port in the agent's configuration below.

Turning the switch off (or pressing **Stop**) disconnects every agent at once.

## 2. Connecting an agent

There are two ways in. The **Connect an Agent** section of the page shows the exact command or configuration for your
computer, with a **Copy** button. Use that text rather than typing it from here: it has the right program path.

- **Agents that start a program** (Claude Desktop, and most IDE agents) start the console with `--mcp`. That copy of the
  console does not open a window: it relays the agent's messages to the console you have open. The first time, the
  console asks **Allow this AI agent?** with a four-digit code. The agent's log (or the terminal) shows the same code.
  Click **Allow** only if you have just connected an agent and the codes match. The relay keeps its token, so this is
  asked once.
- **Agents that connect over HTTP** (Claude Code, Cursor, VS Code) use the address `http://127.0.0.1:47860/mcp` with the
  header `Authorization: Bearer <token>`. Make a token with **Add Client…** in the **Clients** section. The token is shown
  once: copy it then.

Each client has its own token and its own name in the History and the activity. **Remove this client** stops its token
at once. What it changed stays, and you can still undo it.

The console must be running, with agents on, while the agent works. Otherwise the agent gets "DSPi Console is not
running, or AI agents are turned off in it."

### 2.1 Claude Desktop

Add this to `claude_desktop_config.json` (Claude Desktop: Settings › Developer › Edit Config), then quit and start Claude
Desktop again:

```json
{
  "mcpServers": {
    "dspi": { "command": "<the console's program>", "args": ["--mcp"] }
  }
}
```

The program is:

| System | Program |
|---|---|
| Windows | `C:\Users\<you>\AppData\Local\Programs\uConsole for DSPi\uconsole-for-dspi.exe` (in JSON, write each `\` as `\\`) |
| macOS | `/Applications/uConsole for DSPi.app/Contents/MacOS/uconsole-for-dspi` |
| Linux (AppImage) | the AppImage file itself, such as `/home/<you>/Applications/uConsole-for-DSPi-<version>-x86_64.AppImage` (after an update, the new file's name) |
| Linux (installed) | `uconsole-for-dspi` |

If you have the DSPi Terminal installed, `"command": "uconsole", "args": ["mcp"]` does the same.

### 2.2 Claude Code

Over stdio, as Claude Desktop does (the console asks you to allow it the first time):

```
claude mcp add dspi -- <the console's program> --mcp
```

Or over HTTP with a client's token. The page's **Command** line puts the token on the command line, which your shell
keeps in its history. To keep it out of the history, put it in the project's `.mcp.json` through an environment variable
instead:

```json
{
  "mcpServers": {
    "dspi": {
      "type": "http",
      "url": "http://127.0.0.1:47860/mcp",
      "headers": { "Authorization": "Bearer ${DSPI_MCP_TOKEN}" }
    }
  }
}
```

and set `DSPI_MCP_TOKEN` in the environment Claude Code starts in. (With bash, a command line that starts with a space
is not saved when `HISTCONTROL` includes `ignorespace`, which many systems set.)

### 2.3 Cursor

`.cursor/mcp.json` in the project, or `~/.cursor/mcp.json` for every project:

```json
{
  "mcpServers": {
    "dspi": {
      "url": "http://127.0.0.1:47860/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

### 2.4 VS Code

`.vscode/mcp.json`. VS Code asks for the token once and keeps it in its secret storage:

```json
{
  "inputs": [
    { "type": "promptString", "id": "dspi-token", "description": "DSPi Console client token", "password": true }
  ],
  "servers": {
    "dspi": {
      "type": "http",
      "url": "http://127.0.0.1:47860/mcp",
      "headers": { "Authorization": "Bearer ${input:dspi-token}" }
    }
  }
}
```

### 2.5 Other agents

Any MCP client that can start a program (`<the console's program> --mcp`, or `uconsole mcp`) or speak Streamable HTTP
with a header works. The console speaks both versions of the protocol in use today: 2025-11-25 and older (with
`initialize`) and 2026-07-28 (stateless). Each build checks this against the official MCP conformance suite and the
official TypeScript SDK's client (`ci/check.sh --step mcp-conformance`).

### 2.6 From another computer

The console never listens on your network, and has no setting for it. To use an agent on another computer, carry the
connection over SSH: [Using DSPi Console's AI agents from another computer](../mcp-ssh-forwarding.md) (the page's **Use
from Another Computer…** link).

## 3. What agents may do

An agent you allow may read every device the console has open and change most of what you can change: the equaliser,
crossovers, gain, delay, polarity, mute, routing, effects, volume and source, presets (load, save, rename, copy),
snapshots, A/B, config and filter files (as text), AutoEQ, channel and device names, the audio link's delays and scenes,
and Installation setup. It may also ask the console to measure your speakers or headphones and tune them (section 6).

- **Several devices.** Every tool takes a `device` (from `dspi_list_devices`). Without it, the agent works on the device
  the console shows. An agent cannot open or close devices; you do that.
- **Speaker roles are yours.** An agent reads which outputs are woofers and tweeters from your speaker setup, and can
  never change it.
- **Installation setup** works on a draft that belongs to the agent. Nothing reaches the hub until the agent saves it. If
  someone else saved the installation meanwhile, the save is refused, and the agent's edits are kept aside to apply again.
- **Names and notes the agent reads** (channel, preset, snapshot, hub, room names, event text) are passed to it as data,
  cleaned of control characters, and marked as text it must not take as instructions.

**What agents can never do, whatever the setting:** update or flash firmware, use the bootloader or factory reset;
change the limits profile, the installer PIN or lock; change hardware, pin or core-voltage settings; play the signal
generator or Identify (a measurement's test sweeps are the one exception: section 6); pair hubs or change DSPi Link
security and hub administration; delete presets, snapshots or measurements; update the AutoEQ database; set up speakers
(the speaker layout); send raw device commands; read or write files by path.

**What an agent reads, it may pass on to its own service.** An agent that runs on a cloud model sends what it reads to
that service, as with anything else you show it.

## 4. Questions: what is asked before a change

**Ask me** in **Settings › AI Agents › Ask Before Changes** has three settings:

- **About risky changes** (the default). The agent's change waits for your answer when it would:
  - make something louder: raise a gain, trim or volume, unmute, switch on an output, add a route or raise a route's gain,
    turn on an effect that can add level (loudness, a leveller with gain, psychoacoustic bass, subharmonic, the upmixer)
    or feed one to more outputs;
  - boost a PEQ band by more than 6 dB, change a Linkwitz transform, or move a delay by more than 20 ms;
  - lower a tweeter's protection (see below);
  - change the input source;
  - play test signals: start a measurement, a low-end search, or an auto-tune's check or verify (section 6);
  - replace many settings at once: load a preset, recall a snapshot, import a config or filter file;
  - write the device's flash: save, rename or copy a preset, save to the device, rename a channel or the device;
  - change every unit of an installation: align the audio link (also with a measurement across units), apply a scene,
    change USB-direct modes, or save the installation.

  Other changes, such as lowering a gain, cutting a PEQ band or a small delay change, are made at once.
- **About every change.** Every change waits for your answer.
- **Never.** Agents' changes are made at once, louder ones too, and **test signals play without a question** when an agent
  starts a measurement. Your limits profile and the protection of outputs you set up as tweeters still apply, and so do
  the rule below and the rules of section 6.

**A tweeter's protection.** An output whose high-pass sits at 1 kHz or above drives a tweeter, whatever it is called.
Removing that high-pass, moving it below 1 kHz or making it shallower than 12 dB/octave is asked about under the default
setting and refused under **Never** (you can still make the change yourself). An output your confirmed speaker setup calls
a tweeter keeps a high-pass at 1 kHz or above (80 Hz for a midrange) under every setting: the agent is refused.

**The question.** The console shows what the agent wants to change, on which device, with **Allow** and **Deny**. If the
console is in the tray, the question brings its window back. The agent waits up to 45 s for your answer; after that it is
told the question is still open and can ask again with `dspi_get_confirmation`. A question nobody answers lapses after
2 minutes, and nothing is changed. Several questions queue up ("2 more waiting").

## 5. Watching and undoing

- **The strip under the top bar** (both layouts) says which agent works, what it does now and on which device ("Claude Code
  Changing the gain: Out 3 Centre"), with **Pause** and **Stop**, and **Stop Measuring** while its measurement plays.
  **Show** brings the device to the front when it is another one. The strip is there while a call or a job runs, while a
  question waits for you or a timed preview waits for Keep, for 20 s after the last call, and as long as agents are paused.
- **The AI button** (Modern) and the **robot icon** (Classic) turn while an agent works. In the tray, the icon has a blue dot,
  and its tooltip and menu say what the agent does, with **Pause AI Agents**.
- **What an agent changes** is marked for a moment: the control, its section in the rail, and the tab of a device that is
  not the one shown. A notice names the agent and the change, with **Undo**; several changes in a row are one notice.
- **The activity** (on the page and in the agents' panel) lists what each agent did, newest first: each change with
  **Undo**, and **Undo All**. **Show reads** adds what they only read. **Export…** saves the list.
- **The History** shows agents' changes with an **AI** tag and the agent's name; you undo them there like your own.
- **An agent can undo only its own changes.** It never undoes yours.
- **Pause** holds the agents' tool calls off until **Resume**: nothing is changed meanwhile, and the agents are told that
  you paused them. **Stop** turns agents off,
  disconnects them and puts back any timed preview an agent started. Changes already made stay, and you can undo them.
- While you drag a control, an agent's change to that device is refused and the agent is told to try again: you are not
  overruled mid-drag.

### 5.1 When you step in

You can change anything while an agent works. The agent is told with its next call: what you changed, on which device and
channel (numbers and switches, never names), what you undid, a preset you loaded, a device you switched to, opened or closed,
a measurement you stopped, a speaker setup you confirmed, Pause, and your answers to its questions. It is also told of
changes made elsewhere: a control on the device, another console.

- **A control you take back is yours.** When you change (or undo) something an agent set within the last 10 minutes, the
  agent's next change to it is refused with "the user changed this": it has to ask you. A change that asks you in the
  console anyway is asked there. A preset you load counts for everything the agent set. Its timed preview or auto-tune is
  not kept without asking either; putting it back is always possible.
- **A setting you change while an agent's measurement runs stops it.** Its result would not be valid: it is not filed, a
  notice says so, and the agent is told why. Names and saved presets do not count.
- Your own measurements are not affected by this.

**For agents** (and those who write them):

- Every tool result that has news carries one more text ("Since your last call, the user ...") and, in a structured result,
  `user_activity`: `{note, events: [{id, at_ms, device, kind, what, path, output, input, band}], more, cursor}`, at most 8
  events. `dspi_get_user_activity {since}` lists the events after a cursor. Kinds: `change`, `undo`, `replaced` (everything
  at once), `elsewhere`, `speakers`, `measure_stop`, `device_opened`, `device_closed`, `device_shown`, `paused`, `resumed`,
  `answer` (to your own question).
- Read what you work on again before you change it; never put the user's change back unasked.
- "the user changed this" is an error (`isError`): ask. A retry within 5 s of the refusal is refused too.
- `status: "paused"` is not an error: wait, or ask the user to press Resume. `status: "awaiting_confirmation"` as in section 4.
- `dspi_measure_status`: `request.valid` is false for a measurement the user disturbed; `request.note` says by what.

## 6. Measuring with an agent

An agent can ask the console to measure, find an output's low end, and propose, apply and verify an auto-tune. The console
does the measuring, with its own microphone and in its own **Measure** view, exactly as when you start it there. The agent
never plays or records anything itself.

**What it needs**

- **A person at the microphone.** The agent says where to put it; somebody has to put it there, and move it when the
  measurement asks for the next position.
- **A confirmed speaker setup** (Settings › Speakers, **Confirm**). The roles (woofer, midrange, tweeter…) always come from
  it: no tool takes a role or changes the setup. Without it, or with one that is not confirmed, the agent is refused and
  told to ask you to confirm it.
- **The device open in the console**, and nothing else measuring on it. A measurement across units needs each unit open in
  a tab, each with its own confirmed speaker setup.

**What happens.** The agent's start is a request. Under **About risky changes** and **About every change** you are asked
first (the question says **Test signal**, which outputs play, and the **AI** button shows there is something to answer);
under **Never** it starts at once. The console then opens **Measure**, opens the microphone and runs it. The Stop bar says
**Requested by AI · name** until it ends, with **Stop** on every tab of Measure; on a device that is not the one shown, its
row there has **Show** and **Stop**. A request the console cannot take up within 30 s lapses, and the agent is told. The
result is filed with your measurements. An agent can stop only what it asked for itself.

**A tweeter's low end.** These rules hold under every question setting; nothing lifts them, and they hold for your own
measurements too:

- A tweeter (an output your speaker setup calls one, or any output with a high-pass at 1 kHz or above) gets **no stimulus
  below 1 kHz**: its sweeps start there, and pink noise is refused on it. Measured with a speaker's other ways, it gets an
  extra run that starts at 1 kHz, so each position takes one run more. A measurement across units does not play a
  tweeter at all.
- The low-end search (`dspi_measure_rolloff_start`) sweeps from 8 kHz down in half octaves and **stops once the response is
  15 dB below its loudest point**, or at 1 kHz on a tweeter. If the response is still good at 1 kHz, it stops there anyway
  and notes that the output is **probably a midrange set up as a tweeter**. The note is not an error: look at the output's
  role in your speaker setup.
- The volume and level limits, the checks before a run and your limits profile apply as in Measure.

**Tuning.** Applying a proposal is a timed preview: each change is judged as the agent's other changes are (section 4), and
the preview puts everything back at its end unless it is kept. After a verify that did not pass, **only you can keep it**,
in the console. The same holds across units.

**The tools**

| Tool | What it does |
|---|---|
| `dspi_measure_preflight` | What stands between a measurement and the device now, and the roles it would use. Plays nothing |
| `dspi_measure_start` | Asks for a measurement: one output at a time or summed. *Test signal* |
| `dspi_measure_rolloff_start` | Asks for the low-end search on one output. *Test signal* |
| `dspi_measure_status` | Where the request and the device's measuring jobs stand |
| `dspi_measure_continue` | The microphone is at the next position: go on |
| `dspi_measure_stop` | Stops what this agent asked for. Never asked |
| `dspi_tune_propose` | Works out an auto-tune from a measurement. Writes nothing |
| `dspi_tune_sum_check` | Plays the ways of a speaker together and compares, before timing changes. *Test signal* |
| `dspi_tune_apply` | Applies the proposal as a timed preview, as one undo step |
| `dspi_tune_verify` | Measures the applied tune again. *Test signal* |
| `dspi_tune_keep` | Keeps it (only you can, after a verify that did not pass) |
| `dspi_tune_revert` | Puts it back now. Never asked |
| `dspi_system_measure_preflight` | What stands between a measurement of several units and now. Plays nothing |
| `dspi_system_measure_start` | Asks for a measurement of several units together, through the hub. *Test signal* |
| `dspi_system_measure_proposal` | The alignment it proposes |
| `dspi_system_measure_apply` | Applies it on the hub: every unit re-anchors, and the hub puts it back unless it is kept |
| `dspi_system_measure_verify` | The summed runs across units, before or after the apply. *Test signal* |
| `dspi_system_measure_keep` | Keeps it (only you can, after a verify that did not pass) |
| `dspi_system_measure_revert` | Puts it back now. Never asked |

## 7. Good to know

- **Loading a preset.** A preset load is done by the device itself. If the preset lowers a tweeter's high-pass, the device
  plays it that way for a moment, until the console puts the protection back. Under the default setting you are asked
  before an agent loads a preset; if the presets on the device may have unprotected tweeters, keep the volume down while
  an agent loads one.
- **The relay handles one message at a time.** An agent that starts the console with `--mcp` waits for each answer: while a
  question waits for you (up to 45 s), that agent's other requests wait too. HTTP agents do not have this limit.
- **A computer shared with other people.** Anyone signed in to this computer can reach the agents' port, as with any
  program that listens on it, but nobody gets in without a token. If the console is not running, another person's program
  could take the port and receive the token of an agent that connects to it. On a shared computer, pick another **Port**
  than 47860, and if you suspect this happened, remove the client in **Clients** (its token stops at once) and pair it
  again.
- **The port is taken.** If another program has the port, the page says so and **Try Again** retries. Pick another port,
  and change it in the agents' configuration too (`--port <n>` for `uconsole mcp`).

## 8. Owner smoke test checklist (wave 6)

For the owner, on real agent hosts, before the release. Use the demo device or a device with its limits profile set and
the volume at 80 % or less. These steps change your own agent configuration; undo them afterwards if you want.

**Linux, Claude Code over HTTP**
1. Settings › AI Agents: turn agents on. **Listening on** shows `127.0.0.1` and `[::1]` with port 47860.
2. **Add Client…** "Claude Code". Add it with the `.mcp.json` of section 2.2 and `DSPI_MCP_TOKEN` set.
3. In Claude Code, `/mcp` lists `dspi` as connected with its tools. Ask "list my DSPi devices" (`dspi_list_devices`).
4. Ask for a quiet change ("lower output 1 by 1 dB"): it happens at once; the activity and the History show it with the
   **AI** tag. **Undo** in the activity puts it back.
5. Ask for a louder change ("raise output 1 by 3 dB"): the question appears; **Deny**: nothing changes, the agent says so.
   Ask again; **Allow**: the change happens.
6. **Pause**: a change is not made, the agent says it is paused; **Resume**. **Remove this client**: the next request gets 401.

**Linux or macOS, Claude Desktop over stdio**
7. Add the config of section 2.1 and start Claude Desktop again. The console asks **Allow this AI agent?** with the code
   Claude Desktop's MCP log shows. **Allow**. The tools appear in Claude Desktop.
8. Quit and start Claude Desktop again: it connects without asking.
9. Close the console: Claude Desktop's next tool call says the console is not running. Start it again: calls work again.

**Windows, Claude Desktop over stdio** (the GUI program as a stdio server; not proven on a real Windows yet)
10. Add `"command": "C:\\Users\\<you>\\AppData\\Local\\Programs\\uConsole for DSPi\\uconsole-for-dspi.exe", "args":
    ["--mcp"]`. Check that no console window flashes, the pairing question appears in the running console, and the tools
    appear in Claude Desktop. Repeat step 4.

**Questions and settings**
11. **Ask me: Never**: a louder change happens without a question. Removing a tweeter output's high-pass is refused.
12. **Ask me: About every change**: a quiet change is asked about too.
13. **Stop** turns agents off; the agents' next call fails; turning them on again works.

Write down the agent host versions and anything that differed.
