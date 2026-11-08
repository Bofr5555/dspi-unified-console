// The screens of one session (WP WB6a): each WB6 slice adds its screen here.
// They live inside the session's view, never on the page: a fleet page shows
// several sessions at once, each with its own screen chosen. The tab row shows
// only once there is more than one screen.

import type { FunctionComponent } from "preact";
import { useState } from "preact/hooks";
import { MainScreen } from "../screens/main/MainScreen";
import type { Session } from "./session";

export interface ScreenDef {
  id: string;
  label: string;
  View: FunctionComponent<{ session: Session }>;
}

export const SCREENS: ScreenDef[] = [{ id: "main", label: "Main", View: MainScreen }];

export function Screens(props: { session: Session }) {
  const [current, setCurrent] = useState(SCREENS[0]!.id);
  const screen = SCREENS.find((x) => x.id === current) ?? SCREENS[0]!;
  return (
    <>
      {SCREENS.length > 1 ? (
        <div class="screen-tabs" role="tablist" aria-label="Screens">
          {SCREENS.map((x) => (
            <button type="button" role="tab" key={x.id} aria-selected={x === screen} onClick={() => setCurrent(x.id)}>
              {x.label}
            </button>
          ))}
        </div>
      ) : null}
      <screen.View session={props.session} />
    </>
  );
}
