# Object names: LAY5a (Modern's Overview, Routing, Effects and Analyse)

Smoke section: `lay5a-pages` (order 896, `smoke/SmokeLay5a.cpp`). Its screenshots are `gallery/lay5a-<width>-<section>-<tab>-<scheme>.png`
and `gallery/lay5a-1280-overview-hub.png`.

- `PageTabs.<tab id>.dot` (`on`; hidden while the tab has none). `PageHost.page.<section>/<tab>.aside` (loader): a section's inspector.
- `OverviewSection`, `OverviewSection.devices` (without a device); `OverviewTools`, `OverviewTools.perRow` (`.0` Auto to `.3`);
  `OverviewSummary` with `.device`, `.effects`, `.connection` (boxes); the facts `.name`, `.board`, `.firmware`, `.preset`, `.transport`,
  `.hub`, `.security`, `.alink`, `.alarms` (`shownName`); `.effect.<tab id>` (`on`); `.devices`.
- `Dashboard.head` (loader), `DashboardCard.buttons`, `Dashboard.open.<wire channel>` (Modern only).
- `RoutingInspector` (`.sheet`, `.close`, `.scroll`), `.none`, `.equaliser`; `RoutingTools`, `.inspector`;
  `Matrix.header.<output>.select`, `Matrix.row.<row>.select` (on the page only).
- `EffectsChain` (`.sheet`, `.close`, `.scroll`), `.summary`, `.<stage>` (`input`, `matrix`, `crossover`, `output`, `gain`, `limiter`, `delay` and
  the effects' tab ids), `.<effect>.on`, `.input.bypassed`; `EffectsTools`, `.limiters`, `.inspector`.
- `AnalyseTools`, `.stop`, `.graph`.
