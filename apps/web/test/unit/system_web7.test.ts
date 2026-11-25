// WP WEB7 unit tests: the System section's pages as Qt's.

import { test } from "node:test";
import assert from "node:assert/strict";
import { SECTIONS } from "../../src/ui/shell/sections";
import { SCREENS } from "../../src/ui/screens";

test("WEB7: the System section's tabs are Qt's Units | Clock and Links | Installation | Events, then the Signal Chain", () => {
  const sys = SECTIONS.find((s) => s.id === "system")!;
  const label = (id: string) => SCREENS.find((x) => x.id === id)?.label ?? id;
  assert.deepEqual(sys.pages.map(label), ["Units", "Clock and Links", "Installation", "Events", "Signal Chain"]);
});
