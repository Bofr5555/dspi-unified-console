# Android on the new core: phone record (AN13)

AN13's record (`docs/core-merge/PLAN-CLIENTS.md`, **AN13**). Done when: the earlier HIL set passes on the nusb `from_fd`
path: USB, notifications, old firmware, Link with a Mac hub, 45 s background, no secrets in logcat. The `.hil` build runs
side by side with the Play build (`applicationIdSuffix ".hil"`); the Play build is never uninstalled.

0.12.0 shipped (tag `android-v0.12.0` = eb66e5e5, uploaded to Play by the owner) on the partial checks below.

## Sources

| source | what |
|---|---|
| coordinator log `~/.cache/dspi-build/work/clients-wave1.md` | the owner's own phone checks, as the owner reported them |
| `~/.cache/dspi-build/work/phone-test-2/REPORT.md` | agent run, demo mode |
| `~/.cache/dspi-build/work/release-check-2/REPORT.md` | agent run, About screen on the release tip |

## The phone

Nothing A063, Android 15, 1080×2400 px, font scale 1.0. Wireless adb at 192.168.35.139:41559 (the port changes; the owner
gives it). adb ran in a container (`localhost/dspi-android-build:2026-11-01-play`) as root with volume `dspi-adb-keys` as
HOME. The Play app `se.bofr.dspi_console_android` stayed at 0.10.0 (code 10), untouched in both agent runs.

## 1. The owner's checks (owner's hands, real DSPi on USB)

- **Date:** 2026-11-07, before 20:15 (from the order of the log; the log entries carry no times).
- **Build:** not recorded. Most likely the `.hil` 0.12.0 build from `release/android-0.12` @ 4535dda2 that was installed at
  17:32 that day (phone-test-2 found it installed); **unclear**.
- **Device and firmware:** "the DSPi" on the phone's USB; the unit and firmware were not recorded (**unclear**).

| check | result, as reported |
|---|---|
| AN29: the USB chooser | **pass**: the chooser listed USB Audio Player PRO, DSPi and DSPi HIL; "Always" chosen; replugged 3 times with no prompt |
| AN30: the graph overlay below 200 Hz | **pass**: bass shown on the graphs |
| USB basics | **pass**: "no banner, delay stuck" (the owner's words; read as: no firmware banner, and a delay write held) |

Not reported: notifications on USB, logcat during the USB checks, the device restored or diffed.

## 2. phone-test-2 (agent, demo mode)

- **Date:** 2026-11-07 20:15–20:21. **Build:** `se.bofr.dspi_console_android.hil` 0.12.0 (code 12) from
  `release/android-0.12` @ 4535dda2, not rebuilt.
- **Device:** `dumpsys usb` listed a "Weeb Labs DSPi" on the phone's USB, but the app showed "No DSPi connected", so no real
  device was open. Nothing was unplugged; demo was entered from the connection strip ("Nothing is sent over USB"); the app
  log has no USB or connect lines. No device writes, so no backup or diff.

| # | check | result |
|---|---|---|
| 1 | Tools › Spectrum Analyser, "Stats for nerbs" (spelt so in the report), Interrupt Monitor each open their own tab | pass |
| 1 | Tools › Signal Generator opens its tab and does not start ("Idle") | pass |
| 2 | graph bottom-left labels "-25" and "20" do not overlap (landscape and portrait) | pass |
| 3 | analyser Inputs/Outputs and Curves/Bars/Both fit; "Curves" not clipped (both orientations) | pass |
| 4 | Networking shows "Local" and "from here"; the `macr` hub ("user's iMac DSPi Console", 192.168.35.206) was discovered, **not connected**; Link switched back off | pass |
| 4 | firmware page (out of demo) shows "A local .uf2 file"; only viewed | pass |
| 5 | About shows Version 0.12.0 | pass |
| 6 | logcat: no crash, ANR or secrets (0 matches for pin/token/secret/password/bearer/auth in the app log) | pass |

Observations, not blocking: the analyser status line is cut at the right edge; in landscape the analyser's dB labels crowd;
in portrait the Analysis tab row scrolls. Deviation: auto-rotate was on (landscape); it was turned off at the coordinator's
request (`accelerometer_rotation` 1 → 0) and left off. App left out of demo mode, Link off, as found.

## 3. release-check-2 (agent, About on the release tip)

- **Date:** 2026-11-07 20:30–20:50. **Build:** debug APK from `release/android-0.12` @ 3c5c7d0b (CG29), with the local `.hil`
  suffix (reverted afterwards), installed over the `.hil` app: 0.12.0, code 12, same signature.
- **Device:** none on USB (`dumpsys usb` host_connected=false).

| check | result |
|---|---|
| About: "Version 0.12.0", the section title "Weeb Labs: links and support" and its note; "Links & Support" gone | pass |
| logcat: no crash; no E/F lines and no pin/token/secret/password/bearer matches from the app | pass |
| rotation: `accelerometer_rotation` 0 before and after | unchanged |

The shipped build is eb66e5e5 (CG29b, "I (Weeb Labs)" in the support paragraph), one commit after this check; it was not
put on the phone by these runs (**unclear** whether the owner ran it before the upload).

## What this covers of the done criterion

| item | state |
|---|---|
| USB on the nusb `from_fd` path | owner's USB basics, AN29 and AN30 pass; only the owner's run had a device open |
| notifications on USB | not checked |
| old firmware (AN2's read-only proof on the phone) | not checked |
| Link with a Mac hub (or the ESP32 bridge) | not checked: the hub was only discovered, not joined |
| 45 s in the background | not checked |
| no secrets in logcat | checked in demo and without a device only; **not on USB** |

## Still owed (owner, with the phone)

- **The Link half:** pair with and drive a Mac hub (XC2) or the ESP32 bridge (XC4) from the phone; PIN pairing and token
  reuse.
- **Older firmware:** a DSPi on V17–V31 firmware: read-only, no write, the banner (AN2).
- **45 s in the background** on USB and on Link, then return (AN3's reconnect and foreground service).
- **Logcat on USB:** no PIN, token or other secret while a real device is open, and while pairing on Link.
- Notifications on USB (another client's write shows on the phone), and notes on the nusb `from_fd` path.
- **Once they ship:** the LK2 Link changes (first meter, stall tolerance, dead-hub close) on the phone; AN15's phone run of the
  R8 build (AN22); AN20's TalkBack pass. A user report of 0.12.0 over the ESP32 bridge (meters after ~2 min) led to LK1 and LK2;
  it has not been reproduced on this phone.
