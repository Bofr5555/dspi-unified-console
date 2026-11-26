# Both protocol generations: what a client of the core needs to know

*2026-11-19. The design is [`docs/core-merge/PLAN-PG.md`](../core-merge/PLAN-PG.md) (sections 1.1 to 1.11);
this page is what a program calling the core (C ABI, UniFFI, wasm, `uconsole`) has to know about it, as built after
wave PG-D (revised at the PG-D part 2 review). The user's page is [docs/manual/notices.md](../manual/notices.md); the hub's surface is
[hub-api.md](hub-api.md) (sections 3.1, 17 and 21). Where this page and the code disagree, the code is right.*

## 1. The two generations

| Generation | Firmware | On the wire |
|---|---|---|
| legacy | upstream 1.1.6 and older, this fork's `bofr-main` (with or without PAGEGUARD), the DSPi-Zero today, the alink sender of the 1.1.6 line | 1.1.6 opcodes on page 0; this fork adds the extension channel `REQ_EXTENSION` 0x00 |
| paged | upstream 1.1.7, and this fork's later port of it (dual-stack), the alink sender included (FWPG4) | commands by id `0xPPCC` (page `PP`, command `CC`, `wIndex = (PP << 8) \| 2`); this fork's extensions on the private page 0xE0, tunnel `0xE010`, `wValue` = the extension id |

The core decides the generation **once per connection, before anything else is sent** (`dspi_transport::paged::open`):
the probe `SYS.PROTOCOL_INFO` (GET `0x0110`), which 1.1.6 firmware STALLs; then `REQ_GET_PLATFORM` on a legacy
candidate. Over DSPi Link the hub's device object decides and no probe is sent (hub-api.md section 3.1). It is never
cached across connections: a reconnect, a firmware update and a return from the bootloader decide it again.

Every call of the core keeps its 1.1.6 shape. On a paged device an adapter generated from upstream's `legacy_map.json`
turns each into paged requests; a command the device does not list is refused locally, as `Refused` with status 0 (a
STALL), so feature detection by "probe and cache" keeps working. Firmware refusals on a paged device carry the
firmware's result code (`VC_ERR_*`, 0x10 to 0x1A) as `DspiError::Refused { status }`.

## 2. Reading the generation: `device_protocol()`

`DspiCore::device_protocol()` (C `dspi_device_protocol`, wasm `deviceProtocol`) answers a `DeviceProtocol`:

| Field | Meaning |
|---|---|
| `generation` | `Unknown`, `Legacy` or `Paged` |
| `protocol_major`, `protocol_minor` | the paged protocol's version; 0 and 0 on a legacy device |
| `pages` | `[{page, revision}]`, ascending; empty on a legacy device. A paged device reached through the hub's in-process paths (`LocalTransport`, `PrincipalTransport`) lists them too: they pass the attachment's discovery on |
| `ext_envelope` | `NoChannel`, `Legacy` (0x00) or `Tunnel` (0xE010) |
| `board_id`, `board_name` | 0 "DSPi" (a Pico), 1 "DSPi Zero", anything else "Unknown board" (0xFF: a USB product string this build does not know, or a board that could not be read) |
| `device_kind` | `Dsp`, or `AlinkSender` (section 4) |

**It costs one platform read** (the board; the rest sends nothing). Call it **once per connection**, off the UI thread,
and keep the answer: the Terminal, the web console, the Qt console and the Android app do. A demo device opened without
the generation wrapper says `Unknown`. For smoke runs the C ABI opens the demo as each fake generation
(`dspi_demo_open_generation`, with `dspi_demo_generation_violations` counting requests the generation must not get):
fakes only, never a device.

## 3. Raw requests

