// WP FEATUI (AIVIS's web part): the strip that says an AI agent works on this hub's devices, on
// every screen, first in the strip as the desktop app's (AiWorkBanner): who, through which
// console, on which device, doing what. It names; it changes nothing (the agent's console has
// Pause and Stop: -- WP UAG -- it says so). What the person here changes reaches the agent as
// "elsewhere" (AIVIS).

import type { Session } from "./session";
import { useSessionState } from "./components";
import { DETECTING_HERE, cleanPresence, detectingSpeakers, nameFrom, presenceNames, presenceText, stopHint, usePresence } from "./presence";
import { splitGrant } from "./follow"; // -- WP ODEC --

export function AgentStrip(props: { session: Session }) {
  const rows = usePresence(props.session);
  const st = useSessionState(props.session);
  if (!rows.length) return null;
  const mine = st.snapshot?.device.serial ?? "";
  // -- WP UAG -- (UXP A7) another device of the hub by its inventory's name, as the hub pages say it
  const name = nameFrom(presenceNames(props.session));
  const by = rows.find((p) => p.by)?.by ?? "";
  return (
    <div class="banner banner-agent" role="status" aria-live="polite" aria-label="AI agent working">
      <span class="agent-mark" aria-hidden="true">
        AI
      </span>
      <span>
        {rows.map((p, i) => {
          // -- WP ODEC -- the test signals the user allowed for a while, apart from the work
          const { work, grant } = splitGrant(cleanPresence(p.what));
          return (
            <span key={`${p.by}-${i}`} class="agent-row">
              {presenceText({ ...p, what: work }, name, mine)}
              {grant ? <span class="agent-grant"> {grant}</span> : null}
              {/* -- WP ADETW -- */}
              {p.serial && p.serial === mine && detectingSpeakers(work) ? <span class="agent-detect"> {DETECTING_HERE}</span> : null}
            </span>
          );
        })}
        <span class="agent-stop">{stopHint(by)}</span>
      </span>
    </div>
  );
}
