// The screens of one session (WP WB6a): each WB6 slice adds its screen here.
// They live inside the session's view, never on the page: a fleet page shows
// several sessions at once, each with its own screen chosen. The tab row shows
// only once there is more than one screen.
//
// -- WP WB6b -- (c6p WB6a-4) What belongs to the session rather than to one
// screen lives here, so switching screens keeps it: the modal questions, the
// Console's unsaved-changes prompt (M42) and the guard Disconnect asks.

import type { ComponentChildren, FunctionComponent } from "preact";
import { useCallback, useEffect, useState } from "preact/hooks";
import { EqScreen } from "../screens/eq/EqScreen";
import { Dialog } from "../screens/main/Dialog";
import { MainScreen } from "../screens/main/MainScreen";
import { changeSummary } from "../screens/main/model";
import type { Session } from "./session";

/** A question in a modal dialog; the answer is the chosen button's value, or "cancel". */
export interface Question {
  title: string;
  body: ComponentChildren;
  buttons: { label: string; value: string; danger?: boolean }[];
}
export type Ask = (q: Question) => Promise<string>;

/** What the user chose about unsaved changes before an action. */
export type Unsaved = "clean" | "save" | "discard" | "cancel";

/** What every screen gets from its session's view. */
export interface ScreenProps {
  session: Session;
  ask: Ask;
  /** The Console's unsaved-changes prompt (M42), when the live state is not the active preset's. */
  unsaved: () => Promise<Unsaved>;
}

export interface ScreenDef {
  id: string;
  label: string;
  View: FunctionComponent<ScreenProps>;
}

export const SCREENS: ScreenDef[] = [
  { id: "main", label: "Main", View: MainScreen },
  // -- WP WB6b --
  { id: "eq", label: "EQ", View: EqScreen },
];

export function Screens(props: { session: Session }) {
  const session = props.session;
  const [current, setCurrent] = useState(SCREENS[0]!.id);
  const screen = SCREENS.find((x) => x.id === current) ?? SCREENS[0]!;
  const [question, setQuestion] = useState<(Question & { answer: (v: string) => void }) | null>(null);

  const ask = useCallback<Ask>(
    (q) =>
      new Promise((resolve) =>
        setQuestion({
          ...q,
          answer: (v) => {
            setQuestion(null);
            resolve(v);
          },
        }),
      ),
    [],
  );

  const unsaved = useCallback(async (): Promise<Unsaved> => {
    if (session.readOnlyReason) return "clean";
    const r = await session.core.call("presetChanges", undefined).catch(() => null);
    if (!r?.dirty) return "clean";
    const a = await ask({
      title: "Unsaved Changes",
      body: (
        <>
          <p>The current preset has unsaved changes:</p>
          <ul class="change-list">
            {changeSummary(r.changes).map((c, i) => (
              <li key={i}>{c}</li>
            ))}
          </ul>
          <p>Save before continuing?</p>
        </>
      ),
      buttons: [
        { label: "Save", value: "save" },
        { label: "Discard", value: "discard" },
        { label: "Cancel", value: "cancel" },
      ],
    });
    return a === "save" || a === "discard" ? a : "cancel";
  }, [session, ask]);

  // Closing the session (Disconnect) asks first, as the Console's device
  // switch does, whichever screen is shown.
  useEffect(() => {
    session.leaveGuard = async () => {
      const a = await unsaved();
      if (a === "cancel") return false;
      if (a === "save") return session.savePreset(session.state.snapshot?.state.activePresetSlot ?? 0);
      return true;
    };
    return () => {
      session.leaveGuard = null;
    };
  }, [session, unsaved]);

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
      <screen.View session={session} ask={ask} unsaved={unsaved} />
      {question ? (
        <Dialog
          title={question.title}
          onCancel={() => question.answer("cancel")}
          buttons={question.buttons.map((b) => ({ label: b.label, danger: b.danger, onClick: () => question.answer(b.value) }))}
        >
          {question.body}
        </Dialog>
      ) : null}
    </>
  );
}
