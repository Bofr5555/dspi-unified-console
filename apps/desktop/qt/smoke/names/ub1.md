# Object names: UB1 (UXP B1: crossovers and routing from the speaker layout)

Smoke section: `ub1-layout` (order 992), `smoke/SmokeUb1.cpp`.

- `EqualiserCrossover.tools`: the Crossover page's tool row; `EqualiserCrossover.fromLayout`: its "From Speaker
  Layout…" button.
- `RoutingTools.fromLayout`: Routing's "From Speaker Layout…" button (the routing part ticked).
- `SpeakersPage.fromLayout`: Settings › Speakers' "Crossovers and Routing…" (shown while the layout is confirmed).
- `CrossoverFromLayout` (the sheet): `.crossovers`, `.routing` (the parts' check boxes), `.blocker` (the notice
  when nothing can be set; its button `.blocker.action`, "Open Speakers…"), `.problem` (why Apply waits),
  `.refusal` (what apply() refused), `.floor` (the floor line), `.noCrossovers`, `.oneStep`, `.cancel`, `.apply`,
  `.scroll`.
- `CrossoverFromLayout.speaker.<row>`: a speaker's row; `.same` its "Same as Left" check box;
  `.transition.<i>` a transition, with `.words` ("woofer ◂▸ tweeter"), `.freq` (a ChannelValueField: `.freq.field`)
  and `.slope`.
- `CrossoverFromLayout.route.<i>`: an input's routing line; `.outputs` its "→ Left woofer · 1, …".
