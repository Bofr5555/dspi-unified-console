# WP USR1: two user-reported bugs in 0.14.0 (Windows, Classic)

No new object names. Sections `usr1-fields` and `usr1-preset` (order 37, main run, smoke/SmokeUsr1.cpp):

- `usr1-fields`: a number typed key by key into `OutputCard.gainValue.field`, `OutputCard.routeGain.0.field`,
  `OutputCard.delayValue.field`, `OutputDetail.bands.band.0.{freq,gain,q}.field`, `Matrix.cell.0.0.gain`,
  `Matrix.gain.0` and `Matrix.delay.0`, as "-3.5", "-3,5", "−3,5" (U+2212) and "-3.5 dB", left with Enter, Enter
  then a click on another field, Tab, or a click on another field (kept), or Escape (dropped), under en_US, sv_SE
  and de_DE. May run up to 300 s (`allowSectionTime`).
- `usr1-preset`: `GlobalControls.preset` (Classic) and `TopBar.preset` (Modern) show the active slot's title after
  a load from the box (Down), an edit (the "*"), Save, a copy into an empty slot, Rename, a refresh, a load from
  elsewhere and a reconnect; `GlobalControls.source` shows the chosen source after it is renamed.
