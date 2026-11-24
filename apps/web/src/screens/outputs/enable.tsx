// -- WP WEB3 -- An output's switch (Delay & gain's ON column, the Overview's): off at once; on as the
// Console switches over, asking first when the PDM output and the EQ worker's outputs exclude each
// other (`enableConflict`), then the others off and this one on in one step. The matrix's column
// switch does the same (screens/matrix).

import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { enableConflict, eqWorkerRange, pdmOutput } from "./model";

export async function toggleOutput(session: Session, ask: Ask, o: number): Promise<void> {
  const live = session.state.snapshot?.state;
  if (!live) return;
  if (live.outputEnabled[o]) return void (await session.write("out.enable", { bool: false }, [o]));
  const conflict = enableConflict(live, o);
  if (!conflict) return void (await session.write("out.enable", { bool: true }, [o]));
  const [a, b] = eqWorkerRange(live);
  const pdm = conflict === "pdm";
  const answer = await ask({
    title: "Warning",
    body: <p>{pdm ? `Outputs ${a + 1}-${b + 1} will be disabled. Are you sure?` : "The PDM output will be disabled. Are you sure?"}</p>,
    buttons: [
      { label: pdm ? "Enable PDM" : "Disable PDM", value: "go" },
      { label: "Cancel", value: "cancel" },
    ],
  });
  if (answer !== "go") return;
  const off = pdm ? Array.from({ length: b - a + 1 }, (_, k) => a + k) : [pdmOutput(live)];
  await session.writeAll([...off.map((x) => ({ path: "out.enable", value: { bool: false }, indices: [x] })), { path: "out.enable", value: { bool: true }, indices: [o] }]);
}
