# Object names: QP7 (Effects A)

Smoke section: `qp7-effects` (order 70, `smoke/SmokeEffects.cpp`).

Tool windows, opened by their menu rows (`Registrar::Window`):

| Window | Menu row | Size (min) |
|---|---|---|
| `CrossfeedWindow` | `tools.crossfeed` | 780×470 (740×460) |
| `LoudnessWindow` | `tools.loudness` | 780×400 (740×390) |
| `LevellerWindow` | `tools.leveller` | 380×540 (380×400) |
| `PsybassWindow` | `tools.psybass` | 780×500 (740×496) |

Shared parts, named after their window (`<W>` is `CrossfeedWindow`, `LoudnessWindow`, ...):

| Name | What |
|---|---|
| `<W>.header` | the header; `<W>.header.switch` is the effect's on/off switch |
| `<W>.graph` | the graph (`EffectGraph`); `active` is false while the effect is off ("Disabled") |
| `<W>.<param>` | a parameter row (`EffectParameterRow`): signals `live(v)`, `commit(v)`; `liveValue` while held; `<W>.<param>.value` its field (`text`), `<W>.<param>.slider` its slider |

Crossfeed:

| Name | What |
|---|---|
| `CrossfeedWindow.preset.<0..3>` | preset rows: Default, Chu Moy, Jan Meier, Custom |
| `CrossfeedWindow.parameters` | the parameter rows' column; `opacity` 0.5 while a preset other than Custom is chosen |
| `CrossfeedWindow.freq`, `.feed` | the rows; they show the preset's own values while dimmed |
| `CrossfeedWindow.itd` | interaural time delay switch |
| `CrossfeedWindow.pairs` | the Output Pairs section (firmware with the pair mask or the pair map) |
| `CrossfeedWindow.pairs.presets` | its Presets menu; rows `CrossfeedWindow.pairs.all`, `.first`, `.none` |
| `CrossfeedWindow.pairChips` / `CrossfeedWindow.plainPairChips` | the chips' grid with a pair map / without one |
| `CrossfeedWindow.pair.<p>` | a pair chip; `on`, `text` ("1" or "1: Out 1 / Out 2"), `toolTip` |
| `CrossfeedWindow.pairMap` | the pair map controls (hidden without the extension's pair map); `pick("default"|"k340"|"custom")`, `setDraft(pair, "left"|"right", output)`, `draft`, `draftValid`, `editing` |
| `CrossfeedWindow.pairMap.menu` | the Outputs menu button (`text` is the current choice); rows `CrossfeedWindow.pairMap.choice.default`, `.k340`, `.custom` |
| `CrossfeedWindow.pairMap.offer` | the K340 question; `.offer.enable`, `.offer.notNow` |
| `CrossfeedWindow.pairMap.editor` | the custom editor; `.editor.left.<p>`, `.editor.right.<p>` (output pickers), `.editor.warning`, `.editor.apply`, `.editor.cancel` |

The window itself has `editingPairs` (the editor is open).

Loudness: `LoudnessWindow.ref`, `.intensity`; `LoudnessWindow.outputs` (the mask section, wire V19);
`LoudnessWindow.outputs.presets` with rows `.outputs.all`, `.outputs.first`, `.outputs.none`; chips `LoudnessWindow.output.<n>`.

Leveller: `LevellerWindow.channels` (the masks, more than two inputs, wire V18) with `.channels.presets` and rows
`.channels.all`, `.channels.center`, `.channels.front`; chips `LevellerWindow.detector.<n>`, `LevellerWindow.apply.<n>`;
`LevellerWindow.amount`, `.maxGain`, `.gate`; `LevellerWindow.speed` (tabs `.speed.0..2`) and `.speedText`;
`LevellerWindow.lookahead`.

Psychoacoustic bass: `PsybassWindow.body` (wire V23) or `PsybassWindow.unsupported` (the note);
`PsybassWindow.startingPoints` with rows `PsybassWindow.startingPoint.<0..3>`; `PsybassWindow.graph.original`,
`PsybassWindow.graph.harmonics` (the sketch's bands); `PsybassWindow.cutoff`, `.harmonics`, `.drive`, `.character`,
`.original`; `PsybassWindow.outputs` with `.outputs.presets` and rows `.outputs.all`, `.outputs.noSub`, `.outputs.none`;
chips `PsybassWindow.output.<n>`.

Screenshots: `window-crossfeed.png`, `window-crossfeed-preset.png` (dimmed rows), `window-crossfeed-k340.png` (with the
question), `window-crossfeed-editor.png`, `window-crossfeed-editor-short.png` (refused pairs),
`window-crossfeed-stock.png` (a stall), `window-crossfeed-readonly.png`, `window-loudness.png`, `window-leveller.png`,
`window-psybass.png`, `window-psybass-unsupported.png`.
