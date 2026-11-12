# Object names: QP2 (filter lists and channel headers)

Section `qp2-channel` (order 20, `smoke/SmokeChannel.cpp`). The pages are shown
in `SmokeChannelWindow` (action `smoke.channel`, smoke build only) until the
right pane hosts them. `<list>` is a FilterList's name: `InputDetail.bands`,
`OutputDetail.bands` (PEQ) or `OutputDetail.crossover` (XO).

| Name | What |
|---|---|
| `InputDetail` | an input page |
| `InputDetail.header` | its header card |
| `InputDetail.link` | Link 1/2 (a check box) |
| `InputDetail.preamp`, `InputDetail.preampValue` | the preamp slider and its field |
| `InputDetail.clear` | Clear PEQ / Clear 1/2 PEQ |
| `InputDetail.linkQuestion` | the "don't match" question (Keep INn / Keep INm / Cancel) |
| `OutputDetail` | an output page |
| `OutputDetail.title`, `OutputDetail.enable` | its name and the Enabled switch |
| `OutputDetail.core1Question` | the PDM / Core 1 question before an enable |
| `OutputCard` | the output card |
| `OutputCard.route.<input>` | connect input 0 or 1 to this output |
| `OutputCard.routeGain.<input>`, `OutputCard.routeInvert.<input>` | its crosspoint gain field and INV |
| `OutputCard.gain`, `OutputCard.gainValue` | the gain slider and field |
| `OutputCard.delay`, `OutputCard.delayValue` | the delay slider and field |
| `OutputCard.mute`, `OutputCard.limiter` | mute; the limiter cell (hidden without the limiter feature) |
| `<list>` | a filter list (`FilterList` / `FilterList.xo` when nothing names it) |
| `<list>.rows`, `<list>.tabs`, `<list>.tab.peq`, `<list>.tab.xo` | its rows' flickable; the PEQ / XO tabs |
| `<list>.enableAll`, `<list>.bypassAll`, `<list>.clearAll` | the footer buttons |
| `<list>.bypassQuestion`, `<list>.clearQuestion` | their questions |
| `<list>.band.<n>` | band row n (from 0); properties `typePicker` (the type menu) and `linkwitzPopover` |
| `<list>.band.<n>.bypass`, `.number`, `.type`, `.linkwitz` | the bypass disc, the number, the PEQ type button, the Linkwitz button |
| `<list>.band.<n>.freq`, `.gain`, `.q` | the fields (a ChannelValueField; its text field is `<name>.field`) |
| `<list>.band.<n>.family`, `.direction`, `.slope` | crossover rows' pickers |
| `Linkwitz.f0`, `Linkwitz.q0`, `Linkwitz.fp`, `Linkwitz.qp` | the Linkwitz panel's fields (search inside `linkwitzPopover`) |
| `Linkwitz.boost`, `Linkwitz.state`, `Linkwitz.revert`, `Linkwitz.apply` | its DC boost, applied state and buttons |
| `SmokeChannelWindow` | the smoke host; `page` ("input"/"output"), `input`, `output` |

Screenshots: `window-input.png`, `window-input-link.png`, `window-output.png`,
`window-output-xo.png`, `window-linkwitz.png`, `window-output-pdm.png`,
`window-output-readonly.png`.
