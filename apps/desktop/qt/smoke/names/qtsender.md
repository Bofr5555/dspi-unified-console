# Object names: QTSENDER (the alink sender: one banner, the DSP rows greyed)

Section: `qtsender` (order 216, `smoke/SmokeQtSender.cpp`).

| Name | What |
|---|---|
| `SenderBanner` | the main window's banner while the alink sender is open |
| `ToolWindow.senderBanner` | a tool window's banner while its action does nothing on the sender (the content is greyed) |

Properties the section reads: `ToolWindow.actionId` (set by Shell.openWindow), `ToolWindow.senderLocked`.
