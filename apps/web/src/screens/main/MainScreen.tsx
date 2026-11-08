// The main window of one session (WP WB6a, the first WB6 slice): volume,
// presets, and the inputs and outputs. It lives inside its session's view; a
// fleet page holds several at once, so nothing here is the page's.
//
// Writes go through the session (the core's `write` by registry path, with the
// read-back) and are off while the session is read only. The parts are
// memoised on the values they show, so a change elsewhere in the device's
// state (or the meters) does not redraw them.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { ParamInfo } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { ChannelList } from "./Channels";
import { inputRows, liveInputs, outputRows } from "./model";
import { PresetsCard, type PresetView } from "./Presets";
import { VolumeCard } from "./Volume";

// -- WP WB6b -- the questions and the unsaved-changes prompt are the session
// view's now (src/ui/screens.tsx), so every screen shares them.
export type { Ask, Question } from "../../ui/screens";

export function MainScreen(props: ScreenProps) {
  const { session, ask, unsaved } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;
  const ro = session.readOnlyReason !== "";
  const [ranges, setRanges] = useState<{ pre: ParamInfo | null; gain: ParamInfo | null }>({ pre: null, gain: null });

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

  // What is unsaved: the session's, kept current after every change (-- WP WB6b --).
  const dirty = st.unsaved.length > 0 && !ro;

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
    </div>
  );
}
