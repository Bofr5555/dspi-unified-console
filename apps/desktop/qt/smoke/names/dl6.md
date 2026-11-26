# Object names: DL6 (Modern's Overview: one card per channel)

Modern's Overview, `qml/modern/pages/OverviewSection.qml` and `OverviewChannelCard.qml`:

- `OverviewSection.empty` (no device; its button `OverviewSection.empty.action`, replacing `OverviewSection.devices`).
- `Dashboard` (the Flickable of tiles and cards; Classic's Overview's name, kept), `OverviewSection.inputs`, `OverviewSection.outputs`
  (the two groups), `OverviewSection.stale` (the meters-stale chip).
- `Dashboard.open.<wire channel>`: a channel's card (a button that opens it on the Equaliser), as before.
  Inside it: `OverviewChannelCard.name.<ch>`, `.role.<ch>` (text), `.response.<ch>` (canvas), `.meter.<ch>`,
  `.chip.<ch>.<key>` (`shownName`; keys `bands`, `highPass`, `lowPass`, `feeds`, `delay`, `gain`, `inverted`, `muted`).
- Gone: `OverviewTools.perRow` (cards per row), `Dashboard.grid`, `Dashboard.head`, `DashboardCard.buttons` in Modern.
