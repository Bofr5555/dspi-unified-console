// -- WP WEBM4 -- The arrangement's settings (ui/arrangement/arrangement.ts), as the desktop app's
// Settings › Appearance: applied at once, kept in this browser. Shown on Settings' Appearance page
// and, with no device open, from the app menu's Settings….

import { createContext, type ComponentChildren } from "preact";
import { useContext, useState } from "preact/hooks";
import { safeLocalStorage } from "../../fleet/local";
import { Dialog } from "../../screens/main/Dialog";
import {
  CHOICES,
  DEFAULT_ARRANGEMENT,
  inspectorShown,
  setInspectorShown,
  withChoice,
  type Arrangement,
} from "./arrangement";

export interface ArrangementHost {
  arrangement: Arrangement;
  arrange: (a: Arrangement) => void;
}

/** The console's arrangement and its setter (App). Without one, the defaults and nothing to set. */
export const ArrangementContext = createContext<ArrangementHost>({ arrangement: DEFAULT_ARRANGEMENT, arrange: () => undefined });

/** The open devices' tabs, when the arrangement puts them at the rail's top (else null: the top bar has them). */
export const RailTopContext = createContext<ComponentChildren>(null);

export function useArrangement(): ArrangementHost {
  return useContext(ArrangementContext);
}

export function AppearanceSettings() {
  const { arrangement, arrange } = useArrangement();
  const [inspector, setInspector] = useState(() => inspectorShown(safeLocalStorage()));
  return (
    <section class="card settings-card appearance" aria-label="Appearance">
      <h3>Appearance</h3>
      {CHOICES.map((c) => {
        const value = c.key === "inspector" ? inspector : arrangement[c.key];
        // Where the device tabs are matters only with a rail
        const off = c.key === "deviceTabs" && arrangement.navigation !== "rail";
        return (
          <fieldset class="appearance-choice" key={c.key} disabled={off}>
            <legend>{c.label}</legend>
            {c.values.map((v) => (
              <label key={v.value}>
                <input
                  type="radio"
                  name={`appearance-${c.key}`}
                  value={v.value}
                  checked={value === v.value}
                  onChange={() => {
                    if (c.key === "inspector") {
                      setInspectorShown(safeLocalStorage(), v.value as "shown" | "hidden");
                      setInspector(v.value as "shown" | "hidden");
                      return;
                    }
                    const next = withChoice(arrangement, c.key, v.value);
                    if (next) arrange(next);
                  }}
                />
                {v.text}
              </label>
            ))}
          </fieldset>
        );
      })}
      <p class="note">Kept in this browser only. The device tabs move to the rail only where the sections are in a rail.</p>
    </section>
  );
}

/** The same, in a dialog of its own: Settings… while no device is open. */
export function AppearanceDialog(props: { onClose: () => void }) {
  return (
    <Dialog title="Settings" onCancel={props.onClose} buttons={[{ label: "Done", onClick: props.onClose }]}>
      <AppearanceSettings />
    </Dialog>
  );
}
