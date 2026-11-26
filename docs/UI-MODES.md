# UI modes: Simple, Advanced, Installer

*Design, WP UIM1, 2026-11-12. Owner decisions of 2026-11-12 and 2026-11-13 (OD-P1c to OD-P1e). **Built in the core
(WP P1LK, 2026-11-14):** the mode manifest (section 6), the installer PIN, the locked-user state and its write check
(section 3), the hub-pinned mode's place in the resolution (section 4) and the defaults (section 5); calls and rules in
[`integrator/installer-lock.md`](integrator/installer-lock.md). Also built: the Android Home order (section 8) and the
web's slice order (`core-merge/PLAN-CLIENTS.md` section 5.5). The clients' mode switches, PIN dialogs and locked-user
views are their P1 WPs.*

Every client is one app per platform. The app has three **modes**, which decide what the user sees, and a **locked-user
state**, which an installer can put a device in. A mode never changes the device and never grants a permission: what may be
changed is decided by the installer's limits and, on a hub, by the user's role (section 4).

The names in the UI are **Simple**, **Advanced** and **Installer**. The new screens of the Advanced mode (System view, scene bar,
status rail and the rest of section 2) are called the *Pro layer* in plans and code; "Pro" is not a word in the UI.

*Home, the remote (WP UB2, UXP B2 and D4, owner 2026-11-25):* Simple's first screen and the locked user's screen is
**Home**, a remote on every client width: the device and its connection, one big User volume (the Master is the
Installer's) with Mute, the source as segments, the presets as a Recall list (no Save, no Copy Here), the rooms' and
zones' volumes on an installation (web), and one health line; nothing else. Simple's dock has no Master volume and no
Bypass EQ. In the locked-user state nothing that the lock refuses is offered: Save, undo and redo leave the top bar,
and on the desktop every page that changes the device renders read-only (its controls off, its scroll on, one notice
with Go to Home; Devices and Settings stay as they are, the Installer page holds the PIN). The web keeps the locked
user to Home alone (`lockedUser.screens`). Built: web (`screens/home/`, `ui/screens.tsx`), Qt Modern
(`HomeSection.qml`, SectionTable's `overview/home`, PageHost's `lockRead`); Android's Home uses the same words.

*0.16.0 (FIX016, the owner's answers to the REVG review):*
- **The locked user may change the source**, from Home: one core rule (`LOCKED_USER_WRITES` has the input source
  beside volume, mute and preset), so every client offers it as a control, not words.
- **The speaker layout is the installer's under the lock**: the core refuses `speaker_layout_set` while locked and
  not unlocked (the roles are what the tweeter guards trust), and Settings › Speakers, Android's Getting Started and
  the Terminal's Speakers are read-only there.
- **Qt Modern** offers no write the lock refuses: shortcuts, palette rows, Open in Window, panel windows and A|B are
  off under the lock but for volume, mute, preset and source.
- **D4's read-only locked state is Modern, the web and Android.** Classic stays as it was: under the lock it is
  live and the core refuses what the lock covers after the click (owner, 2026-11-26). The Terminal follows the core
  the same way. Qt's Simple mode is Home first and a trimmed dock; its other sections stay.

## 1. The rule that comes first: nothing is locked unless someone locks it

- With **no installer PIN set** on a device, nothing is locked: every mode is reachable from the mode switch, with no PIN
  prompt, and every operation is available as it is today. This is the default for every device and every console.
- The PIN and the locked-user state exist only once an installer has set a PIN (section 3). A user who never sets one never
  sees a lock, a PIN field or a reduced screen they did not choose.
- A console never invents a lock: no default PIN, no lock after an update, no lock because a device is new.

## 2. The modes

| | Simple | Advanced | Installer | Locked user (a state) |
|---|---|---|---|---|
| **For** | Listening on one device | Setting up and tuning; the default everywhere (phones too, owner D9 2026-11-25, until Simple means more there) | Installing and handing over a system | A system an installer has set up for someone else |
| **First screen** | Home, a remote: device and connection, the User volume and mute, source, presets, one health line (UB2: no bypass; meters in the dock) | The classic main window today; the System view once the Pro layer exists (desktop, web). Phones: the front page | As Advanced | The front page, with a lock badge, one line saying what is locked, and the installer's contact |
| **Navigation** | Home, EQ, Tools, Settings: today's screens, under the front page | Today's screens, plus the Pro layer: System view, Channels and EQ, Routing, Dynamics and effects, Hardware, History; a scene bar; a status rail | Advanced plus Limits and power-on; firmware, factory reset, core voltage | Home only; Settings shows About and the installer's note |
| **Changes** | Everything today's screens offer | Everything the limits allow | Everything, the limits included | Volume, mute, source and preset, within the installer's ranges (source: owner D4) |
| **Monitoring** | Output meters and the health line | Meters with hold, limiter gain reduction, CPU, temperature, alarms, history (as the core gains them), the event log and the diagnostics bundle | The same | Meters and the health line |
| **Operations that write flash or can harm hardware** | Behind their usual confirmations, in their usual places | With confirmation | Plus firmware, factory reset, core voltage, limits | None |

Notes:
- **Diagnostics are Advanced and up** (coordinator, 2026-11-13, after the web's Events screen): the diagnostics bundle
  (Save Diagnostics…) is offered in Advanced and Installer, as the desktop's Settings > Advanced has it; Simple does not show it.
- **Simple hides nothing permanently.** It changes the first screen and the order of things; today's screens stay one tap away.
  It is not a restricted mode.
- **Advanced without the Pro layer is today's app.** Until the Pro screens exist, a client that offers modes ships Advanced as
  today's screens, and Simple as a front page in front of them.
- **Installer** shows the pages that change how the system behaves for others: limits, power-on state, firmware, factory reset,
  core voltage. With no PIN set it is reachable like any other mode (section 1). With a PIN set it opens after the PIN.
- **Phones** open in Advanced for now (owner D9, 2026-11-25: until Simple means more there than fewer snapshot buttons). Their Advanced is a subset of the Pro layer: the System view read-mostly, scenes, alarms and
  history; strip editing stays on the desktop and the web.

## 3. The locked-user state and the installer PIN

- An installer sets a PIN for a device in the Installer mode. Setting it is what makes a lock possible; the installer then
  chooses what the lock covers (the limits profile's `lockedPages`, by the manifest's screen ids). *Built (P1LK):*
  `installer_pin_set` (which also shows a recovery code once), `installer_lock_cover`.
- A console enters the locked-user state by itself when the device it opens carries a lock and the console is not unlocked
  for that device, or when the hub says so (section 4). It is not chosen from the mode switch.
- Leaving it is the Installer unlock: the PIN. Unlocking lasts for the session, or until the user locks again; the status
  rail (or the phone's top bar) says "Installer" while it lasts.
- **The PIN is a UI lock, not a security boundary.** Whoever has the device on USB can change it with any tool. The unlock
  dialog says so in one line: the lock is on this console, the DSPi itself is open (OD-P1d; notice
  `installer_lock.scope`). Where real access control is needed, it is the hub's roles (section 4).
- *Built (P1LK):* in the locked-user state the core refuses every write but volume, mute, preset and (0.16.0) the
  input source (within the limits) before anything is sent (`installer_lock.locked`). Repeated wrong PINs open a refusal window kept as the next allowed
  time, never a sleep. A forgotten PIN (OD-P1c): on a hub an Admin clears it; off a hub the recovery code, then
  deleting the device's profile folder as the documented last resort; no "Remove lock" button.
- Removing the PIN (in the Installer mode) removes the lock; the device is back to section 1.

## 4. Fleet: accounts and roles on the hub, the web console as the UI

The future fleet handling runs on the hub: accounts, roles and the device inventory live there, and the web console is its UI
(the `FleetBackend` seam in `apps/web/src/fleet/types.ts`; the hub's roles in `crates/dspi-hub/src/session.rs`). Modes, the
PIN and roles fit into it as follows.

- **Two questions, two owners.** *What may this user change?* is the hub's (roles, enforced by the hub for every client).
  *What does this user see?* is the mode. A mode can never widen what a role allows.
- **The mode follows the role where the hub decides; otherwise it is the user's choice.** A hub (or a fleet server behind
  `FleetBackend`) may pin a mode for a role or an account. Where it does, the console shows that mode and no switch; where it
  does not, the user's own choice applies (section 5).
- **Today's hub roles, mapped:**

  | Hub role | Allowed (enforced by the hub) | Mode |
  |---|---|---|
  | Admin | Everything | The user's choice; Installer reachable (with the PIN where one is set) |
  | Control | Commands classed `control` | The user's choice of Simple or Advanced; Installer pages shown read-only |
  | Viewer | Reads, polls, notifications | The locked-user state's view without its controls, or a read-only Advanced, as the hub decides |

- **The PIN in a fleet.** For a device managed by a hub, the PIN is stored with the device's limits profile on the hub and
  locally, hashed as the hub stores its tokens; an account with the Admin role on the hub can unlock without the PIN. Whether
  the hub may also clear a device's PIN, and how a forgotten PIN is recovered off a hub, is open (section 9).
- **The web console's `FleetBackend`** grows one optional method for this: the mode a backend pins for a device, if any
  (missing: the user's choice). `access()` keeps deciding control or view.

## 5. Per-device memory, defaults and switching

- **Remembered per device, with an app default.** The console keeps the last mode for each device (by serial). A device it has
  never seen opens in the app default.
- **Defaults:** every client opens in **Advanced** (phones too since owner D9, 2026-11-25; they opened in Simple before). A device
  with a lock, or a hub role that pins a mode, overrides the default (sections 3 and 4).
- **Switching:** a Mode control in each client's usual place (the desktop menu bar, the phone's top-bar menu, the web header)
  opens a small dialog: Simple, Advanced, Installer (with the PIN field only when a PIN is set), and a choice between
  "Remember for this device" (the default) and "Use for every device on this console".
- Switching never writes to the device and needs no confirmation. Unsaved edits stay unsaved across a switch.

## 6. The screen list per mode as data in the core

The list of screens per mode, their order, their titles and which ones need the Installer mode live in the core as data, as
CG10's onboarding text does (`assets/onboarding/`). A client renders the list it is given; it does not decide it. Adding a
screen later is a data change plus one view per client. Wording comes from the same file, so the clients use the same words.

*Built (P1LK):* `assets/modes/modes.json`, the typed `ModeManifest` (`mode_manifest()` on UniFFI, `dspi_mode_manifest`
in C, `modeManifest` on the web), and `mode_resolve(platform, chosen)`. The built shape uses camelCase keys, as every
binding record does, adds `platforms`, the locked-user state's `label` and `writes`, `first_in` for `locked_user`, and
the `diagnostics` and `installer_pin` operations:

```json
{
  "version": 1,
  "platforms": ["phone", "desktop", "web"],
  "modes": [
    { "id": "simple", "label": "Simple", "defaultOn": [], "needsPinWhenSet": false },
    { "id": "advanced", "label": "Advanced", "defaultOn": ["phone", "desktop", "web"], "needsPinWhenSet": false },
    { "id": "installer", "label": "Installer", "defaultOn": [], "needsPinWhenSet": true }
  ],
  "lockedUser": { "label": "Locked", "screens": ["front"], "settings": ["about", "installer_note"],
                  "writes": ["vol.master", "vol.user", "vol.mute", "out.mute", "preset.load", "in.source"] },
  "screens": [ { "id": "front", "title": "Home", "modes": ["simple", "locked_user"], "firstIn": ["simple", "locked_user"] }, "..." ],
  "operations": [ { "id": "firmware", "title": "Firmware Update", "modes": ["installer"], "platforms": ["desktop"], "confirm": true }, "..." ]
}
```

The proposal it was built from:

```json
{
  "version": 1,
  "modes": [
    { "id": "simple",    "label": "Simple",    "default_on": [] },
    { "id": "advanced",  "label": "Advanced",  "default_on": ["phone", "desktop", "web"] },
    { "id": "installer", "label": "Installer", "needs_pin_when_set": true }
  ],
  "locked_user": { "screens": ["front"], "settings": ["about", "installer_note"] },
  "screens": [
    { "id": "front",    "title": "Home",                  "modes": ["simple", "locked_user"], "first_in": ["simple"] },
    { "id": "system",   "title": "System",                "modes": ["advanced", "installer"], "first_in": ["advanced", "installer"], "layer": "pro" },
    { "id": "channels", "title": "Channels and EQ",       "modes": ["simple", "advanced", "installer"] },
    { "id": "routing",  "title": "Routing",               "modes": ["advanced", "installer"] },
    { "id": "dynamics", "title": "Dynamics and effects",  "modes": ["simple", "advanced", "installer"] },
    { "id": "hardware", "title": "Hardware",              "modes": ["advanced", "installer"] },
    { "id": "history",  "title": "History",               "modes": ["advanced", "installer"], "layer": "pro" },
    { "id": "limits",   "title": "Limits and power-on",   "modes": ["installer"], "layer": "pro" }
  ],
  "operations": [
    { "id": "firmware",      "modes": ["installer"], "platforms": ["desktop"] },
    { "id": "factory_reset", "modes": ["simple", "advanced", "installer"], "confirm": true },
    { "id": "core_voltage",  "modes": ["installer"], "confirm": true }
  ]
}
```

- `layer: "pro"` marks a screen a client shows only once it has built it; a client without it falls back to the next screen
  in the list (so the manifest can ship before the Pro screens).
- `platforms` narrows an entry to a class of client (`phone`, `desktop`, `web`).
- The rule of section 1 is the core's, not the data's: with no PIN set, `needs_pin_when_set` asks for nothing.
- The manifest says what is shown. What is allowed stays with the core's limits and the hub's roles.

## 7. Channel colours

- A channel's colour is a **small tag**: the descriptor pill (`IN1`, `OUT3`), a dot beside a name, a curve on the graph.
- It is **never a dominant fill**: not meter bars, not card borders or backgrounds, not table text.
- Saturated colour is kept for meaning: level (meters in one neutral colour, red for clipping), alarm states, selection.
- Channels are told apart by position, name and their tag.

Android's Home follows this from UIM1 (meters in one colour, neutral card strokes and band codes, the pills and dots as tags).
The other clients follow when their Pro work or polish pass reaches the screen.

## 8. Client order

| Client | First | Then |
|---|---|---|
| **Qt** (Linux, Windows) | The classic port as planned on `qtport`: today's screens, i.e. Advanced without the Pro layer | The Pro track after the swap to the new tree: the mode switch from the manifest, then the Pro screens |
| **Android** | **Simple first.** Home reordered for the phone (UIM1): volume, preset and source, output meters with a health line, then the graph and the lists | The mode switch from the manifest; later the phone's subset of the Pro layer |
| **Web** | **The Pro layer first**, since fleet handling is what the web is for: the WB6 slices put the System view (WB6d), the scene bar (WB6e) and the status rail (WB6f) before the remaining tool screens | The locked-user view as its simple page; the fleet screens with the hub's accounts and roles |
| **Swift** (macOS, iOS) | Nothing before OD3 | After OD3: the same manifest and the same order as Qt and Android (Advanced by default on macOS and, since owner D9, on iOS too) |
| **Terminal** | Nothing new: the TUI is an Advanced view already | A one-line health summary for scripts |

## 9. Open questions

Questions 1 and 2 were decided on 2026-11-13: 1 by OD-P1c (above, section 3) and 2 by the roles Admin, Control and Viewer
only, with a venue phone as Control within the hub's limits or as Viewer (OD-P1i). The text below is kept as asked.

1. **The PIN's home and recovery.** Proposed: stored with the device's limits profile on the hub and locally, hashed; unlock
   lasts the session. Open: is a recovery path needed (a hub admin clearing a device's PIN), and how is a forgotten PIN
   recovered for a device that is not on a hub?
2. **A fourth hub role.** Today's roles are Viewer, Control and Admin. Is a role between Viewer and Control wanted, for example
   for a phone in a venue that may change volume and preset within the installer's ranges but nothing else, or is Viewer
   enough there?
3. **The desktop window model.** Proposed: the Pro screens as panes in the main window, with "open in window" kept for the
   analyser, the generator and the monitor, and the other classic tool windows as they are. Open: should the Pro track also
   fold the effect windows into the Dynamics and effects screen?
