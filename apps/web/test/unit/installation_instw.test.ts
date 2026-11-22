// WP INSTW unit tests: the Installation tab's form model (src/screens/system/installation/model.ts).
// The form keeps the draft and the edits; the core edits, validates, diffs and saves. Here a fake
// core stands in for the hub's answers, and the real core (crates/dspi-wasm, when
// $DSPI_WASM_DIR has it) for the pure calls: edits, the hub's validator and the diff.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CoreCallError } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { SystemModel } from "../../src/screens/system/model";
import {
  InstallationForm,
  cleanText,
  generalIssues,
  issuePlace,
  issuesAt,
  joinEdit,
  roleOf,
  upstreamChoices,
  type InstallationCall,
} from "../../src/screens/system/installation/model";
import type { InstallationDraft, InstallationEdit, InstallationView } from "../../src/screens/system/installation/protocol";

function site(): SystemModel {
  return {
    rooms: [{ id: "main", name: "Main room" }],
    zones: [],
    groups: [],
    sources: [],
    senders: [{ id: "tx", name: "Sender", serial: "S-TX" }],
    fanOuts: [{ id: "fan", name: "Fan-out", serial: "S-FAN", upstream: "tx" }],
    units: [
      { id: "A", name: "Left", serial: "S-A", room: "main", upstream: "tx" },
      { id: "B", name: "Right", serial: "S-B", room: "main", upstream: "fan", repeater: true },
      { id: "R", name: "Rear", serial: "S-R", room: "main", upstream: "", transport: "usb_direct" },
    ],
    scenes: [],
  };
}

test("INSTW: names are shown without bidi overrides and other format characters", () => {
  assert.equal(cleanText("Removes unit ‮evil‬"), "Removes unit evil");
  assert.equal(cleanText("a​b⁦c⁩﻿\u0007d"), "abcd");
  assert.equal(cleanText("Vardagsrum åäö – ok"), "Vardagsrum åäö – ok");
});

test("INSTW: a unit's role and what may feed it", () => {
  const m = site();
  assert.deepEqual(m.units.map(roleOf), ["receiver", "repeater", "usb_direct"]);
  assert.equal(roleOf({ ...m.units[0]!, transport: "spdif" }), "spdif_follower");
  assert.deepEqual(
    upstreamChoices(m, "receiver", "A").map((c) => c.id),
    ["tx", "fan", "B"],
  );
  assert.deepEqual(
    upstreamChoices(m, "repeater", "B").map((c) => c.id),
    ["tx", "fan"],
    "never itself",
  );
  assert.deepEqual(upstreamChoices(m, "usb_direct"), []);
  m.units[0]!.spdifSource = true;
  assert.deepEqual(
    upstreamChoices(m, "spdif_follower", "B").map((c) => c.id),
    ["A"],
  );
});

test("INSTW: joining makes one addUnit edit; a USB-direct unit has no upstream", () => {
  assert.deepEqual(joinEdit("S-C", { name: " Patio ", room: "main", role: "receiver", upstream: "fan" }), {
    op: "addUnit",
    serial: "S-C",
    name: "Patio",
    room: "main",
    role: "receiver",
    upstream: "fan",
  });
  assert.deepEqual(joinEdit("S-D", { name: "", room: "", role: "usb_direct", upstream: "tx" }), {
    op: "addUnit",
    serial: "S-D",
    room: null,
    role: "usb_direct",
    upstream: null,
  });
});

test("INSTW: an issue is shown at its field, the element, or at the top", () => {
  const up = { subject: "B", message: 'unit B: upstream "fan" is not a sender, fan-out or repeater' };
  const room = { subject: "A", message: 'unit A: no room "gone"' };
  const zone = { subject: "z", message: 'zone z: no room "gone"' };
  const own = { subject: "A", message: "unit A: at most 32 speakers" };
  const top = { subject: "", message: "rateHz: 44100, 48000 or 96000" };
  assert.deepEqual(issuePlace(up), { subject: "B", field: "upstream" });
  assert.deepEqual(issuePlace(room), { subject: "A", field: "room" });
  assert.deepEqual(issuePlace(zone), { subject: "z", field: "rooms" });
  assert.deepEqual(issuePlace(own), { subject: "A", field: "" });
  assert.deepEqual(issuePlace(top), { subject: "", field: "" });
  assert.deepEqual(issuesAt([up, room, own], "A", ""), [own]);
  assert.deepEqual(issuesAt([up, room, own], "B", "upstream"), [up]);
  const gone = { subject: "X", message: "unit X: something" };
  assert.deepEqual(generalIssues([up, top, gone], site()), [top, gone], "the installation's own, and an element no longer there");
});

