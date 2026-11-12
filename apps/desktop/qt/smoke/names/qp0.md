# Object names: QP0 (the base)

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
| `SmokeProbeWindow.<component>` | the component sample: `button`, `switch`, `checkBox`, `comboBox`, `spinBox`, `tabBar`, `tab2`, `scrollBar`, `heading`, `popover`, `question` |
| `QuestionDialog.accept`, `QuestionDialog.reject`, `QuestionDialog.alternative` | a QuestionDialog's three buttons |
