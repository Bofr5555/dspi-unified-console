# Object names: LAY8 (Modern's AI button and popover)

Smoke section: `lay8-ai` (order 983, `smoke/SmokeLay8.cpp`), over a fake controller (`ai` of the button; the popover follows it). Its pictures
are `gallery/lay8-*`. The real server's checks (`lay8: ...`) are in `mcp4-ai`.

- `TopBar.ai` (an `AiButton`: `face` is `off`, `waiting`, `paused`, `connected` or `attention`; `popover`), `AiButton.mark` (`ring`).
- `AiPopover` (`enableDialog` the question before turning agents on), `.allow`, `.status`, `.off`, `.addresses`, `.asks`, `.error`, `.retry`,
  `.connect`, `.pause`, `.activity`, `.settings`.
- `AiPopover.agent.<n>`(`.name`, `.disconnect`), `AiPopover.change.<n>`(`.title`, `.who`, `.undo`).