/** A fake core: edits that add rooms, the hub's revision, and a save that may meet a newer one. */
class FakeHub {
  revision = 4;
  model = site();
  saves: InstallationDraft[] = [];
  /** An issue the validator gives while the draft has a room called "Bad". */
  readonly call: InstallationCall = async (method, params) => {
    const p = params as { draft: InstallationDraft; edits: InstallationEdit[] };
    switch (method) {
      case "installationView": {
        const m = structuredClone(this.model);
        const view: InstallationView = {
          revision: this.revision,
          role: "admin",
          mayEdit: true,
          draft: { revision: this.revision, base: m, model: structuredClone(m) },
          unseen: [{ serial: "S-C", name: "New one", online: true, fw: "1.1.6", outputs: 8 }],
          inventory: true,
        };
        return view as never;
      }
      case "installationDraftEdit": {
        const model = structuredClone(p.draft.model);
        for (const e of p.edits) {
          if (e.op !== "addRoom") throw new CoreCallError("invalid", "edit 1: not in this fake");
          if (model.rooms.some((r) => r.name === e.name)) throw new CoreCallError("invalid", `edit 1: id "${e.name}" is taken`);
          model.rooms.push({ id: e.name.toLowerCase(), name: e.name });
        }
        return { ...p.draft, model } as never;
      }
      case "installationValidate":
        return (p.draft.model.rooms.some((r) => r.name === "Bad") ? [{ subject: "", message: "rooms: bad" }] : []) as never;
      case "installationDiff":
        return p.draft.model.rooms.filter((r) => !p.draft.base.rooms.some((b) => b.id === r.id)).map((r) => `Adds room ${r.name}`) as never;
      case "installationSave": {
        if (p.draft.revision !== this.revision) throw new CoreCallError("conflict", "the installation changed on the hub");
        this.saves.push(p.draft);
        this.model = p.draft.model;
        this.revision++;
        return { revision: this.revision, warnings: ["unit ‮R‬: no audio link"] } as never;
      }
    }
    throw new Error(`unexpected ${method}`);
  };
}

test("INSTW: edits are judged, a refused edit changes nothing, and Save is offered only for what the hub takes", async () => {
  const hub = new FakeHub();
  const form = new InstallationForm(hub.call);
  await form.load();
  assert.equal(form.state.view?.revision, 4);
  assert.equal(form.dirty, false);
  assert.equal(form.savable, false);
  assert.ok(await form.edit({ op: "addRoom", name: "Patio" }));
  assert.deepEqual(form.state.changes, ["Adds room Patio"]);
  assert.equal(form.savable, true);
  // Refused: the draft and the edits are as they were, with the core's reason.
  const before = form.state.draft;
  assert.equal(await form.edit({ op: "addRoom", name: "Patio" }), false);
  assert.equal(form.state.draft, before);
  assert.equal(form.state.edits.length, 1);
  assert.match(form.state.error, /^Edit 1: id "Patio" is taken/);
  // The hub would refuse it: not offered.
  assert.ok(await form.edit({ op: "addRoom", name: "Bad" }));
  assert.equal(form.state.issues.length, 1);
  assert.equal(form.savable, false);
  assert.equal(await form.save(), "failed");
  assert.equal(hub.saves.length, 0);
  await form.revert();
  assert.equal(form.dirty, false);
  assert.deepEqual(form.state.issues, []);
});

test("INSTW: a save is read back, with the hub's warnings cleaned", async () => {
  const hub = new FakeHub();
  const form = new InstallationForm(hub.call);
  await form.load();
  await form.edit({ op: "addRoom", name: "Patio" });
  assert.equal(await form.save(), "ok");
  assert.equal(hub.saves.length, 1);
  assert.equal(hub.saves[0]!.revision, 4, "saved at the revision it was read at");
  assert.equal(form.state.view?.revision, 5, "read again");
  assert.ok(form.state.draft!.base.rooms.some((r) => r.name === "Patio"));
  assert.equal(form.dirty, false);
  assert.deepEqual(form.state.saved, { revision: 5, warnings: ["unit R: no audio link"] });
});

