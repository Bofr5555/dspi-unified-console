// WP WB6i: Core Voltage (M56), the Installer mode's: a thin page over the core's calibration
// flow and manual requests (WP CG11, `crates/dspi-core/src/vreg.rs`). The core decides
// everything: which voltages are offered (the build default up to 1.25 V), when the
// confirmation word is needed (above 1.20 V and above the unit's voltage now, CG14), the
// soak's minimum, the result. This page shows the core's view, polls it about once a second
// while shown (a poll writes only the next step of a run the user started), asks before
// every request, and passes what the user typed. A save carries a `vreg_save` token.

import { useEffect, useRef, useState } from "preact/hooks";
import { CoreCallError, type CoreMethods, type VregCalView, type VregChoice } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { Dialog } from "../main/Dialog";
import { durationText, soakMinutes, voltsText } from "./model";
import type { ComponentChildren } from "preact";
import { Icon } from "./icons"; // -- WP WEB7 --

/** -- WP WEB7 -- A group as Qt's SettingsGroup (group.css), named as a `group` for assistive technology. */
function VGroup(props: { title: string; icon: string; children: ComponentChildren }) {
  return (
    <div class="group" role="group" aria-label={props.title}>
      <div class="group-head">
        <span class="group-icon">
          <Icon name={props.icon} />
        </span>
        <h4 class="group-title">{props.title}</h4>
      </div>
      <div class="group-panel">{props.children}</div>
    </div>
  );
}

const POLL_MS = 1000;

/** While these run, the flow owns the device's voltage: no manual request, no Close. */
const RUNNING = new Set<VregCalView["phase"]>(["clearing", "starting_trial", "soaking", "passed", "ending", "committing"]);

type VregCall = "vregOpen" | "vregClose" | "vregStart" | "vregKeep" | "vregCommit" | "vregTrialEnd" | "vregSet" | "vregTrial" | "vregAuto" | "vregClear";

interface Ask {
  title: string;
  text: string;
  /** The core asks for its word: a field to type it in. */
  word: boolean;
  yes: string;
  run: (typed: string | undefined) => Promise<void>;
}

function messageOf(e: unknown): string {
  return e instanceof CoreCallError ? e.message : e instanceof Error ? e.message : String(e);
}

