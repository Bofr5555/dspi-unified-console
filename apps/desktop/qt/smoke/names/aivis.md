# Object names: AIVIS (the screen shows what an AI agent does)

Smoke section: `aivis-agent` (order 984, `smoke/SmokeAivis.cpp`), a fake agent over HTTP against the real server, in both layouts. Its pictures are
`aivis-*.png`.

- `AiWorkBanner` (a blocking banner, first in both layouts), `.mark` (`working`: it turns), `.agent`, `.text`, `.show` (the device is open but not
  shown), `.stopJob` (a measurement plays), `.pause`, `.stop`.
- `AiButton.working`, `AiButton.workingText` (Modern), `AiStatusIcon.working` (Classic), `AiPopover.stop`.
- `NoticeBanner.action` (a notice's own button: Undo).
- `<control>.touched` (an `AiTouched`: `times` how often it lit up, `lit` now): `Dock.volume`, `GlobalControls.volume`, `ChannelStrip`,
  `OutputCard.<n>`, `Sidebar.channel.<n>`, `BandRow.<n>`, `Matrix.cell.<in>.<out>`, the effects' `<window>.header`, `NavRail.<section>`;
  `DeviceRow.touched.<device id>` (a device's tab).
- `Tray.ai`, `Tray.aiPause` (the tray's menu).