| Call | Takes | Refused locally, nothing sent |
|---|---|---|
| `raw_get` / `raw_set` | a 1.1.6 opcode (8 bits) | `raw_set`: `SET_ALL_PARAMS` and the chunk opcodes (the core's bulk session), the opcodes behind a confirmation token (preset save and delete, `SAVE_PARAMS`, `SAVE_OUTPUT_CONFIG`, factory reset, bootloader: use their `_confirmed` calls), an `EXT_CORE_VREG` write, more than 65535 bytes |
| `raw_get_id` | a paged id `0xPPCC` | a page-0 id (use `raw_get`), a GET of length 0, the chunk read; any id on a legacy device or on a connection that does not say it is paged (`Unsupported`) |
| `raw_set_id` | a paged id `0xPPCC` | the above, and every command whose 1.1.6 equivalent is gated or a bulk session, every core-voltage request (through the tunnel too), **every command the registry or the device's own `SYS.COMMAND_INFO` flags `FLASH`, `RESTART` or `BULK`**, and, with no 1.1.6 equivalent, anything but a plain value SET of a command this build knows |

- `raw_get` and `raw_set` work on both generations: on a paged device the adapter translates the 1.1.6 request. **For a
  command `raw_set_id` refuses as `FLASH`, `RESTART` or `BULK`, send its 1.1.6 form through `raw_set`.** On 1.1.7 the
  `RESTART` flag also covers `INPUT.SOURCE` and the pin and clock commands, which `raw_set` allows on 1.1.6; the
  refusal is deliberately conservative.
- `raw_set_id` reads `SYS.COMMAND_INFO` on every call (one round trip more).
- In the client read-only mode `raw_get_id` passes any GET of a registry command or the tunnel; a GET of an id outside
  both, and every `raw_set_id`, is refused.
- The Terminal: `uconsole raw` refuses an opcode above `0xFF` and a wValue or length above 16 bits before anything is
  opened (they went out truncated before); `uconsole raw --id <0xPPCC|PAGE.NAME>` and `raw out --id` go through
  `raw_get_id` / `raw_set_id`; `raw war --id` is refused (a 1.1.6 form only).

## 4. The alink sender: `device_kind` `AlinkSender`

The alink sender (`firmware/DSPi-Sender`) answers a small vendor set: the extension channel with its alink block, the
platform, serial and build info, its output-config save and the bootloader entry. On the 1.1.6 line it is a legacy
device (guarded by PAGEGUARD); after the port (FWPG4) it is dual-stack: the same set on the paged stack (pages SYS and
STORE) and the tunnel on page 0xE0, and page 0 kept for legacy hosts. The core connects it on either generation with a
**limited connection**:

- recognised only after the connect's bulk read failed (a device whose image reads is a DSP, whatever it is called);
- **legacy**: by its USB product string ("DSPi Audio Link Sender", "DSPi Link Sender"), or without one (MIDI, behind
  a hub) by `REQ_GET_PLATFORM` naming 0 output channels;
- **paged**: by the connection's discovery, all of: (1) the same identity rule on `SYS.PLATFORM` (the
  1.1.6 layout), (2) the tunnel `0xE010` on page 0xE0 (this fork's firmware), (3) no `STORE.ALL_PARAMS` GET in its
  command bitmaps (no bulk image), (4) no page listed but SYS (0x01), STORE (0x02) and 0xE0. Anything else is not the
  sender (fail closed): a paged DSP stays a DSP, a paged device without the tunnel or with any other page fails to open
  as before, and so does a paged connection that carries no discovery. Our hub's in-process and principal
  connections pass the attachment's discovery on, so the sender opens through them too; there the rule
  has no product string, so the platform's 0 outputs decide, as over DSPi Link. Our hub applies the same rule at
  attach (with the USB product string) and asks the sender nothing of the DSP model: no image read (`device.snapshot`
  answers `busy` at once), no poll of what its bitmaps do not list (`unsupported`, page-0 polls included). Since its command bitmaps hold nothing of the DSP model, the adapter refuses every DSP request
  locally, so nothing DSP-only reaches the sender's wire even before its kind is settled;
- behind a 1.2 hub the core speaks legacy only, and a dual-stack sender is the legacy sender (its page-0 stack);
- works: `device_protocol`, `fetch_all` (re-reads the platform only), `ext_status`, `alink_info`, `fetch_alink_*`,
  `reset_alink_stats`, `firmware_target` and the update check (it is a Pico board, so an update is offered);
- refused as `Unsupported` with nothing sent: every DSP setter, `apply_all_params`, `refresh_params`, `fetch_status`
  (no meters); over DSPi Link no meter poll is subscribed;
- a per-item DSP read that a client sends anyway goes out and is STALLed. **Use `device_kind` to show only the sender's
  screens**: the device info, the audio link block and the update.
- `device_protocol` says the generation and the envelope (`Legacy`, or `Tunnel` on the paged stack); the clients' sender
  pages (web, Qt, Android, Terminal) key on `device_kind` only and need no change per generation.
- **Updates.** The sender is judged as a Pico running this fork's firmware, the same on both generations (its core
  voltage is not known, so the voltage phrase is asked for any image but its own stamped `alink_sender` image, which
  is clear (INT-R1: the sender has no core-voltage setting, so there is nothing for the voltage rule to protect); an
  image not proven ours also names the lineage loss).
  From IMGKIND the update check and the write gate also tell a sender image from a DSP image by the image's
  **device-kind stamp** (section 5): a DSP image (`dsp`) is refused on a sender, and the sender's image
  (`alink_sender`) on a DSP, whatever is typed. An image with no kind stamp (upstream's, and this fork's from before
  IMGKIND) may be a DSP image: on a sender it asks the typed confirmation and the message says so. **Still do not
  offer the sender the bundled DSP images** (Qt and Android do not): they carry no stamp.