test("INSTW: on conflict the draft is rebuilt from the fresh view, never saved over the other change", async () => {
  const hub = new FakeHub();
  const form = new InstallationForm(hub.call);
  await form.load();
  await form.edit({ op: "addRoom", name: "Patio" });
  // Someone else saves meanwhile.
  hub.model = { ...site(), rooms: [...site().rooms, { id: "bar", name: "Bar" }] };
  hub.revision = 9;
  assert.equal(await form.save(), "conflict");
  assert.equal(hub.saves.length, 0);
  const st = form.state;
  assert.equal(st.view?.revision, 9);
  assert.equal(st.draft?.revision, 9, "the fresh view's draft, not the old one with a new revision");
  assert.deepEqual(
    st.draft!.model.rooms.map((r) => r.id),
    ["main", "bar"],
    "the other change is there, the lost edit is not",
  );
  assert.equal(form.dirty, false);
  assert.deepEqual(st.lost, [{ op: "addRoom", name: "Patio" }]);
  // Made again on the fresh draft, judged again, then saved at 9.
  assert.ok(await form.remake());
  assert.equal(form.state.lost, null);
  assert.deepEqual(
    form.state.draft!.model.rooms.map((r) => r.id),
    ["main", "bar", "patio"],
  );
  assert.equal(await form.save(), "ok");
  assert.equal(hub.saves[0]!.revision, 9);
  assert.deepEqual(
    hub.model.rooms.map((r) => r.id),
    ["main", "bar", "patio"],
  );
});

test("INSTW: a failed view is said, and nothing is offered", async () => {
  const form = new InstallationForm(async () => {
    throw new CoreCallError("not_open", "no hub session");
  });
  await form.load();
  assert.equal(form.state.view, null);
  assert.equal(form.state.error, "No hub session");
  assert.equal(await form.edit({ op: "addRoom", name: "X" }), false);
});

// ---------------------------------------------------------------- the real core

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

test("INSTW: the real core edits the draft, says what changes, and names a problem after a removal", { skip }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const call: InstallationCall = (m, p) => core.call(m, p as never) as never;
  const m = site();
  const draft: InstallationDraft = { revision: 3, base: m, model: m };
  const joined = await call("installationDraftEdit", {
    draft,
    edits: [
      { op: "addRoom", name: "Patio" },
      joinEdit("S-C", { name: "Patio ‮unit", room: "Patio", role: "receiver", upstream: "fan" }),
      { op: "addZone", name: "Outside", rooms: ["Patio"] },
    ],
  });
  assert.equal(joined.revision, 3);
  assert.deepEqual(await call("installationValidate", { draft: joined }), []);
  const form = new InstallationForm(async (method, params) => {
    if (method === "installationView") {
      return { revision: 3, role: "admin", mayEdit: true, draft: joined, unseen: [], inventory: true } as never;
    }
    return call(method, params);
  });
  await form.load();
  // The diff names the new elements; the unit's bidi override is not shown.
  assert.ok(form.state.changes.length >= 3, JSON.stringify(form.state.changes));
  assert.ok(
    form.state.changes.every((c) => !/[‪-‮⁦-⁩]/.test(c)),
    JSON.stringify(form.state.changes),
  );
  // Removing the fan-out leaves B and the new unit fed by nothing: the hub's validator names one (it stops at the first).
  assert.ok(await form.edit({ op: "removeElement", id: "fan" }));
  assert.equal(form.state.issues.length, 1);
  const place = issuePlace(form.state.issues[0]!);
  assert.equal(place.field, "upstream", form.state.issues[0]!.message);
  const fedByFan = joined.model.units.filter((u) => u.upstream === "fan").map((u) => u.id);
  assert.deepEqual(fedByFan, ["B", "Patio \u202Eunit"], "an id is made from the name as it is");
  assert.ok(fedByFan.includes(place.subject), place.subject);
  assert.equal(form.savable, false);
  // A refused edit names it.
  assert.equal(await form.edit({ op: "removeElement", id: "ghost" }), false);
  assert.match(form.state.error, /^Edit 1/);
});
