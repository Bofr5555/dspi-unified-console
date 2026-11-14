# USB-direct installation: Cinema 7.1

Written by `uconsole usb-direct generate` (WP AUSB3) for the installation `cinema`. Nothing in this folder has been
written to a unit. The guide is `docs/integrator/usb-direct-home-theater.md`.

- Programme: 8 channels (FL FR FC LFE RL RR SL SR) at 48 kHz, PipeWire sink `dspi-usb-direct.cinema` ("Cinema 7.1").
- Quantum: 256 samples (5.33 ms), fixed.
- Mode: 2, phase lock (sample-aligned). D = 192 samples (4.00 ms) on every unit; the units take 170..399 at 48 kHz.
- Keep USB-direct units aligned: on.

## Units

| Unit | Serial | USB stream | Mode | D | On lock error |
|---|---|---|---|---|---|
| Front L/C/R and sub (front) | `E6614C311B4A8B2F` | 4 channels, 24 bit, 48 kHz (RP2350) | 2 phase lock | 192 (4.00 ms) | mute |
| Sides (sides) | `E6614C311B4A9C10` | 2 channels, 24 bit, 48 kHz (RP2350) | 2 phase lock | 192 (4.00 ms) | mute |
| Rears (rears) | `E6614C311B4AA0D7` | 2 channels, 24 bit, 48 kHz (RP2350) | 2 phase lock | 192 (4.00 ms) | play unaligned |

### Front L/C/R and sub (front)

| USB input | Channel | Output | Distance |
|---|---|---|---|
| 0 | FL | 0 (pair 1 L) | 3.40 m |
| 1 | FR | 1 (pair 1 R) | 3.40 m |
| 2 | FC | 2 (pair 2 L) | 3.10 m |
| 3 | LFE | 8 (PDM sub) | 2.60 m |

### Sides (sides)

| USB input | Channel | Output | Distance |
|---|---|---|---|
| 0 | SL | 0 (pair 1 L) | 1.90 m |
| 1 | SR | 1 (pair 1 R) | 2.10 m |

### Rears (rears)

| USB input | Channel | Output | Distance |
|---|---|---|---|
| 0 | RL | 0 (pair 1 L) | 2.40 m |
| 1 | RR | 1 (pair 1 R) | 2.40 m |

## Steps

1. **Cables and hubs** as the guide says: a multi-TT hub, active USB 2.0 cables,
   isolators at the unit end, every unit on one USB controller of this computer.
2. **PipeWire.** Copy `pipewire/dspi-usb-direct-cinema.conf` to
   `~/.config/pipewire/pipewire.conf.d/` and `wireplumber/dspi-usb-direct-cinema.conf`
   to `~/.config/wireplumber/wireplumber.conf.d/`, then
   `systemctl --user restart wireplumber pipewire pipewire-pulse`.
   `wpctl status` lists "Cinema 7.1" and one "... (DSPi USB-direct)" sink per unit;
   `pw-top` shows 48000 Hz and quantum 256 on the driver's row (the other units follow
   it and show 0); `pw-dump` shows each unit's format. Make "Cinema 7.1" the default sink.
3. **Routing.** For each unit: `uconsole --device <serial> -f units/<id>.dspi`, then save
   its preset. It sets the input source to USB and the crosspoints above.
4. **Mode, D and the lock-error action** are written by the hub (Installer, "USB-direct
   units"; Admin): add the units as USB-direct, mode phase lock, system delay D as above,
   "When it can't lock" as above. Or give the hub `hub-system-model.json` (`system.set`,
   `docs/integrator/audio-link.md`), then **Align**. The hub also equalises the units'
   stream start offsets with the outputs' delays.
5. **Check** each unit: `uconsole --device <serial> ext usb-direct` says phase-locked, a
   phase error under half a sample, no growing gap or late counts. The hub's summary
   says "USB-direct N of N locked".

