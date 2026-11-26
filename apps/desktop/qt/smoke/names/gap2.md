# Object names: GAP2 ("Solo this output")

Smoke section: `gap2-solo` (order 992), `smoke/SmokeGap2.cpp`.

- `ContextMenus.channel.channel.solo`: the channel menu's Solo row (an output's; "End Solo" while that output is
  soloed; off for an output that is off or a device that takes no writes).
- `RoutingInspector.solo`: the Routing inspector's Solo switch under Mute (an output's column).
- `SoloBanner`: the status strip's banner while an output is soloed ("Solo: only Left tweeter · 2 plays. ...");
  `SoloBanner.end`: its End Solo.
- `SoloController` (QML singleton): `output` (from 0, -1 with none), `available`, `solo(o)`, `end()`, `toggle(o)`.
