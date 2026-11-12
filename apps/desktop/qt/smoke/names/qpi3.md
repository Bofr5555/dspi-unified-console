# WP QPI3: wave-3 integration and fixups

No new section. Checks named `qpi3: …` sit in the sections they extend: `qp0-core` (a job never run is dropped on
the GUI thread), `qp1-sidebar` (Identify through the generator's controller), `qp9-spectrum` (the options popover
closed), `qp12-surfaces` (V26 read-only Remove), `qp16-firmware` (the wizard's install pair), `qp16-access` (the status
area's offer), `qp16-readonly` (the analyser on read-only), `qp17-onboarding` (the installer put back). `qp17-onboarding`
now runs at order 162, after `qp16-access`.

| Name | What |
|---|---|
| `DeviceStatus.access` | the device status's key button: shows the device-access help again while a unit cannot be opened |
| `SpectrumWindow.unavailable` | (QP9's) its caption names the read-only case once the core refused the analyser |
