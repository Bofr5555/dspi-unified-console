# Object names: UB2 (one remote home for Simple mode and the locked user)

Sections: `ub2-home` (order 993), `smoke/SmokeUb2.cpp`.

| Name | What |
|---|---|
| `HomeSection` | Overview › Home: the remote (Simple's first page, the locked user's) |
| `HomeSection.empty` | its "No Device Connected" state |
| `HomeSection.scroll` | its Flickable |
| `HomeSection.name`, `HomeSection.connection` | the device's name and how it is reached |
| `HomeSection.volumeCard`, `HomeSection.volume`, `HomeSection.readout` | the User volume (a CustomSlider) and its value (`shownName`) |
| `HomeSection.mute` | Mute (an OptionChip, checked while muted) |
| `HomeSection.source`, `HomeSection.source.<i>` | the source as segments (not under the lock) |
| `HomeSection.sourceText` | the source in words, under the lock (`shownName`) |
| `HomeSection.presets`, `HomeSection.preset.<slot>` | the Recall list: a stored slot or the active one (slot from 0; checked: it plays) |
| `HomeSection.health` | the hub installation's one health line (only on one) |
| `ModernShell.lockRead`, `ModernShell.lockRead.action` | the notice over a page the installer's lock shows read-only, and its Go to Home |
| `ModeWindow.about` | what Simple or Advanced is, in the Mode window |
