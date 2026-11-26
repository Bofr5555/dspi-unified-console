# AI agents

*User manual of uConsole for DSPi 0.16.0: AI agents in the desktop app (both layouts). How the owner checks it
on real agent hosts before a release: [`docs/testing/ai-agents-checklist.md`](../testing/ai-agents-checklist.md).*

An AI agent on your computer, such as Claude Desktop, Claude Code, Cursor or VS Code, can read the settings of the DSPi
devices that uConsole for DSPi has open and change them for you: "make the left woofer 2 dB quieter", "why does output 3 sound
thin?", "set up a crossover at 2.5 kHz". The agent talks to the app over MCP (the Model Context Protocol). You see
every change live in the app, as if you had made it, and you can undo each one.

AI agents are **off** until you turn them on. Nothing on your network can reach them: the app listens on this computer
only. The web console and the Android app run no agents; the web console shows when an agent works on a hub's devices
through another console (section 5).

## Connect Claude in three steps

1. **Turn agents on.** In uConsole for DSPi, open **Settings › AI Agents** and switch on **Allow AI agents to control
   uConsole for DSPi**. Read what it allows, then click **Allow**.
2. **Add it to the agent app.** Under **Connect an Agent** (right under the switch until an agent is paired), choose
   **Claude Desktop** or **Claude Code**.
   - Claude Desktop: click **Install in Claude Desktop…**. Claude Desktop should open and show the extension
     "uConsole for DSPi": click **Install** there. This extension has not been tried in a real Claude Desktop yet and is
     not signed; if Claude Desktop does not take it, add the connection by hand instead (section 2.1).
   - Claude Code: click **Copy**, paste the command into a terminal and run it.
