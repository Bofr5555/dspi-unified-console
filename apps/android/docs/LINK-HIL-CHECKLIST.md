# DSPi Link client: hardware-in-the-loop checklist

For the owner / coordinator (agents never run these). Branch `feature/link`.
Build: `tools/dev.sh ./gradlew :app:assembleDebug -Pdspi.rustProfile=release`,
install with adb as usual. Logs: `adb logcat -s DspiApplication DspiLink dspi-core`.

Record for each item: pass / fail, phone, hub, firmware, and the numbers asked for.

## 0. Hubs

| Hub | How |
|---|---|
| **Fake hub** (no hardware) | `tools/link-fakehub.sh [--auth none\|window:123456\|fixed:482913] [--bridge] [--role viewer]` on this host; port 11915 published. Add it by address (`<host LAN IP>`), or, for discovery, advertise it from the host: `avahi-publish -s "Fake Hub" _dspi._tcp 11915 v=1 hid=6f1a2b3c-4d5e-4f60-8a9b-0c1d2e3f4a5b kind=console auth=pin n=1`. Emulator: `10.0.2.2`. |
| **macOS Console hub** | DSPi Console 1.1.6 (`release/v1.1.6`) in the macOS VM, DSPi passed through (QMP id `dspi`), Settings › Networking › "Share Devices on This Network" on. The VM must be bridged onto the LAN for DNS-SD; with NAT, forward 11915 and add by address. Access "Require Pairing" (Console default) and "Open". |
| **ESP32 bridge** | DSPi-Link `bridge/esp32` on the DSPi UART (1 Mbaud, `notify_enable` on), PIN mode fixed. |

## 1. Opt-in and privacy (any hub)

- [ ] Fresh install: Settings › Networking shows "Control DSPi devices over the network" **off**; the strip under the top bar has no "Network…" button; nothing on the network (`adb shell dumpsys connectivity` / router: no traffic from the app; no `_dspi._tcp` queries).
- [ ] Play/Settings › App info lists the network permissions; no other new permission.
- [ ] Turn it on: the trusted-network notice is shown; "Find hubs automatically" browses only while the page is open, or while a session looks for its hub (section 6); leave the page with no session: queries stop.
- [ ] `adb logcat` never shows a PIN or a token.
- [ ] Client name: blank → the hub's client list shows the phone model (`Build.MODEL`); a custom name shows instead.

## 2. Fake hub (no hardware)

- [ ] `--auth none`: tap the hub → strip "Bench via Fake Hub"; the fixture state shows (master volume −20 dB); volume slider works; quit and reopen the page: hub listed once.
- [ ] `--auth window:123456`: prompt "Pair with Fake Hub" / "On Fake Hub, open Settings > Networking …"; wrong PIN → "Pairing failed: the hub did not accept that code."; right PIN → connected; app restart → no prompt (token reused); Settings › Networking › Paired Hubs › Forget → prompt again.
- [ ] `--auth fixed:482913`: prompt text "Enter the hub's PIN."
- [ ] `--role viewer`: strip "… · view only"; every control disabled.
- [ ] `--bridge`: works; RTA (Analysis › Spectrum) shows bands (per-channel fallback).
- [ ] Stop the fake hub: strip "Lost Fake Hub: … Reconnecting…", "Retry now" works once it runs again; device comes back without a prompt.
- [ ] Wait 35 s at the PIN prompt, then enter the PIN: it pairs (sent over a new connection).

## 3. macOS Console hub (VM)

- [ ] Discovery: "Other Hubs" lists the Mac with "Private" (pairing) or "Open"; address and device count right.
- [ ] Pair via Mac "Pair a New Client": the code works; the Mac lists the phone under Paired Clients with its name.
- [ ] **Echo/attribution:** change EQ gain / master volume on the phone → the Mac follows; change them on the Mac → the phone follows within ~0.2 s (patch 0004 origin rule). Drag a slider on the phone: no jumping back.
- [ ] Preset load on the Mac → the phone re-reads (preset name, all curves).
- [ ] Save preset on the phone while the Mac is idle → saved; on the Mac start "Import Device Configuration" while the phone saves → the phone shows "Another console is applying changes" and sends nothing.
- [ ] Mac: change the phone's role to Viewer → next write refused (DENIED) / after reconnect "view only"; revoke the phone → the pairing prompt, with "Access to Mac was revoked. Pair again to reconnect.", no reconnect loop.
- [ ] Mac sleeps, wakes: "Connection lost, reconnecting" with Retry now and Disconnect, the settings stay readable, controls are disabled; the core reconnects by itself (backoff ≤ 30 s, or at once on "Retry now" / app to foreground); Disconnect ends it at once.
- [ ] Phone Wi-Fi off/on: reconnect when Wi-Fi is back (LinkNetworkMonitor).
- [ ] Background: lock the phone 10 min; the notification's volume/mute/next preset still reach the board.
- [ ] Firmware page in REMOTE: the notice, no installer.
- [ ] Plug a DSPi into the phone while REMOTE → "A DSPi is connected over USB" prompt; "Stay on the Network" keeps REMOTE; "Use USB" switches.
- [ ] Two phones (or phone + Mac UI) at once: both see each other's changes.

