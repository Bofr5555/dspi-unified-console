// WP FEATUI (AIVIS's web part): the strip that says an AI agent works on this hub's devices, on
// every screen, first in the strip as the desktop app's (AiWorkBanner): who, through which
// console, on which device, doing what. It names; it changes nothing (the agent's console has
// Pause and Stop). What the person here changes reaches the agent as "elsewhere" (AIVIS).

import type { Session } from "./session";
import { useSessionState } from "./components";
import { presenceText, usePresence } from "./presence";

export function AgentStrip(props: { session: Session }) {
  const rows = usePresence(props.session);
  const st = useSessionState(props.session);
  if (!rows.length) return null;
  const mine = st.snapshot?.device.serial ?? "";
  // Another device of the hub by its serial: the page knows only its own one's name
  const name = (serial: string) => serial;
  return (
    <div class="banner banner-agent" role="status" aria-live="polite" aria-label="AI agent working">
      <span class="agent-mark" aria-hidden="true">
        AI
      </span>
      <span>
        {rows.map((p, i) => (
          <span key={`${p.by}-${i}`} class="agent-row">
            {presenceText(p, name, mine)}
          </span>
        ))}
      </span>
    </div>
  );
}
