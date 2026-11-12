# Firmware control over UART and I2C

A microcontroller can drive a DSPi without a computer: over a UART, or as an I2C
master talking to the DSPi as a target. Both carry the same vendor commands as
USB (the `REQ_*` opcodes), not a separate command language.

Reference (the firmware, extension branch `bofr-main` at `de9614c`):
`Documentation/Features/control_interfaces_spec.md` (the specification, with
worked master-volume examples in section 6), `Documentation/commands.md`
section 17.1, `firmware/DSPi/uart_control.h`, `firmware/DSPi/i2c_control.h`,
`firmware/DSPi/vendor_commands.h` (status codes). In this repository:
`crates/dspi-proto/src/packets.rs` (`dev.uart`, `dev.i2c` and the pin rules).

## Turning it on

Both interfaces are off as shipped and are set **over USB only**:
`SET_UART_CONFIG` (0xF5) and `SET_I2C_CONFIG` (0xF7); read back with 0xF6, 0xF8;
`GET_CTRL_IFACE_STATUS` (0xF9) says whether each is live. The setting persists
across reboots and across a factory reset. If its pins clash with other wiring
at boot, an interface stays down even when enabled: check its live flag.

| | UART | I2C target |
|---|---|---|
| Config (8 bytes, LE) | `enabled, tx_pin, rx_pin, notify_enable, baud u32` | `enabled, sda, scl, address, reserved[4]` |
| Defaults | TX GPIO 16, RX GPIO 17, 115200 baud | SDA GPIO 18, SCL GPIO 19, address 0x42 |
| Range | 9600 to 1 000 000 baud, 8N1, no flow control | 7-bit address 0x08 to 0x77; master at most 400 kHz |
| Pins | TX pin % 4 = 0, RX pin % 4 = 1, one UART instance; RX pulled up | SDA even, SCL odd, one I2C instance, not the one a control-surface display uses; external 2.2 to 4.7 kΩ pull-ups |

3.3 V logic only; use a level shifter for a 5 V host.

To write: how each client sets this up (the Console's hardware page, `uconsole`).

## Framing

**UART**: every frame starts with sync byte `0xA5`, then a type byte; CRC16-CCITT-FALSE
over everything after the sync byte, little-endian.

```
SET request   A5 01 bReq wVal wIdx wLen payload[wLen] crc
GET request   A5 02 bReq wVal wIdx wLen crc            (wLen caps the reply; 0 = full)
SET reply     A5 81 status len(=0) crc
GET reply     A5 82 status len payload[len] crc         (payload only when status is OK)
notification  A5 40 00 len packet[len] crc              (the v2 notify packet, as USB's)
```

(`wVal`, `wIdx`, `wLen`, `len` and `crc` are 16-bit little-endian.)

**I2C**: one write `[type 01=SET / 02=GET][bReq][wVal][wIdx][wLen][payload]`,
then a read of `[status][len LE][payload]`, padded with 0xFF. No sync byte, no
CRC. A read before the command ran answers BUSY: read again. A new write drops
any unread reply.

Status: 0 OK, 1 BUSY, 2 ERROR, 3 BLOCKED, 4 BULK_LOCKED, 5 CRC_ERROR,
6 OVERSIZE, 7 FRAME_ERROR.

## Limits

- Refused on UART and I2C (BLOCKED): 0xF5 and 0xF7, so a controller cannot lock
  itself out, and the chunked bulk commands 0xA2/0xA3; use the whole-image
  0xA0/0xA1 instead.
- A few reads are USB only (the analyser's all-bands read, a clock diagnostic
  larger than the reply buffer): they answer ERROR.
- A SET payload is at most 64 bytes (bulk excepted); a GET reply at most 132.
- One request at a time. A UART frame stalled for 100 ms between bytes is
  dropped. Flash writes block for about 45 ms: allow a few hundred milliseconds
  and retry on BUSY.

## Notifications and attribution

- UART pushes notifications (type 0x40 frames) when `notify_enable` is set,
  between frames only. A gap in the packets' sequence numbers means some were
  lost: read the whole image again (`REQ_GET_ALL_PARAMS`). I2C is poll-only.
- A change made over UART or I2C carries source 8 (UART) or 9 (I2C) in its
  notification, so clients name it: the [event log](event-log.md) records the
  source, and [undo](undo.md) leaves a value another writer changed alone.

## To write

- A complete example per bus (an Arduino or Pico SDK sketch setting the master
  volume and reading it back), from the spec's section 6.
- Which commands an embedded controller usually needs, as a short table.
- Coexistence with a computer on USB and clients on a hub at the same time.
