# Object names: MCP4 (AI agents)

| Name | What |
|---|---|
| `AiStatusIcon` | Classic's icon beside the device button, while agents are on |
| `AiAgentsPage` | Settings > AI Agents; `enableDialog` the question before turning it on |
| `AiAgents.allow`, `AiAgents.statusLine`, `AiAgents.addresses`, `AiAgents.port` | the switch, the state, where it listens, the port |
| `AiAgents.error`, `AiAgents.errorText`, `AiAgents.retry` | why the last start failed, Try Again |
| `AiAgents.limitsOffer`, `AiAgents.setLimits`, `AiAgents.limits`, `AiAgents.limitsSettings` | the limits profile, offered |
| `AiAgents.confirm`(`.combo`) | when agents ask |
| `AiAgents.tabs`, `AiAgents.tab.<desktop, code, other>`, `AiAgents.config`, `AiAgents.url` (`.text`, `.copy`), `AiAgents.otherComputer` | connecting an agent |
| `AiAgents.client.<n>`, `.remove`, `AiAgents.newName`, `AiAgents.add`, `AiAgents.newClient`, `AiAgents.token`, `AiAgents.httpCommand`, `AiAgents.tokenDone` | the clients |
| `AiAgentsPanel`, `AiAgents.status`, `AiAgents.pause`, `AiAgents.stop`, `AiAgents.settings`, `AiAgents.ask.<n>`(`.allow`, `.deny`) | Device > AI Agents... (its window `AiAgentsWindow`) |
| `<prefix>.list`, `.row.<n>`(`.title`, `.detail`, `.outcome`, `.undo`), `.reads`, `.undoAll`, `.export` | the activity; `<prefix>` `AiAgents.log` on the page, `AiActivity` in the panel |
| `AiHost`, `AiPairingSheet`(`.name`, `.code`, `.allow`, `.deny`), `AiConfirmSheet`(`.who`, `.risk.<n>`, `.left`, `.allow`, `.deny`) | what agents ask, in the main window; `armed` once the buttons take answers |
| `HistoryWindow.agent.<n>` | a change's "AI" tag in the History |

Section `mcp4-ai` (982, main).
