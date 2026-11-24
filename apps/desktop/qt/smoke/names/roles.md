# Object names: ROLES (speaker roles from the person's words, Qt)

Smoke section: `roles-agent` (order 988), `smoke/SmokeRoles.cpp`.

- `AiConfirmSheet.roles`: the question's table of speaker roles (a Grid: `AiConfirmSheet.roles.<i>`, the first three
  the header Output, Speaker, Role, then three cells per row); `AiConfirmSheet.rolesAfter`: the server's words after
  the table (outputs not named, the person's words quoted, not checked).
- `AiAgents.roles` (`.switch`): Settings › AI Agents › Agents may set speaker roles from my words; the page's
  `rolesDialog` property is its question (`AiAgents.rolesQuestion`).
- Speakers pills (`Speakers.pill.<output>`) gain the kinds `stated` ("from your words"), `statedTweeter`
  ("tweeter-like") and `notNamed` ("unknown").
