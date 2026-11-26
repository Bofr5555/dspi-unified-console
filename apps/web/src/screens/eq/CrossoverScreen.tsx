// -- WP WEBM2 -- The Equaliser's Crossover tab (MODLAY 2): every output's crossover curve on one
// graph, and one table of their high and low passes (`PassFields`). The curves are the core's
// (`curves`), asked for when the crossovers change.
//
// -- WP WEB3 -- (DL8 on the web, REVD W7) The graph unboxed on the panel tone, its axis labels in
// HTML inside the plot (6 px in, never stretched or cut); the table as §9's (each output's pill),
// with Qt's legend under it.
//
// -- WP WEB6 -- (REVE §3.1) Qt's EqualiserCrossover: the tab sits in the Equaliser's frame (the
// channel strip, the channel list and the inspector stay: EqScreen with `eqTab` "crossover",
// XoverCentre.tsx its centre). The graph is Qt's ±25 dB with every enabled output's response (its PEQ and
// crossover, at its gain), the chosen one drawn over the rest; the table lists every output (a
// disabled one dimmed), a row's click chooses it, each pass Qt's short choice ("LR 24 dB") and its
// 64 px frequency with the unit after; under the legend, the chosen output's four crossover bands
// with their Bypass.

import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { EqScreen } from "./EqScreen";
import { EmptyState } from "../../ui/design"; // -- WP WEB --
import "./eq.css"; // -- WP WEB3 --

export function CrossoverScreen(props: ScreenProps) {
  const st = useSessionState(props.session);
  const s = st.snapshot?.state;
  if (!s) return <EmptyState icon="chart-line" title="Reading the outputs…" text="This DSPi's outputs are not known yet." />; // -- WP WEB --
  if (!Array.isArray(s.xover) || !s.xover.length) return <EmptyState icon="chart-line" title="No crossovers" text="This firmware has no crossovers." />;
  // -- WP WEB6 -- in the Equaliser's frame, as Qt's
  return <EqScreen {...props} eqTab="crossover" />;
}
