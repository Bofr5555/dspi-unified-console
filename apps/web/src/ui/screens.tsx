// The screens of one session (WP WB6a): each WB6 slice adds its screen here.
// They live inside the session's view, never on the page: a fleet page shows
// several sessions at once, each with its own screen chosen. The tab row shows
// only once there is more than one screen.
//
// -- WP WB6b -- (c6p WB6a-4) What belongs to the session rather than to one
// screen lives here, so switching screens keeps it: the modal questions, the
// Console's unsaved-changes prompt (M42) and the guard Disconnect asks.

import type { ComponentChildren, FunctionComponent } from "preact";
import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
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
// -- WP WB8 -- the event log (with Save Diagnostics…), undo and redo
import { EventsScreen } from "../screens/events/EventsScreen";
import { useSessionState } from "./components";
import { StepNotice, UndoButtons, useUndoKeys } from "./undo";
// -- WP P1WB1 -- snapshots, A/B, safe apply, files; the keys Qt's table has
import { SnapshotDesk } from "../screens/snapshots/model";
import { AbBar, GuardBar } from "../screens/snapshots/Bars";
import { SnapshotsScreen } from "../screens/snapshots/SnapshotsScreen";
import { FilesScreen } from "../screens/files/FilesScreen";
import { useConsoleKeys, type ConsoleKey } from "./keys";
// -- WP P1WB2 -- the Installer page, the lock, the mode from the core's manifest
import type { ModeChoice, ModeManifest } from "../core/protocol";
import { InstallerScreen } from "../screens/installer/InstallerScreen";
import { LockedBanner, UnlockDialog } from "../screens/installer/Unlock";
import { RecoveryCode } from "../screens/installer/HubAdmin"; // -- WP HUBADMIN --
// -- WP WB6D -- the System view's blocks open the screen that edits them
import type { ChainTarget } from "../screens/system/chain";

// -- WP WB6F -- the status rail's health row and panel
import { HealthFeed } from "./status-rail/feed";
import { HealthRail } from "./status-rail/HealthRail";

// -- WP WB6G -- the input pages
import { InputsScreen } from "../screens/inputs/InputsScreen";

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
  // -- WP P1WB1 --
  /** The session's snapshots, A/B compare, safe apply and files. */
  desk?: SnapshotDesk;
  // -- WP WB6D --
  /** Open another screen of this session, at a channel (the System view's blocks). */
  open?: (t: ChainTarget) => void;
  /** Where this screen was opened at by `open`, null when its tab was chosen; read when it mounts. */
  target?: ChainTarget | null;
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
  // -- WP WB6G -- (UI-MODES: "Channels and EQ", every mode)
  { id: "inputs", label: "Inputs", View: InputsScreen },
  // -- WP WB6c --
  { id: "matrix", label: "Matrix", View: MatrixScreen },
  // -- WP SW3 -- (UI-MODES: the Pro layer, not in Simple)
  { id: "system", label: "System", View: SystemScreen },
  // -- WP P1WB1 -- (Advanced and Installer: setting up and handing over;
  // -- WP P1DCFIX -- Simple has Snapshots as recall and A/B only)
  { id: "snapshots", label: "Snapshots", View: SnapshotsScreen },
  { id: "files", label: "Files", View: FilesScreen },
  // -- WP WB8 -- (UI-MODES: monitoring of the Advanced and Installer modes)
  { id: "events", label: "Events", View: EventsScreen },
  // -- WP P1WB2 -- (the manifest's "Limits and power-on": the Installer mode only)
  { id: "installer", label: "Installer", View: InstallerScreen },
];

// -- WP P1WB1 --
/** The screens Simple leaves out besides the Pro layer's: files are for setting up (-- WP P1DCFIX -- Snapshots stay, recall and A/B only). */
const SETUP = new Set(["files"]);

// -- WP SW3 --
/**
 * The screens `mode` shows, in order. Advanced and Installer have the System
 * view, first when the session is on a hub with an audio link installation
 * (the Pro layer's first screen) and after today's screens otherwise; Simple
 * has today's screens (UI-MODES section 2: it hides nothing permanently).
 */
