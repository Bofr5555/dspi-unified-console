# alink-scope: alink timing between DSPi units on a PicoScope 2000A MSO (WP SCOPE1)

Measures how far apart several DSPi units play the same sample, with the owner's PicoScope 2000A series scope
(USB `0ce9:1016`) and its 16 digital inputs. Each unit runs a **test firmware** that raises a marker pin at the same
sample of the shared time base (USB sample index S in alink USB-direct mode 2, or the alink frame F). The scope
captures the markers; the tool reports each unit's offset against a reference unit in ns and in samples at 48 and
96 kHz, the jitter, the drift over a long run, and PASS/FAIL against +-1 sample (default).

Firmware side: DSPi fork, branch `wp/SCOPE1`, `Documentation/Bench/scope_marker.md` (where the edge is and why the
edge difference equals the audio difference).

Everything here runs in a podman container. The host needs only a udev rule; no Pico software on the host.

## 1. Wiring

| Unit | Marker pin | Lock pin (optional) | Scope |
|---|---|---|---|
| reference unit (e.g. the K340) | GP2 (Pico pin 4) | GP3 (pin 5) | D0 (marker), D8 (lock) |
| unit 2 | GP2 | GP3 | D1, D9 |
| unit 3 ... | GP2 | GP3 | D2, D10 ... (markers on D0-D7, locks on D8-D15) |

