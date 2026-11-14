# Object names: AUSB2 (USB-direct units' lock error and Installer settings)

Checks named `ausb2: …` sit in section `lkui1-system` (`smoke/SmokeAlinkSystem.cpp`), on a hub started with
`uconsole --demo hub --alink --with-usb-lock-error` (Rear L/R's host out of range).

| Name | What |
|---|---|
| `SystemWindow.lockError.<unit id>` | under a USB-direct unit's name: why it can't lock, or that its firmware cannot play unaligned |
| `SystemWindow.usbEditor` | the USB-direct part of Clock and audio links |
| `SystemWindow.usbMode.<unit id>` | a USB-direct unit's mode (its own, off, rate lock, phase lock) |
| `SystemWindow.onLockError.<unit id>` | what it does when it can't lock (mute, play unaligned) |
| `SystemWindow.keepUsbAligned` | "Keep USB-direct units aligned" |
