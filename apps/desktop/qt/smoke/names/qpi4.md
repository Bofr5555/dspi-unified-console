# WP QPI4: wave-4 integration and fixups

No new section. Checks named `qpi4: …` sit in the sections they extend: `qp0-core` (a job the worker ran is destroyed
on the GUI thread; the coalescing check holds the worker while the writes queue), `qp2-channel` (Identify on the
output card's menu). `qp19-slider-keys` now asserts Up, Down and an accessibility increase (one commit each), and the
pseudo pass prints each clipped and elided text with its window.

| Name | What |
|---|---|
| `OutputCard.menu` | the output card's menu: a right-click on the card away from its controls |
| `OutputCard.menu.identify` | Identify: the channel-ID blips on this output (`SiggenController::playIdent`) |
