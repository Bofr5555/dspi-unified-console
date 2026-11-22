# Object names: LAY4 (Modern's Equaliser and inspector)

Smoke section: `lay4-equaliser` (order 895, `smoke/SmokeLay4.cpp`). Its screenshots are `gallery/lay4-<width>-<tab>-<scheme>.png`,
`gallery/lay4-darker-<scheme>.png` and `lay4-*.png`.

- `PageTabs.tools` (loader); `EqualiserTools`, `EqualiserTools.<action id>`, `EqualiserTools.inspector`.
- The pages: `EqualiserPeq`, `EqualiserCrossover`, `EqualiserDelay`, each with `.channels` (not Delay), `.inspector` (`.sheet`, `.close`,
  `.scroll`) and `.enableQuestion`. Pages are kept, so a name in all three needs a visible-first lookup.
- `EqualiserPeq.bands` (a `FilterList`: `.band.<n>.*`, `.enableAll`, `.bypassAll`, `.clearAll`, `.clearQuestion`, `.rows`).
- `EqualiserGraph.rta`, `.phase`, `.all`, `.popOut`; on the graph `PeqEditor.bandLabel` (`shownName`), `PeqEditor.hint`.
- `ChannelStrip`, `.name`, `.enable`, `.route.<input>`, `.routeGain.<input>`, `.gain`, `.gainSlider`, `.delay`, `.mute`, `.polarity`, `.limiter`,
  `.link` (`.linkQuestion`), `.preamp`, `.preampSlider`.
- `EqualiserCrossover.plot`, `.curves`, `.scroll`, `.row.<output>` with `.highPass` and `.lowPass` (`.type`, `.freq`, `.more`), `.bands`.
- `EqualiserDelay.scroll`, `.row.<output>` with `.enable`, `.gain`, `.delay`, `.distance`, `.arrival` (`share`), `.polarity`, `.mute`, `.limiter`.
- The inspector: `ChannelInspector`, `.title`; `Inspector.<section>` (`band`, `feeds`, `headphones`, `crossover`, `limiter`, `linkwitz`; each
  with `.heading`), `Inspector.feeds.<output>`, `.editRouting`, `.editOutputRouting`, `.browseProfiles`, `.highPass`, `.lowPass`, `.tweeter`,
  `.addLinkwitz`; `BandInspector.none`, `.off`, `.several`, `.shape.<type>`, `.slope`, `.freq`, `.gain`, `.q`, `.on`, `.invert`, `.reset`;
  and the contents' own names (`LimiterSettings.*`, `Linkwitz.*`).
- `Appearance.tone`, `.inspector`, `.bandBar` (Settings > Appearance).
