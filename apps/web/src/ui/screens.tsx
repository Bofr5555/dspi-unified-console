// The screens of one session (WP WB6a): each WB6 slice adds its screen here.
// They live inside the session's view, never on the page: a fleet page shows
// several sessions at once, each with its own screen chosen. The tab row shows
// only once there is more than one screen.
//
// -- WP WB6b -- (c6p WB6a-4) What belongs to the session rather than to one
// screen lives here, so switching screens keeps it: the modal questions, the
// Console's unsaved-changes prompt (M42) and the guard Disconnect asks.

import type { ComponentChildren, FunctionComponent } from "preact";
import { useCallback, useEffect, useMemo, useState } from "preact/hooks";
import { EqScreen } from "../screens/eq/EqScreen";
import { Dialog } from "../screens/main/Dialog";
import { MainScreen } from "../screens/main/MainScreen";
// -- WP WB6c --
import { MatrixScreen } from "../screens/matrix/MatrixScreen";
import { changeSummary } from "../screens/main/model";
import type { Session } from "./session";
// -- WP SW3 -- the System view (WB6d), the scene bar (WB6e), the status rail (WB6f)
import { safeLocalStorage } from "../fleet/local";
import { SystemFeed } from "../screens/system/feed";
import { loadMode, saveMode, type Mode } from "../screens/system/model";
import { SceneBar } from "../screens/system/SceneBar";
import { StatusRail } from "../screens/system/StatusRail";
import { SystemScreen, useFeed } from "../screens/system/SystemScreen";

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
  // -- WP SW3 --
  /** The session's system: the hub's installation, or this device as one unit. */
  system?: SystemFeed;
  /** The mode the console shows (docs/UI-MODES.md). */
  mode?: Mode;
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
  // -- WP WB6c --
  { id: "matrix", label: "Matrix", View: MatrixScreen },
  // -- WP SW3 -- (UI-MODES: the Pro layer, not in Simple)
  { id: "system", label: "System", View: SystemScreen },
];

// -- WP SW3 --
/**
 * The screens `mode` shows, in order. Advanced and Installer have the System
 * view, first when the session is on a hub with an audio link installation
 * (the Pro layer's first screen) and after today's screens otherwise; Simple
 * has today's screens (UI-MODES section 2: it hides nothing permanently).
 */
export function screensFor(mode: Mode, installation: boolean): ScreenDef[] {
  const today = SCREENS.filter((x) => x.id !== "system");
  if (mode === "simple") return today;
  const system = SCREENS.find((x) => x.id === "system")!;
  return installation ? [system, ...today] : [...today, system];
}

export function Screens(props: { session: Session }) {
  const session = props.session;
  // -- WP SW3 -- one system feed per session, and the mode remembered for its device
  const system = useMemo(() => new SystemFeed(session), [session]);
  useEffect(() => {
    system.start();
    return () => system.stop();
  }, [system]);
  const feed = useFeed(system);
  const serial = session.state.snapshot?.device.serial || session.deviceId || session.id;
  const [mode, setMode] = useState<Mode>(() => loadMode(safeLocalStorage(), serial));
  const shown = screensFor(mode, feed.kind === "hub");
  const [chosen, setChosen] = useState<string | null>(null);
  const current = chosen ?? shown[0]!.id;
  const screen = shown.find((x) => x.id === current) ?? shown[0]!;
  const setCurrent = setChosen;
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
      {/* -- WP SW3 -- on an installation, the scene bar */}
      {mode !== "simple" ? <SceneBar feed={system} session={session} ask={ask} /> : null}
      {shown.length > 1 ? (
        <div class="screen-tabs" role="tablist" aria-label="Screens">
          {shown.map((x) => (
            <button type="button" role="tab" key={x.id} aria-selected={x === screen} onClick={() => setCurrent(x.id)}>
              {x.label}
            </button>
          ))}
        </div>
      ) : null}
      <screen.View session={session} ask={ask} unsaved={unsaved} system={system} mode={mode} />
      {/* -- WP SW3 -- the status rail, below the screen as a status bar */}
      <StatusRail
        feed={system}
        session={session}
        mode={mode}
        onMode={(m, every) => {
          saveMode(safeLocalStorage(), serial, m, every);
          setMode(m);
        }}
      />
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
