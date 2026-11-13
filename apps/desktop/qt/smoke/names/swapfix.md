# WP SWAPFIX: hardware-pass findings fixed before the swap

Section `swapfix` (order 152, `smoke/SmokeSwapfix.cpp`): the unsaved-changes question lists an output's delay once
(F3); after a refresh, and after a DSPi Link session to the fake hub is lost and comes back with a change made at the
hub meanwhile, the device switch and Quit still ask (F2). Checks named `swapfix-f4: …` sit in `qp17-onboarding`: the
settings are a file, and Save Diagnostics puts it in the bundle. `smoke/SmokeHw.cpp`'s preflight scans before it counts
the USB list (F6).

No new object names: it uses `FilesHost.unsavedQuestion` (QP15).
