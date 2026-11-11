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

## Names in the base (QP0)

| Name | What |
|---|---|
| `MainWindow` | the main window |
| `Menu.bar` | its menu bar |
| `Menu.<action id>` | every menu row (42 in the base) |
| `BannerHost` | the banner column; `bannerCount` |
| `BannerHost.<type>` | the loader of a registered banner |
| `Sidebar` | the sidebar |
| `Sidebar.list` | its channel list |
| `Sidebar.channel.<wire channel>` | a channel row; `shownName` |
| `DeviceStatus.button` | the device button; `shownName` |
| `RightPane`, `RightPane.graph`, `RightPane.detail` | the right pane and its two placeholders |
| `SmokeProbeBanner`, `SmokeProbeWindow`, `SmokeProbeWindow.slider` | the smoke build's probes |

## Sections

Each work package's section is its own file, registered with
`SmokeDriver::Section("<id>-<screen>", order, fn)`; checks start with the
work package's id (`"qp1: ..."`). The base's section is `qp0-base` (order 0).