- **Not yet:** the dual-stack sender firmware (FWPG4) is written (the fork's `bofr-main`, round R1) but has not run on
  its board (PGHIL), so the paged rule is tested against its fake and the firmware's host tests only; a sender build
  that lists any page beyond the three (a debug build's diagnostics page 0xF0 too) is not recognised (the firmware
  spec states this page set in `extension_channel_spec.md` 1.3.5 from IMGKIND). Our hub tells the paged sender apart
  at attach (above).

## 5. Updates: the update check and the write gate

- `DspiCore::firmware_target()` (C `dspi_firmware_target`, wasm `firmwareTarget`) reads the board as an update target:
  the voltage record, the board (8 platform bytes and the USB descriptor), its lineage (`EXT_LIST`, read once) and the
  generation, and (IMGKIND) `device_kind`, the open connection's `DeviceKind` (`None` when not known: no device, a
  board in BOOTSEL; JSON `deviceKind`, left out when not known). Up to five control transfers; call it before the
  bootloader entry, off the UI thread.
- `firmware_update_check(image, target, typed)` (C `dspi_firmware_update_check`, wasm `firmwareUpdateCheck`) folds
  three rules into the voltage rule's verdict: `rule` `NoBootsel` (no image, whatever is typed), `LineageLoss` (this
  fork's firmware, or a lineage that could not be read on an open board, giving way to an image not proven ours asks
  `LINEAGE_PHRASE` "I ACCEPT LOSING THE EXTENSIONS"; when the voltage rule asks too, its phrase covers both and the
  message names both), and `generation_change` (a sentence with what works afterwards, `""` when none). **Flash only on
  `verdict.allowed`.**
- **The device kind** (IMGKIND). Every DSPi image of this fork carries a binary-info stamp (`ID_AND_STRING`, tag `DS`,
  id `0x0B7A4B1D`, firmware `image_kind.h`) with the text `dsp`; the alink sender's image carries `alink_sender`.
  Against the target's `device_kind`:

  | Image | Target `DSP` | Target `AlinkSender` | Target kind not known |
  |---|---|---|---|
  | stamped for the target's kind | the rules above | the rules above, but the voltage rule does not ask for the sender's own image (INT-R1: no core-voltage setting on a sender) | the rules above |
  | stamped for the other kind | `rule` `KindMismatch`: refused, whatever is typed | `KindMismatch`: refused | the rules above |
  | kind stamp unreadable (another text, two that disagree) | `KindMismatch`: refused | `KindMismatch`: refused | `KindMismatch`: refused |
  | no kind stamp (upstream, ours before IMGKIND) | the rules above, unchanged | asks: the message says the image may be a DSP image; `rule` `KindUnknown` with `KIND_PHRASE` "I ACCEPT THAT THIS MAY BE A DSP IMAGE" when nothing else asks; when the voltage or lineage rule asks, its phrase covers it (and `LineageLoss` keeps its name) | the rules above |

  A refusal has `decision` `Refused`, `reason` `ImageInvalid`, no phrase, and the write returns `InvalidArgument`. A
  kind stamp also proves the image ours (`image_lineage` `Bofr`), as the voltage stamp does: the sender's image has no
  voltage stamp (no `core_vreg.c`). The write gate's fallback (below) has no target, so it applies no kind rule.
