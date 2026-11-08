// The main window of one session (WP WB6a, the first WB6 slice): volume,
// presets, and the inputs and outputs. It lives inside its session's view; a
// fleet page holds several at once, so nothing here is the page's.
//
// Writes go through the session (the core's `write` by registry path, with the
// read-back) and are off while the session is read only. The parts are
// memoised on the values they show, so a change elsewhere in the device's
// state (or the meters) does not redraw them.

import type { ComponentChildren } from "preact";
import { useCallback, useEffect, useMemo, useState } from "preact/hooks";
import type { ParamInfo } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { Session } from "../../ui/session";
import { ChannelList } from "./Channels";
import { Dialog } from "./Dialog";
import { changeSummary, inputRows, liveInputs, outputRows, sameItems } from "./model";
import { PresetsCard, type PresetView, type Unsaved } from "./Presets";
import { VolumeCard } from "./Volume";

/** A question in a modal dialog; the answer is the chosen button's value, or "cancel". */
export interface Question {
  title: string;
  body: ComponentChildren;
  buttons: { label: string; value: string; danger?: boolean }[];
}
export type Ask = (q: Question) => Promise<string>;

export function MainScreen(props: { session: Session }) {
  const session = props.session;
  const st = useSessionState(session);
  const snap = st.snapshot;
  const ro = session.readOnlyReason !== "";
  const [question, setQuestion] = useState<(Question & { answer: (v: string) => void }) | null>(null);
  const [changes, setChanges] = useState<string[]>([]);
  const [ranges, setRanges] = useState<{ pre: ParamInfo | null; gain: ParamInfo | null }>({ pre: null, gain: null });

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

  // The trims' ranges, as the core's registry gives them.
  useEffect(() => {
    let alive = true;
    void Promise.all([
      session.core.call("describe", { path: "pre" }).catch(() => null),
      session.core.call("describe", { path: "out.gain" }).catch(() => null),
    ]).then(([pre, gain]) => alive && setRanges({ pre, gain }));
    return () => {
      alive = false;
    };
  }, [session]);

  // How many inputs USB streams, from the device's status (the Console lists
  // those): read now and every 2 s while the page is seen; a redraw only when
  // the count changes.
  const [usbActive, setUsbActive] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      if (!alive) return;
      if (document.visibilityState === "visible" && session.state.status === "open") {
        try {
          const n = (await session.core.call("meters", undefined)).activeInputChannels;
          if (alive) setUsbActive(n);
        } catch {
          // Kept: the last count stands.
        }
      }
      if (alive) timer = setTimeout(read, 2000);
    };
    void read();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [session]);

  // What is unsaved, after every change the session reports (the core
  // compares with the active preset; no device I/O).
  useEffect(() => {
    if (!snap || ro) return;
    let alive = true;
    session.core.call("presetChanges", undefined).then(
      (r) => alive && setChanges((prev) => (sameItems(prev, r.changes) ? prev : r.changes)),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session, snap, ro]);

  /** The Console's unsaved-changes prompt (M42), when the live state is not the active preset's. */
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

  // Closing the session (Disconnect) asks first, as the Console's device switch does.
  useEffect(() => {
    session.leaveGuard = async () => {
      const a = await unsaved();
      if (a === "cancel") return false;
      if (a === "save") {
        const active = session.state.snapshot?.state.activePresetSlot ?? 0;
        return session.write("preset.save", "trigger", [active]);
      }
      return true;
    };
    return () => {
      session.leaveGuard = null;
    };
  }, [session, unsaved]);

  // Leaving the page with unsaved changes: the browser's own question (the Console's quit guard).
  const dirty = changes.length > 0 && !ro;
  useEffect(() => {
    if (!dirty) return;
    const hold = (e: BeforeUnloadEvent) => e.preventDefault();
    window.addEventListener("beforeunload", hold);
    return () => window.removeEventListener("beforeunload", hold);
  }, [dirty]);

  const s = snap?.state;
  const presetView = useMemo<PresetView | null>(
    () =>
      s
        ? {
            names: s.presetNames,
            occupied: s.presetOccupied,
            active: s.activePresetSlot,
            startupMode: s.presetStartupMode,
            defaultSlot: s.presetDefaultSlot,
          }
        : null,
    [s],
  );
  if (!s || !presetView) return null;
  const inputs = inputRows(s, liveInputs(s, usbActive));
  const outputs = outputRows(s);

  return (
    <div class="main-screen">
      <VolumeCard
        session={session}
        userDb={s.userVolumeDb}
        masterDb={s.masterVolumeDb}
        mute={s.userMute}
        bypass={s.bypass}
        disabled={ro}
      />
      <PresetsCard session={session} view={presetView} dirty={dirty} disabled={ro} ask={ask} unsaved={unsaved} />
      <ChannelList
        session={session}
        title="Inputs"
        rows={inputs}
        gains={inputs.map((r) => s.inputPreampDb[r.index] ?? 0)}
        mutes={null}
        gain={ranges.pre}
        disabled={ro}
      />
      <ChannelList
        session={session}
        title="Outputs"
        rows={outputs}
        gains={outputs.map((r) => s.outputGainDb[r.index] ?? 0)}
        mutes={outputs.map((r) => s.outputMuted[r.index] ?? false)}
        gain={ranges.gain}
        disabled={ro}
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
    </div>
  );
}
