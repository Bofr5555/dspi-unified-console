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

| Unit | Marker pin | Scope |
|---|---|---|
| reference unit (e.g. the K340 PCB unit) | GPIO 22 (Pico / Pico 2 pin 29) | D0 |
| K340 prototype | GPIO 22 | D1 |
| plain Pico (RP2040) / Pico 2 ... | GPIO 22 | D2, D3, ... (up to D15) |

- **Common ground:** a ground wire from every unit (pin 28, next to GPIO 22, or any GND pin) to the scope's digital
  ground (the MSO cable's GND leads). The short ground lead at each probe keeps the edges clean.
- Keep the marker wires short and of similar length (a 1 m difference is 5 ns; the threshold is 20 833 ns).
- K340: GPIO 22 is module pad 29, not connected on the board (probe or tack a wire on the castellation).
- Fallback pin GPIO 21 (pin 27) needs a rebuild with `-DALINK_SCOPE_MARKER_GPIO=21`. Keep S/PDIF input 4 (GPIO 22)
  off on the bench units; for GPIO 21 also S/PDIF input 3 and, on an RP2040, MCK.
- Optional, for clock lock without the marker: probe each unit's I2S LRCLK (GP15). Mode 1 (rate lock): all LRCLKs
  have the same frequency, at a steady but arbitrary phase. Mode 2 (phase lock): the phase between them is constant.

## 2. Flash the test images (owner's step)

Images: DSPi fork branch `wp/SCOPE1-usb3b` = the reviewed USB3b image's source (844017f) plus the marker commits only,
marker on GPIO 22, N 4096, no lock pin. sha256 in `~/.cache/dspi-build/work/wp-SCOPE1/images-v2/SHA256SUMS`.

| File | Board | Build |
|---|---|---|
| `DSPi-scope-gp22-k340.uf2` | K340 PCB unit and prototype (RP2350) | the K340 config: pico2, `DSPI_USB_MIDI=ON`, `DSPI_VREG_MV=1200` |
| `DSPi-scope-gp22-pico2.uf2` | plain Pico 2 (RP2350) | pico2, USB MIDI |
| `DSPi-scope-gp22-pico.uf2` | plain Pico (RP2040) | pico, USB MIDI |

The same build without the marker is byte-identical to the reviewed USB3b K340 image except for the version stamp
(WP SCOPE1 report). Back up each unit first, copy the UF2 in BOOTSEL mode, then on every unit: input source USB,
`EXT_ALINK_USB` mode 2 with the same D (e.g. `02 01 c0 00 00 00 00 00`, D 192 at 48 kHz), same output type, same
DSP delays; never saved (no `0x52`). The marker runs once the unit is phase-locked (state 5) and needs no audio
stream; in mode 0 or 1 it stays low. Restore mode 0 and the normal firmware afterwards.

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
U="--unit D0=k340 --unit D1=k340-proto --unit D2=pico-a --unit D3=pico-b"
$S run --go info                                                   # model, serial; must say "MSO ... yes"
$S run --go capture --channels D0,D1,D2,D3 $U --label "baseline, no extension" --out baseline
$S run --go capture --channels D0,D1,D2,D3 $U --label "active 10 m, brand X" --out active-10m-x
$S run --go capture --channels D0,D1,D2,D3 $U --label "active 10 m, brand X, 1 h" --every 60 --duration 3600 --out x-hour
$S run --go capture --channels D0,D1 --mode window --windows 50   # small memory / best resolution
$S compare ~/.cache/dspi-build/work/alink-scope/{baseline,active-10m-x,x-hour}   # the table across runs
$S analyse ~/.cache/dspi-build/work/alink-scope/baseline/edges.csv                # again, offline
```

- `--label TEXT`: free text for the run (the cable type and length, what changed). `--unit CH=NAME`: the unit on a
  channel. Both go into `meta.json`, every row of `edges.csv` and `pairs.csv` (`run_label`, `unit`, `ref_unit`) and
  the summary; `analyse` and `compare` read them back from `meta.json`.
- `--fs 96000` when the units run at 96 kHz (marker period and threshold), `--n` if the firmware was built with
  another `ALINK_SCOPE_MARKER_N`, `--threshold-samples`, `--level` (logic threshold, 1.5 V for 3.3 V CMOS),
  `--analog` (A and B instead of the digital inputs; automatic when the scope reports no MSO).
- **span** (default): one block over `--periods` (4) marker periods (341 ms at 48 kHz) at the fastest timebase the
  memory allows (`--max-samples`, 50 M): with 32 MS that is about 12 ns per sample.
- **window**: blocks triggered on the reference's rising edge, `--window-us` (200 us) around it, at the fastest
  timebase: for a 2205A MSO (48 kS) or for the best resolution. A unit more than half a window away counts as missing.
- **compare RUN_DIR...**: one row per run and unit: run label, unit, reference unit, offset (ns and samples at
  48 kHz), jitter std and peak-to-peak, drift of the capture means (ns/hour, long runs), captures, missing edges,
  verdict. `--out DIR` also writes `compare.csv` (with 96 kHz samples and in-capture drift) and `compare.txt`.

Output, in `~/.cache/dspi-build/work/alink-scope/<--out>/`: `meta.json`, `edges.csv` (every edge), `pairs.csv`
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
| `alink_scope/cli.py` | `info`, `capture`, `analyse`, `compare` |
| `tests/` | synthetic edges and a fake ps2000a driver |