- **The write gate** (PGFLASH): `flash_confirmed` (C `dspi_flash_confirmed`) and `picoboot_flash_confirmed` judge by the
  update check's rules on the target **the core read itself** from the device: `firmware_target` keeps what it read, and
  the bootloader entry reads it when it was not read on this connection. A caller's target JSON is never used; a
  caller's voltage record only adds doubt (a record that differs from the core's makes the voltage unknown). The kept
  target is dropped when another device is opened and after a write.
- **A client that writes itself** (Android: PICOBOOT or the drive, in Kotlin) never reaches that gate through a write,
  so it asks it: `DspiCore::firmware_write_check(image, board, typed)` (UniFFI only) answers what `flash_confirmed`
  would decide now, on the core's own kept target, and writes nothing. A target the client kept is not a substitute:
  the core drops its own when another device is opened, the client's copy does not know.
- **The fallback:** when this core read no target (a board already in BOOTSEL before this core saw it, a fresh core such
  as the hub rollout's writer), the gate is the voltage rule alone, as before PG. So **write on the core that read the
  target** (or entered the bootloader), and never pass a voltage record kept from another board.
- A client that still judges with `firmware_guard_check` alone asks nothing for an unstamped image on this fork's
  firmware at 1.15 V, restarts the board into BOOTSEL and is then refused at the write gate: nothing is flashed, and
  the board waits in BOOTSEL until it is power-cycled. Judge with the update check **before** the entry, as
  `uconsole flash` does.

| Client | Update screen on the update check |
|---|---|
| Terminal `uconsole flash` | built (PGTM): asks `LINEAGE_PHRASE` on the terminal, `--force` does not answer it; the generation change is said; no image and no reboot for a board without BOOTSEL |
| Qt | built (PGQT): the check before the entry, the phrase in an empty field, the write through `dspi_flash_confirmed` on the same core; no bundled image for the alink sender |
| Web | no firmware write; the check is shown (Check an Image, the rollout dialog) (PGWEB) |
| Android | built (PGAND): the check before the entry, the phrase in an empty field; the write is Kotlin's, and its last check is `firmware_write_check` (PG-D part 2 review); no bundled image for the alink sender |
| Swift | not yet (PGSWIFT) |

## 6. Known limits

- **A paged DSPi-Zero behind an upstream DSPi Link 1.3 hub** reads as a Pico: 1.1.7's `SYS.PLATFORM` answers 7 bytes
  (no board id), the USB descriptor is not visible through the hub, and an upstream hub passes no `board`. The update is
  offered and `SYS.ENTER_BOOTLOADER` would go out (admin only on such a hub). Behind our hub the board comes in the
  device object (`board`), so it is blocked there; a legacy Zero is known by byte 7 on every route.
- The Swift opt-out paths (`-CoreTransport NO`) send `REQ_ENTER_BOOTLOADER` themselves with no board check (PGSWIFT).
- On our hub the extension channel (`0x00` and the tunnel `0xE010`) is classed `config`: a control session gets none of
  it (audio link status, reset info, core voltage); an admin does, and with open access every session is admin.
- An upstream console or hub can offer stock firmware to a K340 (it sees "legacy, update available"); our components
  cannot prevent that (the manual's section 4).
