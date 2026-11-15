// WP WB6e: the scene bar across the top of a session: the device's ten preset
// slots as numbered recall buttons, the active one and its unsaved state
// marked, and Recall and Store as explicit verbs: a number recalls its slot
// while Recall is on; Store arms the bar so the next number stores the live
// settings there instead (asked first), and Recall or Escape disarms it. Every action is the core's (see
// model.ts). Off while read only, and one action at a time.

import { useState } from "preact/hooks";
import type { Session } from "../../ui/session";
import { useSessionState } from "../../ui/components";
import type { Ask, Unsaved } from "../../ui/screens";
import { activeLine, recall, slotLabel, slotViews, store } from "./model";

export function SlotBar(props: { session: Session; ask: Ask; unsaved: () => Promise<Unsaved> }) {
  const { session } = props;
  const st = useSessionState(session);
  const [storing, setStoring] = useState(false);
  const [busy, setBusy] = useState(false);
  const s = st.snapshot?.state;
  if (!s || st.status !== "open") return null;
  const ro = session.readOnlyReason;
  const dirty = st.unsaved.length > 0 && !ro;
  const views = slotViews(s, dirty);
  const verbs = { session, ask: props.ask, unsaved: props.unsaved };

  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } finally {
      setBusy(false);
    }
  };
  const press = (slot: number) =>
    void run(async () => {
      if (storing) {
        setStoring(false);
        await store(verbs, slot);
      } else {
        await recall(verbs, slot);
      }
    });

  const off = !!ro || busy;
  return (
    <div
      class={`sys-scenes slot-bar${storing ? " storing" : ""}`}
      role="group"
      aria-label="Preset slots"
      aria-busy={busy}
      onKeyDown={(e) => {
        if (storing && e.key === "Escape") {
          e.stopPropagation();
          setStoring(false);
        }
      }}
    >
      <div class="sys-scene-row">
        <span class="sys-label">Presets</span>
        <span class="slot-active" aria-label="Active preset">
          {activeLine(views)}
        </span>
        <span class="slot-buttons">
          {views.map((v) => (
            <button
              type="button"
              key={v.slot}
              class={`sys-scene slot${v.active ? " active" : ""}${v.occupied ? "" : " empty"}${v.unsaved ? " unsaved" : ""}`}
              aria-current={v.active ? "true" : undefined}
              aria-label={slotLabel(v, storing)}
              title={ro || slotLabel(v, storing)}
              disabled={off}
              onClick={() => press(v.slot)}
            >
              <span class="sys-scene-n">{v.number}</span>
              <span class="slot-name">{v.name}</span>
              {v.unsaved ? (
                <span class="slot-dot" aria-hidden="true">
                  •
                </span>
              ) : null}
            </button>
          ))}
        </span>
        <span class="slot-verbs" role="group" aria-label="What a slot button does">
          <button
            type="button"
            class={`sys-scene${storing ? "" : " on"}`}
            aria-pressed={!storing}
            disabled={off}
            title={ro || "A slot button recalls that preset"}
            onClick={() => setStoring(false)}
          >
            Recall
          </button>
          <button
            type="button"
            class={`sys-scene${storing ? " on" : ""}`}
            aria-pressed={storing}
            disabled={off}
            title={ro || "The next slot button stores the current settings there"}
            onClick={() => setStoring(!storing)}
          >
            Store
          </button>
        </span>
      </div>
      {storing ? (
        <p class="sys-note" role="status">
          Choose the slot to store the current settings in, or Recall to go back.
        </p>
      ) : null}
    </div>
  );
}