export function screensFor(mode: Mode, installation: boolean): ScreenDef[] {
  // -- WP WB8 -- the event log last, in Advanced and Installer
  const today = SCREENS.filter((x) => x.id !== "system" && x.id !== "events" && x.id !== "installer");
  if (mode === "simple") return today.filter((x) => !SETUP.has(x.id)); // -- WP P1WB1 --
  const system = SCREENS.find((x) => x.id === "system")!;
  const events = SCREENS.find((x) => x.id === "events")!;
  // -- WP P1WB2 -- the Installer page last, in the Installer mode
  const tail = mode === "installer" ? SCREENS.filter((x) => x.id === "installer") : [];
  return installation ? [system, ...today, events, ...tail] : [...today, system, events, ...tail];
}

// -- WP P1WB2 --
/** The locked-user state's screens: the front page alone (the manifest's `lockedUser.screens`). */
export function lockedScreens(): ScreenDef[] {
  return SCREENS.filter((x) => x.id === "main");
}

function isMode(m: string): m is Mode {
  return m === "simple" || m === "advanced" || m === "installer";
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
  // -- WP WB6F -- one health feed per session: problems stay until acknowledged, whichever screen is shown
  const health = useMemo(() => new HealthFeed(session), [session]);
  useEffect(() => {
    health.start();
    return () => health.stop();
  }, [health]);
  const serial = session.state.snapshot?.device.serial || session.deviceId || session.id;
  // -- WP P1WB2 -- the user's choice, then the core's resolution of it (lock, hub or fleet pin)
  const [chosenMode, setMode] = useState<Mode>(() => loadMode(safeLocalStorage(), serial));
  const st = useSessionState(session);
  const [manifest, setManifest] = useState<ModeManifest | null>(null);
  useEffect(() => {
    void session.core.call("modeManifest", undefined).then(setManifest, () => setManifest(null));
  }, [session]);
  const [resolved, setResolved] = useState<{ chosen: Mode; choice: ModeChoice } | null>(null);
  useEffect(() => {
    let alive = true;
    void session.core.call("modeResolve", { platform: "web", chosen: chosenMode }).then(
      (choice) => alive && setResolved({ chosen: chosenMode, choice }),
      () => alive && setResolved(null),
    );
    return () => {
      alive = false;
    };
  }, [session, chosenMode, st.lock, st.lockRev]);
  const choice = resolved && resolved.chosen === chosenMode ? resolved.choice : null;
  const lockedUser = choice?.mode === "locked_user";
  const mode: Mode = choice && isMode(choice.mode) ? choice.mode : lockedUser ? "simple" : chosenMode;
  const modes = (manifest?.modes ?? []).filter((m) => isMode(m.id)).map((m) => ({ id: m.id as Mode, label: m.label }));
  const [unlocking, setUnlocking] = useState(false);
  const shown = lockedUser ? lockedScreens() : screensFor(mode, feed.kind === "hub");
  const [chosen, setChosen] = useState<string | null>(null);
  const current = chosen ?? shown[0]!.id;
  const screen = shown.find((x) => x.id === current) ?? shown[0]!;
  // -- WP WB6D -- a tab chosen opens its screen as it was; `openAt` opens one at a channel
  const [target, setTarget] = useState<ChainTarget | null>(null);
  const setCurrent = (id: string) => {
    setTarget(null);
    setChosen(id);
  };
  const openAt = useCallback((t: ChainTarget) => {
    setTarget(t);
    setChosen(t.screen);
  }, []);
  const [question, setQuestion] = useState<(Question & { answer: (v: string) => void }) | null>(null);
  // -- WP WB8 -- undo and redo: the buttons follow the session's state; the keys act on this view
  // (-- WP P1WB2 -- `st` is read above, for the lock)
  const root = useRef<HTMLDivElement>(null);
  useUndoKeys(session, root);
  // -- WP P1WB1 -- one desk per session: snapshots, A/B, safe apply, files
  const desk = useMemo(() => new SnapshotDesk(session), [session]);
  useEffect(() => {
    void desk.start();
    return () => desk.stop();
  }, [desk]);

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

  // -- WP P1WB1 -- the keys Qt's table has (ui/keys.ts): presets, commit, revert, mute, bypass, A/B
  const asking = useRef(false);
  asking.current = question !== null;
  const onKey = useCallback(
    (k: ConsoleKey) => {
      if (asking.current || session.state.status !== "open") return;
      void runKey(k, session, desk, ask, unsaved);
    },
    [session, desk, ask, unsaved],
  );
  useConsoleKeys(root, onKey);

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
    <div class="session-screens" ref={root}>
      {/* -- WP SW3 -- on an installation, the scene bar */}
      {mode !== "simple" && !lockedUser ? <SceneBar feed={system} session={session} ask={ask} /> : null}
      {/* -- WP P1WB2 -- the locked-user front page's banner; the Installer mode asking for its PIN */}
      {lockedUser && st.lock ? <LockedBanner session={session} lock={st.lock} /> : null}
      {choice?.needsPin && !lockedUser ? (
        <div class="banner banner-pin" role="status">
          <span>The Installer mode needs the installer's PIN.</span>
          <button type="button" onClick={() => setUnlocking(true)}>
            Unlock…
          </button>
        </div>
      ) : null}
      {unlocking ? <UnlockDialog session={session} lock={st.lock} onDone={() => setUnlocking(false)} /> : null}
      {/* -- WP HUBADMIN -- the hub's recovery code, shown once: here, so it stays when the new PIN closes the Installer mode */}
      {st.hubRecovery ? <RecoveryCode session={session} code={st.hubRecovery} /> : null}
      {/* -- WP P1WB1 -- the scene bar's host-side half: the A/B toggle; a safe apply's countdown in every mode */}
      {/* -- WP P1DCFIX -- the A/B toggle in every mode too (Simple has the compare) */}
      {lockedUser ? null : <AbBar desk={desk} />}
      <GuardBar desk={desk} />
      {/* -- WP WB8 -- the tabs and, beside them, Undo and Redo */}
      <div class="screen-bar">
        {shown.length > 1 ? (
          <div class="screen-tabs" role="tablist" aria-label="Screens">
            {shown.map((x) => (
              <button type="button" role="tab" key={x.id} aria-selected={x === screen} onClick={() => setCurrent(x.id)}>
                {x.label}
              </button>
            ))}
          </div>
        ) : null}
        <UndoButtons session={session} state={st} />
      </div>
      <StepNotice session={session} notice={st.notice} />
      <screen.View session={session} ask={ask} unsaved={unsaved} system={system} mode={mode} desk={desk} open={openAt} target={target} />
      {/* -- WP SW3 -- the status rail, below the screen as a status bar */}
      <StatusRail
        feed={system}
        session={session}
        mode={mode}
        choice={choice}
        modes={modes}
        onMode={(m, every) => {
          saveMode(safeLocalStorage(), serial, m, every);
          setMode(m);
        }}
      >
        {/* -- WP WB6F -- */}
        <HealthRail session={session} feed={health} />
      </StatusRail>
      {question ? (
        <Dialog
          title={question.title}
          onCancel={() => question.answer("cancel")}
          buttons={question.buttons.map((b) => ({ label: b.label, danger: b.danger, onClick: () => question.answer(b.value) }))}
        >
          {question.body}
        </Dialog>
      ) : null}
    </div>
  );
}

