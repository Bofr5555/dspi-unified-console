# WP P1QT2: Installer page, lock, tokens, AutoEQ preview, notices, groups

Sections (`smoke/SmokeInstaller.cpp`): `p1qt2-installer` (order 158),
`p1qt2-groups` (159), `p1qt2-autoeq` (160). Screenshots, each dark and light,
in `<out>/p1qt2/`: `installer-no-pin`, `installer-limits`, `installer-pin-set`,
`installer-locked`, `lock-banner`, `unlock-dialog`, `autoeq-preview`.

| Name | What |
|---|---|
| `InstallerPage` | Settings > Installer (after Advanced and Networking; OD-P1e) |
| `Installer.unlockGroup`, `.scope`, `.unlockPin`, `.unlock`, `.unlockError` | the page while locked: where the lock is, the PIN field, Unlock, the refusal |
| `Installer.limits` | the Limits group; `Installer.standingNotice` (an unreadable profile), `Installer.unprotected` and `Installer.protectAll` (PRO1 S2), `Installer.clearLimits` |
| `Installer.masterVolumeMax`, `Installer.maxPeqBoost` | a `LimitField`: `<name>.set` (the box), `<name>.value` (the spin box) |
| `Installer.output.<n>` | output n's group; its fields `.maxGain`, `.limiter`, `.highpass`, `.slope`, `.delayMin`, `.delayMax` (each a `LimitField`) |
| `Installer.noteGroup`, `Installer.note`, `Installer.contact` | the installer's note and contact |
| `Installer.pinGroup`, `.newPin`, `.repeatPin`, `.setPin` | setting a PIN |
| `Installer.recovery`, `.recoveryCode`, `.recoveryDone` | the recovery code, shown once |
| `Installer.recovered`, `Installer.covers`, `Installer.lockNow`, `Installer.clearPin`, `Installer.removeQuestion`, `Installer.pinError` | the PIN's other rows |
| `Installer.hubGroup`, `Installer.hubLockNow` | a lock the hub holds |
| `LockBanner`, `LockBanner.unlock` | the locked-user banner and its Unlock… |
| `UnlockDialog`, `.scope`, `.pin`, `.error`, `.cancel`, `.unlock` | the one unlock prompt (inside `LockBanner`) |
| `LimiterSettings.unprotected` | PRO1 S2's warning row in the limiter's settings |
| `AutoEqWindow.previewArea`, `.pair`, `.preview`, `.previewText` | the channel pair, the preview graph and its lines |
