# Object names: UB5 (one question across devices, the agent's unsaved work, UXP B5)

Smoke section: `ub5-agent` (order 999), `smoke/SmokeUb5.cpp`. Pictures `ub5/<n>-<what>-dark.png` and `-light.png`.

- `AiWorkBanner.save`: beside the strip's last line while an agent's changes are not saved on the device
  ("Changed 12 settings, not saved yet"): "Save to Device" (one device) or "Save All…" (several), which asks the save
  question. `AiController.unsaved` is `{ agent, count, devices, deviceId, text, save }`, `{}` for none.
- `AiConfirmSheet`'s `question`: the ids of the server's sealed group it shows ("4,5,6"); it never grows while the
  sheet is up.
