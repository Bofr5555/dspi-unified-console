# Object names: QP5 (the Overview)

Smoke section: `qp5-dashboard` (order 50, `smoke/SmokeDashboard.cpp`).

| Name | What |
|---|---|
| `Dashboard` | the Overview in the right pane (shown while no channel is selected) |
| `Dashboard.stale` | the "Meters stale" note; visible only while no meter reading arrives |
| `Dashboard.grid` | the card grid; `columns` |
| `Dashboard.card.<key>` | a card: `input`, `out<l>-<r>` for an output pair, `out<n>` for one output (output numbers from 0) |
| `Dashboard.column.<wire channel>` | a card's column; `shownName` is its heading |
| `Dashboard.delay.<wire channel>` | a column's delay readout (`text`, empty on inputs) |
| `Dashboard.meter.<wire channel>` | a column's `Meter`; `level`, `clipping`, `barWidth()` |
| `Dashboard.gear` | the layout gear of a card (one per card; search inside the card) |
| `DashboardLayoutPopup` | the layout popup (a child of its gear); `opened` |
| `DashboardLayoutPopup.tile.<n>` | its tiles: 0 Auto, 1–3 cards per row |
| `DashboardLayoutPopup.summary` | the line under the tiles |

Screenshots: `window-dashboard.png`, `window-dashboard-layout.png`, `window-dashboard-stale.png`.
