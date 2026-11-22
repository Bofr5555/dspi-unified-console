# Object names: LAY2 (popover content split from its hosts)

Smoke section: `lay2-content` (order 950, `smoke/SmokeLay2.cpp`), in the window
`SmokeLay2Window` (action `smoke.lay2`).

The content files keep the names their popovers had, so a name below is found
the same way in a Classic popover and in an inspector section: search inside
the popover (its `contentItem`) or inside the section.

| Content file | Classic host (name unchanged) | Names inside |
|---|---|---|
| `LimiterSettingsContent` | `LimiterSettings` | `LimiterSettings.*` (qp8.md); `active` polls the gain reduction |
| `LinkwitzContent` | `Linkwitz` (a band row's `linkwitzPopover`) | `Linkwitz.*` (qp2.md); `active` starts the draft from the band; `fp`, `dirty` |
| `GraphOptionsContent` | `GraphHost.options.popover`, `GraphWindow.options.popover` | `<prefix>.rtaSlot`, `.setup`, `.popOut` and the setup page's (qp3.md); `setup` (also on the popover) |
| `DashboardLayoutContent` | `DashboardLayoutPopup` | `DashboardLayoutPopup.tile.<n>`, `.summary` (qp5.md) |
| `DevicesContent` | `DevicesPopup` | `DevicesPopup.*` (qp13.md); signal `done` |

| Name | What |
|---|---|
| `SmokeLay2Window` | the test inspector; `channel`, `band`, `output`, `devicesDone`, `graphDone` |
| `Lay2.<limiter, linkwitz, dashboard, graph, devices>` | an `InspectorSection`; `expanded`, `shown` |
| `Lay2.<...>.heading` | its fold button |
| `Lay2.<...>.content` | the content in it; the graph options there use the prefix `Lay2.graphOptions` |

Screenshot: `window-lay2-inspector.png`.
