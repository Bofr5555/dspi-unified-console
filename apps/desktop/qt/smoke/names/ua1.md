# Object names: UA1 (UXP A1, C5: one name per output in Modern)

Smoke section: `ua1-names` (order 991), `smoke/SmokeUa1.cpp`.

- `EqualiserCrossover.row.<output>.name`, `.number`: a Crossover row's output name and its number ("· 1", hidden
  when the name says it: "Out 1 · L", "3 · None").
- `EqualiserDelay.row.<output>.name`, `.number`: the same in Delay & Gain.
- `EqualiserCrossover.unused`, `EqualiserDelay.unused` (`EqualiserUnusedRow`): the row the unused outputs fold under,
  "5 unused"; its text `.text`. A click opens or folds them.
- `Matrix.header.<output>.name`: the column head's name (Modern: the speaker layout's name over the OUTn pill).
- `RoutingInspector.title.output`: the crosspoint title's output name.
- `RoutingInspector.alsoFeeds.add`: "+", the menu of the outputs the input does not feed
  (`RoutingInspector.alsoFeeds.menu`, rows `RoutingInspector.alsoFeeds.menu.<output>`).
