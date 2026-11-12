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
