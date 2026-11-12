# Object names for the smoke driver

The driver finds controls by `objectName`, so every control a section checks
has one, by this rule:

    <Screen>.<control>[.<index or id>]

- `<Screen>` is the QML file's type name without "Window" or "Page" where that
  reads better: `MainWindow`, `Sidebar`, `DeviceStatus`, `BannerHost`,
  `RightPane`, `Matrix`, `Settings`, `Networking`.
- `<control>` is a lower-camel-case name of what it is: `list`, `button`,
  `channel`, `gain`, `save`.
- A repeated control adds the index or id it stands for: `Sidebar.channel.3`
  (wire channel 3), `Matrix.cell.1.4`.
- A menu row is `Menu.<action id>`, the id in `qml/main/MenuTable.qml`.
- A banner is its QML type name (`SmokeProbeBanner`); BannerHost's loader for
  it is `BannerHost.<type>`.
- A tool window is its QML type name (`SmokeProbeWindow`).

A control the driver reads text from exposes what it shows as a `shownName`
property (`Sidebar.channel.<n>`, `DeviceStatus.button`), so a check does not
depend on the item tree inside it.

## Where the names are listed

Each work package lists the names it adds in its own file, `names/<wp>.md`
(`names/qp0.md` is the base's), so packages built in parallel never edit the
same file. QP19 consolidates them.

## Sections

Each work package's section is its own file, registered with
`SmokeDriver::Section("<id>-<screen>", order, fn)` and listed in its
`cmake/wp/<wp>.cmake` (`DSPI_SMOKE_SOURCES`); checks start with the work
package's id (`"qp1: ..."`). The base's sections are `qp0-base` (order 0),
`qp0-core` (1) and `qp0-components` (2); a work package uses order 10 or more.

## The sections, in run order (-- WP QP19 --)

The main run (`--demo`) runs the sections of mode Main; each other launch of
`run.sh` runs only its mode's (`SmokeDriver::Mode`, behaviour.md §23). The
names each section uses are in `names/<wp>.md`.

| Section | Order | Launch |
|---|---|---|
| `qp0-base`, `qp0-core`, `qp0-components` | 0–2 | main |
| `qp1-sidebar` | 10 | main |
| `qp19-slider-keys` | 15 | main (the probe window) |
| `qp2-channel` | 20 | main |
| `qp3-graph` | 30 | main |
| `qp4-peq` | 35 | main |
| `qp10-settings` | 40 | main |
| `qp11-hardware` | 45 | main |
| `qp12-surfaces` | 47 | main |
| `qp5-dashboard` | 50 | main |
| `qp6-matrix` | 60 | main |
| `qp7-effects` | 70 | main |
| `qp8-tools` | 72 | main |
| `qp9-siggen`, `qp9-monitor`, `qp9-stats`, `qp9-spectrum`, `qp9-follow` | 90–94 | main |
| `qp13-link`, `qp13-devices`, `qp13-link-fault` | 130–132 | main (with the fake hub) |
| `qp14-networking`, `qp14-tray`, `qp14-system` | 140–142 | main |
| `qp15-files`, `qp15-autoeq` | 150–151 | main |
| `qp16-firmware`, `qp16-access`, `qp17-onboarding`, `qp16-readonly`, `qp17-updates` | 160–170 | main |
| `qp19-gallery` | 900 | main, last |
| `qp19-demo-entry` | 10 | `demo-entry/` (`DSPI_SMOKE_DEMO_ENTRY`, no `--demo`) |
| `qp19-tray-launch` | 10 | `tray-hidden/`, `tray-hidden-quit/`, `tray-app-quit/`, `tray-no-tray/` (`DSPI_SMOKE_TRAY`) |
| `qp19-hw-preflight`, `-analyser`, `-status`, `-bypass`, `-reconnect`, `-end` | 10–90 | `hw-dry/` (`DSPI_SMOKE_HW=dry`); `run-hw.sh` (`=1`) |
| `qp19-replug` | 10 | `run-replug.sh` (`DSPI_SMOKE_HW=replug`) |
| `qp19-selftest` | 10 | by hand: `DSPI_SMOKE_SELFTEST=thread` must exit 1, `=noquit` exit 3 |

A mode with no section in the build (today `DSPI_SMOKE_PSEUDO`, QP18's) exits
2 and run.sh skips it.