- **Common ground:** a ground wire from every unit (Pico pin 3, next to GP2, or any GND pin) to the scope's digital
  ground (the MSO cable's GND leads). The units are on one USB host, so their grounds already meet there; the short
  ground lead at each probe keeps the edges clean.
- Keep the marker wires short and of similar length (a 1 m difference is 5 ns; the threshold is 20 833 ns).
- K340: GP2/GP3 are module pads 4 and 5, not connected on the board (probe or tack a wire on the castellation).
- Lock pins are only for looking at; the tool needs the markers. A lock pin low during a run means that unit lost its
  phase lock.
- Optional, for clock lock without the marker: probe each unit's I2S LRCLK (GP15). Mode 1 (rate lock): all LRCLKs
  have the same frequency, at a steady but arbitrary phase. Mode 2 (phase lock): the phase between them is constant.

## 2. Flash the test images (owner's step)

Images (DSPi fork `wp/SCOPE1`, marker GP2, lock GP3, N 4096; sha256 in `~/.cache/dspi-build/work/wp-SCOPE1/images/SHA256SUMS`):

| File | Board |
|---|---|
| `DSPi-scope-gp2-k340.uf2` | K340 (RP2350, 1.20 V core, USB MIDI) |
| `DSPi-scope-gp2-pico2.uf2` | stock Pico 2 (RP2350, USB MIDI) |
| `DSPi-scope-gp2-pico.uf2` | stock Pico (RP2040, USB MIDI) |

Back up each unit first (as for every flash), copy the UF2 in BOOTSEL mode, then on every unit: input source USB,
`EXT_ALINK_USB` mode 2 with the same D (e.g. `02 01 c0 00 00 00 00 00`, D 192 at 48 kHz), same output type, same
DSP delays. The marker runs once the unit is phase-locked (state 5) and needs no audio stream. In mode 0 or 1 the
marker stays low. Restore mode 0 and the normal firmware afterwards.

## 3. Host setup and the container (owner's go)

Once, on the host (sudo, the owner's step): a udev rule that gives the logged-in user access to Pico devices.

```sh
sudo tee /etc/udev/rules.d/60-picoscope.rules <<'R'
# PicoScope (Pico Technology, 0ce9): access for the seat's user (alink-scope container, WP SCOPE1)
SUBSYSTEM=="usb", ENV{DEVTYPE}=="usb_device", ATTR{idVendor}=="0ce9", TAG+="uaccess"
R
sudo udevadm control --reload && sudo udevadm trigger --subsystem-match=usb --attr-match=idVendor=0ce9
```

(`uaccess` needs a rule file sorting before `73-seat-late.rules`, hence `60-`. Pico's own package would instead
write `95-pico.rules` with `MODE="0666"` for everyone; not needed here.) Re-plug the scope; then
`getfacl /dev/bus/usb/BBB/DDD` shows `user:bofr:rw-`.

Build the image (downloads from Pico's official repositories; done once, already done by WP SCOPE1):

```sh
tools/alink-scope/scope.sh build          # localhost/dspi-alink-scope:1
tools/alink-scope/scope.sh test           # unit tests, no device
```

What `scope.sh run --go` does with the device (rootless podman):

```sh
dev=$(tools/alink-scope/scope.sh find)    # e.g. /dev/bus/usb/003/012, from sysfs (VID:PID 0ce9:1016)
podman run --rm -it --network none --security-opt label=disable \
  --device "$dev" --group-add keep-groups \
  -v "$PWD/tools/alink-scope":/tool:ro -v ~/.cache/dspi-build/work/alink-scope:/out -w /out \
  -e PYTHONPATH=/tool localhost/dspi-alink-scope:1 python3 -m alink_scope info
```

- `--device` passes only the scope's node, nothing else on the bus. In rootless podman the container's root is the
  host user, so the node's uaccess ACL is what grants access; `--group-add keep-groups` covers a group-based rule
  instead (crun, Fedora's default). `--security-opt label=disable`: SELinux would otherwise deny the device.
- No network in the container.
- If the scope re-enumerates when the driver opens it (a new device number), the node passed in goes stale and
  `info` fails with "not found": run it again (the new node is passed), or as a last resort pass the bus directory
  (`--device /dev/bus/usb/BBB`, which also exposes the other devices on that bus).

## 4. Run

```sh
S=tools/alink-scope/scope.sh
$S run --go info                                   # model, serial; must say "MSO ... yes"
$S run --go capture --channels D0,D1,D2 --label D0=k340 --label D1=pico2 --label D2=pico
$S run --go capture --channels D0,D1 --mode window --windows 50          # small memory / best resolution
$S run --go capture --channels D0,D1,D2 --every 60 --duration 3600       # one hour, a capture a minute
$S analyse ~/.cache/dspi-build/work/alink-scope/<run>/edges.csv --channels D0,D1,D2   # again, offline
```

Options that matter: `--fs 96000` when the units run at 96 kHz (marker period and threshold), `--n` if the firmware
was built with another `ALINK_SCOPE_MARKER_N`, `--threshold-samples`, `--level` (logic threshold, 1.5 V for 3.3 V
CMOS), `--analog` (A and B instead of the digital inputs; automatic when the scope reports no MSO).

- **span** (default): one block over `--periods` (4) marker periods (341 ms at 48 kHz) at the fastest timebase the
  memory allows (`--max-samples`, 50 M): with 32 MS that is about 12 ns per sample.
- **window**: blocks triggered on the reference's rising edge, `--window-us` (200 us) around it, at the fastest
  timebase: for a 2205A MSO (48 kS) or for the best resolution. A unit more than half a window away counts as missing.

Output, in `~/.cache/dspi-build/work/alink-scope/alink-scope-<time>/`: `edges.csv` (every edge), `pairs.csv`
(per capture and pair), `summary.txt`, `run.log`. Ctrl-C ends a long run early and still writes the summary.

## 5. Reading the result

```
PASS  pico2 vs k340: +1000 ns = +0.048 smp@48k / +0.096 smp@96k; jitter std 120 ns, p2p 450 ns; n 4, missing 0, drift +2.0 ns/s
OVERALL PASS (threshold +-20833 ns)
```

- The **offset** is the unit's marker minus the reference's: positive = the unit plays later. With the same output
  type, rate and DAC on both, it is their audio offset. An I2S unit against an S/PDIF unit: subtract 2 frames
  (41.7 us at 48 kHz) from the S/PDIF unit's side (Documentation/Bench/scope_marker.md).
- **Resolution**: digital edges are good to half a sample interval (`dt` in `run.log`).
- **Jitter**: std and peak-to-peak of the offsets. The phase servo holds about +-0.3 sample (+-6 us); much more means
  a problem.
- **missing / extra**: edges without a partner. A few are normal (the firmware skips an edge it could not place
  exactly); more than 10 % fail.
- **Not locked**: a unit whose marker period differs from the reference's by more than 10 ppm is not phase-locked
  (mode 1 or a free-running crystal); its offset slides by the ppm (25 ppm = 25 us/s) and the pair fails.
- **Long run**: `drift` of the capture means in ns/hour, the range of the means, and how many captures failed; any
  failed capture fails the run.
- Edges only define the offset modulo the marker period (85.3 ms at 48 kHz): an offset near half a period cannot be
  matched and reads as "no matched edges".

## Files

| File | |
|---|---|
| `Containerfile` | Ubuntu 24.04 + Pico's `libps2000a` (apt, key fingerprint pinned) + picosdk-python-wrappers (pinned commit) + numpy |
| `scope.sh` | build / test / analyse / find / run --go |
| `alink_scope/analysis.py` | edges, pairing, statistics, verdicts (numpy only) |
| `alink_scope/capture.py` | ps2000a capture (span and window), analog fallback |
| `alink_scope/cli.py` | `info`, `capture`, `analyse` |
| `tests/` | synthetic edges and a fake ps2000a driver |
