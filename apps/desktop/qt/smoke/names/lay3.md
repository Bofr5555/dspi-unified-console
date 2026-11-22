# Object names: LAY3 (the Modern shell)

Smoke section: `lay3-modern` (order 890, `smoke/SmokeModern.cpp`). Its screenshots are `gallery/modern-<width>-<section>-<tab>-<scheme>.png`
and `modern-*.png`.

- `ModernShell`; `ModernShell.sender`, `ModernShell.wizard` (loaders); `ModernShell.toast`, `ModernShell.toast.banner`.
- `TopBar` and `TopBar.preset`, `.presetMenu`, `.ab`, `.search`, `.ai`, `.aiPopover`, `.theme`, `.themeMenu`, `.theme.<value>`, `.undo`,
  `.redo`, `.save` (`.save.button`, `.save.more`, `.save.menu`), `.lock`, `.bell`. (-- WP LAY3b -- the menus: names/lay3b.md)
- `AppMenuButton`, `AppMenu`.
- The device tabs by place: `DeviceTabs` (the top bar), `DeviceRow` (several), `DeviceRail` (the rail's top), each with
  `.tab.<device id>` (or `none`), `.close.<device id>`, `.menu.<device id>`, `.add`, `.strip`, `.unsecured`, `.access`.
- `DeviceSwitcher`, `DeviceSwitcher.<action id>`, `DeviceSwitcher.manage`.
- `NavRail` (the rail) and `NavTabs` (across the top), each with `.<section id>`.
- `PageTabs`, `PageTabs.title`, `PageTabs.<tab id>`, `PageTabs.openWindow`.
- `PageHost`, `PageHost.page.<section>/<tab>` and its `.scroll`, `.inWindow`, `.raise`, `.bringBack`.
- `OverviewSection`, `EqualiserSection` (`.channels`), `DevicesSection`.
- `Dock` and `Dock.source`, `.mode`, `.volume`, `.readout`, `.mute`, `.bypass`, `.meters`, `.meter.<wire channel>`, `.cpu<n>`.
- `StatusStrip`; `NotificationDrawer`, `.sheet`, `.banners`, `.earlier`, `.clear`, `.events`, `.close`.
- `Appearance.theme` (Settings > Appearance). `PresetDialogs`: the `GlobalControls.*` menus and dialogs, names unchanged.