## 4. ESP32 bridge

- [ ] Fixed PIN prompt "Enter the hub's PIN."; granted role admin.
- [ ] First screen time (tap → controls): ______ ms. Bulk re-read (preset load): ______ ms.
- [ ] Spectrum analyser: bands appear (bands-all is refused by the bridge; per-channel fallback).
- [ ] Settings › Control Surfaces macros (SET > 64 B): an error, no crash (bridge OVERSIZE).
- [ ] Meters update smoothly (60 ms status poll over UART).

## 5. Numbers to record

| | Mac hub (Wi-Fi) | Bridge |
|---|---|---|
| Connect → first screen (snapshot) | | |
| GET status round trip (median / p95; logcat) | | |
| Slider drag feel (1–5) | | |
| Reconnect after hub restart | | |
| Battery: 30 min background REMOTE, % | | |

## 6. Network handling (WP AN4; agents had no phone, so none of this has run on one)

- [ ] Hub off, tap it: "failed". Wait 2 min: no dial in the hub's log, no retry on a timer. Turn phone Wi-Fi off and on: exactly one dial per return.
- [ ] Failed with the app in front and the hub starts: DNS-SD sees it, one dial, connected.
- [ ] Change the hub's DHCP lease during a session (F5): connection lost, then connected at the new address with no PIN, board and settings kept. Also after power-cycling an ESP32 bridge that gets a new address: if the phone does not find it until the app is brought to the front, record that (NsdManager may not report a known name again).
- [ ] Wi-Fi without internet, mobile data on (F10): the hub connects; `adb shell dumpsys connectivity` shows Wi-Fi not validated; the app's sockets go over Wi-Fi. Then a Wi-Fi with internet: the binding is gone (no `bind` warning in logcat `DspiLink`).
- [ ] Reconnecting with the screen off, hub stopped: after 10 min the notification goes, logcat says the service stops, `_dspi._tcp` queries stop. Bring the app to the front: notification back, "reconnecting" still shown, the hub reconnects when started.
- [ ] Same, hub restarted after the notification went, app still in the background: when it reconnects, no crash; the notification comes back when the app is next in front.
- [ ] Battery: `adb shell dumpsys batterystats` over a night with a hub that stays away, no multicast lock held after the first 10 min (`dumpsys wifi`).

### F10's items, as decided (`LINK-HUB-FINDINGS.md`)

| Item | Decision | Why |
|---|---|---|
| Board flaps on the bridge | No work: handled | The core reports `NoDevice` and re-attaches; the engine does not detach or dial (`wpAn4ABoardFlapOnABridgeIsNoReconnect`). A real bridge is still section 5 of the findings. |
| Service refused in the background | **Used** | `start()`'s result now counts: a refusal releases the shared service's hold, and the next look at a bound state tries again, at the latest when the app comes to the front. |
| WifiLock | **Rejected** | Without a wake lock the CPU sleeps regardless, so it would not keep the socket answering; with one it is the battery cost this client should not have for a remote control. A hub's 30 s limit is one more reconnect, which the core does. Also AOSP's `WifiServiceImpl` asks for `WAKE_LOCK` on acquire (from memory, not checked against a device), which `tools/ci.sh release` would refuse, unlike the plan's note. |
| Wi-Fi without internet | **Used** | `bindProcessToNetwork` while a session exists and no local network is validated (`LinkNetworkEdges`). Lifted otherwise, so other traffic is untouched. Not bound with validated Wi-Fi. Data safety re-checked in `RELEASE.md`: the answer stands. |
| Partial write on the reader | No work: fixed | `socket.rs`: a failed flush marks the pipe broken and nothing more goes out; `dspi-link` tests that. |
| Command slots | **Rejected** | `RATE_LIMITED` fails the write and the user sees it. Retrying would be the write queue the owner's rule forbids. |