// -- WP P1WB1 --
/**
 * One key of Qt's table, as the menus do it: a preset load and Revert ask about unsaved
 * changes first, Commit asks before it writes flash; mute and bypass flip; A/B toggles a
 * running compare (off while a safe apply waits). Nothing while read only.
 */
export async function runKey(k: ConsoleKey, session: Session, desk: SnapshotDesk, ask: Ask, unsaved: () => Promise<Unsaved>): Promise<void> {
  const snap = session.state.snapshot;
  if (!snap || session.readOnlyReason) return;
  const st = snap.state;
  const confirm = async (title: string, text: string, yes: string) =>
    (await ask({ title, body: <p>{text}</p>, buttons: [{ label: yes, value: "yes" }, { label: "Cancel", value: "cancel" }] })) === "yes";
  switch (k.action) {
    case "preset": {
      if (k.slot === st.activePresetSlot) return;
      const a = await unsaved();
      if (a === "cancel") return;
      if (a === "save" && !(await session.savePreset(st.activePresetSlot))) return;
      await session.write("preset.load", "trigger", [k.slot]);
      return;
    }
    case "commit":
      if (await confirm("Save Preset", `Save current parameters to preset slot ${st.activePresetSlot + 1}?`, "Save")) await session.savePreset(st.activePresetSlot);
      return;
    case "revert":
      if (await confirm("Revert to Saved", "Revert to last saved parameters? Current unsaved changes will be lost.", "Revert"))
        await session.change("presetRevert", undefined);
      return;
    case "mute":
      await session.write("vol.mute", { bool: !st.userMute });
      return;
    case "bypass":
      await session.write("bypass", { bool: !st.bypass });
      return;
    case "ab":
      await desk.abToggle();
      return;
  }
}
