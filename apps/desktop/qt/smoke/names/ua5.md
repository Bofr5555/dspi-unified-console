# Object names: UA5 (Settings landing, keyboard paths, dock words)

Sections: `ua5-settings` (order 991) and `ua5-keys` (992), `smoke/SmokeUa5.cpp`.

| Name | What |
|---|---|
| `Advanced.protocol` | Settings > Advanced's "Protocol" group: the open device's protocol rows (`DeviceInfoRows`), was About's `About.device` |
| `Advanced.raw` | its Raw Request… button (was `About.raw`) |
| `NavRail.<section>` | a rail button: one Tab stop (the section on show); arrows, Home, End move |
| `PageTabs.<tab>` | a page tab: one Tab stop (the tab on show); Left, Right, Home, End move |
| `ShortcutsDialog`, `ShortcutsDialog.list`, `ShortcutsDialog.close` | Keyboard Shortcuts (help.shortcuts, Ctrl+/) |
| `ShortcutsDialog.row.<id>`, `ShortcutsDialog.key.<id>` | a row and its keys: a menu row's action id (a submenu's first row's), or `palette`, `area`, `areaBack`, `arrows`, `tabNext`, `tabBack`, `shortcuts` |
| `Dock.mode.tip` | the User / Master help (the segments' `help` property holds the text) |
| `Dock.dsp`, `Dock.dsp.tip` | the one DSP load readout (was `Dock.cpu0`, `Dock.cpu1`) and its help |
