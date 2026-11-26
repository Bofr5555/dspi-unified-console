# Notices: firmware versions, hubs and updates

*User manual of uConsole for DSPi 0.16.0, for everyone who uses the consoles. The technical side is
[docs/integrator/protocol-generations.md](../integrator/protocol-generations.md) and, for the hub,
[docs/integrator/hub-api.md](../integrator/hub-api.md). This page describes the Terminal, the web console, the desktop
app and the Android app of this release, and the hub. The fork firmware to use is still
[v1.1.6-bofr1](https://github.com/Bofr5555/DSPi/releases/tag/v1.1.6-bofr1), unchanged since 0.14.0. The Swift app for
macOS, iPhone and iPad is not part of this release and has not had these changes; that is said where it matters.*

DSPi firmware now comes in two versions that talk to the consoles in different ways: **firmware 1.1.6** (and older)
and **firmware 1.1.7**. Our consoles work with both. This page lists what you may see because of that, what each
message means, and the few things our consoles cannot protect you from.

## 1. Which firmware a DSPi runs

Every time a console connects to a DSPi, it first asks which of the two versions it runs, and from then on speaks only
that one to it. Nothing is remembered between connections, so after an update the console finds out again.

Where you can see it:

| Console | Where | What it shows |
|---|---|---|
| Terminal | `uconsole info`, and **Settings › About** in the interface | board, firmware version (1.1.6 or 1.1.7 with its protocol version), the list of command pages, and how the extensions are reached |
| Web console | the **Device** screen, just before Settings, under "Connected Device" | the same |
| Desktop app | **Settings › Advanced**, under "Protocol" | the same, with **Raw Request…** |
| Android app | **Settings › About**, the section "Device" at the end | board, which of the two firmware versions (1.1.7 with its protocol version), the command pages, and how the extensions are reached |

"Extensions" are this fork's own additions: the audio link, the reset reason, the core voltage setting and a few more.
Stock firmware has none of them; the console simply does not offer those parts then. Which features need
the DSPi fork firmware ([Bofr5555/DSPi](https://github.com/Bofr5555/DSPi), branch `bofr-main`) is listed in [`../FEATURES.md`](../FEATURES.md#firmware-stock-vs-fork), marked †.

## 2. Messages you may see when connecting

| Title | What it means | What to do |
|---|---|---|
| **Newer protocol** | The DSPi runs firmware newer than this console knows. Nothing was sent to it after the check. | Update the console. |
| **Unrecognised device** | The device answered neither as firmware 1.1.7 nor as 1.1.6 or older. Nothing else was sent to it. | Check that it is a DSPi and reconnect it. If it keeps failing, update its firmware. |
| **Hub too old for this firmware** | You reach the DSPi through an older DSPi Link hub (version 1.2), and the DSPi probably runs firmware 1.1.7, which such a hub cannot serve. Only one harmless question was sent to it. | Update the hub, or connect the DSPi directly. |
| **Hub serves only firmware 1.1.7** | The hub is a newer upstream one (DSPi Link 1.3) and the DSPi runs firmware 1.1.6 or older, which that hub does not pass on. Nothing was sent to it. | Connect the DSPi directly, use our hub (it serves both), or update the DSPi's firmware. |
| **Firmware changed** | The DSPi behind the hub was updated while you were connected and now runs the other version. Nothing more went out on the old connection. | Wait while the console opens it again. If it does not come back, select it again. |

Our own hub serves both firmware versions to our consoles and to older upstream consoles. Newer upstream consoles
(DSPi Link 1.3) leave a DSPi on firmware 1.1.6 alone behind any hub, as their protocol says.

## 3. On our hub, the extensions are for admins

When you use our hub with a PIN, a session with the **control** role does not get the extension channel. So a control
session sees no audio link status, no reset reason and no core voltage. An **admin** session gets all of it. With open
access (no PIN) every session is an admin, so everyone sees it.

Why: the same channel carries the core voltage and other settings that change the hardware itself, so the hub treats
all of it as a setting for admins. The desktop app says so on the alink sender's page when the hub refuses it.

## 4. A K340 must not get stock firmware from an upstream console or hub

**Read this if you own a K340.** The K340 runs this fork's firmware of the 1.1.6 generation, at a core voltage of
1.20 V, which it needs to run reliably. Stock firmware runs the core at 1.15 V and has none of our extensions.

An **upstream** console or hub (Weeb Labs' own apps, or their DSPi Link hub) does not know any of this. It sees a DSPi
on an older firmware and shows it as "legacy, update available". If you accept that offer, it installs stock firmware
on the K340. **Our components cannot prevent that**: the offer and the install happen entirely in the upstream
software.

What would be lost: the 1.20 V setting (the K340 may then hang or restart at random), the audio link, the extension
channel, and the stored IO, control-interface and Control Surfaces settings (stock firmware reads them as missing and
starts from its defaults).

So: **never update a K340 from an upstream console or hub.** Update it only from our consoles, which know the K340 and
keep it at 1.20 V. If it has happened, put this fork's K340 firmware back with one of our consoles as soon as you can.

Our consoles ask before they replace this fork's firmware with anything that may be stock (section 6).

## 5. The DSPi Zero is never updated by our consoles

The DSPi Zero (Weeb Labs' small board) has no BOOTSEL button bootloader that our consoles could update it through. So
our consoles never offer it an image, never restart it into a bootloader, and our hub never includes it in a firmware
rollout (in a group rollout it is listed as "excluded", with the reason). It is shown as "DSPi Zero" in device info.
One gap remains in the Swift app (not part of this release): its developer option that bypasses the core can still send
a Zero the bootloader request.

**One case we cannot detect:** a DSPi Zero on firmware 1.1.7, reached through an **upstream** DSPi Link 1.3 hub. Such a
hub does not pass on what kind of board it is, and the Zero then looks like an ordinary DSPi. A console would then offer
it an update and could send it the request to restart into its bootloader. **Do not update a DSPi Zero through an
upstream hub.** A Zero on firmware 1.1.6 is recognised on every route, and so is a Zero on 1.1.7 that is connected
directly or through our hub.

## 6. What our consoles ask before an update

Before an update starts, a console checks three things:

- **A board without BOOTSEL** (the DSPi Zero, or a board the console cannot identify) is offered no image at all.
- **Replacing this fork's firmware with an image that may be upstream's** asks you to type
  `I ACCEPT LOSING THE EXTENSIONS`. The message lists what would be lost (as in section 4). If the core voltage rule
  asks for its own phrase as well (a K340 at 1.20 V), that one phrase covers both, and the message names both risks.
  An image the console can prove is ours asks nothing extra.
- **A change of firmware version** (1.1.6 to 1.1.7 or back) is shown with what works afterwards. For example, after a
  move to 1.1.7 an older DSPi Link 1.2 hub can no longer serve the DSPi, and consoles older than this one cannot control
  it.
- **An image for the other kind of device is refused.** Every firmware image of this fork carries a stamp saying
  whether it is for a DSP or for the alink sender. The sender's image is refused on a DSP, and a DSP image on the
  sender, whatever you type. An image without that stamp (upstream's, or this fork's from before the stamp) is
  still offered to a DSP as before; on the alink sender it may be a DSP image, so the console asks you to type
  `I ACCEPT THAT THIS MAY BE A DSP IMAGE` (when another phrase is asked as well, that one covers it). The sender has no
  core voltage setting, so the sender's own stamped image asks no voltage phrase.

Which consoles ask this before the restart into the bootloader:

| Console | State |
|---|---|
| Terminal (`uconsole flash`) | asks; `--force` does not answer the phrase for you, and nothing types it |
| Desktop app | asks; the field for the phrase starts empty |
| Web console | has no firmware install of its own; its **Check an Image…** and the hub's rollout dialog show the same verdict |
| Android app | asks; the field for the phrase starts empty |
| The Swift app (not in this release) | not yet |

The last check is in the core itself, at the moment of writing: even a console that did not ask first cannot write such
an image without the phrase, as long as it read the board before restarting it. The Android app writes the image
itself, so it asks the core for that same last check just before it writes. But a console that has not been changed
yet (the Swift app) may already have restarted the board into its bootloader before it is refused. Nothing is written then; the board
waits in its bootloader until you unplug it and plug it in again.

A board that you put into its bootloader by hand (holding BOOTSEL) cannot be asked which firmware it ran. Then only the
core voltage rule applies, and the console asks its phrase whenever it cannot tell the board's voltage.

## 7. The alink sender

The alink sender (the board that sends audio over the audio link) is not a DSP, so a console offers only what it has:
its device info, the audio link block and the firmware update. Every DSP screen is left out.

- Web console: a page of its own with exactly those (its update part is **Check an Image…**).
- Desktop app: a page of its own, with one banner; every DSP menu, shortcut and tool window is greyed while the sender
  is open.
- Android app: a page of its own in place of the main screen, with Firmware Update and Settings.
- The desktop app and the Android app do not offer the sender their bundled firmware, which is for the DSP: choose the
  sender's own image file. A DSP image is refused there by its stamp (section 6).
- The sender speaks both firmware generations (1.1.6 and the paged 1.1.7), and our hub recognises it either way.
- Terminal: `uconsole info` and **Settings › About** say it is the alink sender. The interface itself cannot open it
  yet, since it needs a DSP.

## 8. For advanced users: raw requests in the Terminal

`uconsole raw` sends one request as it is, for diagnosis. It refuses, with nothing sent:

- an opcode above `0xFF` (a firmware 1.1.7 command goes by its id: `uconsole raw --id 0x0550 4`);
- a wValue or a length that does not fit 16 bits (before, such a value went out cut short, as a different request).

`uconsole raw --id` works only on firmware 1.1.7. It refuses every command that writes the flash, restarts the device
or moves the whole settings image. For those, use the 1.1.6 form with `uconsole raw out <opcode>`: the console
translates it for firmware 1.1.7. Saving, factory reset and the bootloader still go only through their own commands,
which ask you to confirm.
