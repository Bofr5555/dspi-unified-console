# Object names: LAY5b (Modern's Measure, Library, System, Devices and Settings)

Smoke section: `lay5b-pages` (order 898, `smoke/SmokeLay5b.cpp`). Its screenshots are `gallery/lay5b-<width>-<section>-<tab>-<scheme>.png` and
`gallery/lay5b-{pair,hub,installation,make-again,slot-menu}.png`.

- A panel's parts are tabs of their section (`PageTabs.<part id>`) and share one page, named for the panel's own tab:
  `PageHost.page.measure/measure`, `PageHost.page.system/status`, `PageHost.page.settings/settings` (`PageHost.pageKey`).
- `DevicesSection` (This Computer): `.access`, `.accessHelp`, `.device.identify`, `.device.firmwareUpdate`, `.device.reconnect`, `.hubs`.
  The list's own rows keep `DevicesPopup.*` (`DevicesLocal`, `DevicesLink`).
- `DevicesHubs`: `.pair` (card) with `.text`, `.key`, `.hubKey`, `.unsecured`, `.unsecuredInfo`, `.fromLink`, `.pin`, `.error`, `.cancel`, `.accept`;
  `.rePair` with `.text`, `.cancel`, `.accept`; `.hub` with `.name`, `.role`, `.security`, `.key`; `.keyNotice`, `.tokenNotice` (`.ok`), `.lockNotice`;
  `.networking`.
- `LibraryPresets`: `.list`, `.none`, `.startup`, `.slot.<n>` with `.name`, `.note`, `.recall`, `.save`, `.menu`.
- `LibraryFiles`: `.body`, `.<action id>`, `.open`, `.noTemplates`, `.template.<n>`.
- `Installation.openInTab.<unit id>`, `Installation.lost` (`shownName`), `.remake`, `.dropLost`.