3. **Allow it.** The first time the agent connects, uConsole for DSPi asks **Allow this AI agent?** and shows a
   four-digit pairing code. Compare it with the code the agent app shows: the agent says it in the chat ("check that
   uConsole for DSPi shows the code …") when it asks for a tool while the question waits, and it is also in the agent
   app's log of the dspi server (Claude Desktop: Settings › Developer; Claude Code: start it with `claude --debug`).
   Click **Allow** only if you have just added an agent and the codes match.

Then ask the agent something, such as "What is connected to my DSPi?" or "Make the left woofer 2 dB quieter".

**Two kinds of questions.** Your agent app may ask you before each tool it uses ("Allow dspi_set_gain?"): that is the
agent app's own question. uConsole for DSPi asks its own before risky changes (section 4), and its limits and speaker
rules hold whatever you answer in the agent app. So you can let the agent app allow the dspi tools without asking each
time: in Claude Desktop, choose to allow the dspi tools always; in Claude Code, add the allow rule `mcp__dspi__*` (with
`/permissions`, or in `.claude/settings.json`: `"permissions": {"allow": ["mcp__dspi__*"]}`).

**Where to find it**

| Where | What |
|---|---|
| **Settings › AI Agents** (both layouts) | the switch, the questions setting, the limits profile, how to connect an agent, the clients and their tokens, the activity |
| **Device › Agent Activity…** (Classic), the robot icon beside the device in the status bar (Classic), the **AI** button in the top bar (Modern) | the agents' panel, **Agent Activity**: what they are doing, their questions, Pause and Stop Agents, the activity with Undo |
| **History** | the agents' changes, tagged **AI** with the agent's name |

## 1. Turning agents on

1. Open **Settings › AI Agents** and switch on **Allow AI agents to control …**.
2. The app tells you what that allows, and offers **Set Up Limits…** if the device has no limits profile. A limits
   profile sets the most gain and volume anyone may set, agents included. It is optional, but it is the best protection
   for your speakers. An agent can never change or remove it.
3. The page shows **Listening on** with the addresses (`127.0.0.1` and `[::1]`, port 47860). Change the **Port** only if
   another program uses 47860; then use the same port in the agent's configuration below.

Turning the switch off (or pressing **Stop Agents**) disconnects every agent at once.

## 2. Connecting an agent

There are two ways in. The **Connect an Agent** section of the page shows the exact command or configuration for your
computer, with a **Copy** button. Use that text rather than typing it from here: it has the right program path.

- **Agents that start a program** (Claude Desktop, and most IDE agents) start the app with `--mcp`. That copy of the
  app does not open a window: it relays the agent's messages to the app you have open. The first time, the
  app asks **Allow this AI agent?** with a four-digit code. The agent app shows the same code in its log, and in the
  chat when it asks for a tool while the question waits (the agent then says the code and asks you to allow it).
  Click **Allow** only if you have just added an agent and the codes match. The relay keeps a token for each agent
  program (by the name the program gives itself), so this is asked once per program: a second program on the same
  computer, say Cursor beside Claude Desktop, is asked about and named on its own. (A version from before this kept one
  token for all; each program is then asked once more.)
- **Agents that connect over HTTP** (Claude Code, Cursor, VS Code) use the address `http://127.0.0.1:47860/mcp` with the
  header `Authorization: Bearer <token>`. Make a token with **Add Client…** in the **Clients** section. The token is shown
  once: copy it then.

Each client has its own token and its own name in the History and the activity. **Remove this client** stops its token
at once. What it changed stays, and you can still undo it.

The app must be running, with agents on, while the agent works. Otherwise the agent gets "uConsole for DSPi is not
running, or AI agents are turned off in it."

### 2.1 Claude Desktop

**In one click.** In Settings › AI Agents, Claude Desktop, click **Install in Claude Desktop…**. The app writes a Claude
Desktop extension (`uConsole for DSPi.mcpb`, in the app's own data folder) that starts this copy of the app with
`--mcp`, and opens it: Claude Desktop should show its install sheet, where you click **Install**. If nothing opens it
(Claude Desktop is not installed, or it is a version without extensions), the page says so and **Show File** shows
where the file is: open it with Claude Desktop, or add the connection by hand as below. The extension names the program
where it is now: after you move the app, or after an AppImage update with a new file name, click the button again to
make it again. Use the extension or the hand-written entry, not both (Claude Desktop would start the app twice).

**Not tried yet.** The extension has not been installed in a real Claude Desktop yet, and it is not signed, so Claude
Desktop may warn about it or not take it. If it does not work, use the hand-written entry below: it does the same without the
extension.

**By hand.** Under the button the page says "Or add this to claude_desktop_config.json by hand, then start Claude
Desktop again", with the entry in the **Configuration** field and its **Copy** button. Add it to
`claude_desktop_config.json`, then quit and start Claude Desktop again. If the file already has an `"mcpServers"`
section, put the `"dspi"` entry inside it, beside the others. The page shows where the file is on your computer, and
**Open Config Folder** opens its folder (in Claude Desktop: Settings › Developer › Edit Config):

```json
{
  "mcpServers": {
    "dspi": { "command": "<the app's program>", "args": ["--mcp"] }
  }
}
```

The program is:

| System | Program |
|---|---|
| Windows | `C:\Users\<you>\AppData\Local\Programs\uConsole for DSPi\uconsole-for-dspi.exe` (in JSON, write each `\` as `\\`) |
| macOS | `/Applications/uConsole for DSPi.app/Contents/MacOS/uConsole for DSPi` |
| Linux (AppImage) | the AppImage file itself, such as `/home/<you>/Applications/uConsole-for-DSPi-<version>-x86_64.AppImage` (after an update, the new file's name) |
| Linux (installed) | `uconsole-for-dspi` |

If you have the Terminal (`uconsole`) installed, `"command": "uconsole", "args": ["mcp"]` does the same.

The file is in:

| System | `claude_desktop_config.json` |
|---|---|
| Windows | `%APPDATA%\Claude\` (Claude Desktop from the Microsoft Store: `%LOCALAPPDATA%\Packages\Claude_…\LocalCache\Roaming\Claude\`) |
| macOS | `~/Library/Application Support/Claude/` |
| Linux | `~/.config/Claude/` |

### 2.2 Claude Code

Over stdio, as Claude Desktop does (the app asks you to allow it the first time):

```
claude mcp add dspi -- <the app's program> --mcp
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
    { "type": "promptString", "id": "dspi-token", "description": "uConsole for DSPi client token", "password": true }
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

Any MCP client that can start a program (`<the app's program> --mcp`, or `uconsole mcp`) or speak Streamable HTTP
with a header works. The app speaks both versions of the protocol in use today: 2025-11-25 and older (with
`initialize`) and 2026-07-28 (stateless). Each build checks this against the official MCP conformance suite and the
official TypeScript SDK's client (`ci/check.sh --step mcp-conformance`).

### 2.6 From another computer

The app never listens on your network, and has no setting for it. To use an agent on another computer, carry the
connection over SSH: [Using uConsole for DSPi's AI agents from another computer](../mcp-ssh-forwarding.md) (the page's **Use
from Another Computer…** link).

## 3. What agents may do

An agent you allow may read every device the app has open and change most of what you can change: the equaliser,
crossovers, gain, delay, polarity, mute, routing, effects, volume and source, presets (load, save, rename, copy),
snapshots, A/B, config and filter files (as text), AutoEQ, channel and device names, the audio link's delays and scenes,
Installation setup and a room's or zone's volume. It may ask the app to open a device it knows, and to measure your
speakers or headphones and tune them (section 6).

- **Several devices.** Every tool takes a `device` (from `dspi_list_devices`). Without it, the agent works on the device
  the app shows.
- **Opening devices.** An agent can ask the app to open a device it **knows**: a DSPi on USB, a unit that the hub of an
  open device lists, or a device you have opened before (`dspi_list_known_devices`, `dspi_open_device`). The app opens it
  in a tab of its own, beside what you have open, and does not switch to it. It never pairs and never connects to an
  address or a hub it has no connection to; that stays yours. An agent can close only a tab it opened
  (`dspi_close_device`), and not while the device has unsaved changes, measures, has a trial waiting for Keep, or you
  hold one of its controls. Everything else is yours to close.
- **Where the music comes from.** A DSPi has no player: it plays what its input receives from another program or box
  (a music app, a streaming app, a TV, a game). The agent is told so, and which input is in use (USB, S/PDIF, I2S,
  ADAT or the audio link). To choose or start music it needs that program's own integration, if you have one; these
  tools only reach the DSPi.
- **This computer's sound settings, read only** (`dspi_get_host_audio`). The agent can see, on the computer the
  app runs on, whether each open DSPi is one of its sound outputs, whether it is the default output, its channel
  count and sample rate, which apps play to it (Linux and Windows; macOS from 14.2), and the system volume, with whether
  that volume moves the DSPi's user volume. It reads PipeWire on Linux (falling back to the ALSA sound cards), the
  Windows audio service, and Core Audio on macOS. It never changes the default output, moves an app or sets the system
  volume: it tells you where to do that (the desktop's sound settings or `wpctl` on Linux; Settings › System › Sound on
  Windows; System Settings › Sound on macOS, which has no per-app output without third-party software). If the system
  cannot be read (no PipeWire, a sandbox, no permission), the agent is told "unknown" and why, and carries on.
- **Speaker roles come from you.** An agent reads which outputs are woofers and tweeters from your speaker setup.
  When you tell it which output drives what ("L and R front speakers, three way, tweeter, mid,
  woofer, left then right, outputs 1–6"), it may set those roles **from your words**: it shows you the whole table in one
  question and quotes your words (section 6, *Speaker roles from your words*). It never sets roles you did not state.
  If you have not set up the speakers and did not say, an agent may ask to **detect** them: the
  app plays quiet sweeps on each output and works out which outputs reach a driver and which drive a tweeter
  (section 6, *Detecting the speakers*). Roles set from your words or detected guard your speakers until you confirm or
  correct them in Settings › Speakers.
- **The installer's lock.** While a DSPi is locked by its installer, an agent may change only what the locked user may:
  the volume, the mutes, the input source and a preset load. Everything else is refused with that reason, and so are
  speaker roles from your words: under the lock the speaker layout is the installer's, and nothing an agent states
  reaches the roles the tweeter rules trust. The agent is told where you unlock it (Settings › Installer); it never
  sees or uses the PIN.
- **Installation setup** works on a draft that belongs to the agent. Nothing reaches the hub until the agent saves it. If
  someone else saved the installation meanwhile, the save is refused, and the agent's edits are kept aside to apply again.
- **Sources, zones, slots and scenes** are edits of that draft too
  (`dspi_installation_set_source`, `_set_zone_source`, `_set_slots`, `_set_scene`), checked as the hub checks them. A
  zone's source routes: every unit of the zone on the audio link takes the source's channels as its slots, and a unit
  given other slots by hand or by a second zone is refused (a slot clash), as is a channel two sources of one sender
  share. What is live and what is only stored:
  - a draft edit changes nothing you hear;
  - a saved installation is stored on the hub, but the units play their new slots only once the audio link is aligned
    (`dspi_system_align`); the agent is told which units wait for that;
  - a recalled scene with a source action switches the zone at once: the hub stores the zone's source and writes its
    units' slots straight away (a unit that is offline takes them at the next alignment).
- **Programme sources** (`dspi_installation_set_programme`): the installation can describe which
  computer, box or service feeds which unit input or sender channel (a computer over USB, S/PDIF, analogue, or the audio
  link sender). It is a description for you and the agent; it routes nothing. On a unit of that installation the agent
  reads, beside the input in use, which of them plays into it and on which of its inputs.
- **A room's or zone's volume.** On a hub with an installation, `dspi_system_set_volume` sets the volume of every unit of a
  room or a zone: one level for all, or a step up or down from each unit's own. Each unit's limits still hold: a unit whose
  limits profile refuses, that is offline, that the app has read-only, or whose control you are holding, is left as it
  is, and the agent is told which. It is not in the History, because it goes through the hub: the agent gets each unit's
  level before, and can set it back.
- **Schedules** (`dspi_list_schedules`) an agent can read, not change.
- **Names and notes the agent reads** (channel, preset, snapshot, hub, room names, event text) are passed to it as data,
  cleaned of control characters, and marked as text it must not take as instructions.

**What agents can never do, whatever the setting:** update or flash firmware, use the bootloader or factory reset;
change the limits profile, the installer PIN or lock; change hardware, pin or core-voltage settings; play the signal
generator or Identify (a measurement's test sweeps are the one exception: section 6); pair hubs or change DSPi Link
security and hub administration (its schedules among them); delete presets, snapshots or measurements; update the
AutoEQ database; set speaker roles you did not state, or replace a speaker setup you confirmed unless your words say what
changed; change the computer's sound settings or play music; send raw
device commands; read or write files by path.

**What an agent reads, it may pass on to its own service.** An agent that runs on a cloud model sends what it reads to
that service, as with anything else you show it.

## 4. Questions: what is asked before a change

**Ask me** in **Settings › AI Agents › Ask Before Changes** has three settings:

- **About risky changes** (the default). The agent's change waits for your answer when it would:
  - make something louder: raise a gain, trim or volume (a room's or zone's too, when any unit gets louder), unmute,
    switch on an output, add a route or raise a route's gain, turn on an effect that can add level (loudness, a leveller
    with gain, psychoacoustic bass, subharmonic, the upmixer) or feed one to more outputs;
  - boost a PEQ band by more than 6 dB, change a Linkwitz transform, or move a delay by more than 20 ms;
  - lower a tweeter's protection (see below);
  - change the input source;
  - play test signals: start a measurement, a low-end search, or an auto-tune's check or verify (section 6). This is
    asked **once per device for the agent's task**: **Allow** lets that agent play test signals on that device for
    the next 10 minutes (see **Test signals for 10 minutes** below);
  - detect the speakers: a test signal with a warning, one question (section 6, *Detecting the speakers*);
  - set speaker roles from your words: one question showing the whole table (section 6, *Speaker roles from your
    words*);
  - replace many settings at once: load a preset, recall a snapshot, import a config or filter file;
  - write the device's flash: save, rename or copy a preset, save to the device, rename a channel or the device;
  - change every unit of an installation: align the audio link (also with a measurement across units), apply a scene,
    change USB-direct modes, or save the installation.

  Other changes, such as lowering a gain, cutting a PEQ band, a small delay change or opening a known device, are made at
  once.
- **About every change.** Every change waits for your answer; test signals are asked once per device for 10
  minutes, as above.
- **Never.** Agents' changes are made at once, louder ones too, and **test signals play without a question** when an agent
  starts a measurement. Your limits profile and the protection of outputs you set up as tweeters still apply, and so do
  the rule below and the rules of section 6. A speaker detection is not asked under **Never** either: it is refused
  unless you switch on Settings › AI Agents › **Agents may detect speakers** (off by default; switching it on shows its
  warning once). Nor are speaker roles from your words: they are refused unless you switch on
  Settings › AI Agents › **Agents may set speaker roles from my words** (off by default; switching it on asks once).

**A tweeter's protection.** An output whose high-pass sits at 1 kHz or above drives a tweeter, whatever it is called.
Removing that high-pass, moving it below 1 kHz or making it shallower than 12 dB/octave is asked about under the default
setting and refused under **Never** (you can still make the change yourself).

An output your confirmed speaker setup calls a tweeter (or, until you confirm it, a detected one, or one set from your words; with roles from your words, an output your words did not name too) is protected under every setting, **Never** included: whenever it
can play (an input is routed to it, it is on and not muted), it must have a high-pass at 1 kHz or above (or the higher
corner your limits profile sets for it), 12 dB/octave or steeper. Every change an agent makes is judged on the result: a
route, an unmute or switching the output on, a crossover or EQ change, an input's EQ, a preset load or import, keeping a
tune. The agent is refused, with the reason, when the result would let programme reach the tweeter without that high-pass,
when it sets a high-pass below the floor or too shallow, or when an EQ boost below the floor lets more through than a
12 dB/octave high-pass at the floor would (by more than 3 dB): a boost behind a steep high-pass, where nothing reaches the
tweeter, is fine. A midrange is held the same way at 80 Hz. Several changes in one call are judged together, and played
quietest first: the agent can protect the tweeter and route it in one go. If you have left a tweeter playing without a
high-pass yourself, an agent may still turn it down or mute it, but not feed it more: no higher gain, trim or volume, no
new route, no effect that adds level and no EQ boost on it (or on an input that feeds it), also not through a preset.

**The question.** The heading says the change in one line, in the screen's words, with the device in brackets:
"Claude Code wants to make Right woofer louder (Demo 1)". Below it is the whole call, the outputs and presets named as
the app names them: the rows that are asked about marked ▲ ("Right woofer: gain −5.0 dB → −2.0 dB"), then what the
same **Allow** also does ("Sub: gain 0 → −6 dB"). The same row on several outputs is one row ("Left woofer, Right
woofer: limiter off"). An output keeps the name your speaker layout gives it ("Left tweeter"); a name an agent gave an
output itself is shown with the output's number ("Sub (quieter) · Out 3"), so a name cannot hide which output it is.
The last line says honestly what you can do afterwards: undo it in the agents' activity; for a preset save and other
writes to the device's memory, "This is written to the device's memory; Undo cannot take it back"; for a preset load or
a snapshot, that the settings as they are now are kept as a snapshot first; for test signals, that sound cannot be
taken back once it plays and **Stop Measuring** in the agent strip ends it.
The speaker detection's question lists what you do first (the microphone, the volume, staying by **Stop Measuring**,
checking Settings › Speakers afterwards); how it works is under **Details**. **Allow** and **Deny** answer it. If the
app is in the tray, the question brings its window back. The agent waits up to 45 s for your answer; after that it is
told the question is still open and can ask again with `dspi_get_confirmation`. A question nobody answers lapses after
2 minutes, and nothing is changed. Up to 8 questions of one agent may wait at once; the sheet says how many more are
waiting ("2 more waiting").

**Questions raised together are one sheet.** Questions one agent raises within about 1.5 s (calls sent at once, such as
night mode on three devices) come as one sheet: "Claude Code wants to make 6 changes", grouped by device, each device
once with its rows. **Allow** and **Deny** answer exactly the rows on the sheet; a question raised while the sheet is up
comes as the next sheet, never onto it. A sheet takes another question only while everything fits its 12 rows (each
device's heading, the changes asked about first, then the calls' other changes); what does not fit is the next sheet.
One call with more than 12 changes is a sheet of its own, the changes asked about first. A speaker detection or a
speaker-roles question is always a sheet of its own. **Answer One by One** splits a sheet into its questions; if a
sheet's questions ask about more changes than it shows, its Allow is off and the sheet says to answer them one by one.

Only agents that connect over HTTP (Claude Code with a token, Cursor, VS Code) get grouped sheets. An agent that starts
the app with `--mcp` (Claude Desktop, or Claude Code added with `--mcp`) handles one request at a time, so its questions
come one after another (section 7).

**Test signals for 10 minutes.** A question about test signals says that **Allow** also lets that agent
play test signals on that device for the next 10 minutes, for the task it is doing: its next measurements, low-end
searches, sum checks and verifies on that device are not asked again. Louder changes, preset saves and everything else are still
asked one at a time, and the rules of section 6 (the loudest measurement, a tweeter's low end) still hold. The agents'
panel and Settings › AI Agents show each such permission with the device, the agent and the time left, and **Revoke** ends it at
once (the agent is told, and asked again next time). The permission belongs to the agent's pairing (the token it was
paired with), not to one connection: an agent that reconnects or restarts within the 10 minutes keeps it, and so does
any program paired under the same name. It ends after 10 minutes, on **Revoke**, when that pairing is removed, when the
device is closed, and with **Stop Agents**. It covers that one device: a measurement, verify or sum check across several units
of a hub is still asked each time. Under **Never** nothing is asked, so nothing is granted.

## 5. Watching and undoing

- **The strip under the top bar** (both layouts) says which agent works, what it does now and on which device ("Claude Code
  Changing the gain: Out 3 Centre"), with **Pause** and **Stop Agents**, and **Stop Measuring** while its measurement plays.
- **When the app refuses an agent's change** (a tweeter's or midrange's high-pass, the installer
  lock, a control you took back, your limits), the strip says why for a few seconds: "Refused: Kept Right tweeter's
  high-pass at 1 kHz · Demo 1", and the activity keeps it under **Not done**. Nothing is changed, no question is asked
  and no sound plays. The agent gets the rule in its own words, so what it tells you can be checked here.
  **Show** brings the device to the front when it is another one. The strip is there while a call or a job runs, while a
  question waits for you or a trial waits for Keep, for 20 s after the last call, and as long as agents are paused.
- **What an agent left unsaved.** Agents' changes are live at once but kept only once saved on
  the device. When an agent's work rests, the strip says so: "Claude Code Changed 12 settings, not saved yet"
  ("… on 2 devices, not saved yet" when it changed several), with **Save to Device** (**Save All…** for several
  devices). It stays until the changes are saved, put back or the agents are turned off. Agents are told to say this at
  the end of a task and to offer to save.
- **The AI button** (Modern) and the **robot icon** (Classic) turn while an agent works. In the tray, the icon has a blue dot,
  and its tooltip and menu say what the agent does, with **Pause AI Agents**.
- **What an agent changes** is marked for a moment: the control, its section in the rail, and the tab of a device that is
  not the one shown. A notice names the agent and the change, with **Undo**; several changes in a row are one notice.
- **The activity** (on the page and in the agents' panel) lists what each agent did, newest first: each change with
  **Undo**, and **Undo All**. **Show reads** adds what they only read. **Export…** saves the list.
- **The History** shows agents' changes with an **AI** tag and the agent's name; you undo them there like your own.
- **An agent can undo only its own changes.** It never undoes yours.
- **Pause** holds the agents' tool calls off until **Resume**: nothing is changed meanwhile, and the agents are told that
  you paused them. **Stop Agents** is the one Stop for agents (the strip's **Stop Measuring** ends only a measurement):
  it turns agents off, disconnects them and puts back any trial an agent started. Changes already made stay, and you can undo them.
  When you turn agents on again, each agent that was working is told with its next call that you stopped it.
- **The screen follows the agent.** When an agent changes something, the Modern layout shows that
  device's tab and the page it is changing (Classic shows the device), and an agent's measurement opens **Measure**, so
  you see what it does. Never while you are working yourself: while you drag a control, type in a field or have a dialog
  open, only the strip says what the agent does. It switches at most every few seconds, so the screen does not flicker.
  The web console does the same for an agent of another console on the hub's devices.
- While you drag a control, an agent's change to that device is refused and the agent is told to try again: you are not
  overruled mid-drag.
- **On a hub's device** (DSPi Link), the app also says on the hub what its agent does, so the **web console** and other
  consoles of that hub show it too: a strip on every screen ("Claude, through Studio Mac, is working on this device:
  Changing the gain: Out 1") and an **AI** mark on the device's tab, for as long as the agent works (at most 20 s after its
  last call). What a person changes there reaches the agent as a change made elsewhere.

### 5.1 When you step in

You can change anything while an agent works. The agent is told with its next call: what you changed, on which device and
channel (numbers and switches, never names), what you undid, a preset you loaded, a device you switched to, opened or closed,
a measurement you stopped, a speaker setup you confirmed, Pause, and your answers to its questions. It is also told of
changes made elsewhere: a control on the device, another console.

- **A control you take back is yours.** When you change (or undo) something an agent set within the last 10 minutes, the
  agent's next change to it is refused with "the user changed this": it has to ask you. A change that asks you in the
  app anyway is asked there. A preset you load counts for everything the agent set. Its trial or auto-tune is
  not kept without asking either; putting it back is always possible.
- **A setting you change while an agent's measurement runs stops it.** Its result would not be valid: it is not filed, a
  notice says so, and the agent is told why. Names and saved presets do not count.
- Your own measurements are not affected by this.

**For agents** (and those who write them):

- Every tool result that has news carries one more text ("Since your last call, the user ...") and, in a structured result,
  `user_activity`: `{note, events: [{id, at_ms, device, kind, what, path, output, input, band}], more, cursor}`, at most 8
  events. `dspi_get_user_activity {since}` lists the events after a cursor. Kinds: `change`, `undo`, `replaced` (everything
  at once), `elsewhere`, `speakers`, `measure_stop`, `device_opened`, `device_closed`, `device_shown`, `paused`, `resumed`,
  `answer` (to your own question, and the test-signal permission: given, revoked, ended), `stopped` (the user pressed Stop
  before agents were turned on again).
- Read what you work on again before you change it; never put the user's change back unasked.
- "the user changed this" is an error (`isError`): ask. A retry within 5 s of the refusal is refused too.
- `status: "paused"` is not an error: wait, or ask the user to press Resume. `status: "awaiting_confirmation"` as in section 4.
- `dspi_measure_status`: `request.valid` is false for a measurement the user disturbed; `request.note` says by what.

## 6. Measuring with an agent

An agent can ask the app to measure, find an output's low end, and propose, apply and verify an auto-tune. The app
does the measuring, with its own microphone and in its own **Measure** view, exactly as when you start it there. The agent
never plays or records anything itself.

**One microphone.** The app measures with one microphone, so an agent's measurement waits while any open device
measures (yours too): the agent is told which.

**What it needs**

- **A person at the microphone.** The agent says where to put it; somebody has to put it there, and move it when the
  measurement asks for the next position.
- **A confirmed speaker setup** (Settings › Speakers, **Confirm**), one the speaker detection found, or
  one an agent set from your words. The roles (woofer, midrange, tweeter…) always come from it: no measuring tool takes a
  role. Without one, the agent is refused and told to set the roles from what you said, detect the speakers, or ask you
  to set them up.
- **The device open in the app**, and nothing else measuring on it. A measurement across units needs each unit open in
  a tab, each with its own confirmed speaker setup.

**What happens.** The agent's start is a request. Under **About risky changes** and **About every change** you are asked
first (the question says **Test signal**, which outputs play, and the **AI** button shows there is something to answer),
once per device for 10 minutes (section 4); under **Never** it starts at once. The app then opens **Measure**, opens the microphone and runs it. The Stop bar says
**Requested by AI · name** until it ends, with **Stop** on every tab of Measure; on a device that is not the one shown, its
row there has **Show** and **Stop**. A request the app cannot take up within 30 s lapses, and the agent is told. The
result is filed with your measurements. An agent can stop only what it asked for itself.

**How loud.** Settings › AI Agents › **Loudest measurement** (85 dB SPL by default, 60 to 95) caps what an agent's
measurement may play, under every question setting, **Never** too: an agent may only ask for less. It needs a
microphone calibrated in dB SPL: the app's own calibration is used when the agent names none. Without one, an agent's
measurement is refused, and the agent is told to ask you to calibrate the microphone in **Measure** or to turn the cap
off. With **Limit agents' measurements in dB SPL** off, only the level ceiling (-12 dBFS at a speaker, -30 dBFS on
headphones) holds.

**Opening devices.** An agent may ask to open a device the app knows: one on USB, one listed by an open hub, or one
you opened before on USB. The app remembers the USB devices you open across restarts (Settings › AI Agents › **Devices agents
may open**, with **Forget Devices**). A device an agent opens or closes is a notice.

**Detecting the speakers.** When nobody has set up the speakers, an agent can ask the app to find
out what is wired to each output (`dspi_detect_speakers`). The app plays quiet test sweeps on each output in turn,
from the microphone at the listening position (or close to each driver, moved when you are asked). Each output is first
played only above 1 kHz (or a higher floor: your limits profile's for that output, or one the agent names), at most −24
dBFS (−36 dBFS on headphones). Only an output whose response says it is not a tweeter (or that is not heard above its
floor) is played lower, and then at most −50 dBFS (−60 dBFS on headphones), no lower than 157 Hz. One that is still flat
at 177 Hz there, as no tweeter is, is then played down to 20 Hz (never below your limits profile's high-pass minimum for
that output) at the level its top end was heard at, never above −24 dBFS (−36 dBFS on headphones), or at −50 dBFS (−60
dBFS) if its top end was not heard. A higher floor never lets that last step start sooner: the quiet sweep still has to
find the driver flat at 177 Hz first. Whatever cannot be told apart is treated as a tweeter. Outputs that are off or
muted are not probed. The question says so, with a warning: nothing is known about these outputs yet, and a tweeter
wired where the probe expects a woofer could receive a few seconds of sweep below its floor at −50 dBFS (more with a
raised floor): below 1 kHz less than a hundredth of what a 1 kHz crossover lets through from music, but not nothing.
Only a driver still playing flat at 177 Hz under that quiet sweep, as no tweeter does, is then swept down to 20 Hz at up
to −24 dBFS. Keep the volume where it is and stay by **Stop Measuring**. **Allow** also
lets that agent play test signals and detect again on that device for 10 minutes (section 4); **Revoke** stops a running
detection at once. A higher floor in the limits profile belongs on tweeter outputs only: an output's
high-pass minimum also holds its music's high-pass there, so on a woofer's output it would take away the bass (on the
Koss K340, 2.5 kHz on the two tweeter outputs, not on the woofers'). If you do not know yet which output is which, tell
the agent the floor instead ("nothing below 2.5 kHz may reach the tweeters"): it passes it with the detection
(`floor_hz`), for every output and for that detection only, and no filter changes. Under **Never** it needs Settings ›
AI Agents › **Agents may detect speakers**. The result is in
Settings › Speakers: each output's role, marked as detected, and a summary such as "2 tweeters, 2 woofers, 1 midrange?;
nothing heard on Out 4". Until you confirm or correct it, the detected roles guard your speakers as a confirmed setup
would, and an agent may measure and tune with them. An output where nothing was heard is left out: an agent may not route
to it, switch it on or unmute it (you can). An agent may not detect over a setup you have confirmed: correct it in
Settings › Speakers.

**Speaker roles from your words.** When you tell an agent which output
drives what, it may set the roles from what you said (`dspi_set_speaker_roles`): one row per output you named, with the
side or speaker you gave it, and your words quoted. You are asked once, with the whole table ("Set the speaker roles on
main front?": Out 1 · L · Tweeter, Out 2 · L · Midrange, …), the outputs you did not name, your words, and that the roles
are not checked. Under **Never** it needs Settings › AI Agents › **Agents may set speaker roles from my words**. The
roles then guard your speakers at once, as a confirmed setup's do: a tweeter keeps its high-pass at 1 kHz or above, a
midrange at 80 Hz. An output your words did not name is unknown and treated as a tweeter. Settings › Speakers and the
main screen say **Set by AI from your words, not checked**, each such row is marked **from your words**, and you confirm
or correct them there. Over a setup you have confirmed, an agent sets roles only when your words say what changed ("I
rewired output 5"); you are still asked. Under the installer's lock an agent cannot set roles at all, not even after
an **Allow**: the speaker layout is the installer's until it is unlocked.

**Trust but check.** The roles are your words, not a measurement, and a wire can be in the wrong place. Before an
agent's test signal plays below 1 kHz on an output your words made a midrange, woofer, sub or full range, the app
checks that output's top end first ( the check is for the app's own test signals; routing music,
unmuting and filters follow the roles as you stated them): a quiet sweep above 1 kHz only, at most −24 dBFS (an output silent there is probed lower at most
−50 dBFS, as the detection does). If the top end falls as a woofer's or a midrange's does, its stated role holds from then on. If
it holds up to 16 kHz as a tweeter's does, nothing below 1 kHz plays on that output, the safety rules keep it a tweeter, and the agent tells you ("Out 2 looks like a tweeter");
the row shows **tweeter-like**. A check stands for the role it checked; stating the same role again does not run it
again. The check judges the driver, not the output's own filters: an output whose low-pass already hides its top end is
not checked and stays held at 1 kHz, and the agent is told to measure it before setting its low-pass, or to ask you to
confirm the role. Your own measurements, and confirmed setups, are not checked.

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

**Arrivals and aligning speakers.** A measurement gives each output's arrival time at the microphone. Arrivals compare
only within one run: normally one speaker's outputs, as each speaker is measured on its own. Asked with `one_run`, the
app measures every listed output in one run, and then proposes the delays that bring the device's speakers (left and
right, for example) to the same arrival, each speaker by its lowest way; the agent applies them as its other changes. A
subwoofer is left out: it is aligned at its crossover. Across devices, arrivals need the audio link's time base (a
measurement across units).

**What a response includes.** A measured response is the driver through the output's filters as they were: a tweeter's
measured low end is its high-pass's. The agent is told so (the high-pass is named); the app does not divide it out.

**Summed runs** play the chosen outputs together, each with the polarity its routes give the programme, as the music
plays them. The result is one response, filed under the first output.

**Tuning.** Applying a proposal starts a **trial** (the agent's own texts call it a timed preview): each change is judged
as the agent's other changes are (section 4), and the trial puts everything back at its end unless it is kept. An agent may keep it only **after a verify that passed**;
without one (none run yet, or one that did not pass) **only you can keep it**, in the app. The same holds across units.

**What an agent may choose.** `dspi_get_measure_options` lists it. The **microphone calibration** is one of those you have
added in Measure, and the **target curve** is a built-in one or one of yours: the agent names it by its id, never a file.
Without a choice the measurement is uncalibrated (relative levels), with the usual target. The **level ceiling** and the
**SPL limit** an agent may only lower below the app's own (−12 dBFS and 95 dB SPL for speakers, −30 dBFS and 90 dB SPL
for headphones), under every setting. The SPL limit works only with a calibration that reads dB SPL. With the microphone on
a headphone **coupler**, the headphones' lower limits apply, in the low-end search too.

**The tools**

| Tool | What it does |
|---|---|
| `dspi_get_measure_options` | Your calibrations and target curves, the couplers, and the level limits |
| `dspi_measure_preflight` | What stands between a measurement and the device now, and the roles it would use. Plays nothing |
| `dspi_detect_speakers` | Asks to detect what is wired to each output, when the speakers are not set up. *Test signal* with a warning |
| `dspi_set_speaker_roles` | Sets the speaker roles from what you told the agent, your words quoted. One question with the whole table |
| `dspi_measure_start` | Asks for a measurement: one output at a time or summed. *Test signal* |
| `dspi_measure_rolloff_start` | Asks for the low-end search on one output. *Test signal* |
| `dspi_measure_status` | Where the request and the device's measuring jobs stand; with `wait_s` (up to 25 s) it answers when something changes, so the agent asks rarely. A finished measurement's arrivals, and with `one_run` the delays that align the speakers |
| `dspi_measure_continue` | The microphone is at the next position: go on |
| `dspi_measure_stop` | Stops what this agent asked for. Never asked |
| `dspi_tune_propose` | Works out an auto-tune from a measurement. Writes nothing |
| `dspi_tune_sum_check` | Plays the ways of a speaker together and compares, before timing changes. *Test signal* |
| `dspi_tune_apply` | Applies the proposal as a trial, as one undo step |
| `dspi_tune_verify` | Measures the applied tune again. *Test signal* |
| `dspi_tune_keep` | Keeps it, only after a verify that passed (otherwise only you can) |
| `dspi_tune_revert` | Puts a tune this agent applied back now. Never asked |
| `dspi_system_measure_preflight` | What stands between a measurement of several units and now. Plays nothing |
| `dspi_system_measure_start` | Asks for a measurement of several units together, through the hub. *Test signal* |
| `dspi_system_measure_proposal` | The alignment it proposes |
| `dspi_system_measure_apply` | Applies it on the hub: every unit re-anchors, and the hub puts it back unless it is kept |
| `dspi_system_measure_verify` | The summed runs across units, before or after the apply. *Test signal* |
| `dspi_system_measure_keep` | Keeps it, only after a verify that passed (otherwise only you can) |
| `dspi_system_measure_revert` | Puts it back now. Never asked |

## 7. Good to know

- **Loading a preset.** A preset load is done by the device itself, so the app cannot look at the preset first. When
  an agent loads one on a device with a confirmed tweeter (or midrange), the app turns the master volume all the way
  down first, loads, checks the result, puts the device back if the preset would leave the tweeter unprotected, and only
  then brings the volume back. That works when the master volume is independent of presets (the default). If your
  device saves the master volume with each preset (its master volume mode), the preset sets the volume itself and the
  app could not keep the device quiet while it checks: **an agent may not load a preset (or revert to the saved
  one) on such a device**, under every setting. Load it yourself in the app. Your own load is guarded the same way where the master
  volume is independent of presets: silent while it loads, and a tweeter or midrange the preset leaves without its high-pass
  is muted, with a notice; give it its high-pass, then unmute it. A refused agent load leaves the device on the preset
  it was on. A hub scene an agent recalls that loads presets is judged the same way, on each unit this app has open.
- **A full-range driver or a small dome midrange is detected as a tweeter.** The detection does not
  look below 1 kHz on a driver that plays flat to 20 kHz. Correct it in Settings › Speakers.
- **Roles from your words are not a measurement.** The top-fall check catches a tweeter wired where
  you said midrange, woofer or sub before anything low plays on it; it does not catch a midrange or woofer where you said
  tweeter (that one is only over-protected: it is measured from 1 kHz up). Confirm the roles in Settings › Speakers once
  the measurements look right.
- **Numbers count from 1, as the app shows them.** An agent reads and gives every number as
  the app counts it: output 1 is Out 1, input 1 is In 1, preset 1 is the app's preset 1, PEQ bands 1 to 10 and
  crossover slot 1 are the app's, and "change 1" in a refused batch is its first change. So "output 3" and "Out 3"
  are the same output, and "saved to preset 1" is the preset 1 of the app's list. Codes are not counts and stay as they
  are: an input source (0 is USB), the audio link's channels and slots (0 to 63), ids and masks.
- **What Undo can undo.** The activity offers **Undo** only for changes Undo can reverse: a preset save, a flash write or a
  preset load have none. An agent's preset load is one notice: "Claude Code: Loaded preset 2 “Lunch”: your limits kept
  Left tweeter and Right tweeter at 1 kHz" when the limits profile held part of it.
- **Forget Agent** in the AI popover (Modern) stops that agent's token: it has to be allowed again, as after
  **Remove this client** in Settings › AI Agents › **Clients**. Nothing else forgets an agent: **Pause** and
  **Stop Agents** keep its token.
- **The guide.** Agents are told to read the server's guide first: the `dspi://guide` resource,
  or the `dspi_get_guide` tool in agent apps that cannot read resources, so every agent works from the same guide.
- **The relay handles one message at a time.** An agent that starts the app with `--mcp` (or `uconsole mcp`) waits for
  each answer: while a question waits for you (up to 45 s), that agent's other requests wait too, and its questions never
  come together as one sheet. HTTP agents do not have this limit.
- **A computer shared with other people.** Anyone signed in to this computer can reach the agents' port, as with any
  program that listens on it, but nobody gets in without a token. If the app is not running, another person's program
  could take the port and receive the token of an agent that connects to it. On a shared computer, pick another **Port**
  than 47860, and if you suspect this happened, remove the client in **Clients** (its token stops at once) and pair it
  again.
- **The port is taken.** If another program has the port, the page says so and **Try Again** retries. Pick another port,
  and change it in the agents' configuration too (`--port <n>` for `uconsole mcp`).