export function CoreVoltage(props: { session: Session }) {
  const { session } = props;
  const [view, setView] = useState<VregCalView | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [typed, setTyped] = useState("");
  const [trialCode, setTrialCode] = useState<number | null>(null);
  const [manualCode, setManualCode] = useState<number | null>(null);
  const [soakMin, setSoakMin] = useState<number | null>(null);
  const shown = useRef<VregCalView | null>(null);
  shown.current = view;

  useEffect(() => {
    let alive = true;
    const poll = () =>
      session.core.call("vregPoll", undefined).then(
        (v) => alive && setView(v),
        (e) => alive && setError(messageOf(e)),
      );
    void poll();
    const t = setInterval(() => void poll(), POLL_MS);
    return () => {
      alive = false;
      clearInterval(t);
      // Leaving the page closes an open flow; a run goes on and is found again on return.
      if (shown.current?.phase === "review") void session.core.call("vregClose", undefined).catch(() => undefined);
    };
  }, [session]);

  const act = async <M extends VregCall>(method: M, params: CoreMethods[M]["params"]) => {
    setBusy(true);
    try {
      const v = (await session.core.call(method, params)) as VregCalView;
      setView(v);
      setError("");
    } catch (e) {
      setError(messageOf(e));
      void session.core.call("vregView", undefined).then(setView, () => undefined);
    } finally {
      setBusy(false);
    }
  };

  if (!view) {
    return (
      <section class="settings-vreg" aria-label="Core Voltage">
        <p class="note">Reading…</p>
        {error ? <p class="error" role="alert">{error}</p> : null}
      </section>
    );
  }

  const ro = !!session.readOnlyReason;
  const st = view.status;
  const mv = (code: number) => view.choices.find((c) => c.code === code)?.millivolts ?? 0;
  const choiceText = (c: VregChoice) => voltsText(c.millivolts);
  const running = RUNNING.has(view.phase);
  const off = ro || busy || !view.available;
  const soaks = soakMinutes(view.minSoakS, view.defaultSoakS);
  const soak = soakMin ?? Math.round(view.defaultSoakS / 60);
  const trial = view.choices.find((c) => c.code === trialCode) ?? view.choices.find((c) => c.code === view.nextVreg) ?? view.choices[0] ?? null;
  const manual = view.choices.find((c) => c.code === manualCode) ?? view.choices.find((c) => c.code === st?.liveVreg) ?? view.choices[0] ?? null;
  const target = mv(view.targetVreg) ? voltsText(mv(view.targetVreg)) : `code ${view.targetVreg}`;
  const word = view.confirmationWord;

  const confirm = (a: Ask) => {
    setTyped("");
    setAsk(a);
  };
  const save = async (method: "vregCommit" | "vregSet", typedWord: string | undefined, value?: number) => {
    const token = await session.token("vreg_save");
    if (method === "vregCommit") await act("vregCommit", { typed: typedWord, token });
    else await act("vregSet", { value: value ?? 0, typed: typedWord, token });
  };

  return (
    <section class="settings-vreg" aria-label="Core Voltage">
      {!view.available && view.phase === "idle" ? (
        <p class="note">This DSPi offers no core-voltage control to this console.</p>
      ) : null}

      <VGroup title="On the Device" icon="cpu">
        {st ? (
          <dl class="settings-facts">
            <dt>Running at</dt>
            <dd>
              {voltsText(st.millivolts)}
              {st.trialActive ? " (trial)" : ""}
            </dd>
            <dt>Saved</dt>
            <dd>{st.storedSel ? voltsText(mv(st.storedSel)) || `code ${st.storedSel}` : "Build default"}</dd>
            <dt>Build default</dt>
            <dd>
              {voltsText(mv(st.defaultVreg)) || `code ${st.defaultVreg}`}
              {st.autoRaised ? ", raised by the device" : ""}
            </dd>
            <dt>Automatic raising</dt>
            <dd>{st.autoDisabled ? "Off" : "On"}</dd>
            <dt>Hangs this session</dt>
            <dd>{st.hangCount}</dd>
            <dt>Last failed trial</dt>
            <dd>
              {st.failedTrialVreg
                ? `${voltsText(mv(st.failedTrialVreg)) || `code ${st.failedTrialVreg}`}${st.failedTrialUptimeS ? `, hung after ${durationText(st.failedTrialUptimeS)}` : ""}`
                : "None"}
            </dd>
          </dl>
        ) : (
          <p class="note">Reading…</p>
        )}
      </VGroup>

      <VGroup title="Calibrate" icon="gauge">
        <p class="vreg-state" role="status" aria-label="Calibration state">
          {view.message}
        </p>
        {view.linkLost ? <p class="note">The device stopped answering. The result is still open.</p> : null}
        {view.phase === "soaking" ? (
          <p class="vreg-progress">
            {durationText(view.elapsedS)} of {durationText(view.soakS)} at {target}
          </p>
        ) : null}
        {view.phase !== "idle" ? (
          <>
            <p class="vreg-warning">{view.warning}</p>
            <p class="note">{view.soakNote}</p>
          </>
        ) : null}
        {view.canStart || view.canKeep ? (
          <div class="settings-row">
            {view.canStart ? (
              <label>
                Trial voltage{" "}
                <select value={trial?.code ?? ""} disabled={off} onChange={(e) => setTrialCode(Number((e.target as HTMLSelectElement).value))}>
                  {view.choices.map((c) => (
                    <option key={c.code} value={c.code}>
                      {choiceText(c)}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}
            <label>
              Soak time{" "}
              <select value={soak} disabled={off} onChange={(e) => setSoakMin(Number((e.target as HTMLSelectElement).value))}>
                {soaks.map((m) => (
                  <option key={m} value={m}>
                    {m * 60 === view.defaultSoakS ? `${durationText(m * 60)} (suggested)` : durationText(m * 60)}
                  </option>
                ))}
              </select>
            </label>
          </div>
        ) : null}
        <div class="settings-buttons">
          {view.phase === "idle" || view.phase === "ended" || view.phase === "committed" || view.phase === "verify_failed" ? (
            <button type="button" disabled={off} onClick={() => void act("vregOpen", undefined)}>
              Calibrate…
            </button>
          ) : null}
          {view.canStart && trial ? (
            <button
              type="button"
              disabled={off}
              onClick={() =>
                confirm({
                  title: "Start Trial",
                  text: `Start a trial at ${choiceText(trial)} for ${durationText(soak * 60)}? Nothing is saved; a hang or a reboot returns to the saved voltage.`,
                  word: trial.needsWord,
                  yes: "Start",
                  run: (w) => act("vregStart", { code: trial.code, soakS: soak * 60, typed: w }),
                })
              }
            >
              Start…
            </button>
          ) : null}
          {view.canKeep ? (
            <button
              type="button"
              disabled={off}
              onClick={() =>
                confirm({
                  title: "Keep Trial",
                  text: `Keep the trial at ${target} and soak it for ${durationText(soak * 60)}?`,
                  word: false,
                  yes: "Keep",
                  run: () => act("vregKeep", { soakS: soak * 60 }),
                })
              }
            >
              Keep…
            </button>
          ) : null}
          {view.canCommit ? (
            <button
              type="button"
              disabled={off}
              onClick={() =>
                confirm({
                  title: "Save Voltage",
                  text: `Save ${target} on the device? It runs at this voltage from now on, after every reboot.`,
                  word: view.commitNeedsWord,
                  yes: "Save",
                  run: (w) => save("vregCommit", w),
                })
              }
            >
              Save…
            </button>
          ) : null}
          {view.phase === "soaking" || view.phase === "starting_trial" || view.phase === "clearing" ? (
            <button type="button" disabled={ro || busy} onClick={() => void act("vregTrialEnd", undefined)}>
              Abort
            </button>
          ) : null}
          {view.phase === "passed" || view.phase === "trial_found" || view.phase === "trial_left_running" || view.phase === "verify_failed" ? (
            <button type="button" disabled={ro || busy} onClick={() => void act("vregTrialEnd", undefined)}>
              End Trial
            </button>
          ) : null}
          {view.phase === "review" || view.phase === "failed" || view.phase === "unknown" ? (
            <button type="button" disabled={busy} onClick={() => void act("vregClose", undefined)}>
              Close
            </button>
          ) : null}
        </div>
      </VGroup>

      <VGroup title="Manual" icon="sliders-horizontal">
        {running ? <p class="note">Not available while a calibration runs.</p> : null}
        <div class="settings-row">
          <label>
            Voltage{" "}
            <select value={manual?.code ?? ""} disabled={off || running || !view.choices.length} onChange={(e) => setManualCode(Number((e.target as HTMLSelectElement).value))}>
              {view.choices.map((c) => (
                <option key={c.code} value={c.code}>
                  {choiceText(c)}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div class="settings-buttons">
          <button
            type="button"
            disabled={off || running || !manual}
            onClick={() =>
              manual &&
              confirm({
                title: "Trial Voltage",
                text: `Try ${choiceText(manual)} until the next reboot? Nothing is saved.`,
                word: manual.needsWord,
                yes: "Try",
                run: (w) => act("vregTrial", { value: manual.code, typed: w }),
              })
            }
          >
            Trial…
          </button>
          <button
            type="button"
            disabled={off || running || !manual}
            onClick={() =>
              manual &&
              confirm({
                title: "Save Voltage",
                text: `Save ${choiceText(manual)} on the device? It runs at this voltage from now on, after every reboot.`,
                word: manual.saveNeedsWord,
                yes: "Save",
                run: (w) => save("vregSet", w, manual.code),
              })
            }
          >
            Save…
          </button>
          <button
            type="button"
            disabled={off || running || !st}
            onClick={() =>
              confirm({
                title: "Reset Voltage",
                text: `Go back to the build default (${voltsText(mv(st?.defaultVreg ?? 0)) || "the factory setting"})?`,
                word: view.resetNeedsWord,
                yes: "Reset",
                run: (w) => save("vregSet", w, 0),
              })
            }
          >
            Reset…
          </button>
          <button
            type="button"
            disabled={off || running || !st}
            onClick={() => {
              const on = !!st?.autoDisabled;
              confirm({
                title: "Automatic raising",
                text: on
                  ? "Let the device raise its saved voltage by itself after repeated hangs?"
                  : "Stop the device from raising its saved voltage by itself?",
                word: false,
                yes: on ? "Turn On" : "Turn Off",
                run: () => act("vregAuto", { on }),
              });
            }}
          >
            {st?.autoDisabled ? "Turn On Automatic Raising…" : "Turn Off Automatic Raising…"}
          </button>
          <button
            type="button"
            disabled={off || running}
            onClick={() =>
              confirm({
                title: "Clear Record",
                text: "Forget the hang count and the last failed trial?",
                word: false,
                yes: "Clear",
                run: () => act("vregClear", undefined),
              })
            }
          >
            Clear…
          </button>
        </div>
      </VGroup>

      {error || view.lastError ? (
        <p class="error" role="alert">
          {error || view.lastError}
        </p>
      ) : null}

      {ask ? (
        <Dialog
          title={ask.title}
          onCancel={() => setAsk(null)}
          buttons={[
            {
              label: ask.yes,
              danger: ask.word,
              onClick: () => {
                const a = ask;
                setAsk(null);
                void a.run(a.word ? typed : undefined);
              },
            },
            { label: "Cancel", onClick: () => setAsk(null) },
          ]}
        >
          <p>{ask.text}</p>
          {ask.word ? (
            <>
              <p class="vreg-warning">{view.warning}</p>
              <label class="vreg-word">
                {`The core asks for its word for this voltage: type ${word}, exactly, to confirm.`}
                <input type="text" aria-label="Confirmation word" autocomplete="off" spellcheck={false} value={typed} onInput={(e) => setTyped((e.target as HTMLInputElement).value)} />
              </label>
            </>
          ) : null}
        </Dialog>
      ) : null}
    </section>
  );
}
